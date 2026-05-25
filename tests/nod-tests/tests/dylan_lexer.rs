//! Sprint 45 — Dylan lexer tests.
//!
//! 45a covers the dump-infrastructure acceptance (this file's lone
//! test); 45b fills out the per-token-kind tests against the real
//! `lex` implementation; 45d adds the oracle test against
//! `nod-reader::lex`.
//!
//! The Dylan-in-Dylan lexer source lives at
//! `tests/nod-tests/fixtures/dylan-lexer.dylan`. The 45a stub `lex`
//! returns a one-element `<stretchy-vector>` containing a single
//! `<eof-token>` at byte offset 0, so the canonical dump for any
//! input is exactly `1:1-1:1  EOF\n`. The driver subcommand
//! `nod-driver dump-dylan-tokens <path>` AOT-compiles the lexer
//! source, runs the resulting EXE with `<path>` as argv[1], and
//! forwards stdout — which we shell out to here, mirroring the
//! pattern from `aot_dylan.rs`.
//!
//! Each test is `#[ignore]` + `serial_test::serial` because the AOT
//! pipeline shells out to `cargo run --bin nod-driver` plus MSVC's
//! `link.exe`, and concurrent invocations would stall on Cargo's
//! build-system lock.
//!
//! Run with:
//!
//! ```text
//! cargo test --test dylan_lexer -- --ignored --nocapture
//! ```

#![cfg(windows)]

use std::path::PathBuf;
use std::process::Command;

use serial_test::serial;

/// Workspace root inferred from `CARGO_MANIFEST_DIR`. Mirrors the
/// helper in `aot_dylan.rs`.
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

/// Sprint 45a headline acceptance — driver subcommand round-trips
/// `hello.dylan` through the stub lex and prints exactly
/// `1:1-1:1  EOF\n` to stdout.
///
/// What this proves:
///   * `tests/nod-tests/fixtures/dylan-lexer.dylan` compiles cleanly
///     through the AOT pipeline (parse → expand → lower → codegen →
///     link).
///   * The Dylan class hierarchy (`<span>`, `<token>` + every
///     concrete subclass) builds and `make(<eof-token>, span: …)`
///     dispatches correctly.
///   * `dump-tokens` calls `print-token-to-string` → `token-kind-name`
///     → `offset-to-line-col-packed` for the lone EOF token and
///     produces the canonical-format line locked in by §6.45a of
///     the design doc.
///   * The `nod-driver dump-dylan-tokens` subcommand wires the
///     embedded source through the build pipeline, runs the EXE,
///     and forwards stdout byte-for-byte.
///
/// 45b will add per-token-kind assertions against the real `lex`;
/// 45d will diff the dump against the Rust lexer's normalised
/// output to lock in the oracle contract.
#[test]
#[ignore]
#[serial]
fn dump_dylan_tokens_for_hello_prints_eof_only() {
    let workspace = workspace_root();
    let hello = workspace
        .join("tests/nod-tests/fixtures/hello.dylan");
    assert!(
        hello.is_file(),
        "hello.dylan fixture missing at {}",
        hello.display()
    );

    // Pre-build the driver + runtime so the subcommand invocation
    // doesn't trip Cargo's build lock partway through the lexer
    // EXE compilation.
    let build = Command::new("cargo")
        .current_dir(&workspace)
        .args(["build", "-p", "nod-driver", "-p", "nod-runtime"])
        .output()
        .expect("spawn cargo build");
    assert!(
        build.status.success(),
        "cargo build failed: {}\nstderr:\n{}",
        build.status,
        String::from_utf8_lossy(&build.stderr)
    );

    let driver = Command::new("cargo")
        .current_dir(&workspace)
        .args([
            "run",
            "--quiet",
            "--bin",
            "nod-driver",
            "--",
            "dump-dylan-tokens",
            hello.to_str().unwrap(),
        ])
        .output()
        .expect("spawn nod-driver dump-dylan-tokens");
    let stdout = String::from_utf8_lossy(&driver.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&driver.stderr).into_owned();
    assert!(
        driver.status.success(),
        "dump-dylan-tokens exit code: {}\nstdout:\n{}\nstderr:\n{}",
        driver.status,
        stdout,
        stderr
    );
    assert_eq!(
        stdout, "1:1-1:1  EOF\n",
        "dump-dylan-tokens stdout mismatch; stderr=\n{stderr}"
    );
}
