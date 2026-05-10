//! Planner trait + reference scheduler implementations.
//!
//! Planners decide *which* typed move to emit at each decision point.
//! The framework's algebra (the `Safe<G, N>` typestate, the witness
//! constructions) guarantees the per-move correctness; the planner
//! provides the policy that drives the sequence of moves.
//!
//! ## The trait
//!
//! ```ignore
//! pub trait Planner<const N: usize, G: SchurConvex<N>> {
//!     fn step(&mut self, safe: &Safe<G, N>) -> Option<TypedMove<N>>;
//! }
//! ```
//!
//! `step` returns `Some(move)` to advance the schedule or `None` when the
//! planner has nothing more to emit (queue exhausted, or imbalance below
//! ε for migration-only planners).
//!
//! ## Reference implementations
//!
//! Four planners are provided to demonstrate the pattern and exercise
//! the framework on representative scheduling shapes:
//!
//! | Planner             | Move kind          | Notes                                 |
//! | ------------------- | ------------------ | ------------------------------------- |
//! | [`LeastLoaded`]     | `Place`            | Greedy min-utilization placement      |
//! | [`PowerOfTwo`]      | `Place`            | Sample 2 random sleds; deterministic seeded PRNG |
//! | [`MaxMinFair`]      | `HotToCold`        | Migration-only; iterative Pigou-Dalton |
//! | [`BestFitDecreasing`] | `Place`          | Bin-packing; sort items desc, smallest fit |
//!
//! ## Where they live
//!
//! Planners are in their own layer (per `basis.yaml`), strictly above
//! `laboratory`. They're pure (no IO, no `system_clock`); randomness is
//! deterministically seeded by the constructor so test reproducibility
//! is trivial.

pub mod best_fit_decreasing;
pub mod least_loaded;
pub mod max_min_fair;
pub mod power_of_two;

pub use best_fit_decreasing::BestFitDecreasing;
pub use least_loaded::LeastLoaded;
pub use max_min_fair::MaxMinFair;
pub use power_of_two::PowerOfTwo;

use crate::gauge::SchurConvex;
use crate::move_kind::{ColdToHot, HotToCold, Neutral, Place, Remove};
use crate::safe::{GaugeError, Safe};

/// A typed move emitted by a planner. Each variant wraps one of the
/// five move kinds in the alphabet.
///
/// The variants mirror the five `Primitive` impls. Adding a new move
/// kind requires touching this union (basis.yaml lists `TypedMove` as
/// an exhaustive_matching union — the build will fail if a new variant
/// is added without updating every match site).
pub enum TypedMove<const N: usize> {
    Remove(Remove<N>),
    HotToCold(HotToCold<N>),
    Neutral(Neutral<N>),
    ColdToHot(ColdToHot<N>),
    Place(Place<N>),
}

impl<const N: usize> TypedMove<N> {
    /// Apply the move, threading the `Safe<G, N>` typestate through.
    ///
    /// Returns `Result` for uniform handling at call sites: the typed-
    /// pure variants (Remove, HotToCold, Neutral) are infallible at the
    /// type level (their underlying `apply` returns `Safe` directly),
    /// and the catch-all variants (ColdToHot, Place) re-check the gauge.
    /// Wrapping the typed-pure result in `Ok` is a no-op at runtime.
    pub fn apply<G: SchurConvex<N>>(
        self,
        safe: Safe<G, N>,
    ) -> Result<Safe<G, N>, GaugeError> {
        match self {
            TypedMove::Remove(m) => Ok(m.apply(safe)),
            TypedMove::HotToCold(m) => Ok(m.apply(safe)),
            TypedMove::Neutral(m) => Ok(m.apply(safe)),
            TypedMove::ColdToHot(m) => m.apply_with_recheck(safe),
            TypedMove::Place(m) => m.apply_with_recheck(safe),
        }
    }

    /// Whether this move's underlying primitive is typed-pure (its apply
    /// is total at the type level). Useful for instrumentation: counting
    /// the typed-pure ratio across a planner's emitted sequence.
    pub fn is_typed_pure(&self) -> bool {
        matches!(
            self,
            TypedMove::Remove(_) | TypedMove::HotToCold(_) | TypedMove::Neutral(_)
        )
    }

    /// Human-readable kind name. Useful for diagnostics.
    pub fn kind_name(&self) -> &'static str {
        match self {
            TypedMove::Remove(_) => "Remove",
            TypedMove::HotToCold(_) => "HotToCold",
            TypedMove::Neutral(_) => "Neutral",
            TypedMove::ColdToHot(_) => "ColdToHot",
            TypedMove::Place(_) => "Place",
        }
    }
}

/// A scheduler.
///
/// `step` is the unit of decision: given the current state, emit a single
/// typed move (or `None` if the planner has nothing more to do).
///
/// Some planners are queue-driven (they store work to place internally
/// and emit `Place` moves until empty); others are state-driven (they
/// look at fleet imbalance and emit migrations until balanced). The
/// trait is agnostic between these styles.
///
/// Planners are generic over the gauge `G` and dimension `N`. Most
/// reference planners don't read the gauge value directly — they make
/// decisions from the fleet state alone — but they consume `Safe<G, N>`
/// to participate in the typestate threading.
pub trait Planner<const N: usize, G: SchurConvex<N>> {
    /// Emit the next move, or `None` if the planner is exhausted.
    fn step(&mut self, safe: &Safe<G, N>) -> Option<TypedMove<N>>;
}
