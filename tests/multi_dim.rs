//! Phase 2 acceptance tests: multi-dimensional load.
//!
//! The `Fleet<N>`, `Mass<N>`, `Safe<G, N>` types lift the framework to
//! `N` resource dimensions (CPU, memory, …). Witness conditions become
//! per-dimension: a transfer is typed-pure iff it satisfies the Phase 1
//! conditions in *every* dimension where mass moves.
//!
//! These tests pin the load-bearing properties:
//!
//! - 2D Pigou-Dalton transfers admissible in both dims are typed-pure.
//! - Transfers admissible in one dim but anti-Robin-Hood in another are
//!   *rejected* by the witness (this is the key Phase 2 test).
//! - The component-wise gauge interpretation (per-machine worst-dim,
//!   then top-K) is non-increasing across typed-pure moves.
//! - Sparse mass vectors (zero in some dims) are evaluated dim-by-dim;
//!   only dims with non-zero mass contribute conditions.

use fulcrum::gauge::Gauge;
use fulcrum::{
    Capacity, ColdToHot, Fleet, HotToCold, Linfty, MachineId, Mass, Neutral, Place, Remove, Safe,
    SumTopK, Utilization,
};

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
fn two_dim_transfer_typed_pure_when_dominance_holds_in_both_dims() {
    // Source dominates dest in both dimensions:
    //   m1: caps (100, 100), loads (80, 70). utils (0.80, 0.70).
    //   m2: caps (100, 100), loads (30, 20). utils (0.30, 0.20).
    // Transfer Mass([10, 5]):
    //   m1 → utils (0.70, 0.65). m2 → utils (0.40, 0.25).
    // Per-dim Pigou-Dalton in both. Typed-pure.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 70]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([30, 20]));

    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass([10, 5]), &f);
    assert!(w.is_some(), "transfer dominant in both dims must be typed-pure");
}

#[test]
fn two_dim_transfer_rejected_when_one_dim_is_anti_robin_hood() {
    // m1 hotter than m2 in dim 0, but m2 hotter than m1 in dim 1. A
    // transfer that moves mass in dim 1 from m1 → m2 is anti-Robin-Hood
    // in dim 1 — rejected by the witness, even though it's Pigou-Dalton
    // in dim 0.
    //
    //   m1: caps (100, 100), loads (80, 20). utils (0.80, 0.20).
    //   m2: caps (100, 100), loads (30, 70). utils (0.30, 0.70).
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 20]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([30, 70]));

    // Mass that moves in both dims: dim 0 is rich-to-poor, dim 1 is
    // poor-to-rich. Witness must reject.
    let w_both = HotToCold::witness(MachineId(1), MachineId(2), Mass([10, 5]), &f);
    assert!(
        w_both.is_none(),
        "transfer that's anti-Robin-Hood in any dim must be rejected"
    );

    // Mass concentrated in dim 0 only (dim 1 mass = 0). Now the rich-poor
    // direction in dim 1 doesn't matter — no transfer in that dim.
    let w_d0 = HotToCold::witness(MachineId(1), MachineId(2), Mass([10, 0]), &f);
    assert!(
        w_d0.is_some(),
        "transfer with mass=0 in the conflicting dim must be admitted"
    );
}

#[test]
fn two_dim_unsoundness_pinned_to_concrete_gauge_violation() {
    // The witness's per-dim cap restriction is necessary, not curatorial.
    // Same shape as the Phase 1 cap_src_le_cap_dst test, lifted to 2D:
    // dim 0 has cap_src > cap_dst, so the dim-0 transfer can lift the
    // gauge. Verify the post-transfer fleet exceeds the threshold.
    let mut before: Fleet<2> = Fleet::new();
    before.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 30]));
    before.add_machine(MachineId(2), Capacity([10, 100]), Mass([5, 30]));

    // After transferring Mass([1, 0]) (dim 0 only):
    //   m1: loads (79, 30) → utils (0.79, 0.30) → worst 0.79
    //   m2: loads (6, 30)  → utils (0.60, 0.30) → worst 0.60
    let mut after: Fleet<2> = Fleet::new();
    after.add_machine(MachineId(1), Capacity([100, 100]), Mass([79, 30]));
    after.add_machine(MachineId(2), Capacity([10, 100]), Mass([6, 30]));

    let g = SumTopK::<2, 2>::default();
    let g_before = g.eval(&before);
    let g_after = g.eval(&after);
    assert!((g_before - 1.30).abs() < 1e-9, "expected 1.30, got {}", g_before);
    assert!((g_after - 1.39).abs() < 1e-9, "expected 1.39, got {}", g_after);
    assert!(g_after > g_before);

    assert!(
        HotToCold::witness(MachineId(1), MachineId(2), Mass([1, 0]), &before).is_none(),
        "witness must reject the high-to-low-cap dim-0 transfer"
    );
}

#[test]
fn two_dim_apply_preserves_safety() {
    // End-to-end: apply a typed-pure 2D transfer and verify the gauge
    // bound holds.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 70]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([30, 20]));
    f.add_machine(MachineId(3), Capacity([100, 100]), Mass([40, 50]));

    let safe: Safe<Linfty<2>, 2> = Safe::new(f, 0.85).unwrap();
    let g_before = safe.gauge();

    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([10, 5]), safe.fleet()).unwrap();
    let safe = m.apply(safe);

    assert_le(safe.gauge(), 0.85, "Linfty within threshold");
    assert_le(safe.gauge(), g_before, "non-increasing");
}

#[test]
fn place_in_multi_dim_can_violate_threshold() {
    // Per-machine worst-dim is what matters. A `Place` that pushes any
    // dim over threshold gets rejected.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 50]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([30, 30]));

    let safe: Safe<Linfty<2>, 2> = Safe::new(f, 0.85).unwrap();

    // Place that lifts dim 0 over threshold (80 + 10 = 90 → util 0.90).
    let p = Place::new(MachineId(1), Mass([10, 0]));
    let result = p.apply_with_recheck(safe);
    assert!(matches!(
        result,
        Err(fulcrum::GaugeError::ThresholdExceeded { .. })
    ));
}

#[test]
fn place_in_quiet_dim_does_not_violate() {
    // Place that only adds in a dim where there's headroom.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 30]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([30, 30]));

    let safe: Safe<Linfty<2>, 2> = Safe::new(f, 0.85).unwrap();
    // dim 0 stays at 80 (worst-dim already), dim 1 grows to 0.40. Worst-dim
    // unchanged at 0.80.
    let p = Place::new(MachineId(1), Mass([0, 10]));
    let safe = p.apply_with_recheck(safe).expect("place in quiet dim should fit");
    assert_le(safe.gauge(), 0.85, "still within threshold");
}

#[test]
fn neutral_in_multi_dim_requires_full_equality() {
    // Same caps, same loads in every dim → admitted.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 200]), Mass([50, 80]));
    f.add_machine(MachineId(2), Capacity([100, 200]), Mass([50, 80]));
    assert!(
        Neutral::witness(MachineId(1), MachineId(2), Mass([10, 5]), &f).is_some(),
        "Neutral admitted with equal caps and loads in every dim"
    );

    // Mismatched in one dim → rejected.
    let mut g: Fleet<2> = Fleet::new();
    g.add_machine(MachineId(1), Capacity([100, 200]), Mass([50, 80]));
    g.add_machine(MachineId(2), Capacity([100, 200]), Mass([50, 79]));
    assert!(
        Neutral::witness(MachineId(1), MachineId(2), Mass([10, 5]), &g).is_none(),
        "Neutral rejected when loads differ in any dim"
    );
}

#[test]
fn cold_to_hot_classifies_anti_robin_hood_in_dim_one() {
    // Caller knows the move is Pigou-Dalton in dim 0 but anti-Robin-Hood
    // in dim 1; the witness refuses, so they fall through to ColdToHot
    // (catch-all) which re-checks the gauge.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 20]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([30, 70]));

    let safe: Safe<Linfty<2>, 2> = Safe::new(f, 0.85).unwrap();

    // Mass that's Pigou-Dalton in dim 0 (m1=80 > m2=30) and anti-Robin-
    // Hood in dim 1 (m1=20 < m2=70). The witness rejected; user tries
    // ColdToHot::apply_with_recheck, which evaluates the gauge:
    //   m1: (75, 25) → 0.75
    //   m2: (35, 65) → 0.65
    //   worst-dim Linfty = 0.75 (down from 0.80).
    // The catch-all path admits it because the post-move gauge is fine.
    let m = ColdToHot::new(MachineId(1), MachineId(2), Mass([5, 5]));
    let result = m.apply_with_recheck(safe);
    assert!(result.is_ok(), "catch-all should succeed when gauge stays within bound");
    assert_le(result.unwrap().gauge(), 0.85, "gauge within threshold");
}

#[test]
fn long_chain_of_two_dim_transfers_stays_safe() {
    // Compose a sequence of 2D typed-pure moves; verify the gauge stays
    // bounded throughout.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 70]));
    f.add_machine(MachineId(2), Capacity([200, 200]), Mass([60, 50]));
    f.add_machine(MachineId(3), Capacity([100, 100]), Mass([40, 30]));
    f.add_machine(MachineId(4), Capacity([200, 200]), Mass([80, 60]));

    let safe: Safe<Linfty<2>, 2> = Safe::new(f, 0.85).unwrap();

    // 1→2: cap 100 → 200 in both dims. utils 1: (0.80, 0.70), 2: (0.30, 0.25).
    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([10, 10]), safe.fleet())
        .expect("1→2 typed-pure");
    let safe = m.apply(safe);
    assert_le(safe.gauge(), 0.85, "after 1→2");

    // 1→3: equal caps. Now m1: (70, 60), m3: (40, 30).
    let m = HotToCold::witness(MachineId(1), MachineId(3), Mass([5, 5]), safe.fleet())
        .expect("1→3 typed-pure");
    let safe = m.apply(safe);
    assert_le(safe.gauge(), 0.85, "after 1→3");

    // Remove from 4 — total apply, no Result.
    let safe = Remove::new(MachineId(4), Mass([10, 0])).apply(safe);
    assert_le(safe.gauge(), 0.85, "after remove");
}

#[test]
fn worst_dim_reduction_is_correct() {
    // Verify the per-machine worst-dim reduction directly.
    let mut f: Fleet<3> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100, 100]), Mass([50, 80, 30]));

    let spec = f.spec(MachineId(1)).unwrap();
    assert!((spec.worst_utilization() - 0.80).abs() < 1e-9);
    assert_eq!(spec.utilization(), Utilization([0.50, 0.80, 0.30]));
}

#[test]
fn higher_n_compiles_and_runs() {
    // Smoke test: 4-dim fleet (e.g., CPU + memory + disk + network).
    let mut f: Fleet<4> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100, 100, 100]), Mass([10, 20, 30, 40]));
    f.add_machine(MachineId(2), Capacity([100, 100, 100, 100]), Mass([5, 5, 5, 5]));

    let safe: Safe<Linfty<4>, 4> = Safe::new(f, 0.50).unwrap();
    assert!((safe.gauge() - 0.40).abs() < 1e-9);

    // Move the 4-dim mass.
    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([5, 5, 5, 5]), safe.fleet())
        .expect("4-dim transfer typed-pure");
    let safe = m.apply(safe);
    assert!(safe.gauge() <= 0.40 + 1e-9);
}

#[test]
fn zero_mass_witness_rejected() {
    // A no-op move is not a meaningful HotToCold; reject.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 70]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([30, 20]));
    assert!(HotToCold::witness(MachineId(1), MachineId(2), Mass([0, 0]), &f).is_none());
}

#[test]
fn multi_dim_neutral_admits_partial_mass() {
    // Two equal machines. A Neutral that moves mass in one dim but not
    // the other is still a valid permutation in the dim where mass moves
    // and a no-op in the dim where it doesn't.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([50, 50]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([50, 50]));

    let m = Neutral::witness(MachineId(1), MachineId(2), Mass([10, 0]), &f);
    assert!(m.is_some(), "Neutral with sparse mass should be admitted");
}
