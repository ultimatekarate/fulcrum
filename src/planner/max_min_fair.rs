//! Max-min fair migration planner.
//!
//! Iteratively emits `HotToCold` (Pigou-Dalton) transfers from the
//! highest-utilization machine to the lowest, in the dimension where
//! the source's utilization is worst. Returns `None` when:
//!
//! - the fleet is empty, or
//! - the gap between max and min worst-dim utilization is below `epsilon`,
//!   or
//! - no `HotToCold` witness can be constructed for the chosen pair (e.g.
//!   the per-dim cap restriction `cap(src) ≤ cap(dst)` blocks the move).
//!
//! References: Bertsekas-Gallager (network flow textbook); the canonical
//! analysis of max-min fairness via iterated Pigou-Dalton transfers. The
//! framework's typestate makes each iteration's safety free at compile
//! time — every emitted move is a witness-validated `HotToCold`, hence
//! `apply` is total.
//!
//! ## Mass selection
//!
//! Each emitted transfer moves enough mass to roughly halve the
//! source-side utilization gap in the worst dim. Capped by the witness's
//! per-dim mass condition and by the source's remaining load. A more
//! sophisticated version could use the joint multi-dim majorization
//! theory (Marshall-Olkin §15) to pick mass that targets the *joint*
//! utilization gap across dims; deferred.

use crate::gauge::SchurConvex;
use crate::load::{Fleet, MachineId, Mass};
use crate::move_kind::HotToCold;
use crate::planner::{Planner, TypedMove};
use crate::safe::Safe;

/// What `MaxMinFair` would do with one ordered `(src, dst)` pair — the entire
/// per-pair decision, factored out of [`MaxMinFair::step`] so the planner and
/// any diagnostic share **one** source of truth (the analysis cannot drift
/// from the policy it claims to measure).
///
/// `step` applies this to the single global (max-util, min-util) pair. The
/// stall diagnostics apply it to *every* ordered pair, which is how we tell a
/// genuine convergence apart from a planner that has simply stopped looking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PairVerdict {
    /// Fewer than two machines, or a machine id was not found.
    NoCandidates,
    /// The worst-dim utilization gap `src_u − dst_u` is below ε (or ≤ 0):
    /// nothing worth shedding from `src` to `dst`.
    BelowEpsilon { gap: f64 },
    /// There *is* a gap and the source's binding dimension is `worst_d`, but a
    /// **load-conserving** transfer is blocked by the capacity guard
    /// (`cap_src > cap_dst`, or a zero capacity) in that dimension. This is the
    /// exact seam an *elastic* (utilization-conserving) carrier would dissolve:
    /// on an elastic dim the gauge carrier is conserved directly, so the guard
    /// would not apply and this would become a typed-pure `HotToCold`.
    GuardBlocked {
        worst_d: usize,
        src_u: f64,
        dst_u: f64,
        cap_src: u64,
        cap_dst: u64,
    },
    /// Admissible direction by per-machine worst-util, but in `worst_d`
    /// specifically `src` is already ≤ `dst` (the two machines' binding dims
    /// differ) — no anti-Robin-Hood-free mass to move there.
    BalancedInDim { worst_d: usize },
    /// Admissible, but the halved-gap mass rounds to zero (no integer progress).
    MassZero { worst_d: usize },
    /// A typed-pure `HotToCold` of `mass` on dimension `worst_d` is admissible.
    Emit { worst_d: usize, mass: u64 },
}

impl PairVerdict {
    /// Would the planner emit a typed-pure move for this pair?
    pub fn is_emit(&self) -> bool {
        matches!(self, PairVerdict::Emit { .. })
    }

    /// Is this a capacity-guard stall *with* real imbalance — i.e. a move the
    /// planner wants to make and can't, the population an elastic carrier
    /// targets?
    pub fn is_guard_blocked(&self) -> bool {
        matches!(self, PairVerdict::GuardBlocked { .. })
    }
}

/// The whole per-pair policy of `MaxMinFair`, as a pure function of fleet state.
///
/// This is byte-for-byte the arithmetic that used to live inline in `step`
/// (worst-dim selection, the `cap_src ≤ cap_dst` guard, the exact-integer
/// cross-product gap, the halve-the-gap mass), only named and reusable.
pub fn evaluate_pair<const N: usize>(
    fleet: &Fleet<N>,
    src: MachineId,
    dst: MachineId,
    epsilon: f64,
) -> PairVerdict {
    let (src_spec, dst_spec) = match (fleet.spec(src), fleet.spec(dst)) {
        (Some(s), Some(d)) => (s, d),
        _ => return PairVerdict::NoCandidates,
    };
    let src_u = src_spec.worst_utilization();
    let dst_u = dst_spec.worst_utilization();
    if src == dst || (src_u - dst_u) < epsilon {
        return PairVerdict::BelowEpsilon { gap: src_u - dst_u };
    }

    // src's worst dimension — the one most worth shedding mass from.
    let worst_d = src_spec
        .utilization()
        .0
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(d, _)| d)
        .unwrap_or(0);

    let cap_src = src_spec.capacity[worst_d];
    let cap_dst = dst_spec.capacity[worst_d];
    if cap_src == 0 || cap_dst == 0 || cap_src > cap_dst {
        return PairVerdict::GuardBlocked { worst_d, src_u, dst_u, cap_src, cap_dst };
    }

    // Max admissible per-dim mass (u128 to avoid overflow):
    //   mass · cap_src ≤ load_src · cap_dst − load_dst · cap_src
    let load_src_x_cap_dst = src_spec.load[worst_d] as u128 * cap_dst as u128;
    let load_dst_x_cap_src = dst_spec.load[worst_d] as u128 * cap_src as u128;
    if load_src_x_cap_dst <= load_dst_x_cap_src {
        return PairVerdict::BalancedInDim { worst_d };
    }
    let max_mass_x_cap_src = load_src_x_cap_dst - load_dst_x_cap_src;
    // Halve the gap; minimum of 1 to make progress.
    let chosen_mass_x_cap_src = (max_mass_x_cap_src / 2).max(1);
    let mass_d = (chosen_mass_x_cap_src / cap_src as u128) as u64;
    // Also cap by source's available load.
    let mass_d = mass_d.min(src_spec.load[worst_d]);
    if mass_d == 0 {
        return PairVerdict::MassZero { worst_d };
    }
    PairVerdict::Emit { worst_d, mass: mass_d }
}

pub struct MaxMinFair {
    /// Stop emitting when (max worst-dim util) − (min worst-dim util) < ε.
    epsilon: f64,
    /// Per-`step` verdict log. Opt-in instrumentation: pushed on every call,
    /// never read by the policy, so it cannot change what the planner does.
    /// The last entry of a drained run is the *halt reason*.
    log: Vec<PairVerdict>,
}

impl MaxMinFair {
    /// Construct with the convergence threshold `epsilon`. A smaller ε
    /// means more migrations to converge to a tighter balance; larger ε
    /// means fewer migrations but more residual imbalance. For most
    /// production-shaped fleets, 1e-2 is a reasonable starting point.
    pub fn new(epsilon: f64) -> Self {
        Self { epsilon, log: Vec::new() }
    }

    /// The per-`step` verdict log, in call order. After driving the planner to
    /// exhaustion, the final element is why it stopped.
    pub fn log(&self) -> &[PairVerdict] {
        &self.log
    }
}

impl<const N: usize, G: SchurConvex<N>> Planner<N, G> for MaxMinFair {
    fn step(&mut self, safe: &Safe<G, N>) -> Option<TypedMove<N>> {
        let fleet = safe.fleet();
        if fleet.len() < 2 {
            self.log.push(PairVerdict::NoCandidates);
            return None;
        }

        // Find max-util and min-util machines (worst-dim per machine).
        let mut max_id: Option<(MachineId, f64)> = None;
        let mut min_id: Option<(MachineId, f64)> = None;
        for (id, spec) in fleet.iter() {
            let u = spec.worst_utilization();
            if max_id.map(|(_, mu)| u > mu).unwrap_or(true) {
                max_id = Some((id, u));
            }
            if min_id.map(|(_, mu)| u < mu).unwrap_or(true) {
                min_id = Some((id, u));
            }
        }
        let (src, _) = max_id?;
        let (dst, _) = min_id?;

        // One per-pair decision, shared with the diagnostics.
        let verdict = evaluate_pair(fleet, src, dst, self.epsilon);
        self.log.push(verdict);
        match verdict {
            PairVerdict::Emit { worst_d, mass } => {
                let mut mass_arr = [0u64; N];
                mass_arr[worst_d] = mass;
                // Final witness check — should pass given how mass was chosen,
                // but the witness is the source of truth.
                let m = HotToCold::witness(src, dst, Mass(mass_arr), fleet)?;
                Some(TypedMove::HotToCold(m))
            }
            _ => None,
        }
    }
}
