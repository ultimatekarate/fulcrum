//! The sealed alphabet of move kinds.
//!
//! This module exists to convert the social discipline "the alphabet stays
//! small" into a mechanical one. New typed-pure primitives must touch this
//! module. They must declare a mathematical effect category and cite the
//! totality theorem that justifies the apply signature. Reviewers' job
//! reduces to checking whether the cited theorem actually justifies the
//! declared effect — a single review point per primitive, with a published
//! answer.
//!
//! The structural fact behind the small alphabet is **Birkhoff-von
//! Neumann**: every doubly stochastic matrix is a convex combination of
//! permutation matrices. Translated to load vectors, every mass-preserving
//! majorization-decreasing operation decomposes into a sequence of
//! Pigou-Dalton transfers and permutations. Combined with mass-changing
//! operations (Place, Remove), the framework's five primitives form a
//! generating set for every operation the algebra needs to express. A
//! sixth primitive can only be redundant (a composition of existing ones —
//! belongs in `Derived`) or outside the algebra (catch-all — belongs as
//! `MassPreservingFree` or `MassIncreasing`). There is no third category
//! of legitimate new typed-pure primitive.

mod sealed {
    pub trait Sealed {}
}

/// The five mathematical effect categories an operation on a load vector
/// can have. Each category determines whether `apply` is total or
/// fallible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Mass strictly decreases. Total `apply` under any monotone
    /// Schur-convex gauge on non-negative vectors.
    MassDecreasing,

    /// Mass-preserving Pigou-Dalton transfer (rich → poor, with the
    /// transferred mass not exceeding the rich-poor gap). Total `apply`
    /// under any Schur-convex gauge.
    PigouDalton,

    /// Mass-preserving exchange between equally-loaded coordinates.
    /// Total `apply` under any symmetric gauge (which Schur-convex gauges
    /// are).
    Permutation,

    /// Mass strictly increases. `apply` is fallible — adding mass can
    /// push any monotone Schur-convex gauge upward.
    MassIncreasing,

    /// Mass-preserving but not majorization-decreasing (anti-Pigou-Dalton,
    /// neutral migrations whose loads aren't equal). `apply` is fallible.
    MassPreservingFree,
}

impl Effect {
    /// Whether this effect admits a total `apply`. Typed-pure effects
    /// preserve the safety claim by mathematical argument; catch-all
    /// effects require runtime re-evaluation.
    pub const fn is_typed_pure(self) -> bool {
        matches!(
            self,
            Effect::MassDecreasing | Effect::PigouDalton | Effect::Permutation
        )
    }
}

/// A primitive of the move alphabet.
///
/// Sealed: only `move_kind.rs` may declare types implementing this trait.
/// Adding a new primitive requires:
/// 1. Defining the type and its data fields (in `move_kind.rs`).
/// 2. Implementing `apply` (in `safe.rs`) with the appropriate signature
///    (total for typed-pure effects, fallible for catch-all).
/// 3. Implementing `Primitive` here with a non-empty `THEOREM` citation
///    that justifies the totality argument for the declared `EFFECT`.
///
/// Reviewers verify (3) once. Users trust the typing thereafter.
pub trait Primitive: sealed::Sealed {
    /// The mathematical effect this primitive has on a load vector. Drives
    /// the apply signature (total vs. fallible).
    const EFFECT: Effect;

    /// Citation for the totality / well-definedness theorem. Must be
    /// non-empty and reference a published source. Verified by
    /// `tests/alphabet.rs`.
    const THEOREM: &'static str;

    /// Human-readable name for diagnostics.
    const NAME: &'static str;
}

/// Marker for moves that are *not* in the sealed primitive alphabet.
///
/// External crates that need a convenience move kind beyond the five
/// primitives must implement `Derived`. The discipline is that derived
/// moves decompose into primitive moves — they do not perform fleet
/// mutation directly. The framework cannot mechanically verify the
/// decomposition (Rust lacks higher-kinded types to express
/// "this trait method is a sequence of primitive applies"), but the
/// `DECOMPOSITION` const documents the intended composition for review.
///
/// Implementors are expected to write `apply` as a sequence of calls to
/// primitive `apply` methods, threading `Safe<G>` through. Touching the
/// `Fleet` directly via the crate-internal API would defeat the algebra.
pub trait Derived {
    /// English description of how this derived move composes from
    /// primitives. e.g. `"Remove + Place"` for a "drain-and-replace"
    /// convenience kind.
    const DECOMPOSITION: &'static str;
}

// Crate-internal: re-export for `move_kind.rs` to seal its impls.
pub(crate) use sealed::Sealed;
