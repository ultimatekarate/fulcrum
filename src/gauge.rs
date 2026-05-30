//! Gauge functions over fleet state.
//!
//! A `Gauge<N>` is a real-valued function on `Fleet<N>`. The framework's
//! claim is that *Schur-convex* gauges admit a particular composition
//! algebra: any mass-decreasing or majorization-decreasing operation
//! reduces a Schur-convex gauge, so such operations preserve the safety
//! claim `g(fleet) ≤ τ` unconditionally.
//!
//! `SchurConvex<N>` is sealed — the only impls live in this module. The
//! seal is tighter than honor-system: the impls are `SumTopK<K, N>` (the
//! parameterized **Ky Fan k-norm**) and `WeightedKyFan<K, N>` (non-negative
//! weighted combinations). These are mathematically faithful to the *Ky
//! Fan dominance theorem*: `x ≻ y` (majorization) iff `‖x‖_(k) ≤ ‖y‖_(k)`
//! for every `k`, where `‖·‖_(k)` is the Ky Fan k-norm (sum of the k
//! largest entries). The Ky Fan family generates the partial order at the
//! heart of the framework, and non-negative combinations are themselves
//! Schur-convex (by linearity of the Schur ordering). Restricting
//! `SchurConvex` to this family makes the seal a precise mathematical
//! statement.
//!
//! ## Multi-dimensional reduction
//!
//! Phase 2 adds an `N`-dimensional load model. The component-wise gauges
//! reduce each machine's `[f64; N]` utilization vector to a scalar by
//! taking the worst dimension (`max_d`), and then apply the Schur-convex
//! reduction over machines. This is sound under multi-dimensional
//! majorization (Marshall-Olkin §15.A) when the move algebra ensures
//! per-dimension Pigou-Dalton conditions — the witnesses in
//! `move_kind.rs` enforce exactly that.
//!
//! Joint multi-dim gauges (operating on the joint utilization
//! distribution rather than the per-machine worst-dim) are deferred to a
//! follow-up; the component-wise reduction is the natural starting point
//! and covers the operationally interesting "no machine should be over τ
//! in any dimension" framing.
//!
//! Common gauges are exposed as type aliases: `Linfty<N> = SumTopK<1, N>`
//! is the worst-machine, worst-dimension utilization.

use crate::load::Fleet;

mod sealed {
    pub trait Sealed {}
}

/// A real-valued function on `Fleet<N>`. Method takes `&self` so gauges
/// may carry runtime parameters (e.g., `WeightedKyFan`'s weights array).
pub trait Gauge<const N: usize>: sealed::Sealed {
    /// Evaluate the gauge on a fleet snapshot.
    fn eval(&self, fleet: &Fleet<N>) -> f64;

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
/// **Sealed by Ky Fan dominance**: the only `SchurConvex<N>` impls in
/// this crate are `SumTopK<K, N>` and `WeightedKyFan<K, N>`. By the Ky Fan
/// dominance theorem, the Ky Fan family generates the majorization order
/// on non-negative `n`-vectors. Restricting `SchurConvex` to this family
/// loses no expressive power that matters for the framework, and it makes
/// the seal a precise mathematical statement.
pub trait SchurConvex<const N: usize>: Gauge<N> {}

/// Sum of the top K worst-dimension utilizations across the fleet — the
/// **Ky Fan k-norm** of the per-machine worst-dim utilization vector.
///
/// Per machine, reduce `[f64; N]` to a scalar via `max_d`. Then sort the
/// per-machine scalars descending and sum the top `K`.
///
/// **Schur-convex**: top-K sum is the canonical Schur-convex example
/// (Marshall-Olkin §3.A.1; Hardy-Littlewood-Pólya). Composing with the
/// per-machine `max_d` reduction preserves Schur-convexity on the
/// per-machine vector: if the per-dim utilization vectors are
/// componentwise weakly-super-majorized (which the multi-dim witnesses
/// enforce), then the worst-dim per machine is also pointwise non-
/// increasing in the partial order, so the resulting scalar vector is
/// also weakly-super-majorized, and the top-K sum is non-increasing.
#[derive(Default)]
pub struct SumTopK<const K: usize, const N: usize>;

impl<const K: usize, const N: usize> sealed::Sealed for SumTopK<K, N> {}

// `impl Gauge<N> for SumTopK` (the `eval` body) lives in the laboratory
// layer: see `src/gauge_eval.rs`. The marker `SchurConvex` membership stays
// here beside the type, alongside `Sealed`.
impl<const K: usize, const N: usize> SchurConvex<N> for SumTopK<K, N> {}

/// ℓ_∞ gauge: max-machine, max-dimension utilization.
///
/// `eval(fleet) = max_i max_d (load[i][d] / cap[i][d])`.
///
/// Equivalent to `SumTopK<1, N>`. Provided as a type alias for clarity —
/// it is the operationally meaningful "no machine should exceed τ% of
/// capacity in any dimension" gauge.
pub type Linfty<const N: usize> = SumTopK<1, N>;

/// **Non-negative weighted combination of Ky Fan k-norms** over the
/// per-machine worst-dim utilization vector.
///
/// `eval(fleet) = Σ_{k=1}^{K} weights[k-1] · ‖worst_dim_util‖_(k)`
///
/// where `weights[k-1]` is the coefficient on the k-th Ky Fan norm.
///
/// **Schur-convex**: a non-negative linear combination of Schur-convex
/// functions is Schur-convex.
///
/// **Construction**: use [`WeightedKyFan::new`] (returns `Option` —
/// rejects negative or NaN weights). Direct construction is forbidden
/// because the framework's totality argument depends on weights being
/// non-negative.
pub struct WeightedKyFan<const K: usize, const N: usize> {
    weights: [f64; K],
}

impl<const K: usize, const N: usize> WeightedKyFan<K, N> {
    /// Construct a `WeightedKyFan<K, N>` if all weights are finite and
    /// non-negative. Negative or NaN weights would break Schur-convexity
    /// and invalidate the framework's totality argument; the constructor
    /// rejects them.
    pub fn new(weights: [f64; K]) -> Option<Self> {
        if weights.iter().all(|w| w.is_finite() && *w >= 0.0) {
            Some(Self { weights })
        } else {
            None
        }
    }

    /// Borrow the weights. Construction validation guarantees these are
    /// non-negative and finite.
    pub fn weights(&self) -> &[f64; K] {
        &self.weights
    }
}

impl<const K: usize, const N: usize> sealed::Sealed for WeightedKyFan<K, N> {}

// `impl Gauge<N> for WeightedKyFan` (the `eval` body) lives in the
// laboratory layer: see `src/gauge_eval.rs`.
impl<const K: usize, const N: usize> SchurConvex<N> for WeightedKyFan<K, N> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::{Fleet, MachineId};

    fn fleet1_uniform(loads: &[(u64, u64)], capacity: u64) -> Fleet<1> {
        let mut f = Fleet::new();
        for &(id, load) in loads {
            f.add_machine(MachineId(id), [capacity], [load]);
        }
        f
    }

    #[test]
    fn linfty_empty_fleet_is_zero() {
        let fleet: Fleet<1> = Fleet::new();
        assert_eq!(Linfty::<1>::default().eval(&fleet), 0.0);
    }

    #[test]
    fn linfty_picks_max_utilization_1d() {
        let fleet = fleet1_uniform(&[(1, 30), (2, 80), (3, 50)], 100);
        assert!((Linfty::<1>::default().eval(&fleet) - 0.80).abs() < 1e-9);
    }

    #[test]
    fn sumtopk_sums_top_k_utilizations_1d() {
        let fleet = fleet1_uniform(&[(1, 30), (2, 80), (3, 50)], 100);
        assert!((SumTopK::<2, 1>::default().eval(&fleet) - 1.30).abs() < 1e-9);
        assert!(
            (SumTopK::<1, 1>::default().eval(&fleet) - Linfty::<1>::default().eval(&fleet)).abs()
                < 1e-9
        );
        assert!((SumTopK::<3, 1>::default().eval(&fleet) - 1.60).abs() < 1e-9);
        assert!((SumTopK::<10, 1>::default().eval(&fleet) - 1.60).abs() < 1e-9);
    }

    #[test]
    fn weighted_kyfan_combines_norms_1d() {
        let fleet = fleet1_uniform(&[(1, 80), (2, 50), (3, 30)], 100);
        let g = WeightedKyFan::<2, 1>::new([1.0, 0.5]).unwrap();
        assert!((g.eval(&fleet) - 1.45).abs() < 1e-9);
    }

    #[test]
    fn weighted_kyfan_rejects_invalid_weights() {
        assert!(WeightedKyFan::<2, 1>::new([1.0, -0.5]).is_none());
        assert!(WeightedKyFan::<2, 1>::new([1.0, f64::NAN]).is_none());
        assert!(WeightedKyFan::<2, 1>::new([1.0, f64::INFINITY]).is_none());
        assert!(WeightedKyFan::<2, 1>::new([0.0, 0.0]).is_some());
        assert!(WeightedKyFan::<3, 1>::new([1.0, 0.5, 0.25]).is_some());
    }

    // ----- Multi-dim (component-wise) -----

    #[test]
    fn linfty_multi_dim_takes_worst_dim_per_machine() {
        // Two-dim. Machine 1 is balanced (0.40, 0.40); machine 2 is
        // skewed (0.20, 0.90). Worst-dim per machine: 0.40 vs 0.90.
        // Linfty picks the worst worst-dim → 0.90.
        let mut f: Fleet<2> = Fleet::new();
        f.add_machine(MachineId(1), [100, 100], [40, 40]);
        f.add_machine(MachineId(2), [100, 100], [20, 90]);
        assert!((Linfty::<2>::default().eval(&f) - 0.90).abs() < 1e-9);
    }

    #[test]
    fn sumtopk_multi_dim_uses_per_machine_worst_dim() {
        let mut f: Fleet<2> = Fleet::new();
        f.add_machine(MachineId(1), [100, 100], [80, 30]); // worst-dim 0.80
        f.add_machine(MachineId(2), [100, 100], [40, 60]); // worst-dim 0.60
        f.add_machine(MachineId(3), [100, 100], [10, 50]); // worst-dim 0.50
        // Top-2 worst-dim: 0.80 + 0.60 = 1.40.
        assert!((SumTopK::<2, 2>::default().eval(&f) - 1.40).abs() < 1e-9);
    }

    #[test]
    fn linfty_under_heterogeneous_capacity_multi_dim() {
        let mut f: Fleet<2> = Fleet::new();
        f.add_machine(MachineId(1), [100, 200], [80, 80]); // utils (0.80, 0.40), worst 0.80
        f.add_machine(MachineId(2), [200, 100], [80, 80]); // utils (0.40, 0.80), worst 0.80
        f.add_machine(MachineId(3), [100, 100], [30, 30]); // utils (0.30, 0.30), worst 0.30
        assert!((Linfty::<2>::default().eval(&f) - 0.80).abs() < 1e-9);
    }
}
