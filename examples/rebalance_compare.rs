//! Multi-pair rebalance comparison — following the stall experiment's lead.
//!
//! The stall experiment (`examples/rebalance_stalls.rs`) showed `MaxMinFair`
//! halts the moment the single global (max,min) pair is capacity-guard blocked,
//! leaving admissible typed-pure moves untried. This runs the multi-pair
//! `MaxMinFairGreedy` against that baseline on the same post-placement fleet and
//! reports:
//!
//!   * the lift in typed-pure migrations and the typed-pure *ratio*, and
//!   * the residue that stays guard-blocked AFTER greedy exhausts every
//!     admissible move — the irreducible *carrier* opportunity (planner coverage
//!     is spent, so what remains is purely about absolute-vs-elastic carriers).
//!
//! Run: `cargo run --example rebalance_compare`

use fulcrum::{compare_rebalancers, Power, PowerBudget, RebalanceComparison, TwinConfig};

const DIM: [&str; 4] = ["Cpu", "Mem", "DiskIo", "NetIo"];

fn main() {
    let scenarios = [
        (
            "headline (examples/cluster_twin.rs)",
            TwinConfig {
                seed: 0xC0FFEE,
                n_workloads: 24,
                max_mass: [120, 1200, 150, 150],
                threshold: 0.95,
                budget: PowerBudget(Power(32_000.0)),
                rebalance_epsilon: 1e-3,
            },
        ),
        (
            "memory-heavy (tests/cluster_twin.rs)",
            TwinConfig {
                seed: 0xABCDEF,
                n_workloads: 30,
                max_mass: [150, 1500, 150, 150],
                threshold: 0.95,
                budget: PowerBudget(Power(32_000.0)),
                rebalance_epsilon: 1e-3,
            },
        ),
    ];

    for (name, config) in scenarios {
        let RebalanceComparison {
            baseline_migrations,
            baseline_ratio,
            baseline_gauge,
            greedy_migrations,
            greedy_ratio,
            greedy_gauge,
            greedy_residual_guard_blocked,
            greedy_residual_emits,
        } = compare_rebalancers(config).expect("empty fleet within threshold");

        println!("== {name} (seed {:#x}) ==\n", config.seed);
        println!(
            "  {:<26}{:>12}{:>12}",
            "", "MaxMinFair", "Greedy"
        );
        println!(
            "  {:<26}{:>12}{:>12}",
            "typed-pure migrations", baseline_migrations, greedy_migrations
        );
        println!(
            "  {:<26}{:>12.3}{:>12.3}",
            "typed-pure ratio", baseline_ratio, greedy_ratio
        );
        println!(
            "  {:<26}{:>12.3}{:>12.3}",
            "load gauge (Linfty)", baseline_gauge, greedy_gauge
        );
        println!();

        println!(
            "  After greedy exhausts: {} Emit pair(s) left (planner coverage spent),",
            greedy_residual_emits
        );
        println!(
            "  {} pair(s) still capacity-guard blocked (the CARRIER residue):",
            greedy_residual_guard_blocked.len()
        );
        let mut by_dim = [0usize; 4];
        for (s, d, wd) in &greedy_residual_guard_blocked {
            by_dim[*wd] += 1;
            println!("      {:?} -> {:?}  binding = {}", s.0, d.0, DIM[*wd]);
        }
        let residue_dims: Vec<&str> = (0..4).filter(|&d| by_dim[d] > 0).map(|d| DIM[d]).collect();
        println!();
        if greedy_residual_guard_blocked.is_empty() {
            println!("  => Greedy fully balanced the fleet. No carrier residue in this scenario.");
        } else {
            println!(
                "  => Carrier residue is on {residue_dims:?}. These move only if that dimension is\n     made *elastic*; planner coverage cannot touch them."
            );
        }
        println!("\n{}\n", "=".repeat(72));
    }
}
