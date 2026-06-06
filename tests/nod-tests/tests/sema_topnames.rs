//! Sprint 53.2 / 53.3 / 53.4 — byte-match oracle gate for the Dylan-side
//! sema recording walk.
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
//!     lines), and the `=== sealing ===` section (sorted `sealed-class`
//!     lines then sorted `sealed-generic` lines).
//!
//!   * **Rust oracle** — `nod-driver --parse-with-rust dump-sema <fx>`
//!     prints the same four sections via `nod_sema::format_sema_model`.
//!
//! Sprint 53.2 gated only the `=== top-names ===` section for CLASS-FREE
//! fixtures. Sprint 53.3 adds the slot-accessor `fn` entries, the
//! `=== generics ===` section, and the `=== classes ===` section, and
//! gates two single-class fixtures (`point`, `gc_precise_two_makes`).
//! Sprint 53.4 adds generics from `define generic`, drops the spurious
//! `fn` line for `define method`, fills in the `=== sealing ===` body,
//! and gates `richards-shape` (a 5-class hierarchy with a sealed generic
//! and four methods). We now compare the Dylan EXE's full stdout against
//! the oracle's complete four-section dump — no slicing — since both
//! sides emit the whole sealing body.
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
/// `tests/nod-tests/fixtures/`.
///
/// Sprints 53.2–53.4 grew the Dylan walk section by section (top-names,
/// generics, classes, sealing). Sprint 53.5 then ran the byte-match over
/// the whole fixture corpus and found the Dylan walk already reproduces
/// the Rust oracle for the great majority of inputs — so the list below
/// is broadened to that verified-matching set, not just the hand-picked
/// shapes. Sprint 53.5c closed the `macro-when-cleanup` divergence: the
/// Dylan parser now recognizes the NAME-token body-shaped statement macro
/// `with-cleanup … cleanup … end` (it was previously parsed as a bare
/// variable-ref and desynced, dropping the enclosing `define function`).
/// (The one known remaining divergence is documented in the journal:
/// anonymous-method lifting `__anon-method-N` — `rope`, `ide_rope`,
/// `unified_ide`, `nod-ide` — which awaits its own focused sprint.)
const FIXTURES: &[&str] = &[
    // 53.2 — class-free top-names (functions / constants / variables).
    "factorial",
    "sprint09-add",
    "mutual",
    "hello",
    "stdlib-size-call",
    "kernel-arith",
    "stdlib-min",
    // 53.3 — single-class fixtures (one class, super `<object>`, slots).
    "point",
    "gc_precise_two_makes",
    // 53.4 — class hierarchy + sealing + `define generic`.
    "richards-shape",       // sealed `<task>` hierarchy + sealed generic
    "richards-shape-open",  // same shape, open (non-sealed) classes
    // 53.5 — corpus broadening: fixtures the Dylan walk already byte-matches
    // (verified by a full-corpus survey). Macro-using surface + the macro
    // engine's test inputs + GAP/GC repros + jit-cache + translate + IDE
    // helpers — a wide spread of real shapes, all green with no walk change.
    "cond_smoke",
    "macros-unless",
    "macro-when-only",
    "macro-for-range",
    // 53.5c — NAME-token body-shaped statement macro (`with-cleanup`).
    "macro-when-cleanup",
    "dylan-lexer-main",
    "dylan-macro-collect",
    "dylan-macro-expand",
    "dylan-macro-file",
    "dylan-macro-match",
    "dylan-macro-walk",
    "expand-pipeline-smoke",
    "gap-007-repro",
    "gap011-repro",
    "gap011-repro2",
    "gap011-jcs-min-crash",
    "jit_cache_sample",
    "jit_cache_sample_items",
    "translate-class",
    "translate-loop",
    "ide_helpers",
    "ide_syntax",
    "ide_win_calls",
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

/// The Dylan EXE prints all four sections, including the full
/// `=== sealing ===` body (Sprint 53.4), so its whole stdout is the block
/// to compare (after normalization).
fn dylan_model(text: &str) -> String {
    normalize(text)
}

/// The whole oracle four-section dump, normalized. As of Sprint 53.4 the
/// Dylan walk emits the complete `=== sealing ===` body too (sorted
/// `sealed-class` lines then sorted `sealed-generic` lines), so the test
/// compares against the oracle's entire output rather than slicing it at
/// the `=== sealing ===` header. The first eight fixtures have an empty
/// sealing section; `richards-shape` exercises a non-empty one.
fn oracle_full(text: &str) -> String {
    normalize(text)
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
        let orc = oracle_full(&run_oracle(&ws, &input));

        if dyl != orc {
            failures.push(format!(
                "FIXTURE {fx} MISMATCH\n\
                 ----- dylan-sema.exe (full model) -----\n{dyl}\n\
                 ----- oracle (full four-section dump) -----\n{orc}\n\
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
