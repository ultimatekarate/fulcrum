//! # Fulcrum
//!
//! A typed move algebra for load-balancing invariants under Schur-convex
//! gauges.
//!
//! ## What this is for
//!
//! Fulcrum lifts the algebra of majorization into the type system.
//! Operations that preserve a Schur-convex gauge (mass removal, Pigou-
//! Dalton transfers, mass-preserving neutral migrations) are typed as
//! total functions over a `Safe<G, N>` typestate — the type system
//! guarantees the gauge bound is preserved, with no runtime re-evaluation.
//! Operations that *can't* preserve the bound unconditionally (placement
//! of new mass, anti-Robin-Hood migrations) are typed as fallible
//! operations, where the runtime check is unavoidable and visible at
//! every call site.
//!
//! ## Workspace layout
//!
//! Fulcrum is split into crates whose boundaries are the architecture's
//! linguistic layers, enforced as `Cargo.toml` facts:
//!
//! - [`fulcrum_dictionary`] — inert data (load, capacity, power, topology,
//!   trace). No logic, no internal deps.
//! - [`fulcrum_laboratory`] — pure logic: the sealed `Primitive` alphabet,
//!   the Ky Fan gauge family and its evaluation, the move-kind witnesses,
//!   power evaluation, and the `Safe<G, N>` typestate. Depends only on the
//!   dictionary.
//! - `fulcrum` (this crate) — the planners ([`planner`]) and the IO-facing
//!   hands ([`twin`], [`replay`]), plus a flat re-export of the two lower
//!   crates so `fulcrum::Mass`, `fulcrum::Safe`, etc. resolve unchanged.
//!
//! The gauge declaration and its `eval` impls live *together* in the
//! laboratory: Rust's orphan rule forbids `impl Gauge for SumTopK` from any
//! crate owning neither, so the decl/eval separation is a module boundary,
//! never a crate boundary.

// Re-export the lower crates' modules so `fulcrum::load::X`,
// `fulcrum::gauge::Y`, `fulcrum::safe::Z`, … keep resolving.
pub use fulcrum_dictionary::{cluster, load, power, trace};
pub use fulcrum_laboratory::{alphabet, gauge, move_kind, power_eval, safe};

pub mod planner;
pub mod replay;
pub mod twin;

pub use alphabet::{Derived, Effect, Primitive};
pub use cluster::{turing_pi_2, NodeProfile, ResourceDim, Topology};
pub use gauge::{Gauge, Linfty, SchurConvex, SumTopK, WeightedKyFan};
pub use load::{Capacity, Fleet, MachineId, MachineSpec, Mass, Utilization};
pub use move_kind::{ColdToHot, HotToCold, Neutral, Place, Remove};
pub use planner::{
    evaluate_pair, BestFitDecreasing, LeastLoaded, MaxMinFair, MaxMinFairGreedy, PairVerdict,
    Planner, PowerOfTwo, TypedMove,
};
pub use power::{Power, PowerBudget, PowerCoeffs};
pub use power_eval::{fleet_power, node_power};
pub use safe::{GaugeError, Safe};
pub use trace::{MoveHistory, MoveRecord};
pub use twin::{
    compare_rebalancers, diagnose_turing_pi_2_rebalance, greedy_outcome, run_turing_pi_2_twin,
    steady_state_churn, timeline_to_csv, ChurnConfig, ChurnReport, GreedyOutcome,
    RebalanceComparison, RebalanceStallReport, Sim, SimStats, TimelineRow, TwinConfig, TwinReport,
    WorkloadGen,
};
