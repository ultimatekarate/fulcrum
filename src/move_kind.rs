//! Move kinds: the alphabet.
//!
//! Each move kind is a marker around the data needed to apply the move.
//! The kinds partition by *mathematical effect* on a Schur-convex gauge:
//!
//! | Kind        | Effect                              | `apply`            |
//! | ----------- | ----------------------------------- | ------------------ |
//! | `Remove`    | Mass-decreasing                     | total              |
//! | `HotToCold` | Pigou-Dalton transfer (per-dim)     | total              |
//! | `Neutral`   | Mass-preserving, equal utilization  | total              |
//! | `ColdToHot` | Anti-Robin-Hood (per-dim)           | fallible           |
//! | `Place`     | Mass-adding                         | fallible           |
//!
//! `HotToCold` and `Neutral` are *witness types* — their public
//! construction is gated by a fallible check. Once constructed, the
//! `apply` is total. The actual `apply` impls live in `safe.rs`.
//!
//! Phase 2 propagates a const-generic dimension `N` through every kind.
//! Witness conditions become per-dimension: a `HotToCold` is typed-pure
//! iff each dimension where mass moves independently satisfies the Phase 1
//! conditions. A move that's Pigou-Dalton in one dimension and anti-
//! Robin-Hood in another falls through to the catch-all path.

use crate::alphabet::{Effect, Primitive, Sealed};
use crate::load::{Fleet, MachineId, Mass};

/// Mass removal from a single machine. Mass-decreasing on every dimension
/// — strictly reduces any monotone Schur-convex gauge on non-negative
/// load vectors.
///
/// **Totality argument**:
/// - The new load vector is componentwise ≤ the old in every dimension.
/// - The per-machine worst-dim utilization is therefore non-increasing.
/// - Schur-convex monotone gauges are non-increasing on the resulting
///   per-machine scalar vector.
#[derive(Clone, Copy, Debug)]
pub struct Remove<const N: usize> {
    pub machine: MachineId,
    pub mass: Mass<N>,
}

impl<const N: usize> Remove<N> {
    /// Construct a `Remove` move. Always succeeds — `Remove` has no
    /// preconditions to enforce at construction time.
    pub fn new(machine: MachineId, mass: Mass<N>) -> Self {
        Remove { machine, mass }
    }
}

/// Pigou-Dalton transfer, multi-dimensional version: mass moved from a
/// higher-utilization source to a lower-utilization destination such that
/// the per-dimension Phase 1 condition holds in every dimension where
/// mass is non-zero.
///
/// **Construction**: the public constructor is fallible — see
/// [`Self::witness`]. Direct construction is intentionally not provided;
/// the witness check is the entire point of the type.
#[derive(Clone, Copy, Debug)]
pub struct HotToCold<const N: usize> {
    pub source: MachineId,
    pub destination: MachineId,
    pub mass: Mass<N>,
    /// Private. Construction requires the witness check to pass.
    _witness: WitnessToken,
}

impl<const N: usize> HotToCold<N> {
    /// Construct a `HotToCold` if and only if the move is weak-super-
    /// majorization-decreasing in every dimension against `fleet`. For
    /// each dimension `d` where `mass[d] > 0`:
    ///
    /// 1. `cap(src)[d] ≤ cap(dst)[d]` — transfer to a same-or-larger
    ///    machine in that dim.
    /// 2. `util(src)[d] > util(dst)[d]` — rich → poor on utilization.
    /// 3. `mass[d] · cap(src)[d] ≤ load(src)[d] · cap(dst)[d] −
    ///    load(dst)[d] · cap(src)[d]` — destination-side utilization gap.
    ///
    /// Dimensions with `mass[d] = 0` are no-ops in that dimension and
    /// have no condition. The move is rejected if the source equals the
    /// destination, either machine is unknown, any capacity is zero in a
    /// dimension where mass moves, or the mass vector is entirely zero
    /// (no-op).
    ///
    /// When `N = 1` and capacities match, this collapses to the Phase 1
    /// rule, which collapses further to the v0 rule when capacities are
    /// uniform. The framework's per-dim conditions cleanly subsume both
    /// earlier versions.
    pub fn witness(
        source: MachineId,
        destination: MachineId,
        mass: Mass<N>,
        fleet: &Fleet<N>,
    ) -> Option<Self> {
        if source == destination {
            return None;
        }
        if mass.is_zero() {
            return None;
        }
        let src_spec = fleet.spec(source)?;
        let dst_spec = fleet.spec(destination)?;
        for d in 0..N {
            if mass.0[d] == 0 {
                continue;
            }
            if src_spec.capacity[d] == 0 || dst_spec.capacity[d] == 0 {
                return None;
            }
            if src_spec.capacity[d] > dst_spec.capacity[d] {
                return None;
            }
            // util(src)[d] > util(dst)[d] in integer cross-product form.
            let load_src_x_cap_dst = src_spec.load[d] as u128 * dst_spec.capacity[d] as u128;
            let load_dst_x_cap_src = dst_spec.load[d] as u128 * src_spec.capacity[d] as u128;
            if load_src_x_cap_dst <= load_dst_x_cap_src {
                return None;
            }
            // mass[d]·cap(src)[d] ≤ load(src)[d]·cap(dst)[d] − load(dst)[d]·cap(src)[d].
            let gap = load_src_x_cap_dst - load_dst_x_cap_src;
            let mass_x_cap_src = mass.0[d] as u128 * src_spec.capacity[d] as u128;
            if mass_x_cap_src > gap {
                return None;
            }
        }
        Some(HotToCold {
            source,
            destination,
            mass,
            _witness: WitnessToken,
        })
    }
}

/// Mass-preserving migration between machines at equal utilization in
/// every dimension. Per-dim, equal capacity AND equal load required (the
/// Permutation effect demands a true coordinate transposition).
#[derive(Clone, Copy, Debug)]
pub struct Neutral<const N: usize> {
    pub source: MachineId,
    pub destination: MachineId,
    pub mass: Mass<N>,
    _witness: WitnessToken,
}

impl<const N: usize> Neutral<N> {
    /// Construct a `Neutral` move if and only if source and destination
    /// have equal capacity AND equal load in every dimension.
    pub fn witness(
        source: MachineId,
        destination: MachineId,
        mass: Mass<N>,
        fleet: &Fleet<N>,
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
        for d in 0..N {
            if mass.0[d] > src_spec.load[d] {
                return None;
            }
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
/// utilization machine in some dimension, or otherwise failing the
/// per-dim Pigou-Dalton conditions. Can violate the gauge bound — `apply`
/// is fallible.
#[derive(Clone, Copy, Debug)]
pub struct ColdToHot<const N: usize> {
    pub source: MachineId,
    pub destination: MachineId,
    pub mass: Mass<N>,
}

impl<const N: usize> ColdToHot<N> {
    pub fn new(source: MachineId, destination: MachineId, mass: Mass<N>) -> Self {
        ColdToHot { source, destination, mass }
    }
}

/// Fresh placement of mass on a machine. Mass-adding — can hot-spot.
/// `apply` is fallible.
#[derive(Clone, Copy, Debug)]
pub struct Place<const N: usize> {
    pub machine: MachineId,
    pub mass: Mass<N>,
}

impl<const N: usize> Place<N> {
    pub fn new(machine: MachineId, mass: Mass<N>) -> Self {
        Place { machine, mass }
    }
}

/// Private token. The presence of one of these inside a move struct is
/// the type-level evidence that the witness check passed. The token is
/// intentionally not constructible outside this module.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WitnessToken;

// --------- Primitive trait impls (sealed alphabet membership) ---------
//
// Adding a new typed-pure primitive requires three coordinated changes:
//   1. Define the data type above.
//   2. Implement `apply` in `safe.rs` with the appropriate signature.
//   3. Add `Sealed` + `Primitive` impls below with EFFECT/THEOREM/NAME.

impl<const N: usize> Sealed for Remove<N> {}
impl<const N: usize> Primitive for Remove<N> {
    const EFFECT: Effect = Effect::MassDecreasing;
    const THEOREM: &'static str =
        "Marshall-Olkin §3.A: mass-decreasing on non-negative vectors \
         strictly reduces every monotone Schur-convex gauge. The N-dim \
         lift is componentwise: every dimension's per-machine load is \
         non-increased, so the per-machine worst-dim utilization vector \
         is also non-increased, and the gauge follows.";
    const NAME: &'static str = "Remove";
}

impl<const N: usize> Sealed for HotToCold<N> {}
impl<const N: usize> Primitive for HotToCold<N> {
    const EFFECT: Effect = Effect::PigouDalton;
    const THEOREM: &'static str =
        "Hardy-Littlewood-Pólya 1929; Marshall-Olkin §1.A.1, §3.A.8, \
         §15.A. Per-dimension witness conditions enforce that each dim's \
         load transfer decomposes as a T-transform on that dim's \
         utilization vector composed with a same-dim mass-decrease at src \
         (cap(src)[d] ≤ cap(dst)[d] makes the second piece non-negative). \
         Each per-dim utilization vector is therefore weak-super-\
         majorization decreasing. The component-wise gauge then takes the \
         per-machine worst-dim, which is monotone in each dim's vector \
         under the partial order, and applies a Schur-convex top-K \
         reduction. Composition non-increases the gauge.";
    const NAME: &'static str = "HotToCold";
}

impl<const N: usize> Sealed for Neutral<N> {}
impl<const N: usize> Primitive for Neutral<N> {
    const EFFECT: Effect = Effect::Permutation;
    const THEOREM: &'static str =
        "Symmetric gauges are invariant under coordinate permutation. \
         The witness requires equal capacity AND equal load in every \
         dimension, so the post-transfer per-dim utilization vector is a \
         coordinate transposition of the pre-transfer one in each \
         dimension. The component-wise gauge composes a per-machine \
         worst-dim reduction with a Schur-convex top-K, both symmetric, \
         so the gauge value is unchanged.";
    const NAME: &'static str = "Neutral";
}

impl<const N: usize> Sealed for ColdToHot<N> {}
impl<const N: usize> Primitive for ColdToHot<N> {
    const EFFECT: Effect = Effect::MassPreservingFree;
    const THEOREM: &'static str =
        "Anti-Robin-Hood transfers (in any dimension) can produce \
         majorization-incomparable per-dim utilization vectors; Schur-\
         convex gauges may strictly increase. Catch-all: apply re-checks \
         the gauge at runtime.";
    const NAME: &'static str = "ColdToHot";
}

impl<const N: usize> Sealed for Place<N> {}
impl<const N: usize> Primitive for Place<N> {
    const EFFECT: Effect = Effect::MassIncreasing;
    const THEOREM: &'static str =
        "Mass-increasing operations on non-negative vectors can push any \
         monotone Schur-convex gauge upward (in particular: max \
         utilization increases when mass is added to the most-loaded \
         machine in some dimension). Catch-all: apply re-checks the \
         gauge at runtime.";
    const NAME: &'static str = "Place";
}
