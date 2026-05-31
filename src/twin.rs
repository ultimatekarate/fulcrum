//! The cluster digital twin: a deterministic simulation harness.
//!
//! This is the `hands` layer's second inhabitant (after `replay.rs`). It
//! *drives* the move algebra rather than classifying a trace: it stands up
//! a heterogeneous `Fleet<4>` from a [`fulcrum_dictionary::cluster`] topology, generates
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

use fulcrum_dictionary::cluster::turing_pi_2;
use fulcrum_laboratory::gauge::{Linfty, SchurConvex};
use fulcrum_dictionary::load::{Capacity, Fleet, MachineId, MachineSpec, Mass};
use fulcrum_laboratory::move_kind::Remove;
use crate::planner::{
    evaluate_pair, LeastLoaded, MaxMinFair, MaxMinFairGreedy, PairVerdict, Planner, TypedMove,
};
use fulcrum_dictionary::power::{Power, PowerBudget, PowerCoeffs};
use fulcrum_laboratory::power_eval::fleet_power;
use fulcrum_laboratory::safe::{GaugeError, Safe};
use fulcrum_dictionary::trace::{MoveHistory, MoveRecord};

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
    coeffs: Vec<fulcrum_dictionary::power::PowerCoeffs>,
    budget: PowerBudget,
    history: MoveHistory<4>,
    timeline: Vec<TimelineRow>,
    stats: SimStats,
    /// When false, applied moves are not appended to `history`/`timeline` (only
    /// `stats` advance). Default true. Large-fleet, high-move-count drivers (the
    /// churn benchmark) turn it off: a per-move timeline row is O(machines), so
    /// recording tens of thousands of moves over a 256-node fleet is a needless
    /// O(moves · machines) memory blow-up when only the counters are wanted.
    record_enabled: bool,
}

impl<G: SchurConvex<4>> Sim<G> {
    /// Start a simulation from an initial safe state, per-node coefficients
    /// (parallel to `safe.fleet().iter()`), and a power budget.
    pub fn new(safe: Safe<G, 4>, coeffs: Vec<fulcrum_dictionary::power::PowerCoeffs>, budget: PowerBudget) -> Self {
        Self {
            safe: Some(safe),
            coeffs,
            budget,
            history: MoveHistory::new(),
            timeline: Vec::new(),
            stats: SimStats::default(),
            record_enabled: true,
        }
    }

    /// Toggle per-move history/timeline recording. Off ⇒ only `stats` advance
    /// (used by the churn benchmark, which reads counters, not the timeline).
    pub fn set_recording(&mut self, on: bool) {
        self.record_enabled = on;
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
                            if self.record_enabled {
                                self.record(
                                    &s,
                                    "Place",
                                    MoveRecord::Place { machine: p.machine, mass: p.mass },
                                );
                            }
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
                if self.record_enabled {
                    self.record(&s, kind, record);
                }
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
/// `MaxMinFairGreedy`. Fixes the load gauge to `Linfty<4>` (worst-machine,
/// worst-dimension utilization).
///
/// Phase B uses the *multi-pair* greedy rebalancer rather than single-pair
/// `MaxMinFair`: the latter halts as soon as the global hottest/coldest pair
/// is capacity-guard-blocked, which (measured, see [`compare_rebalancers`])
/// strands admissible typed-pure transfers behind one blocked pair. The
/// greedy planner relieves the hottest source that *can* shed, so the
/// reference path makes the cheap (typed-pure) moves the single-pair planner
/// left on the table.
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
    // Multi-pair greedy: unlike single-pair `MaxMinFair`, it does not halt
    // when the global hottest/coldest pair is capacity-guard-blocked — it
    // relieves the hottest source that *can* shed, so it doesn't strand
    // admissible typed-pure transfers behind one blocked pair.
    let mut rebalancer = MaxMinFairGreedy::new(config.rebalance_epsilon);
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

/// The result of the rebalance-stall experiment: does `MaxMinFair` stop
/// because the fleet is *balanced*, or because the capacity guard *blocks* it
/// while imbalance remains — and on which dimension?
///
/// This is the falsifiable core of the elastic-carrier hypothesis. If Phase B
/// halts via `BelowEpsilon`, the carrier work is right-but-immaterial. If it
/// halts via `GuardBlocked` with `Emit`-able pairs left in `halt_scan`, then
/// typed-pure rebalancing is being suppressed — and the binding dimension of
/// those stalls says whether an *elastic* carrier (the guard is spurious on
/// elastic dims) or merely *more planner coverage* (try another pair) is the
/// right fix.
#[derive(Clone, Debug)]
pub struct RebalanceStallReport {
    /// `MaxMinFair`'s per-`step` verdicts over the real Phase-B run. Leading
    /// entries are the emitted migrations; the final entry is the halt reason.
    pub phase_b_log: Vec<PairVerdict>,
    /// Count of typed-pure `HotToCold` migrations Phase B actually applied.
    pub migrations_applied: usize,
    /// Every ordered `(src, dst)` pair evaluated against the **halted** fleet —
    /// the counterfactual "what else was available when the planner gave up".
    pub halt_scan: Vec<(MachineId, MachineId, PairVerdict)>,
    /// Load gauge after Phase B halted.
    pub halted_gauge: f64,
    /// Per-node `(id, worst-util)` at the halt, ascending id.
    pub halted_per_node: Vec<(MachineId, f64)>,
}

/// Run the Turing Pi 2 twin's two phases, then interrogate *why* the
/// `MaxMinFair` rebalance pass stopped. Phase A (placement) is byte-identical
/// to [`run_turing_pi_2_twin`]; Phase B uses an instrumented `MaxMinFair`
/// whose per-pair verdicts are logged (the log never feeds back into policy,
/// so behavior is unchanged). After the halt, every ordered machine pair is
/// re-evaluated against the final fleet via the *same* [`evaluate_pair`] the
/// planner uses — so the counterfactual cannot drift from the policy.
/// Stand up the fleet and run Phase A (least-loaded placement) only — the
/// shared prefix of [`run_turing_pi_2_twin`], the stall diagnostic, and the
/// rebalancer comparison. Deterministic in `config`, so callers that need the
/// *same* post-placement state just call it again.
fn placed_sim(config: TwinConfig) -> Result<Sim<Linfty<4>>, GaugeError> {
    let topo = turing_pi_2();
    let fleet: Fleet<4> = topo.fleet();
    let coeffs = topo.coeffs();
    let safe: Safe<Linfty<4>, 4> = Safe::new(fleet, config.threshold)?;

    let mut gen = WorkloadGen::new(config.seed, config.max_mass);
    let items = gen.take(config.n_workloads);

    let mut sim = Sim::new(safe, coeffs, config.budget);
    let mut placer = LeastLoaded::new(items);
    sim.drive(&mut placer);
    Ok(sim)
}

pub fn diagnose_turing_pi_2_rebalance(
    config: TwinConfig,
) -> Result<RebalanceStallReport, GaugeError> {
    let mut sim = placed_sim(config)?;

    // Phase B: instrumented rebalance.
    let mut rebalancer = MaxMinFair::new(config.rebalance_epsilon);
    sim.drive(&mut rebalancer);

    let phase_b_log: Vec<PairVerdict> = rebalancer.log().to_vec();
    let migrations_applied = phase_b_log.iter().filter(|v| v.is_emit()).count();

    // Counterfactual: re-evaluate every ordered pair against the halted fleet.
    let halted = sim.safe().fleet();
    let ids: Vec<MachineId> = halted.iter().map(|(id, _)| id).collect();
    let mut halt_scan = Vec::new();
    for &src in &ids {
        for &dst in &ids {
            if src == dst {
                continue;
            }
            let verdict = evaluate_pair(halted, src, dst, config.rebalance_epsilon);
            halt_scan.push((src, dst, verdict));
        }
    }
    let halted_per_node: Vec<(MachineId, f64)> = halted
        .iter()
        .map(|(id, spec)| (id, spec.worst_utilization()))
        .collect();
    let halted_gauge = sim.safe().gauge();

    Ok(RebalanceStallReport {
        phase_b_log,
        migrations_applied,
        halt_scan,
        halted_gauge,
        halted_per_node,
    })
}

/// Side-by-side result of the single-pair `MaxMinFair` baseline vs the
/// multi-pair `MaxMinFairGreedy`, on the *same* post-placement fleet.
///
/// The decisive field is [`Self::greedy_residual_guard_blocked`]: once the
/// greedy planner has made every admissible typed-pure transfer (so
/// `greedy_residual_emits == 0`, planner coverage is spent), whatever remains
/// capacity-guard-blocked is the **irreducible carrier residue** — the moves
/// that only an *elastic* carrier could unlock. This is the clean separation
/// the stall experiment asked for: planner coverage first, then what's left is
/// purely about carriers.
#[derive(Clone, Debug)]
pub struct RebalanceComparison {
    /// `MaxMinFair` (single global pair): migrations applied.
    pub baseline_migrations: usize,
    /// `MaxMinFair`: typed-pure ratio over the whole run (placements included).
    pub baseline_ratio: f64,
    /// `MaxMinFair`: load gauge at halt.
    pub baseline_gauge: f64,
    /// `MaxMinFairGreedy` (all pairs): migrations applied.
    pub greedy_migrations: usize,
    /// `MaxMinFairGreedy`: typed-pure ratio over the whole run.
    pub greedy_ratio: f64,
    /// `MaxMinFairGreedy`: load gauge at halt.
    pub greedy_gauge: f64,
    /// After greedy exhausts: the guard-blocked residue as `(src, dst, worst_d)`
    /// — the pure carrier opportunity.
    pub greedy_residual_guard_blocked: Vec<(MachineId, MachineId, usize)>,
    /// After greedy exhausts: count of any `Emit` pairs still available. Should
    /// be 0 — the greedy planner does not leave admissible moves on the table.
    pub greedy_residual_emits: usize,
}

/// Run both rebalancers from the identical post-placement fleet and compare:
/// how many more typed-pure moves the multi-pair planner makes, how much it
/// lifts the typed-pure ratio, and — crucially — what stays capacity-guard
/// blocked after it exhausts (the carrier residue).
pub fn compare_rebalancers(config: TwinConfig) -> Result<RebalanceComparison, GaugeError> {
    // Baseline: single-global-pair MaxMinFair.
    let mut base = placed_sim(config)?;
    let mut mmf = MaxMinFair::new(config.rebalance_epsilon);
    base.drive(&mut mmf);
    let baseline_migrations = base.stats().typed_pure_applied as usize;
    let baseline_ratio = base.history().typed_pure_ratio();
    let baseline_gauge = base.safe().gauge();

    // Multi-pair greedy, from the same Phase-A state (deterministic re-run).
    let mut greedy_sim = placed_sim(config)?;
    let mut greedy = MaxMinFairGreedy::new(config.rebalance_epsilon);
    greedy_sim.drive(&mut greedy);
    let greedy_migrations = greedy_sim.stats().typed_pure_applied as usize;
    let greedy_ratio = greedy_sim.history().typed_pure_ratio();
    let greedy_gauge = greedy_sim.safe().gauge();

    // The residue after greedy exhausts: what's still guard-blocked (carrier
    // opportunity) and a sanity count of any Emit left (must be 0).
    let halted = greedy_sim.safe().fleet();
    let ids: Vec<MachineId> = halted.iter().map(|(id, _)| id).collect();
    let mut greedy_residual_guard_blocked = Vec::new();
    let mut greedy_residual_emits = 0usize;
    for &src in &ids {
        for &dst in &ids {
            if src == dst {
                continue;
            }
            match evaluate_pair(halted, src, dst, config.rebalance_epsilon) {
                PairVerdict::GuardBlocked { worst_d, .. } => {
                    greedy_residual_guard_blocked.push((src, dst, worst_d));
                }
                PairVerdict::Emit { .. } => greedy_residual_emits += 1,
                _ => {}
            }
        }
    }

    Ok(RebalanceComparison {
        baseline_migrations,
        baseline_ratio,
        baseline_gauge,
        greedy_migrations,
        greedy_ratio,
        greedy_gauge,
        greedy_residual_guard_blocked,
        greedy_residual_emits,
    })
}

/// The fully-rebalanced (post-`MaxMinFairGreedy`) fleet, with everything a
/// caller needs to ask whether the leftover residue actually *matters*: the
/// per-machine specs (load + capacity, so both utilization and **absolute** free
/// capacity are recoverable), the node power coefficients, the gauge, the
/// threshold, and the guard-blocked residue.
#[derive(Clone, Debug)]
pub struct GreedyOutcome {
    /// Post-greedy per-machine specs, ascending id (parallel to `coeffs`).
    pub specs: Vec<(MachineId, MachineSpec<4>)>,
    /// Per-node power coefficients, parallel to `specs`.
    pub coeffs: Vec<fulcrum_dictionary::power::PowerCoeffs>,
    /// Load gauge (worst-machine worst-dim utilization) at the stuck state.
    pub gauge: f64,
    /// The `Safe` threshold τ — the only hard bound the residue must respect.
    pub threshold: f64,
    /// Capacity-guard residue as `(src, dst, worst_d)` (the stuck transfers).
    pub residual_guard_blocked: Vec<(MachineId, MachineId, usize)>,
}

/// Drive the fleet to the multi-pair-greedy stuck state and hand back the full
/// post-rebalance snapshot, so callers can measure the operational significance
/// of whatever residue remains (safety headroom, absolute free capacity,
/// admission slack) rather than just its existence.
pub fn greedy_outcome(config: TwinConfig) -> Result<GreedyOutcome, GaugeError> {
    let mut sim = placed_sim(config)?;
    let mut greedy = MaxMinFairGreedy::new(config.rebalance_epsilon);
    sim.drive(&mut greedy);

    let gauge = sim.safe().gauge();
    let fleet = sim.safe().fleet();
    let specs: Vec<(MachineId, MachineSpec<4>)> = fleet.iter().map(|(id, s)| (id, *s)).collect();

    let ids: Vec<MachineId> = fleet.iter().map(|(id, _)| id).collect();
    let mut residual_guard_blocked = Vec::new();
    for &src in &ids {
        for &dst in &ids {
            if src == dst {
                continue;
            }
            if let PairVerdict::GuardBlocked { worst_d, .. } =
                evaluate_pair(fleet, src, dst, config.rebalance_epsilon)
            {
                residual_guard_blocked.push((src, dst, worst_d));
            }
        }
    }

    Ok(GreedyOutcome {
        specs,
        coeffs: turing_pi_2().coeffs(),
        gauge,
        threshold: config.threshold,
        residual_guard_blocked,
    })
}

/// A one-shot planner that emits a single `Remove` (a workload departing), then
/// halts. Lets the churn driver inject a typed-pure removal through the same
/// `Sim` apply/accounting path the reference planners use.
struct RemoveOnce {
    mv: Option<(MachineId, Mass<4>)>,
}

impl<G: SchurConvex<4>> Planner<4, G> for RemoveOnce {
    fn step(&mut self, _safe: &Safe<G, 4>) -> Option<TypedMove<4>> {
        self.mv.take().map(|(m, mass)| TypedMove::Remove(Remove::new(m, mass)))
    }
}

/// Draw a per-dimension demand uniformly from `0..=max_mass[d]`.
fn draw_mass(rng: &mut Xorshift64, max_mass: &[u64; 4]) -> Mass<4> {
    let mut m = [0u64; 4];
    for d in 0..4 {
        m[d] = rng.next() % (max_mass[d] + 1);
    }
    Mass(m)
}

/// Pick a random non-empty machine and a per-dimension removal capped at its
/// current load (a job finishing). Returns `None` if the fleet is empty or the
/// draw is all-zero. The cap guarantees the typed-pure `Remove` never
/// underflows.
fn pick_departure(
    fleet: &Fleet<4>,
    rng: &mut Xorshift64,
    max_mass: &[u64; 4],
) -> Option<(MachineId, Mass<4>)> {
    let m = fleet.len();
    if m == 0 {
        return None;
    }
    let start = (rng.next() % m as u64) as usize;
    for off in 0..m {
        let id = MachineId(((start + off) % m) as u64 + 1);
        if let Some(load) = fleet.load(id) {
            if load.0.iter().any(|&x| x > 0) {
                let draw = draw_mass(rng, max_mass);
                let mut rm = [0u64; 4];
                for d in 0..4 {
                    rm[d] = draw.0[d].min(load.0[d]);
                }
                if rm.iter().any(|&x| x > 0) {
                    return Some((id, Mass(rm)));
                }
            }
        }
    }
    None
}

/// Configuration for the steady-state churn benchmark (benchmark #2): a
/// populated fleet under continuous arrival/departure, measuring how the
/// typed-pure ratio `p` responds as the workload shifts from cold-start
/// (placement-only) toward steady-state churn (departures + active rebalance).
#[derive(Clone, Copy, Debug)]
pub struct ChurnConfig {
    pub seed: u64,
    /// Fleet size M (synthetic uniform-capacity machines, ids `1..=M`).
    pub machines: usize,
    /// Uniform per-dimension capacity.
    pub capacity: u64,
    /// Cold-start placements used to populate the fleet (excluded from the
    /// measured steady-state window).
    pub fill: usize,
    /// Steady-state ticks to measure over.
    pub ticks: usize,
    /// Probability a tick is a departure (typed-pure `Remove`) rather than an
    /// arrival (catch-all `Place`). 0.5 ≈ steady state (mass neither grows nor
    /// shrinks); 0.0 ≈ cold-start (arrivals only).
    pub departure_prob: f64,
    /// Per-dimension max demand for a single workload.
    pub max_mass: [u64; 4],
    /// Load-gauge threshold τ.
    pub threshold: f64,
    /// Convergence epsilon for the rebalance pass.
    pub rebalance_epsilon: f64,
    /// Whether to rebalance to convergence (`MaxMinFairGreedy`) after each tick.
    pub rebalance_each_tick: bool,
}

/// Outcome of a steady-state churn run, measured over the steady-state window
/// only (the cold-start fill is excluded). The headline is
/// [`Self::typed_pure_ratio`] — the `p` that benchmark #1's realized speedup
/// `B/(p·A + (1−p)·B)` is gated by — decomposed into removals (jobs finishing)
/// vs migrations (active rebalancing), so the *source* of the lift is legible.
#[derive(Clone, Copy, Debug)]
pub struct ChurnReport {
    /// Applied `Place` moves (catch-all; the only rechecked kind).
    pub placements: u64,
    /// Arrivals rejected by the τ recheck (admission pressure).
    pub place_rejected: u64,
    /// Applied `Remove` moves (typed-pure; departures).
    pub removals: u64,
    /// Applied `HotToCold` moves (typed-pure; rebalancing).
    pub migrations: u64,
    /// `(removals + migrations) / (removals + migrations + placements)`.
    pub typed_pure_ratio: f64,
    /// Load gauge at the end of the run.
    pub final_gauge: f64,
}

/// Run a steady-state churn simulation and report the typed-pure ratio over the
/// steady-state window. Populates an M-machine fleet, then alternates arrivals
/// (`Place`) and departures (`Remove`) under `departure_prob`, optionally
/// rebalancing to convergence each tick. The cold-start fill is excluded from
/// the measured window, so the ratio reflects *operating* mix — not the one-time
/// bulk placement that pins the cold-start twin at 0.12.
///
/// Power is inert here (the churn question is move-mix, not energy): coeffs are
/// zero and the budget is infinite, so only the load gauge gates placements.
pub fn steady_state_churn(config: ChurnConfig) -> Result<ChurnReport, GaugeError> {
    let mut fleet: Fleet<4> = Fleet::new();
    for i in 0..config.machines {
        fleet.add_machine(MachineId(i as u64 + 1), Capacity([config.capacity; 4]), Mass([0; 4]));
    }
    let coeffs = vec![PowerCoeffs { idle: Power(0.0), dynamic: Power(0.0) }; config.machines];
    let safe: Safe<Linfty<4>, 4> = Safe::new(fleet, config.threshold)?;
    let mut sim = Sim::new(safe, coeffs, PowerBudget(Power(f64::INFINITY)));
    // Counters only — a per-move timeline row over a 256-node fleet × tens of
    // thousands of moves would be gigabytes for data we never read here.
    sim.set_recording(false);

    let mut rng = Xorshift64::new(config.seed);

    // Cold-start fill (excluded from the measured window).
    let fill_items: Vec<Mass<4>> =
        (0..config.fill).map(|_| draw_mass(&mut rng, &config.max_mass)).collect();
    sim.drive(&mut LeastLoaded::new(fill_items));
    if config.rebalance_each_tick {
        sim.drive(&mut MaxMinFairGreedy::new(config.rebalance_epsilon));
    }
    let s0 = sim.stats();

    let mut removals: u64 = 0;
    for _ in 0..config.ticks {
        let coin = (rng.next() % 1_000_000) as f64 / 1_000_000.0;
        if coin < config.departure_prob {
            if let Some((id, mass)) = pick_departure(sim.safe().fleet(), &mut rng, &config.max_mass) {
                sim.drive(&mut RemoveOnce { mv: Some((id, mass)) });
                removals += 1;
            }
        } else {
            let demand = draw_mass(&mut rng, &config.max_mass);
            sim.drive(&mut LeastLoaded::new(vec![demand]));
        }
        if config.rebalance_each_tick {
            sim.drive(&mut MaxMinFairGreedy::new(config.rebalance_epsilon));
        }
    }
    let s1 = sim.stats();

    let placements = s1.placed - s0.placed;
    let typed_pure = s1.typed_pure_applied - s0.typed_pure_applied;
    let migrations = typed_pure.saturating_sub(removals);
    let place_rejected = (s1.load_rejected - s0.load_rejected)
        + (s1.power_rejected - s0.power_rejected)
        + (s1.malformed - s0.malformed);

    let denom = (typed_pure + placements).max(1);
    Ok(ChurnReport {
        placements,
        place_rejected,
        removals,
        migrations,
        typed_pure_ratio: typed_pure as f64 / denom as f64,
        final_gauge: sim.safe().gauge(),
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
    use fulcrum_dictionary::power::{Power, PowerBudget};

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

    /// The reference twin's Phase B is `MaxMinFairGreedy`. Its final state must
    /// match the greedy arm of [`compare_rebalancers`] exactly — same seeded
    /// post-placement fleet, same planner — which guards against a silent
    /// revert to the single-pair `MaxMinFair` that stalls early. (Phase A is
    /// `LeastLoaded`, which emits only `Place`, so every typed-pure move is a
    /// Phase-B migration.)
    #[test]
    fn reference_twin_phase_b_is_greedy() {
        let cfg = config();
        let run = run_turing_pi_2_twin(cfg).unwrap();
        let cmp = compare_rebalancers(cfg).unwrap();

        assert!(
            (run.final_gauge - cmp.greedy_gauge).abs() < 1e-12,
            "reference final gauge {} != greedy arm {} (did Phase B revert to single-pair?)",
            run.final_gauge,
            cmp.greedy_gauge,
        );
        assert_eq!(run.stats.typed_pure_applied as usize, cmp.greedy_migrations);
    }
}
