//! Benchmark #2 as a pinned test: the typed-pure ratio `p` is a property of the
//! *workload regime*, not a constant. Cold-start (placement-only) is the worst
//! case; steady-state churn lifts `p` because departures are typed-pure
//! `Remove`s and the holes they open give the rebalancer real `HotToCold` work.
//!
//! These assertions are deliberately about *direction and determinism*, not
//! exact magnitudes (those are reported by `examples/bench_rebalance_ratio.rs`),
//! so the test stays robust to tuning while still guarding the claim.

use fulcrum::{steady_state_churn, ChurnConfig};

fn base(departure_prob: f64) -> ChurnConfig {
    ChurnConfig {
        seed: 0xBEEF_F00D,
        machines: 96,
        capacity: 1000,
        fill: 1500,
        ticks: 400,
        departure_prob,
        max_mass: [80, 80, 80, 80],
        threshold: 0.90,
        // Realistic convergence: stop at <5% worst-util gap. (A tight epsilon
        // like 1e-3 makes the gap-halving rebalancer emit a flood of
        // near-worthless micro-migrations that inflate p — see
        // examples/bench_rebalance_ratio.rs, Part 2.)
        rebalance_epsilon: 0.05,
        rebalance_each_tick: true,
    }
}

#[test]
fn churn_is_deterministic() {
    let a = steady_state_churn(base(0.5)).unwrap();
    let b = steady_state_churn(base(0.5)).unwrap();
    assert!((a.typed_pure_ratio - b.typed_pure_ratio).abs() < 1e-12);
    assert_eq!(a.placements, b.placements);
    assert_eq!(a.removals, b.removals);
    assert_eq!(a.migrations, b.migrations);
}

#[test]
fn steady_state_churn_lifts_typed_pure_ratio() {
    let cold = steady_state_churn(base(0.0)).unwrap();
    let steady = steady_state_churn(base(0.5)).unwrap();

    // Cold-start (arrivals only) is placement-dominated: p stays low, like the
    // twin. Steady-state churn lifts it substantially.
    assert!(
        steady.typed_pure_ratio > cold.typed_pure_ratio + 0.1,
        "churn should lift p: cold={:.3} steady={:.3}",
        cold.typed_pure_ratio,
        steady.typed_pure_ratio,
    );
    // Steady-state should cross into typed-pure-substantial territory.
    assert!(
        steady.typed_pure_ratio > 0.3,
        "steady-state p unexpectedly low: {:.3}",
        steady.typed_pure_ratio,
    );
}
