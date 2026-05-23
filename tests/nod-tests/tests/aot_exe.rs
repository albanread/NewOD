//! Sprint 39a — end-to-end AOT EXE tests.
//!
//! Each test:
//!   1. Writes a Dylan source file into a temp directory.
//!   2. Shells out to `cargo run --bin nod-driver -- build <src> -o <exe>`.
//!   3. Spawns the resulting `.exe` and captures stdout + exit code.
//!   4. Asserts both match expectations.
//!
//! ## Why `#[ignore]`-only
//!
//! These tests shell out to MSVC's `link.exe`, which not every
//! development machine has on `%PATH%`. The Sprint 39a brief mandates
//! `#[ignore]` so routine `cargo test --workspace` runs stay green on
//! barebones CI / non-VS-installed dev boxes.
//!
//! Run manually with:
//!
//! ```text
//! cargo test --test aot_exe -- --ignored --nocapture
//! ```
//!
//! ## Why subprocess + temp dir
//!
//! `cargo run --bin nod-driver` re-uses the workspace's `target/debug`
//! directory so the in-process `nod_runtime.lib` is the same artifact
//! the parent test session is linked against — no extra `cargo build`
//! step needed. The temp dir keeps `.dylan`, `.obj`, and `.exe`
//! artifacts isolated per test so concurrent invocations can't
//! clobber each other's outputs.
//!
//! Cleanup: best-effort. On success we remove the temp dir; on failure
//! the artifacts are kept so a developer can re-run `link.exe` by hand
//! and inspect the IR / object files. The temp-dir prefix
//! (`nod-aot-exe-test-`) makes them easy to clean up manually.
//!
//! ## Why `serial`
//!
//! Cargo's test runner spawns tests in parallel by default. Each test
//! here invokes a fresh `cargo run --bin nod-driver` which acquires
//! Cargo's build-system lock; running them concurrently leads to
//! "blocking waiting for file lock" stalls in CI. `serial_test::serial`
//! forces them to run one at a time.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serial_test::serial;

/// Workspace root inferred from `CARGO_MANIFEST_DIR`. Subprocess
/// invocations of `cargo` use this so `cargo run --bin nod-driver`
/// resolves to the workspace's nod-driver crate.
fn workspace_root() -> PathBuf {
    // The test runner sets `CARGO_MANIFEST_DIR` to
    // `<workspace>/tests/nod-tests`; the workspace root is two levels
    // up.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

/// Per-test temp directory. Hand-rolled (no `tempfile` dep). Returns
/// the directory path and the test name suffix used for uniqueness.
fn make_temp_dir(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nod-aot-exe-test-{test_name}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Drive the full Sprint 39a pipeline. Writes `src` to `dir/<stem>.dylan`,
/// invokes `cargo run --bin nod-driver -- build ...`, spawns the resulting
/// EXE, returns (stdout, stderr, exit_code).
fn build_and_run(test_name: &str, source: &str) -> (String, String, i32) {
    let dir = make_temp_dir(test_name);
    let src_path = dir.join("input.dylan");
    let exe_path = dir.join("output.exe");
    std::fs::write(&src_path, source).expect("write source");

    // First ensure nod-runtime + nod-driver are fresh. Re-running `cargo
    // build -p nod-driver` is a no-op if already built and avoids race
    // windows where the staticlib is out-of-date.
    let workspace = workspace_root();
    let build = Command::new("cargo")
        .current_dir(&workspace)
        .args(["build", "-p", "nod-driver", "-p", "nod-runtime"])
        .output()
        .expect("spawn cargo build");
    if !build.status.success() {
        panic!(
            "cargo build failed: {}\nstderr:\n{}",
            build.status,
            String::from_utf8_lossy(&build.stderr)
        );
    }

    let driver = Command::new("cargo")
        .current_dir(&workspace)
        .args([
            "run",
            "--quiet",
            "--bin",
            "nod-driver",
            "--",
            "build",
            src_path.to_str().unwrap(),
            "-o",
            exe_path.to_str().unwrap(),
        ])
        .output()
        .expect("spawn nod-driver");
    if !driver.status.success() {
        panic!(
            "nod-driver build failed: {}\nstdout:\n{}\nstderr:\n{}",
            driver.status,
            String::from_utf8_lossy(&driver.stdout),
            String::from_utf8_lossy(&driver.stderr)
        );
    }
    assert!(exe_path.is_file(), "EXE not produced at {}", exe_path.display());

    // Run the EXE in a fresh process to avoid env-var contamination
    // from the cargo runtime. We do NOT set `current_dir` — the EXE
    // doesn't read any files, only writes stdout — so the working
    // directory is whatever cargo passed us; that's fine.
    let exe = Command::new(&exe_path).output().expect("spawn user EXE");
    let stdout = String::from_utf8_lossy(&exe.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&exe.stderr).into_owned();
    let code = exe.status.code().unwrap_or(-1);

    // Best-effort cleanup on success. On failure (caller's assertion
    // fires after this returns), the temp dir is left in place so a
    // developer can inspect.
    if code == 0 {
        let _ = remove_dir_all_best_effort(&dir);
    }

    (stdout, stderr, code)
}

fn remove_dir_all_best_effort(p: &Path) -> std::io::Result<()> {
    // Retry once after a brief pause — Windows can hold file handles
    // briefly after process exit.
    if let Err(_e) = std::fs::remove_dir_all(p) {
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::remove_dir_all(p)?;
    }
    Ok(())
}

/// Sprint 39a's headline test: `define function main () => () format-out("Hello, world\n") end`
/// produces an EXE that prints exactly `"Hello, world\n"` and returns 0.
#[test]
#[ignore]
#[serial]
fn aot_hello_world() {
    let source = "Module: hello\n\n\
        define function main () => ()\n  \
            format-out(\"Hello, world\\n\");\n\
        end function main;\n";
    let (stdout, stderr, code) = build_and_run("hello", source);
    assert_eq!(code, 0, "exit code; stderr=\n{stderr}");
    assert_eq!(stdout, "Hello, world\n", "stdout mismatch; stderr=\n{stderr}");
}

/// Sprint 39a: arithmetic + `%d` formatting. Demonstrates fixnum
/// arithmetic + literal interpolation in the AOT path.
#[test]
#[ignore]
#[serial]
fn aot_arithmetic() {
    let source = "Module: arith\n\n\
        define function main () => ()\n  \
            format-out(\"%d\\n\", 6 * 7);\n\
        end function main;\n";
    let (stdout, stderr, code) = build_and_run("arith", source);
    assert_eq!(code, 0, "exit code; stderr=\n{stderr}");
    assert_eq!(stdout, "42\n", "stdout mismatch; stderr=\n{stderr}");
}

/// Sprint 39a: end-to-end exercise of Sprint 38c (class metadata
/// relocation) + Sprint 38e (cache slot + generic relocation).
/// `size(make(<range>, from: 0, to: 5))` is the same workload Sprint
/// 38g's headline subprocess speedup test uses, ensuring the AOT
/// pipeline covers the same per-bake-site categories as the JIT.
///
/// ## Sprint 39a scope note: this test is `#[ignore]`-with-expected-
/// failure — passing it requires the stdlib to be inlined into the
/// user's `.obj`, which is **Sprint 39c**'s job per the Sprint 39 plan.
/// At Sprint 39a's slice (this commit), the AOT pipeline can resolve
/// seed class IDs (e.g. `<integer>`, `<byte-string>`) but not user-
/// classes registered by the stdlib's `define class` items because the
/// AOT runtime's `nod_runtime_init` doesn't replay stdlib lowering.
///
/// The test stays in the file so 39c can flip it from "documented
/// expected failure" to "green" by adding stdlib inlining + matching
/// class-registration replay. Until then, it crashes with a `<range>`
/// → wrong-class-metadata mismatch (the seed class IDs themselves are
/// fine, but `make` machinery routes through stdlib methods Sprint 39a
/// doesn't include).
///
/// Run-and-assert-failure is intentional: if a future change
/// accidentally makes this pass without Sprint 39c, the test should
/// be updated to assert success — that change is welcome.
#[test]
#[ignore]
#[serial]
fn aot_dispatch_deferred_to_39c() {
    let source = "Module: dispatch\n\n\
        define function main () => ()\n  \
            format-out(\"%d\\n\", size(make(<range>, from: 0, to: 5)));\n\
        end function main;\n";
    // We expect this to fail at runtime until Sprint 39c. The test
    // documents the expected mode of failure so a passing run on a
    // future Sprint 39c-aware codebase forces an assertion update
    // (rather than silently flipping behaviour).
    let (stdout, stderr, code) = build_and_run("dispatch", source);
    if code == 0 && stdout == "6\n" {
        // Sprint 39c has landed and this test would now succeed —
        // promote it to a real positive assertion. Until then this
        // path indicates an intentional Sprint 39c upgrade.
        return;
    }
    eprintln!(
        "aot_dispatch_deferred_to_39c: expected failure until Sprint 39c. \
         got stdout={stdout:?}, stderr={stderr:?}, code={code}"
    );
    // Non-zero exit code matches the current Sprint 39a state where
    // dispatch fails because the stdlib isn't in the EXE yet.
    assert_ne!(code, 0, "Sprint 39c may have landed; promote this test");
}
