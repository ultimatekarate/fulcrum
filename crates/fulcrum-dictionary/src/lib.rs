//! # Fulcrum dictionary
//!
//! The inert data layer — the nouns of the move algebra. Pure data types
//! with no logic and no dependencies on any other Fulcrum crate (only
//! `std`). This crate is the smallest possible foundation: load/capacity/
//! utilization vectors, per-machine specs and the fleet, power data,
//! cluster topology, and move-record traces.
//!
//! What is *not* here: the gauge trait and its evaluation, the move-kind
//! witnesses, the `Safe` typestate, and the sealed `Primitive` alphabet —
//! those are *logic* and live in `fulcrum-laboratory`. The crate boundary
//! makes the separation a `Cargo.toml` fact: the dictionary literally
//! cannot import laboratory logic, because it does not depend on it.

pub mod cluster;
pub mod load;
pub mod power;
pub mod trace;

pub use cluster::{turing_pi_2, NodeProfile, ResourceDim, Topology};
pub use load::{Capacity, Fleet, FleetError, MachineId, MachineSpec, Mass, Utilization};
pub use power::{Power, PowerBudget, PowerCoeffs};
pub use trace::{MoveHistory, MoveRecord};
