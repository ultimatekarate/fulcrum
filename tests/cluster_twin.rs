//! Integration tests for the cluster digital twin: power-model unit checks,
//! topology shape, and end-to-end determinism / budget adherence.

use fulcrum::{
    fleet_power, node_power, run_turing_pi_2_twin, turing_pi_2, Fleet, HotToCold, Linfty, MachineId,
    Mass, Power, PowerBudget, PowerCoeffs, ResourceDim, Safe,
};

fn coeffs(idle: f64, dynamic: f64) -> PowerCoeffs {
    PowerCoeffs { idle: Power(idle), dynamic: Power(dynamic) }
}

#[test]
fn node_power_idle_at_zero_util() {
    let c = coeffs(2700.0, 7300.0);
    assert!((node_power(0.0, &c).milliwatts() - 2700.0).abs() < 1e-9);
}

#[test]
fn node_power_full_is_idle_plus_dynamic() {
    let c = coeffs(2700.0, 7300.0);
    // util = 1.0 ⇒ idle + dynamic·1² = idle + dynamic.
    assert!((node_power(1.0, &c).milliwatts() - 10_000.0).abs() < 1e-9);
}

#[test]
fn node_power_is_convex_in_util() {
    let c = coeffs(2700.0, 7300.0);
    // Half utilization draws less than half the dynamic span above idle:
    // 2700 + 7300·0.25 = 4525, strictly below the linear midpoint 6350.
    assert!((node_power(0.5, &c).milliwatts() - 4525.0).abs() < 1e-9);
}

#[test]
fn fleet_power_idle_only_is_sum_of_idle() {
    let topo = turing_pi_2();
    let fleet = topo.fleet(); // zero load everywhere
    let cs = topo.coeffs();
    let total = fleet_power(&fleet, &cs).milliwatts();
    let expected: f64 = cs.iter().map(|c| c.idle.milliwatts()).sum();
    assert!((total - expected).abs() < 1e-9);
}

#[test]
fn topology_is_four_heterogeneous_nodes() {
    let topo = turing_pi_2();
    assert_eq!(topo.len(), 4);
    let nodes = topo.nodes();
    // Two distinct board profiles (2× Pi 5, 2× Pi 4) ⇒ heterogeneous.
    let mem = ResourceDim::Mem.index();
    assert_ne!(nodes[0].1.capacity[mem], nodes[2].1.capacity[mem]);
    // Slots are ascending MachineId, parallel to coeffs().
    assert_eq!(nodes[0].0, MachineId(1));
    assert_eq!(nodes[3].0, MachineId(4));
}

/// A mass-preserving Pigou-Dalton transfer (`HotToCold`) must not increase
/// fleet power and strictly decreases it when it reduces imbalance — the
/// Schur-convexity of the convex model.
#[test]
fn hot_to_cold_does_not_increase_power() {
    let c = coeffs(2700.0, 7300.0);
    let cs = vec![c, c];

    let mut fleet: Fleet<4> = Fleet::new();
    fleet.add_machine(MachineId(1), [100, 100, 100, 100], [80, 0, 0, 0]);
    fleet.add_machine(MachineId(2), [100, 100, 100, 100], [20, 0, 0, 0]);

    let before = fleet_power(&fleet, &cs).milliwatts();

    let safe: Safe<Linfty<4>, 4> = Safe::new(fleet, 1.0).unwrap();
    // Move 30 CPU from the hot node (0.80) to the cold node (0.20): both
    // land at 0.50, strictly reducing imbalance.
    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([30, 0, 0, 0]), safe.fleet())
        .expect("witness: 0.80 > 0.20, mass within gap");
    let safe = m.apply(safe);

    let after = fleet_power(safe.fleet(), &cs).milliwatts();
    assert!(after < before, "power should drop: before={before} after={after}");
}

#[test]
fn twin_is_deterministic_and_within_budget() {
    let config = fulcrum::TwinConfig {
        seed: 0xABCDEF,
        n_workloads: 30,
        max_mass: [150, 1500, 150, 150],
        threshold: 0.95,
        budget: PowerBudget(Power(32_000.0)),
        rebalance_epsilon: 1e-3,
    };
    let a = run_turing_pi_2_twin(config).unwrap();
    let b = run_turing_pi_2_twin(config).unwrap();

    assert_eq!(a.stats, b.stats);
    assert_eq!(a.timeline.len(), b.timeline.len());
    assert!((a.final_gauge - b.final_gauge).abs() < 1e-12);

    for row in &a.timeline {
        assert!(config.budget.within(row.power), "budget exceeded at step {}", row.step);
    }
}

/// The power control point must actually fire under a tight budget: with
/// only ~3 W of dynamic headroom above the ~10.8 W idle baseline, some
/// placements are rejected for power — and the budget is still never
/// exceeded on any recorded step.
#[test]
fn tight_budget_rejects_some_placements() {
    let budget = PowerBudget(Power(14_000.0));
    let config = fulcrum::TwinConfig {
        seed: 0xC0FFEE,
        n_workloads: 24,
        max_mass: [120, 1200, 150, 150],
        threshold: 0.95,
        budget,
        rebalance_epsilon: 1e-3,
    };
    let report = run_turing_pi_2_twin(config).unwrap();
    assert!(report.stats.power_rejected > 0, "tight budget should reject placements");
    for row in &report.timeline {
        assert!(budget.within(row.power), "budget exceeded at step {}", row.step);
    }
}
