//! Benchmark #2: does the typed-pure ratio `p` climb off 0.12 when the workload
//! is churn-/rebalance-dominated — and is `p` even an honest figure of merit?
//!
//! Benchmark #1 fixed the *per-move* economics: typed-pure costs A (a local
//! witness), the catch-all recheck costs B (a full gauge eval), B/A ≈ 12x..37000x.
//! The *realized* system speedup is `B / (p·A + (1−p)·B)`, gated by `p` = the
//! fraction of applied moves that are typed-pure. The cold-start twin sits at
//! p ≈ 0.12 (24 placements, ~4 migrations, no departures) — its worst case.
//!
//! Part 1 drives a populated fleet through steady-state churn (arrivals = `Place`,
//! catch-all; departures = `Remove`, typed-pure; rebalance = `HotToCold`,
//! typed-pure) with a *realistic* convergence epsilon, and shows `p` rising with
//! the departure rate — decomposed into removals (jobs finishing) vs migrations
//! (rebalancing), so the lift isn't mistaken for one when it's the other.
//!
//! Part 2 is the catch. `p` is **gameable**: because the rebalancer halves the
//! gap each move, tightening epsilon makes it emit a flood of near-worthless
//! micro-migrations that drive `p` → 1.0 while the load gauge barely moves. The
//! realized-speedup formula rewards that — it counts recheck cost "saved" on
//! moves you should not be making. So `p` alone is not the figure of merit;
//! useful work must be held fixed.
//!
//!     cargo run --release --example bench_rebalance_ratio

use std::hint::black_box;
use std::time::Instant;

use fulcrum::{
    steady_state_churn, Capacity, ChurnConfig, Fleet, Gauge, HotToCold, Linfty, MachineId, Mass,
};

const N: usize = 4;

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

/// Per-move costs A (witness) and B (full Linfty eval) on a synthetic m-machine
/// fleet — same method as benchmark #1 — so realized speedups use costs
/// consistent with the fleet size.
fn measure_a_b(m: usize) -> (f64, f64) {
    let mut fleet: Fleet<N> = Fleet::new();
    for i in 0..m {
        let load0 = 50 + (i as u64 * 900) / (m as u64).max(1);
        fleet.add_machine(MachineId(i as u64 + 1), Capacity([1000; N]), Mass([load0, 0, 0, 0]));
    }
    let (hot, cold, mass) = (MachineId(m as u64), MachineId(1), Mass([1u64, 0, 0, 0]));
    let g = Linfty::<N>::default();
    let a = bench(1_000_000, || {
        let r = HotToCold::witness(black_box(hot), black_box(cold), black_box(mass), black_box(&fleet));
        black_box(r.is_some());
    });
    let b = bench(200_000, || {
        black_box(black_box(&g).eval(black_box(&fleet)) <= 0.99);
    });
    (a, b)
}

/// Realized recheck-work speedup vs a baseline that re-evaluates the gauge after
/// every applied move: `B / (p·A + (1−p)·B)`.
fn realized_speedup(p: f64, a: f64, b: f64) -> f64 {
    b / (p * a + (1.0 - p) * b)
}

fn main() {
    // ---- Part 1: p vs churn regime, realistic epsilon ----
    const M1: usize = 128;
    let (a, b) = measure_a_b(M1);
    // bench #1 @ M=65536, post gauge-eval optimization (select-not-sort).
    // The realized speedup -> 1/(1-p) once A << B, so the exact B barely
    // matters here; kept consistent with bench_typed_pure all the same.
    let (a_big, b_big) = (26.0_f64, 372_530.0_f64);
    println!("# Part 1 fleet M={M1}: A=witness {a:.1}ns  B=eval {b:.1}ns  B/A={:.0}", b / a);
    println!("# epsilon=0.05 (stop at <5% worst-util gap -> migrations are meaningful)\n");
    println!(
        "{:>8} | {:>5} | {:>7} {:>7} {:>8} | {:>6} {:>6} | {:>6} | {:>10} {:>11}",
        "dep_prob", "p", "place", "remove", "migrate", "rem%", "mig%", "gauge", "spdup@128", "spdup@65k",
    );
    println!("{}", "-".repeat(96));
    for dep in [0.0, 0.1, 0.2, 0.3, 0.4, 0.5] {
        let cfg = ChurnConfig {
            seed: 0xBEEF_F00D,
            machines: M1,
            capacity: 1000,
            max_mass: [80, 80, 80, 80],
            fill: 2000,
            ticks: 1000,
            departure_prob: dep,
            threshold: 0.90,
            rebalance_epsilon: 0.05,
            rebalance_each_tick: true,
        };
        let r = steady_state_churn(cfg).expect("empty fleet is under threshold");
        let applied = (r.removals + r.migrations + r.placements).max(1) as f64;
        let p = r.typed_pure_ratio;
        println!(
            "{:>8.1} | {:>5.2} | {:>7} {:>7} {:>8} | {:>5.0}% {:>5.0}% | {:>6.3} | {:>10.2} {:>11.2}",
            dep,
            p,
            r.placements,
            r.removals,
            r.migrations,
            100.0 * r.removals as f64 / applied,
            100.0 * r.migrations as f64 / applied,
            r.final_gauge,
            realized_speedup(p, a, b),
            realized_speedup(p, a_big, b_big),
        );
    }

    // ---- Part 2: p is gameable — tighten epsilon, inflate p, gauge stays flat ----
    const M2: usize = 64;
    println!("\n# Part 2 fleet M={M2}: fixed churn dep=0.3, sweep rebalance epsilon.");
    println!("# Watch p climb toward 1.0 while the gauge barely moves: the extra");
    println!("# migrations are near-worthless precision-chasing.\n");
    println!(
        "{:>8} | {:>5} | {:>8} {:>9} | {:>6} | {:>10}",
        "epsilon", "p", "migrate", "mig/tick", "gauge", "spdup@128*",
    );
    println!("{}", "-".repeat(64));
    let ticks2 = 300usize;
    for eps in [0.10, 0.05, 0.02, 0.01, 0.005] {
        let cfg = ChurnConfig {
            seed: 0xBEEF_F00D,
            machines: M2,
            capacity: 1000,
            max_mass: [80, 80, 80, 80],
            fill: 1000,
            ticks: ticks2,
            departure_prob: 0.3,
            threshold: 0.90,
            rebalance_epsilon: eps,
            rebalance_each_tick: true,
        };
        let r = steady_state_churn(cfg).expect("empty fleet is under threshold");
        let p = r.typed_pure_ratio;
        println!(
            "{:>8.3} | {:>5.2} | {:>8} {:>9.1} | {:>6.3} | {:>10.2}",
            eps,
            p,
            r.migrations,
            r.migrations as f64 / ticks2 as f64,
            r.final_gauge,
            // *uses Part-1 costs; the point is the formula INFLATES with p even
            // though the gauge (useful work) is flat.
            realized_speedup(p, a, b),
        );
    }

    println!(
        "\n# Part 1: p genuinely rises with churn — but read rem%/mig%: most of the\n\
         #   lift is Removes (jobs finishing), which are typed-pure for free.\n\
         # Part 2: p is gameable. Tightening epsilon pushes p -> ~1.0 and the\n\
         #   'speedup' with it, while the Linfty gauge is flat -- those migrations\n\
         #   relieve sub-dominant machines the gauge can't even see. So 'fraction\n\
         #   of moves that are typed-pure' is NOT a clean figure of merit; the\n\
         #   honest question is recheck cost saved per unit of USEFUL balancing."
    );
}
