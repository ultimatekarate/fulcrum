//! # Fulcrum laboratory
//!
//! Pure logic — the verbs of the move algebra. Depends only on
//! `fulcrum-dictionary`. Everything here is `std`-only computation: no IO,
//! no clock, no randomness.
//!
//! Contents:
//! - [`alphabet`] — the sealed `Primitive` trait and the five `Effect`
//!   categories. Sealed *here* (not in the dictionary): an external crate
//!   cannot mint a new primitive, and the `pub(crate)` seal only composes
//!   if the trait and its impls share a crate.
//! - [`gauge`] (+ private `gauge_eval`) — the `Gauge`/`SchurConvex` traits,
//!   the Ky Fan gauge family, and their `eval` bodies. Trait *and* impls
//!   live together: Rust's orphan rule forbids `impl Gauge for SumTopK`
//!   from any crate that owns neither, so the decl/eval split that was a
//!   module boundary cannot be a crate boundary.
//! - [`move_kind`] — the five move kinds and their witness constructors.
//! - [`power_eval`] — `node_power` / `fleet_power` (free functions over
//!   dictionary power data).
//! - [`safe`] — the `Safe<G, N>` typestate and `apply` impls.

pub mod alphabet;
pub mod gauge;
// Private: holds only `impl Gauge<N> for …` eval bodies. Trait impls are in
// scope crate-wide regardless, so nothing needs to name this module path.
mod gauge_eval;
pub mod move_kind;
pub mod power_eval;
pub mod safe;

pub use alphabet::{Derived, Effect, Primitive};
pub use gauge::{Gauge, Linfty, SchurConvex, SumTopK, WeightedKyFan};
pub use move_kind::{ColdToHot, HotToCold, Neutral, Place, Remove};
pub use power_eval::{fleet_power, node_power};
pub use safe::{GaugeError, Safe};
