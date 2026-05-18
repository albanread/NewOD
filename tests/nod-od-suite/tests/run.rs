//! Curated OpenDylan-flavoured fixtures, run end-to-end through the
//! NewOpenDylan JIT. Each test names a fixture under `fixtures/`,
//! compiles it, calls a designated entry point (typically `main`),
//! and asserts the i64 return value.
//!
//! These are the substitute for self-hosting that PLAN.md §2.7
//! commits to: every fixture is a small program a Dylan programmer
//! would recognise, expressed using only features the current
//! compiler implements. Each is small enough to debug by hand if it
//! regresses.

use std::path::{Path, PathBuf};

use serial_test::serial;

use nod_sema::run_function_to_i64;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn run_main(fixture: &str) -> i64 {
    let path = fixtures_dir().join(fixture);
    run_function_to_i64(&path, "main")
        .unwrap_or_else(|e| panic!("run {fixture}::main: {e:?}"))
}

/// Sprint 07-shape: pure recursion + branching + i64 arithmetic.
#[test]
#[serial]
fn fibonacci_10_is_55() {
    assert_eq!(run_main("fibonacci.dylan"), 55);
}

/// Sprint 07-shape: recursion with `mod`. Euclid's GCD on (48, 18).
#[test]
#[serial]
fn euclid_gcd_48_18_is_6() {
    assert_eq!(run_main("euclid-gcd.dylan"), 6);
}

/// Sprint 07-shape: mutual recursion across two `define function`s.
/// `is-even(8) = 1` after a 9-frame stack walk through is-odd/is-even.
#[test]
#[serial]
fn mutual_recursion_is_even_8() {
    assert_eq!(run_main("even-rec.dylan"), 1);
}

/// Sprint 12-shape: single-dispatch generic with two methods over a
/// shape hierarchy. `area(circle{radius=2}) + area(square{side=5})`
/// = 12 + 25 = 37.
#[test]
#[serial]
fn single_dispatch_over_shapes_sums_to_37() {
    assert_eq!(run_main("area-shapes.dylan"), 37);
}

/// Sprint 12-shape: inherited slot access through a CPL walk. A
/// <point-3d> reads its own `z` slot and the inherited `x` / `y`
/// slots. `1 + 2 + 3 = 6`.
#[test]
#[serial]
fn inherited_slot_access_sums_coords() {
    assert_eq!(run_main("point-3d-sum.dylan"), 6);
}
