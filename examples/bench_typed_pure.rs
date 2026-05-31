//! Benchmark #1: is the typed-pure path actually *computationally* cheaper?
//!
//! The thesis is "type discipline makes load balancing cheap": a typed-pure
//! move (`HotToCold`) is admitted by a **local witness** and its `apply` is
//! total — no gauge recheck — whereas a catch-all move pays
//! `apply_with_recheck`, i.e. a full `gauge.eval(&fleet)`. This harness puts a
//! number on that, swept across fleet size `M`, and — crucially — pits it
//! against the *honest* baseline (a hand-maintained incremental gauge), not
//! just the naive full re-evaluation, so the result can puncture the claim as
//! easily as vindicate it.
//!
//! Three strategies for "decide that a rebalancing move keeps `g(fleet) ≤ τ`":
//!
//!   A. WITNESS         — `HotToCold::witness(..)`: 2 BTreeMap lookups + an
//!                        O(N) Pigou-Dalton check. Evaluates *no* gauge.
//!                        Gauge-agnostic: the same check certifies safety for
//!                        every Schur-convex gauge at once.  ── O(log M + N)
//!   B. FULL RECHECK    — `gauge.eval(&fleet)`: the worst-dim-per-machine
//!                        reduction + a sort. This is exactly what Fulcrum's
//!                        own `apply_with_recheck` does, so A-vs-B is a
//!                        Fulcrum-internal comparison, not a strawman.  ── O(M log M)
//!   C. INCREMENTAL MAX — steelman: keep the per-machine worst-utils in an
//!                        ordered multiset, update the two touched machines,
//!                        read the max. The competent imperative baseline.
//!                        Must be re-derived per gauge; here only for Linfty.  ── O(log M)
//!
//! Run it for real numbers:
//!     cargo run --release --example bench_typed_pure
//!
//! Reading the result: B/A is the speedup of the typed-pure path over
//! Fulcrum's catch-all recheck (the thesis, fairly measured). C/A is the
//! honest reality check — if C ≈ A, a competent incremental balancer already
//! gets the cheap path without the type machinery, and the win is *not* raw
//! compute but gauge-agnosticism + correctness-by-construction.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

use fulcrum::{Capacity, Fleet, Gauge, HotToCold, Linfty, MachineId, Mass, SumTopK, WeightedKyFan};

const N: usize = 4;
/// Ky Fan order for the non-trivial gauges (sum of top-K worst-dim utils).
const K: usize = 8;
/// Safety threshold the recheck would compare against (unused by the witness).
const TAU: f64 = 0.99;

/// A fleet of `m` machines, uniform capacity, a utilization gradient on dim 0
/// from ~0.05 (coldest, id 1) to ~0.95 (hottest, id m). The gradient gives the
/// sort/multiset something realistic to chew on.
fn build_fleet(m: usize) -> Fleet<N> {
    let mut f = Fleet::new();
    for i in 0..m {
        // load0 spread across [50, 950] so worst-util spans [0.05, 0.95].
        let load0 = 50 + (i as u64 * 900) / (m as u64).max(1);
        f.add_machine(MachineId(i as u64 + 1), Capacity([1000; N]), Mass([load0, 0, 0, 0]));
    }
    f
}

/// Time `f` at `iters` iterations (with a 10% warmup), returning ns/op.
fn bench<F: FnMut()>(iters: u64, mut f: F) -> f64 {
    for _ in 0..(iters / 10).max(1) {
        f();
    }
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    t.elapsed().as_nanos() as f64 / iters as f64
}

/// An ordered multiset of per-machine worst-utilizations, keyed by the f64 bit
/// pattern (monotonic for non-negative finite f64). Supports the O(log M)
/// "incremental max" a competent balancer would maintain instead of re-evaluating.
struct IncrementalMax {
    counts: BTreeMap<u64, u32>,
}

impl IncrementalMax {
    fn from_fleet(fleet: &Fleet<N>) -> Self {
        let mut counts: BTreeMap<u64, u32> = BTreeMap::new();
        for (_, spec) in fleet.iter() {
            *counts.entry(spec.worst_utilization().to_bits()).or_insert(0) += 1;
        }
        Self { counts }
    }

    fn remove(&mut self, key: u64) {
        if let Some(c) = self.counts.get_mut(&key) {
            *c -= 1;
            if *c == 0 {
                self.counts.remove(&key);
            }
        }
    }

    fn insert(&mut self, key: u64) {
        *self.counts.entry(key).or_insert(0) += 1;
    }

    fn max(&self) -> f64 {
        self.counts
            .last_key_value()
            .map(|(k, _)| f64::from_bits(*k))
            .unwrap_or(0.0)
    }
}

fn main() {
    let sizes = [16usize, 64, 256, 1024, 4096, 16384, 65536];

    // One-time sanity: confirm the witness path we're timing actually succeeds
    // (so we measure the full PD check, not an early reject).
    {
        let f = build_fleet(1024);
        let ok = HotToCold::witness(MachineId(1024), MachineId(1), Mass([1, 0, 0, 0]), &f).is_some();
        let bad = HotToCold::witness(MachineId(1), MachineId(1024), Mass([1, 0, 0, 0]), &f).is_some();
        println!("# witness sanity: hot->cold admitted = {ok}, cold->hot admitted = {bad}");
        assert!(ok && !bad, "benchmark must time the witness success path");
    }

    println!(
        "# N={N}, K={K} (SumTopK/WeightedKyFan), tau={TAU}\n\
         # A=witness  B=full eval (= apply_with_recheck)  C=incremental max (steelman)\n\
         # all times ns/op; B/A = typed-pure speedup vs Fulcrum's own recheck\n"
    );
    println!(
        "{:>7} | {:>9} | {:>9} {:>9} {:>9} | {:>9} | {:>8} {:>8}",
        "M", "A:witness", "B:Linfty", "B:TopK", "B:WKyFan", "C:incr", "B/A", "C/A",
    );
    println!("{}", "-".repeat(86));

    let linfty = Linfty::<N>::default();
    let topk = SumTopK::<K, N>::default();
    let wkf = WeightedKyFan::<K, N>::new([1.0; K]).expect("uniform weights are valid");

    for &m in &sizes {
        let fleet = build_fleet(m);
        let hot = MachineId(m as u64); // hottest
        let cold = MachineId(1); // coldest
        let mass = Mass([1u64, 0, 0, 0]);

        // Cheap strategies: many iterations.
        let cheap_iters = 1_000_000u64;
        // Eval strategy: O(M log M) per op — scale iterations down with M to
        // bound wall time, then normalize to ns/op.
        let eval_iters = (40_000_000u64 / m as u64).clamp(300, 1_000_000);

        // A — the typed-pure witness (gauge-agnostic, no eval).
        let a = bench(cheap_iters, || {
            let r = HotToCold::witness(
                black_box(hot),
                black_box(cold),
                black_box(mass),
                black_box(&fleet),
            );
            black_box(r.is_some());
        });

        // B — full gauge eval (== apply_with_recheck), three gauges.
        let b_linfty = bench(eval_iters, || {
            black_box(black_box(&linfty).eval(black_box(&fleet)) <= TAU);
        });
        let b_topk = bench(eval_iters, || {
            black_box(black_box(&topk).eval(black_box(&fleet)) <= TAU * K as f64);
        });
        let b_wkf = bench(eval_iters, || {
            black_box(black_box(&wkf).eval(black_box(&fleet)) <= TAU * K as f64 * K as f64);
        });

        // C — incremental max maintenance (steelman). One move touches two
        // machines: remove their old keys, insert the new, read the max.
        let mut inc = IncrementalMax::from_fleet(&fleet);
        let k_hot = (0.95f64).to_bits();
        let k_cold = (0.05f64).to_bits();
        let c = bench(cheap_iters, || {
            // representative per-move cost: 2 removes + 2 inserts + 1 max query
            inc.remove(black_box(k_hot));
            inc.remove(black_box(k_cold));
            inc.insert(black_box(k_hot));
            inc.insert(black_box(k_cold));
            black_box(inc.max());
        });

        println!(
            "{:>7} | {:>9.1} | {:>9.1} {:>9.1} {:>9.1} | {:>9.1} | {:>8.0} {:>8.1}",
            m,
            a,
            b_linfty,
            b_topk,
            b_wkf,
            c,
            b_linfty / a,
            c / a,
        );
    }

    println!(
        "\n# Read: B/A is how much cheaper the typed-pure path is than Fulcrum's\n\
         # catch-all recheck (the thesis). C/A near 1.0 means a hand-maintained\n\
         # incremental gauge matches the witness on raw compute — so for cheap,\n\
         # separable gauges the win is gauge-agnosticism + correctness, not speed.\n\
         # B:TopK / B:WKyFan vs C: no O(log M) incremental is provided for those,\n\
         # so the witness is the only cheap, gauge-agnostic option there."
    );
}
