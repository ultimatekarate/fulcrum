//! Composition tests.
//!
//! These verify the framework's load-bearing claim: typed-pure moves compose
//! into safety-preserving sequences. A long chain of Pigou-Dalton transfers
//! and removals stays within the gauge bound, and the type system carries the
//! claim through without runtime re-evaluation at each step.

use fulcrum::{Fleet, HotToCold, Linfty, MachineId, Mass, Remove, Safe, SumTopK};

fn fleet(loads: &[(u64, u64)], capacity: u64) -> Fleet<1> {
    let mut f = Fleet::new();
    for &(id, load) in loads {
        f.add_machine(MachineId(id), [capacity], [load]);
    }
    f
}

#[test]
fn long_typed_pure_chain_stays_safe() {
    let f = fleet(&[(1, 80), (2, 40), (3, 40), (4, 40)], 100);
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.85).expect("starts within threshold");

    let m1 = HotToCold::witness(MachineId(1), MachineId(2), Mass([10]), safe.fleet()).unwrap();
    let safe = m1.apply(safe);

    let m2 = HotToCold::witness(MachineId(1), MachineId(3), Mass([10]), safe.fleet()).unwrap();
    let safe = m2.apply(safe);

    let r1 = Remove::new(MachineId(1), Mass([5]));
    let safe = r1.apply(safe);

    let m3 = HotToCold::witness(MachineId(2), MachineId(4), Mass([5]), safe.fleet()).unwrap();
    let safe = m3.apply(safe);

    let r2 = Remove::new(MachineId(3), Mass([10]));
    let safe = r2.apply(safe);

    assert!(
        safe.gauge() <= 0.85,
        "gauge {} should not exceed threshold 0.85",
        safe.gauge()
    );
}

#[test]
fn typed_pure_chain_decreases_or_holds_gauge() {
    let f = fleet(&[(1, 80), (2, 30), (3, 50)], 100);
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.85).unwrap();
    let g_before = safe.gauge();

    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([20]), safe.fleet()).unwrap();
    let safe = m.apply(safe);

    assert!(
        safe.gauge() <= g_before + 1e-9,
        "Pigou-Dalton must not increase the gauge: before={}, after={}",
        g_before,
        safe.gauge()
    );
}

#[test]
fn witness_rejects_anti_robin_hood() {
    let f = fleet(&[(1, 30), (2, 80)], 100);
    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass([10]), &f);
    assert!(w.is_none(), "witness should reject anti-Robin-Hood direction");
}

#[test]
fn witness_rejects_overshoot() {
    let f = fleet(&[(1, 80), (2, 30)], 100);
    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass([51]), &f);
    assert!(
        w.is_none(),
        "witness should reject mass that would invert the order"
    );
}

#[test]
fn safe_threads_through_alternate_gauge() {
    let f = fleet(&[(1, 80), (2, 40), (3, 40), (4, 20)], 100);
    let safe: Safe<SumTopK<2, 1>, 1> = Safe::new(f, 1.30).expect("starts within threshold");

    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([20]), safe.fleet()).unwrap();
    let safe = m.apply(safe);

    assert!(safe.gauge() <= 1.30);

    let safe = Remove::new(MachineId(2), Mass([10])).apply(safe);
    assert!(safe.gauge() <= 1.30);
}

#[test]
fn alternate_gauge_rejects_threshold_violation() {
    let f = fleet(&[(1, 80), (2, 60), (3, 40), (4, 20)], 100);
    let r: Result<Safe<SumTopK<2, 1>, 1>, _> = Safe::new(f, 1.30);
    assert!(matches!(
        r,
        Err(fulcrum::GaugeError::ThresholdExceeded { .. })
    ));
}
