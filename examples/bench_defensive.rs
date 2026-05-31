//! Benchmark #3: the typed-pure witness vs a *defensive conventional* baseline.
//!
//! The "correctness implies performance" claim: a conventional balancer that
//! lacks the `Safe<G,N>` proof must re-establish `g(fleet) ≤ τ` at runtime, and
//! Fulcrum elides exactly that work. This prices it — honestly, with two
//! conventional baselines, naive and smart, so it can't be dismissed as a
//! strawman.
//!
//! Per typed-pure (`HotToCold`) move, the *safety validation* each path does
//! (the physical load mutation is identical in all three and excluded):
//!
//!   A. WITNESS         — `HotToCold::witness(..)`: a local, gauge-agnostic
//!                        Pigou-Dalton check. Proves the gauge can't rise for
//!                        ANY Schur-convex gauge, without evaluating one.  ── O(log M + N)
//!   B. NAIVE DEFENSIVE — the only safe option without a proof: snapshot the
//!                        fleet (to roll back), apply, re-evaluate the gauge,
//!                        commit-or-revert. This is literally Fulcrum's OWN
//!                        catch-all `Place` pattern (the projection-clone in
//!                        `Sim::apply`), now forced onto every move.  ── O(M) clone + O(M) eval
//!   C. SMART DEFENSIVE — a competent untyped coder checks only the two touched
//!                        machines against τ (cheap undo-log, no clone). Fast —
//!                        but Linfty-ONLY (a local check can't see a top-K sum),
//!                        and it carries no proof: it verifies τ for one gauge,
//!                        not gauge-non-increase for all.  ── O(N)
//!
//!     cargo run --release --example bench_defensive

use std::hint::black_box;
use std::time::Instant;

use fulcrum::{Capacity, Fleet, Gauge, HotToCold, Linfty, MachineId, Mass};

const N: usize = 4;
const TAU: f64 = 0.99;

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

fn build_fleet(m: usize) -> Fleet<N> {
    let mut f = Fleet::new();
    for i in 0..m {
        let load0 = 50 + (i as u64 * 900) / (m as u64).max(1);
        f.add_machine(MachineId(i as u64 + 1), Capacity([1000; N]), Mass([load0, 0, 0, 0]));
    }
    f
}

/// The smart conventional coder's local safety check: a `HotToCold` only touches
/// `src` and `dst`, so under Linfty only `dst`'s utilization can newly breach τ
/// (and `src` must not underflow). O(N), no clone, no global scan — but valid
/// *only* for a separable (max-style) gauge, and it proves nothing beyond "this
/// one gauge is ≤ τ for this one move".
fn local_safe(fleet: &Fleet<N>, src: MachineId, dst: MachineId, mass: Mass<N>) -> bool {
    let (s, d) = match (fleet.spec(src), fleet.spec(dst)) {
        (Some(s), Some(d)) => (s, d),
        _ => return false,
    };
    for k in 0..N {
        if s.load[k] < mass[k] {
            return false; // source underflow
        }
        if (d.load[k] + mass[k]) as f64 > TAU * d.capacity[k] as f64 {
            return false; // destination would breach τ
        }
    }
    true
}

fn main() {
    let sizes = [16usize, 64, 256, 1024, 4096, 16384, 65536];
    let g = Linfty::<N>::default();

    println!("# N={N}, tau={TAU}. Per-move SAFETY-VALIDATION cost (apply excluded; identical to all).");
    println!("# A=witness (proof)  B=naive defensive (clone+recheck)  C=smart defensive (local, Linfty-only)\n");
    println!(
        "{:>7} | {:>9} | {:>9} {:>9} {:>11} | {:>9} | {:>7} {:>6}",
        "M", "A:witness", "clone", "eval", "B:clone+eval", "C:local", "B/A", "C/A",
    );
    println!("{}", "-".repeat(82));

    for &m in &sizes {
        let fleet = build_fleet(m);
        let (hot, cold, mass) = (MachineId(m as u64), MachineId(1), Mass([1u64, 0, 0, 0]));

        let cheap = 1_000_000u64;
        let evals = (40_000_000u64 / m as u64).clamp(300, 1_000_000);

        // A — Fulcrum's witness: the whole safety argument, locally, no eval.
        let a = bench(cheap, || {
            let r = HotToCold::witness(black_box(hot), black_box(cold), black_box(mass), black_box(&fleet));
            black_box(r.is_some());
        });

        // B — naive defensive: snapshot (rollback buffer) + global recheck.
        let clone_t = bench(evals, || {
            black_box(black_box(&fleet).clone());
        });
        let eval_t = bench(evals, || {
            black_box(black_box(&g).eval(black_box(&fleet)) <= TAU);
        });
        let b = clone_t + eval_t;

        // C — smart defensive: local τ-check on the two touched machines.
        let c = bench(cheap, || {
            black_box(local_safe(black_box(&fleet), black_box(hot), black_box(cold), black_box(mass)));
        });

        println!(
            "{:>7} | {:>9.1} | {:>9.1} {:>9.1} {:>11.1} | {:>9.1} | {:>7.0} {:>6.1}",
            m, a, clone_t, eval_t, b, c, b / a, c / a,
        );
    }

    println!(
        "\n# vs NAIVE defensive (B): Fulcrum elides an O(M) snapshot + an O(M) gauge\n\
         #   recheck on every typed-pure move — the simulate-and-verify a proof-less\n\
         #   system is forced into. This is the measured 'correctness => performance'.\n\
         # vs SMART defensive (C): a ~tie on speed. The witness's edge here is NOT\n\
         #   cycles — it's that C is Linfty-only (no local check exists for a top-K\n\
         #   sum: you'd be back to B) and proof-free (checks τ for one gauge, not\n\
         #   gauge-non-increase for all). So against a competent untyped coder the\n\
         #   win is correctness + gauge-agnosticism; against a cautious one, it's\n\
         #   also raw compute. Either way C buys its speed by re-deriving the\n\
         #   witness by hand, unverified."
    );
}
