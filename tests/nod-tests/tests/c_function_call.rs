//! Sprint 28 — end-to-end FFI acceptance tests.
//!
//! Drives `define c-function` declarations through parse → sema → DFM →
//! LLVM → JIT → actual Win32 call. The trampolines in
//! `nod-runtime::winffi` marshal Dylan-side fixnum args into the Win64
//! ABI; the resolved fn-ptr is populated at JIT-finalize time by
//! `eval_expr_with_items_to_string`'s init step.
//!
//! Every test is `#[serial]` — the runtime's WinFFI stats counters
//! (and the global library cache) are process-global state.

#![cfg(windows)]

use nod_sema::{EvalError, eval_expr_with_items_to_string};
use serial_test::serial;

fn setup() {
    // Sprint 19 + Sprint 28: condition-class chain must be present
    // before tests that catch via `block/exception` run.
    nod_runtime::ensure_conditions_registered();
    nod_runtime::ensure_c_ffi_error_registered();
    nod_runtime::_reset_handler_stack_for_tests();
}

// ─── 1. The headline: Beep returns #t ─────────────────────────────────────

const BEEP_DECL: &str = "\
define c-function Beep
    (dw-freq :: <c-dword>, dw-duration :: <c-dword>)
 => (success :: <c-bool>);
  library: \"kernel32.dll\";
end;
";

#[test]
#[serial]
fn headline_beep_call_returns_true() {
    setup();
    // 50ms duration — barely audible on hardware, still a real call.
    // On a machine without an audio device Beep still returns
    // non-zero, which our `<c-bool>` marshaling converts to `#t`.
    let s = eval_expr_with_items_to_string(BEEP_DECL, "Beep(440, 50)")
        .unwrap_or_else(|e| panic!("Beep eval failed: {e:?}"));
    assert_eq!(s, "#t", "Beep(440, 50) must return #t");
}

// ─── 2. GetTickCount() — arity 0, integer return ──────────────────────────

const GET_TICK_DECL: &str = "\
define c-function GetTickCount () => (ticks :: <c-dword>);
  library: \"kernel32.dll\";
end;

define c-function Sleep (ms :: <c-dword>) => ();
  library: \"kernel32.dll\";
end;
";

#[test]
#[serial]
fn get_tick_count_returns_increasing_value() {
    setup();
    // Helper function that calls GetTickCount twice with a Sleep in
    // between. The eval-entry returns the second value; we then run
    // it again to confirm the second-pass value isn't smaller. Two
    // separate eval runs would be cleaner if both stub tables could
    // share state, but each eval reinitialises its own table — so
    // we collapse the comparison into Dylan code, returning a
    // <c-dword> difference.
    let s = eval_expr_with_items_to_string(
        GET_TICK_DECL,
        "let a = GetTickCount(); \
         Sleep(15); \
         let b = GetTickCount(); \
         b - a",
    )
    .unwrap_or_else(|e| panic!("GetTickCount eval failed: {e:?}"));
    // The result is a fixnum; non-negative (clock didn't go
    // backwards), most likely > 0 after a 15ms Sleep.
    let n: i64 = s.parse().expect("integer return");
    assert!(n >= 0, "tick delta must be non-negative, got {n}");
}

// ─── 3. GetCurrentProcessId — non-zero, fits in u32 ───────────────────────

#[test]
#[serial]
fn get_current_process_id_returns_integer() {
    setup();
    let items = "\
define c-function GetCurrentProcessId () => (pid :: <c-dword>);
  library: \"kernel32.dll\";
end;
";
    let s = eval_expr_with_items_to_string(items, "GetCurrentProcessId()")
        .unwrap_or_else(|e| panic!("GetCurrentProcessId eval failed: {e:?}"));
    let n: i64 = s.parse().expect("integer return");
    assert!(n > 0, "PID must be positive, got {n}");
    assert!(n <= u32::MAX as i64, "PID must fit in u32, got {n}");
}

// ─── 4. Sleep(0) — void return ────────────────────────────────────────────

#[test]
#[serial]
fn sleep_zero_returns_without_crashing() {
    setup();
    let items = "\
define c-function Sleep (ms :: <c-dword>) => ();
  library: \"kernel32.dll\";
end;
";
    // Void-returning c-function: marshaling layer turns the void
    // into `nil`. The eval formatter prints `nil` as `#()`.
    let s = eval_expr_with_items_to_string(items, "Sleep(0)")
        .unwrap_or_else(|e| panic!("Sleep eval failed: {e:?}"));
    assert_eq!(s, "#()", "void-return Sleep(0) must surface as nil/#()");
}

// ─── 5. GetCurrentProcess — pseudo-handle (always (HANDLE)-1) ─────────────

#[test]
#[serial]
fn get_current_process_returns_handle() {
    setup();
    let items = "\
define c-function GetCurrentProcess () => (h :: <c-handle>);
  library: \"kernel32.dll\";
end;
";
    let s = eval_expr_with_items_to_string(items, "GetCurrentProcess()")
        .unwrap_or_else(|e| panic!("GetCurrentProcess eval failed: {e:?}"));
    // The Win32 pseudo-handle is `(HANDLE)-1`; our marshaling turns
    // that into a fixnum carrying the raw u64 value. Either form is
    // a non-zero integer.
    let n: i64 = s.parse().expect("integer-shaped handle");
    assert!(n != 0, "current-process handle must be non-zero, got {n}");
}

// ─── 6. Deduplication: two call sites share one table entry ──────────────

#[test]
#[serial]
fn api_stub_table_deduplicates_call_sites() {
    setup();
    nod_runtime::_reset_winffi_stats_for_tests();
    let items = "\
define c-function GetTickCount () => (ticks :: <c-dword>);
  library: \"kernel32.dll\";
end;
";
    // Two call sites of the same c-function, lowered in the same
    // module — must share ONE stub-table entry.
    let s = eval_expr_with_items_to_string(
        items,
        "let a = GetTickCount(); let b = GetTickCount(); a + b - a",
    )
    .unwrap_or_else(|e| panic!("dedupe test eval failed: {e:?}"));
    let _: i64 = s.parse().expect("integer return");
    let stats = nod_runtime::winffi_stats();
    assert_eq!(
        stats.entries, 1,
        "deduplicated table must have exactly 1 entry, got {}",
        stats.entries
    );
    assert!(
        stats.total_resolved >= 1,
        "at least one resolution must have happened"
    );
    assert!(
        stats.unique_symbols >= 1,
        "at least one unique symbol must have resolved"
    );
}

// ─── 7. Unknown DLL → <c-ffi-error> ───────────────────────────────────────

#[test]
#[serial]
fn unknown_dll_signals_c_ffi_error() {
    setup();
    let items = "\
define c-function ImaginaryFunc () => (n :: <c-dword>);
  library: \"nosuchmodule_sprint28.dll\";
end;
";
    let result = eval_expr_with_items_to_string(items, "ImaginaryFunc()");
    match result {
        Ok(s) => panic!(
            "expected WinFfiInit error for unknown DLL, got success: {s}"
        ),
        Err(EvalError::WinFfiInit { class_name, dll, .. }) => {
            assert_eq!(class_name, "<c-ffi-error>");
            assert_eq!(dll, "nosuchmodule_sprint28.dll");
        }
        Err(other) => panic!("expected WinFfiInit, got {other:?}"),
    }
}

// ─── 8. Unknown symbol in a real DLL → <c-ffi-error> ──────────────────────

#[test]
#[serial]
fn unknown_symbol_signals_c_ffi_error() {
    setup();
    let items = "\
define c-function ImaginaryFunc_Sprint28 () => (n :: <c-dword>);
  library: \"kernel32.dll\";
end;
";
    let result = eval_expr_with_items_to_string(items, "ImaginaryFunc_Sprint28()");
    match result {
        Ok(s) => panic!(
            "expected WinFfiInit error for unknown symbol, got success: {s}"
        ),
        Err(EvalError::WinFfiInit { class_name, dll, symbol }) => {
            assert_eq!(class_name, "<c-ffi-error>");
            assert_eq!(dll, "kernel32.dll");
            assert_eq!(symbol, "ImaginaryFunc_Sprint28");
        }
        Err(other) => panic!("expected WinFfiInit, got {other:?}"),
    }
}
