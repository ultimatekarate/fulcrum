//! Toy rebalancer demo.
//!
//! Builds a small fleet, applies a sequence of moves through the typed
//! framework, and prints the gauge value and typed-pure ratio at the end.
//!
//! Run with: `cargo run --example synthetic`

use fulcrum::{
    Fleet, HotToCold, Linfty, MachineId, Mass, MoveHistory, MoveRecord, Place, Remove, Safe,
};

fn main() {
    // Three machines, capacity 100 each, with a hot-spot at machine 1.
    let mut fleet = Fleet::new();
    fleet.add_machine(MachineId(1), 100, 80);
    fleet.add_machine(MachineId(2), 100, 30);
    fleet.add_machine(MachineId(3), 100, 30);

    let threshold = 0.85;
    let safe: Safe<Linfty> = Safe::new(fleet, threshold)
        .expect("starting fleet should be within threshold");

    println!("starting:  gauge = {:.3}, threshold = {:.3}", safe.gauge(), threshold);

    let mut history = MoveHistory::new();

    // Step 1: rebalance machine 1 -> machine 2 via Pigou-Dalton transfer.
    let m = HotToCold::witness(MachineId(1), MachineId(2), Mass(20), safe.fleet())
        .expect("witness should pass: 80 > 30, mass 20 ≤ 50");
    history.push(MoveRecord::HotToCold {
        source: MachineId(1),
        destination: MachineId(2),
        mass: Mass(20),
    });
    let safe = m.apply(safe); // total — no Result
    println!("after H→C: gauge = {:.3}", safe.gauge());

    // Step 2: free a slot on machine 1.
    let r = Remove::new(MachineId(1), Mass(10));
    history.push(MoveRecord::Remove { machine: MachineId(1), mass: Mass(10) });
    let safe = r.apply(safe); // total — no Result
    println!("after rem: gauge = {:.3}", safe.gauge());

    // Step 3: try a fresh placement that fits.
    // Note the asymmetric naming: catch-all apply is `apply_with_recheck`,
    // making every runtime-validated site grep-able by method name.
    let p = Place::new(MachineId(3), Mass(20));
    history.push(MoveRecord::Place { machine: MachineId(3), mass: Mass(20) });
    let safe = p.apply_with_recheck(safe).expect("place should fit");
    println!("after place: gauge = {:.3}", safe.gauge());

    // Step 4: try a fresh placement that would violate threshold.
    let p_bad = Place::new(MachineId(2), Mass(40));
    let result = p_bad.apply_with_recheck(safe);
    match result {
        Ok(_) => println!("UNEXPECTED: bad placement accepted"),
        Err(e) => println!("good: bad placement rejected — {:?}", e),
    }

    println!();
    println!(
        "history: {} moves, {:.0}% typed-pure",
        history.len(),
        history.typed_pure_ratio() * 100.0
    );
}
