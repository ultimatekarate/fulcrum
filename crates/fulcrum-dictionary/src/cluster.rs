//! Cluster topology for the Turing Pi 2 digital twin.
//!
//! Two things live here: the typed selector for the four balancing
//! dimensions (`ResourceDim`), and the static per-node profile + topology
//! builder that stands up a heterogeneous `Fleet<4>`.
//!
//! Dictionary-layer, pure. `N = 4` is the *dimension count per node*
//! (Cpu/Mem/DiskIo/NetIo), not the node count — a Turing Pi 2's four board
//! slots are an unrelated coincidence. A 6-node cluster of 4-dimension
//! nodes would still be `Fleet<4>`.

use crate::load::{Capacity, Fleet, MachineId, Mass};
use crate::power::{Power, PowerCoeffs};

/// The four balancing dimensions each node carries, mapped to the load /
/// capacity array slots. A typed selector replaces bare `const CPU: usize`
/// indices at call sites (per `basis.yaml`'s newtype governance).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceDim {
    Cpu,
    Mem,
    DiskIo,
    NetIo,
}

impl ResourceDim {
    /// Number of resource dimensions — the const-generic `N` for the twin.
    pub const COUNT: usize = 4;

    /// The load-vector slot this dimension occupies.
    pub const fn index(self) -> usize {
        match self {
            ResourceDim::Cpu => 0,
            ResourceDim::Mem => 1,
            ResourceDim::DiskIo => 2,
            ResourceDim::NetIo => 3,
        }
    }
}

/// A node's static profile: per-dimension capacity and power coefficients.
///
/// `capacity` is branded `Capacity<N>` — it cannot be swapped with a load
/// (`Mass`) vector at any API boundary, even though both are `[u64; N]`
/// underneath.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeProfile {
    pub capacity: Capacity<{ ResourceDim::COUNT }>,
    pub power: PowerCoeffs,
}

/// A fleet's static layout: the node slots in ascending `MachineId` order.
///
/// `coeffs()` is parallel to `fleet().iter()` order (both ascending
/// `MachineId`); `power_eval::fleet_power` relies on that alignment.
pub struct Topology {
    nodes: Vec<(MachineId, NodeProfile)>,
}

impl Topology {
    /// Build a zero-load `Fleet<4>` from the node profiles.
    pub fn fleet(&self) -> Fleet<4> {
        let mut f = Fleet::new();
        for (id, profile) in &self.nodes {
            f.add_machine(*id, profile.capacity, Mass([0; ResourceDim::COUNT]));
        }
        f
    }

    /// Per-node power coefficients, ordered parallel to `fleet().iter()`.
    pub fn coeffs(&self) -> Vec<PowerCoeffs> {
        self.nodes.iter().map(|(_, p)| p.power).collect()
    }

    /// The node slots, ascending `MachineId`.
    pub fn nodes(&self) -> &[(MachineId, NodeProfile)] {
        &self.nodes
    }

    /// Number of node slots.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Stand-in profile for a Raspberry Pi 5 (8 GB): higher CPU throughput,
/// NVMe storage IO, gigabit network. Power per datasheet stand-ins
/// (≈ 2.7 W idle, ≈ 10 W all-core).
fn raspberry_pi_5_8gb() -> NodeProfile {
    NodeProfile {
        // [Cpu, Mem(MiB), DiskIo, NetIo]
        capacity: Capacity([1000, 8192, 1000, 1000]),
        power: PowerCoeffs {
            idle: Power(2700.0),
            dynamic: Power(7300.0), // idle + dynamic = 10_000 mW at full load
        },
    }
}

/// Stand-in profile for a Raspberry Pi 4 (4 GB): lower CPU, SD-class
/// storage IO, gigabit network. Power per datasheet stand-ins
/// (≈ 2.7 W idle, ≈ 6.5 W all-core).
fn raspberry_pi_4_4gb() -> NodeProfile {
    NodeProfile {
        capacity: Capacity([600, 4096, 200, 1000]),
        power: PowerCoeffs {
            idle: Power(2700.0),
            dynamic: Power(3800.0), // idle + dynamic = 6_500 mW at full load
        },
    }
}

/// The Turing Pi 2 reference topology: four heterogeneous board slots —
/// two Raspberry Pi 5 (8 GB) and two Raspberry Pi 4 (4 GB) — all drawing
/// from one shared power input.
pub fn turing_pi_2() -> Topology {
    Topology {
        nodes: vec![
            (MachineId(1), raspberry_pi_5_8gb()),
            (MachineId(2), raspberry_pi_5_8gb()),
            (MachineId(3), raspberry_pi_4_4gb()),
            (MachineId(4), raspberry_pi_4_4gb()),
        ],
    }
}
