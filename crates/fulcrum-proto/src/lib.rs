//! # Fulcrum governor protocol
//!
//! The shared nouns and the single contract that cross the boundary between
//! the **controller** (the authority — one process holding the desired
//! `Safe<G, N>`) and the **sled-agents** (one per physical node, each the
//! sole writer of its own node's reality).
//!
//! ## The shape of authority
//!
//! The controller never issues imperative "start X / stop Y" commands. It
//! publishes, per node, the *complete desired set* of workloads as a
//! [`NodeIntent`]; the agent reconciles its reality to that set,
//! idempotently. This is declarative, kubelet/sled-agent style: a dropped or
//! reordered message is harmless because the next intent is the whole truth,
//! not a delta. Each agent is the single writer of its node — which is where
//! the "single writer per machine" of the concurrency design actually lives.
//!
//! ## The honest boundary
//!
//! The controller's `Safe<G, N>` guarantees it never *intends* an unsafe
//! state. It cannot make reality safe — a node dies, a container OOMs. So
//! reports ([`NodeReport`]) carry *observed* state, which can be anything;
//! the controller's job is to drive observed → a safe desired.
//!
//! These nouns are reused from the move algebra deliberately: a workload's
//! footprint is a [`Mass`], a node's capacity is a [`Capacity`]. The governor
//! speaks the same language as the gauge it protects.
//!
//! Wire serialization (serde/postcard) is intentionally **not** here yet: the
//! first walking skeleton drives the [`SledAgent`] contract in-process. The
//! transport encoding is a separate decision, made when we put it on a wire.

use fulcrum_dictionary::{Capacity, MachineId, Mass};

/// Stable identity of a managed workload (a container instance).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkloadId(pub u64);

/// What to run and what it costs. The `request` is the workload's resource
/// footprint — the [`Mass`] it adds to whatever node it lands on, so placing
/// it is exactly a `Place` move of `request`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkloadSpec<const N: usize> {
    pub id: WorkloadId,
    /// Container image reference, e.g. `docker.io/library/nginx:alpine`.
    pub image: String,
    /// Resource footprint — the load this workload places on its node.
    pub request: Mass<N>,
}

/// The controller's *complete* desired workload set for one node, as of
/// `epoch`. Not a delta — the whole truth. The agent reconciles to it and
/// ignores any intent whose `epoch` is older than the last it applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeIntent<const N: usize> {
    pub node: MachineId,
    /// Monotonic generation counter. Guards against out-of-order delivery.
    pub epoch: u64,
    pub desired: Vec<WorkloadSpec<N>>,
}

/// Where a workload is in its lifecycle, as the agent observes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadPhase {
    /// Accepted into the desired set; not yet running.
    Pending,
    /// Running and healthy.
    Running,
    /// Exited non-zero or could not be started.
    Failed,
}

/// One workload's observed status on a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkloadStatus {
    pub id: WorkloadId,
    pub phase: WorkloadPhase,
}

/// What one node actually reports: its discovered capacity, current observed
/// load, and per-workload status, tagged with the `epoch` of the intent this
/// report reflects. The controller folds these into its observed model of the
/// fleet; divergence from desired is what the reconcile loop closes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeReport<const N: usize> {
    pub node: MachineId,
    /// The intent epoch this report reflects (lets the controller tell a
    /// reconciled node from one still converging to a fresh intent).
    pub epoch: u64,
    pub capacity: Capacity<N>,
    /// Actual current load — *observed*, not desired. May exceed capacity.
    pub observed_load: Mass<N>,
    pub workloads: Vec<WorkloadStatus>,
}

/// Errors crossing the controller↔agent boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentError {
    /// This agent does not manage the node id in the request.
    UnknownNode(MachineId),
    /// The agent could not be reached (process or network failure).
    Unreachable,
    /// The agent has already applied a newer intent; this one is stale.
    StaleEpoch { intent: u64, current: u64 },
    /// Reconciliation failed at the runtime for a concrete reason.
    ReconcileFailed(String),
}

/// The controller↔agent contract.
///
/// The controller holds it as a trait object / generic so it never names a
/// concrete agent: an in-process mock for the twin, a real network adapter
/// driving Podman on a Pi. `apply_intent` publishes the desired set;
/// `report` fetches observed reality. Single-writer-per-node means the agent
/// is the only thing that mutates its node's runtime.
///
/// Sync for the in-process skeleton; the networked transport will wrap this
/// (the shape — publish desired, fetch observed — is transport-agnostic).
pub trait SledAgent<const N: usize> {
    /// Publish the complete desired set for this node. Idempotent: applying
    /// the same intent twice is a no-op. Rejects intents older than the last
    /// applied (`StaleEpoch`).
    fn apply_intent(&mut self, intent: &NodeIntent<N>) -> Result<(), AgentError>;

    /// Fetch the node's current observed state.
    fn report(&self) -> Result<NodeReport<N>, AgentError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial in-memory agent: it "runs" exactly the desired set and
    /// reports observed load as the sum of requests. Proves the contract and
    /// nouns compose into a usable round-trip without any runtime.
    struct MockAgent<const N: usize> {
        node: MachineId,
        capacity: Capacity<N>,
        epoch: u64,
        running: Vec<WorkloadSpec<N>>,
    }

    impl<const N: usize> SledAgent<N> for MockAgent<N> {
        fn apply_intent(&mut self, intent: &NodeIntent<N>) -> Result<(), AgentError> {
            if intent.node != self.node {
                return Err(AgentError::UnknownNode(intent.node));
            }
            if intent.epoch < self.epoch {
                return Err(AgentError::StaleEpoch { intent: intent.epoch, current: self.epoch });
            }
            self.epoch = intent.epoch;
            self.running = intent.desired.clone();
            Ok(())
        }

        fn report(&self) -> Result<NodeReport<N>, AgentError> {
            let mut observed = [0u64; N];
            for w in &self.running {
                for d in 0..N {
                    observed[d] = observed[d].saturating_add(w.request[d]);
                }
            }
            let workloads = self
                .running
                .iter()
                .map(|w| WorkloadStatus { id: w.id, phase: WorkloadPhase::Running })
                .collect();
            Ok(NodeReport {
                node: self.node,
                epoch: self.epoch,
                capacity: self.capacity,
                observed_load: Mass(observed),
                workloads,
            })
        }
    }

    #[test]
    fn intent_round_trips_through_the_contract() {
        let mut agent = MockAgent::<2> {
            node: MachineId(1),
            capacity: Capacity([100, 100]),
            epoch: 0,
            running: Vec::new(),
        };

        let intent = NodeIntent {
            node: MachineId(1),
            epoch: 1,
            desired: vec![
                WorkloadSpec { id: WorkloadId(10), image: "nginx".into(), request: Mass([30, 20]) },
                WorkloadSpec { id: WorkloadId(11), image: "redis".into(), request: Mass([10, 40]) },
            ],
        };
        agent.apply_intent(&intent).unwrap();

        let report = agent.report().unwrap();
        assert_eq!(report.epoch, 1);
        assert_eq!(report.observed_load, Mass([40, 60]));
        assert_eq!(report.workloads.len(), 2);
        assert!(report.workloads.iter().all(|w| w.phase == WorkloadPhase::Running));
    }

    #[test]
    fn stale_epoch_is_rejected() {
        let mut agent = MockAgent::<1> {
            node: MachineId(1),
            capacity: Capacity([100]),
            epoch: 5,
            running: Vec::new(),
        };
        let stale = NodeIntent { node: MachineId(1), epoch: 4, desired: Vec::new() };
        assert_eq!(
            agent.apply_intent(&stale),
            Err(AgentError::StaleEpoch { intent: 4, current: 5 }),
        );
    }

    #[test]
    fn wrong_node_is_rejected() {
        let mut agent = MockAgent::<1> {
            node: MachineId(1),
            capacity: Capacity([100]),
            epoch: 0,
            running: Vec::new(),
        };
        let misrouted = NodeIntent { node: MachineId(2), epoch: 1, desired: Vec::new() };
        assert_eq!(agent.apply_intent(&misrouted), Err(AgentError::UnknownNode(MachineId(2))));
    }
}
