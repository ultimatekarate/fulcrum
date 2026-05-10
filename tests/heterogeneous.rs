//! Phase 1 acceptance tests: heterogeneous capacity.
//!
//! These verify that the move algebra remains sound when machines have
//! different capacities. The witnesses now compare *utilization* (load /
//! capacity) instead of raw load, and `HotToCold` requires the source
//! capacity to be at most the destination capacity. The same set of moves
//! that were typed-pure under uniform capacity remain typed-pure when the
//! capacities are uniform; new heterogeneous-capacity scenarios are
//! exercised below.

use fulcrum::gauge::Gauge;
use fulcrum::{Fleet, HotToCold, Linfty, MachineId, Mass, Neutral, Safe, SumTopK};

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
    // Same load 80 on both machines, but capacities differ. Machine 1 is
    // hotter (80/100 = 0.80) than machine 2 (80/200 = 0.40).
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 80);
    f.add_machine(MachineId(2), 200, 80);

    assert!((f.utilization(MachineId(1)).unwrap() - 0.80).abs() < 1e-9);
    assert!((f.utilization(MachineId(2)).unwrap() - 0.40).abs() < 1e-9);

    // Linfty picks machine 1.
    assert!((Linfty::default().eval(&f) - 0.80).abs() < 1e-9);
}

#[test]
fn hot_to_cold_uses_utilization_not_load() {
    // Loads are equal (80) but utilizations differ. Machine 1 (cap 100)
    // is hotter than machine 2 (cap 200); a transfer 1 → 2 with mass 10
    // is typed-pure even though load_src == load_dst.
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 80);
    f.add_machine(MachineId(2), 200, 80);

    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass(10), &f);
    assert!(
        w.is_some(),
        "transfer from utilization 0.80 to 0.40 must be typed-pure"
    );

    // Reverse direction (cold → hot on utilization) must be rejected.
    let w_rev = HotToCold::witness(MachineId(2), MachineId(1), Mass(10), &f);
    assert!(
        w_rev.is_none(),
        "transfer from utilization 0.40 to 0.80 must be rejected"
    );
}

#[test]
fn hot_to_cold_rejects_high_to_low_capacity() {
    // util(src) > util(dst) but cap(src) > cap(dst). The transfer can
    // increase top-k sums under SumTopK<2>; reject as typed-pure.
    //
    // Concrete: caps (100, 10), loads (80, 5). utils (0.80, 0.50).
    // Transfer mass 1: new utils (0.79, 0.60). SumTopK<2>:
    //   before = 0.80 + 0.50 = 1.30
    //   after  = 0.79 + 0.60 = 1.39   (INCREASED)
    // The witness must refuse.
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 80);
    f.add_machine(MachineId(2), 10, 5);

    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass(1), &f);
    assert!(
        w.is_none(),
        "high-to-low-capacity transfer must be rejected even when util(src) > util(dst)"
    );
}

#[test]
fn cap_src_le_cap_dst_restriction_is_necessary_for_soundness() {
    // The witness's `cap(src) ≤ cap(dst)` restriction is necessary for
    // soundness, not just a conservative choice. Same setup as
    // `hot_to_cold_rejects_high_to_low_capacity`, but here we additionally
    // verify the *consequence* of admitting the transfer: the resulting
    // fleet would actually exceed the gauge threshold. This is the test
    // that pins the witness's rejection to a concrete unsoundness.
    //
    // Caps (100, 10), loads (80, 5). utils (0.80, 0.50). SumTopK<2> = 1.30.
    // Transfer mass 1 from src=1 to dst=2:
    //   new utils (79/100, 6/10) = (0.79, 0.60), SumTopK<2> = 1.39.
    let mut before = Fleet::new();
    before.add_machine(MachineId(1), 100, 80);
    before.add_machine(MachineId(2), 10, 5);

    let mut after = Fleet::new();
    after.add_machine(MachineId(1), 100, 79);
    after.add_machine(MachineId(2), 10, 6);

    let g = SumTopK::<2>::default();
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

    // The framework correctly refuses to type this transfer as safe.
    assert!(
        HotToCold::witness(MachineId(1), MachineId(2), Mass(1), &before).is_none(),
        "witness must reject the unsound transfer"
    );

    // Pin the threshold tight enough to expose the violation: the pre-
    // transfer fleet is admissible at threshold 1.35; the post-transfer
    // fleet is not. If the witness ever started admitting this transfer,
    // a `Safe<SumTopK<2>>` constructed at 1.35 would carry a false claim
    // after the apply.
    assert!(Safe::<SumTopK<2>>::new(before, 1.35).is_ok());
    assert!(matches!(
        Safe::<SumTopK<2>>::new(after, 1.35),
        Err(fulcrum::GaugeError::ThresholdExceeded { .. })
    ));
}

#[test]
fn hot_to_cold_collapses_to_v0_under_uniform_capacity() {
    // When all capacities are equal, the new witness condition reduces to
    // the v0 rule: mass ≤ load(src) − load(dst). Verify by exercising the
    // boundary cases.
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 80);
    f.add_machine(MachineId(2), 100, 30);

    // Boundary: mass exactly equals the load gap.
    assert!(
        HotToCold::witness(MachineId(1), MachineId(2), Mass(50), &f).is_some(),
        "mass = load_src - load_dst must pass under uniform capacity"
    );
    // Just over the gap.
    assert!(
        HotToCold::witness(MachineId(1), MachineId(2), Mass(51), &f).is_none(),
        "mass > load_src - load_dst must fail under uniform capacity"
    );
    // Equal load (no Pigou-Dalton direction).
    let mut g = Fleet::new();
    g.add_machine(MachineId(1), 100, 50);
    g.add_machine(MachineId(2), 100, 50);
    assert!(
        HotToCold::witness(MachineId(1), MachineId(2), Mass(1), &g).is_none(),
        "equal utilization must reject HotToCold"
    );
}

#[test]
fn hot_to_cold_admits_more_mass_when_destination_is_larger() {
    // caps (100, 1000), loads (80, 100). utils (0.80, 0.10), gap 0.70.
    // V0 rule (load gap) would cap mass at 80 - 100 = (negative) — would
    // forbid the transfer entirely.
    // Phase 1 rule: mass ≤ cap_dst · gap = 1000 · 0.70 = 700. But also
    // load conservation: mass ≤ load_src = 80. So max admissible is 80.
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 80);
    f.add_machine(MachineId(2), 1000, 100);

    // Transfer 80: new u_src = 0, new u_dst = 180/1000 = 0.18.
    // Both within [0, 0.80]; weak-super-majorization holds.
    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass(80), &f);
    assert!(w.is_some(), "transfer to a larger machine should admit larger mass");
}

#[test]
fn hot_to_cold_apply_preserves_safety_with_heterogeneous_caps() {
    // Apply the move and verify the gauge stays under threshold.
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 80);   // util 0.80
    f.add_machine(MachineId(2), 200, 80);   // util 0.40
    f.add_machine(MachineId(3), 100, 30);   // util 0.30

    let safe: Safe<Linfty> = Safe::new(f, 0.85).unwrap();
    let g_before = safe.gauge();

    // Transfer 1 → 2 (cap_src=100 ≤ cap_dst=200) with mass 20.
    // New u_src = 60/100 = 0.60. New u_dst = 100/200 = 0.50.
    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass(20), safe.fleet()).unwrap();
    let safe = m.apply(safe);

    assert_le(safe.gauge(), 0.85, "Linfty within threshold after transfer");
    assert_le(safe.gauge(), g_before, "gauge non-increasing");
}

#[test]
fn hot_to_cold_apply_preserves_sumtopk_with_heterogeneous_caps() {
    // Same scenario, SumTopK<2>. The weak-super-majorization argument
    // says no top-k sum increases; verify on a concrete fleet.
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 80);   // util 0.80
    f.add_machine(MachineId(2), 200, 80);   // util 0.40
    f.add_machine(MachineId(3), 100, 30);   // util 0.30

    let g2 = SumTopK::<2>::default();
    let before = g2.eval(&f);  // 0.80 + 0.40 = 1.20

    let safe: Safe<SumTopK<2>> = Safe::new(f, 1.30).unwrap();
    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass(20), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    let after = safe.gauge();  // top-2 of (0.60, 0.50, 0.30) = 1.10

    assert_le(after, before, "SumTopK<2> non-increasing");
    assert_le(after, 1.30, "SumTopK<2> within threshold");
}

#[test]
fn neutral_admits_only_equal_capacity() {
    // Equal utilization but different capacities → reject.
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 50);   // util 0.50
    f.add_machine(MachineId(2), 200, 100);  // util 0.50
    assert!(
        Neutral::witness(MachineId(1), MachineId(2), Mass(10), &f).is_none(),
        "Neutral must require equal capacity"
    );

    // Equal capacity AND equal load → admit.
    let mut g = Fleet::new();
    g.add_machine(MachineId(1), 100, 50);
    g.add_machine(MachineId(2), 100, 50);
    assert!(
        Neutral::witness(MachineId(1), MachineId(2), Mass(10), &g).is_some(),
        "Neutral must admit equal-capacity equal-load pair"
    );
}

#[test]
fn unknown_machine_returns_none() {
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 80);
    assert!(HotToCold::witness(MachineId(1), MachineId(99), Mass(10), &f).is_none());
    assert!(HotToCold::witness(MachineId(99), MachineId(1), Mass(10), &f).is_none());
    assert!(Neutral::witness(MachineId(1), MachineId(99), Mass(10), &f).is_none());
}

#[test]
fn zero_capacity_machine_rejects_witness() {
    // A machine with capacity 0 has infinite utilization. Even if util(src)
    // > util(dst) holds in the limit, the witness rejects to avoid
    // operating on a divide-by-zero coordinate.
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 0, 0);
    f.add_machine(MachineId(2), 100, 50);
    assert!(HotToCold::witness(MachineId(2), MachineId(1), Mass(1), &f).is_none());
    assert!(HotToCold::witness(MachineId(1), MachineId(2), Mass(1), &f).is_none());
}

#[test]
fn long_chain_of_heterogeneous_transfers_stays_safe() {
    // Build a heterogeneous fleet, apply a sequence of typed-pure moves,
    // verify the gauge stays under threshold throughout. This is the
    // structural composition guarantee at the heart of the framework, now
    // exercised under heterogeneous capacity.
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), 100, 80);    // util 0.80
    f.add_machine(MachineId(2), 200, 60);    // util 0.30
    f.add_machine(MachineId(3), 400, 80);    // util 0.20
    f.add_machine(MachineId(4), 100, 80);    // util 0.80

    let safe: Safe<Linfty> = Safe::new(f, 0.85).unwrap();

    // 1 → 2 (cap 100 → 200): admissible.
    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass(10), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    assert_le(safe.gauge(), 0.85, "after 1→2");

    // 4 → 3 (cap 100 → 400): admissible.
    let m = HotToCold::witness(MachineId(4), MachineId(3), Mass(15), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    assert_le(safe.gauge(), 0.85, "after 4→3");

    // 1 → 3 (cap 100 → 400): admissible.
    let m = HotToCold::witness(MachineId(1), MachineId(3), Mass(20), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    assert_le(safe.gauge(), 0.85, "after 1→3");
}
