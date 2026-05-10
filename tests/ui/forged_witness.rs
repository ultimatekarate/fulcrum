//! Attempt to construct a `HotToCold` without going through `witness()`.
//!
//! This must fail to compile. The `_witness: WitnessToken` field is private,
//! and `WitnessToken` is `pub(crate)` — neither can be supplied from
//! outside the `fulcrum` crate. The only legal construction path is
//! `HotToCold::witness(src, dst, mass, fleet)`, which performs the
//! Pigou-Dalton check.

use fulcrum::{HotToCold, MachineId, Mass};

fn main() {
    let _forged: HotToCold<1> = HotToCold {
        source: MachineId(1),
        destination: MachineId(2),
        mass: Mass([10]),
    };
}
