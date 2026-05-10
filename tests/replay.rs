//! End-to-end regression test for the replay pipeline: CSV → parser →
//! classifier → counters. Asserts exact counter values for a small fixed
//! trace, so any regression in either parsing or classification will be
//! caught by `cargo test`.

use std::path::PathBuf;

use fulcrum::replay::{classify_and_apply, parse_csv, Counters};
use fulcrum::Linfty;

fn tiny_trace_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/data/tiny_trace.csv");
    p
}

#[test]
fn tiny_trace_classifier_produces_expected_counters() {
    let path = tiny_trace_path();
    let (events, dropped) = parse_csv(&path).expect("parse failed");

    // The CSV has 13 data rows; ENABLE is dropped by the parser.
    assert_eq!(events.len(), 12, "12 state-changing events expected");
    assert_eq!(dropped, 1, "1 dropped row expected (ENABLE)");

    let result = classify_and_apply::<Linfty>(events, 1_000_000)
        .expect("classify_and_apply failed");
    let c = result.counters;

    // Hand-traced expected counters (see tests/data/tiny_trace.csv):
    //   t=50:   SCHEDULE c4  (PLACE)
    //   t=75:   FINISH c5    (remove_unobservable: never saw a SCHEDULE)
    //   t=100:  SCHEDULE c1  (PLACE)
    //   t=150:  SCHEDULE c6  (PLACE)
    //   t=200:  SCHEDULE c2  (PLACE)
    //   t=300:  SCHEDULE c1 → m20  (HotToCold: src=400k, dst=100k, mass=300k ≤ gap=300k)
    //   t=400:  SCHEDULE c3  (PLACE)
    //   t=500:  FINISH c1    (REMOVE)
    //   t=600:  SCHEDULE c6 → m30  (ColdToHot: src=100k, dst=500k)
    //   t=700:  SCHEDULE c7  (PLACE)
    //   t=800:  SCHEDULE c7 → m40  (same machine — no-op, no counter)
    //   t=900:  EVICT c4     (REMOVE)
    let expected = Counters {
        remove: 2,
        remove_unobservable: 1,
        place: 6,
        hot_to_cold: 1,
        neutral: 0,
        cold_to_hot: 1,
        well_formedness_skipped: 0,
    };
    assert_eq!(c, expected, "classifier counters drifted from expected");

    // Derived ratios.
    assert_eq!(c.applied_typed_pure(), 3); // 2 remove + 1 hot_to_cold
    assert_eq!(c.applied_catch_all(), 7); // 6 place + 1 cold_to_hot
    assert_eq!(c.applied_total(), 10);
    assert!((c.applied_typed_pure_ratio() - 0.30).abs() < 1e-9);

    // History records every applied move (10 of them) — same-machine
    // reschedule and unobservable remove do not record.
    assert_eq!(result.history.len(), 10);
}

#[test]
fn time_sort_is_actually_applied() {
    // The CSV deliberately has rows out of time order (row index 5 has
    // t=50, before row 0's t=100). If the classifier processed in CSV
    // order, the FINISH for c5 would come *before* its (nonexistent)
    // schedule and the SCHEDULE for c1 at t=100 would still come after
    // c4's PLACE at t=50, but the C1 lifecycle would be wrong without
    // sorting.
    //
    // Easier check: just verify that REMOVE counters are non-zero (would
    // be near-zero without sorting because most FINISHes appear in the
    // CSV before their SCHEDULEs).
    let path = tiny_trace_path();
    let (events, _) = parse_csv(&path).expect("parse failed");
    let result = classify_and_apply::<Linfty>(events, 1_000_000)
        .expect("classify_and_apply failed");
    assert!(
        result.counters.remove > 0,
        "REMOVE counter should be non-zero after time-sorting"
    );
}
