//! Convex power *evaluation* over fleet state.
//!
//! Parallels `gauge_eval.rs`: the `Power` types live in the dictionary
//! (`src/power.rs`); the functions that evaluate draw over fleet/utilization
//! state live here (laboratory, pure). These are plain laboratory functions,
//! deliberately *not* part of the sealed `SchurConvex` gauge family or the
//! `Safe` typestate — the twin rechecks the budget at `Place` apply time as
//! a runtime convention (plan decision #2).

use fulcrum_dictionary::load::Fleet;
use fulcrum_dictionary::power::{Power, PowerCoeffs};

/// Fixed convexity exponent. Only convexity (any exponent > 1) is
/// load-bearing — it is what makes fleet power Schur-convex, so a
/// Pigou-Dalton transfer cannot increase it. The exact value 2 is pinned,
/// not a tunable knob: it only scales magnitudes, and isn't universally
/// fixable anyway (it depends on the CPU governor regime). See plan
/// decision #2.
const POWER_EXPONENT: f64 = 2.0;

/// Single-node convex draw: `idle + dynamic · worst_utilization²`.
///
/// `worst_utilization` is the per-machine scalar the component-wise gauges
/// already use (`MachineSpec::worst_utilization`).
pub fn node_power(worst_utilization: f64, coeffs: &PowerCoeffs) -> Power {
    coeffs.idle + coeffs.dynamic * worst_utilization.powf(POWER_EXPONENT)
}

/// Total fleet draw: the convex per-node draw summed across the fleet.
///
/// `coeffs` is indexed parallel to `fleet.iter()` (ascending `MachineId`);
/// `cluster::Topology::coeffs` produces a slice with exactly that ordering.
/// Nodes beyond the length of `coeffs` are skipped by the `zip` — callers
/// must supply one coefficient per node.
pub fn fleet_power<const N: usize>(fleet: &Fleet<N>, coeffs: &[PowerCoeffs]) -> Power {
    fleet
        .iter()
        .zip(coeffs.iter())
        .map(|((_, spec), c)| node_power(spec.worst_utilization(), c))
        .sum()
}
