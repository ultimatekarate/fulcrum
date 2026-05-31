//! Locks the residue-impact experiment (see `examples/residue_impact.rs`):
//! **is the absolute-memory residue worth caring about?** Measured over a broad
//! random sweep via `greedy_outcome` (the post-greedy stuck fleet).
//!
//! ## Verdict: no — and "fixing" it would be counterproductive.
//!
//! 1. **Safety**: memory-residue trials keep a real buffer below τ (median
//!    headroom ~0.16; worst node ~0.79 of 0.95). Not on the edge.
//! 2. **Magnitude**: the memory utilization imbalance is small to begin with
//!    (median spread ~0.09).
//! 3. **The decisive fact**: on memory the util-*hottest* node carries *more*
//!    absolute free MB than the util-*coolest* (median ~+425 MB). The
//!    utilization gauge inverts the real (absolute) scarcity ranking. So
//!    equalizing memory *utilization* — exactly what an elastic memory carrier
//!    would unlock — moves load toward the absolutely-scarcer node, lowering the
//!    minimum free memory and worsening the OOM margin. The capacity guard that
//!    blocks the move is operationally *right*.
//!
//! Conclusion: for an absolute (quantity) resource, utilization is the wrong
//! cost metric below τ; the memory residue is a heterogeneity artifact of the
//! gauge, not an imbalance to fix. The carrier's value is confined to elastic
//! dimensions, where utilization genuinely tracks contention.

use fulcrum::{greedy_outcome, Power, PowerBudget, TwinConfig};

const MEM: usize = 1;

struct Rng(u64);
impl Rng {
    fn new(s: u64) -> Self {
        Rng(s.max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
}

fn cfg(seed: u64, n: usize, max_mass: [u64; 4]) -> TwinConfig {
    TwinConfig {
        seed,
        n_workloads: n,
        max_mass,
        threshold: 0.95,
        budget: PowerBudget(Power(32_000.0)),
        rebalance_epsilon: 1e-3,
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    assert!(!v.is_empty());
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

struct MemStats {
    n: usize,
    headroom: Vec<f64>,
    util_spread: Vec<f64>,
    hot_minus_cool_free: Vec<f64>,
    min_free_mb: Vec<f64>,
    elastic_n: usize,
    elastic_headroom: Vec<f64>,
}

fn sweep(trials: usize) -> MemStats {
    let mut rng = Rng::new(0x5EED_5EED);
    let mut s = MemStats {
        n: 0,
        headroom: Vec::new(),
        util_spread: Vec::new(),
        hot_minus_cool_free: Vec::new(),
        min_free_mb: Vec::new(),
        elastic_n: 0,
        elastic_headroom: Vec::new(),
    };
    for _ in 0..trials {
        let max_mass = [
            rng.range(20, 200),
            rng.range(200, 2400),
            rng.range(20, 180),
            rng.range(50, 400),
        ];
        let n = rng.range(16, 34) as usize;
        let o = greedy_outcome(cfg(rng.next(), n, max_mass)).unwrap();
        if o.residual_guard_blocked.is_empty() {
            continue;
        }
        let involves_mem = o.residual_guard_blocked.iter().any(|(_, _, d)| *d == MEM);
        if !involves_mem {
            s.elastic_n += 1;
            s.elastic_headroom.push(o.threshold - o.gauge);
            continue;
        }
        s.n += 1;
        s.headroom.push(o.threshold - o.gauge);

        let mut utils = Vec::new();
        let mut frees = Vec::new();
        for (_, spec) in &o.specs {
            let cap = spec.capacity[MEM] as f64;
            utils.push(spec.load[MEM] as f64 / cap);
            frees.push(spec.capacity[MEM].saturating_sub(spec.load[MEM]) as f64);
        }
        let umax = utils.iter().cloned().fold(f64::MIN, f64::max);
        let umin = utils.iter().cloned().fold(f64::MAX, f64::min);
        s.util_spread.push(umax - umin);
        s.min_free_mb.push(frees.iter().cloned().fold(f64::MAX, f64::min));
        let hot = utils
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
            .0;
        let cool = utils
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
            .0;
        s.hot_minus_cool_free.push(frees[hot] - frees[cool]);
    }
    s
}

#[test]
fn memory_residue_is_not_worth_caring_about() {
    let s = sweep(2000);
    assert!(s.n > 150, "need a decent memory-residue sample; got {}", s.n);

    // 1. Safety: a real buffer below τ.
    let headroom = median(s.headroom);
    assert!(headroom > 0.08, "memory residue should keep τ-headroom; got {headroom:.3}");

    // 2. The imbalance the gauge sees is modest.
    let spread = median(s.util_spread);
    assert!(spread < 0.25, "memory util spread should be modest; got {spread:.3}");

    // 3. Nobody is actually starving for absolute memory.
    let min_free = median(s.min_free_mb);
    assert!(min_free > 500.0, "scarcest free memory should be ample; got {min_free:.0} MB");
}

/// The decisive fact: on memory the utilization gauge inverts absolute scarcity.
/// The util-hottest node has MORE absolute free memory than the util-coolest, so
/// equalizing memory utilization would move load toward the scarcer node — the
/// guard that blocks it is operationally correct, and an elastic memory carrier
/// would be actively harmful.
#[test]
fn equalizing_memory_utilization_would_worsen_absolute_headroom() {
    let s = sweep(2000);
    let hot_minus_cool = median(s.hot_minus_cool_free);
    assert!(
        hot_minus_cool > 50.0,
        "util-hottest memory node should hold MORE absolute free than util-coolest \
         (got {hot_minus_cool:+.0} MB); a value >0 means equalizing util moves load \
         toward the scarcer node"
    );
}

/// Context: the elastic residue — where the carrier's value actually lives — is
/// also not a safety problem (even more headroom), so the carrier is an
/// efficiency/contention optimization, not a capacity or safety fix.
#[test]
fn elastic_residue_also_has_headroom() {
    let s = sweep(2000);
    assert!(s.elastic_n > 50, "need an elastic-residue sample; got {}", s.elastic_n);
    let h = median(s.elastic_headroom);
    assert!(h > 0.10, "elastic residue should also sit well below τ; got {h:.3}");
}
