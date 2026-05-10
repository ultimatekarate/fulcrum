//! Same attack on `Neutral`. Same outcome required.

use fulcrum::{MachineId, Mass, Neutral};

fn main() {
    let _forged = Neutral {
        source: MachineId(1),
        destination: MachineId(2),
        mass: Mass(10),
    };
}
