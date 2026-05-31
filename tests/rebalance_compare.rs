//! Locks the multi-pair rebalance comparison (see
//! `examples/rebalance_compare.rs`). Following the stall experiment's lead, the
//! cheap planner-coverage fix (`MaxMinFairGreedy`) is built first and measured
//! against the single-pair `MaxMinFair` baseline.
//!
//! ## Findings
//!
//! 1. The multi-pair planner strictly lifts typed-pure migrations and the
//!    typed-pure ratio on both workloads — with NO carrier change, and (free
//!    from the algebra) no loss of the gauge guarantee.
//! 2. On the memory-heavy workload it also materially lowers the load gauge
//!    (0.83 -> 0.72): genuine objective improvement the single-pair planner
//!    left unrealized.
//! 3. After greedy exhausts every admissible move (`greedy_residual_emits == 0`),
//!    the residue *on these two seeds* happens to sit on an elastic dimension —
//!    DiskIo in the headline, Cpu in the memory-heavy run. **These two seeds are
//!    NOT representative** — see `tests/residue_sweep.rs`, which shows memory
//!    (absolute) stranding in ~27% of headline-profile seeds and ~74% of
//!    memory-heavy seeds. So the residue is a *mix*: an elastic part the carrier
//!    could clear, and a common absolute-memory part it cannot (where the guard
//!    is correct). The assertions below pin the two specific seeds; do not read
//!    them as the general case.

use fulcrum::{compare_rebalancers, Power, PowerBudget, ResourceDim, TwinConfig};

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

/// Robust invariants that hold by construction, on both workloads.
#[test]
fn greedy_is_a_strict_coverage_superset() {
    for cfg in [headline(), memory_heavy()] {
        let c = compare_rebalancers(cfg).unwrap();
        // Greedy never makes fewer typed-pure moves than the single-pair planner.
        assert!(
            c.greedy_migrations >= c.baseline_migrations,
            "greedy must cover at least the baseline's moves"
        );
        // Greedy lifts the typed-pure ratio.
        assert!(
            c.greedy_ratio >= c.baseline_ratio,
            "greedy ratio {} should be >= baseline {}",
            c.greedy_ratio,
            c.baseline_ratio
        );
        // Greedy leaves NO admissible move on the table — that is its definition.
        assert_eq!(
            c.greedy_residual_emits, 0,
            "greedy must exhaust planner coverage (0 Emit pairs remaining)"
        );
        // The gauge never worsens (every move is a witnessed HotToCold).
        assert!(c.greedy_gauge <= c.baseline_gauge + 1e-12);
    }
}

/// The decisive carrier-isolation result: after planner coverage is spent, the
/// residue is on elastic-plausible dimensions only — never absolute memory.
#[test]
fn carrier_residue_is_on_elastic_dimensions_only() {
    let mem = ResourceDim::Mem.index();
    let cpu = ResourceDim::Cpu.index();
    let disk = ResourceDim::DiskIo.index();

    for cfg in [headline(), memory_heavy()] {
        let c = compare_rebalancers(cfg).unwrap();
        assert!(
            !c.greedy_residual_guard_blocked.is_empty(),
            "expected a non-empty carrier residue"
        );
        for (_, _, worst_d) in &c.greedy_residual_guard_blocked {
            assert_ne!(*worst_d, mem, "carrier residue must NOT be on absolute memory");
            assert!(
                *worst_d == cpu || *worst_d == disk,
                "carrier residue should be on an elastic dim (Cpu/DiskIo), got {worst_d}"
            );
        }
    }
}

/// Pin the exact numbers the experiment reported, so a regression in either
/// planner is visible. These are expected to move if the planners change.
#[test]
fn pinned_headline_numbers() {
    let c = compare_rebalancers(headline()).unwrap();
    assert_eq!(c.baseline_migrations, 3);
    assert_eq!(c.greedy_migrations, 4);
    assert!(c.greedy_ratio > c.baseline_ratio);
    // Residue: all four guard-blocked pairs bind on DiskIo (a rate -> elastic).
    let disk = ResourceDim::DiskIo.index();
    assert_eq!(c.greedy_residual_guard_blocked.len(), 4);
    assert!(c.greedy_residual_guard_blocked.iter().all(|(_, _, d)| *d == disk));
}

#[test]
fn pinned_memory_heavy_numbers() {
    let c = compare_rebalancers(memory_heavy()).unwrap();
    assert_eq!(c.baseline_migrations, 3);
    // Multi-pair coverage triples the migrations and lifts the ratio to ~0.30…
    assert_eq!(c.greedy_migrations, 9);
    assert!(c.greedy_ratio > 0.29 && c.greedy_ratio < 0.31);
    // …and here it also materially lowers the actual gauge.
    assert!(
        c.greedy_gauge < c.baseline_gauge - 0.05,
        "greedy should cut the gauge: {} -> {}",
        c.baseline_gauge,
        c.greedy_gauge
    );
    // Residue: all on Cpu (a share -> elastic).
    let cpu = ResourceDim::Cpu.index();
    assert_eq!(c.greedy_residual_guard_blocked.len(), 4);
    assert!(c.greedy_residual_guard_blocked.iter().all(|(_, _, d)| *d == cpu));
}
