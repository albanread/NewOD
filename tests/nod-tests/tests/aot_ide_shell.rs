//! Sprint 41a — the IDE-shell demo as a standalone AOT EXE.
//!
//! THIS TEST POPS A REAL WIN32 WINDOW. It is `#[ignore]`-gated so
//! routine `cargo test` doesn't disturb the user's screen. Run
//! manually with:
//!
//! ```text
//! cargo test --test aot_ide_shell -- --ignored --nocapture
//! ```
//!
//! When you run this, a window titled "NewOpenDylan IDE" appears
//! showing "hello, dylan" rendered via DirectWrite through Direct2D
//! into an HWND-bound DXGI swap chain. Click the close box (X) to
//! close it; the test then asserts the EXE exited with code 0.
//!
//! ## What this proves
//!
//! Sprint 36 shipped the JIT-side IDE shell as `ide_shell.rs`. Sprint
//! 41a both completes that JIT test (replacing the `Sleep(5000)`
//! placeholder with a real blocking message loop) AND lifts the same
//! Dylan source body into an AOT-built EXE — the user's "real Windows
//! app" criterion. Every Sprint-39+40 deliverable has to compose for
//! this to work:
//!
//!   * Sprint 39a's `nod_runtime_init` → eager class / condition /
//!     C-FFI-error registration before user main runs.
//!   * Sprint 39b's IAT-resolved Win32 imports → `CreateWindowExW`,
//!     `ShowWindow`, `UpdateWindow`, `DefWindowProcW`, `PostQuitMessage`
//!     all wired via `dllimport` declarations the linker satisfies
//!     out of `user32.lib`.
//!   * Sprint 39c's merged stdlib → the user code's `format-out`-free
//!     body still drags in dispatch metadata for `<integer>` arithmetic
//!     and the `as-wndproc-callback` stdlib helper.
//!   * Sprint 40b's Win32 callbacks → `as-wndproc-callback`'s
//!     `nod_register_wndproc` call lands in the staticlib-linked
//!     `callbacks.rs` trampoline pool, hands back a real C-ABI
//!     function pointer the OS can invoke.
//!   * Sprint 40c's COM in AOT → DXGI / D3D11 / D2D / DirectWrite
//!     factories + device chain + bitmap creation all reachable from
//!     `nod_runtime.lib`.
//!   * Sprint 40d's bare-name Win32 calls → `PostQuitMessage`,
//!     `DefWindowProcW`, `CreateWindowExW`, `ShowWindow`,
//!     `UpdateWindow` are the bare-name path (no explicit
//!     `define c-function`).
//!   * Sprint 41a's `%run-message-loop()` → the standardlib's
//!     newly-added blocking `GetMessage`/`Translate`/`Dispatch` loop
//!     primitive, statically linked into the EXE via the
//!     `nod_run_message_loop` shim in `com_shim.rs`.
//!
//! ## Why explicit `define c-function` declarations
//!
//! The Dylan source below carries the same `define c-function`
//! declarations as the JIT IDE-shell test (`ide_shell.rs`'s
//! `IDE_SHELL_DECL`). Sprint 31's bare-name Win32 materialization
//! works for both the JIT and AOT pipelines, but `CreateWindowExW`'s
//! second arg (`lpClassName`, `LPCWSTR` in `windows_api.db`) gets
//! classified as a string-typed arg by sema; when the test passes the
//! `atom` Word from `%register-window-class` (a fixnum, not a
//! `<byte-string>`), the winffi marshaler panics on string-shape
//! coercion. The JIT test sidestepped this from Sprint 36 by
//! declaring `lpClassName` as `<c-pointer>` (an integer-shaped arg)
//! via an explicit `define c-function`. The AOT test mirrors that
//! exact shape — both pipelines route through the same lowering
//! path for declared c-functions, so what works for JIT works
//! identically for AOT. Tightening sema's bare-name LPCWSTR
//! classification (allowing it to accept integer-shaped args where
//! the parameter is documented as accepting an atom) is a follow-up
//! cleanup, not part of Sprint 41a.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serial_test::serial;

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

fn make_temp_dir(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("nod-aot-ide-test-{test_name}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn remove_dir_all_best_effort(p: &Path) -> std::io::Result<()> {
    if let Err(_e) = std::fs::remove_dir_all(p) {
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::remove_dir_all(p)?;
    }
    Ok(())
}

/// Build the Dylan source to an EXE under a temp dir, return the EXE
/// path. The temp dir is kept for forensic inspection — it's cleaned
/// up by the caller on success only. Panics on build failure.
fn build_exe(test_name: &str, source: &str) -> (PathBuf, PathBuf) {
    let dir = make_temp_dir(test_name);
    let src_path = dir.join("input.dylan");
    let exe_path = dir.join("output.exe");
    std::fs::write(&src_path, source).expect("write source");

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
    assert!(
        exe_path.is_file(),
        "EXE not produced at {}",
        exe_path.display()
    );
    (dir, exe_path)
}

/// **The Sprint 41a headline.** Build an AOT-linked EXE from the same
/// Dylan source the JIT IDE-shell test (`ide_shell.rs`) uses, launch
/// it, and wait for the window to close. The test blocks here until
/// the user clicks the X box on the window; PostQuitMessage(0) inside
/// the WNDPROC's WM_DESTROY handler signals the message loop to exit,
/// the message loop returns 0, the main function returns, and the
/// process exits with code 0.
///
/// Acceptance: the EXE exits with status 0 within a reasonable bound
/// of user patience. The test framework's `.wait()` blocks until the
/// child process exits — so the test only completes after the human
/// has interacted with the window.
///
/// `#[ignore]`-gated because it's interactive (window pops on the
/// user's desktop). `#[serial]` prevents concurrent Cargo build-lock
/// contention with other AOT tests.
#[test]
#[ignore = "interactive: pops a real Win32 window. Run with `cargo test --test aot_ide_shell -- --ignored --nocapture`."]
#[serial]
fn aot_ide_shell_window_renders_hello_dylan() {
    // Identical Dylan body to the JIT test in `ide_shell.rs`, wrapped
    // as a `define function main`. The `%`-prefixed primitives all
    // resolve to staticlib symbols in `nod_runtime.lib`. The explicit
    // `define c-function` declarations match `ide_shell.rs`'s
    // `IDE_SHELL_DECL` — `CreateWindowExW`'s `lpClassName` needs to be
    // typed as `<c-pointer>` (an integer-shaped arg) so the
    // `%register-window-class` atom Word reaches Win32 as a raw int
    // rather than going through the string-marshaling path.
    let source = "Module: ide-shell\n\n\
        define c-function CreateWindowExW\n  \
            (dwExStyle :: <c-int>, lpClassName :: <c-pointer>, lpWindowName :: <c-wide-string>,\n   \
             dwStyle :: <c-int>, x :: <c-int>, y :: <c-int>, nWidth :: <c-int>, nHeight :: <c-int>,\n   \
             hWndParent :: <c-pointer>, hMenu :: <c-pointer>, hInstance :: <c-pointer>,\n   \
             lpParam :: <c-pointer>)\n   \
         => (hwnd :: <c-pointer>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function ShowWindow\n  \
            (hwnd :: <c-pointer>, nCmdShow :: <c-int>)\n   \
         => (was-visible :: <c-bool>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function UpdateWindow\n  \
            (hwnd :: <c-pointer>)\n   \
         => (success :: <c-bool>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function DefWindowProcW\n  \
            (hwnd :: <c-pointer>, msg :: <c-int>,\n   \
             wparam :: <c-pointer>, lparam :: <c-pointer>)\n   \
         => (lresult :: <c-pointer>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function PostQuitMessage\n  \
            (exit-code :: <c-int>)\n   \
         => ();\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define function main () => ()\n  \
            let d3d-device   = %d3d11-create-device();\n  \
            let dxgi-factory = %dxgi-factory-from-d3d-device(d3d-device);\n  \
            let dxgi-device  = %dxgi-device-from-d3d-device(d3d-device);\n  \
            let d2d-factory  = %d2d-create-factory();\n  \
            let d2d-device   = %d2d-create-device(d2d-factory, dxgi-device);\n  \
            let dc           = %d2d-create-device-context(d2d-device);\n  \
            let dwrite       = %dwrite-create-factory();\n  \
            let format       = %dwrite-create-text-format(dwrite, \"Segoe UI\", 2400, \"en-us\");\n  \
            let swap = 0;\n  \
            let bitmap = 0;\n  \
            let wp = method (hwnd, msg, wparam, lparam)\n            \
                       if (msg = 15)\n              \
                         if (swap ~= 0)\n                \
                           if (bitmap = 0)\n                  \
                             bitmap := %d2d-create-bitmap-from-swap-chain(dc, swap);\n                \
                           else 0 end;\n                \
                           %d2d-set-target(dc, bitmap);\n                \
                           %d2d-begin-draw(dc);\n                \
                           %d2d-clear(dc, 255, 255, 255, 255);\n                \
                           let brush  = %d2d-create-solid-color-brush(dc, 0, 0, 0, 255);\n                \
                           let layout = %dwrite-create-text-layout(dwrite, \"hello, dylan\", format, 800, 600);\n                \
                           %d2d-draw-text-layout(dc, 50, 50, layout, brush);\n                \
                           %d2d-end-draw(dc);\n                \
                           %com-release(brush);\n                \
                           %com-release(layout);\n                \
                           %dxgi-swap-chain-present(swap);\n              \
                         else 0 end;\n              \
                         0\n            \
                       elseif (msg = 2)\n              \
                         PostQuitMessage(0);\n              \
                         0\n            \
                       else\n              \
                         DefWindowProcW(hwnd, msg, wparam, lparam)\n            \
                       end\n          \
                     end;\n  \
            let cb = as-wndproc-callback(wp);\n  \
            let atom = %register-window-class(cb, \"NodAotIdeShell\");\n  \
            let hwnd = CreateWindowExW(0, atom, \"NewOpenDylan IDE\",\n                                       \
                13565952, -2147483648, -2147483648, 800, 600,\n                                       \
                0, 0, 0, 0);\n  \
            swap := %dxgi-create-swap-chain-for-hwnd(dxgi-factory, d3d-device, hwnd, 800, 600);\n  \
            ShowWindow(hwnd, 5);\n  \
            UpdateWindow(hwnd);\n  \
            %run-message-loop();\n\
        end function main;\n";
    let (dir, exe_path) = build_exe("ide-shell", source);

    eprintln!(
        "[sprint-41a headline] AOT EXE built at {}; spawning — \
         A WINDOW WILL APPEAR. Click the X to close it. The test will \
         then validate exit code 0.",
        exe_path.display()
    );

    // Spawn the EXE and block until it exits. The user has to close
    // the window manually for `.wait()` to return.
    let mut child = Command::new(&exe_path)
        .spawn()
        .expect("spawn AOT IDE shell EXE");
    let status = child.wait().expect("wait for AOT IDE shell EXE");
    let code = status.code().unwrap_or(-1);
    eprintln!("[sprint-41a headline] AOT IDE shell EXE exited with code {code}");

    assert_eq!(
        code, 0,
        "AOT IDE shell must exit cleanly with code 0 (WM_QUIT received \
         via PostQuitMessage(0) in WM_DESTROY handler); exe={}",
        exe_path.display()
    );

    // Success — clean up the temp dir.
    let _ = remove_dir_all_best_effort(&dir);
}

/// **The Sprint 41b headline.** Build an AOT-linked EXE — "nod-ide" —
/// that opens a window, reads a `.dylan` source file path from `argv[1]`,
/// and renders its contents as monospace text via DirectWrite. The
/// window handles WM_SIZE correctly: dragging the window edge resizes
/// the swap chain, recreates the bitmap, and re-renders without
/// artifacts.
///
/// This is the first step from "Sprint 41a demo" to "I could use this
/// to look at Dylan code." Read-only — no cursor, no scrollbar, no
/// editing. Sprint 41c adds those.
///
/// What this test exercises beyond Sprint 41a:
///   * `%read-file(path)` — new Sprint 41b runtime shim
///     `nod_read_file_to_string` reads bytes off disk, allocates a fresh
///     Dylan `<byte-string>` in the static-area literal pool, returns
///     its Word. Errors return the `nil` immediate.
///   * `%argv1()` — new Sprint 41b runtime shim `nod_get_argv1` reads
///     `std::env::args().nth(1)` and surfaces it as a `<byte-string>`
///     Word (or `nil` if absent).
///   * `%lo-word(v)` / `%hi-word(v)` — minimal bitwise extraction
///     shims for unpacking WM_SIZE's `lparam` (low 16 = width, high 16
///     = height). Dylan currently lacks general `logand`/`ash`
///     primitives; these are the path of least resistance for the
///     Sprint 41b deliverable. A future sprint can promote them to
///     `%logand` / `%ash`.
///   * WM_SIZE handler in the Dylan WNDPROC. On resize we drop the
///     cached D2D bitmap (bound to the old swap-chain dimensions),
///     call `%dxgi-swap-chain-resize-buffers` with the unpacked width
///     and height, and let the next WM_PAINT see `bitmap = 0` and
///     recreate it for the new size.
///
/// `#[ignore]`-gated because it's interactive. The test runner spawns
/// the EXE, the user resizes / closes the window, and the test asserts
/// exit code 0 after `.wait()` returns.
#[test]
#[ignore = "interactive: pops a real Win32 window. Run with `cargo test --test aot_ide_shell -- --ignored --nocapture aot_nod_ide_source_viewer`."]
#[serial]
fn aot_nod_ide_source_viewer() {
    // The Dylan source for `nod-ide.exe`. The first `define c-function`
    // declarations match `aot_ide_shell_window_renders_hello_dylan`
    // verbatim — see that test's docstring for why `lpClassName` must
    // be `<c-pointer>` (an integer-shaped arg) instead of the default
    // string-marshaling path.
    //
    // The structure differs from Sprint 41a in two ways:
    //   1. main reads `%argv1()` then `%read-file(path)`, falling back
    //      to a hardcoded test message if either is absent or fails.
    //   2. The WNDPROC handles WM_SIZE (msg=5): release the cached
    //      D2D bitmap, unpack `lparam` via `%lo-word`/`%hi-word`, call
    //      `%dxgi-swap-chain-resize-buffers`. Next WM_PAINT then sees
    //      `bitmap = 0` and re-creates it at the new size.
    //
    // The render path passes the entire file source as one DirectWrite
    // layout — DWrite handles `\n` line breaks natively, so we don't
    // need a Dylan-side line-splitter. The layout box is set to
    // (width-padding, height-padding) so text wraps to the viewport
    // and clips beyond it. Read-only display, no scrolling.
    let source = "Module: nod-ide\n\n\
        define c-function CreateWindowExW\n  \
            (dwExStyle :: <c-int>, lpClassName :: <c-pointer>, lpWindowName :: <c-wide-string>,\n   \
             dwStyle :: <c-int>, x :: <c-int>, y :: <c-int>, nWidth :: <c-int>, nHeight :: <c-int>,\n   \
             hWndParent :: <c-pointer>, hMenu :: <c-pointer>, hInstance :: <c-pointer>,\n   \
             lpParam :: <c-pointer>)\n   \
         => (hwnd :: <c-pointer>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function ShowWindow\n  \
            (hwnd :: <c-pointer>, nCmdShow :: <c-int>)\n   \
         => (was-visible :: <c-bool>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function UpdateWindow\n  \
            (hwnd :: <c-pointer>)\n   \
         => (success :: <c-bool>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function DefWindowProcW\n  \
            (hwnd :: <c-pointer>, msg :: <c-int>,\n   \
             wparam :: <c-pointer>, lparam :: <c-pointer>)\n   \
         => (lresult :: <c-pointer>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function PostQuitMessage\n  \
            (exit-code :: <c-int>)\n   \
         => ();\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define function main () => ()\n  \
            let arg-path = %argv1();\n  \
            let source-text = if (empty?(arg-path))\n                      \
                                \"nod-ide: no argv[1] supplied; pass a Dylan source path as the first argument.\"\n                    \
                              else\n                      \
                                let bytes = %read-file(arg-path);\n                      \
                                if (empty?(bytes))\n                        \
                                  \"nod-ide: could not read the file passed via argv[1].\"\n                      \
                                else\n                        \
                                  bytes\n                      \
                                end\n                    \
                              end;\n  \
            let d3d-device   = %d3d11-create-device();\n  \
            let dxgi-factory = %dxgi-factory-from-d3d-device(d3d-device);\n  \
            let dxgi-device  = %dxgi-device-from-d3d-device(d3d-device);\n  \
            let d2d-factory  = %d2d-create-factory();\n  \
            let d2d-device   = %d2d-create-device(d2d-factory, dxgi-device);\n  \
            let dc           = %d2d-create-device-context(d2d-device);\n  \
            let dwrite       = %dwrite-create-factory();\n  \
            let format       = %dwrite-create-text-format(dwrite, \"Consolas\", 1400, \"en-us\");\n  \
            let swap = 0;\n  \
            let bitmap = 0;\n  \
            let width = 1024;\n  \
            let height = 768;\n  \
            let wp = method (hwnd, msg, wparam, lparam)\n            \
                       if (msg = 15)\n              \
                         if (swap ~= 0)\n                \
                           if (bitmap = 0)\n                  \
                             bitmap := %d2d-create-bitmap-from-swap-chain(dc, swap);\n                \
                           else 0 end;\n                \
                           %d2d-set-target(dc, bitmap);\n                \
                           %d2d-begin-draw(dc);\n                \
                           %d2d-clear(dc, 255, 255, 255, 255);\n                \
                           let brush  = %d2d-create-solid-color-brush(dc, 0, 0, 0, 255);\n                \
                           let layout = %dwrite-create-text-layout(dwrite, source-text, format, width, height);\n                \
                           %d2d-draw-text-layout(dc, 8, 8, layout, brush);\n                \
                           %d2d-end-draw(dc);\n                \
                           %com-release(brush);\n                \
                           %com-release(layout);\n                \
                           %dxgi-swap-chain-present(swap);\n              \
                         else 0 end;\n              \
                         0\n            \
                       elseif (msg = 5)\n              \
                         if (swap ~= 0 & wparam ~= 1)\n                \
                           let new-w = %lo-word(lparam);\n                \
                           let new-h = %hi-word(lparam);\n                \
                           if (new-w > 0 & new-h > 0)\n                  \
                             if (bitmap ~= 0)\n                    \
                               %d2d-set-target(dc, 0);\n                    \
                               %com-release(bitmap);\n                    \
                               bitmap := 0;\n                  \
                             else 0 end;\n                  \
                             width := new-w;\n                  \
                             height := new-h;\n                  \
                             %dxgi-swap-chain-resize-buffers(swap, new-w, new-h);\n                \
                           else 0 end;\n              \
                         else 0 end;\n              \
                         0\n            \
                       elseif (msg = 2)\n              \
                         PostQuitMessage(0);\n              \
                         0\n            \
                       else\n              \
                         DefWindowProcW(hwnd, msg, wparam, lparam)\n            \
                       end\n          \
                     end;\n  \
            let cb = as-wndproc-callback(wp);\n  \
            let atom = %register-window-class(cb, \"NodIDE\");\n  \
            let hwnd = CreateWindowExW(0, atom, \"NewOpenDylan IDE\",\n                                       \
                13565952, -2147483648, -2147483648, 1024, 768,\n                                       \
                0, 0, 0, 0);\n  \
            swap := %dxgi-create-swap-chain-for-hwnd(dxgi-factory, d3d-device, hwnd, 1024, 768);\n  \
            ShowWindow(hwnd, 5);\n  \
            UpdateWindow(hwnd);\n  \
            %run-message-loop();\n\
        end function main;\n";

    let (dir, exe_path) = build_exe("nod-ide", source);

    // Write a small Dylan source fixture into the build dir so the
    // test doesn't depend on any workspace-relative path being correct
    // at runtime. The EXE reads this via `%argv1()` → `%read-file(path)`.
    let fixture_path = dir.join("sample.dylan");
    let fixture_content = "Module: sample\n\n\
        // sample.dylan — Sprint 41b fixture for nod-ide.exe\n\
        //\n\
        // The IDE reads this file at startup and renders its contents\n\
        // via DirectWrite. Resize the window — text should re-render\n\
        // at the new viewport size without artifacts.\n\n\
        define function greet () => ()\n  \
            format-out(\"hello from a real Dylan source file!\\n\");\n\
        end function;\n\n\
        define function add (a, b) => (sum)\n  \
            a + b\n\
        end function;\n\n\
        define function main () => ()\n  \
            greet();\n  \
            format-out(\"2 + 3 = %d\\n\", add(2, 3));\n\
        end function main;\n";
    std::fs::write(&fixture_path, fixture_content).expect("write sample fixture");

    eprintln!(
        "[sprint-41b headline] AOT nod-ide EXE built at {}; spawning with \
         argv[1] = {}.\n  \
         A WINDOW WILL APPEAR showing the file's source. \
         RESIZE THE WINDOW (drag the corner / edge) — text should re-render \
         at the new size without artifacts or crashes.\n  \
         Click X to close. The test will then validate exit code 0.",
        exe_path.display(),
        fixture_path.display(),
    );

    // Spawn the EXE and block until it exits. The user resizes the
    // window then closes it.
    let mut child = Command::new(&exe_path)
        .arg(&fixture_path)
        .spawn()
        .expect("spawn AOT nod-ide EXE");
    let status = child.wait().expect("wait for AOT nod-ide EXE");
    let code = status.code().unwrap_or(-1);
    eprintln!("[sprint-41b headline] AOT nod-ide EXE exited with code {code}");

    assert_eq!(
        code, 0,
        "AOT nod-ide EXE must exit cleanly with code 0 (WM_QUIT received \
         via PostQuitMessage(0) in WM_DESTROY handler); exe={}",
        exe_path.display()
    );

    // Success — clean up the temp dir.
    let _ = remove_dir_all_best_effort(&dir);
}

/// **The Sprint 41c headline.** Build an AOT-linked EXE — the scrollable
/// nod-ide — and exercise the native resize + scroll feel.
///
/// Differences vs. Sprint 41b (`aot_nod_ide_source_viewer`):
///
///   * Swap-chain `Scaling: DXGI_SCALING_NONE` (set inside
///     `com_shim.rs`). With STRETCH (the Sprint 41b default), dragging
///     the window edge visually stretches the rendered text until
///     ResizeBuffers refreshes the back buffer. With NONE, the back
///     buffer stays at native size; the OS fills the newly-exposed
///     region with the window's background brush — exactly the
///     Notepad++ feel.
///   * `WS_VSCROLL` in the window's dwStyle. The scrollbar appears on
///     the right edge of the window; its range and proportional thumb
///     come from the new `%set-scroll-info` Dylan primitive.
///   * New WNDPROC handlers for WM_VSCROLL, WM_MOUSEWHEEL, WM_KEYDOWN
///     (PgUp/PgDn/Home/End — arrow keys are deferred to Sprint 41d
///     when there's a cursor to move).
///   * WM_PAINT renders the text translated by the current scroll
///     position; lines off-viewport fall off the top/bottom and
///     Direct2D clips them.
///   * On WM_SIZE we now also recompute `viewport-lines` and call
///     `%set-scroll-info` to keep the thumb proportional to the new
///     window height.
///
/// What this test exercises end-to-end:
///   * The Sprint 41c runtime shims `nod_set_scroll_info`,
///     `nod_get_scroll_pos`, `nod_count_newlines` — all reachable from
///     `nod_runtime.lib` linked into the EXE.
///   * The Sprint 41c codegen wiring (lowering + JIT-symbol bindings)
///     surfaces correctly through the AOT pipeline.
///   * The Sprint 41c Dylan source compiles, links, and runs as an EXE
///     that opens a scrollable read-only Dylan viewer.
///
/// `#[ignore]`-gated because it's interactive. The test runner spawns
/// the EXE, the user scrolls / resizes / closes, and the test asserts
/// exit code 0 after `.wait()` returns.
#[test]
#[ignore = "interactive: pops a real Win32 window. Run with `cargo test --test aot_ide_shell -- --ignored --nocapture aot_nod_ide_scrollable_source_viewer`."]
#[serial]
fn aot_nod_ide_scrollable_source_viewer() {
    // The Sprint 41c Dylan source body. Mirrors
    // `tests/nod-tests/fixtures/nod-ide.dylan` — that fixture is the
    // standalone source you could `nod-driver build` directly; the
    // body below embeds the same Dylan code via a string literal so
    // the test is self-contained.
    let source = "Module: nod-ide\n\n\
        define c-function CreateWindowExW\n  \
            (dwExStyle :: <c-int>, lpClassName :: <c-pointer>, lpWindowName :: <c-wide-string>,\n   \
             dwStyle :: <c-int>, x :: <c-int>, y :: <c-int>, nWidth :: <c-int>, nHeight :: <c-int>,\n   \
             hWndParent :: <c-pointer>, hMenu :: <c-pointer>, hInstance :: <c-pointer>,\n   \
             lpParam :: <c-pointer>)\n   \
         => (hwnd :: <c-pointer>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function ShowWindow\n  \
            (hwnd :: <c-pointer>, nCmdShow :: <c-int>)\n   \
         => (was-visible :: <c-bool>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function UpdateWindow\n  \
            (hwnd :: <c-pointer>)\n   \
         => (success :: <c-bool>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function InvalidateRect\n  \
            (hwnd :: <c-pointer>, lpRect :: <c-pointer>, bErase :: <c-bool>)\n   \
         => (success :: <c-bool>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function DefWindowProcW\n  \
            (hwnd :: <c-pointer>, msg :: <c-int>,\n   \
             wparam :: <c-pointer>, lparam :: <c-pointer>)\n   \
         => (lresult :: <c-pointer>);\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define c-function PostQuitMessage\n  \
            (exit-code :: <c-int>)\n   \
         => ();\n    \
            library: \"user32.dll\";\n\
        end;\n\n\
        define function main () => ()\n  \
            let arg-path = %argv1();\n  \
            let source-text = if (empty?(arg-path))\n                      \
                                \"nod-ide: no argv[1] supplied; pass a Dylan source path as the first argument.\"\n                    \
                              else\n                      \
                                let bytes = %read-file(arg-path);\n                      \
                                if (empty?(bytes))\n                        \
                                  \"nod-ide: could not read the file passed via argv[1].\"\n                      \
                                else\n                        \
                                  bytes\n                      \
                                end\n                    \
                              end;\n  \
            let d3d-device   = %d3d11-create-device();\n  \
            let dxgi-factory = %dxgi-factory-from-d3d-device(d3d-device);\n  \
            let dxgi-device  = %dxgi-device-from-d3d-device(d3d-device);\n  \
            let d2d-factory  = %d2d-create-factory();\n  \
            let d2d-device   = %d2d-create-device(d2d-factory, dxgi-device);\n  \
            let dc           = %d2d-create-device-context(d2d-device);\n  \
            let dwrite       = %dwrite-create-factory();\n  \
            let format       = %dwrite-create-text-format(dwrite, \"Consolas\", 1400, \"en-us\");\n  \
            let swap = 0;\n  \
            let bitmap = 0;\n  \
            let width = 1024;\n  \
            let height = 768;\n  \
            let line-height = 18;\n  \
            let line-count = %count-newlines(source-text);\n  \
            let scroll-y-line = 0;\n  \
            let viewport-lines = 768 / 18;\n  \
            let wp = method (hwnd, msg, wparam, lparam)\n            \
                       if (msg = 15)\n              \
                         if (swap ~= 0)\n                \
                           if (bitmap = 0)\n                  \
                             bitmap := %d2d-create-bitmap-from-swap-chain(dc, swap);\n                \
                           else 0 end;\n                \
                           %d2d-set-target(dc, bitmap);\n                \
                           %d2d-begin-draw(dc);\n                \
                           %d2d-clear(dc, 255, 255, 255, 255);\n                \
                           let brush  = %d2d-create-solid-color-brush(dc, 0, 0, 0, 255);\n                \
                           let layout = %dwrite-create-text-layout(dwrite, source-text, format, width, height + scroll-y-line * line-height);\n                \
                           %d2d-draw-text-layout(dc, 8, 8 - scroll-y-line * line-height, layout, brush);\n                \
                           %d2d-end-draw(dc);\n                \
                           %com-release(brush);\n                \
                           %com-release(layout);\n                \
                           %dxgi-swap-chain-present(swap);\n              \
                         else 0 end;\n              \
                         0\n            \
                       elseif (msg = 5)\n              \
                         if (swap ~= 0 & wparam ~= 1)\n                \
                           let new-w = %lo-word(lparam);\n                \
                           let new-h = %hi-word(lparam);\n                \
                           if (new-w > 0 & new-h > 0)\n                  \
                             if (bitmap ~= 0)\n                    \
                               %d2d-set-target(dc, 0);\n                    \
                               %com-release(bitmap);\n                    \
                               bitmap := 0;\n                  \
                             else 0 end;\n                  \
                             width := new-w;\n                  \
                             height := new-h;\n                  \
                             %dxgi-swap-chain-resize-buffers(swap, new-w, new-h);\n                  \
                             viewport-lines := new-h / line-height;\n                  \
                             %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, scroll-y-line, 1);\n                \
                           else 0 end;\n              \
                         else 0 end;\n              \
                         0\n            \
                       elseif (msg = 277)\n              \
                         let action = %lo-word(wparam);\n              \
                         let new-pos = if (action = 0)\n                                         \
                                         scroll-y-line - 1\n                                       \
                                       elseif (action = 1)\n                                         \
                                         scroll-y-line + 1\n                                       \
                                       elseif (action = 2)\n                                         \
                                         scroll-y-line - (viewport-lines - 1)\n                                       \
                                       elseif (action = 3)\n                                         \
                                         scroll-y-line + (viewport-lines - 1)\n                                       \
                                       elseif (action = 4)\n                                         \
                                         %hi-word(wparam)\n                                       \
                                       elseif (action = 5)\n                                         \
                                         %hi-word(wparam)\n                                       \
                                       elseif (action = 6)\n                                         \
                                         0\n                                       \
                                       elseif (action = 7)\n                                         \
                                         line-count - viewport-lines\n                                       \
                                       else\n                                         \
                                         scroll-y-line\n                                       \
                                       end;\n              \
                         let max-scroll = if (line-count > viewport-lines)\n                                            \
                                            line-count - viewport-lines\n                                          \
                                          else 0 end;\n              \
                         let clamped = if (new-pos < 0) 0\n                                       \
                                       elseif (new-pos > max-scroll) max-scroll\n                                       \
                                       else new-pos end;\n              \
                         if (clamped ~= scroll-y-line)\n                \
                           scroll-y-line := clamped;\n                \
                           %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, clamped, 1);\n                \
                           InvalidateRect(hwnd, 0, 0);\n              \
                         else 0 end;\n              \
                         0\n            \
                       elseif (msg = 522)\n              \
                         let raw-delta = %hi-word(wparam);\n              \
                         let signed-delta = if (raw-delta > 32767)\n                                              \
                                              raw-delta - 65536\n                                            \
                                            else\n                                              \
                                              raw-delta\n                                            \
                                            end;\n              \
                         let lines-to-scroll = -1 * signed-delta * 3 / 120;\n              \
                         let new-pos = scroll-y-line + lines-to-scroll;\n              \
                         let max-scroll = if (line-count > viewport-lines)\n                                            \
                                            line-count - viewport-lines\n                                          \
                                          else 0 end;\n              \
                         let clamped = if (new-pos < 0) 0\n                                       \
                                       elseif (new-pos > max-scroll) max-scroll\n                                       \
                                       else new-pos end;\n              \
                         if (clamped ~= scroll-y-line)\n                \
                           scroll-y-line := clamped;\n                \
                           %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, clamped, 1);\n                \
                           InvalidateRect(hwnd, 0, 0);\n              \
                         else 0 end;\n              \
                         0\n            \
                       elseif (msg = 256)\n              \
                         let vk = %lo-word(wparam);\n              \
                         let max-scroll = if (line-count > viewport-lines)\n                                            \
                                            line-count - viewport-lines\n                                          \
                                          else 0 end;\n              \
                         let new-pos = if (vk = 33)\n                                         \
                                         scroll-y-line - (viewport-lines - 1)\n                                       \
                                       elseif (vk = 34)\n                                         \
                                         scroll-y-line + (viewport-lines - 1)\n                                       \
                                       elseif (vk = 36)\n                                         \
                                         0\n                                       \
                                       elseif (vk = 35)\n                                         \
                                         max-scroll\n                                       \
                                       else\n                                         \
                                         scroll-y-line\n                                       \
                                       end;\n              \
                         let clamped = if (new-pos < 0) 0\n                                       \
                                       elseif (new-pos > max-scroll) max-scroll\n                                       \
                                       else new-pos end;\n              \
                         if (clamped ~= scroll-y-line)\n                \
                           scroll-y-line := clamped;\n                \
                           %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, clamped, 1);\n                \
                           InvalidateRect(hwnd, 0, 0);\n              \
                         else 0 end;\n              \
                         0\n            \
                       elseif (msg = 2)\n              \
                         PostQuitMessage(0);\n              \
                         0\n            \
                       else\n              \
                         DefWindowProcW(hwnd, msg, wparam, lparam)\n            \
                       end\n          \
                     end;\n  \
            let cb = as-wndproc-callback(wp);\n  \
            let atom = %register-window-class(cb, \"NodIDE\");\n  \
            let hwnd = CreateWindowExW(0, atom, \"NewOpenDylan IDE\",\n                                       \
                15663104, -2147483648, -2147483648, 1024, 768,\n                                       \
                0, 0, 0, 0);\n  \
            swap := %dxgi-create-swap-chain-for-hwnd(dxgi-factory, d3d-device, hwnd, 1024, 768);\n  \
            %set-scroll-info(hwnd, 1, 0, line-count, viewport-lines, 0, 1);\n  \
            ShowWindow(hwnd, 5);\n  \
            UpdateWindow(hwnd);\n  \
            %run-message-loop();\n\
        end function main;\n";

    let (dir, exe_path) = build_exe("nod-ide-scrollable", source);

    // Write a tall Dylan source fixture (bigger than the default
    // viewport) so the scrollbar has something to scroll. The
    // repeated-block pad makes the file ~80 lines, well past the
    // ~42 lines that fit in a 1024x768 default window.
    let fixture_path = dir.join("tall-sample.dylan");
    let mut fixture_content = String::from(
        "Module: tall-sample\n\n\
         // tall-sample.dylan - Sprint 41c fixture for the scrollable nod-ide.\n\
         //\n\
         // This file is deliberately taller than the default 1024x768\n\
         // window so the scrollbar engages immediately on open. Resize\n\
         // the window - the text should stay at its native size (no\n\
         // stretching) and the scrollbar thumb should grow / shrink\n\
         // proportionally. Spin the mouse wheel, drag the scrollbar\n\
         // thumb, press PgUp / PgDn / Home / End - all should scroll.\n\n\
         define function repeated-block-1 () => ()\n  \
             format-out(\"block 1: this line is part of the long sample.\\n\");\n\
         end function;\n\n",
    );
    for i in 2..=18 {
        fixture_content.push_str(&format!(
            "define function repeated-block-{i} () => ()\n  \
                 format-out(\"block {i}: this line is part of the long sample.\\n\");\n\
             end function;\n\n",
        ));
    }
    fixture_content.push_str(
        "define function main () => ()\n  \
             repeated-block-1();\n  \
             // ... and so on.\n\
         end function main;\n",
    );
    std::fs::write(&fixture_path, fixture_content).expect("write tall sample fixture");

    eprintln!(
        "[sprint-41c headline] AOT scrollable nod-ide EXE built at {}; spawning with \
         argv[1] = {}.\n  \
         A WINDOW WILL APPEAR showing the file's source with a vertical \
         scrollbar on the right.\n  \
         * RESIZE the window - text should NOT stretch; you should just \
           see more or less of the file.\n  \
         * Spin the MOUSE WHEEL - text should scroll up/down by 3 lines per notch.\n  \
         * Press PgUp / PgDn / Home / End - should page-scroll / jump to top/bottom.\n  \
         * Click / drag the SCROLLBAR - should scroll the viewport.\n  \
         Click X to close. The test will then validate exit code 0.",
        exe_path.display(),
        fixture_path.display(),
    );

    let mut child = Command::new(&exe_path)
        .arg(&fixture_path)
        .spawn()
        .expect("spawn AOT nod-ide (scrollable) EXE");
    let status = child.wait().expect("wait for AOT nod-ide (scrollable) EXE");
    let code = status.code().unwrap_or(-1);
    eprintln!(
        "[sprint-41c headline] AOT nod-ide (scrollable) EXE exited with code {code}"
    );

    assert_eq!(
        code, 0,
        "AOT nod-ide (scrollable) EXE must exit cleanly with code 0; exe={}",
        exe_path.display()
    );

    let _ = remove_dir_all_best_effort(&dir);
}

/// **The Sprint 41d headline.** Build an AOT-linked EXE — the
/// horizontally-scrollable nod-ide — and exercise the corrected editor
/// model.
///
/// Differences vs. Sprint 41c (`aot_nod_ide_scrollable_source_viewer`):
///
///   * **Client area is BUFFER-sized, not window-sized.** The rendered
///     canvas is `(buffer-max-cols × char-width)` wide by
///     `(buffer-lines × line-height)` tall. Resizing the OS window
///     never changes it.
///   * **Horizontal scrollbar.** `WS_HSCROLL` (0x00100000) added to
///     dwStyle (now 16711680). New SB_HORZ (nbar=0) configuration via
///     the existing `%set-scroll-info`.
///   * **Pixel-based scroll offsets.** `scroll-x-px` / `scroll-y-px`
///     replace the line-counter from 41c. Drawing translation is
///     `(pad - scroll-x-px, pad - scroll-y-px)` — same coordinate
///     system as the scrollbar ranges.
///   * **WM_HSCROLL (276) handler.** Mirrors WM_VSCROLL for the
///     horizontal axis.
///   * **Shift+MouseWheel = horizontal scroll** (matches IE / Edge /
///     Notepad++ convention).
///   * **VK_LEFT (37) / VK_RIGHT (39)** scroll horizontally one
///     char-width.
///
/// What this test exercises beyond Sprint 41c:
///   * The new `nod_max_line_chars` runtime shim — the only new runtime
///     addition Sprint 41d makes (one helper, parallel to
///     `nod_count_newlines`).
///   * The corrected editor model end-to-end: buffer / client / window
///     all distinct.
///
/// Test fixture lives under `F:\scratch` per the project hard rule that
/// test inputs must never live inside the NewOpenDylan repo. The
/// fixture has deliberately long lines so the horizontal scrollbar
/// engages on open.
///
/// `#[ignore]`-gated because it's interactive. The test runner spawns
/// the EXE, the user scrolls / resizes / closes, and the test asserts
/// exit code 0 after `.wait()` returns.
#[test]
#[ignore = "interactive: pops a real Win32 window. Run with `cargo test --test aot_ide_shell -- --ignored --nocapture aot_nod_ide_horizontal_scroll`."]
#[serial]
fn aot_nod_ide_horizontal_scroll() {
    // The Sprint 41d Dylan body — pulled in from the standalone fixture
    // file so the test and the `nod-driver build`-able source stay in
    // lockstep automatically. (Sprint 41c embedded the body via an
    // inline string literal which had to be hand-synchronised with the
    // fixture; `include_str!` removes that footgun.)
    let source = include_str!("../fixtures/nod-ide.dylan");

    let (dir, exe_path) = build_exe("nod-ide-hscroll", source);

    // Sprint 41d hard rule: test fixtures live under F:\scratch, NEVER
    // inside the NewOpenDylan repo. Create the directory if it's
    // missing (Windows users may not have F:\ populated); the test is
    // already #[ignore]-gated so an absent F:\ drive just means the
    // test never runs on that machine.
    let scratch_root = PathBuf::from(r"F:\scratch");
    if !scratch_root.is_dir() {
        eprintln!(
            "[sprint-41d headline] F:\\scratch is missing on this machine — \
             this test requires F:\\scratch to exist (hard project rule: \
             test inputs live there, not inside the repo). Skipping."
        );
        return;
    }
    let fixture_path = scratch_root.join("nod-ide-41d-long-lines.dylan");
    // A fixture with intentionally long lines (well past the 1024-px
    // window width) so the horizontal scrollbar engages immediately.
    // Mix of long signatures, comment banners, and a tall block so
    // both scrollbars do real work.
    let long_line_a =
        "// This is a deliberately long comment line designed to exceed the default 1024 pixel window width so the horizontal scrollbar engages immediately on open; you should be able to drag it to reveal more text.";
    let long_line_b =
        "define function very-long-function-name-that-keeps-going-and-going (parameter-one, parameter-two, parameter-three, parameter-four, parameter-five, parameter-six) => (result-with-a-long-name)";
    let mut fixture_content = String::from(
        "Module: long-lines-sample\n\n\
         // long-lines-sample.dylan - Sprint 41d fixture for the\n\
         // horizontally-scrollable nod-ide.\n\
         //\n\
         // Some lines below are intentionally wider than the default\n\
         // 1024 px window so the horizontal scrollbar engages on open.\n\
         // Total height is also taller than the window so the vertical\n\
         // scrollbar engages too. Both should be usable simultaneously.\n\n",
    );
    fixture_content.push_str(long_line_a);
    fixture_content.push('\n');
    fixture_content.push_str(long_line_b);
    fixture_content.push('\n');
    fixture_content.push('\n');
    for i in 1..=20 {
        fixture_content.push_str(&format!(
            "define function block-{i} () => ()\n  \
                 format-out(\"block {i}: short line.\\n\");\n\
             end function;\n\n",
        ));
    }
    fixture_content.push_str(long_line_a);
    fixture_content.push('\n');
    fixture_content.push_str(long_line_b);
    fixture_content.push('\n');
    fixture_content.push_str(
        "\n\
         define function main () => ()\n  \
             block-1();\n\
         end function main;\n",
    );
    std::fs::write(&fixture_path, fixture_content).expect("write long-lines fixture");

    eprintln!(
        "[sprint-41d headline] AOT horizontally-scrollable nod-ide EXE built at {}; \
         spawning with argv[1] = {}.\n  \
         A WINDOW WILL APPEAR with BOTH a vertical scrollbar on the right \
         and a horizontal scrollbar at the bottom.\n  \
         * The window opens with horizontal scrollbar present (long lines wider than the window).\n  \
         * DRAG the HORIZONTAL scrollbar - text scrolls left/right; long lines reveal their tails.\n  \
         * SHIFT+MOUSEWHEEL - text scrolls left/right (3 chars per notch).\n  \
         * LEFT / RIGHT ARROW KEYS - text scrolls one char-width left/right.\n  \
         * RESIZE the window NARROWER - horizontal scrollbar thumb shrinks (more canvas hidden); text does NOT reflow.\n  \
         * RESIZE the window WIDER (past the longest line) - horizontal scrollbar thumb fills the bar; no more horizontal scroll needed.\n  \
         * Vertical scrolling (wheel / PgUp / PgDn / Home / End / drag) still works from Sprint 41c.\n  \
         Click X to close. The test will then validate exit code 0.",
        exe_path.display(),
        fixture_path.display(),
    );

    let mut child = Command::new(&exe_path)
        .arg(&fixture_path)
        .spawn()
        .expect("spawn AOT nod-ide (hscroll) EXE");
    let status = child.wait().expect("wait for AOT nod-ide (hscroll) EXE");
    let code = status.code().unwrap_or(-1);
    eprintln!(
        "[sprint-41d headline] AOT nod-ide (hscroll) EXE exited with code {code}"
    );

    assert_eq!(
        code, 0,
        "AOT nod-ide (hscroll) EXE must exit cleanly with code 0; exe={}",
        exe_path.display()
    );

    let _ = remove_dir_all_best_effort(&dir);
    // Leave the F:\scratch fixture in place so the user can rerun the
    // EXE manually after the test. It's tiny (a few KB) and harmless.
}
