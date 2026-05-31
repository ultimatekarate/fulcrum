//! Locks the residue sweep (see `examples/residue_sweep.rs`), which **falsified**
//! the clean two-seed claim that the post-greedy carrier residue is always on an
//! elastic dimension.
//!
//! ## Corrected findings
//!
//! 1. **Memory residue is common, not absent.** Across seeds at the headline
//!    profile it appears ~27% of the time; at a memory-heavy profile ~74%; under
//!    adversarial memory-dominant demand >90%. The single cherry-picked seeds in
//!    `rebalance_compare` (DiskIo-only / Cpu-only residue) are NOT representative.
//!    So the elastic carrier alone does **not** clear all residue — a large slice
//!    is irreducible absolute-memory imbalance (where the guard is *correct*).
//!
//! 2. **Equal-capacity dimensions never strand.** NetIo capacity is identical on
//!    every node, so a big<->small Net transfer is never capacity-guard blocked;
//!    NetIo therefore never appears in the residue. The whole phenomenon is a
//!    *heterogeneous-capacity* artifact: residue only forms on dimensions where
//!    the big nodes have strictly more capacity than the small ones.
//!
//! 3. **The multi-pair planner often suffices outright.** On ~40% of random
//!    workloads greedy fully balances (empty residue) — no carrier opportunity at
//!    all. The residue, when present, is the 2x2 big->small pattern (<= 4 pairs).
//!
//! Net: the cheap, already-built planner fix is the robust win; the elastic
//! carrier addresses a real but *minority* slice (Cpu/DiskIo residue), and most
//! residue is either cleared by the planner or legitimately stuck on memory.

use fulcrum::{compare_rebalancers, Power, PowerBudget, ResourceDim, TwinConfig};

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

#[derive(Default)]
struct Counts {
    trials: u64,
    empty: u64,
    touches: [u64; 4],
    residue_pairs: [u64; 4],
    max_pairs: usize,
}

fn run(configs: impl Iterator<Item = TwinConfig>) -> Counts {
    let mut c = Counts::default();
    for config in configs {
        let r = compare_rebalancers(config).unwrap();
        c.trials += 1;
        let res = &r.greedy_residual_guard_blocked;
        c.max_pairs = c.max_pairs.max(res.len());
        if res.is_empty() {
            c.empty += 1;
        }
        let mut touched = [false; 4];
        for (_, _, d) in res {
            c.residue_pairs[*d] += 1;
            touched[*d] = true;
        }
        for d in 0..4 {
            if touched[d] {
                c.touches[d] += 1;
            }
        }
    }
    c
}

/// The structural invariant: an equal-capacity dimension (NetIo) can never be
/// capacity-guard blocked, so it never appears in the residue. Holds across
/// every workload we throw at it.
#[test]
fn equal_capacity_dimension_never_strands() {
    let net = ResourceDim::NetIo.index();
    let mut rng = Rng::new(0xBADC0DE);
    let c = run((0..800).map(|_| {
        let max_mass = [
            rng.range(20, 200),
            rng.range(200, 2400),
            rng.range(20, 180),
            rng.range(50, 400),
        ];
        let n = rng.range(16, 34) as usize;
        cfg(rng.next(), n, max_mass)
    }));
    assert_eq!(
        c.residue_pairs[net], 0,
        "NetIo has equal capacity everywhere; it must never strand residue"
    );
    // The residue, when present, is the 2x2 big->small pattern.
    assert!(c.max_pairs <= 4, "residue should be at most 4 pairs, saw {}", c.max_pairs);
}

/// The falsification: memory (absolute) DOES appear in the residue — frequently —
/// so the elastic carrier alone cannot clear all of it. The cherry-picked
/// `rebalance_compare` seeds were not representative.
#[test]
fn memory_residue_is_common_not_absent() {
    let mem = ResourceDim::Mem.index();

    // At the headline profile, across seeds, memory strands a meaningful share.
    let headline = run((1..=400u64).map(|s| cfg(s, 24, [120, 1200, 150, 150])));
    assert!(
        headline.touches[mem] > 0,
        "memory must appear in the residue at the headline profile across seeds \
         (it did not, which would resurrect the false 'always elastic' claim)"
    );

    // Under memory-heavy demand it should be the dominant residue dimension.
    let memheavy = run((1..=400u64).map(|s| cfg(s, 30, [150, 1500, 150, 150])));
    assert!(
        memheavy.touches[mem] as f64 > 0.5 * memheavy.trials as f64,
        "memory should dominate the residue under memory-heavy demand: {}/{}",
        memheavy.touches[mem],
        memheavy.trials
    );
}

/// Adversarial memory-dominant demand strands memory in the vast majority of
/// trials, and never strands the starved elastic dimensions.
#[test]
fn adversarial_memory_pressure_strands_memory() {
    let mem = ResourceDim::Mem.index();
    let net = ResourceDim::NetIo.index();
    let mut rng = Rng::new(0xA77AC4);
    let c = run((0..600).map(|_| {
        let max_mass = [
            rng.range(5, 40),
            rng.range(1500, 3200),
            rng.range(5, 30),
            rng.range(20, 120),
        ];
        let n = rng.range(16, 34) as usize;
        cfg(rng.next(), n, max_mass)
    }));
    assert!(
        c.touches[mem] as f64 > 0.8 * c.trials as f64,
        "memory-dominant demand should strand memory in >80% of trials: {}/{}",
        c.touches[mem],
        c.trials
    );
    assert_eq!(c.residue_pairs[net], 0, "NetIo still never strands");
}

/// The robust win: on a broad random sweep, the multi-pair planner fully
/// balances a substantial fraction of workloads on its own (no carrier needed).
#[test]
fn greedy_fully_balances_a_substantial_fraction() {
    let mut rng = Rng::new(0x5EED_5EED);
    let c = run((0..1000).map(|_| {
        let max_mass = [
            rng.range(20, 200),
            rng.range(200, 2400),
            rng.range(20, 180),
            rng.range(50, 400),
        ];
        let n = rng.range(16, 34) as usize;
        cfg(rng.next(), n, max_mass)
    }));
    assert!(
        c.empty as f64 > 0.25 * c.trials as f64,
        "greedy should fully balance a substantial fraction: {}/{} empty",
        c.empty,
        c.trials
    );
}
