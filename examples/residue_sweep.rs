//! Residue sweep — trying to break the carrier claim.
//!
//! The comparison experiment (`examples/rebalance_compare.rs`) found that on two
//! canonical workloads, the capacity-guard residue left *after* the multi-pair
//! `MaxMinFairGreedy` exhausts planner coverage always sits on a semantically
//! **elastic** dimension (DiskIo / Cpu), never on absolute **memory**. Two data
//! points is not a structural law. This sweep tries hard to falsify it:
//!
//!   * Sweep 1 — many seeds at each canonical profile: is the per-profile
//!     finding stable across workload *instances*, or cherry-picked?
//!   * Sweep 2 — a large *randomized-profile* sweep: across every demand mix,
//!     does the residue EVER land on memory? (The falsifier.)
//!
//! Deterministic: the sweep's own randomness is a seeded xorshift, so the whole
//! run is reproducible.
//!
//! Run: `cargo run --release --example residue_sweep`

use fulcrum::{compare_rebalancers, Power, PowerBudget, TwinConfig};

const DIM: [&str; 4] = ["Cpu", "Mem", "DiskIo", "NetIo"];
const MEM: usize = 1;

/// Tiny deterministic xorshift64 for generating the sweep's own configs.
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

#[derive(Default)]
struct Tally {
    trials: u64,
    /// Trials where greedy fully balanced (no guard-blocked residue).
    empty_residue: u64,
    /// Per-dimension count of guard-blocked residue pairs (summed over trials).
    residue_pairs_by_dim: [u64; 4],
    /// Trials whose residue touches a given dimension at all.
    trials_touching_dim: [u64; 4],
    /// Trials where greedy made strictly more migrations than the baseline.
    greedy_strictly_more: u64,
    /// Sum of (greedy_ratio - baseline_ratio), for the mean lift.
    ratio_lift_sum: f64,
    /// Largest residue (pairs) seen in any single trial.
    max_residue_pairs: usize,
}

impl Tally {
    fn record(&mut self, config: TwinConfig) {
        let c = compare_rebalancers(config).expect("empty fleet within threshold");
        self.trials += 1;
        self.ratio_lift_sum += c.greedy_ratio - c.baseline_ratio;
        if c.greedy_migrations > c.baseline_migrations {
            self.greedy_strictly_more += 1;
        }
        let residue = &c.greedy_residual_guard_blocked;
        self.max_residue_pairs = self.max_residue_pairs.max(residue.len());
        if residue.is_empty() {
            self.empty_residue += 1;
        }
        let mut touched = [false; 4];
        for (_, _, d) in residue {
            self.residue_pairs_by_dim[*d] += 1;
            touched[*d] = true;
        }
        for d in 0..4 {
            if touched[d] {
                self.trials_touching_dim[d] += 1;
            }
        }
    }

    fn report(&self, title: &str) {
        println!("== {title} ==");
        println!("  trials:                 {}", self.trials);
        println!(
            "  greedy > baseline migs: {} ({:.0}%)",
            self.greedy_strictly_more,
            100.0 * self.greedy_strictly_more as f64 / self.trials as f64
        );
        println!(
            "  mean typed-pure lift:   {:+.4}",
            self.ratio_lift_sum / self.trials as f64
        );
        println!(
            "  fully balanced (empty): {} ({:.0}%)",
            self.empty_residue,
            100.0 * self.empty_residue as f64 / self.trials as f64
        );
        println!("  largest residue:        {} pairs", self.max_residue_pairs);
        println!("  residue touches dim, by trial count:");
        for d in 0..4 {
            let flag = if d == MEM && self.trials_touching_dim[d] > 0 {
                "   <-- MEMORY (absolute) appeared in residue!"
            } else {
                ""
            };
            println!(
                "      {:<7}: {:>5} trials   ({} residue-pairs total){}",
                DIM[d], self.trials_touching_dim[d], self.residue_pairs_by_dim[d], flag
            );
        }
        if self.trials_touching_dim[MEM] == 0 {
            println!("  => memory NEVER appears in the residue. Carrier claim survives this sweep.");
        } else {
            println!("  => memory DOES appear: the elastic carrier alone cannot clear all residue.");
        }
        println!();
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

fn main() {
    // --- Sweep 1: many seeds at each canonical profile ---
    let mut headline = Tally::default();
    let mut memheavy = Tally::default();
    for seed in 1..=500u64 {
        headline.record(cfg(seed, 24, [120, 1200, 150, 150]));
        memheavy.record(cfg(seed, 30, [150, 1500, 150, 150]));
    }
    headline.report("Sweep 1a: headline profile, seeds 1..=500");
    memheavy.report("Sweep 1b: memory-heavy profile, seeds 1..=500");

    // --- Sweep 2: randomized demand profiles ---
    // Per-dim max demand drawn over wide ranges (roughly scaled so each dim CAN
    // saturate the small nodes), with random workload count. If memory residue
    // is reachable at all, a memory-dominant draw should surface it.
    let mut rnd = Tally::default();
    let mut gen = Rng::new(0x5EED_5EED);
    for _ in 0..4000 {
        let max_mass = [
            gen.range(20, 200),   // Cpu   (small cap 600)
            gen.range(200, 2400), // Mem   (small cap 4096)
            gen.range(20, 180),   // DiskIo(small cap 200)
            gen.range(50, 400),   // NetIo (small cap 1000)
        ];
        let n = gen.range(16, 34) as usize;
        let seed = gen.next();
        rnd.record(cfg(seed, n, max_mass));
    }
    rnd.report("Sweep 2: 4000 randomized demand profiles");

    // --- Targeted attack: memory-dominant profiles only ---
    // Deliberately starve every other dimension and pile on memory, to give the
    // residue the best possible chance of stranding on absolute memory.
    let mut attack = Tally::default();
    let mut gen2 = Rng::new(0xA77AC4);
    for _ in 0..2000 {
        let max_mass = [
            gen2.range(5, 40),     // Cpu starved
            gen2.range(1500, 3200),// Mem dominant
            gen2.range(5, 30),     // DiskIo starved
            gen2.range(20, 120),   // NetIo starved
        ];
        let n = gen2.range(16, 34) as usize;
        let seed = gen2.next();
        attack.record(cfg(seed, n, max_mass));
    }
    attack.report("Sweep 3: 2000 memory-DOMINANT profiles (adversarial)");
}
