//! The reification's payoff: `Capacity` and `Mass` are both `[u64; N]`
//! underneath, but branding them makes a load/capacity swap a *type* error
//! at the `add_machine` boundary, not a silent bug.

use fulcrum::{Capacity, Fleet, MachineId, Mass};

fn main() {
    let mut f: Fleet<1> = Fleet::new();
    // Arguments swapped: `Mass` where a `Capacity` is required, and
    // `Capacity` where a `Mass` is required. Must not compile.
    f.add_machine(MachineId(1), Mass([100]), Capacity([50]));
}
