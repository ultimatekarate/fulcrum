//! Power-of-d-choices placement (d = 2 here, the canonical case).
//!
//! For each work item, sample two random machines and place on the less
//! loaded one. With high probability, this gives near-optimal load
//! distribution while requiring O(1) lookups per decision (vs. O(n) for
//! [`super::LeastLoaded`]).
//!
//! References: Mitzenmacher 1996 (the original "Power of Two Choices"
//! analysis); Vöcking 2003 (asymmetric splitting). The exponential gap
//! in maximum-load expectation between random and 2-of-d random
//! placement is the load-bearing result.
//!
//! ## Determinism
//!
//! The randomness is from a deterministically seeded xorshift64 PRNG —
//! no `system_clock` (per `basis.yaml`'s purity rule for the planners
//! layer). Test reproducibility is trivial: same seed, same sequence.

use std::collections::VecDeque;

use crate::gauge::SchurConvex;
use crate::load::Mass;
use crate::move_kind::Place;
use crate::planner::{Planner, TypedMove};
use crate::safe::Safe;

pub struct PowerOfTwo<const N: usize> {
    queue: VecDeque<Mass<N>>,
    rng: Xorshift64,
}

impl<const N: usize> PowerOfTwo<N> {
    /// Construct from a list of work items and a PRNG seed. The seed
    /// must be non-zero (xorshift64 has a fixed point at zero); the
    /// constructor coerces zero to one to keep the API total.
    pub fn new(items: Vec<Mass<N>>, seed: u64) -> Self {
        Self {
            queue: items.into(),
            rng: Xorshift64::new(seed),
        }
    }

    /// Number of items remaining in the queue.
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }
}

impl<const N: usize, G: SchurConvex<N>> Planner<N, G> for PowerOfTwo<N> {
    fn step(&mut self, safe: &Safe<G, N>) -> Option<TypedMove<N>> {
        let mass = self.queue.pop_front()?;
        let n = safe.fleet().len();
        if n == 0 {
            return None;
        }
        // Snapshot the iteration order so two random indices land in the
        // same vector view (Fleet::iter is stable but we want random
        // access).
        let machines: Vec<_> = safe.fleet().iter().collect();
        let i = (self.rng.next() as usize) % n;
        let j = if n == 1 { 0 } else { (self.rng.next() as usize) % n };
        let (id_i, spec_i) = machines[i];
        let (id_j, spec_j) = machines[j];
        let target = if spec_i.worst_utilization() <= spec_j.worst_utilization() {
            id_i
        } else {
            id_j
        };
        Some(TypedMove::Place(Place::new(target, mass)))
    }
}

/// Inline xorshift64 PRNG. Tiny, deterministic, fine for sampling two
/// indices per decision; not cryptographically anything.
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        // xorshift64 has a fixed point at zero — coerce.
        Xorshift64 { state: seed.max(1) }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}
