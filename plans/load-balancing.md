# Plan: Extending Fulcrum for load-balancing use

## Context

Fulcrum's v0 is a typed move algebra over a single-dimensional load vector
with uniform capacity. It demonstrates the principle but is not deployable as
a load-balancer or scheduler — production workloads need heterogeneous
capacity, multi-dimensional load (CPU + memory at minimum), and a planner
that emits typed moves in response to events. This plan extends fulcrum to
the smallest set of additions that makes it usable for load balancing while
preserving the algebraic-correctness properties that justified the framework.

The plan has three parts:

1. A scoping clarification — "load balancing" is ambiguous between
   *request routing* (sub-millisecond, stateless) and *scheduling*
   (per-workload, stateful). Fulcrum's algebra fits scheduling; routing is a
   different shape of problem and is out of scope here.
2. A survey of famous load-balancing and scheduling algorithms, with each
   mapped to how it would express through fulcrum's typestate.
3. The concrete extensions (heterogeneous capacity, multi-dim load, planner
   trait, reconciler), phased so each phase is independently meaningful.

## Scope clarification: load balancing vs. scheduling

The two terms get used loosely. They aren't the same problem:

| Aspect | Request routing (load balancer) | Scheduling (load distribution) |
| --- | --- | --- |
| Decision unit | Per-request | Per-workload |
| Time scale | Sub-millisecond | Seconds to minutes |
| State | Stateless / near-stateless | Stateful (placements persist) |
| Migration | Doesn't exist | Real operation |
| Fulcrum fit | Poor (sequence of decisions doesn't compose under typestate; routing is independent per request) | Good (fulcrum's primitives are scheduler-shaped) |

This plan targets scheduling. A fulcrum-based request router would need a
different design — possibly the feedback-shaped spectral-stability layer
applied to backend load distributions over time, with no Place/Remove
algebra. That's a separate project.

## Famous algorithms and their fulcrum mappings

Algorithms that fit fulcrum's algebra cleanly, ordered by how directly they
exercise the typestate:

| Algorithm | Reference | Fulcrum mapping |
| --- | --- | --- |
| **Max-min fair share** | Bertsekas-Gallager textbook | The canonical Pigou-Dalton iteration: each step is a `HotToCold` transfer, all typed-pure, total apply. The whole convergence is a Lyapunov descent under any Schur-convex gauge. |
| **JSQ (Join-shortest-queue)** | Winston 1977 (and many followups) | Greedy minimizer of $\ell_\infty$. Each placement targets `argmin` of utilization; expressed as `Place::apply_with_recheck`. Migration variant uses `HotToCold::witness` from any sled exceeding threshold to the shortest-queue sled — typed-pure if the witness fits. |
| **DRF (Dominant Resource Fairness)** | Ghodsi et al. NSDI 2011 | Multi-dimensional max-min. Requires fulcrum's multi-D extension. Once extended, same shape as max-min: typed transfers until DRF equilibrium, with the joint Schur-convex gauge as the Lyapunov function. |
| **Power-of-d-choices** | Mitzenmacher 1996, Vöcking 2003 | Sample d random sleds, place on the least loaded. Each placement is `Place::apply_with_recheck`. Typestate doesn't add much for individual decisions but tracks the cumulative invariant across sequences. Provides probabilistic balance guarantees that are bounded by the Schur-convex gauge fulcrum tracks. |
| **Best-fit-decreasing (BFD)** | Johnson 1973 | Bin-packing: sort items decreasing, each placed in the smallest fitting bin. Each placement is catch-all (`Place::apply_with_recheck`). Typestate doesn't reorder the algorithm but provides the structural guarantee that no individual placement violates capacity. |
| **Tetris** | Grandl et al. SIGCOMM 2014 | N-dimensional bin packing, vector-aware. Requires multi-D extension. Initial placements are catch-all; consolidation/defrag passes use typed-pure migrations. |
| **Filter-and-score (Kubernetes-style)** | kube-scheduler design | Filter feasible nodes, score each by a sum of weighted plugins, pick highest. Fulcrum's gauge value can be one scoring component; the final placement is `Place::apply_with_recheck`. The framework verifies the placement preserves the gauge bound that scoring optimized for. |
| **Quincy / Firmament** | Isard et al. SOSP 2009; Gog et al. OSDI 2016 | Min-cost flow LP for placement. The LP runs outside fulcrum and emits a set of placement decisions; each decision is wrapped as a typed `Place` or `HotToCold` (where applicable) and applied through the framework. The framework provides the per-decision typestate guarantee; the LP provides the global optimality. |

Algorithms that don't fit cleanly:

- **Round-robin / weighted round-robin**: doesn't reason about load, so the
  typestate's algebra is a no-op. Each placement is `apply_with_recheck`.
  The framework adds nothing structural; the discipline is wasted.
- **Consistent hashing / rendezvous hashing / Maglev**: deterministic
  per-request routing. Doesn't fit because there's no notion of "moving"
  state — the hash function is the planner.
- **Lottery scheduling / stride scheduling**: probabilistic decisions
  don't compose with typestate naturally; you don't know which move will
  be made until after.
- **EDF / Rate-Monotonic**: real-time scheduling on a single processor,
  different domain entirely.

## What changes when an algorithm uses fulcrum's typestate

For algorithms in the "fits cleanly" group, four things change:

1. **The algorithm's correctness obligation is partially discharged at
   compile time.** Specifically, the Schur-convex gauge bound is structurally
   preserved across any sequence of typed-pure moves. The algorithm doesn't
   re-validate the gauge after a Pigou-Dalton transfer; the type system has
   proven it.
2. **Placement and anti-Robin-Hood moves are syntactically conspicuous.** The
   `apply_with_recheck` rename forces every catch-all site to be grep-able.
   Reviewers can audit the runtime-validated decisions in a single pass.
3. **The alphabet is closed.** New move kinds require touching the sealed
   `Primitive` trait with a written `THEOREM` citation. This makes the
   algorithm's vocabulary explicit and stable.
4. **Bug localization is structural.** A hot-spot incident traces to one of
   K named witness checks, the policy module (gauge + threshold), or the
   planner's choice logic. The first two are inside fulcrum; the third is
   in the planner. Without fulcrum, the same bug could be anywhere in the
   algorithm.

The first property is the load-bearing one. It means:
**`HotToCold` is a free operation** in the typestate sense — it never
re-evaluates the gauge, never returns a `Result`, never has an error path.
The mathematical proof that Pigou-Dalton transfers are
majorization-decreasing (Hardy-Littlewood-Pólya) is the only thing carrying
the safety claim. Algorithms that emit many migrations (max-min, DRF,
Tetris consolidation) get most of their work done with no per-step gauge
re-evaluation.

For algorithms that don't emit migrations (RR, hashing, lottery), the
typestate is dead weight and they shouldn't use fulcrum at all.

## Required extensions to fulcrum

### Phase 1 — Heterogeneous capacity

**Why**: every real fleet has machines of different sizes. Without this,
fulcrum can't ingest production data.

**What changes**:

- `Fleet` stores per-machine capacity (`BTreeMap<MachineId, MachineSpec>`
  where `MachineSpec` carries `capacity: u64`).
- `Fleet::utilization` continues to return per-machine fractions; gauge
  evaluation already operates on utilization, so the gauges
  (`SumTopK<K>`, `WeightedKyFan<N>`) need no change.
- `HotToCold::witness` and `Neutral::witness` change from comparing raw
  load to comparing utilization. The Pigou-Dalton condition becomes:

  `util(src) > util(dst)` and `mass / cap_src ≤ util(src) - util(dst)`

  (the transferred *utilization* must not exceed the gap, not the
  transferred *mass*).
- `tests/composition.rs`, `tests/totality.rs`, `tests/seeded_bugs.rs`
  updated to use heterogeneous capacities.

**Files touched**: `src/load.rs`, `src/move_kind.rs`, `src/safe.rs`, all
test files. ~300-500 lines net.

**Effort**: ~1 week of focused work.

**Acceptance**: existing tests pass under heterogeneous capacities (with
new test fixtures); new tests demonstrate that mass-equivalent moves on
machines of different sizes are correctly classified.

### Phase 2 — Multi-dimensional load

**Why**: production workloads are at minimum 2D (CPU + memory). The
algebra extends, but the witness conditions get more delicate.

**What changes**:

- New type `Resource<const N: usize>` representing an N-dimensional load
  vector (per machine: `[u64; N]`; per capacity: `[u64; N]`).
- `Fleet` parameterized by `N`: `Fleet<const N: usize>`.
- `Mass` becomes `Mass<const N: usize>([u64; N])`.
- Gauges become `Gauge<const N: usize>` — the eval signature takes a
  multi-dim fleet. `SumTopK` and `WeightedKyFan` need component-wise vs
  joint variants:
  - **Component-wise** `SumTopK<K, N>`: max-pool over dimensions, then
    Ky Fan over machines. Schur-convex per-dim, then the worst.
  - **Joint** `JointSumTopK<K, N>`: a single Schur-convex gauge over the
    joint utilization space. More delicate; uses Marshall-Olkin §15
    multi-dim majorization theory.
- Witness conditions: a `HotToCold` transfer in 2D needs to be
  majorization-decreasing in *both* dimensions simultaneously. Stricter
  than 1D — some moves that were 1D-typed-pure become catch-all under
  the joint condition. Expect the typed-pure rate on real workloads to
  drop somewhat.

**Files touched**: all of `src/`. The const generic `N` propagates
through `Fleet`, `Mass`, gauges, move kinds, witnesses, apply impls. ~800-1200 lines net.

**Effort**: ~2-3 weeks. The math (joint majorization, multi-dim
Pigou-Dalton conditions) is the hardest part; the Rust mechanics are
straightforward const-generic propagation.

**Acceptance**: new test demonstrates that a 2D Pigou-Dalton transfer is
typed-pure when the dominance holds in both dimensions and is
catch-all-classified when it doesn't. Existing 1D tests continue to
pass via `Fleet<1>`.

### Phase 3 — Planner trait + reference algorithms

**Why**: a planner is the actual scheduler. Fulcrum provides the typed
move alphabet; the planner decides which moves to emit. Without a
planner trait, every user has to reinvent the integration.

**What changes**:

- New module `src/planner.rs`:

  ```rust
  pub trait Planner<const N: usize, G: SchurConvex> {
      /// Given the current safe fleet state and a queue of work,
      /// emit a typed move (or no-op).
      fn step(&mut self, safe: &Safe<G>, work: &mut WorkQueue<N>)
          -> Option<TypedMove<N>>;
  }

  pub enum TypedMove<const N: usize> {
      Remove(Remove<N>),
      HotToCold(HotToCold<N>),
      Neutral(Neutral<N>),
      ColdToHot(ColdToHot<N>),
      Place(Place<N>),
  }
  ```

- Reference impls in `src/planner/`:
  - `LeastLoaded`: greedy min-utilization placement, no migration.
  - `PowerOfTwo`: sample 2 random sleds, pick less loaded.
  - `MaxMinFair`: iteratively emits `HotToCold` until the fleet is
    max-min fair under the chosen gauge.
  - `BestFitDecreasing`: standard bin-packing.

- The planners output typed moves; the consuming code is responsible
  for applying them (the `replay` module's pattern, generalized).

**Files touched**: new `src/planner.rs` + 3-4 small files under
`src/planner/`. ~700-1000 lines.

**Effort**: ~1-2 weeks.

**Acceptance**: each reference planner produces a non-empty typed move
sequence on a synthetic fleet, and the resulting fleet state passes the
gauge bound (verified via the typestate) at every step.

### Phase 4 — Reconciler + integration shim

**Why**: a real scheduler runs continuously. The reconciler is the loop
that calls the planner periodically and applies the moves it emits.

**What changes**:

- New module `src/reconciler.rs`:

  ```rust
  pub struct Reconciler<P: Planner<...>, G: SchurConvex> {
      planner: P,
      safe: Safe<G>,
      history: MoveHistory,
      tick_interval: Duration,
  }

  impl Reconciler {
      pub fn tick(&mut self, work: &mut WorkQueue<N>);
      pub fn run_until(&mut self, condition: impl Fn(&Safe<G>) -> bool);
  }
  ```

- A simple event-stream input format (replacing the borg-replay
  classifier with a generic event type).
- Examples that demonstrate end-to-end runs:
  - Replay the Borg trace through the JSQ planner; report the
    typed-pure rate and final gauge.
  - Same trace through the MaxMinFair planner; compare.
  - Synthetic fleet with adversarial event arrival; show that the
    typestate prevents any sequence of moves from violating the gauge.

**Files touched**: new `src/reconciler.rs`, updates to `examples/`. ~400-700 lines.

**Effort**: ~1 week.

**Acceptance**: a fulcrum-driven reconciler can be run end-to-end on
the Borg trace producing per-tick typed moves, and the gauge value is
bounded throughout.

## Verification plan

After all four phases:

**1. Replay against the Borg trace under each reference planner.** The
existing `examples/borg_replay.rs` becomes the test harness. Compare:

- Gauge trajectory over the trace.
- Typed-pure ratio.
- Number of catch-all `apply_with_recheck` invocations.

Different planners will produce different trajectories. The framework
guarantee: every planner's output sequence is gauge-bounded (the
typestate prevents otherwise). Different planners differ in *quality*
(how low they keep the gauge), not in *correctness*.

**2. Algorithm-vs-algorithm comparison.** Run identical traces through
LeastLoaded, PowerOfTwo, MaxMinFair, BestFitDecreasing. Report:

- Average gauge value
- Peak gauge value (worst hot-spot)
- Number of migrations
- Typed-pure rate per algorithm

This produces a benchmark table that can be folded into the write-up.

**3. Spectral-stability analysis (forward-looking).** If the
feedback-shaped channel/Jacobian extension is added (separate project),
the same reconciler can be instrumented with phalanx-style
spectral-stability monitoring. This is out of scope for the four phases
above but is the natural successor.

**4. Existing tests must keep passing throughout.** All ~50 tests pass
through each phase boundary, possibly with fixture updates but no logic
changes to the framework's safety guarantees.

## Critical files (existing, will be touched)

- `src/load.rs` — `Fleet` and `Mass`. Heterogeneous capacity (Phase 1)
  and multi-dim parameterization (Phase 2) both land here.
- `src/move_kind.rs` — witness construction. Phase 1 changes the
  Pigou-Dalton condition; Phase 2 propagates const generic `N`.
- `src/safe.rs` — `Safe<G>` and the apply impls. Phase 2 propagates
  `N`.
- `src/gauge.rs` — Phase 2 adds component-wise and joint gauge
  variants; Phase 1 doesn't change because gauges already operate on
  utilization.
- `src/replay.rs` — Phase 4 generalizes the classifier into a
  reconciler-friendly event stream.
- `examples/borg_replay.rs` — Phase 4 turns this into the verification
  harness.

## New files (to be created)

- `src/planner.rs` (Phase 3)
- `src/planner/least_loaded.rs`
- `src/planner/power_of_two.rs`
- `src/planner/max_min_fair.rs`
- `src/planner/best_fit_decreasing.rs`
- `src/reconciler.rs` (Phase 4)
- `examples/scheduler.rs` — end-to-end demo (Phase 4)

## Effort summary

| Phase | Effort | Net code (est.) |
| --- | --- | --- |
| 1: Heterogeneous capacity | 1 week | ~400 lines |
| 2: Multi-dim load | 2-3 weeks | ~1000 lines |
| 3: Planner trait + 4 reference algorithms | 1-2 weeks | ~800 lines |
| 4: Reconciler + integration | 1 week | ~500 lines |
| **Total** | **~6-8 weeks** | **~2700 lines added to fulcrum** |

After all four phases, fulcrum doubles in size from ~1000 lines (algebraic
core) to ~3700 lines (algebraic core + planners + reconciler), at which
point it is a *research-grade scheduler*: end-to-end runnable on real
workload data, with empirical comparison across reference algorithms,
suitable as the substrate for a method-paper write-up.

It is still **not** a production scheduler — that requires concurrency,
persistence, observability, operator tooling, constraint plugins, failure
recovery, and platform integration, none of which are in this plan and
which collectively would be 10-100× the work above. The plan above is the
research-grade artifact; the production version is a downstream decision
contingent on a customer who actually wants to deploy.

## Success criteria

The plan is successful if all of the following hold at the end:

1. **All four phases complete with their acceptance criteria met.**
2. **The Borg trace replay produces a `gauge value over time` plot
   for at least three reference planners.** This is the headline result of
   the extension work and the strongest evidence that fulcrum is usable
   for real workload scheduling.
3. **The typed-pure rate measured under each planner is reported
   honestly.** Production planners (BestFitDecreasing, filter-and-score)
   will likely have lower typed-pure rates than max-min-fair because
   they're placement-heavy. That's fine; the rate is a property of the
   planner, not of the framework.
4. **The success criteria from PLAN.md continue to hold.** The framework's
   per-move correctness, alphabet smallness, and discipline matrix are not
   regressed by the extensions. Specifically: the alphabet stays at 5
   primitives, basis.yaml's enforcement still passes, and adding more
   gauges remains a single-file change.

## Out of scope (explicitly)

- Production scheduling (concurrency, persistence, observability, etc.)
- Request-routing-style load balancing (different problem shape)
- Constraint plugins (affinity, anti-affinity, fault domains, taints)
- Multi-tenancy and fairness across tenants
- Workload class differentiation (services vs. batch vs. system jobs)
- Spectral-stability monitoring (phalanx-shaped extension; separate
  project, plausibly the natural successor to this plan)
- Integration with any specific platform (Kubernetes, Nomad, Omicron,
  etc.) — that's a deployment decision tied to a specific customer

## Why this is the right scope

The plan above produces an artifact that is:

- **Useful for evaluation by skeptical readers**: the framework is run
  end-to-end on real workload data through reference algorithms, with
  measurable comparisons.
- **Sized to the empirical claim**: the framework's value depends on
  real workloads exercising the algebra, not on hypothetical use. The
  extensions make that exercise possible.
- **A natural setup for the method paper**: the four phases produce a
  paper-shaped structure (problem, design, implementation, evaluation),
  with the empirical evaluation across multiple reference algorithms
  being the strongest novel contribution after the typed-move algebra
  itself.
- **Non-committal about deployment**: each phase is independently
  meaningful. If interest in deploying drops at any point, the work to
  date stands as a contribution.
- **Compounds with the rest of the project stack**: the reconciler
  produced in Phase 4 is the natural integration point for
  feedback/phalanx-shaped spectral-stability monitoring, when/if that
  layer is added later.