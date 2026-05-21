//! Sprint 27 Phase A — `define c-function` parser + sema acceptance.
//!
//! These tests exercise the **parse + sema lowering** path. No actual
//! FFI call is executed; Sprint 28 lands that.

use nod_reader::{Item, Module, SourceMap, lex, parse_module, scan_preamble};
use nod_sema::{LoweringWarning, lower_module_full};
use serial_test::serial;

fn parse_src(src: &str) -> Module {
    let mut sm = SourceMap::new();
    let id = sm.add("<t>", src.to_string()).unwrap();
    let toks = lex(src, id);
    let pre = scan_preamble(src);
    parse_module(src, &toks, pre.as_ref())
        .unwrap_or_else(|d| panic!("parse_module diagnostics: {d:?}\n--- src ---\n{src}"))
}

const BEEP_SRC: &str = "\
define c-function Beep
    (dw-freq :: <c-dword>, dw-duration :: <c-dword>)
 => (success :: <c-bool>);
  c-name: \"Beep\";
  library: \"kernel32.dll\";
end c-function;
";

#[test]
fn parse_define_c_function_records_library() {
    let m = parse_src(BEEP_SRC);
    assert_eq!(m.items.len(), 1);
    match &m.items[0] {
        Item::DefineCFunction { name, params, return_, c_name, library, .. } => {
            assert_eq!(name, "Beep");
            assert_eq!(library, "kernel32.dll");
            assert_eq!(c_name.as_deref(), Some("Beep"));
            assert_eq!(params.len(), 2);
            assert!(return_.is_some(), "expected => (...) return clause");
        }
        other => panic!("expected Item::DefineCFunction, got {}", other.kind_tag()),
    }
}

#[test]
fn parse_define_c_function_accepts_implicit_c_name() {
    // No explicit `c-name:` — Sema defaults to the Dylan-side name.
    let src = "\
define c-function Beep
    (dw-freq :: <c-dword>, dw-duration :: <c-dword>)
 => (success :: <c-bool>);
  library: \"kernel32.dll\";
end;
";
    let m = parse_src(src);
    assert_eq!(m.items.len(), 1);
    match &m.items[0] {
        Item::DefineCFunction { name, c_name, library, .. } => {
            assert_eq!(name, "Beep");
            assert_eq!(c_name.as_deref(), None);
            assert_eq!(library, "kernel32.dll");
        }
        other => panic!("expected Item::DefineCFunction, got {}", other.kind_tag()),
    }
}

#[test]
#[serial]
fn c_function_binding_records_dll_provenance() {
    let m = parse_src(BEEP_SRC);
    let lm = lower_module_full(&m).unwrap_or_else(|e| panic!("sema errors: {e:?}"));
    assert_eq!(lm.c_functions.len(), 1, "expected one c-function binding");
    let b = &lm.c_functions[0];
    assert_eq!(b.dylan_name, "Beep");
    assert_eq!(b.c_name, "Beep");
    assert_eq!(b.library, "kernel32.dll");
    assert!(
        b.resolved_in_db,
        "Beep@kernel32.dll must be present in the embedded windows_api index"
    );
    // No warnings for a fully-resolved binding.
    assert!(
        lm.warnings.is_empty(),
        "expected no warnings; got {:?}",
        lm.warnings
    );
}

#[test]
#[serial]
fn c_function_unknown_in_db_produces_warning() {
    // `ImaginaryFunc` doesn't exist in kernel32.dll. Sema accepts
    // the declaration (returns successfully) but surfaces a warning.
    let src = "\
define c-function ImaginaryFunc
    (a :: <c-dword>)
 => (b :: <c-bool>);
  library: \"kernel32.dll\";
end;
";
    let m = parse_src(src);
    let lm = lower_module_full(&m)
        .unwrap_or_else(|e| panic!("sema errors (should warn, not error): {e:?}"));
    assert_eq!(lm.c_functions.len(), 1);
    let b = &lm.c_functions[0];
    assert!(!b.resolved_in_db, "ImaginaryFunc must NOT be resolved");
    assert_eq!(lm.warnings.len(), 1, "expected exactly one warning");
    match &lm.warnings[0] {
        LoweringWarning::CFunctionNotInDb { name, library, c_name, .. } => {
            assert_eq!(name, "ImaginaryFunc");
            assert_eq!(library, "kernel32.dll");
            assert_eq!(c_name, "ImaginaryFunc");
        }
    }
}

#[test]
#[serial]
fn c_function_call_site_errors_in_sprint27() {
    // The c-function is declared AND called — Sprint 27 sema must
    // refuse with a Sprint-28 deferral message.
    let src = "\
define c-function Beep
    (dw-freq :: <c-dword>, dw-duration :: <c-dword>)
 => (success :: <c-bool>);
  library: \"kernel32.dll\";
end;

define function call-beep ()
  Beep(440, 1000);
end function;
";
    let m = parse_src(src);
    match lower_module_full(&m) {
        Ok(_) => panic!("expected sema error for c-function call site"),
        Err(errs) => {
            let msg = format!("{errs:#?}");
            assert!(
                msg.contains("Sprint 28")
                    && (msg.contains("Beep") || msg.contains("c-function calls")),
                "expected Sprint-28 deferral diagnostic mentioning `Beep` and c-function calls; got:\n{msg}"
            );
        }
    }
}

#[test]
#[serial]
fn c_function_call_site_only_errors_for_c_function_names() {
    // A function with the same arity-pattern as a c-function but
    // declared as a regular Dylan function must NOT trip the
    // c-function call diagnostic.
    let src = "\
define function regular-fn (a, b) a + b end function;

define function call-it ()
  regular-fn(1, 2);
end function;
";
    let m = parse_src(src);
    lower_module_full(&m).unwrap_or_else(|e| {
        panic!("regular function call must lower cleanly; got: {e:?}")
    });
}

#[test]
#[serial]
fn empty_library_attribute_errors() {
    // `library:` is mandatory and must be non-empty.
    let src = "\
define c-function Beep
    (dw-freq :: <c-dword>, dw-duration :: <c-dword>)
 => (success :: <c-bool>);
end;
";
    let m = parse_src(src);
    match lower_module_full(&m) {
        Ok(_) => panic!("expected sema error for missing library:"),
        Err(errs) => {
            let msg = format!("{errs:#?}");
            assert!(
                msg.contains("library:"),
                "expected diagnostic mentioning `library:` attribute; got:\n{msg}"
            );
        }
    }
}

