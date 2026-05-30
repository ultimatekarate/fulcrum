//! Phase 1 acceptance tests: heterogeneous capacity (1D).
//!
//! These verify that the move algebra remains sound when machines have
//! different capacities. The Phase 2 multi-dim lift instantiates these
//! at `N = 1`. The Phase 1 properties — utilization-based comparison,
//! `cap(src) ≤ cap(dst)` restriction, equal-capacity-required Neutral —
//! are special cases of the per-dimension witness conditions enforced
//! in Phase 2.

use fulcrum::gauge::Gauge;
use fulcrum::{Capacity, Fleet, HotToCold, Linfty, MachineId, Mass, Neutral, Safe, SumTopK};

fn assert_le(actual: f64, threshold: f64, msg: &str) {
    assert!(
        actual <= threshold + 1e-9,
        "{}: {} > {}",
        msg,
        actual,
        threshold
    );
}

#[test]
fn utilization_compares_via_per_machine_capacity() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    f.add_machine(MachineId(2), Capacity([200]), Mass([80]));

    assert!((f.utilization(MachineId(1)).unwrap()[0] - 0.80).abs() < 1e-9);
    assert!((f.utilization(MachineId(2)).unwrap()[0] - 0.40).abs() < 1e-9);

    assert!((Linfty::<1>::default().eval(&f) - 0.80).abs() < 1e-9);
}

#[test]
fn hot_to_cold_uses_utilization_not_load() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    f.add_machine(MachineId(2), Capacity([200]), Mass([80]));

    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass([10]), &f);
    assert!(
        w.is_some(),
        "transfer from utilization 0.80 to 0.40 must be typed-pure"
    );

    let w_rev = HotToCold::witness(MachineId(2), MachineId(1), Mass([10]), &f);
    assert!(
        w_rev.is_none(),
        "transfer from utilization 0.40 to 0.80 must be rejected"
    );
}

#[test]
fn hot_to_cold_rejects_high_to_low_capacity() {
    // util(src) > util(dst) but cap(src) > cap(dst). The transfer can
    // increase top-k sums under SumTopK<2>; reject as typed-pure.
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    f.add_machine(MachineId(2), Capacity([10]), Mass([5]));

    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass([1]), &f);
    assert!(
        w.is_none(),
        "high-to-low-capacity transfer must be rejected even when util(src) > util(dst)"
    );
}

#[test]
fn cap_src_le_cap_dst_restriction_is_necessary_for_soundness() {
    // The witness's `cap(src) ≤ cap(dst)` restriction is necessary for
    // soundness, not just a conservative choice. The post-transfer fleet
    // would actually exceed the gauge threshold; the witness's rejection
    // is pinned to a concrete unsoundness.
    let mut before: Fleet<1> = Fleet::new();
    before.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    before.add_machine(MachineId(2), Capacity([10]), Mass([5]));

    let mut after: Fleet<1> = Fleet::new();
    after.add_machine(MachineId(1), Capacity([100]), Mass([79]));
    after.add_machine(MachineId(2), Capacity([10]), Mass([6]));

    let g = SumTopK::<2, 1>::default();
    let g_before = g.eval(&before);
    let g_after = g.eval(&after);
    assert!((g_before - 1.30).abs() < 1e-9, "expected gauge 1.30, got {}", g_before);
    assert!((g_after - 1.39).abs() < 1e-9, "expected gauge 1.39, got {}", g_after);
    assert!(
        g_after > g_before,
        "the unsoundness scenario: gauge increases on this transfer ({} > {})",
        g_after,
        g_before
    );

    assert!(
        HotToCold::witness(MachineId(1), MachineId(2), Mass([1]), &before).is_none(),
        "witness must reject the unsound transfer"
    );

    assert!(Safe::<SumTopK<2, 1>, 1>::new(before, 1.35).is_ok());
    assert!(matches!(
        Safe::<SumTopK<2, 1>, 1>::new(after, 1.35),
        Err(fulcrum::GaugeError::ThresholdExceeded { .. })
    ));
}

#[test]
fn hot_to_cold_collapses_to_v0_under_uniform_capacity() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    f.add_machine(MachineId(2), Capacity([100]), Mass([30]));

    assert!(
        HotToCold::witness(MachineId(1), MachineId(2), Mass([50]), &f).is_some(),
        "mass = load_src - load_dst must pass under uniform capacity"
    );
    assert!(
        HotToCold::witness(MachineId(1), MachineId(2), Mass([51]), &f).is_none(),
        "mass > load_src - load_dst must fail under uniform capacity"
    );
    let mut g: Fleet<1> = Fleet::new();
    g.add_machine(MachineId(1), Capacity([100]), Mass([50]));
    g.add_machine(MachineId(2), Capacity([100]), Mass([50]));
    assert!(
        HotToCold::witness(MachineId(1), MachineId(2), Mass([1]), &g).is_none(),
        "equal utilization must reject HotToCold"
    );
}

#[test]
fn hot_to_cold_admits_more_mass_when_destination_is_larger() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    f.add_machine(MachineId(2), Capacity([1000]), Mass([100]));

    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass([80]), &f);
    assert!(w.is_some(), "transfer to a larger machine should admit larger mass");
}

#[test]
fn hot_to_cold_apply_preserves_safety_with_heterogeneous_caps() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    f.add_machine(MachineId(2), Capacity([200]), Mass([80]));
    f.add_machine(MachineId(3), Capacity([100]), Mass([30]));

    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.85).unwrap();
    let g_before = safe.gauge();

    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([20]), safe.fleet()).unwrap();
    let safe = m.apply(safe);

    assert_le(safe.gauge(), 0.85, "Linfty within threshold after transfer");
    assert_le(safe.gauge(), g_before, "gauge non-increasing");
}

#[test]
fn hot_to_cold_apply_preserves_sumtopk_with_heterogeneous_caps() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    f.add_machine(MachineId(2), Capacity([200]), Mass([80]));
    f.add_machine(MachineId(3), Capacity([100]), Mass([30]));

    let g2 = SumTopK::<2, 1>::default();
    let before = g2.eval(&f);

    let safe: Safe<SumTopK<2, 1>, 1> = Safe::new(f, 1.30).unwrap();
    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([20]), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    let after = safe.gauge();

    assert_le(after, before, "SumTopK<2> non-increasing");
    assert_le(after, 1.30, "SumTopK<2> within threshold");
}

#[test]
fn neutral_admits_only_equal_capacity() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([50]));
    f.add_machine(MachineId(2), Capacity([200]), Mass([100]));
    assert!(
        Neutral::witness(MachineId(1), MachineId(2), Mass([10]), &f).is_none(),
        "Neutral must require equal capacity"
    );

    let mut g: Fleet<1> = Fleet::new();
    g.add_machine(MachineId(1), Capacity([100]), Mass([50]));
    g.add_machine(MachineId(2), Capacity([100]), Mass([50]));
    assert!(
        Neutral::witness(MachineId(1), MachineId(2), Mass([10]), &g).is_some(),
        "Neutral must admit equal-capacity equal-load pair"
    );
}

#[test]
fn unknown_machine_returns_none() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    assert!(HotToCold::witness(MachineId(1), MachineId(99), Mass([10]), &f).is_none());
    assert!(HotToCold::witness(MachineId(99), MachineId(1), Mass([10]), &f).is_none());
    assert!(Neutral::witness(MachineId(1), MachineId(99), Mass([10]), &f).is_none());
}

#[test]
fn zero_capacity_machine_rejects_witness() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([0]), Mass([0]));
    f.add_machine(MachineId(2), Capacity([100]), Mass([50]));
    assert!(HotToCold::witness(MachineId(2), MachineId(1), Mass([1]), &f).is_none());
    assert!(HotToCold::witness(MachineId(1), MachineId(2), Mass([1]), &f).is_none());
}

#[test]
fn long_chain_of_heterogeneous_transfers_stays_safe() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    f.add_machine(MachineId(2), Capacity([200]), Mass([60]));
    f.add_machine(MachineId(3), Capacity([400]), Mass([80]));
    f.add_machine(MachineId(4), Capacity([100]), Mass([80]));

    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.85).unwrap();

    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([10]), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    assert_le(safe.gauge(), 0.85, "after 1→2");

    let m = HotToCold::witness(MachineId(4), MachineId(3), Mass([15]), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    assert_le(safe.gauge(), 0.85, "after 4→3");

    let m = HotToCold::witness(MachineId(1), MachineId(3), Mass([20]), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    assert_le(safe.gauge(), 0.85, "after 1→3");
}
