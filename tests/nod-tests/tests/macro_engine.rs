//! Sprint 50a — Dylan-side macro engine smoke.
//!
//! AOT-builds `dylan-macro-smoke.dylan` and asserts its stdout matches
//! the expected pattern-match + substitution output for the stdlib
//! `unless` rule expansion. This locks in the V1 macro engine
//! (`<fragment>`, `<pattern-elem>`, `<template-elem>`, `match-pattern`,
//! `substitute`) before Sprint 50b parses real `define macro` source.
//!
//! Run with:
//!   cargo test -p nod-tests --test macro_engine -- --nocapture

use std::path::PathBuf;
use std::process::Command;

use serial_test::serial;

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

#[test]
#[serial]
fn macro_engine_unless_expansion() {
    // Fresh build so the test always reflects the on-disk fixture.
    let build = Command::new("cargo")
        .current_dir(workspace_root())
        .args(["build", "-p", "nod-driver"])
        .output()
        .expect("spawn cargo build");
    assert!(
        build.status.success(),
        "cargo build -p nod-driver failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let workspace = workspace_root();
    let fixture = workspace
        .join("tests")
        .join("nod-tests")
        .join("fixtures")
        .join("dylan-macro-smoke.dylan");
    let exe = std::env::temp_dir().join("dylan-macro-smoke.exe");

    let aot = Command::new(workspace.join("target").join("debug").join("nod-driver.exe"))
        .args(["build"])
        .arg(&fixture)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("spawn nod-driver build");
    assert!(
        aot.status.success(),
        "nod-driver build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&aot.stdout),
        String::from_utf8_lossy(&aot.stderr),
    );

    let run = Command::new(&exe).output().expect("spawn smoke exe");
    assert!(
        run.status.success(),
        "dylan-macro-smoke.exe failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let stdout = String::from_utf8_lossy(&run.stdout);
    // Normalise CRLF → LF — Windows pipes can transcode.
    let stdout = stdout.replace("\r\n", "\n");

    // Sprint 50a hand-built phase + Sprint 50b parsed-def phase. Both
    // produce identical match + substitute output for the same call site.
    let expected = "\
PHASE: hand-built\n\
MATCH: ok\n\
BIND cond: 1 frag\n\
BIND body: 1 frag\n\
EXPAND: if ( ~ x ) ( foo ) else #f end\n\
PHASE: parsed-def\n\
PARSE-DEF: ok, rules=1\n\
MATCH: ok\n\
BIND cond: 1 frag\n\
BIND body: 1 frag\n\
EXPAND: if ( ~ x ) ( foo ) else #f end\n";
    assert_eq!(
        stdout, expected,
        "smoke output diverged:\n--- expected ---\n{expected}--- got ---\n{stdout}",
    );
}
