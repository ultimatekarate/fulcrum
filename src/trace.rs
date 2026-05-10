//! Move-history recording for replay and debugging.
//!
//! `MoveHistory` is intentionally separate from `Safe<G>`. The typestate
//! carries the *correctness* claim; the trace carries the *operational*
//! record. They serve different purposes and combining them dilutes both.

use crate::load::{MachineId, Mass};

/// One recorded move. The variant tells you what kind it was; the fields
/// give the operational data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveRecord {
    Remove { machine: MachineId, mass: Mass },
    HotToCold { source: MachineId, destination: MachineId, mass: Mass },
    Neutral { source: MachineId, destination: MachineId, mass: Mass },
    ColdToHot { source: MachineId, destination: MachineId, mass: Mass },
    Place { machine: MachineId, mass: Mass },
}

impl MoveRecord {
    /// Whether this kind has a total `apply` under any Schur-convex gauge.
    pub fn is_typed_pure(&self) -> bool {
        matches!(self, MoveRecord::Remove { .. } | MoveRecord::HotToCold { .. } | MoveRecord::Neutral { .. })
    }
}

/// Append-only list of moves applied to a fleet.
#[derive(Clone, Debug, Default)]
pub struct MoveHistory {
    records: Vec<MoveRecord>,
}

impl MoveHistory {
    pub fn new() -> Self {
        MoveHistory { records: Vec::new() }
    }

    pub fn push(&mut self, r: MoveRecord) {
        self.records.push(r);
    }

    pub fn iter(&self) -> impl Iterator<Item = &MoveRecord> {
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

    /// Typed-pure ratio over the whole history. The metric the framework
    /// claims is load-bearing for its localization properties.
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
        let h = MoveHistory::new();
        assert_eq!(h.typed_pure_ratio(), 0.0);
    }

    #[test]
    fn ratio_counts_typed_pure_correctly() {
        let mut h = MoveHistory::new();
        h.push(MoveRecord::Remove { machine: MachineId(1), mass: Mass(10) });
        h.push(MoveRecord::HotToCold {
            source: MachineId(1),
            destination: MachineId(2),
            mass: Mass(5),
        });
        h.push(MoveRecord::Place { machine: MachineId(3), mass: Mass(20) });
        // 2 of 3 typed-pure
        assert!((h.typed_pure_ratio() - 2.0 / 3.0).abs() < 1e-9);
    }
}
