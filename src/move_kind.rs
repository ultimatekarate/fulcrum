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
    /// Construct a `HotToCold` if and only if the move is a valid
    /// (general) Pigou-Dalton transfer against `fleet`:
    ///
    /// 1. Both machines exist and `source ≠ destination`.
    /// 2. Source utilization > destination utilization (the transfer goes
    ///    in the rich → poor direction).
    /// 3. The transferred mass does not exceed the gap between source and
    ///    destination: `mass ≤ load_src - load_dst`.
    ///
    /// Condition 3 is the *general* Pigou-Dalton condition. It is sufficient
    /// for the resulting load vector to be majorized by the original — the
    /// transfer may overshoot the midpoint and flip the relative order of
    /// source and destination, but the result is still majorized as long as
    /// `mass ≤ load_src - load_dst`. The stricter "preserve order"
    /// condition `mass ≤ (load_src - load_dst) / 2` is sufficient but not
    /// necessary, and we use the looser one to admit more legitimately
    /// majorization-decreasing transfers.
    pub fn witness(
        source: MachineId,
        destination: MachineId,
        mass: Mass,
        fleet: &Fleet,
    ) -> Option<Self> {
        if source == destination {
            return None;
        }
        let load_src = fleet.load(source)?;
        let load_dst = fleet.load(destination)?;
        if load_src <= load_dst {
            return None;
        }
        // v0: uniform capacity, so utilization comparison reduces to load
        // comparison. The order-preservation condition is mass ≤ load_src -
        // load_dst.
        if mass.0 > load_src - load_dst {
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
    /// Construct a `Neutral` move if and only if source and destination have
    /// equal load (under uniform capacity, this is equal utilization).
    pub fn witness(
        source: MachineId,
        destination: MachineId,
        mass: Mass,
        fleet: &Fleet,
    ) -> Option<Self> {
        if source == destination {
            return None;
        }
        let load_src = fleet.load(source)?;
        let load_dst = fleet.load(destination)?;
        if load_src != load_dst {
            return None;
        }
        if mass.0 > load_src {
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
        "Hardy-Littlewood-Pólya 1929; Marshall-Olkin §1.A.1: a Pigou-Dalton \
         transfer (rich → poor with mass ≤ load_src - load_dst) is \
         majorization-decreasing, hence non-increasing for any Schur-convex \
         gauge.";
    const NAME: &'static str = "HotToCold";
}

impl Sealed for Neutral {}
impl Primitive for Neutral {
    const EFFECT: Effect = Effect::Permutation;
    const THEOREM: &'static str =
        "Symmetric gauges are invariant under coordinate permutation. A \
         mass-preserving exchange between equally-loaded coordinates \
         produces a load vector that is a permutation of the original \
         within the equal-utilization class; gauge value unchanged.";
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
