//! Gauge *evaluation* — the laboratory-layer logic behind the dictionary's
//! gauge declarations.
//!
//! Per `basis.yaml`'s governing split: the gauge *types*, the sealed
//! `SchurConvex` trait *declaration*, and well-formedness constructors live
//! in `src/gauge.rs` (dictionary); the functions that *evaluate* a gauge
//! over fleet/utilization state live here (laboratory). These `impl Gauge`
//! blocks carry the `eval` bodies relocated out of `gauge.rs`.
//!
//! This module is declared `mod gauge_eval;` (private) in `lib.rs`: trait
//! impls are in scope crate-wide regardless of the module's visibility, so
//! nothing needs to name the module path.

use crate::gauge::{Gauge, SumTopK, WeightedKyFan};
use fulcrum_dictionary::load::Fleet;
use std::cmp::Ordering;

/// Descending f64 comparator, matching the original
/// `sort_by(|a, b| b.partial_cmp(a).unwrap_or(Equal))`: non-orderable pairs
/// (NaN) compare `Equal` and never panic, so NaN handling is identical to the
/// pre-optimization eval.
fn desc(a: &f64, b: &f64) -> Ordering {
    b.partial_cmp(a).unwrap_or(Ordering::Equal)
}

/// The `k` largest per-machine worst-dim utilizations, in descending order.
///
/// Replaces a full `O(M log M)` sort with an `O(M)`-average `select_nth_unstable`
/// partition plus a sort of only the `k` selected (`O(k log k)`). The result is
/// **bit-identical** to sorting the whole vector descending and taking the first
/// `k`: the partition yields the same top-`k` *multiset* (boundary ties carry
/// equal values, so which physical element lands in the top `k` cannot change
/// any sum), and sorting those `k` reproduces the exact descending sequence the
/// old code summed — and f64 addition is order-sensitive, so reproducing the
/// order is what keeps the bits identical. Cheap when `k ≪ M`; `k = 1`
/// (`Linfty`) collapses to an `O(M)` max.
///
/// Callers guard `K == 0` and the empty fleet, so here `1 ≤ k ≤ len`.
fn top_k_desc<const N: usize>(fleet: &Fleet<N>, k: usize) -> Vec<f64> {
    let mut utils: Vec<f64> =
        fleet.iter().map(|(_, spec)| spec.worst_utilization()).collect();
    let k = k.min(utils.len());
    if k < utils.len() {
        // Partition so the `k` largest occupy `[0, k)` (unordered among
        // themselves), then keep and sort just those.
        utils.select_nth_unstable_by(k - 1, desc);
        utils.truncate(k);
    }
    utils.sort_by(desc);
    utils
}

impl<const K: usize, const N: usize> Gauge<N> for SumTopK<K, N> {
    fn eval(&self, fleet: &Fleet<N>) -> f64 {
        if fleet.is_empty() || K == 0 {
            return 0.0;
        }
        // Sum of the top-K worst-dim utilizations, in descending order — same
        // values, same order as the old full-sort-and-take, hence same bits.
        top_k_desc(fleet, K).iter().sum()
    }

    fn name(&self) -> &'static str {
        "SumTopK"
    }
}

impl<const K: usize, const N: usize> Gauge<N> for WeightedKyFan<K, N> {
    fn eval(&self, fleet: &Fleet<N>) -> f64 {
        if fleet.is_empty() || K == 0 {
            return 0.0;
        }
        // Top-K worst-dim utilizations, descending. The cumulative loop below
        // only ever reads indices `< K`, so the full sort was wasted work; this
        // is the same `utils[0..min(K, len)]` sequence, bit-for-bit.
        let utils = top_k_desc(fleet, K);

        // Σ_{k=1}^{K} weights[k-1] · (sum of top-k worst-dim utilizations).
        // `weights()` accessor — the private field isn't reachable from this
        // module, but construction already guaranteed non-negativity.
        let mut total = 0.0_f64;
        let mut cumulative = 0.0_f64;
        for (k, w) in self.weights().iter().enumerate() {
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
