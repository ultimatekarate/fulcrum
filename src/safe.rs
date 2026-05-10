//! `Safe<G>`: the typestate carrying the proof that `g(fleet) ≤ τ`.
//!
//! Constructed once via runtime check in [`Safe::new`] (for unit gauges
//! with `Default`) or [`Safe::with_gauge`] (for parameterized gauges like
//! [`crate::gauge::WeightedKyFan`]). Subsequent moves are applied by
//! consuming `Safe<G>` and returning a new one. For typed-pure moves
//! (`Remove`, `HotToCold`, `Neutral`), `apply` is total — no `Result`,
//! no error path. For catch-all moves (`Place`, `ColdToHot`), `apply` is
//! fallible — the runtime check is at the apply site, visible to readers.

use crate::gauge::SchurConvex;
use crate::load::{Fleet, FleetError};
use crate::move_kind::{ColdToHot, HotToCold, Neutral, Place, Remove};

/// Errors arising from `Safe<G>` operations.
#[derive(Debug, PartialEq)]
pub enum GaugeError {
    /// Construction or fallible apply found `g(fleet) > τ`.
    ThresholdExceeded { value: f64, threshold: f64 },
    /// A move referenced a machine not in the fleet.
    UnknownMachine,
    /// A `Remove` was applied to a machine without enough load. This
    /// indicates a well-formedness bug, not a gauge violation.
    InsufficientLoad,
}

impl From<FleetError> for GaugeError {
    fn from(e: FleetError) -> Self {
        match e {
            FleetError::UnknownMachine(_) => GaugeError::UnknownMachine,
            FleetError::InsufficientLoad { .. } => GaugeError::InsufficientLoad,
        }
    }
}

/// Type-level claim: `g(fleet) ≤ threshold`.
///
/// Constructed only via [`Safe::new`] or [`Safe::with_gauge`], either of
/// which performs the runtime check once. Operations that produce a
/// `Safe<G>` from another `Safe<G>` are either total (typed-pure: by
/// mathematical argument, no re-check needed) or fallible (catch-all:
/// re-check at apply time). The gauge instance is preserved across
/// applies, so `WeightedKyFan` weights stay attached to the typestate.
#[derive(Debug)]
pub struct Safe<G: SchurConvex> {
    fleet: Fleet,
    threshold: f64,
    gauge: G,
}

impl<G: SchurConvex> Safe<G> {
    /// Construct `Safe<G>` from a fleet and an explicit gauge instance,
    /// runtime-checking `gauge.eval(fleet) ≤ τ`. Use this when the gauge
    /// carries runtime data (e.g., `WeightedKyFan` with custom weights).
    /// For unit gauges, prefer [`Safe::new`].
    pub fn with_gauge(
        fleet: Fleet,
        threshold: f64,
        gauge: G,
    ) -> Result<Self, GaugeError> {
        let value = gauge.eval(&fleet);
        if value <= threshold {
            Ok(Safe { fleet, threshold, gauge })
        } else {
            Err(GaugeError::ThresholdExceeded { value, threshold })
        }
    }

    /// Borrow the fleet for read-only inspection (e.g., constructing
    /// witnesses). Mutation is impossible through this borrow.
    pub fn fleet(&self) -> &Fleet {
        &self.fleet
    }

    /// The threshold this `Safe<G>` is bound by.
    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    /// Borrow the gauge instance. Useful for inspecting parameters
    /// (e.g., `WeightedKyFan::weights`).
    pub fn gauge_ref(&self) -> &G {
        &self.gauge
    }

    /// Current gauge value. Cheap inspection; doesn't consume the safe.
    pub fn gauge(&self) -> f64 {
        self.gauge.eval(&self.fleet)
    }

    /// Crate-internal: rebuild a `Safe<G>` after a typed-pure move. Does
    /// not re-check the gauge — the move's totality argument is the proof.
    fn rebuild_total(fleet: Fleet, threshold: f64, gauge: G) -> Self {
        Safe { fleet, threshold, gauge }
    }
}

impl<G: SchurConvex + Default> Safe<G> {
    /// Convenience constructor for unit gauges (those implementing
    /// `Default`). Equivalent to `Safe::with_gauge(fleet, threshold,
    /// G::default())`.
    pub fn new(fleet: Fleet, threshold: f64) -> Result<Self, GaugeError> {
        Self::with_gauge(fleet, threshold, G::default())
    }
}

// ----------------- typed-pure applies (total) -----------------

impl Remove {
    /// Apply `Remove` to a `Safe<G>`. Total: no `Result`.
    ///
    /// Soundness: `Remove` is mass-decreasing, so the new load vector is
    /// componentwise ≤ the old, hence majorized by the old, hence `g(new) ≤
    /// g(old) ≤ τ` for any Schur-convex `g`.
    ///
    /// The internal `expect` paths trigger only on well-formedness errors
    /// (unknown machine, insufficient load). They are not gauge violations.
    /// In normal use, callers construct `Remove` from a fleet snapshot they
    /// own; the borrow checker prevents the fleet from changing under them.
    pub fn apply<G: SchurConvex>(self, safe: Safe<G>) -> Safe<G> {
        let mut fleet = safe.fleet;
        fleet
            .remove_load(self.machine, self.mass)
            .expect("Remove: well-formedness violated (unknown machine or insufficient load)");
        Safe::rebuild_total(fleet, safe.threshold, safe.gauge)
    }
}

impl HotToCold {
    /// Apply a Pigou-Dalton transfer. Total: no `Result`.
    ///
    /// Soundness: the witness construction guarantees `util(src) > util(dst)`
    /// and the transferred mass preserves the order. This is the canonical
    /// Pigou-Dalton step, which is majorization-decreasing. Schur-convex
    /// gauges are non-increasing under majorization-decreasing operations,
    /// so `g(new) ≤ g(old) ≤ τ`.
    pub fn apply<G: SchurConvex>(self, safe: Safe<G>) -> Safe<G> {
        let mut fleet = safe.fleet;
        fleet
            .remove_load(self.source, self.mass)
            .expect("HotToCold: witness invariant violated (source)");
        fleet
            .add_load(self.destination, self.mass)
            .expect("HotToCold: witness invariant violated (destination)");
        Safe::rebuild_total(fleet, safe.threshold, safe.gauge)
    }
}

impl Neutral {
    /// Apply a mass-preserving migration between equally-utilized machines.
    /// Total: no `Result`.
    ///
    /// Soundness: source and destination have equal load (under uniform
    /// capacity, equal utilization). The new load vector is a transposition
    /// of the old vector's coordinates within the equal-utilization class.
    /// Symmetric gauges are invariant under coordinate permutation; in
    /// particular, the Schur-convex gauges supported here are symmetric, so
    /// `g(new) = g(old) ≤ τ`.
    pub fn apply<G: SchurConvex>(self, safe: Safe<G>) -> Safe<G> {
        let mut fleet = safe.fleet;
        fleet
            .remove_load(self.source, self.mass)
            .expect("Neutral: witness invariant violated (source)");
        fleet
            .add_load(self.destination, self.mass)
            .expect("Neutral: witness invariant violated (destination)");
        Safe::rebuild_total(fleet, safe.threshold, safe.gauge)
    }
}

// ----------------- catch-all applies (fallible) -----------------
//
// These methods are deliberately named `apply_with_recheck` rather than
// `apply`. The asymmetry with typed-pure `apply` is the *feature* — it
// makes catch-all sites grep-able by method name in any codebase using
// fulcrum, no external tooling required. Wrapping `apply_with_recheck`
// behind a function that returns `Safe<G>` directly is still possible
// in principle, but the inner method name remains visible to grep.

impl ColdToHot {
    /// Apply an anti-Robin-Hood migration. Fallible: the gauge is re-evaluated
    /// at the end. Named `apply_with_recheck` (rather than `apply`) so every
    /// call site is grep-able for audit: `rg apply_with_recheck` enumerates
    /// every place runtime gauge re-evaluation happens.
    pub fn apply_with_recheck<G: SchurConvex>(
        self,
        safe: Safe<G>,
    ) -> Result<Safe<G>, GaugeError> {
        let mut fleet = safe.fleet;
        fleet.remove_load(self.source, self.mass)?;
        fleet.add_load(self.destination, self.mass)?;
        Safe::with_gauge(fleet, safe.threshold, safe.gauge)
    }
}

impl Place {
    /// Apply a fresh placement. Fallible: the gauge is re-evaluated at the
    /// end. Named `apply_with_recheck` (rather than `apply`) so every call
    /// site is grep-able for audit: `rg apply_with_recheck` enumerates
    /// every place runtime gauge re-evaluation happens.
    pub fn apply_with_recheck<G: SchurConvex>(
        self,
        safe: Safe<G>,
    ) -> Result<Safe<G>, GaugeError> {
        let mut fleet = safe.fleet;
        fleet.add_load(self.machine, self.mass)?;
        Safe::with_gauge(fleet, safe.threshold, safe.gauge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gauge::{Linfty, WeightedKyFan};
    use crate::load::{Fleet, MachineId, Mass};

    fn fleet_3x100_loads(a: u64, b: u64, c: u64) -> Fleet {
        let mut f = Fleet::new();
        f.add_machine(MachineId(1), 100, a);
        f.add_machine(MachineId(2), 100, b);
        f.add_machine(MachineId(3), 100, c);
        f
    }

    #[test]
    fn safe_rejects_threshold_violation() {
        let f = fleet_3x100_loads(95, 50, 50);
        let r: Result<Safe<Linfty>, _> = Safe::new(f, 0.9);
        assert!(matches!(r, Err(GaugeError::ThresholdExceeded { .. })));
    }

    #[test]
    fn remove_preserves_safety() {
        let f = fleet_3x100_loads(80, 50, 50);
        let safe: Safe<Linfty> = Safe::new(f, 0.9).unwrap();
        let safe = Remove::new(MachineId(1), Mass(20)).apply(safe);
        assert!(safe.gauge() <= 0.9);
    }

    #[test]
    fn hot_to_cold_witness_validates_direction() {
        let f = fleet_3x100_loads(80, 50, 50);
        // valid: 1 -> 2 with mass 10
        assert!(HotToCold::witness(MachineId(1), MachineId(2), Mass(10), &f).is_some());
        // invalid: 2 -> 1 (cold to hot)
        assert!(HotToCold::witness(MachineId(2), MachineId(1), Mass(10), &f).is_none());
        // invalid: mass too large (would invert order)
        assert!(HotToCold::witness(MachineId(1), MachineId(2), Mass(40), &f).is_none());
    }

    #[test]
    fn hot_to_cold_apply_preserves_safety() {
        let f = fleet_3x100_loads(80, 50, 50);
        let safe: Safe<Linfty> = Safe::new(f, 0.9).unwrap();
        let m = HotToCold::witness(MachineId(1), MachineId(2), Mass(10), safe.fleet()).unwrap();
        let safe = m.apply(safe);
        assert!(safe.gauge() <= 0.9);
    }

    #[test]
    fn place_can_fail_safety_check() {
        let f = fleet_3x100_loads(85, 50, 50);
        let safe: Safe<Linfty> = Safe::new(f, 0.9).unwrap();
        // adding 10 to machine 1 would push it to 95, gauge = 0.95 > 0.9
        let r = Place::new(MachineId(1), Mass(10)).apply_with_recheck(safe);
        assert!(matches!(r, Err(GaugeError::ThresholdExceeded { .. })));
    }

    #[test]
    fn weighted_kyfan_typestate_threading() {
        // WeightedKyFan can be used as the gauge parameter on `Safe`, and
        // its weights persist across typed-pure applies.
        let f = fleet_3x100_loads(60, 40, 30);
        let weights = WeightedKyFan::new([0.7, 0.3]).unwrap();
        let safe: Safe<WeightedKyFan<2>> = Safe::with_gauge(f, 2.0, weights).unwrap();

        let g_before = safe.gauge();
        // Pigou-Dalton transfer: 1 → 2 with mass 5. Loads become (55, 45, 30).
        let m = HotToCold::witness(MachineId(1), MachineId(2), Mass(5), safe.fleet()).unwrap();
        let safe = m.apply(safe);
        let g_after = safe.gauge();

        // The weights survived through apply.
        assert_eq!(safe.gauge_ref().weights(), &[0.7, 0.3]);
        // Pigou-Dalton transfer must not increase the gauge.
        assert!(g_after <= g_before + 1e-9);
    }
}
