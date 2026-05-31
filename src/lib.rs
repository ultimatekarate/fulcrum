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
//! ## What's in the current build
//!
//! - Gauge family: [`gauge::SumTopK`] (Ky Fan k-norm) over per-machine
//!   worst-dimension utilization, [`gauge::Linfty`] (alias for
//!   `SumTopK<1, N>`), and [`gauge::WeightedKyFan`] (non-negative linear
//!   combinations).
//! - Five move kinds in [`move_kind`]: `Remove`, `HotToCold`, `Neutral`,
//!   `ColdToHot`, `Place`.
//! - **Heterogeneous per-machine capacity** (Phase 1).
//! - **Multi-dimensional load** (Phase 2): `Fleet<N>`, `Mass<N>`,
//!   `Safe<G, N>`. Component-wise gauge interpretation: per-machine
//!   utilization is reduced by `max_d`, then sorted and Ky Fan-ed across
//!   machines. Joint multi-dim gauges are deferred.
//! - **Reference planners** (Phase 3): [`planner::LeastLoaded`],
//!   [`planner::PowerOfTwo`], [`planner::MaxMinFair`],
//!   [`planner::BestFitDecreasing`]. Each implements the
//!   [`planner::Planner`] trait, emitting a [`planner::TypedMove`] per
//!   `step` call.
//!
//! ## Stability
//!
//! Pre-1.0. Everything is subject to change. See `PLAN.md` for v0 scope
//! and kill criteria.

pub mod alphabet;
pub mod cluster;
pub mod gauge;
// Private: holds only `impl Gauge<N> for …` eval bodies (the laboratory half
// of the gauge decl/eval split). Trait impls are in scope crate-wide
// regardless, so nothing needs to name this module path.
mod gauge_eval;
pub mod load;
pub mod move_kind;
pub mod planner;
pub mod power;
pub mod power_eval;
pub mod replay;
pub mod safe;
pub mod trace;
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
