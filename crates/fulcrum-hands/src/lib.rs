//! # Fulcrum hands
//!
//! The IO-facing layer — the *simulation* hands. This is where the move
//! algebra meets the outside world in offline form: the deterministic
//! cluster twin ([`twin`]) and Borg-trace replay ([`replay`]). May depend
//! on every lower layer (dictionary, laboratory, planners) — the twin
//! drives the planners.
//!
//! These are the *simulated* shell. The *production* shell that drives real
//! hardware (a sled-agent obeying a controller) lives in its own crates;
//! the twin is its permanent deterministic test double.

pub mod replay;
pub mod twin;

pub use twin::{
    compare_rebalancers, diagnose_turing_pi_2_rebalance, greedy_outcome, run_turing_pi_2_twin,
    steady_state_churn, timeline_to_csv, ChurnConfig, ChurnReport, GreedyOutcome,
    RebalanceComparison, RebalanceStallReport, Sim, SimStats, TimelineRow, TwinConfig, TwinReport,
    WorkloadGen,
};
