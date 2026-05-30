//! Phase 3 acceptance tests: planner trait + reference implementations.
//!
//! For each reference planner the acceptance contract is:
//!
//! 1. The planner emits a non-empty typed-move sequence on a synthetic
//!    fleet (i.e., the planner *does* something — the trait contract is
//!    not vacuous).
//! 2. Every emitted move applies cleanly (the catch-all moves don't
//!    spuriously violate the gauge bound on a well-shaped scenario).
//! 3. The gauge bound is preserved at every intermediate state.
//!
//! Property (3) is the load-bearing claim — it's what the framework
//! provides and what each planner is supposed to respect. The test
//! harness `drive_to_completion` makes the property check structural:
//! it can't be silently relaxed by a planner regression.

use fulcrum::{
    BestFitDecreasing, Capacity, Fleet, LeastLoaded, Linfty, MachineId, MaxMinFair, Mass, Planner,
    PowerOfTwo, Safe, TypedMove,
};

/// Drive a planner to completion, asserting the gauge bound at every
/// intermediate state. Returns the final safe and the move sequence.
fn drive_to_completion<P, const N: usize>(
    mut planner: P,
    mut safe: Safe<Linfty<N>, N>,
    threshold: f64,
    label: &str,
) -> (Safe<Linfty<N>, N>, Vec<&'static str>)
where
    P: Planner<N, Linfty<N>>,
{
    let mut moves: Vec<&'static str> = Vec::new();
    let mut step = 0;
    // Hard cap on iterations — if a planner doesn't terminate, we don't
    // want the test to hang; we want a loud failure.
    while step < 10_000 {
        let m = match planner.step(&safe) {
            Some(m) => m,
            None => break,
        };
        moves.push(m.kind_name());
        let result = m.apply(safe);
        match result {
            Ok(s) => {
                safe = s;
                let g = safe.gauge();
                assert!(
                    g <= threshold + 1e-9,
                    "{}: gauge bound violated at step {}: {} > {}",
                    label,
                    step,
                    g,
                    threshold
                );
            }
            Err(e) => {
                panic!(
                    "{}: planner emitted a move that failed apply at step {}: {:?}",
                    label, step, e
                )
            }
        }
        step += 1;
    }
    assert!(step < 10_000, "{}: planner failed to terminate within 10k steps", label);
    (safe, moves)
}

fn empty_fleet_1d_with_capacity(machines: &[(u64, u64)]) -> Fleet<1> {
    let mut f = Fleet::new();
    for &(id, cap) in machines {
        f.add_machine(MachineId(id), Capacity([cap]), Mass([0]));
    }
    f
}

#[test]
fn least_loaded_distributes_work_uniformly_at_uniform_capacity() {
    // Five empty machines, capacity 100 each. Place ten unit-mass items
    // (mass=10 each). LeastLoaded should distribute roughly evenly: each
    // machine ends with two items, gauge = 0.20.
    let f = empty_fleet_1d_with_capacity(&[(1, 100), (2, 100), (3, 100), (4, 100), (5, 100)]);
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.50).unwrap();

    let items: Vec<Mass<1>> = (0..10).map(|_| Mass([10])).collect();
    let planner: LeastLoaded<1> = LeastLoaded::new(items);

    let (final_safe, moves) = drive_to_completion(planner, safe, 0.50, "LeastLoaded uniform");
    assert_eq!(moves.len(), 10, "should place all 10 items");
    assert!(moves.iter().all(|k| *k == "Place"), "all moves should be Place");

    // Ten items × mass 10 = 100 total mass spread over 5 machines = avg
    // load 20, gauge = 0.20. LeastLoaded round-robins under the stable
    // tie-break → exactly 0.20.
    assert!((final_safe.gauge() - 0.20).abs() < 1e-9);
}

#[test]
fn least_loaded_prefers_lighter_machine_under_heterogeneous_caps() {
    // m1: cap 100 with prior load 30 → util 0.30.
    // m2: cap 200 with prior load 30 → util 0.15.
    // Same absolute load, but m2 is less utilized; LeastLoaded picks m2.
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([30]));
    f.add_machine(MachineId(2), Capacity([200]), Mass([30]));
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.99).unwrap();

    let mut planner: LeastLoaded<1> = LeastLoaded::new(vec![Mass([10])]);
    let m = planner.step(&safe).unwrap();
    match m {
        TypedMove::Place(p) => assert_eq!(p.machine, MachineId(2), "should pick m2 (lower util)"),
        _ => panic!("expected Place"),
    }
}

#[test]
fn power_of_two_distributes_work() {
    // Same shape as LeastLoaded test, but with a fixed seed for
    // reproducibility. The exact final gauge depends on the PRNG sequence,
    // so we check structural properties rather than exact value.
    let f = empty_fleet_1d_with_capacity(&[(1, 100), (2, 100), (3, 100), (4, 100), (5, 100)]);
    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.99).unwrap();

    let items: Vec<Mass<1>> = (0..20).map(|_| Mass([5])).collect();
    let planner: PowerOfTwo<1> = PowerOfTwo::new(items, 0xDEADBEEFu64);

    let (final_safe, moves) = drive_to_completion(planner, safe, 0.99, "PowerOfTwo");
    assert_eq!(moves.len(), 20);
    assert!(moves.iter().all(|k| *k == "Place"));

    // Total mass: 20 × 5 = 100, spread over 5 machines, avg load 20 →
    // expected gauge ~0.20. Power-of-two under random sampling won't
    // hit perfect uniformity, but it should be much better than worst-
    // case (all on one machine = 1.00). Cap at a reasonable bound.
    assert!(
        final_safe.gauge() <= 0.50,
        "PowerOfTwo with 20 unit items should not exceed 0.50 gauge"
    );
}

#[test]
fn power_of_two_is_deterministic_under_fixed_seed() {
    let f1 = empty_fleet_1d_with_capacity(&[(1, 100), (2, 100), (3, 100)]);
    let f2 = empty_fleet_1d_with_capacity(&[(1, 100), (2, 100), (3, 100)]);
    let safe1: Safe<Linfty<1>, 1> = Safe::new(f1, 0.99).unwrap();
    let safe2: Safe<Linfty<1>, 1> = Safe::new(f2, 0.99).unwrap();

    let items: Vec<Mass<1>> = (0..10).map(|i| Mass([5 + i])).collect();
    let p1: PowerOfTwo<1> = PowerOfTwo::new(items.clone(), 42);
    let p2: PowerOfTwo<1> = PowerOfTwo::new(items, 42);

    let (s1, _) = drive_to_completion(p1, safe1, 0.99, "PoT seed 42 a");
    let (s2, _) = drive_to_completion(p2, safe2, 0.99, "PoT seed 42 b");

    assert_eq!(s1.gauge(), s2.gauge(), "same seed → same outcome");
}

#[test]
fn max_min_fair_balances_an_unbalanced_fleet() {
    // Heavily skewed initial fleet. MaxMinFair should iteratively shed
    // load from the most-loaded to the least-loaded, terminating when
    // the gap drops below epsilon.
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([80]));
    f.add_machine(MachineId(2), Capacity([100]), Mass([70]));
    f.add_machine(MachineId(3), Capacity([100]), Mass([10]));
    f.add_machine(MachineId(4), Capacity([100]), Mass([10]));

    let initial_gauge = Linfty::<1>::default();
    use fulcrum::gauge::Gauge;
    let initial = initial_gauge.eval(&f);

    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.99).unwrap();
    let planner = MaxMinFair::new(0.05);

    let (final_safe, moves) = drive_to_completion(planner, safe, 0.99, "MaxMinFair");

    assert!(!moves.is_empty(), "should emit at least one migration");
    assert!(
        moves.iter().all(|k| *k == "HotToCold"),
        "MaxMinFair should emit only HotToCold; got {:?}",
        moves
    );
    assert!(
        final_safe.gauge() <= initial,
        "gauge should decrease or hold: before {}, after {}",
        initial,
        final_safe.gauge()
    );
    // After balancing, the max-min spread should be within epsilon.
    let min_u = final_safe
        .fleet()
        .iter()
        .map(|(_, s)| s.worst_utilization())
        .fold(f64::INFINITY, f64::min);
    let max_u = final_safe
        .fleet()
        .iter()
        .map(|(_, s)| s.worst_utilization())
        .fold(0.0_f64, f64::max);
    assert!(
        max_u - min_u < 0.10,
        "post-MaxMinFair gap: max {}, min {}, gap {}",
        max_u,
        min_u,
        max_u - min_u
    );
}

#[test]
fn max_min_fair_no_op_on_already_balanced_fleet() {
    // If the fleet is already balanced, MaxMinFair returns None on the
    // first step.
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([50]));
    f.add_machine(MachineId(2), Capacity([100]), Mass([50]));
    f.add_machine(MachineId(3), Capacity([100]), Mass([50]));

    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.99).unwrap();
    let mut planner = MaxMinFair::new(0.01);
    assert!(planner.step(&safe).is_none(), "balanced fleet → no migrations");
}

#[test]
fn best_fit_decreasing_packs_items() {
    // Heterogeneous fleet: a couple of large machines and a couple of
    // small ones. BFD with a mix of items should respect feasibility
    // and produce a non-empty Place sequence.
    //
    // Threshold 1.0 — BFD's job is to pack tightly, possibly to 100%
    // utilization on individual bins. Total item mass 230 vs total
    // capacity 400; perfectly fittable.
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([0]));
    f.add_machine(MachineId(2), Capacity([200]), Mass([0]));
    f.add_machine(MachineId(3), Capacity([50]), Mass([0]));
    f.add_machine(MachineId(4), Capacity([50]), Mass([0]));

    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 1.0).unwrap();
    let items = vec![Mass([80]), Mass([60]), Mass([40]), Mass([30]), Mass([20])];
    let planner: BestFitDecreasing<1> = BestFitDecreasing::new(items);

    let (final_safe, moves) = drive_to_completion(planner, safe, 1.0, "BFD");
    assert_eq!(moves.len(), 5, "all 5 items should fit");
    assert!(moves.iter().all(|k| *k == "Place"));
    // Total placed mass = 230, distributed across 4 machines totaling
    // 400 cap. The worst-loaded machine's util determines the gauge.
    assert!(final_safe.gauge() <= 1.0);
}

#[test]
fn best_fit_decreasing_skips_infeasible_items() {
    // Item too big to fit anywhere → BFD's filter rejects it, planner
    // returns None at that step.
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([0]));
    f.add_machine(MachineId(2), Capacity([100]), Mass([0]));

    let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.99).unwrap();
    let mut planner: BestFitDecreasing<1> = BestFitDecreasing::new(vec![Mass([200])]);
    assert!(planner.step(&safe).is_none(), "no machine fits 200 in cap-100 fleet");
}

// ----- Multi-dim acceptance -----

#[test]
fn least_loaded_works_in_two_dim() {
    // 2D fleet. Place items with mass in both dims.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([0, 0]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([0, 0]));
    f.add_machine(MachineId(3), Capacity([100, 100]), Mass([0, 0]));

    let safe: Safe<Linfty<2>, 2> = Safe::new(f, 0.99).unwrap();
    let items = vec![Mass([10, 10]), Mass([10, 10]), Mass([10, 10]), Mass([10, 10])];
    let planner: LeastLoaded<2> = LeastLoaded::new(items);

    let (final_safe, moves) = drive_to_completion(planner, safe, 0.99, "LeastLoaded 2D");
    assert_eq!(moves.len(), 4);
    // 4 items × 10 mass spread over 3 machines: best case ⌈4/3⌉=2 items
    // on at least one machine, so worst-machine load = 20 in each dim,
    // util 0.20.
    assert!(final_safe.gauge() <= 0.30);
}

#[test]
fn max_min_fair_works_in_two_dim_when_imbalance_lies_in_one_dim() {
    // m1: utils (0.80, 0.20), m2: utils (0.20, 0.20). Worst-dim per
    // machine: m1=0.80, m2=0.20. MaxMinFair migrates in m1's worst dim
    // (dim 0) toward m2.
    let mut f: Fleet<2> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100, 100]), Mass([80, 20]));
    f.add_machine(MachineId(2), Capacity([100, 100]), Mass([20, 20]));

    let safe: Safe<Linfty<2>, 2> = Safe::new(f, 0.99).unwrap();
    let initial = safe.gauge();
    let planner = MaxMinFair::new(0.05);
    let (final_safe, moves) = drive_to_completion(planner, safe, 0.99, "MaxMinFair 2D");

    assert!(!moves.is_empty(), "should emit migrations");
    assert!(
        final_safe.gauge() <= initial,
        "non-increasing: before {}, after {}",
        initial,
        final_safe.gauge()
    );
}

#[test]
fn typed_move_is_typed_pure_classifies_correctly() {
    // Smoke test of TypedMove::is_typed_pure across all five variants.
    use fulcrum::{ColdToHot, HotToCold, Neutral, Place, Remove};
    let mut f: Fleet<1> = Fleet::new();
    f.add_machine(MachineId(1), Capacity([100]), Mass([50]));
    f.add_machine(MachineId(2), Capacity([100]), Mass([30]));

    let r: TypedMove<1> = TypedMove::Remove(Remove::new(MachineId(1), Mass([5])));
    let h = TypedMove::HotToCold(
        HotToCold::witness(MachineId(1), MachineId(2), Mass([5]), &f).unwrap(),
    );
    // Build a Neutral on a separate equal-load fleet.
    let mut feq: Fleet<1> = Fleet::new();
    feq.add_machine(MachineId(1), Capacity([100]), Mass([50]));
    feq.add_machine(MachineId(2), Capacity([100]), Mass([50]));
    let n = TypedMove::Neutral(
        Neutral::witness(MachineId(1), MachineId(2), Mass([5]), &feq).unwrap(),
    );
    let c: TypedMove<1> =
        TypedMove::ColdToHot(ColdToHot::new(MachineId(2), MachineId(1), Mass([5])));
    let p: TypedMove<1> = TypedMove::Place(Place::new(MachineId(1), Mass([5])));

    assert!(r.is_typed_pure());
    assert!(h.is_typed_pure());
    assert!(n.is_typed_pure());
    assert!(!c.is_typed_pure());
    assert!(!p.is_typed_pure());
}
