//! Best-Fit-Decreasing bin-packing.
//!
//! Items are sorted descending by total mass at construction. Each step
//! emits a `Place` targeting the *feasible* machine with the smallest
//! remaining headroom in any dimension — i.e., the bin that fits but
//! is most-nearly-full.
//!
//! References: Johnson 1973 (the original BFD analysis with the
//! `(11/9) · OPT + 6/9` performance bound). The framework's typestate
//! catches infeasibility at the `Place::apply_with_recheck` site —
//! every BFD decision is catch-all (a placement is always re-checked
//! against the gauge), so the typestate adds the same per-decision
//! safety guarantee that holds across all placement-style planners.

use std::collections::VecDeque;

use crate::gauge::SchurConvex;
use crate::load::Mass;
use crate::move_kind::Place;
use crate::planner::{Planner, TypedMove};
use crate::safe::Safe;

pub struct BestFitDecreasing<const N: usize> {
    queue: VecDeque<Mass<N>>,
}

impl<const N: usize> BestFitDecreasing<N> {
    /// Construct from a list of items. Items are sorted descending by
    /// total mass (sum across dimensions) at construction.
    pub fn new(mut items: Vec<Mass<N>>) -> Self {
        items.sort_by(|a, b| {
            let sa: u64 = a.0.iter().sum();
            let sb: u64 = b.0.iter().sum();
            sb.cmp(&sa)
        });
        Self { queue: items.into() }
    }

    /// Number of items remaining in the queue.
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }
}

impl<const N: usize, G: SchurConvex<N>> Planner<N, G> for BestFitDecreasing<N> {
    fn step(&mut self, safe: &Safe<G, N>) -> Option<TypedMove<N>> {
        let mass = self.queue.pop_front()?;
        let fleet = safe.fleet();

        // Among machines that can fit `mass` in every dimension without
        // overflow, pick the one with the smallest remaining headroom in
        // any dim (the tightest fit).
        let target = fleet
            .iter()
            .filter(|(_, spec)| {
                spec.load
                    .0
                    .iter()
                    .zip(spec.capacity.0.iter())
                    .zip(mass.0.iter())
                    .all(|((&l, &c), &m)| l.saturating_add(m) <= c)
            })
            .min_by_key(|(_, spec)| {
                let mut min_room = u64::MAX;
                for d in 0..N {
                    let room = spec.capacity[d].saturating_sub(spec.load[d]);
                    if room < min_room {
                        min_room = room;
                    }
                }
                min_room
            })
            .map(|(id, _)| id)?;

        Some(TypedMove::Place(Place::new(target, mass)))
    }
}
