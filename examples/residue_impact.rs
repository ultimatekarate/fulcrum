//! Is the absolute-memory residue worth caring about?
//!
//! The residue is "stuck" — `MaxMinFairGreedy` cannot shed it with typed-pure
//! moves. But the fleet is always below the threshold τ by construction, so
//! "stuck" might be entirely benign. This measures the *operational* weight of
//! the residue, per class:
//!
//!   * safety headroom  τ − gauge  (how close to the load limit is the stuck
//!     fleet?),
//!   * absolute free capacity on the stuck dimension (is anyone actually low?),
//!   * admission slack (can the fleet still place another max-size workload?).
//!
//! And it tests the key reframe for memory: because the "hot" big node (high
//! *utilization*) carries the same *absolute* free MB as the "cool" small node,
//! the utilization gauge massively overstates the real memory imbalance.
//!
//! Run: `cargo run --release --example residue_impact`

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

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

#[derive(Default)]
struct Class {
    headroom_tau: Vec<f64>,       // τ − gauge
    admit_max: u64,               // can place a max-size workload somewhere
    n: u64,
    // memory-specific (only filled for the memory class):
    mem_util_spread: Vec<f64>,    // max−min memory utilization (the gauge's view)
    mem_free_spread: Vec<f64>,    // max−min absolute free MB (the real view)
    mem_min_free_mb: Vec<f64>,    // scarcest absolute free MB on memory
    mem_min_free_in_wl: Vec<f64>, // scarcest free as multiples of a max mem workload
    hot_minus_cool_free: Vec<f64>,// free MB of util-hottest node − util-coolest node
}

impl Class {
    fn report(&mut self, name: &str) {
        if self.n == 0 {
            println!("  [{name}] no trials");
            return;
        }
        println!("  [{name}]  n = {}", self.n);
        println!("     median safety headroom (τ − gauge): {:.3}", median(&mut self.headroom_tau));
        println!(
            "     can still admit a MAX-size workload:  {}/{} ({:.0}%)",
            self.admit_max,
            self.n,
            100.0 * self.admit_max as f64 / self.n as f64
        );
        if !self.mem_util_spread.is_empty() {
            println!(
                "     memory utilization spread (gauge view):   {:.3}",
                median(&mut self.mem_util_spread)
            );
            println!(
                "     memory ABSOLUTE free spread (real view):   {:.0} MB",
                median(&mut self.mem_free_spread)
            );
            println!(
                "     scarcest absolute free memory:             {:.0} MB  (= {:.1}x a max workload)",
                median(&mut self.mem_min_free_mb),
                median(&mut self.mem_min_free_in_wl)
            );
            println!(
                "     free(util-hottest) − free(util-coolest):   {:+.0} MB  (≈0 ⇒ util misranks them)",
                median(&mut self.hot_minus_cool_free)
            );
        }
        println!();
    }
}

fn analyze(config: TwinConfig, memc: &mut Class, elac: &mut Class, balc: &mut Class) {
    let o = greedy_outcome(config).unwrap();
    let tau = o.threshold;
    let involves_mem = o.residual_guard_blocked.iter().any(|(_, _, d)| *d == MEM);
    let has_residue = !o.residual_guard_blocked.is_empty();

    // Admission: can a max-size workload land on SOME node under τ?
    let w = config.max_mass;
    let admit = o.specs.iter().any(|(_, s)| {
        (0..4).all(|d| {
            let cap = s.capacity[d] as f64;
            cap > 0.0 && (s.load[d] + w[d]) as f64 / cap <= tau
        })
    });

    let class = if !has_residue {
        &mut *balc
    } else if involves_mem {
        &mut *memc
    } else {
        &mut *elac
    };
    class.n += 1;
    class.headroom_tau.push(tau - o.gauge);
    if admit {
        class.admit_max += 1;
    }

    // Memory-specific absolute-vs-utilization view.
    if involves_mem {
        let mut utils: Vec<f64> = Vec::new();
        let mut frees: Vec<f64> = Vec::new();
        for (_, s) in &o.specs {
            let cap = s.capacity[MEM] as f64;
            utils.push(s.load[MEM] as f64 / cap);
            frees.push((s.capacity[MEM].saturating_sub(s.load[MEM])) as f64);
        }
        let umax = utils.iter().cloned().fold(f64::MIN, f64::max);
        let umin = utils.iter().cloned().fold(f64::MAX, f64::min);
        let fmax = frees.iter().cloned().fold(f64::MIN, f64::max);
        let fmin = frees.iter().cloned().fold(f64::MAX, f64::min);
        memc.mem_util_spread.push(umax - umin);
        memc.mem_free_spread.push(fmax - fmin);
        memc.mem_min_free_mb.push(fmin);
        let max_wl = config.max_mass[MEM].max(1) as f64;
        memc.mem_min_free_in_wl.push(fmin / max_wl);
        // free of the util-hottest vs util-coolest node.
        let hot_i = utils
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
            .0;
        let cool_i = utils
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
            .0;
        memc.hot_minus_cool_free.push(frees[hot_i] - frees[cool_i]);
    }
}

fn main() {
    let mut memc = Class::default();
    let mut elac = Class::default();
    let mut balc = Class::default();

    // Broad randomized sweep — same generator family as residue_sweep.
    let mut rng = Rng::new(0x5EED_5EED);
    for _ in 0..4000 {
        let max_mass = [
            rng.range(20, 200),
            rng.range(200, 2400),
            rng.range(20, 180),
            rng.range(50, 400),
        ];
        let n = rng.range(16, 34) as usize;
        analyze(cfg(rng.next(), n, max_mass), &mut memc, &mut elac, &mut balc);
    }

    println!("== Operational weight of the post-greedy residue (4000 random workloads) ==\n");
    memc.report("MEMORY residue");
    elac.report("ELASTIC residue (Cpu/DiskIo)");
    balc.report("BALANCED (no residue)");

    println!("Reading:");
    println!("  If MEMORY-residue trials keep large τ-headroom, still admit max workloads,");
    println!("  and show ABSOLUTE free spread ≈ 0 despite a large utilization spread, then the");
    println!("  memory residue is a heterogeneity artifact of the utilization gauge — not a");
    println!("  real imbalance, and not worth a carrier to 'fix'.");
}
