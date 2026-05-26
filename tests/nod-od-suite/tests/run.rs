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
use nod_runtime;

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

/// GC allocation loop: 1000 <box> objects allocated inside a while
/// loop; slot read from each before it dies.  Sum must be 500500.
/// Exercises allocation + slot reads under repeated object churn.
#[test]
#[serial]
fn gc_alloc_loop_1000_boxes() {
    assert_eq!(run_main("gc-alloc-loop.dylan"), 500500);
}

/// Rope buffer loaded from a real 86 296-byte file on disk.  Exercises
/// every rope op on real data: size, element, line-count,
/// line-to-offset, offset-to-line, for-each-leaf, rope-substring,
/// rope-concatenate, rope-split-at, rope-insert, rope-delete,
/// rope->string.  Returns rope-line-count = 2221 iff all assertions
/// pass; returns 0 on any failure.
///
/// The Dylan fixture loops 150 times (load → all-ops → discard) to
/// build up GC pressure.  After the run we force a full collection so
/// the report shows what the GC actually reclaimed.
#[test]
#[serial]
fn gc_rope_file_load_all_ops() {
    let gc_before    = nod_runtime::gc_metrics_snapshot();
    let young_before = nod_runtime::with_literal_pool(|p| p.heap.young_used_bytes());
    let old_before   = nod_runtime::with_literal_pool(|p| p.heap.old_used_bytes());

    let result = run_main("gc-rope-file-load.dylan");

    let young_after_run  = nod_runtime::with_literal_pool(|p| p.heap.young_used_bytes());
    let old_after_run    = nod_runtime::with_literal_pool(|p| p.heap.old_used_bytes());

    // Force a full collection so the shadow metrics reflect what was reclaimed.
    nod_runtime::with_literal_pool(|p| p.heap.collect_full());

    let gc_after     = nod_runtime::gc_metrics_snapshot();
    let young_after_gc = nod_runtime::with_literal_pool(|p| p.heap.young_used_bytes());
    let old_after_gc   = nod_runtime::with_literal_pool(|p| p.heap.old_used_bytes());

    let minor_delta = gc_after.minor_collections - gc_before.minor_collections;
    let major_delta = gc_after.major_collections - gc_before.major_collections;
    let prom_delta  = gc_after.bytes_promoted.saturating_sub(gc_before.bytes_promoted);

    println!("\n=== GC activity: gc-rope-file-load (150 passes) ===");
    println!("  ── heap before run ──────────────────────────");
    println!("    young used   : {} bytes", young_before);
    println!("    old used     : {} bytes", old_before);
    println!("  ── heap after run (before forced GC) ────────");
    println!("    young used   : {} bytes  (+{} bytes garbage)",
        young_after_run,
        young_after_run.saturating_sub(young_before));
    println!("    old used     : {} bytes", old_after_run);
    println!("  ── after forced full GC ─────────────────────");
    println!("    young used   : {} bytes", young_after_gc);
    println!("    old used     : {} bytes", old_after_gc);
    println!("    reclaimed    : {} bytes",
        (young_after_run + old_after_run)
            .saturating_sub(young_after_gc + old_after_gc));
    println!("  ── GC counters (delta over full test) ───────");
    println!("    minor collections  : +{}", minor_delta);
    println!("    major collections  : +{}", major_delta);
    println!("    bytes promoted     : +{} bytes", prom_delta);
    if gc_after.last_major_pause_ns > gc_before.last_major_pause_ns || major_delta > 0 {
        println!("    last major pause   : {} us", gc_after.last_major_pause_ns / 1_000);
        println!("    roots at last major: {}", gc_after.roots_at_last_major);
    }
    println!("====================================================");

    assert_eq!(result, 2221);
}
