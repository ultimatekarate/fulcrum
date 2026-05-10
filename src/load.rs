//! Load-vector primitives.
//!
//! Phase 1: heterogeneous capacity. Each machine carries its own capacity;
//! `Fleet` no longer enforces a uniform value. Single-dimensional load
//! (one resource type) remains a v0 simplification, lifted in Phase 2.

use std::collections::BTreeMap;

/// Stable identifier for a machine in the fleet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MachineId(pub u64);

/// Mass — the load contribution of a single instance, or the amount being
/// moved between machines. Units are intentionally abstract; in v0 think
/// "milli-vCPU" or similar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mass(pub u64);

/// Per-machine state: current load and total capacity. Both are abstract
/// scalars in the same unit (e.g., milli-vCPU). Utilization is the ratio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineSpec {
    pub load: u64,
    pub capacity: u64,
}

impl MachineSpec {
    /// Utilization as a fraction in [0, 1+]. Exceeds 1.0 if load > capacity
    /// (which is a gauge violation under any unit-thresholded gauge).
    pub fn utilization(&self) -> f64 {
        if self.capacity == 0 {
            // A zero-capacity machine cannot host load. Treat any positive
            // load as infinitely utilized so gauge evaluation flags it
            // immediately rather than dividing by zero.
            if self.load == 0 { 0.0 } else { f64::INFINITY }
        } else {
            self.load as f64 / self.capacity as f64
        }
    }
}

/// State of the fleet at a single instant.
///
/// Owns its machines by value. There is no shared `&Fleet` — `Safe<G>` owns
/// the fleet, and modifications happen by consuming `Safe<G>` and returning a
/// new one.
#[derive(Clone, Debug)]
pub struct Fleet {
    machines: BTreeMap<MachineId, MachineSpec>,
}

/// Errors arising from operations that touch the fleet directly. These are
/// internal — they shouldn't surface in normal use because the move algebra
/// is supposed to keep them unreachable.
#[derive(Debug, PartialEq, Eq)]
pub enum FleetError {
    UnknownMachine(MachineId),
    InsufficientLoad { machine: MachineId, requested: u64, available: u64 },
}

impl Fleet {
    /// Construct an empty fleet. Capacities are now per-machine; pass them
    /// to [`Fleet::add_machine`] for each machine you register.
    pub fn new() -> Self {
        Fleet { machines: BTreeMap::new() }
    }

    /// Add a machine with the given capacity and starting load.
    pub fn add_machine(&mut self, id: MachineId, capacity: u64, initial_load: u64) {
        self.machines.insert(id, MachineSpec { load: initial_load, capacity });
    }

    /// Capacity of `id`, or `None` if the machine is unknown.
    pub fn capacity_of(&self, id: MachineId) -> Option<u64> {
        self.machines.get(&id).map(|s| s.capacity)
    }

    /// Current load on `id`, or `None` if the machine is unknown.
    pub fn load(&self, id: MachineId) -> Option<u64> {
        self.machines.get(&id).map(|s| s.load)
    }

    /// Utilization on `id` as a fraction in [0, 1+], or `None` if unknown.
    /// May exceed 1.0 if load > capacity (which is a gauge violation under
    /// `Linfty` with τ ≤ 1).
    pub fn utilization(&self, id: MachineId) -> Option<f64> {
        self.machines.get(&id).map(|s| s.utilization())
    }

    /// Borrow the per-machine spec, or `None` if unknown. Witness
    /// constructors use this to compare the source and destination
    /// utilizations and capacities in one lookup.
    pub fn spec(&self, id: MachineId) -> Option<&MachineSpec> {
        self.machines.get(&id)
    }

    /// Iterate over `(MachineId, &MachineSpec)` pairs in stable order.
    pub fn iter(&self) -> impl Iterator<Item = (MachineId, &MachineSpec)> + '_ {
        self.machines.iter().map(|(id, spec)| (*id, spec))
    }

    /// Number of machines.
    pub fn len(&self) -> usize {
        self.machines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.machines.is_empty()
    }

    /// Crate-internal: add `mass` to a machine's load. Used by `apply` impls.
    pub(crate) fn add_load(&mut self, id: MachineId, mass: Mass) -> Result<(), FleetError> {
        let spec = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
        spec.load = spec.load.saturating_add(mass.0);
        Ok(())
    }

    /// Crate-internal: subtract `mass` from a machine's load. Used by `apply`
    /// impls. Errors if the machine doesn't have enough load — but the
    /// move-algebra invariants should keep this branch unreachable in
    /// well-formed programs.
    pub(crate) fn remove_load(&mut self, id: MachineId, mass: Mass) -> Result<(), FleetError> {
        let spec = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
        if spec.load < mass.0 {
            return Err(FleetError::InsufficientLoad {
                machine: id,
                requested: mass.0,
                available: spec.load,
            });
        }
        spec.load -= mass.0;
        Ok(())
    }
}

impl Default for Fleet {
    fn default() -> Self {
        Fleet::new()
    }
}
