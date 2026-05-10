//! Load-vector primitives.
//!
//! v0.2 (heterogeneous capacity): each machine carries its own capacity. The
//! gauges operate on the resulting per-machine utilization (load / capacity)
//! rather than on raw load. Pigou-Dalton witness conditions become utilization-
//! comparison-based rather than load-comparison-based.
//!
//! The single remaining v0 simplification is single-dimensional load
//! (one resource type). Multi-dim is deferred to Phase 2 of the extension
//! plan.

use std::collections::BTreeMap;

/// Stable identifier for a machine in the fleet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MachineId(pub u64);

/// Mass — the load contribution of a single instance, or the amount being
/// moved between machines. Units are intentionally abstract; in v0 think
/// "milli-vCPU" or similar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mass(pub u64);

/// Specification of a machine (immutable structural facts about it). v0.2
/// has just `capacity`; future versions extend with affinity tags, fault
/// domain, hardware features, etc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineSpec {
    /// Capacity of this machine. Mass is consumed against it; utilization is
    /// `load / capacity`.
    pub capacity: u64,
}

/// State of the fleet at a single instant.
///
/// Owns its machines by value. There is no shared `&Fleet` — `Safe<G>` owns
/// the fleet, and modifications happen by consuming `Safe<G>` and returning a
/// new one.
#[derive(Clone, Debug, Default)]
pub struct Fleet {
    /// Per-machine spec and current load.
    machines: BTreeMap<MachineId, (MachineSpec, u64)>,
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
    /// Construct an empty fleet.
    pub fn new() -> Self {
        Fleet { machines: BTreeMap::new() }
    }

    /// Add a machine with the given capacity and starting load.
    pub fn add_machine(&mut self, id: MachineId, capacity: u64, initial_load: u64) {
        self.machines.insert(id, (MachineSpec { capacity }, initial_load));
    }

    /// Capacity of `id`, or `None` if the machine is unknown.
    pub fn capacity_of(&self, id: MachineId) -> Option<u64> {
        self.machines.get(&id).map(|(spec, _)| spec.capacity)
    }

    /// Current load on `id`, or `None` if the machine is unknown.
    pub fn load(&self, id: MachineId) -> Option<u64> {
        self.machines.get(&id).map(|(_, load)| *load)
    }

    /// Utilization on `id` as a fraction in `[0, 1+]`, or `None` if unknown.
    /// May exceed `1.0` if load > capacity (which is a gauge violation under
    /// `Linfty` with τ ≤ 1).
    pub fn utilization(&self, id: MachineId) -> Option<f64> {
        self.machines.get(&id).map(|(spec, load)| {
            *load as f64 / spec.capacity as f64
        })
    }

    /// Iterate over `(MachineId, capacity, load)` triples in stable order.
    pub fn iter(&self) -> impl Iterator<Item = (MachineId, u64, u64)> + '_ {
        self.machines.iter().map(|(id, (spec, load))| (*id, spec.capacity, *load))
    }

    /// Iterate over `(MachineId, utilization)` pairs in stable order.
    pub fn utilizations(&self) -> impl Iterator<Item = (MachineId, f64)> + '_ {
        self.machines.iter().map(|(id, (spec, load))| {
            (*id, *load as f64 / spec.capacity as f64)
        })
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
        let (_spec, load) = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
        *load = load.saturating_add(mass.0);
        Ok(())
    }

    /// Crate-internal: subtract `mass` from a machine's load. Used by `apply`
    /// impls. Errors if the machine doesn't have enough load — but the
    /// move-algebra invariants should keep this branch unreachable in
    /// well-formed programs.
    pub(crate) fn remove_load(&mut self, id: MachineId, mass: Mass) -> Result<(), FleetError> {
        let (_spec, load) = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
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
