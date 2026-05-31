//! Benchmark #4: the coordination win — the one place the per-move economics
//! might escape the Amdahl cap of benchmark #2.
//!
//! Benchmarks #1–3 priced *local computation*. This one prices *coordination*.
//! A conventional balancer that maintains a global invariant `g(fleet) ≤ τ`
//! must serialize moves against a global check — a fleet-wide lock / barrier /
//! consensus. The witness certifies a `HotToCold` from only its two machines'
//! state, so two moves that touch disjoint machines need **no shared
//! coordination at all**. That collapses the coordination granularity from
//! global (contend with every move) to local (contend only with moves touching
//! the same machine).
//!
//! Modeled with real, measured lock contention (shared-memory threads), not
//! asserted network latencies:
//!
//!   GLOBAL — the whole fleet behind one `Mutex`. Every move serializes. This
//!            is what a proof-less global-invariant system is forced into.
//!   LOCAL  — one `Mutex` per machine; a move locks only its two machines
//!            (lower id first, deadlock-free). Disjoint moves run in parallel.
//!            Safe *because* the witness proves each move locally (benches #1–3).
//!
//! Two access patterns, because this is exactly where the local win can be
//! oversold:
//!   UNIFORM — random disjoint pairs: the best case for locality.
//!   SKEWED  — every move pulls from one of a few hot machines: locality
//!            re-serializes on those, the honest limit.
//!
//! The critical section is the same trivial transfer in both schemes (no
//! gauge check charged to GLOBAL — generous, so this is a *lower bound*: a
//! faithful conventional system also pays an O(M) recheck under its lock).
//!
//!     cargo run --release --example bench_distributed

use std::hint::black_box;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

const M: usize = 1024; // machines
const MOVES: u64 = 2_000_000; // total transfers across all threads
const HOT: usize = 8; // skewed: number of hot source machines

#[inline]
fn xs(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// Pick an ordered (src, dst), src != dst, per the access pattern.
#[inline]
fn pick(s: &mut u64, skewed: bool) -> (usize, usize) {
    if skewed {
        let src = (xs(s) as usize) % HOT;
        let dst = HOT + (xs(s) as usize) % (M - HOT);
        (src, dst)
    } else {
        let src = (xs(s) as usize) % M;
        let mut dst = (xs(s) as usize) % M;
        if dst == src {
            dst = (dst + 1) % M;
        }
        (src, dst)
    }
}

/// GLOBAL: one lock for the whole fleet. Every move serializes.
fn run_global(threads: usize, skewed: bool) -> f64 {
    let fleet = Arc::new(Mutex::new(vec![1_000_000u64; M]));
    let per = MOVES / threads as u64;
    let t = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|tid| {
            let fleet = Arc::clone(&fleet);
            thread::spawn(move || {
                let mut s = 0x9E37_79B9_7F4A_7C15u64 ^ (tid as u64 + 1).wrapping_mul(0x1234_5678);
                for _ in 0..per {
                    let (src, dst) = pick(&mut s, skewed);
                    let mut g = fleet.lock().unwrap();
                    if g[src] > 0 {
                        g[src] -= 1;
                        g[dst] += 1;
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let secs = t.elapsed().as_secs_f64();
    black_box(fleet.lock().unwrap().iter().sum::<u64>());
    (per * threads as u64) as f64 / secs
}

/// LOCAL: one lock per machine; a move locks only its two machines.
fn run_local(threads: usize, skewed: bool) -> f64 {
    let fleet: Arc<Vec<Mutex<u64>>> = Arc::new((0..M).map(|_| Mutex::new(1_000_000u64)).collect());
    let per = MOVES / threads as u64;
    let t = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|tid| {
            let fleet = Arc::clone(&fleet);
            thread::spawn(move || {
                let mut s = 0x9E37_79B9_7F4A_7C15u64 ^ (tid as u64 + 1).wrapping_mul(0x1234_5678);
                for _ in 0..per {
                    let (src, dst) = pick(&mut s, skewed);
                    // Lock lower index first (deadlock-free), then transfer src->dst.
                    let (lo, hi) = (src.min(dst), src.max(dst));
                    let mut g_lo = fleet[lo].lock().unwrap();
                    let mut g_hi = fleet[hi].lock().unwrap();
                    let (sref, dref): (&mut u64, &mut u64) = if src < dst {
                        (&mut g_lo, &mut g_hi)
                    } else {
                        (&mut g_hi, &mut g_lo)
                    };
                    if *sref > 0 {
                        *sref -= 1;
                        *dref += 1;
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let secs = t.elapsed().as_secs_f64();
    black_box(fleet.iter().map(|m| *m.lock().unwrap()).sum::<u64>());
    (per * threads as u64) as f64 / secs
}

fn section(title: &str, skewed: bool, thread_counts: &[usize]) {
    println!("\n## {title}");
    println!(
        "{:>8} | {:>12} {:>12} | {:>9}",
        "threads", "global M/s", "local M/s", "local/glob",
    );
    println!("{}", "-".repeat(50));
    for &th in thread_counts {
        let g = run_global(th, skewed) / 1e6;
        let l = run_local(th, skewed) / 1e6;
        println!("{:>8} | {:>12.2} {:>12.2} | {:>8.2}x", th, g, l, l / g);
    }
}

fn main() {
    let cores = thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let thread_counts: Vec<usize> = [1usize, 2, 4, 8, 16]
        .into_iter()
        .filter(|&t| t <= cores.max(8) * 2)
        .collect();
    println!("# M={M} machines, {MOVES} transfers, {cores} hardware threads");
    println!("# critical section = bare transfer (no gauge check charged to GLOBAL => lower bound)");

    section("UNIFORM access (disjoint pairs — best case for locality)", false, &thread_counts);
    section("SKEWED access (all pulls from 8 hot machines — locality's limit)", true, &thread_counts);

    println!(
        "\n# Read: GLOBAL serializes every move through one lock, so throughput is\n\
         #   flat-to-falling in thread count. LOCAL lets disjoint moves proceed in\n\
         #   parallel, so it scales — UNTIL the access pattern concentrates on a few\n\
         #   machines (SKEWED), where it re-serializes on those locks.\n\
         # This is a LOWER BOUND on the distributed gap: (1) shared-memory locks are\n\
         #   ~1000x cheaper than network coordination, so the real gap is far larger;\n\
         #   (2) GLOBAL was not even charged the O(M) recheck it actually needs.\n\
         # The witness is what makes LOCAL correct: without a per-move proof you\n\
         #   cannot drop the global lock. This prices what the witness ENABLES.\n\
         # Amdahl footnote: admission (Place) still needs coordination for a\n\
         #   non-separable gauge; for a separable gauge (Linfty) admission is also\n\
         #   local, so the #2 cap dissolves and the fleet rebalances coordination-free."
    );
}
