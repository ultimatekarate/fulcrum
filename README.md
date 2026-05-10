# Fulcrum

A typed move algebra for load-balancing invariants under Schur-convex gauges.

## What this is

Fulcrum lifts the algebra of majorization into Rust's type system. Operations
that preserve a Schur-convex gauge — mass removal, Pigou-Dalton transfers,
neutral migrations — are typed as **total** functions over a `Safe<G>`
typestate. Operations that *can't* preserve the bound unconditionally —
placement, anti-Robin-Hood migrations — are typed as **fallible** operations,
where the runtime check is unavoidable and visible.

```rust
let safe: Safe<Linfty> = Safe::new(fleet, 0.85)?;

// Pigou-Dalton transfer — total apply, no Result.
let m = HotToCold::witness(src, dst, mass, safe.fleet())?;
let safe = m.apply(safe);

// Mass removal — total apply, no Result.
let safe = Remove::new(machine, mass).apply(safe);

// Fresh placement — fallible apply, deliberately named to be grep-able.
// Any site that performs runtime gauge re-evaluation contains the literal
// substring `apply_with_recheck`. `rg apply_with_recheck` enumerates them.
let safe = Place::new(machine, mass).apply_with_recheck(safe)?;
```

The composition guarantee is structural: any sequence of `Remove`, `HotToCold`,
and `Neutral` moves typechecks with no per-step gauge re-evaluation, because
each individual `apply` is total. The proof obligation lives at witness
construction — once, fallible, named — not at every connection point.

## Why bother

Two prior questions had to land favorably before this was worth coding:

- **Math**: Schur-convexity, symmetric gauge functions, and Pigou-Dalton
  transfers are standard mathematical inequalities going back a century
  (Hardy-Littlewood-Pólya 1929; Marshall-Olkin-Arnold for the canonical
  treatment). The framework reflects pre-existing structure, not invented
  abstractions.

- **Data**: a 405K-event subset of the Google 2019 Borg cluster trace
  decomposes into the proposed move alphabet at ~81.5% typed-pure. The
  catch-all volume is ~17% fresh placements and ~1% anti-Robin-Hood
  migrations. The algebra captures the dominant operations.

See [PLAN.md](PLAN.md) for the full background, scope, and kill criteria.

## Status

Pre-1.0. Single-dimensional load with **heterogeneous per-machine capacity**
(Phase 1 of the load-balancing extension). The gauge family is the
**Ky Fan k-norm** `SumTopK<K>` plus its non-negative linear span
`WeightedKyFan<N>`, with `Linfty = SumTopK<1>` exposed as a type alias. The
seal on `SchurConvex` is mathematically precise: by Ky Fan dominance, the
family `{‖·‖_(k)}` generates the majorization order, and non-negative
combinations are themselves Schur-convex — so the framework covers the
full non-negative-combinations cone. Five move kinds: `Remove`, `HotToCold`,
`Neutral`, `ColdToHot`, `Place`.

Under heterogeneous capacity the witness conditions tighten: `HotToCold`
requires `cap(src) ≤ cap(dst)` and `mass · cap(src) ≤ load(src) · cap(dst)
− load(dst) · cap(src)` (the destination-side utilization gap), which
collapses to the v0 rule `mass ≤ load(src) − load(dst)` when capacities
match. `Neutral` requires equal capacity AND equal load. Multi-dimensional
load is Phase 2.

The Borg replay example reads a 405K-event subset of the Google 2019
cluster trace through the typed framework and reports counts by move
kind. Of the 13,197 migrations in the subset, **99.98% classify as
HotToCold** (typed-pure Pigou-Dalton transfers). Of all 68,997 events
the framework processes, 19.12% are typed-pure overall — the rest are
fresh placements, which are inherently catch-all under any
mass-conserving algebra. See `PLAN.md` for the full analysis, including
the matched-subset comparison against the duckdb prototype and an honest
note about what the trace subset doesn't validate (no complete instance
lifecycles, so `Remove::apply` was not exercised on real trace data).

Compile-fail tests (in `tests/ui/`) demonstrate that forging a witness
without going through `HotToCold::witness` or `Neutral::witness` is
rejected by the type system — bad code does not compile. A regression
test (`tests/replay.rs`) runs the classifier against a small fixed CSV
with hand-traced expected counters; any drift in parse or classify
behavior is caught by `cargo test`.

## Try it

```bash
cargo test
cargo run --example synthetic
cargo run --release --example borg_replay -- /path/to/borg_traces_data.csv
```

## Layout

```
src/
├── lib.rs        public API
├── load.rs       Fleet, MachineId, Mass
├── gauge.rs      Gauge trait, sealed SchurConvex, Linfty
├── move_kind.rs  Move kinds + witness construction
├── safe.rs       Safe<G> typestate + apply impls
└── trace.rs      MoveHistory for replay/debugging

examples/
├── synthetic.rs  toy rebalancer demo
└── borg_replay.rs  trace classifier (stub)

tests/
├── composition.rs   typed-pure chain tests
├── totality.rs      total-apply assertions
├── seeded_bugs.rs   bug battery demonstrating localization
├── replay.rs        end-to-end CSV → classifier regression tests
├── ui.rs            trybuild compile-fail registry
├── ui/              compile-fail snippets (forged witnesses)
└── data/            small fixed CSVs for regression tests
```

## License

MIT OR Apache-2.0.
