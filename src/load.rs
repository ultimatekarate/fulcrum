//! Load-vector primitives.
//!
//! Phase 2: multi-dimensional load. The fleet, mass, and per-machine spec
//! are parameterized by a const-generic dimension `N`. Each machine carries
//! `[u64; N]` for both load and capacity; utilization is taken
//! componentwise (`load[d] / capacity[d]`). Per-machine utilization is
//! reduced to a scalar by the gauges (component-wise: max-over-d, then
//! Ky Fan over machines).
//!
//! Phase 1 carries forward as the `N = 1` instantiation. All existing
//! single-dimensional uses become `Fleet<1>`, `Mass<1>`, etc.

use std::collections::BTreeMap;

/// Stable identifier for a machine in the fleet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MachineId(pub u64);

/// Mass — the load contribution of a single instance, or the amount being
/// moved between machines. The wrapped array is per-dimension; in 2D
/// (e.g., CPU + memory) `Mass([cpu, mem])`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mass<const N: usize>(pub [u64; N]);

impl<const N: usize> Mass<N> {
    /// Zero in every dimension.
    pub fn zero() -> Self {
        Mass([0; N])
    }

    /// True if every dimension is zero (the move would be a no-op).
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&x| x == 0)
    }
}

/// Per-machine state: current load and capacity, both N-dimensional.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineSpec<const N: usize> {
    pub load: [u64; N],
    pub capacity: [u64; N],
}

impl<const N: usize> MachineSpec<N> {
    /// Per-dimension utilization. Returns infinity for any dimension with
    /// zero capacity and positive load (so gauges flag the violation
    /// rather than dividing by zero).
    pub fn utilization(&self) -> [f64; N] {
        let mut out = [0.0; N];
        for d in 0..N {
            out[d] = if self.capacity[d] == 0 {
                if self.load[d] == 0 { 0.0 } else { f64::INFINITY }
            } else {
                self.load[d] as f64 / self.capacity[d] as f64
            };
        }
        out
    }

    /// Worst-dimension utilization. The reduction the component-wise
    /// gauges apply per machine before sorting and Ky Fan-ing across the
    /// fleet.
    pub fn worst_utilization(&self) -> f64 {
        self.utilization()
            .iter()
            .copied()
            .fold(0.0_f64, f64::max)
    }
}

/// State of the fleet at a single instant.
///
/// Owns its machines by value. There is no shared `&Fleet` — `Safe<G, N>`
/// owns the fleet, and modifications happen by consuming `Safe` and
/// returning a new one.
#[derive(Clone, Debug)]
pub struct Fleet<const N: usize> {
    machines: BTreeMap<MachineId, MachineSpec<N>>,
}

/// Errors arising from operations that touch the fleet directly. These are
/// internal — they shouldn't surface in normal use because the move algebra
/// is supposed to keep them unreachable.
#[derive(Debug, PartialEq, Eq)]
pub enum FleetError {
    UnknownMachine(MachineId),
    InsufficientLoad {
        machine: MachineId,
        dimension: usize,
        requested: u64,
        available: u64,
    },
}

impl<const N: usize> Fleet<N> {
    /// Construct an empty fleet.
    pub fn new() -> Self {
        Fleet { machines: BTreeMap::new() }
    }

    /// Add a machine with the given per-dimension capacity and starting
    /// load.
    pub fn add_machine(&mut self, id: MachineId, capacity: [u64; N], initial_load: [u64; N]) {
        self.machines.insert(id, MachineSpec { load: initial_load, capacity });
    }

    /// Capacity vector for `id`, or `None` if the machine is unknown.
    pub fn capacity_of(&self, id: MachineId) -> Option<[u64; N]> {
        self.machines.get(&id).map(|s| s.capacity)
    }

    /// Current load on `id`, or `None` if the machine is unknown.
    pub fn load(&self, id: MachineId) -> Option<[u64; N]> {
        self.machines.get(&id).map(|s| s.load)
    }

    /// Per-dimension utilization on `id`, or `None` if unknown.
    pub fn utilization(&self, id: MachineId) -> Option<[f64; N]> {
        self.machines.get(&id).map(|s| s.utilization())
    }

    /// Worst-dimension utilization on `id`, or `None` if unknown. This
    /// is the per-machine scalar reduction used by component-wise gauges.
    pub fn worst_utilization(&self, id: MachineId) -> Option<f64> {
        self.machines.get(&id).map(|s| s.worst_utilization())
    }

    /// Borrow the per-machine spec, or `None` if unknown. Witness
    /// constructors use this to compare per-dimension load and capacity
    /// in one lookup.
    pub fn spec(&self, id: MachineId) -> Option<&MachineSpec<N>> {
        self.machines.get(&id)
    }

    /// Iterate over `(MachineId, &MachineSpec<N>)` pairs in stable order.
    pub fn iter(&self) -> impl Iterator<Item = (MachineId, &MachineSpec<N>)> + '_ {
        self.machines.iter().map(|(id, spec)| (*id, spec))
    }

    /// Number of machines.
    pub fn len(&self) -> usize {
        self.machines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.machines.is_empty()
    }

    /// Crate-internal: add `mass` componentwise to a machine's load.
    pub(crate) fn add_load(&mut self, id: MachineId, mass: Mass<N>) -> Result<(), FleetError> {
        let spec = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
        for d in 0..N {
            spec.load[d] = spec.load[d].saturating_add(mass.0[d]);
        }
        Ok(())
    }

    /// Crate-internal: subtract `mass` componentwise from a machine's load.
    /// Errors if any dimension would underflow.
    pub(crate) fn remove_load(&mut self, id: MachineId, mass: Mass<N>) -> Result<(), FleetError> {
        let spec = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
        for d in 0..N {
            if spec.load[d] < mass.0[d] {
                return Err(FleetError::InsufficientLoad {
                    machine: id,
                    dimension: d,
                    requested: mass.0[d],
                    available: spec.load[d],
                });
            }
        }
        for d in 0..N {
            spec.load[d] -= mass.0[d];
        }
        Ok(())
    }
}

impl<const N: usize> Default for Fleet<N> {
    fn default() -> Self {
        Fleet::new()
    }
}
