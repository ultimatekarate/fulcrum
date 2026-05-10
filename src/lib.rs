//! # Fulcrum
//!
//! A typed move algebra for load-balancing invariants under Schur-convex
//! gauges.
//!
//! ## What this is for
//!
//! Fulcrum lifts the algebra of majorization into the type system. Operations
//! that preserve a Schur-convex gauge (mass removal, Pigou-Dalton transfers,
//! mass-preserving neutral migrations) are typed as total functions over a
//! `Safe<G>` typestate — the type system guarantees the gauge bound is
//! preserved, with no runtime re-evaluation. Operations that *can't* preserve
//! the bound unconditionally (placement of new mass, anti-Robin-Hood
//! migrations) are typed as fallible operations, where the runtime check is
//! unavoidable and visible at every call site.
//!
//! ## What's in v0
//!
//! - One concrete gauge: [`gauge::Linfty`] (max utilization).
//! - Five move kinds in [`move_kind`]: `Remove`, `HotToCold`, `Neutral`,
//!   `ColdToHot`, `Place`.
//! - Single-dimensional load with uniform capacity. Heterogeneous and
//!   multi-dimensional fleets are future work.
//!
//! ## Stability
//!
//! Pre-1.0. Everything is subject to change. See `PLAN.md` for v0 scope and
//! kill criteria.

pub mod alphabet;
pub mod gauge;
pub mod load;
pub mod move_kind;
pub mod replay;
pub mod safe;
pub mod trace;

pub use alphabet::{Derived, Effect, Primitive};
pub use gauge::{Gauge, Linfty, SchurConvex, SumTopK};
pub use load::{Fleet, MachineId, Mass};
pub use move_kind::{ColdToHot, HotToCold, Neutral, Place, Remove};
pub use safe::{GaugeError, Safe};
pub use trace::{MoveHistory, MoveRecord};
