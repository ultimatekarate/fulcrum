//! `Safe<G, N>`: the typestate carrying the proof that `g(fleet) ≤ τ`
//! over an N-dimensional fleet.
//!
//! Constructed once via runtime check in [`Safe::new`] (for unit gauges
//! with `Default`) or [`Safe::with_gauge`] (for parameterized gauges like
//! [`crate::gauge::WeightedKyFan`]). Subsequent moves are applied by
//! consuming `Safe` and returning a new one. For typed-pure moves
//! (`Remove`, `HotToCold`, `Neutral`), `apply` is total — no `Result`,
//! no error path. For catch-all moves (`Place`, `ColdToHot`), `apply` is
//! fallible — the runtime check is at the apply site, visible to readers.

use crate::gauge::SchurConvex;
use crate::load::{Fleet, FleetError};
use crate::move_kind::{ColdToHot, HotToCold, Neutral, Place, Remove};

/// Errors arising from `Safe<G, N>` operations.
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
/// `Safe<G, N>` from another `Safe<G, N>` are either total (typed-pure:
/// by mathematical argument, no re-check needed) or fallible (catch-all:
/// re-check at apply time).
#[derive(Debug)]
pub struct Safe<G: SchurConvex<N>, const N: usize> {
    fleet: Fleet<N>,
    threshold: f64,
    gauge: G,
}

impl<G: SchurConvex<N>, const N: usize> Safe<G, N> {
    /// Construct `Safe` from a fleet and an explicit gauge instance,
    /// runtime-checking `gauge.eval(fleet) ≤ τ`.
    pub fn with_gauge(
        fleet: Fleet<N>,
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
    pub fn fleet(&self) -> &Fleet<N> {
        &self.fleet
    }

    /// The threshold this `Safe` is bound by.
    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    /// Borrow the gauge instance.
    pub fn gauge_ref(&self) -> &G {
        &self.gauge
    }

    /// Current gauge value. Cheap inspection; doesn't consume the safe.
    pub fn gauge(&self) -> f64 {
        self.gauge.eval(&self.fleet)
    }

    /// Crate-internal: rebuild after a typed-pure move. Does not re-check
    /// the gauge — the move's totality argument is the proof.
    fn rebuild_total(fleet: Fleet<N>, threshold: f64, gauge: G) -> Self {
        Safe { fleet, threshold, gauge }
    }
}

impl<G: SchurConvex<N> + Default, const N: usize> Safe<G, N> {
    /// Convenience constructor for unit gauges (those implementing
    /// `Default`).
    pub fn new(fleet: Fleet<N>, threshold: f64) -> Result<Self, GaugeError> {
        Self::with_gauge(fleet, threshold, G::default())
    }
}

// ----------------- typed-pure applies (total) -----------------

impl<const N: usize> Remove<N> {
    /// Apply `Remove` to a `Safe`. Total: no `Result`.
    pub fn apply<G: SchurConvex<N>>(self, safe: Safe<G, N>) -> Safe<G, N> {
        let mut fleet = safe.fleet;
        fleet
            .remove_load(self.machine, self.mass)
            .expect("Remove: well-formedness violated (unknown machine or insufficient load)");
        Safe::rebuild_total(fleet, safe.threshold, safe.gauge)
    }
}

impl<const N: usize> HotToCold<N> {
    /// Apply a Pigou-Dalton transfer. Total: no `Result`.
    pub fn apply<G: SchurConvex<N>>(self, safe: Safe<G, N>) -> Safe<G, N> {
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

impl<const N: usize> Neutral<N> {
    /// Apply a mass-preserving migration between equally-utilized
    /// machines. Total: no `Result`.
    pub fn apply<G: SchurConvex<N>>(self, safe: Safe<G, N>) -> Safe<G, N> {
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
// `apply_with_recheck` (rather than `apply`) is the deliberate naming so
// every catch-all site is grep-able by method name.

impl<const N: usize> ColdToHot<N> {
    pub fn apply_with_recheck<G: SchurConvex<N>>(
        self,
        safe: Safe<G, N>,
    ) -> Result<Safe<G, N>, GaugeError> {
        let mut fleet = safe.fleet;
        fleet.remove_load(self.source, self.mass)?;
        fleet.add_load(self.destination, self.mass)?;
        Safe::with_gauge(fleet, safe.threshold, safe.gauge)
    }
}

impl<const N: usize> Place<N> {
    pub fn apply_with_recheck<G: SchurConvex<N>>(
        self,
        safe: Safe<G, N>,
    ) -> Result<Safe<G, N>, GaugeError> {
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

    fn fleet_3x100_loads(a: u64, b: u64, c: u64) -> Fleet<1> {
        let mut f = Fleet::new();
        f.add_machine(MachineId(1), [100], [a]);
        f.add_machine(MachineId(2), [100], [b]);
        f.add_machine(MachineId(3), [100], [c]);
        f
    }

    #[test]
    fn safe_rejects_threshold_violation() {
        let f = fleet_3x100_loads(95, 50, 50);
        let r: Result<Safe<Linfty<1>, 1>, _> = Safe::new(f, 0.9);
        assert!(matches!(r, Err(GaugeError::ThresholdExceeded { .. })));
    }

    #[test]
    fn remove_preserves_safety() {
        let f = fleet_3x100_loads(80, 50, 50);
        let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.9).unwrap();
        let safe = Remove::new(MachineId(1), Mass([20])).apply(safe);
        assert!(safe.gauge() <= 0.9);
    }

    #[test]
    fn hot_to_cold_witness_validates_direction() {
        let f = fleet_3x100_loads(80, 50, 50);
        assert!(HotToCold::witness(MachineId(1), MachineId(2), Mass([10]), &f).is_some());
        assert!(HotToCold::witness(MachineId(2), MachineId(1), Mass([10]), &f).is_none());
        assert!(HotToCold::witness(MachineId(1), MachineId(2), Mass([40]), &f).is_none());
    }

    #[test]
    fn hot_to_cold_apply_preserves_safety() {
        let f = fleet_3x100_loads(80, 50, 50);
        let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.9).unwrap();
        let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([10]), safe.fleet()).unwrap();
        let safe = m.apply(safe);
        assert!(safe.gauge() <= 0.9);
    }

    #[test]
    fn place_can_fail_safety_check() {
        let f = fleet_3x100_loads(85, 50, 50);
        let safe: Safe<Linfty<1>, 1> = Safe::new(f, 0.9).unwrap();
        let r = Place::new(MachineId(1), Mass([10])).apply_with_recheck(safe);
        assert!(matches!(r, Err(GaugeError::ThresholdExceeded { .. })));
    }

    #[test]
    fn weighted_kyfan_typestate_threading() {
        let f = fleet_3x100_loads(60, 40, 30);
        let weights = WeightedKyFan::<2, 1>::new([0.7, 0.3]).unwrap();
        let safe: Safe<WeightedKyFan<2, 1>, 1> = Safe::with_gauge(f, 2.0, weights).unwrap();

        let g_before = safe.gauge();
        let m = HotToCold::witness(MachineId(1), MachineId(2), Mass([5]), safe.fleet()).unwrap();
        let safe = m.apply(safe);
        let g_after = safe.gauge();

        assert_eq!(safe.gauge_ref().weights(), &[0.7, 0.3]);
        assert!(g_after <= g_before + 1e-9);
    }
}
