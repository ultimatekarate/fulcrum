//! Locks the rebalance-stall experiment's findings (see
//! `examples/rebalance_stalls.rs`). The falsifiable core of the elastic-carrier
//! hypothesis, measured on the real twin via the *same* `evaluate_pair` the
//! planner uses (so the diagnostic cannot drift from the policy).
//!
//! ## Robust finding (both canonical workloads)
//!
//! `MaxMinFair` never *converges*: in both scenarios Phase B halts on a
//! capacity-guard block while super-epsilon imbalance remains. So the low
//! typed-pure ratio is partly **suppression**, not balance. (`step` only ever
//! tries the single global max/min pair; when that pair's binding dimension is
//! guard-blocked it gives up.)
//!
//! ## Workload-dependent finding (which fix applies)
//!
//! - **Headline (`0xC0FFEE`, the scenario the 0.12 ratio is quoted from):** the
//!   guard-blocked residue lands on **Cpu** and **DiskIo** — both semantically
//!   *elastic* (a share / a rate, not a conserved quantity). Here the guard is
//!   spurious and the elastic carrier is the right tool: the suppression is a
//!   genuine carrier mismatch.
//! - **Memory-heavy (`0xABCDEF`):** the residue lands entirely on **memory** —
//!   irreducibly *absolute*, where the guard is correct and the carrier would
//!   NOT apply. Here the fix is more planner coverage (typed-pure moves are
//!   left untried), and the carrier signal is a false lead.
//!
//! The two together are the point: carrier-vs-planner is empirically separable,
//! and it depends on which resource is binding — exactly the absolute/elastic
//! split the end-state is built around.

use fulcrum::{
    diagnose_turing_pi_2_rebalance, PairVerdict, Power, PowerBudget, ResourceDim, TwinConfig,
};

fn headline() -> TwinConfig {
    TwinConfig {
        seed: 0xC0FFEE,
        n_workloads: 24,
        max_mass: [120, 1200, 150, 150],
        threshold: 0.95,
        budget: PowerBudget(Power(32_000.0)),
        rebalance_epsilon: 1e-3,
    }
}

fn memory_heavy() -> TwinConfig {
    TwinConfig {
        seed: 0xABCDEF,
        n_workloads: 30,
        max_mass: [150, 1500, 150, 150],
        threshold: 0.95,
        budget: PowerBudget(Power(32_000.0)),
        rebalance_epsilon: 1e-3,
    }
}

/// Worst-dim of every guard-blocked pair in the halt scan.
fn guard_blocked_dims(r: &fulcrum::RebalanceStallReport) -> Vec<usize> {
    r.halt_scan
        .iter()
        .filter_map(|(_, _, v)| match v {
            PairVerdict::GuardBlocked { worst_d, .. } => Some(*worst_d),
            _ => None,
        })
        .collect()
}

#[test]
fn diagnosis_is_deterministic() {
    for cfg in [headline(), memory_heavy()] {
        let a = diagnose_turing_pi_2_rebalance(cfg).unwrap();
        let b = diagnose_turing_pi_2_rebalance(cfg).unwrap();
        assert_eq!(a.phase_b_log, b.phase_b_log);
        assert_eq!(a.halt_scan, b.halt_scan);
        assert_eq!(a.migrations_applied, b.migrations_applied);
        assert_eq!(a.halted_gauge.to_bits(), b.halted_gauge.to_bits());
    }
}

/// The robust invariant: neither workload converges — both stall on the guard
/// with residual imbalance. This is the mechanism the whole hypothesis rests on.
#[test]
fn neither_workload_converges_both_stall_on_the_guard() {
    for cfg in [headline(), memory_heavy()] {
        let r = diagnose_turing_pi_2_rebalance(cfg).unwrap();
        assert_eq!(r.migrations_applied, 3, "expected 3 typed-pure migrations");
        let halt = r.phase_b_log.last().copied().expect("at least one step");
        match halt {
            PairVerdict::GuardBlocked { src_u, dst_u, cap_src, cap_dst, .. } => {
                assert!(src_u - dst_u > cfg.rebalance_epsilon, "residual imbalance at halt");
                assert!(cap_src > cap_dst, "guard fires: source is the larger machine");
            }
            other => panic!("expected a GuardBlocked stall, not convergence; got {other:?}"),
        }
    }
}

/// Headline scenario (the 0.12 everyone quotes): the suppression is a genuine
/// CARRIER mismatch — the guard-blocked residue is on elastic-plausible
/// dimensions (Cpu, DiskIo), never on absolute memory.
#[test]
fn headline_residue_is_on_elastic_dimensions() {
    let r = diagnose_turing_pi_2_rebalance(headline()).unwrap();

    // Halts on CPU specifically.
    match r.phase_b_log.last().copied().unwrap() {
        PairVerdict::GuardBlocked { worst_d, .. } => {
            assert_eq!(worst_d, ResourceDim::Cpu.index(), "headline halts on CPU");
        }
        other => panic!("expected GuardBlocked, got {other:?}"),
    }

    let dims = guard_blocked_dims(&r);
    assert!(!dims.is_empty(), "expected guard-blocked pairs");
    // Every guard-blocked stall is on an elastic-plausible dimension…
    let cpu = ResourceDim::Cpu.index();
    let disk = ResourceDim::DiskIo.index();
    let mem = ResourceDim::Mem.index();
    assert!(
        dims.iter().all(|&d| d == cpu || d == disk),
        "headline residue must be on Cpu/DiskIo (elastic), got dims {dims:?}"
    );
    // …and none on absolute memory.
    assert!(!dims.contains(&mem), "headline residue must not touch memory");
    assert!(dims.contains(&cpu), "headline residue includes CPU");
}

/// Memory-heavy scenario: the guard-blocked residue is entirely on memory — the
/// irreducibly ABSOLUTE dimension, where the guard is correct and the elastic
/// carrier does not apply. Here the carrier signal is a false lead and the cheap
/// fix is planner coverage (typed-pure moves left untried at the halt).
#[test]
fn memory_heavy_residue_is_all_absolute_and_planner_coverage_remains() {
    let r = diagnose_turing_pi_2_rebalance(memory_heavy()).unwrap();

    let mem = ResourceDim::Mem.index();
    let dims = guard_blocked_dims(&r);
    assert!(!dims.is_empty(), "expected guard-blocked pairs");
    assert!(
        dims.iter().all(|&d| d == mem),
        "memory-heavy residue should be all on memory (absolute), got {dims:?}"
    );

    // Planner-coverage opportunity: typed-pure moves the single-pair planner
    // never tried still exist at the halt.
    let emits = r
        .halt_scan
        .iter()
        .filter(|(_, _, v)| matches!(v, PairVerdict::Emit { .. }))
        .count();
    assert!(emits > 0, "expected untried typed-pure moves at the halt; found {emits}");
}
