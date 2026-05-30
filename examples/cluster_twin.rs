//! Turing Pi 2 cluster digital twin.
//!
//! Builds the heterogeneous four-slot topology (2× Pi 5, 2× Pi 4),
//! generates a deterministically-seeded stream of 4-dimensional workload
//! demands (CPU / memory / disk IO / net IO), places them least-loaded
//! under a shared power budget, then rebalances with max-min fair
//! Pigou-Dalton transfers. Prints the resulting timeline.
//!
//! Run with: `cargo run --example cluster_twin`

use fulcrum::{
    run_turing_pi_2_twin, timeline_to_csv, turing_pi_2, Power, PowerBudget, ResourceDim, TwinConfig,
};

fn main() {
    let budget = PowerBudget(Power(32_000.0)); // 32 W shared ceiling
    let config = TwinConfig {
        seed: 0xC0FFEE,
        n_workloads: 24,
        // Max demand per dimension: [Cpu, Mem(MiB), DiskIo, NetIo].
        max_mass: [120, 1200, 150, 150],
        threshold: 0.95,
        budget,
        rebalance_epsilon: 1e-3,
    };

    let topo = turing_pi_2();
    println!("Topology: {} nodes (Turing Pi 2)", topo.len());
    for (id, profile) in topo.nodes() {
        println!(
            "  {:?}: cap[cpu={}, mem={}, disk={}, net={}]  idle={:.1}W full={:.1}W",
            id,
            profile.capacity[ResourceDim::Cpu.index()],
            profile.capacity[ResourceDim::Mem.index()],
            profile.capacity[ResourceDim::DiskIo.index()],
            profile.capacity[ResourceDim::NetIo.index()],
            profile.power.idle.milliwatts() / 1000.0,
            (profile.power.idle.milliwatts() + profile.power.dynamic.milliwatts()) / 1000.0,
        );
    }
    println!("Budget: {:.1} W\n", budget.ceiling().milliwatts() / 1000.0);

    let report = run_turing_pi_2_twin(config).expect("empty starting fleet is within threshold");

    println!(
        "{:>4}  {:<10}  {:>7}  {:>9}  {:>10}  per-node worst-util",
        "step", "kind", "gauge", "power(W)", "typedpure"
    );
    for row in &report.timeline {
        let nodes: Vec<String> = row
            .per_node
            .iter()
            .map(|(id, u)| format!("{}:{:.2}", id.0, u))
            .collect();
        println!(
            "{:>4}  {:<10}  {:>7.3}  {:>9.2}  {:>10.2}  [{}]",
            row.step,
            row.kind,
            row.gauge,
            row.power.milliwatts() / 1000.0,
            row.typed_pure_ratio,
            nodes.join(" "),
        );
    }

    let s = report.stats;
    println!();
    println!(
        "placed={} power_rejected={} load_rejected={} malformed={} migrations={}",
        s.placed, s.power_rejected, s.load_rejected, s.malformed, s.typed_pure_applied,
    );
    println!(
        "final: gauge={:.3}  power={:.2} W  typed-pure ratio={:.2}",
        report.final_gauge,
        report.final_power.milliwatts() / 1000.0,
        report.typed_pure_ratio,
    );

    // Machine-readable timeline (uncomment to pipe to a plotter):
    let _csv = timeline_to_csv(&report.timeline);
    // print!("{_csv}");
}
