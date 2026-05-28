//! Sprint 46 — Dylan-in-Dylan parser tests.
//!
//! The parser source lives at
//! `tests/nod-tests/fixtures/dylan-parser.dylan` and is compiled
//! together with `dylan-lexer.dylan` into a cached EXE by the driver
//! subcommand `nod-driver parse-dylan <path>`. That subcommand AOT-builds
//! [lexer, parser] once into the OS tempdir, then runs the EXE with
//! `<path>` as argv[1] and forwards stdout (the indented AST dump).
//!
//! These tests mirror `dylan_lexer.rs`: each is `#[ignore]` + `#[serial]`
//! because the pipeline shells out to `cargo run --bin nod-driver` plus
//! MSVC's `link.exe`, and concurrent invocations would stall on Cargo's
//! build-system lock.
//!
//! Run with:
//!
//! ```text
//! cargo test --test dylan_parser -- --ignored --nocapture
//! ```

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use nod_tests::test_support::run_command_with_watchdog;
use serial_test::serial;

/// Workspace root inferred from `CARGO_MANIFEST_DIR`. Mirrors the helper
/// in `dylan_lexer.rs`.
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

/// Per-test scratch dir for the snippets we'll parse. All snippet files go
/// under `<target>/dylan-parser-test-snippets/<name>.dylan`.
fn snippet_dir() -> PathBuf {
    let workspace = workspace_root();
    let dir = workspace
        .join("target")
        .join("dylan-parser-test-snippets");
    std::fs::create_dir_all(&dir).expect("create snippet dir");
    dir
}

/// Write a snippet to a temp file and return its absolute path.
fn write_snippet(name: &str, contents: &str) -> PathBuf {
    let path = snippet_dir().join(format!("{name}.dylan"));
    std::fs::write(&path, contents).expect("write snippet");
    path
}

/// Pre-build `nod-driver` + `nod-runtime` once per test to avoid races
/// against Cargo's build lock when the parser EXE is built.
fn prebuild_driver(workspace: &Path) {
    let mut build = Command::new("cargo");
    build
        .current_dir(workspace)
        .args(["build", "-p", "nod-driver", "-p", "nod-runtime"]);
    let build = run_command_with_watchdog(
        "dylan_parser",
        "cargo-build",
        Duration::from_secs(300),
        &mut build,
    );
    assert!(
        build.status.success(),
        "cargo build failed: {}\nstderr:\n{}\nstdout log: {}\nstderr log: {}\nmeta: {}",
        build.status,
        build.stderr,
        build.stdout_path.display(),
        build.stderr_path.display(),
        build.meta_path.display()
    );
}

/// Parse one snippet through the cached parser EXE; return stdout as a
/// UTF-8 String (the AST dump).
fn parse_snippet(snippet: &Path) -> String {
    let workspace = workspace_root();
    prebuild_driver(&workspace);
    let mut driver = Command::new("cargo");
    driver.current_dir(&workspace).args([
        "run",
        "--quiet",
        "--bin",
        "nod-driver",
        "--",
        "parse-dylan",
        snippet.to_str().unwrap(),
    ]);
    let driver = run_command_with_watchdog(
        "dylan_parser",
        "parse-dylan",
        Duration::from_secs(180),
        &mut driver,
    );
    let stdout = driver.stdout.clone();
    let stderr = driver.stderr.clone();
    assert!(
        driver.status.success(),
        "parse-dylan on {} failed: {}\nstdout:\n{}\nstderr:\n{}\nstdout log: {}\nstderr log: {}\nmeta: {}",
        snippet.display(),
        driver.status,
        stdout,
        stderr,
        driver.stdout_path.display(),
        driver.stderr_path.display(),
        driver.meta_path.display()
    );
    stdout
}

/// Parse a snippet and assert the dump contains all of `expected` and no
/// `ERROR` lines.
fn assert_dump(name: &str, source: &str, expected: &[&str]) {
    let snippet = write_snippet(name, source);
    let dump = parse_snippet(&snippet);
    assert!(
        !dump.lines().any(|l| l.contains("ERROR")),
        "test {name}: unexpected ERROR in dump:\n{dump}"
    );
    for want in expected {
        assert!(
            dump.contains(want),
            "test {name}: expected substring {want:?}\ndump:\n{dump}"
        );
    }
}

// ─── headline: define method with typed params + return ───────────────────

/// Sprint 46 headline acceptance — `define method foo (x :: <integer>) =>
/// (y :: <integer>)` must parse into a structured signature instead of
/// crashing with "expected ) after arguments".
#[test]
#[ignore]
#[serial]
fn define_method_typed_signature() {
    let source = "\
define method foo (x :: <integer>) => (y :: <integer>)
  x + 1
end method;
";
    assert_dump(
        "define_method_typed_signature",
        source,
        &[
            "DEFINE-BODY method foo",
            "PARAMS",
            "PARAM x",
            "RETURNS",
            "VALUE y",
            "NAME <integer>",
        ],
    );
}

#[test]
#[ignore]
#[serial]
fn define_method_multiple_params() {
    let source = "\
define method add (x :: <integer>, y :: <integer>) => (z :: <integer>)
  x + y
end method;
";
    assert_dump(
        "define_method_multiple_params",
        source,
        &["DEFINE-BODY method add", "PARAM x", "PARAM y", "VALUE z"],
    );
}

#[test]
#[ignore]
#[serial]
fn define_method_untyped_param_and_value() {
    let source = "\
define method id (x) => (y)
  x
end method;
";
    assert_dump(
        "define_method_untyped",
        source,
        &["DEFINE-BODY method id", "PARAM x", "VALUE y"],
    );
}

#[test]
#[ignore]
#[serial]
fn define_method_rest_param() {
    let source = "\
define method collect (#rest more) => ()
  more
end method;
";
    assert_dump(
        "define_method_rest",
        source,
        &["DEFINE-BODY method collect", "PARAMS", "REST more", "RETURNS"],
    );
}

#[test]
#[ignore]
#[serial]
fn define_method_key_params() {
    let source = "\
define method opts (x, #key a b) => ()
  x
end method;
";
    assert_dump(
        "define_method_key",
        source,
        &["PARAM x", "KEY", "KEY-PARAM a", "KEY-PARAM b"],
    );
}

#[test]
#[ignore]
#[serial]
fn define_method_all_keys() {
    let source = "\
define method anyopts (#key a, #all-keys) => ()
  a
end method;
";
    assert_dump(
        "define_method_all_keys",
        source,
        &["KEY", "KEY-PARAM a", "ALL-KEYS"],
    );
}

#[test]
#[ignore]
#[serial]
fn define_method_empty_return() {
    let source = "\
define method noret (x :: <integer>) => ()
  x
end method;
";
    // `=> ()` is present but carries no values: RETURNS block with no VALUE.
    assert_dump(
        "define_method_empty_return",
        source,
        &["DEFINE-BODY method noret", "PARAM x", "RETURNS"],
    );
}

#[test]
#[ignore]
#[serial]
fn define_method_bare_return_name() {
    let source = "\
define method bare (x) => name
  x
end method;
";
    assert_dump(
        "define_method_bare_return",
        source,
        &["DEFINE-BODY method bare", "RETURNS", "VALUE name"],
    );
}

// ─── anonymous method literal in expression position ───────────────────────

/// `method (x :: <integer>) => (<integer>) x end` as an anonymous literal.
#[test]
#[ignore]
#[serial]
fn anonymous_method_literal() {
    let source = "let f = method (x :: <integer>) => (<integer>) x end;\n";
    assert_dump(
        "anonymous_method_literal",
        source,
        &["STMT method", "PARAMS", "PARAM x", "RETURNS"],
    );
}

// ─── local method ──────────────────────────────────────────────────────────

#[test]
#[ignore]
#[serial]
fn local_method_signature() {
    let source = "\
local method helper (a :: <integer>, b) => (s :: <integer>)
  a + b
end method helper;
";
    assert_dump(
        "local_method_signature",
        source,
        &["LOCAL", "STMT method helper", "PARAM a", "PARAM b", "VALUE s"],
    );
}

// ─── no-regression: simpler shapes still parse ─────────────────────────────

#[test]
#[ignore]
#[serial]
fn simple_call_and_let_still_parse() {
    let source = "format-out(\"hi\");\nlet x = 1 + 2;\n";
    assert_dump(
        "simple_call_and_let",
        source,
        &["CALL", "NAME format-out", "LET", "BINOP"],
    );
}
