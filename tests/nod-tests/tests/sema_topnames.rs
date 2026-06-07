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
/// Sprint 53.5b closed the anonymous-method-lifting divergence: the Dylan
/// walk now mirrors the Rust lowering pre-pass (`lift_anonymous_methods`),
/// lifting every `method (...) ... end` literal in expression position to a
/// synthetic `__anon-method-N` top-level function, numbered in the same
/// depth-first source order. `nod-ide` (four such literals) now byte-matches
/// end-to-end and joins the gate. Ground truth corrected the 53.5(1) survey,
/// which had blamed the `rope` / `ide_rope` / `unified_ide` divergence solely
/// on anon-methods: those three actually carried TWO further, independent
/// gaps the anon-method work does not touch.
///
/// Sprint 53.5d then closed the first of those two: implicit generics from
/// bare `define method`. The oracle's `collect_generic_names` records a
/// `generic <name>` per `DefineMethod` name (alongside `DefineGeneric` names
/// and slot accessors); the Dylan walk now does the same, deduped against the
/// explicit generics. The `rope` family's `=== generics ===` section now
/// matches.
///
/// Sprint 53.5e then closed the last one: user-class return estimates.
/// `empty-rope () => (r :: <rope-leaf>)` dumped `return=Class(<id>)` in the
/// oracle (a *raw, process-global* class-id — a portability leak, since
/// everything else in the dump refers to classes by name via
/// `sema_class_name`) vs `return=Top` here. `format_sema_model` now renders a
/// `Class` return by NAME (`return=Class(<rope-leaf>)`), and the Dylan walk
/// maps a user-class return type to the same. With all three gaps closed the
/// rope family — `rope`, `ide_rope`, `unified_ide` — joins the gate, so every
/// fixture the 53.5(1) survey flagged is now byte-matched and gated.
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
    // 53.5b — anonymous-method lifting (`__anon-method-N`). `nod-ide` has
    // four `method (…) … end` literals and now byte-matches end-to-end.
    "nod-ide",
    // 53.5b/d/e — the rope family: anon-method literal + implicit method
    // generics + user-class return (`empty-rope () => (r :: <rope-leaf>)`
    // now dumps `return=Class(<rope-leaf>)` by name on both sides).
    "rope",
    "ide_rope",
    "unified_ide",
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

/// Sprint 54b — run the IN-PROCESS Dylan sema walk via the statically-linked
/// `dylan-sema-emit` shim entry (`nod-driver dump-dylan-sema <fx>`), returning
/// `(stdout, stderr, success)`. The model dump lands on stdout; the
/// "override installed" startup log goes to stderr.
fn run_in_process_sema(ws: &Path, input: &Path) -> (String, String, bool) {
    let out = Command::new("cargo")
        .current_dir(ws)
        .args([
            "run",
            "--quiet",
            "--bin",
            "nod-driver",
            "--",
            "dump-dylan-sema",
            input.to_str().unwrap(),
        ])
        .output()
        .expect("spawn nod-driver dump-dylan-sema");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// Sprint 54b — the IN-PROCESS load-bearing path: the Dylan sema recording
/// walk runs INSIDE the host via the statically-linked `dylan-sema-emit` shim
/// entry (`dump-dylan-sema`), not the standalone EXE. Its model dump must
/// byte-match the Rust oracle (`dump-sema --parse-with-rust`) across the same
/// gated corpus the standalone gate covers. Sibling of
/// `dylan_sema_top_names_byte_match` (which exercises the standalone EXE) —
/// together they prove the same Dylan `collect-top-names` matches the oracle
/// whether run as an EXE or in-process through the shim.
///
/// Skips cleanly when the shim isn't statically linked (the
/// `dylan-lex-shim.lib.obj` wasn't built — `dump-dylan-sema` then fails with a
/// "shim init" message). Build it via the bootstrap, then re-run.
#[test]
#[ignore]
#[serial]
fn dylan_sema_in_process_byte_match() {
    let ws = workspace_root();

    // Probe once: no shim linked ⇒ "shim init failed" ⇒ skip the whole gate.
    let probe = fixtures_dir().join("hello.dylan");
    let (_, probe_err, probe_ok) = run_in_process_sema(&ws, &probe);
    if !probe_ok && probe_err.contains("shim init") {
        eprintln!(
            "SKIP dylan_sema_in_process_byte_match: dylan-lex-shim.lib.obj not linked.\n{probe_err}"
        );
        return;
    }

    let mut failures: Vec<String> = Vec::new();
    for fx in FIXTURES {
        let input = fixtures_dir().join(format!("{fx}.dylan"));
        assert!(input.is_file(), "missing fixture {}", input.display());

        let (ip_out, ip_err, ip_ok) = run_in_process_sema(&ws, &input);
        assert!(ip_ok, "dump-dylan-sema failed for {fx}:\nstderr:\n{ip_err}");
        let ip = normalize(&ip_out);
        let orc = normalize(&run_oracle(&ws, &input));
        if ip != orc {
            failures.push(format!(
                "FIXTURE {fx} MISMATCH (in-process Dylan sema vs oracle)\n\
                 ----- dump-dylan-sema (in-process shim) -----\n{ip}\n\
                 ----- oracle (--parse-with-rust dump-sema) -----\n{orc}\n\
                 --------------------------------------"
            ));
        } else {
            eprintln!("MATCH: {fx}");
        }
    }

    assert!(
        failures.is_empty(),
        "In-process Dylan sema walk diverged from the Rust oracle:\n\n{}",
        failures.join("\n\n")
    );
}

/// Sprint 54c — `nod-driver dump-dfm [--sema-with-dylan] <fx>`, returns
/// `(stdout, stderr, success)`.
fn run_dump_dfm(ws: &Path, input: &Path, sema_with_dylan: bool) -> (String, String, bool) {
    let mut args: Vec<&str> = vec!["run", "--quiet", "--bin", "nod-driver", "--"];
    if sema_with_dylan {
        args.push("--sema-with-dylan");
    }
    args.push("dump-dfm");
    args.push(input.to_str().unwrap());
    let out = Command::new("cargo")
        .current_dir(ws)
        .args(&args)
        .output()
        .expect("spawn nod-driver dump-dfm");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// Sprint 55 Phase 0 / 55a — fixtures the Dylan-side lowering reproduces
/// byte-for-byte so far: straight-line `define function`s (literals / binops /
/// direct calls / `let` bindings; no control flow, classes, or closures).
/// Grows form-by-form as 55a adds `if` / loops / etc.
const PHASE0_LOWER_FIXTURES: &[&str] = &[
    "sprint09-add",          // params + binop
    "mutual",                // 3 fns: direct calls + int consts + binops
    "hello",                 // string literal (Rust `{:?}` escaping) + call
    "gap011-jcs-min-crash",  // 40 fns, chained direct calls
    "lower-let",             // 55a: chained `let` bindings + arithmetic
    "lower-if",              // 55a: `let` + `if`/`else` (block-param SSA diamond)
    "lower-shortcircuit",    // 55a: `|` / `&` short-circuit diamonds
    "lower-loop",            // 55a: while/until loops + `:=` (env-merge phis)
    "factorial",             // 55a: recursion + `if` (real corpus fixture)
    "jit_cache_sample_items", // 55a: real corpus fixture, now fully lowered
];

/// Sprint 55 Phase 0 — `nod-driver dump-dylan-dfm <fx>` (in-process Dylan
/// AST→DFM lowering via the `dylan-lower-emit` shim entry). Returns
/// `(stdout, stderr, success)`.
fn run_dump_dylan_dfm(ws: &Path, input: &Path) -> (String, String, bool) {
    let out = Command::new("cargo")
        .current_dir(ws)
        .args([
            "run",
            "--quiet",
            "--bin",
            "nod-driver",
            "--",
            "dump-dylan-dfm",
            input.to_str().unwrap(),
        ])
        .output()
        .expect("spawn nod-driver dump-dylan-dfm");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// Sprint 55 Phase 0 — the lowering byte-match gate: the Dylan-side AST→DFM
/// lowering (`dump-dylan-dfm`, in-process via the shim) must produce DFM
/// byte-identical to the Rust lowering (`dump-dfm`) on the straight-line
/// subset. This is the oracle for the lowering port, the analogue of the sema
/// `dump-sema` gates. Grows fixture-by-fixture as 55a/b/c add forms. Skips
/// cleanly when the shim isn't statically linked.
#[test]
#[ignore]
#[serial]
fn dylan_lower_phase0_dump_dfm_byte_match() {
    let ws = workspace_root();

    // Probe: no shim ⇒ dump-dylan-dfm fails with "shim init" ⇒ skip.
    let probe = fixtures_dir().join("sprint09-add.dylan");
    let (_, probe_err, probe_ok) = run_dump_dylan_dfm(&ws, &probe);
    if !probe_ok && probe_err.contains("shim init") {
        eprintln!("SKIP dylan_lower_phase0_dump_dfm_byte_match: shim not linked.\n{probe_err}");
        return;
    }

    let mut failures: Vec<String> = Vec::new();
    for fx in PHASE0_LOWER_FIXTURES {
        let input = fixtures_dir().join(format!("{fx}.dylan"));
        assert!(input.is_file(), "missing fixture {}", input.display());

        let (dyl_out, dyl_err, dyl_ok) = run_dump_dylan_dfm(&ws, &input);
        assert!(dyl_ok, "dump-dylan-dfm failed for {fx}:\nstderr:\n{dyl_err}");
        let d = normalize(&dyl_out);
        // An empty dump means the Dylan lowering bailed (fixture left to Rust);
        // a Phase-0 fixture MUST be lowerable, so empty is a failure here.
        assert!(
            !d.is_empty(),
            "Phase-0 fixture {fx} produced no Dylan DFM (lowering bailed)"
        );
        let r = normalize(&run_dump_dfm(&ws, &input, false).0);
        if d != r {
            let first = r
                .lines()
                .zip(d.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map(|(i, (a, b))| format!("  line {i}:\n    rust : {a}\n    dylan: {b}"))
                .unwrap_or_else(|| "  (length differs)".to_string());
            failures.push(format!("FIXTURE {fx} DFM MISMATCH\n{first}"));
        } else {
            eprintln!("MATCH: {fx}");
        }
    }

    assert!(
        failures.is_empty(),
        "Dylan AST→DFM lowering diverged from the Rust lowering:\n\n{}",
        failures.join("\n\n")
    );
}

/// Sprint 54c — THE load-bearing gate (the roadmap's Sprint 54 acceptance
/// criterion): the back-end's DFM must be byte-identical whether the sema
/// recording came from the Rust `analyse_module` or from the Dylan walk
/// (`--sema-with-dylan`, reconstructed host-side from the in-process
/// `dylan-sema-emit` model dump). Passing this means the Dylan sema is
/// authoritative for codegen — Rust sema is retired from the `dump-dfm` path.
/// Skips cleanly when the shim isn't statically linked.
#[test]
#[ignore]
#[serial]
fn dump_dfm_sema_with_dylan_byte_match() {
    let ws = workspace_root();

    // Probe: no shim ⇒ `--sema-with-dylan` warns + falls back to Rust (the run
    // wouldn't exercise the Dylan path), so skip rather than pass vacuously.
    let probe = fixtures_dir().join("hello.dylan");
    let (_, probe_err, probe_ok) = run_dump_dfm(&ws, &probe, true);
    if !probe_ok || probe_err.contains("not statically linked") {
        eprintln!("SKIP dump_dfm_sema_with_dylan_byte_match: shim not linked.\n{probe_err}");
        return;
    }

    let mut failures: Vec<String> = Vec::new();
    for fx in FIXTURES {
        let input = fixtures_dir().join(format!("{fx}.dylan"));
        assert!(input.is_file(), "missing fixture {}", input.display());

        let (rust_out, _, rust_ok) = run_dump_dfm(&ws, &input, false);
        let (dyl_out, dyl_err, dyl_ok) = run_dump_dfm(&ws, &input, true);
        assert!(rust_ok, "dump-dfm (rust sema) failed for {fx}");
        assert!(
            dyl_ok,
            "dump-dfm --sema-with-dylan failed for {fx}:\nstderr:\n{dyl_err}"
        );

        let r = normalize(&rust_out);
        let d = normalize(&dyl_out);
        if r != d {
            // DFM dumps are large; report the first divergent line pair.
            let first = r
                .lines()
                .zip(d.lines())
                .enumerate()
                .find(|(_, (a, b))| a != b)
                .map(|(i, (a, b))| format!("  line {i}:\n    rust : {a}\n    dylan: {b}"))
                .unwrap_or_else(|| "  (length differs)".to_string());
            failures.push(format!("FIXTURE {fx} DFM MISMATCH (rust sema vs dylan sema)\n{first}"));
        } else {
            eprintln!("MATCH: {fx}");
        }
    }

    assert!(
        failures.is_empty(),
        "DFM diverged between the Rust and Dylan sema recordings:\n\n{}",
        failures.join("\n\n")
    );
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
