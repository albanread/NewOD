//! Sprint 53.2 / 53.3 — byte-match oracle gate for the Dylan-side sema
//! recording walk.
//!
//! Two implementations of the same recording pass must agree, byte for
//! byte, on the sema model:
//!
//!   * **Dylan walk** — `collect-top-names` in
//!     `tests/nod-tests/fixtures/dylan-sema.dylan`, AOT-compiled into
//!     `dylan-sema.exe` from `dylan-sema.prj`. Running the EXE on a
//!     fixture prints, in order: `=== top-names ===` (sorted
//!     `fn <name> arity=<N> return=<Est>` lines then sorted
//!     `constant <name>` / `variable <name>` lines), `=== generics ===`
//!     (sorted getter/setter generic names), `=== classes ===` (one
//!     block per user class: `class`, `parents`, `cpl`, `slot …`
//!     lines), and an empty `=== sealing ===` header.
//!
//!   * **Rust oracle** — `nod-driver --parse-with-rust dump-sema <fx>`
//!     prints the same four sections via `nod_sema::format_sema_model`.
//!
//! Sprint 53.2 gated only the `=== top-names ===` section for CLASS-FREE
//! fixtures. Sprint 53.3 adds the slot-accessor `fn` entries, the
//! `=== generics ===` section, and the `=== classes ===` section, and
//! gates two single-class fixtures (`point`, `gc_precise_two_makes`).
//! We now compare everything the Dylan walk emits — the oracle sliced
//! up to and **including** the `=== sealing ===` header — against the
//! Dylan EXE's full stdout. The `=== sealing ===` body is empty for all
//! eight fixtures and is the subject of Sprint 53.4, so any sealed-*
//! lines the oracle prints after that header are sliced off here.
//!
//! `kernel-arith` exercises a `define constant` (`*answer*`): the Dylan
//! walk emits a single `constant *answer*` line and *no* `fn` line for
//! it. The Rust oracle records constant / variable names in
//! `top_names.fns` too (they lower to zero-arg getter functions — see
//! `collect_top_level_names`), but `format_sema_model` filters those out
//! of the `fn` listing so the dump matches the Dylan walk's
//! classification.
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

/// The fixtures the gate proves byte-match. All live under
/// `tests/nod-tests/fixtures/`. The first six are class-free (Sprint
/// 53.2); `point` and `gc_precise_two_makes` are single-class fixtures
/// added in Sprint 53.3 (one user class, super `<object>`, two slots,
/// both with setters).
const FIXTURES: &[&str] = &[
    "factorial",
    "sprint09-add",
    "mutual",
    "hello",
    "stdlib-size-call",
    "kernel-arith",
    "point",
    "gc_precise_two_makes",
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

/// The Dylan EXE prints all four sections through the (empty)
/// `=== sealing ===` header, so its whole stdout is the block to compare
/// (after normalization).
fn dylan_model(text: &str) -> String {
    normalize(text)
}

/// Slice the oracle's four-section dump down to everything the Dylan walk
/// covers: from the start up to **and including** the `=== sealing ===`
/// header line, dropping any sealed-class / sealed-generic / domain lines
/// that follow it (Sprint 53.4 territory; empty for all gated fixtures).
fn oracle_through_sealing(text: &str) -> String {
    let lf = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut block = String::new();
    for line in lf.lines() {
        block.push_str(line);
        block.push('\n');
        if line.trim_end() == "=== sealing ===" {
            break;
        }
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

        let dyl = dylan_model(&dyl_stdout);
        let orc = oracle_through_sealing(&run_oracle(&ws, &input));

        if dyl != orc {
            failures.push(format!(
                "FIXTURE {fx} MISMATCH\n\
                 ----- dylan-sema.exe (full model) -----\n{dyl}\n\
                 ----- oracle (sliced through === sealing ===) -----\n{orc}\n\
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
