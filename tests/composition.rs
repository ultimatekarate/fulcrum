//! Composition tests.
//!
//! These verify the framework's load-bearing claim: typed-pure moves compose
//! into safety-preserving sequences. A long chain of Pigou-Dalton transfers
//! and removals stays within the gauge bound, and the type system carries the
//! claim through without runtime re-evaluation at each step.

use fulcrum::{Fleet, HotToCold, Linfty, MachineId, Mass, Remove, Safe, SumTopK};

fn fleet(loads: &[(u64, u64)], capacity: u64) -> Fleet {
    let mut f = Fleet::new();
    for &(id, load) in loads {
        f.add_machine(MachineId(id), capacity, load);
    }
    f
}

#[test]
fn long_typed_pure_chain_stays_safe() {
    // Start with an unbalanced fleet within the threshold.
    let f = fleet(&[(1, 80), (2, 40), (3, 40), (4, 40)], 100);
    let safe: Safe<Linfty> = Safe::new(f, 0.85).expect("starts within threshold");

    // Apply 4 Pigou-Dalton transfers and 2 removals.
    let m1 = HotToCold::witness(MachineId(1), MachineId(2), Mass(10), safe.fleet()).unwrap();
    let safe = m1.apply(safe);

    let m2 = HotToCold::witness(MachineId(1), MachineId(3), Mass(10), safe.fleet()).unwrap();
    let safe = m2.apply(safe);

    let r1 = Remove::new(MachineId(1), Mass(5));
    let safe = r1.apply(safe);

    let m3 = HotToCold::witness(MachineId(2), MachineId(4), Mass(5), safe.fleet()).unwrap();
    let safe = m3.apply(safe);

    let r2 = Remove::new(MachineId(3), Mass(10));
    let safe = r2.apply(safe);

    // All applies were total — no Result, no error path. The type system
    // carried the safety claim through the entire chain. Sanity-check the
    // gauge value here.
    assert!(
        safe.gauge() <= 0.85,
        "gauge {} should not exceed threshold 0.85",
        safe.gauge()
    );
}

#[test]
fn typed_pure_chain_decreases_or_holds_gauge() {
    let f = fleet(&[(1, 80), (2, 30), (3, 50)], 100);
    let safe: Safe<Linfty> = Safe::new(f, 0.85).unwrap();
    let g_before = safe.gauge();

    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass(20), safe.fleet()).unwrap();
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
    // Trying to construct a HotToCold from cold (30) to hot (80) must fail.
    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass(10), &f);
    assert!(w.is_none(), "witness should reject anti-Robin-Hood direction");
}

#[test]
fn witness_rejects_overshoot() {
    let f = fleet(&[(1, 80), (2, 30)], 100);
    // src - dst = 50, so mass=51 would invert the order.
    let w = HotToCold::witness(MachineId(1), MachineId(2), Mass(51), &f);
    assert!(
        w.is_none(),
        "witness should reject mass that would invert the order"
    );
}

// The load-bearing test of the framework's extensibility: the same move
// alphabet, the same `Safe<G>` typestate, the same `apply` impls — just
// swap the gauge from `Linfty` to `SumTopK<K>`. If this requires changing
// any code outside `gauge.rs`, the framework's thesis is wrong. It does
// not.
#[test]
fn safe_threads_through_alternate_gauge() {
    let f = fleet(&[(1, 80), (2, 40), (3, 40), (4, 20)], 100);
    // Threshold for SumTopK<2>: top-2 utilizations sum.
    // Initial top-2 = 0.80 + 0.40 = 1.20.
    let safe: Safe<SumTopK<2>> = Safe::new(f, 1.30).expect("starts within threshold");

    // Pigou-Dalton transfer: still total apply, no Result.
    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass(20), safe.fleet()).unwrap();
    let safe = m.apply(safe);

    // After: loads (60, 60, 40, 20), top-2 = 0.60 + 0.60 = 1.20.
    assert!(safe.gauge() <= 1.30);

    // Removal works the same way.
    let safe = Remove::new(MachineId(2), Mass(10)).apply(safe);
    assert!(safe.gauge() <= 1.30);
}

#[test]
fn alternate_gauge_rejects_threshold_violation() {
    let f = fleet(&[(1, 80), (2, 60), (3, 40), (4, 20)], 100);
    // Top-2 = 0.80 + 0.60 = 1.40, threshold 1.30 → must reject.
    let r: Result<Safe<SumTopK<2>>, _> = Safe::new(f, 1.30);
    assert!(matches!(
        r,
        Err(fulcrum::GaugeError::ThresholdExceeded { .. })
    ));
}
