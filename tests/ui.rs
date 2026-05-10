//! Compile-fail tests: things the type system *must* reject.
//!
//! These are the strongest demonstration of the framework's lego-style
//! guarantee — bad code doesn't compile, period. Each `tests/ui/*.rs`
//! is a separate snippet; the corresponding `.stderr` file pins the
//! expected error.
//!
//! To regenerate `.stderr` files after rustc changes its diagnostics:
//!     TRYBUILD=overwrite cargo test --test ui

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
