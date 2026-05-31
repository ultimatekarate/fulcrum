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

/// Per-dimension capacity of a machine — the denominator of utilization.
/// Branded so it cannot be swapped with `Mass` (load) at a call boundary:
/// the two are the same `[u64; N]` underneath but mean opposite things, and
/// the whole point of the newtype rule is that a swap is a *type* error, not
/// a silent bug.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity<const N: usize>(pub [u64; N]);

/// Per-dimension utilization (load ÷ capacity) — the carrier the gauges are
/// Schur-convex over. Distinct from `Mass`/`Capacity` because it is the
/// *quotient*, not a conserved quantity: a load transfer conserves `Mass`
/// but does not in general conserve `Utilization` across heterogeneous
/// capacities. Naming it makes the load↔utilization change-of-basis explicit
/// rather than an anonymous division scattered through the code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Utilization<const N: usize>(pub [f64; N]);

impl<const N: usize> Capacity<N> {
    /// The change-of-basis from the conserved carrier (load) to the gauged
    /// carrier (utilization). This is the *sole* site in the crate where
    /// load becomes utilization. Returns infinity for any dimension with
    /// zero capacity and positive load (so gauges flag the violation rather
    /// than dividing by zero).
    pub fn utilization_of(&self, load: &Mass<N>) -> Utilization<N> {
        let mut out = [0.0; N];
        for d in 0..N {
            out[d] = if self.0[d] == 0 {
                if load.0[d] == 0 { 0.0 } else { f64::INFINITY }
            } else {
                load.0[d] as f64 / self.0[d] as f64
            };
        }
        Utilization(out)
    }
}

impl<const N: usize> Utilization<N> {
    /// Worst-dimension utilization. The per-machine scalar reduction the
    /// component-wise gauges apply before sorting and Ky Fan-ing across the
    /// fleet.
    pub fn worst(&self) -> f64 {
        self.0.iter().copied().fold(0.0_f64, f64::max)
    }
}

impl<const N: usize> std::ops::Index<usize> for Mass<N> {
    type Output = u64;
    fn index(&self, d: usize) -> &u64 {
        &self.0[d]
    }
}

impl<const N: usize> std::ops::Index<usize> for Capacity<N> {
    type Output = u64;
    fn index(&self, d: usize) -> &u64 {
        &self.0[d]
    }
}

impl<const N: usize> std::ops::Index<usize> for Utilization<N> {
    type Output = f64;
    fn index(&self, d: usize) -> &f64 {
        &self.0[d]
    }
}

/// Per-machine state: current load and capacity, both N-dimensional and
/// branded — `load` is a `Mass` (conserved by transfers), `capacity` is a
/// `Capacity` (the utilization denominator). They are no longer
/// interchangeable `[u64; N]` arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MachineSpec<const N: usize> {
    pub load: Mass<N>,
    pub capacity: Capacity<N>,
}

impl<const N: usize> MachineSpec<N> {
    /// Per-dimension utilization, via the one change-of-basis
    /// ([`Capacity::utilization_of`]).
    pub fn utilization(&self) -> Utilization<N> {
        self.capacity.utilization_of(&self.load)
    }

    /// Worst-dimension utilization. The reduction the component-wise
    /// gauges apply per machine before sorting and Ky Fan-ing across the
    /// fleet.
    pub fn worst_utilization(&self) -> f64 {
        self.utilization().worst()
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
    /// load. The two vectors are branded (`Capacity` vs `Mass`) so they
    /// cannot be passed in the wrong order.
    pub fn add_machine(&mut self, id: MachineId, capacity: Capacity<N>, initial_load: Mass<N>) {
        self.machines.insert(id, MachineSpec { load: initial_load, capacity });
    }

    /// Capacity vector for `id`, or `None` if the machine is unknown.
    pub fn capacity_of(&self, id: MachineId) -> Option<Capacity<N>> {
        self.machines.get(&id).map(|s| s.capacity)
    }

    /// Current load on `id`, or `None` if the machine is unknown.
    pub fn load(&self, id: MachineId) -> Option<Mass<N>> {
        self.machines.get(&id).map(|s| s.load)
    }

    /// Per-dimension utilization on `id`, or `None` if unknown.
    pub fn utilization(&self, id: MachineId) -> Option<Utilization<N>> {
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

    /// Workspace-internal: add `mass` componentwise to a machine's load.
    ///
    /// `#[doc(hidden)] pub` rather than `pub(crate)` only because the
    /// laboratory's `apply` impls live in a sibling crate and must call it.
    /// This is *not* a public mutation path: the real guarantee is that
    /// `Safe` never hands out `&mut Fleet`, so no consumer can reach this on
    /// a fleet that is under a gauge invariant.
    #[doc(hidden)]
    pub fn add_load(&mut self, id: MachineId, mass: Mass<N>) -> Result<(), FleetError> {
        let spec = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
        for d in 0..N {
            spec.load.0[d] = spec.load.0[d].saturating_add(mass.0[d]);
        }
        Ok(())
    }

    /// Workspace-internal: subtract `mass` componentwise from a machine's
    /// load. Errors if any dimension would underflow. `#[doc(hidden)] pub`
    /// for the same reason as [`Fleet::add_load`] — the laboratory's `apply`
    /// impls are a sibling crate.
    #[doc(hidden)]
    pub fn remove_load(&mut self, id: MachineId, mass: Mass<N>) -> Result<(), FleetError> {
        let spec = self.machines.get_mut(&id).ok_or(FleetError::UnknownMachine(id))?;
        for d in 0..N {
            if spec.load.0[d] < mass.0[d] {
                return Err(FleetError::InsufficientLoad {
                    machine: id,
                    dimension: d,
                    requested: mass.0[d],
                    available: spec.load.0[d],
                });
            }
        }
        for d in 0..N {
            spec.load.0[d] -= mass.0[d];
        }
        Ok(())
    }
}

impl<const N: usize> Default for Fleet<N> {
    fn default() -> Self {
        Fleet::new()
    }
}
