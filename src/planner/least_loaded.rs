//! Least-loaded placement (greedy minimizer of `ℓ_∞`).
//!
//! At each step, dequeues one item from the work queue and emits a
//! `Place` targeting the machine with the lowest worst-dim utilization.
//! Returns `None` when the queue is empty.
//!
//! References: Winston 1977 (the classic Join-Shortest-Queue analysis;
//! `LeastLoaded` is JSQ for state-aware single-server-per-machine
//! placement). The framework's typestate doesn't add much for individual
//! decisions — each Place is catch-all — but the cumulative invariant is
//! tracked: any sequence of placements that exceeds the gauge bound is
//! rejected at the apply site.

use std::collections::VecDeque;

use fulcrum_laboratory::gauge::SchurConvex;
use fulcrum_dictionary::load::Mass;
use fulcrum_laboratory::move_kind::Place;
use crate::planner::{Planner, TypedMove};
use fulcrum_laboratory::safe::Safe;

pub struct LeastLoaded<const N: usize> {
    queue: VecDeque<Mass<N>>,
}

impl<const N: usize> LeastLoaded<N> {
    /// Construct from a list of work items (in the order they should be
    /// considered).
    pub fn new(items: Vec<Mass<N>>) -> Self {
        Self { queue: items.into() }
    }

    /// Number of items remaining in the queue.
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }
}

impl<const N: usize, G: SchurConvex<N>> Planner<N, G> for LeastLoaded<N> {
    fn step(&mut self, safe: &Safe<G, N>) -> Option<TypedMove<N>> {
        let mass = self.queue.pop_front()?;
        // Find the machine with the minimum worst-dim utilization. Stable
        // tie-break by MachineId (lower wins), inherited from the
        // BTreeMap iteration order.
        let target = safe.fleet().iter().min_by(|(_, a), (_, b)| {
            a.worst_utilization()
                .partial_cmp(&b.worst_utilization())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        Some(TypedMove::Place(Place::new(target.0, mass)))
    }
}
