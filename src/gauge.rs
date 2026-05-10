//! Gauge functions over fleet state.
//!
//! A `Gauge` is a real-valued function on `Fleet`. The framework's claim is
//! that *Schur-convex* gauges admit a particular composition algebra: any
//! mass-decreasing or majorization-decreasing operation reduces a Schur-convex
//! gauge, so such operations preserve the safety claim `g(fleet) ≤ τ`
//! unconditionally.
//!
//! `SchurConvex` is sealed — the only impls live in this module. The seal is
//! tighter than honor-system: the `SchurConvex` impls are `SumTopK<K>` (the
//! parameterized **Ky Fan k-norm**) and `WeightedKyFan<N>` (non-negative
//! weighted combinations of Ky Fan norms). This is mathematically faithful to
//! the *Ky Fan dominance theorem*: `x ≻ y` (majorization) iff
//! `‖x‖_(k) ≤ ‖y‖_(k)` for every `k`, where `‖·‖_(k)` is the Ky Fan k-norm
//! (sum of the k largest entries). The Ky Fan k-norms generate the partial
//! order at the heart of the framework, and non-negative combinations of
//! them are themselves Schur-convex (by linearity of the Schur ordering).
//! Restricting `SchurConvex` to this family makes the seal a precise
//! mathematical statement, not a curatorial one.
//!
//! Common gauges are exposed as type aliases: `Linfty = SumTopK<1>` is the
//! ℓ_∞ norm (max utilization).

use crate::load::Fleet;

mod sealed {
    /// Sealing trait. Prevents downstream crates from declaring their own
    /// `SchurConvex` impls and silently breaking the totality of `apply`
    /// for typed-pure moves.
    pub trait Sealed {}
}

/// A real-valued function on `Fleet`. Method takes `&self` so gauges may
/// carry runtime parameters (e.g., `WeightedKyFan`'s weights array).
pub trait Gauge: sealed::Sealed {
    /// Evaluate the gauge on a fleet snapshot.
    fn eval(&self, fleet: &Fleet) -> f64;

    /// Human-readable name for diagnostics. Default: type name.
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Marker for Schur-convex gauges.
///
/// A function `f` is Schur-convex iff `x ≻ y` (x majorizes y) implies
/// `f(x) ≥ f(y)`. The Pigou-Dalton transfer is the canonical
/// majorization-decreasing operation; mass removal strictly decreases any
/// monotone Schur-convex gauge on non-negative load vectors.
///
/// **Sealed by Ky Fan dominance**: the only `SchurConvex` impls in this crate
/// are `SumTopK<K>` (Ky Fan k-norm) and `WeightedKyFan<N>` (non-negative
/// linear combinations of Ky Fan norms). By the Ky Fan dominance theorem,
/// the family `{‖·‖_(k) : k = 1, …, n}` *generates* the majorization order
/// on non-negative `n`-vectors — equivalently, any symmetric gauge function
/// on those vectors is dominated by a non-negative combination of these.
/// So restricting `SchurConvex` to the Ky Fan family loses no expressive
/// power that matters for the framework, and it makes the seal a precise
/// mathematical statement.
pub trait SchurConvex: Gauge {}

/// Sum of the top K utilizations across the fleet — the **Ky Fan k-norm**
/// of the utilization vector.
///
/// `eval(fleet) = ‖util‖_(K) = Σ_{i ∈ top-K} (load_i / capacity_i)`.
///
/// **Schur-convex**: the sum of the k largest entries is the canonical
/// example of a Schur-convex function on non-negative vectors (Marshall &
/// Olkin §3.A.1; Hardy-Littlewood-Pólya). Equivalently to majorization:
/// `x ≻ y` iff `Σ_{i ≤ k} x_(i) ≥ Σ_{i ≤ k} y_(i)` for every `k` (Ky Fan
/// dominance). So the family `{SumTopK<K> : K = 1, 2, …}` is the partial
/// order at the heart of the framework, in code form.
///
/// **Schur-convexity sketch**:
/// - Let φ(x) = sum of the top K of x. φ depends only on sorted descending
///   order, so it is symmetric (invariant under permutation).
/// - For any Pigou-Dalton transfer (rich → poor with order-preserving mass),
///   the new top-K sum cannot exceed the old: either neither rich nor poor
///   is in the top K (sum unchanged), or the rich is in the top K and the
///   poor isn't (sum decreases by the transferred amount), or both are in
///   the top K (sum unchanged because mass is preserved within the top K).
/// - Therefore `x ≻ y ⇒ φ(x) ≥ φ(y)`.
///
/// **Adding zero coordinates** (used by `borg_replay` when machines are
/// pre-registered with zero load): adding a 0 entry to the vector cannot
/// increase the sum of the top K — at worst, the new 0 displaces nothing,
/// at best it pushes a positive value out of the top K. Safe.
#[derive(Default)]
pub struct SumTopK<const K: usize>;

impl<const K: usize> sealed::Sealed for SumTopK<K> {}

impl<const K: usize> Gauge for SumTopK<K> {
    fn eval(&self, fleet: &Fleet) -> f64 {
        if fleet.is_empty() || K == 0 {
            return 0.0;
        }
        let cap = fleet.capacity() as f64;
        let mut utils: Vec<f64> = fleet.iter().map(|(_, l)| l as f64 / cap).collect();
        // Sort descending so the top-K is the prefix.
        utils.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        utils.iter().take(K).sum()
    }

    fn name(&self) -> &'static str {
        "SumTopK"
    }
}

impl<const K: usize> SchurConvex for SumTopK<K> {}

/// ℓ_∞ gauge: maximum utilization across the fleet.
///
/// `eval(fleet) = max_i (load_i / capacity_i)`.
///
/// Equivalent to the Ky Fan 1-norm: `‖x‖_(1) = max_i x_i`. Provided as a
/// type alias for clarity — it is a common operationally-meaningful gauge,
/// and the alias makes the typical "no machine should exceed τ% of
/// capacity" framing read naturally in user code.
pub type Linfty = SumTopK<1>;

/// **Non-negative weighted combination of Ky Fan k-norms.**
///
/// `eval(fleet) = Σ_{k=1}^{N} weights[k-1] · ‖util‖_(k)`
///
/// where `weights[k-1]` is the coefficient on the k-th Ky Fan norm. This
/// covers essentially every operationally interesting symmetric gauge:
/// every monotone symmetric gauge function on non-negative vectors can be
/// written as a non-negative weighted combination (or, in the general
/// case, supremum thereof) of Ky Fan norms.
///
/// **Schur-convex**: a non-negative linear combination of Schur-convex
/// functions is Schur-convex. Each `SumTopK<K>` is Schur-convex (Marshall-
/// Olkin §3.A.1); non-negative scalars and addition both preserve the
/// Schur-convex cone. Therefore `WeightedKyFan<N>` with weights ≥ 0 is
/// Schur-convex.
///
/// **Construction**: use [`WeightedKyFan::new`] (returns `Option` —
/// rejects negative or NaN weights). Direct construction is forbidden
/// because the framework's totality argument depends on weights being
/// non-negative.
///
/// # Examples
///
/// ```
/// use fulcrum::gauge::{Gauge, WeightedKyFan};
/// use fulcrum::load::{Fleet, MachineId};
///
/// let mut fleet = Fleet::new(100);
/// fleet.add_machine(MachineId(1), 80);
/// fleet.add_machine(MachineId(2), 50);
/// fleet.add_machine(MachineId(3), 30);
///
/// // 1.0 · ‖·‖_(1) + 0.5 · ‖·‖_(2)
/// // = 0.80 + 0.5 · (0.80 + 0.50)
/// // = 0.80 + 0.65 = 1.45
/// let g = WeightedKyFan::new([1.0, 0.5]).unwrap();
/// assert!((g.eval(&fleet) - 1.45).abs() < 1e-9);
///
/// // Negative weights are rejected — they would break Schur-convexity.
/// assert!(WeightedKyFan::new([1.0, -0.5]).is_none());
/// ```
pub struct WeightedKyFan<const N: usize> {
    weights: [f64; N],
}

impl<const N: usize> WeightedKyFan<N> {
    /// Construct a `WeightedKyFan<N>` if all weights are finite and
    /// non-negative. Negative or NaN weights would break Schur-convexity
    /// and invalidate the framework's totality argument; the constructor
    /// rejects them.
    pub fn new(weights: [f64; N]) -> Option<Self> {
        if weights.iter().all(|w| w.is_finite() && *w >= 0.0) {
            Some(Self { weights })
        } else {
            None
        }
    }

    /// Borrow the weights. Construction validation guarantees these are
    /// non-negative and finite.
    pub fn weights(&self) -> &[f64; N] {
        &self.weights
    }
}

impl<const N: usize> sealed::Sealed for WeightedKyFan<N> {}

impl<const N: usize> Gauge for WeightedKyFan<N> {
    fn eval(&self, fleet: &Fleet) -> f64 {
        if fleet.is_empty() || N == 0 {
            return 0.0;
        }
        let cap = fleet.capacity() as f64;
        let mut utils: Vec<f64> = fleet.iter().map(|(_, l)| l as f64 / cap).collect();
        utils.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

        // Compute Σ_{k=1}^{N} weights[k-1] · (sum of top-k utilizations)
        // by walking the sorted utils once and accumulating the cumulative
        // top-k sum.
        let mut total = 0.0_f64;
        let mut cumulative = 0.0_f64;
        for (k, w) in self.weights.iter().enumerate() {
            if k < utils.len() {
                cumulative += utils[k];
            }
            total += w * cumulative;
        }
        total
    }

    fn name(&self) -> &'static str {
        "WeightedKyFan"
    }
}

impl<const N: usize> SchurConvex for WeightedKyFan<N> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::{Fleet, MachineId};

    #[test]
    fn linfty_empty_fleet_is_zero() {
        let fleet = Fleet::new(100);
        assert_eq!(Linfty::default().eval(&fleet), 0.0);
    }

    #[test]
    fn linfty_picks_max_utilization() {
        let mut fleet = Fleet::new(100);
        fleet.add_machine(MachineId(1), 30);
        fleet.add_machine(MachineId(2), 80);
        fleet.add_machine(MachineId(3), 50);
        assert!((Linfty::default().eval(&fleet) - 0.80).abs() < 1e-9);
    }

    #[test]
    fn sumtopk_sums_top_k_utilizations() {
        let mut fleet = Fleet::new(100);
        fleet.add_machine(MachineId(1), 30);
        fleet.add_machine(MachineId(2), 80);
        fleet.add_machine(MachineId(3), 50);
        // top 2 are 80 and 50 → utilizations 0.80 + 0.50 = 1.30
        assert!((SumTopK::<2>::default().eval(&fleet) - 1.30).abs() < 1e-9);
        // top 1 IS Linfty (the alias).
        assert!(
            (SumTopK::<1>::default().eval(&fleet) - Linfty::default().eval(&fleet)).abs() < 1e-9
        );
        // top 3 = sum of all
        assert!((SumTopK::<3>::default().eval(&fleet) - 1.60).abs() < 1e-9);
        // K > n → still just sum of all
        assert!((SumTopK::<10>::default().eval(&fleet) - 1.60).abs() < 1e-9);
    }

    #[test]
    fn sumtopk_respects_majorization_under_pigou_dalton() {
        // x = (80, 30, 30); transfer 20 from coord 1 to coord 2.
        // x' = (60, 50, 30). Both have mass 140; x ≻ x'.
        // SumTopK<2> on x = 80 + 30 = 110. On x' = 60 + 50 = 110. Equal.
        // SumTopK<1> on x = 80. On x' = 60. Strict decrease.
        let mut before = Fleet::new(100);
        before.add_machine(MachineId(1), 80);
        before.add_machine(MachineId(2), 30);
        before.add_machine(MachineId(3), 30);

        let mut after = Fleet::new(100);
        after.add_machine(MachineId(1), 60);
        after.add_machine(MachineId(2), 50);
        after.add_machine(MachineId(3), 30);

        let g2 = SumTopK::<2>::default();
        let g1 = SumTopK::<1>::default();
        assert!(g2.eval(&before) >= g2.eval(&after));
        assert!(g1.eval(&before) > g1.eval(&after));
    }

    #[test]
    fn linfty_is_sumtopk_one() {
        let mut fleet = Fleet::new(100);
        fleet.add_machine(MachineId(1), 73);
        fleet.add_machine(MachineId(2), 19);
        fleet.add_machine(MachineId(3), 84);
        let a = Linfty::default().eval(&fleet);
        let b = SumTopK::<1>::default().eval(&fleet);
        assert_eq!(a, b);
    }

    #[test]
    fn weighted_kyfan_combines_norms() {
        let mut fleet = Fleet::new(100);
        fleet.add_machine(MachineId(1), 80);
        fleet.add_machine(MachineId(2), 50);
        fleet.add_machine(MachineId(3), 30);

        // 1.0 · ‖·‖_(1) + 0.5 · ‖·‖_(2)
        // = 0.80 + 0.5 · (0.80 + 0.50)
        // = 0.80 + 0.65 = 1.45
        let g = WeightedKyFan::new([1.0, 0.5]).unwrap();
        assert!((g.eval(&fleet) - 1.45).abs() < 1e-9);
    }

    #[test]
    fn weighted_kyfan_rejects_invalid_weights() {
        // Negative weights — would break Schur-convexity.
        assert!(WeightedKyFan::new([1.0, -0.5]).is_none());
        // NaN — would break the gauge value altogether.
        assert!(WeightedKyFan::new([1.0, f64::NAN]).is_none());
        // Infinity — not finite.
        assert!(WeightedKyFan::new([1.0, f64::INFINITY]).is_none());
        // Zero is allowed (degenerate but valid).
        assert!(WeightedKyFan::new([0.0, 0.0]).is_some());
        // All non-negative is fine.
        assert!(WeightedKyFan::new([1.0, 0.5, 0.25]).is_some());
    }

    #[test]
    fn weighted_kyfan_respects_majorization() {
        // Majorization-decreasing transfer reduces or preserves any
        // non-negative weighted Ky Fan combination.
        let mut before = Fleet::new(100);
        before.add_machine(MachineId(1), 90);
        before.add_machine(MachineId(2), 20);
        before.add_machine(MachineId(3), 10);
        let mut after = Fleet::new(100);
        after.add_machine(MachineId(1), 60);
        after.add_machine(MachineId(2), 40);
        after.add_machine(MachineId(3), 20);

        // Various non-trivial weight vectors.
        for weights in [[1.0, 0.0], [0.5, 0.5], [0.0, 1.0], [1.0, 0.5]] {
            let g = WeightedKyFan::new(weights).unwrap();
            assert!(
                g.eval(&before) >= g.eval(&after),
                "majorization not respected with weights {:?}: {} < {}",
                weights,
                g.eval(&before),
                g.eval(&after)
            );
        }
    }

    #[test]
    fn weighted_kyfan_collapses_to_sumtopk_with_unit_weight() {
        let mut fleet = Fleet::new(100);
        fleet.add_machine(MachineId(1), 80);
        fleet.add_machine(MachineId(2), 50);
        fleet.add_machine(MachineId(3), 30);

        // weights [1.0, 0.0] = pure top-1 = Linfty
        let g_one = WeightedKyFan::new([1.0, 0.0]).unwrap();
        let linfty = Linfty::default();
        assert!((g_one.eval(&fleet) - linfty.eval(&fleet)).abs() < 1e-9);

        // weights [0.0, 1.0] = pure top-2
        let g_two = WeightedKyFan::new([0.0, 1.0]).unwrap();
        let sum2 = SumTopK::<2>::default();
        assert!((g_two.eval(&fleet) - sum2.eval(&fleet)).abs() < 1e-9);
    }
}
