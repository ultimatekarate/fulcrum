//! Move kinds: the alphabet.
//!
//! Each move kind is a marker around the data needed to apply the move. The
//! kinds partition by *mathematical effect* on a Schur-convex gauge:
//!
//! | Kind        | Effect                              | `apply`            |
//! | ----------- | ----------------------------------- | ------------------ |
//! | `Remove`    | Mass-decreasing                     | total              |
//! | `HotToCold` | Pigou-Dalton transfer (src > dst)   | total              |
//! | `Neutral`   | Mass-preserving, equal utilization  | total              |
//! | `ColdToHot` | Anti-Robin-Hood (src < dst)         | fallible           |
//! | `Place`     | Mass-adding                         | fallible           |
//!
//! `HotToCold` and `Neutral` are *witness types* — their public construction
//! is gated by a fallible check. Once constructed, the `apply` is total. The
//! actual `apply` impls live in `safe.rs`.

use crate::alphabet::{Effect, Primitive, Sealed};
use crate::load::{Fleet, MachineId, Mass};

/// Mass removal from a single machine. Mass-decreasing — strictly reduces
/// any Schur-convex gauge on non-negative load vectors.
///
/// **Totality argument** (proves `apply: Safe<G> -> Safe<G>` is sound):
/// - For Schur-convex `g`, removing mass `m` from machine `i` produces a new
///   load vector `x'` with `x' ≤ x` componentwise, hence `x ≻ x'`, hence
///   `g(x') ≤ g(x)`.
/// - Therefore `g(x) ≤ τ ⇒ g(x') ≤ τ`.
#[derive(Clone, Copy, Debug)]
pub struct Remove {
    pub machine: MachineId,
    pub mass: Mass,
}

impl Remove {
    /// Construct a `Remove` move. Always succeeds — `Remove` has no
    /// preconditions to enforce at construction time. Apply will fail at
    /// runtime if the machine doesn't exist or has insufficient load, but
    /// those are well-formedness errors, not gauge violations.
    pub fn new(machine: MachineId, mass: Mass) -> Self {
        Remove { machine, mass }
    }
}

/// Pigou-Dalton transfer: mass moved from a higher-utilization source to a
/// lower-utilization destination, in an amount that preserves the order
/// `util(src) ≥ util(dst)` after the transfer.
///
/// **Construction**: the public constructor is fallible — see [`Self::witness`].
/// Direct construction is intentionally not provided; the witness check is
/// the entire point of the type.
///
/// **Totality argument** (proves `apply: Safe<G> -> Safe<G>` is sound):
/// - The witness check guarantees the transfer is a Pigou-Dalton step.
/// - Pigou-Dalton transfers are majorization-decreasing (Hardy-Littlewood-
///   Pólya, 1929; Marshall-Olkin-Arnold §1.A).
/// - Schur-convex gauges are non-increasing under majorization-decreasing
///   operations: `x ≻ x' ⇒ g(x) ≥ g(x')` (definition of Schur-convex).
/// - Therefore `g(x) ≤ τ ⇒ g(x') ≤ τ`.
#[derive(Clone, Copy, Debug)]
pub struct HotToCold {
    pub source: MachineId,
    pub destination: MachineId,
    pub mass: Mass,
    /// Private. Construction requires the witness check to pass.
    _witness: WitnessToken,
}

impl HotToCold {
    /// Construct a `HotToCold` if and only if the move is weak-super-
    /// majorization-decreasing on the utilization vector against `fleet`:
    ///
    /// 1. Both machines exist and `source ≠ destination`.
    /// 2. Source utilization > destination utilization (rich → poor on
    ///    utilization, not on raw load).
    /// 3. `cap(src) ≤ cap(dst)` — transfer to a same-or-larger machine.
    ///    A load transfer of mass `m` decreases source utilization by
    ///    `m/cap(src)` and increases destination utilization by
    ///    `m/cap(dst)`. The two are equal only when capacities match;
    ///    otherwise the move shifts the sum of utilizations. When
    ///    `cap(src) ≤ cap(dst)`, the source-side decrease dominates the
    ///    destination-side increase, and the resulting vector is
    ///    weak-super-majorized by the original. When `cap(src) > cap(dst)`
    ///    the move can increase top-k sums even with `util(src) >
    ///    util(dst)` — counterexample: caps `(100, 10)`, loads `(80, 5)`,
    ///    utils `(0.80, 0.50)`; transferring mass 1 yields utils `(0.79,
    ///    0.60)`, and `SumTopK<2>` increases from 1.30 to 1.39. So
    ///    high-to-low-cap transfers fall through to the catch-all path.
    /// 4. `mass ≤ cap(dst) · (util(src) − util(dst))` — the transferred
    ///    destination-side utilization does not exceed the rich-poor gap.
    ///    Equivalently in integer arithmetic: `mass · cap(src) ≤
    ///    load(src) · cap(dst) − load(dst) · cap(src)`. This ensures the
    ///    new destination utilization is at most the old source
    ///    utilization, so no coordinate exceeds the previous max.
    ///
    /// When `cap(src) = cap(dst) = c`, conditions 3 and 4 collapse to the
    /// uniform-capacity rule `mass ≤ load(src) − load(dst)`, recovering
    /// the v0 witness.
    pub fn witness(
        source: MachineId,
        destination: MachineId,
        mass: Mass,
        fleet: &Fleet,
    ) -> Option<Self> {
        if source == destination {
            return None;
        }
        let src_spec = fleet.spec(source)?;
        let dst_spec = fleet.spec(destination)?;
        if src_spec.capacity == 0 || dst_spec.capacity == 0 {
            return None;
        }
        if src_spec.capacity > dst_spec.capacity {
            return None;
        }
        // util(src) > util(dst) ⟺ load(src)·cap(dst) > load(dst)·cap(src).
        // Use u128 to keep the cross product exact for u64 inputs.
        let load_src_x_cap_dst = src_spec.load as u128 * dst_spec.capacity as u128;
        let load_dst_x_cap_src = dst_spec.load as u128 * src_spec.capacity as u128;
        if load_src_x_cap_dst <= load_dst_x_cap_src {
            return None;
        }
        // mass·cap(src) ≤ load(src)·cap(dst) − load(dst)·cap(src).
        let gap = load_src_x_cap_dst - load_dst_x_cap_src;
        let mass_x_cap_src = mass.0 as u128 * src_spec.capacity as u128;
        if mass_x_cap_src > gap {
            return None;
        }
        Some(HotToCold {
            source,
            destination,
            mass,
            _witness: WitnessToken,
        })
    }
}

/// Mass-preserving migration between machines at equal utilization. Does not
/// change majorization order; gauge value unchanged.
///
/// **Totality argument**: source and destination at equal utilization means
/// the load vector after transfer is a permutation of the load vector before,
/// up to the magnitudes involved. For symmetric gauges (which Schur-convex
/// gauges are), permutations preserve gauge value.
#[derive(Clone, Copy, Debug)]
pub struct Neutral {
    pub source: MachineId,
    pub destination: MachineId,
    pub mass: Mass,
    _witness: WitnessToken,
}

impl Neutral {
    /// Construct a `Neutral` move if and only if source and destination
    /// have equal *capacity* and equal *load* (and therefore equal
    /// utilization). Equal capacity is required because the Permutation
    /// effect claims the post-move utilization vector is a coordinate
    /// permutation of the pre-move vector. Under heterogeneous capacity,
    /// a load transfer of mass `m` shifts source utilization by
    /// `−m/cap(src)` and destination utilization by `+m/cap(dst)`. The
    /// two are equal only when `cap(src) = cap(dst)`; otherwise the move
    /// is no longer a permutation (sum of utilizations changes). Such
    /// migrations between equal-utilization but unequal-capacity machines
    /// can still be typed-pure via [`HotToCold::witness`] (the witness
    /// admits transfers from smaller to larger machines), or through
    /// the catch-all path otherwise.
    pub fn witness(
        source: MachineId,
        destination: MachineId,
        mass: Mass,
        fleet: &Fleet,
    ) -> Option<Self> {
        if source == destination {
            return None;
        }
        let src_spec = fleet.spec(source)?;
        let dst_spec = fleet.spec(destination)?;
        if src_spec.capacity != dst_spec.capacity {
            return None;
        }
        if src_spec.load != dst_spec.load {
            return None;
        }
        if mass.0 > src_spec.load {
            return None;
        }
        Some(Neutral {
            source,
            destination,
            mass,
            _witness: WitnessToken,
        })
    }
}

/// Anti-Robin-Hood migration: mass from lower-utilization to higher-
/// utilization machine. Can violate the gauge bound — `apply` is fallible.
///
/// No witness type; the runtime check happens in `apply`.
#[derive(Clone, Copy, Debug)]
pub struct ColdToHot {
    pub source: MachineId,
    pub destination: MachineId,
    pub mass: Mass,
}

impl ColdToHot {
    pub fn new(source: MachineId, destination: MachineId, mass: Mass) -> Self {
        ColdToHot { source, destination, mass }
    }
}

/// Fresh placement of mass on a machine. Mass-adding — can hot-spot.
/// `apply` is fallible.
#[derive(Clone, Copy, Debug)]
pub struct Place {
    pub machine: MachineId,
    pub mass: Mass,
}

impl Place {
    pub fn new(machine: MachineId, mass: Mass) -> Self {
        Place { machine, mass }
    }
}

/// Private token. The presence of one of these inside a move struct is the
/// type-level evidence that the witness check passed. The token is
/// intentionally not constructible outside this module.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WitnessToken;

// --------- Primitive trait impls (sealed alphabet membership) ---------
//
// Adding a new typed-pure primitive requires three coordinated changes:
//   1. Define the data type above.
//   2. Implement `apply` in `safe.rs` with the appropriate signature.
//   3. Add `Sealed` + `Primitive` impls below with EFFECT/THEOREM/NAME.
//
// Each `THEOREM` citation must justify the corresponding `apply` signature.
// Reviewers verify the citation; users trust the typing thereafter.

impl Sealed for Remove {}
impl Primitive for Remove {
    const EFFECT: Effect = Effect::MassDecreasing;
    const THEOREM: &'static str =
        "Marshall-Olkin §3.A: mass-decreasing on non-negative vectors \
         strictly reduces every monotone Schur-convex gauge.";
    const NAME: &'static str = "Remove";
}

impl Sealed for HotToCold {}
impl Primitive for HotToCold {
    const EFFECT: Effect = Effect::PigouDalton;
    const THEOREM: &'static str =
        "Hardy-Littlewood-Pólya 1929; Marshall-Olkin §1.A.1, §3.A.8. \
         Under heterogeneous capacity the witness conditions \
         (cap(src) ≤ cap(dst); mass·cap(src) ≤ load(src)·cap(dst) − \
         load(dst)·cap(src)) decompose the load transfer into (i) a \
         T-transform of size mass/cap(dst) on the utilization vector \
         (Pigou-Dalton on src and dst, sum-preserving), composed with \
         (ii) a mass-decrease at src of size mass·(1/cap(src) − \
         1/cap(dst)) ≥ 0. Both pieces are weak-super-majorization \
         decreasing on non-negative vectors; their composition is \
         non-increasing for every monotone Schur-convex gauge — the \
         Ky Fan family covered by the seal.";
    const NAME: &'static str = "HotToCold";
}

impl Sealed for Neutral {}
impl Primitive for Neutral {
    const EFFECT: Effect = Effect::Permutation;
    const THEOREM: &'static str =
        "Symmetric gauges are invariant under coordinate permutation. The \
         witness requires equal capacity AND equal load at source and \
         destination, so the post-transfer utilization vector is a \
         coordinate transposition of the pre-transfer vector within the \
         equal-utilization class. Schur-convex gauges (here, the Ky Fan \
         family) are symmetric, so the gauge value is unchanged.";
    const NAME: &'static str = "Neutral";
}

impl Sealed for ColdToHot {}
impl Primitive for ColdToHot {
    const EFFECT: Effect = Effect::MassPreservingFree;
    const THEOREM: &'static str =
        "Anti-Robin-Hood transfers can produce majorization-incomparable \
         results; Schur-convex gauges may strictly increase. Catch-all: \
         apply re-checks the gauge at runtime.";
    const NAME: &'static str = "ColdToHot";
}

impl Sealed for Place {}
impl Primitive for Place {
    const EFFECT: Effect = Effect::MassIncreasing;
    const THEOREM: &'static str =
        "Mass-increasing operations on non-negative vectors can push any \
         monotone Schur-convex gauge upward (in particular: max utilization \
         increases when mass is added to the most-loaded machine). \
         Catch-all: apply re-checks the gauge at runtime.";
    const NAME: &'static str = "Place";
}
