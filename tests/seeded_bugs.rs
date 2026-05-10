//! Seeded-bug battery.
//!
//! Each test constructs a scenario that a naive coordinate-cap check would
//! accept but the typed framework — under a Schur-convex gauge — rejects, or
//! vice versa. The point is to demonstrate the framework's *localization*
//! claim: when a hot-spot or imbalance appears, the rejection lands at one
//! named site (`Place::apply`, `ColdToHot::apply`, or `HotToCold::witness`).
//!
//! These tests are the v0 evidence that the framework is doing more than
//! re-implementing per-sled capacity checks.

use fulcrum::{ColdToHot, Fleet, HotToCold, Linfty, MachineId, Mass, Place, Safe};

fn fleet(loads: &[(u64, u64)], capacity: u64) -> Fleet<1> {
    let mut f = Fleet::new();
    for &(id, load) in loads {
        f.add_machine(MachineId(id), [capacity], [load]);
    }
    f
}

#[test]
fn place_creating_hotspot_is_rejected_at_apply() {
    let f = fleet(&[(1, 80), (2, 30), (3, 30)], 100);
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.85).unwrap();

    let p = Place::new(MachineId(1), Mass([10]));
    let result = p.apply_with_recheck(safe);

    assert!(
        matches!(
            result,
            Err(fulcrum::GaugeError::ThresholdExceeded { .. })
        ),
        "framework must reject hot-spot-creating placement"
    );
}

#[test]
fn cold_to_hot_creating_hotspot_is_rejected_at_apply() {
    let f = fleet(&[(1, 80), (2, 30), (3, 30)], 100);
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.85).unwrap();

    let m = ColdToHot::new(MachineId(2), MachineId(1), Mass([10]));
    let result = m.apply_with_recheck(safe);

    assert!(
        matches!(
            result,
            Err(fulcrum::GaugeError::ThresholdExceeded { .. })
        ),
        "framework must reject anti-Robin-Hood that creates a hot-spot"
    );
}

#[test]
fn forged_hot_to_cold_against_wrong_fleet_is_caught_by_witness() {
    let f = fleet(&[(1, 30), (2, 80)], 100);
    let attempted = HotToCold::witness(MachineId(1), MachineId(2), Mass([10]), &f);
    assert!(
        attempted.is_none(),
        "witness must reject Pigou-Dalton in the wrong direction"
    );
}

#[test]
fn rebalance_chain_keeps_fleet_safe_through_intermediate_states() {
    let f = fleet(&[(1, 84), (2, 20), (3, 20), (4, 20)], 100);
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.85).unwrap();
    assert!(safe.gauge() <= 0.85);

    let m1 = HotToCold::witness(MachineId(1), MachineId(2), Mass([20]), safe.fleet()).unwrap();
    let safe = m1.apply(safe);
    assert!(safe.gauge() <= 0.85);

    let m2 = HotToCold::witness(MachineId(1), MachineId(3), Mass([20]), safe.fleet()).unwrap();
    let safe = m2.apply(safe);
    assert!(safe.gauge() <= 0.85);

    let m3 = HotToCold::witness(MachineId(2), MachineId(4), Mass([10]), safe.fleet()).unwrap();
    let safe = m3.apply(safe);
    assert!(safe.gauge() <= 0.85);
}
