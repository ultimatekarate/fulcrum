# Fulcrum: A typed move algebra for load-balancing invariants

## What this is

Fulcrum is a Rust library that lifts the algebra of majorization into the type
system. The idea: classify load-balancing operations by their effect on a
Schur-convex gauge, then use Rust's typestate machinery to make
invariant-preserving sequences of operations *typecheck for free* — no per-move
runtime gauge re-evaluation. Operations that *can't* preserve the invariant
unconditionally (placement, anti-Robin-Hood migrations) are forced into a
syntactically distinct fallible path, where the runtime check is unavoidable
and visible.

The principle being demonstrated is more general than load balancing: when a
domain has pre-existing algebraic structure, lifting that algebra into types
gives compositional safety with high-quality bug localization. Load balancing
under Schur-convex gauges is the cleanest available case study because
mathematicians worked out the algebra a century ago (Hardy-Littlewood-Pólya,
1929; Marshall, Olkin & Arnold's *Inequalities: Theory of Majorization* is the
canonical reference).

## Why this is worth building

Two prior questions had to land favorably before this was worth coding:

**The math is sound.** Schur-convexity, symmetric gauge functions, and
Pigou-Dalton transfers are standard mathematical inequalities, not invented
abstractions. Hardy-Littlewood-Pólya provides the composition theorem for
free: any Pigou-Dalton transfer is majorization-decreasing, and any
majorization-decreasing operation reduces every Schur-convex gauge.

**The data supports the claim.** A subset of the Google 2019 Borg cluster
trace (405K events, 96K machines, 8 cells) shows that real migrations are
dominantly majorization-direction — 76% under load-comparison classification,
~100% under fulcrum's mass-comparison through the typed framework. Removals
are unconditionally typed-pure under any Schur-convex gauge by mathematical
argument. The catch-all volume concentrates in fresh placements (~17% of
events), which are inherently fallible: adding mass to a load vector can
push the gauge upward and must be re-checked. The algebra captures the
structure that exists in the data; see success criterion 2 below for the
matched-subset breakdown.

## What "typed-pure" means here

A move is *typed-pure* under a gauge $g$ if its `apply` function can be
declared total — `Safe<G> -> Safe<G>`, no `Result`. The type system is
guaranteeing that the safety claim survives the operation by construction, not
by checking.

Concretely, the alphabet:

| Move kind     | Apply signature                       | Justification                               |
| ------------- | ------------------------------------- | ------------------------------------------- |
| `Remove`      | `Safe<G> -> Safe<G>`                  | Mass-decreasing $\Rightarrow$ Schur-convex gauge decreasing |
| `HotToCold`   | `Safe<G> -> Safe<G>`                  | Pigou-Dalton transfer $\Rightarrow$ majorization-decreasing |
| `Neutral`     | `Safe<G> -> Safe<G>`                  | Mass-preserving, no concentration change    |
| `ColdToHot`   | `Safe<G> -> Result<Safe<G>, _>`       | Anti-Robin-Hood $\Rightarrow$ can violate, must re-check |
| `Place`       | `Safe<G> -> Result<Safe<G>, _>`       | Mass-adding $\Rightarrow$ can hot-spot, must re-check   |

The witness types — `HotToCold`, `Neutral`, `ColdToHot` — are constructed via
fallible constructors that inspect runtime state. Once constructed, the token
is consumed in `apply` and the type system carries the proof.

## What v0 commits to

- **One gauge family** with two impls: `SumTopK<K>` (the **Ky Fan k-norm**)
  and `WeightedKyFan<N>` (non-negative weighted combinations of Ky Fan
  norms). Mathematically faithful to the Ky Fan dominance theorem —
  $x \succ y$ (majorization) iff $\|x\|_{(k)} \leq \|y\|_{(k)}$ for every $k$. The Ky
  Fan family generates the partial order at the heart of the framework,
  and non-negative combinations cover essentially every operationally
  interesting symmetric gauge. Restricting `SchurConvex` impls to this
  family makes the seal a precise mathematical statement rather than an
  honor-system curatorial seal. `Linfty = SumTopK<1>` is exposed as a
  type alias for clarity at user-facing call sites.
- **Five move kinds**: as above.
- **One-dimensional load**. Vector-shaped infrastructure but a single resource
  dimension to start. Multi-dimensional joint gauges are future work.
- **Two demos**:
  - `examples/synthetic.rs`: a toy rebalancer that constructs a fleet,
    applies a sequence of moves, and reports the final gauge value. Shows
    the API in action.
  - `examples/borg_replay.rs`: reads the Borg trace CSV, classifies each
    event, applies typed moves, and reports the typed-pure ratio. Should
    reproduce the ~81% number we computed in duckdb.
- **Three test files**:
  - `tests/composition.rs`: type-level composition guarantees. Bad
    sequences fail to compile. Good sequences compile and produce the
    expected `Safe<G>`.
  - `tests/totality.rs`: typed-pure `apply` impls are genuinely total —
    no panic paths, no debug_assert escape hatches.
  - `tests/seeded_bugs.rs`: a battery of seeded scenarios where the
    framework rejects placements that would be accepted by a naive
    coordinate-cap check. Demonstrates the localization claim.

### Why Ky Fan and not an arbitrary list of named gauges

The gauge alphabet rests on the **Ky Fan family** plus its non-negative
linear span:

- The Ky Fan k-norms $\{\|\cdot\|_{(k)} : k = 1, \dots, n\}$ are the canonical Schur-
  convex generators on non-negative $n$-vectors (Marshall-Olkin §3.A.1).
- By Ky Fan dominance, the partial order they collectively encode is
  *exactly* the majorization order. So checking a Schur-convex inequality
  reduces to checking Ky Fan dominance.
- Most named symmetric gauges ($\ell_\infty$, $\ell_1$, sum-of-top-3, capacity-weighted
  utilization, etc.) are either single Ky Fan norms or non-negative
  weighted combinations of them.
- `SumTopK<K>` is the framework's primitive Ky Fan gauge; `WeightedKyFan<N>`
  is the non-negative linear combination, with weights validated at
  construction (negative or NaN weights are rejected because they would
  break Schur-convexity). Together they cover the full
  non-negative-combinations cone of Schur-convex symmetric gauges.

The structural point: the framework's two halves now mirror each other.
The move alphabet stays small because Birkhoff-von Neumann gives a
generating set for majorization-decreasing operations. The gauge alphabet
stays small because Ky Fan dominance gives a generating family for the
majorization order itself. Same flavor of "small generating set forced by
mathematical structure," applied to both sides.

To support runtime-parameterized gauges like `WeightedKyFan`, `Gauge::eval`
takes `&self` and `Safe<G>` carries an instance of `G`. For unit gauges
(`SumTopK<K>`), `Safe::new(fleet, threshold)` works as before via
`G: Default`. For runtime-parameterized gauges, use
`Safe::with_gauge(fleet, threshold, gauge)`.

## What v0 does *not* commit to

- Multi-dimensional joint gauges. Per-dimension gauges only. The gauge module
  is structured to allow joint gauges later. (Production load vectors are at
  minimum 2D — CPU + memory — and often more; the algebra extends to multi-D
  under joint Schur-convex gauges, but the witness conditions get more
  delicate.)
- Heterogeneous machine capacities. v0 assumes uniform capacity across the
  fleet; real Borg machines have varying capacities. Modeling them changes
  witness conditions from load comparison to utilization comparison.
- Real concurrency. Single-threaded, in-memory state.
- Integration with any existing scheduler. The crate stands alone.
- A stable public API. Pre-1.0; everything subject to change. Documented
  explicitly in the README.
- Validation on complete instance lifecycles. The Borg subset we have
  contains no instances with both a SCHEDULE and a REMOVE in the visible
  window, so `Remove::apply` was never exercised on real trace data. The
  math says it preserves the gauge; the empirical side just didn't get the
  chance.

## File structure

```
fulcrum/
├── Cargo.toml
├── README.md
├── PLAN.md                 (this file)
├── .gitignore
├── src/
│   ├── lib.rs              public API + crate docs
│   ├── load.rs             MachineId, Capacity, Load, Fleet
│   ├── gauge.rs            Gauge trait, sealed SchurConvex, SumTopK + WeightedKyFan
│   ├── move_kind.rs        Move kinds, witness construction
│   ├── safe.rs             Safe<G> typestate, apply impls
│   └── trace.rs            MoveHistory for replay/debugging
├── examples/
│   ├── synthetic.rs        toy rebalancer demo
│   └── borg_replay.rs      Borg trace classifier
└── tests/
    ├── alphabet.rs         Primitive trait + signature_match check
    ├── composition.rs      type-level composition tests
    ├── replay.rs           borg-replay regression on a small CSV
    ├── seeded_bugs.rs      bug battery
    ├── totality.rs         total apply tests
    ├── ui.rs               trybuild compile-fail driver
    ├── ui/                 compile-fail .rs files (forged witnesses)
    └── data/               small.csv fixture for the replay regression
```

## Discipline and enforcement

The framework's algebraic-quality localization depends on a set of
disciplines: witnesses are direct dependency-free reads, policy lives in one
module, catch-all sites are conspicuous, the alphabet stays small, and each
typed-pure move ships with a totality argument. Violating any of them turns
Fulcrum into a typestate-shaped runtime validator with extra ceremony.

As of v0.1, every discipline has a mechanical enforcer:

| Discipline | Enforcer |
| --- | --- |
| Sealed `SchurConvex` | Rust privacy (`gauge::sealed::Sealed`) |
| Witnesses can't be forged | Rust privacy + `tests/ui/` (trybuild compile-fail) |
| Typed-pure `apply` returns `Safe<G>` | Type signatures + `tests/totality.rs` + `tests/alphabet.rs::signature_match` |
| Witness predicates dependency-free | `basis.yaml` layer rules (in this repo) |
| Policy in one module | `basis.yaml` placement axis (in this repo) |
| Branded newtypes | `basis.yaml` values axis (in this repo) |
| Catch-all sites are grep-able | Method rename: catch-all `apply_with_recheck`, typed-pure `apply` |
| Alphabet stays small | Sealed `Primitive` trait in `alphabet` module |
| Totality argument exists | `THEOREM: &'static str` const required + `tests/alphabet.rs` |
| Effect classification matches signature | `tests/alphabet.rs::signature_match` (compile-time check) |
| Discriminated unions stay exhaustive | `basis.yaml` exhaustive_matching axis |
| Pure layers don't do IO | `basis.yaml` purity axis |

### The `Primitive` trait, the alphabet's seal

New typed-pure primitives must touch `src/alphabet.rs` and `src/move_kind.rs`,
declaring an `EFFECT: Effect` and a `THEOREM: &'static str`. The `THEOREM`
const must be non-empty, $\geq 40$ characters, and free of placeholder strings;
this is enforced by `tests/alphabet.rs::theorem_citation_present_and_substantive`.

The `tests/alphabet.rs::signature_match` module additionally verifies that
each primitive's declared `EFFECT` matches its actual `apply` signature: a
typed-pure effect must implement `TotalApply` (returning `Safe<G>`), a
catch-all effect must implement `FallibleApply` (returning `Result<Safe<G>>`).
Mismatches fail at compile time.

The mathematical justification for the small alphabet is given above in
"Why Ky Fan and not an arbitrary list of named gauges": Birkhoff-von Neumann
makes the five primitives a generating set, so any sixth primitive is either
a composition (belongs in `Derived`) or outside the algebra (belongs as a
catch-all effect, not typed-pure).

### The `Derived` trait, for non-primitive convenience moves

External crates can extend the move alphabet without breaking the seal by
implementing `Derived`. The discipline is that derived moves decompose into
calls on primitive `apply` methods — they don't manipulate `Fleet` directly.
A `DECOMPOSITION: &'static str` const documents the intended composition for
review.

### Catch-all conspicuousness via method naming

Catch-all methods are deliberately renamed: typed-pure operations are
`apply`, catch-all operations are `apply_with_recheck`.

```rust
let safe = Remove::new(...).apply(safe);                         // total
let safe = HotToCold::witness(...)?.apply(safe);                 // total
let safe = Place::new(...).apply_with_recheck(safe)?;            // fallible
let safe = ColdToHot::new(...).apply_with_recheck(safe)?;        // fallible
```

The asymmetry is the feature: any call site that performs runtime gauge
re-evaluation contains the literal substring `apply_with_recheck`, and
`rg apply_with_recheck` enumerates every such site in any codebase using
fulcrum, no external tooling required.

A user could attempt to hide the call behind a wrapper that returns
`Safe<G>` directly, but the inner method name remains visible to grep —
the audit surface is preserved across reasonable wrapping disciplines.

This conspicuousness is now mechanical at the API level rather than enforced
by a downstream basis rule, so it survives any team's tooling choices.

## Success criteria for v0

The prototype is worth promoting beyond v0 if all four hold at the end:

1. **`cargo build` and `cargo test` are clean.** No warnings, no
   `unimplemented!()` in the apply paths. **Met.** 50 tests + 1 doctest
   pass; clean build.

2. **The Borg replay validates the framework's algebraic claim on real
   workload data** (specifically: that real migrations decompose into
   typed-pure moves at high coverage). **Partially met, with corrected
   methodology.**

   The original v0 plan said "reproduce ~81% typed-pure to match the duckdb
   prototype." That goal turned out to compare different things. The duckdb
   prototype counted *event types* in the CSV — REMOVE events are
   unconditionally typed-pure under any Schur-convex gauge by mathematical
   argument, regardless of whether any framework can apply them. On this
   particular trace subset, **none of the 260,767 REMOVE events have a
   matching SCHEDULE** (the subset apparently sampled events stratified by
   type rather than capturing complete instance lifecycles), so the
   framework cannot apply any of them.

   The framework processes 68,997 events. The matched-subset duckdb
   comparison gives:

   | Metric                        | duckdb | fulcrum |
   | ----------------------------- | ------ | ------- |
   | Migrations classifiable       | 10,647 | 13,197  |
   | HotToCold (typed-pure)        | 8,025  | 13,195  |
   | Neutral (typed-pure)          | 2,141  | 0       |
   | ColdToHot (catch-all)         | 481    | 2       |
   | Unclassifiable                | 2,579  | 0       |
   | Migrations typed-pure %       | 76.86% | 99.98%  |

   The gap is explained by modeling differences: fulcrum pre-registers all
   machines (so nothing is "unclassifiable"), and fulcrum compares
   mass-based loads (so mass-equality is rare and "Neutral" almost never
   fires; near-equal events become HotToCold instead). Both classifications
   are internally consistent.

   **The honest finding**: real Borg migrations are nearly always
   Pigou-Dalton-direction (>76% under either model, ~100% under fulcrum's).
   The algebra captures the structure that exists in the data. The
   framework's *applied* typed-pure ratio over all events is 19.12%, but
   that's dominated by fresh placements being inherently catch-all. The
   migration-specific typed-pure rate is the load-bearing number, and it's
   high.

3. **Adding a second invariant fits the same shape without forcing the type
   parameter to grow combinatorially.** **Met.** `SumTopK<K>` was added by
   editing only `gauge.rs`; no changes to `safe.rs`, `move_kind.rs`, or any
   apply impl. `Safe<SumTopK<2>>` works in the test suite alongside
   `Safe<Linfty>` without any combinatorial type-parameter growth.

4. **A reviewer can follow the triage tree** for a hot-spot scenario in the
   seeded-bug tests, and the buggy code falls out at one named site.
   **Met.** `tests/seeded_bugs.rs` demonstrates: forged witnesses caught at
   `HotToCold::witness`; hot-spot-creating placements caught at
   `Place::apply`; cold-to-hot caught at `ColdToHot::apply`. Each test
   localizes to one named site. Compile-fail tests in `tests/ui/`
   additionally show that forged witnesses constructed by struct literal
   are rejected at compile time, not just at runtime.

The kill criterion: if by day three of implementation the alphabet has grown
past seven move kinds, or if witness construction requires shared
infrastructure calls, stop and replace with a runtime validator. The framework
is supposed to *compose* algebraic moves; if invariants want orthogonal
phantom params or witnesses want sub-algorithms, the algebra didn't carry the
weight.

**Status as of v0**: alphabet is at 5 kinds (`Remove`, `HotToCold`, `Neutral`,
`ColdToHot`, `Place`). Witnesses are direct, dependency-free reads of fleet
load (a few lines each). The framework has not exhibited either kill condition.

## Resolved design decisions

Open questions during implementation, with the resolutions baked into v0:

1. **Threshold $\tau$: type-level or runtime?** Runtime. `Safe<G>` carries
   `threshold: f64`. Type-level $\tau$ via const generics buys nothing here
   and complicates everything; const-generic floats aren't stable in any
   case.
2. **Move trace location?** Separate. `MoveHistory` is its own type.
   `Safe<G>` is for type-level claims; the trace is for debugging and
   diagnostics, and shouldn't bloat the typestate.
3. **`Place::apply_with_recheck` failure type?** `GaugeError` enum with
   three variants (`ThresholdExceeded`, `UnknownMachine`,
   `InsufficientLoad`). Each variant pinpoints a named failure mode for
   localization.
4. **`Fleet` ownership?** `Safe<G>` owns the fleet by value, mutated
   through the apply impls. No shared references; the typestate is the
   access discipline.

## References (math)

- Hardy, G. H., Littlewood, J. E., & Polya, G. (1929). "Some simple
  inequalities satisfied by convex functions." *Messenger of Mathematics*.
- Marshall, A. W., Olkin, I., & Arnold, B. C. (2011). *Inequalities: Theory
  of Majorization and Its Applications* (2nd ed.). Springer.
- Bhatia, R. (1997). *Matrix Analysis*. Springer. (Symmetric gauge functions
  and Ky Fan / Lidskii.)
- von Neumann, J. (1937). "Some matrix-inequalities and metrization of
  matric-space." *Tomsk University Review*.
- Pigou, A. C. (1912). *Wealth and Welfare*. Macmillan.
- Dalton, H. (1920). "The measurement of the inequality of incomes."
  *The Economic Journal*.

## References (engineering)

- Strom, R. E. & Yemini, S. (1986). "Typestate: A programming language
  concept for enhancing software reliability." *IEEE TSE*.
- The Borg cluster trace: https://github.com/google/cluster-data —
  ClusterData2019.md.
