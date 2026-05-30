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
use crate::load::Fleet;

impl<const K: usize, const N: usize> Gauge<N> for SumTopK<K, N> {
    fn eval(&self, fleet: &Fleet<N>) -> f64 {
        if fleet.is_empty() || K == 0 {
            return 0.0;
        }
        let mut utils: Vec<f64> = fleet.iter().map(|(_, spec)| spec.worst_utilization()).collect();
        utils.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        utils.iter().take(K).sum()
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
        let mut utils: Vec<f64> = fleet.iter().map(|(_, spec)| spec.worst_utilization()).collect();
        utils.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

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
