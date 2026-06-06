//! Sprint 53.2 — byte-match oracle gate for the Dylan-side sema
//! "top-level name table" walk.
//!
//! Two implementations of the same recording pass must agree, byte for
//! byte, on the `=== top-names ===` section of the sema model:
//!
//!   * **Dylan walk** — `collect-top-names` in
//!     `tests/nod-tests/fixtures/dylan-sema.dylan`, AOT-compiled into
//!     `dylan-sema.exe` from `dylan-sema.prj`. Running the EXE on a
//!     fixture prints *only* the top-names section: sorted
//!     `fn <name> arity=<N> return=<Est>` lines, then sorted
//!     `constant <name>` / `variable <name>` lines.
//!
//!   * **Rust oracle** — `nod-driver --parse-with-rust dump-sema <fx>`
//!     prints four sections via `nod_sema::format_sema_model`
//!     (`=== top-names ===`, `=== generics ===`, `=== classes ===`,
//!     `=== sealing ===`). We slice from the start up to (not including)
//!     `=== generics ===` — that prefix is the top-names section the
//!     Dylan walk should reproduce.
//!
//! Scope (Sprint 53.2): CLASS-FREE fixtures only. `define class`
//! generates auto slot-accessor `fn` entries in the oracle that the
//! 53.2 Dylan walk intentionally omits (those arrive in Sprint 53.3),
//! so class fixtures are out of scope here.
//!
//! The gate covers six fixtures that were confirmed to byte-match:
//! `factorial`, `sprint09-add`, `mutual`, `hello`, `stdlib-size-call`,
//! and `kernel-arith`. `kernel-arith` exercises a `define constant`
//! (`*answer*`): the Dylan walk emits a single `constant *answer*` line
//! and *no* `fn` line for it. The Rust oracle records constant /
//! variable names in `top_names.fns` too (they lower to zero-arg getter
//! functions — see `collect_top_level_names`), but `format_sema_model`
//! filters those out of the `fn` listing so the dump matches the Dylan
//! walk's classification.
//!
//! `#[ignore]` like the other AOT tests — it shells out to cargo + the
//! linker to build the EXE once, then runs it per fixture. Run with:
//!
//! ```text
//! cargo test -p nod-tests --test sema_topnames -- --ignored --nocapture
//! ```

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serial_test::serial;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// The class-free fixtures the gate proves byte-match. All live under
/// `tests/nod-tests/fixtures/`.
const FIXTURES: &[&str] = &[
    "factorial",
    "sprint09-add",
    "mutual",
    "hello",
    "stdlib-size-call",
    "kernel-arith",
];

/// Normalize a top-names block the same way on both sides: CRLF -> LF,
/// strip trailing whitespace from every line, and trim trailing blank
/// lines from the whole block. This makes the comparison robust to
/// platform line endings and a stray trailing newline without masking
/// any real content difference.
fn normalize(block: &str) -> String {
    let lf = block.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::new();
    for line in lf.lines() {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// The Dylan EXE prints *only* the top-names section, so its whole
/// stdout is the block to compare (after normalization).
fn dylan_top_names(text: &str) -> String {
    normalize(text)
}

/// Slice the oracle's four-section dump down to the top-names section:
/// everything from the start up to (not including) `=== generics ===`.
fn oracle_top_names(text: &str) -> String {
    let lf = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut block = String::new();
    for line in lf.lines() {
        if line.trim_end() == "=== generics ===" {
            break;
        }
        block.push_str(line);
        block.push('\n');
    }
    normalize(&block)
}

/// Build `dylan-sema.exe` once into a temp path. Panics (failing the
/// test) on any build error.
fn build_dylan_sema_exe(ws: &Path) -> PathBuf {
    let prj = fixtures_dir().join("dylan-sema.prj");
    let exe = std::env::temp_dir().join("nod-sema-topnames-gate.exe");
    let _ = std::fs::remove_file(&exe);

    let build = Command::new("cargo")
        .current_dir(ws)
        .args([
            "run",
            "--quiet",
            "--bin",
            "nod-driver",
            "--",
            "--parse-with-rust",
            "build",
            "--project",
            prj.to_str().unwrap(),
            "-o",
            exe.to_str().unwrap(),
        ])
        .output()
        .expect("spawn dylan-sema build");
    assert!(
        build.status.success(),
        "building dylan-sema failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    assert!(
        exe.is_file(),
        "dylan-sema EXE not produced at {}",
        exe.display()
    );
    exe
}

/// Run the Rust oracle (`nod-driver --parse-with-rust dump-sema <fx>`)
/// and return its stdout. The driver is invoked through `cargo run` so
/// we don't depend on a particular `target/<profile>` layout.
fn run_oracle(ws: &Path, input: &Path) -> String {
    let out = Command::new("cargo")
        .current_dir(ws)
        .args([
            "run",
            "--quiet",
            "--bin",
            "nod-driver",
            "--",
            "--parse-with-rust",
            "dump-sema",
            input.to_str().unwrap(),
        ])
        .output()
        .expect("spawn nod-driver dump-sema");
    assert!(
        out.status.success(),
        "oracle dump-sema failed for {}:\nstderr:\n{}",
        input.display(),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
#[ignore]
#[serial]
fn dylan_sema_top_names_byte_match() {
    let ws = workspace_root();
    let exe = build_dylan_sema_exe(&ws);

    let mut failures: Vec<String> = Vec::new();

    for fx in FIXTURES {
        let input = fixtures_dir().join(format!("{fx}.dylan"));
        assert!(input.is_file(), "missing fixture {}", input.display());

        // Dylan side: run the AOT EXE on the fixture.
        let run = Command::new(&exe)
            .arg(&input)
            .output()
            .unwrap_or_else(|e| panic!("spawn dylan-sema EXE for {fx}: {e}"));
        let dyl_stdout = String::from_utf8_lossy(&run.stdout);
        let dyl_stderr = String::from_utf8_lossy(&run.stderr);
        assert_eq!(
            run.status.code(),
            Some(0),
            "dylan-sema EXE did not exit 0 for {fx}:\nstdout:\n{dyl_stdout}\nstderr:\n{dyl_stderr}"
        );

        let dyl = dylan_top_names(&dyl_stdout);
        let orc = oracle_top_names(&run_oracle(&ws, &input));

        if dyl != orc {
            failures.push(format!(
                "FIXTURE {fx} MISMATCH\n\
                 ----- dylan-sema.exe (top-names) -----\n{dyl}\n\
                 ----- oracle (top-names slice) -----\n{orc}\n\
                 --------------------------------------"
            ));
        } else {
            eprintln!("MATCH: {fx}");
        }
    }

    let _ = std::fs::remove_file(&exe);

    assert!(
        failures.is_empty(),
        "Dylan sema top-names walk diverged from the Rust oracle:\n\n{}",
        failures.join("\n\n")
    );
}
