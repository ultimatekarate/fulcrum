//! # Fulcrum controller
//!
//! The authority. One process holds the **desired** state as a `Safe<G, N>`
//! — provably within the gauge threshold — and drives a set of sled-agents
//! toward it via declarative [`NodeIntent`]s.
//!
//! This crate is the *logic* of the authority, deliberately built sim-first:
//! it talks to agents only through the [`SledAgent`] contract, so it runs
//! identically against an in-process fault-injecting mock (the tests below)
//! and, later, against real Podman-driving agents over a network. No tokio,
//! no IO — the placement and reconcile loop is pure and fully testable.
//!
//! ## What it guarantees, and what it doesn't
//!
//! Every placement goes through the `Safe` typestate, so the controller
//! **cannot decide an over-threshold state** — an unsafe placement is a
//! rejected `submit`, not a corrupted fleet. It cannot make *reality* safe:
//! agents report observed state that may diverge (a workload crashed). The
//! [`Controller::reconcile`] loop closes that gap by re-asserting desired
//! intent — driving observed → the safe desired.

use std::collections::{BTreeMap, BTreeSet};

use fulcrum_dictionary::{MachineId, Mass};
use fulcrum_laboratory::gauge::SchurConvex;
use fulcrum_laboratory::safe::{GaugeError, Safe};
use fulcrum_proto::{
    AgentError, NodeIntent, SledAgent, WorkloadId, WorkloadPhase, WorkloadSpec,
};

/// Why a `submit` could not be satisfied.
#[derive(Debug, PartialEq)]
pub enum SubmitError {
    /// No node can host the workload without breaching the gauge threshold.
    NoFeasibleNode,
    /// The chosen node rejected the placement at the safety re-check. Should
    /// not occur after feasibility selection; surfaced rather than panicked.
    Gauge(GaugeError),
    /// The chosen node has no registered agent.
    UnknownNode(MachineId),
    /// The agent failed to apply the intent.
    Agent(AgentError),
}

/// Outcome of one [`Controller::reconcile`] pass, per node.
#[derive(Debug, Default, PartialEq)]
pub struct ReconcileReport {
    /// Nodes whose observed running set already matched desired.
    pub converged: Vec<MachineId>,
    /// Nodes whose observed set diverged from desired (a crash, a miss).
    pub diverged: Vec<MachineId>,
    /// Nodes whose intent was re-asserted to correct divergence.
    pub republished: Vec<MachineId>,
    /// Nodes whose agent could not be reached this pass.
    pub unreachable: Vec<MachineId>,
}

/// The authority over a fleet.
///
/// `G` is the Schur-convex gauge it protects; `N` the load dimensionality.
/// Agents are held behind the [`SledAgent`] trait object so the controller
/// never names a concrete transport.
pub struct Controller<G: SchurConvex<N>, const N: usize> {
    /// Authoritative desired placement — always within threshold.
    desired: Safe<G, N>,
    /// One agent per node, keyed by the fleet's `MachineId`.
    agents: BTreeMap<MachineId, Box<dyn SledAgent<N>>>,
    /// Which node each workload is placed on (in desired).
    assignments: BTreeMap<WorkloadId, MachineId>,
    /// Workload specs, for rebuilding a node's intent.
    specs: BTreeMap<WorkloadId, WorkloadSpec<N>>,
    /// Monotonic intent epoch per node.
    epochs: BTreeMap<MachineId, u64>,
}

impl<G: SchurConvex<N>, const N: usize> Controller<G, N> {
    /// Build a controller over an initial (typically zero-load) desired
    /// fleet and the agents that manage its nodes.
    pub fn new(desired: Safe<G, N>, agents: BTreeMap<MachineId, Box<dyn SledAgent<N>>>) -> Self {
        Controller {
            desired,
            agents,
            assignments: BTreeMap::new(),
            specs: BTreeMap::new(),
            epochs: BTreeMap::new(),
        }
    }

    /// Current gauge value of the desired fleet (≤ threshold by construction).
    pub fn desired_gauge(&self) -> f64 {
        self.desired.gauge()
    }

    /// Which node a workload is assigned to, if any.
    pub fn node_of(&self, id: WorkloadId) -> Option<MachineId> {
        self.assignments.get(&id).copied()
    }

    /// Resulting gauge if `request` were placed on `node`, or `None` if the
    /// node is unknown. A pure read — clones the fleet, never the `Safe`.
    fn resulting_gauge_if_placed(&self, node: MachineId, request: Mass<N>) -> Option<f64> {
        let mut trial = self.desired.fleet().clone();
        trial.add_load(node, request).ok()?;
        Some(self.desired.gauge_ref().eval(&trial))
    }

    /// Pick the feasible node that leaves the fleet most balanced (lowest
    /// resulting gauge). `None` if no node can host it under threshold.
    fn choose_node(&self, request: Mass<N>) -> Option<MachineId> {
        self.desired
            .fleet()
            .iter()
            .filter_map(|(id, _)| {
                let g = self.resulting_gauge_if_placed(id, request)?;
                (g <= self.desired.threshold()).then_some((id, g))
            })
            .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(id, _)| id)
    }

    /// Submit a workload: choose a node under the gauge, commit it to desired,
    /// and publish the node's new intent. The placement is the type-enforced
    /// safety point — an over-threshold submit is rejected here.
    pub fn submit(&mut self, spec: WorkloadSpec<N>) -> Result<MachineId, SubmitError> {
        let target = self.choose_node(spec.request).ok_or(SubmitError::NoFeasibleNode)?;
        // Commit to desired. Pre-selected as feasible, so this should not fail;
        // if it does, surface it rather than corrupt state.
        self.desired.try_place(target, spec.request).map_err(SubmitError::Gauge)?;
        self.assignments.insert(spec.id, target);
        self.specs.insert(spec.id, spec);
        self.publish(target)?;
        Ok(target)
    }

    /// (Re)publish a node's complete desired set as a fresh intent, bumping
    /// its epoch. Idempotent on the agent side.
    fn publish(&mut self, node: MachineId) -> Result<(), SubmitError> {
        let epoch = {
            let e = self.epochs.entry(node).or_insert(0);
            *e += 1;
            *e
        };
        let desired: Vec<WorkloadSpec<N>> = self
            .assignments
            .iter()
            .filter(|(_, &n)| n == node)
            .filter_map(|(wid, _)| self.specs.get(wid).cloned())
            .collect();
        let intent = NodeIntent { node, epoch, desired };
        let agent = self.agents.get_mut(&node).ok_or(SubmitError::UnknownNode(node))?;
        agent.apply_intent(&intent).map_err(SubmitError::Agent)
    }

    /// One reconcile pass: fetch each node's observed state, and where it
    /// diverges from desired, re-assert the node's intent. Returns a per-node
    /// summary. This is the loop that drives observed → safe desired.
    pub fn reconcile(&mut self) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        let nodes: Vec<MachineId> = self.agents.keys().copied().collect();

        for node in nodes {
            let observed = match self.agents.get(&node).and_then(|a| a.report().ok()) {
                Some(r) => r,
                None => {
                    report.unreachable.push(node);
                    continue;
                }
            };

            let desired_ids: BTreeSet<WorkloadId> = self
                .assignments
                .iter()
                .filter(|(_, &n)| n == node)
                .map(|(w, _)| *w)
                .collect();
            let running_ids: BTreeSet<WorkloadId> = observed
                .workloads
                .iter()
                .filter(|w| w.phase == WorkloadPhase::Running)
                .map(|w| w.id)
                .collect();

            if desired_ids == running_ids {
                report.converged.push(node);
            } else {
                report.diverged.push(node);
                if self.publish(node).is_ok() {
                    report.republished.push(node);
                }
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use fulcrum_dictionary::{Capacity, Fleet, MachineId, Mass};
    use fulcrum_laboratory::gauge::Linfty;
    use fulcrum_proto::{NodeReport, WorkloadStatus};

    /// Shared mutable node state, so a test can inject faults into an agent it
    /// has already handed to the controller (which only sees `dyn SledAgent`).
    struct SimState<const N: usize> {
        node: MachineId,
        capacity: Capacity<N>,
        epoch: u64,
        running: Vec<WorkloadSpec<N>>,
    }

    #[derive(Clone)]
    struct SimAgent<const N: usize> {
        state: Rc<RefCell<SimState<N>>>,
    }

    impl<const N: usize> SimAgent<N> {
        fn new(node: MachineId, capacity: Capacity<N>) -> Self {
            SimAgent {
                state: Rc::new(RefCell::new(SimState {
                    node,
                    capacity,
                    epoch: 0,
                    running: Vec::new(),
                })),
            }
        }
        /// Fault injection: a running workload exits (crashes) out of band.
        fn crash(&self, id: WorkloadId) {
            self.state.borrow_mut().running.retain(|w| w.id != id);
        }
        fn running_ids(&self) -> Vec<WorkloadId> {
            self.state.borrow().running.iter().map(|w| w.id).collect()
        }
    }

    impl<const N: usize> SledAgent<N> for SimAgent<N> {
        fn apply_intent(&mut self, intent: &NodeIntent<N>) -> Result<(), AgentError> {
            let mut s = self.state.borrow_mut();
            if intent.node != s.node {
                return Err(AgentError::UnknownNode(intent.node));
            }
            if intent.epoch < s.epoch {
                return Err(AgentError::StaleEpoch { intent: intent.epoch, current: s.epoch });
            }
            s.epoch = intent.epoch;
            s.running = intent.desired.clone();
            Ok(())
        }

        fn report(&self) -> Result<NodeReport<N>, AgentError> {
            let s = self.state.borrow();
            let mut observed = [0u64; N];
            for w in &s.running {
                for d in 0..N {
                    observed[d] = observed[d].saturating_add(w.request[d]);
                }
            }
            let workloads = s
                .running
                .iter()
                .map(|w| WorkloadStatus { id: w.id, phase: WorkloadPhase::Running })
                .collect();
            Ok(NodeReport {
                node: s.node,
                epoch: s.epoch,
                capacity: s.capacity,
                observed_load: Mass(observed),
                workloads,
            })
        }
    }

    /// Build a 3-node controller with handles to each agent for fault injection.
    fn three_node() -> (Controller<Linfty<2>, 2>, Vec<SimAgent<2>>) {
        let mut fleet: Fleet<2> = Fleet::new();
        let mut handles = Vec::new();
        let mut agents: BTreeMap<MachineId, Box<dyn SledAgent<2>>> = BTreeMap::new();
        for i in 1..=3u64 {
            let id = MachineId(i);
            fleet.add_machine(id, Capacity([100, 100]), Mass([0, 0]));
            let agent = SimAgent::new(id, Capacity([100, 100]));
            handles.push(agent.clone());
            agents.insert(id, Box::new(agent));
        }
        let desired: Safe<Linfty<2>, 2> = Safe::new(fleet, 0.9).unwrap();
        (Controller::new(desired, agents), handles)
    }

    fn spec(id: u64, c: u64, m: u64) -> WorkloadSpec<2> {
        WorkloadSpec { id: WorkloadId(id), image: "test".into(), request: Mass([c, m]) }
    }

    #[test]
    fn submit_places_under_the_gauge_and_runs_on_an_agent() {
        let (mut ctl, handles) = three_node();
        let node = ctl.submit(spec(1, 40, 30)).unwrap();
        assert!(ctl.desired_gauge() <= 0.9);
        assert_eq!(ctl.node_of(WorkloadId(1)), Some(node));
        // The agent for the chosen node is actually running it.
        let agent = &handles[(node.0 - 1) as usize];
        assert_eq!(agent.running_ids(), vec![WorkloadId(1)]);
    }

    #[test]
    fn submit_spreads_load_across_nodes() {
        let (mut ctl, _h) = three_node();
        // Three equal workloads should land on three distinct nodes (lowest
        // resulting gauge each time picks an empty node).
        let n1 = ctl.submit(spec(1, 50, 50)).unwrap();
        let n2 = ctl.submit(spec(2, 50, 50)).unwrap();
        let n3 = ctl.submit(spec(3, 50, 50)).unwrap();
        let distinct: BTreeSet<_> = [n1, n2, n3].into_iter().collect();
        assert_eq!(distinct.len(), 3);
        assert!(ctl.desired_gauge() <= 0.9);
    }

    #[test]
    fn submit_rejects_when_no_node_fits_under_threshold() {
        let (mut ctl, _h) = three_node();
        // Fill every node to 0.80 (under the 0.90 cap).
        ctl.submit(spec(1, 80, 0)).unwrap();
        ctl.submit(spec(2, 80, 0)).unwrap();
        ctl.submit(spec(3, 80, 0)).unwrap();
        // A workload needing 0.15 more CPU can't fit anywhere (0.80+0.15>0.90).
        assert_eq!(ctl.submit(spec(4, 15, 0)), Err(SubmitError::NoFeasibleNode));
    }

    #[test]
    fn reconcile_reasserts_intent_after_a_crash() {
        let (mut ctl, handles) = three_node();
        let node = ctl.submit(spec(1, 40, 30)).unwrap();
        let agent = &handles[(node.0 - 1) as usize];
        assert_eq!(agent.running_ids(), vec![WorkloadId(1)]);

        // The workload crashes out of band.
        agent.crash(WorkloadId(1));
        assert!(agent.running_ids().is_empty());

        // Reconcile must notice the divergence and re-assert the node's intent.
        let report = ctl.reconcile();
        assert!(report.diverged.contains(&node));
        assert!(report.republished.contains(&node));
        // The agent is running it again.
        assert_eq!(agent.running_ids(), vec![WorkloadId(1)]);
    }

    #[test]
    fn reconcile_is_quiet_when_reality_matches() {
        let (mut ctl, _h) = three_node();
        ctl.submit(spec(1, 40, 30)).unwrap();
        ctl.submit(spec(2, 20, 50)).unwrap();
        let report = ctl.reconcile();
        assert!(report.diverged.is_empty());
        assert!(report.republished.is_empty());
        assert_eq!(report.converged.len(), 3); // all three nodes match desired
    }
}
