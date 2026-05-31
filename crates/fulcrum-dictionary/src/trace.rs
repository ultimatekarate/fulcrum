//! Move-history recording for replay and debugging.
//!
//! `MoveHistory<N>` is intentionally separate from `Safe<G, N>`. The
//! typestate carries the *correctness* claim; the trace carries the
//! *operational* record. They serve different purposes and combining them
//! dilutes both.

use crate::load::{MachineId, Mass};

/// One recorded move, parameterized by load dimension `N`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveRecord<const N: usize> {
    Remove { machine: MachineId, mass: Mass<N> },
    HotToCold { source: MachineId, destination: MachineId, mass: Mass<N> },
    Neutral { source: MachineId, destination: MachineId, mass: Mass<N> },
    ColdToHot { source: MachineId, destination: MachineId, mass: Mass<N> },
    Place { machine: MachineId, mass: Mass<N> },
}

impl<const N: usize> MoveRecord<N> {
    /// Whether this kind has a total `apply` under any Schur-convex gauge.
    pub fn is_typed_pure(&self) -> bool {
        matches!(
            self,
            MoveRecord::Remove { .. }
                | MoveRecord::HotToCold { .. }
                | MoveRecord::Neutral { .. }
        )
    }
}

/// Append-only list of moves applied to a fleet.
#[derive(Clone, Debug)]
pub struct MoveHistory<const N: usize> {
    records: Vec<MoveRecord<N>>,
}

impl<const N: usize> Default for MoveHistory<N> {
    fn default() -> Self {
        MoveHistory { records: Vec::new() }
    }
}

impl<const N: usize> MoveHistory<N> {
    pub fn new() -> Self {
        MoveHistory::default()
    }

    pub fn push(&mut self, r: MoveRecord<N>) {
        self.records.push(r);
    }

    pub fn iter(&self) -> impl Iterator<Item = &MoveRecord<N>> {
        self.records.iter()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Count of typed-pure records.
    pub fn typed_pure_count(&self) -> usize {
        self.records.iter().filter(|r| r.is_typed_pure()).count()
    }

    /// Typed-pure ratio over the whole history.
    pub fn typed_pure_ratio(&self) -> f64 {
        if self.records.is_empty() {
            return 0.0;
        }
        self.typed_pure_count() as f64 / self.records.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_history_ratio_is_zero() {
        let h: MoveHistory<1> = MoveHistory::new();
        assert_eq!(h.typed_pure_ratio(), 0.0);
    }

    #[test]
    fn ratio_counts_typed_pure_correctly() {
        let mut h: MoveHistory<1> = MoveHistory::new();
        h.push(MoveRecord::Remove { machine: MachineId(1), mass: Mass([10]) });
        h.push(MoveRecord::HotToCold {
            source: MachineId(1),
            destination: MachineId(2),
            mass: Mass([5]),
        });
        h.push(MoveRecord::Place { machine: MachineId(3), mass: Mass([20]) });
        // 2 of 3 typed-pure
        assert!((h.typed_pure_ratio() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn move_history_works_at_higher_dimensions() {
        let mut h: MoveHistory<2> = MoveHistory::new();
        h.push(MoveRecord::Place {
            machine: MachineId(1),
            mass: Mass([10, 20]),
        });
        assert_eq!(h.len(), 1);
        assert_eq!(h.typed_pure_count(), 0); // place is catch-all
    }
}
