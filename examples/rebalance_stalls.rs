//! Rebalance-stall experiment.
//!
//! Tests the falsifiable core of the elastic-carrier hypothesis on the real
//! Turing Pi 2 twin scenario: when the `MaxMinFair` rebalance pass stops, is it
//! because the fleet is *balanced* (carrier work would be immaterial), or
//! because the capacity guard *blocks* it while imbalance remains (typed-pure
//! rebalancing is being suppressed)? And on which dimension — telling an
//! *elastic carrier* fix apart from a mere *planner-coverage* fix?
//!
//! Run: `cargo run --example rebalance_stalls`

use fulcrum::{
    diagnose_turing_pi_2_rebalance, Power, PowerBudget, RebalanceStallReport, TwinConfig,
    PairVerdict,
};

/// Resource-dimension labels, parallel to `ResourceDim` (Cpu/Mem/DiskIo/NetIo).
const DIM: [&str; 4] = ["Cpu", "Mem", "DiskIo", "NetIo"];

fn fmt_verdict(v: &PairVerdict) -> String {
    match v {
        PairVerdict::NoCandidates => "NoCandidates".to_string(),
        PairVerdict::BelowEpsilon { gap } => format!("BelowEpsilon(gap={gap:+.4})"),
        PairVerdict::GuardBlocked { worst_d, src_u, dst_u, cap_src, cap_dst } => format!(
            "GuardBlocked[{}] src_u={:.3} dst_u={:.3} cap {} -> {} (cap_src>cap_dst)",
            DIM[*worst_d], src_u, dst_u, cap_src, cap_dst
        ),
        PairVerdict::BalancedInDim { worst_d } => format!("BalancedInDim[{}]", DIM[*worst_d]),
        PairVerdict::MassZero { worst_d } => format!("MassZero[{}]", DIM[*worst_d]),
        PairVerdict::Emit { worst_d, mass } => format!("Emit[{}] mass={}", DIM[*worst_d], mass),
    }
}

fn main() {
    // Both canonical scenarios. The headline 0.12 ratio everyone quotes is the
    // `cluster_twin` example's config (0xC0FFEE); the determinism test uses a
    // memory-heavier mix (0xABCDEF). Run the experiment on both.
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
            "determinism test (tests/cluster_twin.rs)",
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
        run_scenario(name, config);
        println!("\n{}\n", "=".repeat(72));
    }
}

fn run_scenario(name: &str, config: TwinConfig) {
    let RebalanceStallReport {
        phase_b_log,
        migrations_applied,
        halt_scan,
        halted_gauge,
        halted_per_node,
    } = diagnose_turing_pi_2_rebalance(config).expect("empty fleet cannot exceed tau");

    println!("== Rebalance-stall experiment: {name} (seed {:#x}) ==\n", config.seed);

    println!("Fleet at the moment Phase B halted (gauge = {halted_gauge:.4}):");
    for (id, u) in &halted_per_node {
        println!("  machine {:?}: worst-util {:.3}", id.0, u);
    }
    println!();

    // --- What the real Phase-B run did, step by step ---
    println!("Phase B (MaxMinFair) per-step verdicts:");
    for (i, v) in phase_b_log.iter().enumerate() {
        let last = i + 1 == phase_b_log.len();
        println!("  step {:>2}: {}{}", i + 1, fmt_verdict(v), if last { "   <- HALT" } else { "" });
    }
    println!("\n  migrations applied (typed-pure HotToCold): {migrations_applied}");
    let halt_reason = phase_b_log.last().copied();
    match halt_reason {
        Some(PairVerdict::BelowEpsilon { gap }) => println!(
            "  halt reason: CONVERGED (global max/min gap {gap:+.4} < epsilon)\n"
        ),
        Some(PairVerdict::GuardBlocked { worst_d, src_u, dst_u, .. }) => println!(
            "  halt reason: GUARD-BLOCKED on {} with residual gap {:+.4} (src_u {:.3} > dst_u {:.3})\n",
            DIM[worst_d], src_u - dst_u, src_u, dst_u
        ),
        Some(other) => println!("  halt reason: {}\n", fmt_verdict(&other)),
        None => println!("  halt reason: (no steps)\n"),
    }

    // --- Counterfactual: all ordered pairs against the halted fleet ---
    let mut emit = 0usize;
    let mut below_eps = 0usize;
    let mut balanced = 0usize;
    let mut mass_zero = 0usize;
    let mut guard_by_dim = [0usize; 4];
    let mut guard_total = 0usize;
    for (_s, _d, v) in &halt_scan {
        match v {
            PairVerdict::Emit { .. } => emit += 1,
            PairVerdict::BelowEpsilon { .. } => below_eps += 1,
            PairVerdict::BalancedInDim { .. } => balanced += 1,
            PairVerdict::MassZero { .. } => mass_zero += 1,
            PairVerdict::GuardBlocked { worst_d, .. } => {
                guard_total += 1;
                guard_by_dim[*worst_d] += 1;
            }
            PairVerdict::NoCandidates => {}
        }
    }

    println!("All {} ordered (src,dst) pairs re-evaluated against the halted fleet:", halt_scan.len());
    println!("  Emit          (typed-pure move available, planner left it): {emit}");
    println!("  GuardBlocked  (imbalance, but cap_src>cap_dst):             {guard_total}");
    for d in 0..4 {
        if guard_by_dim[d] > 0 {
            println!("                  - on {:<7}: {}", DIM[d], guard_by_dim[d]);
        }
    }
    println!("  BalancedInDim (binding dim differs, nothing to shed):       {balanced}");
    println!("  BelowEpsilon  (gap < epsilon or wrong direction):           {below_eps}");
    println!("  MassZero      (admissible but rounds to 0):                 {mass_zero}");
    println!();

    // Show the guard-blocked pairs explicitly — these are the suppressed moves.
    if guard_total > 0 {
        println!("Guard-blocked pairs (the population an elastic carrier would unblock):");
        for (s, d, v) in &halt_scan {
            if let PairVerdict::GuardBlocked { worst_d, src_u, dst_u, cap_src, cap_dst } = v {
                println!(
                    "  {:?} -> {:?}: binding={:<7} gap={:+.4}  cap {} -> {}",
                    s.0, d.0, DIM[*worst_d], src_u - dst_u, cap_src, cap_dst
                );
            }
        }
        println!();
    }

    // --- Verdict ---
    println!("== Interpretation ==");
    let converged = matches!(halt_reason, Some(PairVerdict::BelowEpsilon { .. }));
    if converged && guard_total == 0 && emit == 0 {
        println!("FALSIFIED: Phase B converged and no pair has a suppressed move.");
        println!("The elastic carrier would be right-but-immaterial in this scenario.");
    } else {
        if emit > 0 {
            println!(
                "PLANNER-COVERAGE signal: {emit} typed-pure move(s) were available at the halt\n\
                 that single-global-pair MaxMinFair never tried. A multi-pair planner would\n\
                 lift the ratio with NO carrier change."
            );
        }
        if guard_total > 0 {
            let dims: Vec<&str> = (0..4).filter(|&d| guard_by_dim[d] > 0).map(|d| DIM[d]).collect();
            println!(
                "CARRIER signal: {guard_total} pair(s) have real imbalance but are capacity-guard\n\
                 blocked, on dimension(s): {dims:?}. On any of these that is semantically *elastic*\n\
                 (a share, not a quantity), the guard is spurious and the move would be typed-pure."
            );
        }
        if !converged {
            println!(
                "Phase B did NOT converge — it stopped with residual imbalance. The 0.12 ratio is\n\
                 (partly) suppression, not balance."
            );
        }
    }
}
