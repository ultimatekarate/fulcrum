//! Load-vector primitives.
//!
//! v0 simplifications:
//! - Single-dimensional load (one resource type).
//! - Uniform capacity across the fleet (every machine has the same capacity).
//!
//! Both are documented limits of v0 and are tracked in `PLAN.md`. The data
//! model is shaped so heterogeneous capacity and multi-dimensional load can be
//! added without breaking the move-algebra surface.

use std::collections::BTreeMap;

/// Stable identifier for a machine in the fleet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MachineId(pub u64);

/// Mass — the load contribution of a single instance, or the amount being
/// moved between machines. Units are intentionally abstract; in v0 think
/// "milli-vCPU" or similar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mass(pub u64);

/// State of the fleet at a single instant.
///
/// Owns its machines by value. There is no shared `&Fleet` — `Safe<G>` owns
/// the fleet, and modifications happen by consuming `Safe<G>` and returning a
/// new one.
#[derive(Clone, Debug)]
pub struct Fleet {
    /// Per-machine load.
    machines: BTreeMap<MachineId, u64>,
    /// Uniform capacity across the fleet (v0 simplification).
    capacity: u64,
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
    /// Construct an empty fleet with the given uniform capacity.
    pub fn new(capacity: u64) -> Self {
        Fleet { machines: BTreeMap::new(), capacity }
    }

    /// Add a machine with the given starting load.
    pub fn add_machine(&mut self, id: MachineId, initial_load: u64) {
        self.machines.insert(id, initial_load);
    }

    /// Uniform capacity (v0).
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Current load on `id`, or `None` if the machine is unknown.
    pub fn load(&self, id: MachineId) -> Option<u64> {
        self.machines.get(&id).copied()
    }

    /// Utilization on `id` as a fraction in [0, 1+], or `None` if unknown.
    /// May exceed 1.0 if load > capacity (which is a gauge violation under
    /// `Linfty` with τ ≤ 1).
    pub fn utilization(&self, id: MachineId) -> Option<f64> {
        self.machines.get(&id).map(|&load| load as f64 / self.capacity as f64)
    }

    /// Iterate over `(MachineId, load)` pairs in stable order.
    pub fn iter(&self) -> impl Iterator<Item = (MachineId, u64)> + '_ {
        self.machines.iter().map(|(id, load)| (*id, *load))
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
        let load = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
        *load = load.saturating_add(mass.0);
        Ok(())
    }

    /// Crate-internal: subtract `mass` from a machine's load. Used by `apply`
    /// impls. Errors if the machine doesn't have enough load — but the
    /// move-algebra invariants should keep this branch unreachable in
    /// well-formed programs.
    pub(crate) fn remove_load(&mut self, id: MachineId, mass: Mass) -> Result<(), FleetError> {
        let load = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
        if *load < mass.0 {
            return Err(FleetError::InsufficientLoad {
                machine: id,
                requested: mass.0,
                available: *load,
            });
        }
        *load -= mass.0;
        Ok(())
    }
}

impl Default for Fleet {
    fn default() -> Self {
        Fleet::new(1)
    }
}
