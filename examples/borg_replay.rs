//! Borg trace replay through the typed framework.
//!
//! Reads a CSV slice of the Google 2019 Borg cluster trace, classifies each
//! event into the typed move alphabet, and reports the typed-pure ratio.
//!
//! Logic lives in `fulcrum::replay`; this binary is a CLI shell around it.
//!
//! Run with:
//!   cargo run --release --example borg_replay -- <path-to-csv>

use std::env;
use std::path::Path;
use std::process;

use fulcrum::replay::{classify_and_apply, parse_csv};
use fulcrum::Linfty;

const CAPACITY: u64 = 1_000_000;

fn main() {
    let path = match env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: cargo run --release --example borg_replay -- <path-to-csv>");
            process::exit(2);
        }
    };

    let path = Path::new(&path);
    eprintln!("Pass 1: parsing {} ...", path.display());
    let (events, dropped) = match parse_csv(path) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: {}", e);
            process::exit(1);
        }
    };
    eprintln!("  {} state-changing events, {} rows dropped", events.len(), dropped);

    eprintln!("Pass 2: classifying and applying through the framework ...");
    let result = match classify_and_apply::<Linfty<1>>(events, CAPACITY) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {:?}", e);
            process::exit(1);
        }
    };

    let c = &result.counters;

    println!();
    println!("=== Applied through framework (events the framework actually processed) ===");
    println!("  REMOVE       (typed-pure): {:>8}", c.remove);
    println!("  HotToCold    (typed-pure): {:>8}", c.hot_to_cold);
    println!("  Neutral      (typed-pure): {:>8}", c.neutral);
    println!("  --- typed-pure subtotal:   {:>8}", c.applied_typed_pure());
    println!();
    println!("  Place        (catch-all):  {:>8}", c.place);
    println!("  ColdToHot    (catch-all):  {:>8}", c.cold_to_hot);
    println!("  --- catch-all subtotal:    {:>8}", c.applied_catch_all());
    println!();
    println!("  total applied:             {:>8}", c.applied_total());
    println!(
        "  applied typed-pure ratio:  {:>8.2}%   <-- the framework's headline number",
        c.applied_typed_pure_ratio() * 100.0
    );

    println!();
    println!("=== Skipped or unobservable ===");
    println!(
        "  REMOVE for unseen instance: {:>8}  (instance scheduled before trace window;",
        c.remove_unobservable
    );
    println!("                                       framework cannot apply, but the");
    println!("                                       event itself is unambiguously");
    println!("                                       mass-decreasing)");
    println!(
        "  Well-formedness skipped:    {:>8}  (bookkeeping divergence; should be 0)",
        c.well_formedness_skipped
    );

    println!();
    println!("=== History summary ===");
    println!("  recorded moves:    {:>8}", result.history.len());
    println!(
        "  typed-pure ratio:  {:>8.2}%",
        result.history.typed_pure_ratio() * 100.0
    );
    println!("  final gauge:       {:.4}", result.safe.gauge());
}
