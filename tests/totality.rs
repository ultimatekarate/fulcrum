//! Totality tests.
//!
//! Verify that typed-pure `apply` impls — `Remove`, `HotToCold`, `Neutral` —
//! never panic in well-formed programs and never return `Result`. The
//! signatures themselves are the strongest test (signatures don't lie); the
//! tests below exercise representative cases.

use fulcrum::{Fleet, HotToCold, Linfty, MachineId, Mass, Neutral, Remove, Safe};

fn one_machine(load: u64) -> Fleet<1> {
    let mut f = Fleet::new();
    f.add_machine(MachineId(1), [100], [load]);
    f
}

#[test]
fn remove_apply_returns_safe_directly() {
    let safe: Safe<Linfty<1>, 1> = Safe::new(one_machine(50), 0.9).unwrap();
    let safe = Remove::new(MachineId(1), Mass([10])).apply(safe);
    assert_eq!(safe.fleet().load(MachineId(1)), Some([40]));
}

#[test]
fn hot_to_cold_apply_returns_safe_directly() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), [100], [80]);
    f.add_machine(MachineId(2), [100], [30]);
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.9).unwrap();

    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([20]), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    assert_eq!(safe.fleet().load(MachineId(1)), Some([60]));
    assert_eq!(safe.fleet().load(MachineId(2)), Some([50]));
}

#[test]
fn neutral_apply_returns_safe_directly() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), [100], [50]);
    f.add_machine(MachineId(2), [100], [50]);
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.9).unwrap();

    let m = Neutral::witness(MachineId(1), MachineId(2), Mass([10]), safe.fleet()).unwrap();
    let safe = m.apply(safe);
    assert_eq!(safe.fleet().load(MachineId(1)), Some([40]));
    assert_eq!(safe.fleet().load(MachineId(2)), Some([60]));
}

#[test]
fn neutral_witness_rejects_unequal_loads() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), [100], [50]);
    f.add_machine(MachineId(2), [100], [51]);
    let w = Neutral::witness(MachineId(1), MachineId(2), Mass([1]), &f);
    assert!(w.is_none(), "Neutral witness should reject unequal loads");
}

#[test]
fn neutral_witness_rejects_unequal_capacities() {
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), [100], [50]);
    f.add_machine(MachineId(2), [200], [100]);
    let w = Neutral::witness(MachineId(1), MachineId(2), Mass([10]), &f);
    assert!(
        w.is_none(),
        "Neutral witness should reject unequal capacities even at equal utilization"
    );
}
