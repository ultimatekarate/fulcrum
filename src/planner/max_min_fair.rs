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
use crate::load::{MachineId, Mass};
use crate::move_kind::HotToCold;
use crate::planner::{Planner, TypedMove};
use crate::safe::Safe;

pub struct MaxMinFair {
    /// Stop emitting when (max worst-dim util) − (min worst-dim util) < ε.
    epsilon: f64,
}

impl MaxMinFair {
    /// Construct with the convergence threshold `epsilon`. A smaller ε
    /// means more migrations to converge to a tighter balance; larger ε
    /// means fewer migrations but more residual imbalance. For most
    /// production-shaped fleets, 1e-2 is a reasonable starting point.
    pub fn new(epsilon: f64) -> Self {
        Self { epsilon }
    }
}

impl<const N: usize, G: SchurConvex<N>> Planner<N, G> for MaxMinFair {
    fn step(&mut self, safe: &Safe<G, N>) -> Option<TypedMove<N>> {
        let fleet = safe.fleet();
        if fleet.len() < 2 {
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
        let (src, src_u) = max_id?;
        let (dst, dst_u) = min_id?;
        if src == dst || (src_u - dst_u) < self.epsilon {
            return None;
        }

        // Identify src's worst dimension — that's the one most worth
        // shedding mass from.
        let src_spec = fleet.spec(src)?;
        let dst_spec = fleet.spec(dst)?;
        let worst_d = src_spec
            .utilization()
            .0
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(d, _)| d)?;

        let cap_src = src_spec.capacity[worst_d];
        let cap_dst = dst_spec.capacity[worst_d];
        if cap_src == 0 || cap_dst == 0 || cap_src > cap_dst {
            // Witness will reject — abandon this pair. Returning None
            // gives up on rebalancing entirely; a smarter planner could
            // pick a different (src, dst) pair, but for the reference
            // version we keep it simple.
            return None;
        }

        // Max admissible per-dim mass (in u128 to avoid overflow):
        //   mass · cap_src ≤ load_src · cap_dst − load_dst · cap_src
        let load_src_x_cap_dst = src_spec.load[worst_d] as u128 * cap_dst as u128;
        let load_dst_x_cap_src = dst_spec.load[worst_d] as u128 * cap_src as u128;
        if load_src_x_cap_dst <= load_dst_x_cap_src {
            // Already balanced in this dim.
            return None;
        }
        let max_mass_x_cap_src = load_src_x_cap_dst - load_dst_x_cap_src;
        // Halve the gap; minimum of 1 to make progress.
        let chosen_mass_x_cap_src = (max_mass_x_cap_src / 2).max(1);
        let mass_d = (chosen_mass_x_cap_src / cap_src as u128) as u64;
        // Also cap by source's available load (witness will check, but
        // computing here lets us produce a valid mass).
        let mass_d = mass_d.min(src_spec.load[worst_d]);
        if mass_d == 0 {
            return None;
        }

        let mut mass_arr = [0u64; N];
        mass_arr[worst_d] = mass_d;
        let mass = Mass(mass_arr);

        // Final witness check — should pass given how we chose mass, but
        // the witness is the source of truth.
        let m = HotToCold::witness(src, dst, mass, fleet)?;
        Some(TypedMove::HotToCold(m))
    }
}
