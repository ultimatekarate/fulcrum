//! The cluster digital twin: a deterministic simulation harness.
//!
//! This is the `hands` layer's second inhabitant (after `replay.rs`). It
//! *drives* the move algebra rather than classifying a trace: it stands up
//! a heterogeneous `Fleet<4>` from a [`crate::cluster`] topology, generates
//! a reproducible stream of workload demands, runs the existing reference
//! planners, and threads `Safe<G, 4>` forward — enforcing the load gauge
//! (the typestate's job) and the fleet-wide power budget (a runtime
//! recheck) at every `Place`.
//!
//! ## The one new control point
//!
//! Under the convex power model, only `Place` (mass-increasing) can raise
//! fleet power; `HotToCold` / `Neutral` / `Remove` are power-non-increasing.
//! So the budget is enforced as a **projection recheck at `Place`**: clone
//! the fleet, apply the placement to the clone, and reject (counting it)
//! if the projected draw exceeds the ceiling — mirroring how the load gauge
//! is rechecked. Projecting on a clone means a rejection leaves the live
//! `Safe` untouched (the fallible `apply_with_recheck` would otherwise
//! consume it).
//!
//! ## Determinism
//!
//! The workload generator is a deterministically seeded xorshift64 PRNG (no
//! `system_clock`). Same seed ⇒ same stream ⇒ bit-identical timeline.

use crate::cluster::turing_pi_2;
use crate::gauge::{Linfty, SchurConvex};
use crate::load::{Fleet, MachineId, Mass};
use crate::planner::{LeastLoaded, MaxMinFair, Planner, TypedMove};
use crate::power::{Power, PowerBudget};
use crate::power_eval::fleet_power;
use crate::safe::{GaugeError, Safe};
use crate::trace::{MoveHistory, MoveRecord};

/// Inline xorshift64 PRNG — same approach as `planner::power_of_two`.
/// Deterministic; not cryptographic.
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        // xorshift64 has a fixed point at zero — coerce to keep it total.
        Xorshift64 { state: seed.max(1) }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

/// Seeded generator of `Mass<4>` workload demands, one per `next()`.
///
/// Each dimension is drawn uniformly from `0..=max_mass[d]`.
pub struct WorkloadGen {
    rng: Xorshift64,
    max_mass: [u64; 4],
}

impl WorkloadGen {
    /// Construct from a seed and per-dimension maximum demand.
    pub fn new(seed: u64, max_mass: [u64; 4]) -> Self {
        Self { rng: Xorshift64::new(seed), max_mass }
    }

    /// Draw the next workload demand vector.
    pub fn next(&mut self) -> Mass<4> {
        let mut m = [0u64; 4];
        for d in 0..4 {
            m[d] = self.rng.next() % (self.max_mass[d] + 1);
        }
        Mass(m)
    }

    /// Draw `count` demands.
    pub fn take(&mut self, count: usize) -> Vec<Mass<4>> {
        (0..count).map(|_| self.next()).collect()
    }
}

/// Outcome counters over a simulation run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimStats {
    /// Placements that passed both the budget and the load-gauge recheck.
    pub placed: u64,
    /// Placements rejected because projected fleet power exceeded the budget.
    pub power_rejected: u64,
    /// Placements rejected because the projected load gauge exceeded τ.
    pub load_rejected: u64,
    /// Placements skipped because the move was malformed (unknown machine).
    pub malformed: u64,
    /// Typed-pure moves applied (migrations / removals) — total at the type
    /// level, no recheck.
    pub typed_pure_applied: u64,
}

/// One timeline row, recorded after each *applied* move.
#[derive(Clone, Debug)]
pub struct TimelineRow {
    /// 1-based index of the applied move.
    pub step: usize,
    /// Kind name of the move applied at this step.
    pub kind: &'static str,
    /// Load gauge value after the move.
    pub gauge: f64,
    /// Total fleet power draw after the move.
    pub power: Power,
    /// Typed-pure ratio over the history so far.
    pub typed_pure_ratio: f64,
    /// Per-node worst-dimension utilization after the move (ascending
    /// `MachineId`).
    pub per_node: Vec<(MachineId, f64)>,
}

/// A running simulation over a fixed gauge `G`, four resource dimensions.
///
/// Owns the live `Safe` (in an `Option` so moves — which consume `Safe` —
/// can take it and put the successor back), the per-node power coefficients,
/// the budget, and the accumulated history / timeline / stats.
pub struct Sim<G: SchurConvex<4>> {
    safe: Option<Safe<G, 4>>,
    coeffs: Vec<crate::power::PowerCoeffs>,
    budget: PowerBudget,
    history: MoveHistory<4>,
    timeline: Vec<TimelineRow>,
    stats: SimStats,
}

impl<G: SchurConvex<4>> Sim<G> {
    /// Start a simulation from an initial safe state, per-node coefficients
    /// (parallel to `safe.fleet().iter()`), and a power budget.
    pub fn new(safe: Safe<G, 4>, coeffs: Vec<crate::power::PowerCoeffs>, budget: PowerBudget) -> Self {
        Self {
            safe: Some(safe),
            coeffs,
            budget,
            history: MoveHistory::new(),
            timeline: Vec::new(),
            stats: SimStats::default(),
        }
    }

    /// Run a planner to exhaustion, applying every move it emits.
    pub fn drive<P: Planner<4, G>>(&mut self, planner: &mut P) {
        while let Some(mv) = planner.step(self.safe.as_ref().expect("safe present between moves")) {
            self.apply(mv);
        }
    }

    fn apply(&mut self, mv: TypedMove<4>) {
        let safe = self.safe.take().expect("safe present at apply");
        let safe = match mv {
            TypedMove::Place(p) => {
                // Project the placement on a cloned fleet so a rejection
                // leaves the live `Safe` untouched.
                let mut projected = safe.fleet().clone();
                match projected.add_load(p.machine, p.mass) {
                    Err(_) => {
                        self.stats.malformed += 1;
                        safe
                    }
                    Ok(()) => {
                        let proj_power = fleet_power(&projected, &self.coeffs);
                        let proj_gauge = safe.gauge_ref().eval(&projected);
                        if !self.budget.within(proj_power) {
                            self.stats.power_rejected += 1;
                            safe
                        } else if proj_gauge > safe.threshold() {
                            self.stats.load_rejected += 1;
                            safe
                        } else {
                            // Both prechecks pass ⇒ the recheck inside
                            // apply_with_recheck cannot fail.
                            let s = p
                                .apply_with_recheck(safe)
                                .expect("prechecked: within gauge and budget");
                            self.stats.placed += 1;
                            self.record(&s, "Place", MoveRecord::Place { machine: p.machine, mass: p.mass });
                            s
                        }
                    }
                }
            }
            other => {
                // MaxMinFair (and the other reference planners) only emit
                // Place or typed-pure moves. Typed-pure `apply` is total; a
                // catch-all ColdToHot would be fallible, but no reference
                // planner emits one, so the expect documents that contract.
                debug_assert!(
                    other.is_typed_pure(),
                    "twin driver expects Place or typed-pure moves; got {}",
                    other.kind_name(),
                );
                let kind = other.kind_name();
                let record = record_of(&other);
                let s = other
                    .apply(safe)
                    .expect("twin driver: non-Place move must be typed-pure");
                self.stats.typed_pure_applied += 1;
                self.record(&s, kind, record);
                s
            }
        };
        self.safe = Some(safe);
    }

    fn record(&mut self, safe: &Safe<G, 4>, kind: &'static str, record: MoveRecord<4>) {
        self.history.push(record);
        let per_node = safe
            .fleet()
            .iter()
            .map(|(id, spec)| (id, spec.worst_utilization()))
            .collect();
        self.timeline.push(TimelineRow {
            step: self.timeline.len() + 1,
            kind,
            gauge: safe.gauge(),
            power: fleet_power(safe.fleet(), &self.coeffs),
            typed_pure_ratio: self.history.typed_pure_ratio(),
            per_node,
        });
    }

    /// Borrow the live safe state.
    pub fn safe(&self) -> &Safe<G, 4> {
        self.safe.as_ref().expect("safe present")
    }

    /// The recorded timeline.
    pub fn timeline(&self) -> &[TimelineRow] {
        &self.timeline
    }

    /// The move history.
    pub fn history(&self) -> &MoveHistory<4> {
        &self.history
    }

    /// Outcome counters.
    pub fn stats(&self) -> SimStats {
        self.stats
    }

    /// Current total fleet power draw.
    pub fn power(&self) -> Power {
        fleet_power(self.safe().fleet(), &self.coeffs)
    }
}

/// Reconstruct a `MoveRecord` from a `TypedMove` for the history. Exhaustive
/// over `TypedMove` (no `_` arm), per `basis.yaml`'s exhaustive-matching rule.
fn record_of(mv: &TypedMove<4>) -> MoveRecord<4> {
    match mv {
        TypedMove::Remove(m) => MoveRecord::Remove { machine: m.machine, mass: m.mass },
        TypedMove::HotToCold(m) => {
            MoveRecord::HotToCold { source: m.source, destination: m.destination, mass: m.mass }
        }
        TypedMove::Neutral(m) => {
            MoveRecord::Neutral { source: m.source, destination: m.destination, mass: m.mass }
        }
        TypedMove::ColdToHot(m) => {
            MoveRecord::ColdToHot { source: m.source, destination: m.destination, mass: m.mass }
        }
        TypedMove::Place(m) => MoveRecord::Place { machine: m.machine, mass: m.mass },
    }
}

/// Configuration for the Turing Pi 2 reference twin.
#[derive(Clone, Copy, Debug)]
pub struct TwinConfig {
    /// PRNG seed for the workload generator.
    pub seed: u64,
    /// Number of workload demands to generate and place.
    pub n_workloads: usize,
    /// Per-dimension maximum demand for a single workload.
    pub max_mass: [u64; 4],
    /// Load-gauge threshold τ (the `Safe` bound).
    pub threshold: f64,
    /// Fleet-wide power ceiling.
    pub budget: PowerBudget,
    /// Convergence epsilon for the `MaxMinFair` rebalance pass.
    pub rebalance_epsilon: f64,
}

/// The full result of a twin run.
pub struct TwinReport {
    pub timeline: Vec<TimelineRow>,
    pub history: MoveHistory<4>,
    pub stats: SimStats,
    pub final_gauge: f64,
    pub final_power: Power,
    pub typed_pure_ratio: f64,
}

/// Run the reference Turing Pi 2 twin: build the topology, generate a seeded
/// workload, place it least-loaded under the budget, then rebalance with
/// `MaxMinFair`. Fixes the load gauge to `Linfty<4>` (worst-machine,
/// worst-dimension utilization).
///
/// Returns an error only if the *empty* starting fleet somehow exceeds τ
/// (it cannot, but the constructor is fallible).
pub fn run_turing_pi_2_twin(config: TwinConfig) -> Result<TwinReport, GaugeError> {
    let topo = turing_pi_2();
    let fleet: Fleet<4> = topo.fleet();
    let coeffs = topo.coeffs();
    let safe: Safe<Linfty<4>, 4> = Safe::new(fleet, config.threshold)?;

    let mut gen = WorkloadGen::new(config.seed, config.max_mass);
    let items = gen.take(config.n_workloads);

    let mut sim = Sim::new(safe, coeffs, config.budget);

    // Phase A: placement (catch-all `Place`, budget + gauge rechecked).
    let mut placer = LeastLoaded::new(items);
    sim.drive(&mut placer);

    // Phase B: rebalance (typed-pure `HotToCold`, power-non-increasing).
    let mut rebalancer = MaxMinFair::new(config.rebalance_epsilon);
    sim.drive(&mut rebalancer);

    Ok(TwinReport {
        timeline: sim.timeline().to_vec(),
        history: sim.history().clone(),
        stats: sim.stats(),
        final_gauge: sim.safe().gauge(),
        final_power: sim.power(),
        typed_pure_ratio: sim.history().typed_pure_ratio(),
    })
}

/// Render a timeline as CSV. A pure string builder; the caller does the IO.
pub fn timeline_to_csv(rows: &[TimelineRow]) -> String {
    let mut out = String::from("step,kind,gauge,power_mw,typed_pure_ratio\n");
    for r in rows {
        out.push_str(&format!(
            "{},{},{:.6},{:.3},{:.6}\n",
            r.step,
            r.kind,
            r.gauge,
            r.power.milliwatts(),
            r.typed_pure_ratio,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::{Power, PowerBudget};

    fn config() -> TwinConfig {
        TwinConfig {
            seed: 0xC0FFEE,
            n_workloads: 24,
            max_mass: [120, 1200, 150, 150],
            threshold: 0.95,
            budget: PowerBudget(Power(32_000.0)),
            rebalance_epsilon: 1e-3,
        }
    }

    #[test]
    fn deterministic_for_fixed_seed() {
        let a = run_turing_pi_2_twin(config()).unwrap();
        let b = run_turing_pi_2_twin(config()).unwrap();
        assert_eq!(a.stats, b.stats);
        assert_eq!(a.timeline.len(), b.timeline.len());
        assert!((a.final_gauge - b.final_gauge).abs() < 1e-12);
        assert!((a.final_power.milliwatts() - b.final_power.milliwatts()).abs() < 1e-9);
    }

    #[test]
    fn never_exceeds_budget() {
        let report = run_turing_pi_2_twin(config()).unwrap();
        let budget = config().budget;
        for row in &report.timeline {
            assert!(
                budget.within(row.power),
                "step {} drew {:?} over budget {:?}",
                row.step,
                row.power,
                budget,
            );
        }
    }

    #[test]
    fn workload_gen_is_seed_reproducible() {
        let mut a = WorkloadGen::new(42, [100, 100, 100, 100]);
        let mut b = WorkloadGen::new(42, [100, 100, 100, 100]);
        assert_eq!(a.take(10), b.take(10));
    }
}
