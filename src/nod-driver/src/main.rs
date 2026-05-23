//! NewOpenDylan compiler driver.
//!
//! Sprint 02: `dump-tokens` lights up. `compile` and `repl` are still
//! stubs; they land in later sprints.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// LLVM major version this driver is targeted against. Read at
/// `--version` time; the inkwell linkage itself lights up in Sprint 06.
const LLVM_VERSION: &str = "22.1";

/// NewOpenDylan compiler driver.
#[derive(Parser)]
#[command(
    name = "nod-driver",
    version = env!("CARGO_PKG_VERSION"),
    long_version = concat!(env!("CARGO_PKG_VERSION"), " (LLVM 22.1)"),
    about = "NewOpenDylan compiler driver",
    long_about = "NewOpenDylan: a from-scratch Rust+LLVM JIT for the Dylan language.\n\
                  See PLAN.md and SPRINTS.md in the workspace root.",
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Compile a Dylan source file or LID-rooted library. Not yet implemented.
    Compile {
        /// Path to a `.dylan` file or a `.lid` library manifest.
        input: Option<PathBuf>,
    },
    /// Sprint 39a — compile a Dylan source file to a standalone Windows
    /// EXE. Pipeline: parse → expand → lower → codegen → AOT entry-stub
    /// injection → emit `.obj` → link against `nod_runtime.lib`.
    ///
    /// Out of scope for Sprint 39a: `define c-function` declarations
    /// (Sprint 39b lands Win32-import handling), stdlib pre-compilation
    /// (Sprint 39c). Programs that use either feature will fail to
    /// link with a missing-symbol error from `link.exe`.
    Build {
        /// Path to a `.dylan` source file with a `define function main`.
        input: PathBuf,
        /// Output EXE path. Defaults to `<input stem>.exe`.
        #[arg(short = 'o', long = "output")]
        output: Option<PathBuf>,
        /// Print the chosen target triple, object path, and linker
        /// command before invoking it.
        #[arg(long = "verbose")]
        verbose: bool,
    },
    /// Start an interactive REPL. Not yet implemented.
    Repl,
    /// Lex a Dylan source file and print the token stream.
    ///
    /// Output format is fixed by `specs/01-lexer.md` §5 — line-oriented,
    /// stable, suitable for diffing.
    DumpTokens {
        /// Path to a `.dylan` source file.
        input: PathBuf,
    },
    /// Lex + parse a Dylan source file and print the AST.
    DumpAst {
        /// Path to a `.dylan` source file.
        input: PathBuf,
    },
    /// Load a `.lid` (resolving any `LID:` include chain) and print the
    /// library/module graph as Graphviz.
    DumpGraph {
        /// Path to a `.lid` file.
        input: PathBuf,
    },
    /// Lex + parse + lower a Dylan source file and print the DFM IR.
    DumpDfm {
        /// Path to a `.dylan` source file.
        input: PathBuf,
    },
    /// Lex + parse + lower + codegen a Dylan source file; print textual LLVM IR.
    DumpLlvm {
        /// Path to a `.dylan` source file.
        input: PathBuf,
    },
    /// Parse + lower + codegen + JIT one Dylan expression; print the result.
    Eval {
        /// Dylan expression source.
        expr: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        None => {
            println!(
                "nod-driver {} (LLVM {LLVM_VERSION})",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        Some(Command::Compile { input }) => {
            let target = input
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<no input>".into());
            eprintln!("nod-driver compile: not yet implemented (input: {target})");
            ExitCode::from(2)
        }
        Some(Command::Build { input, output, verbose }) => {
            let out = output.unwrap_or_else(|| default_exe_path(&input));
            run_build(&input, &out, verbose)
        }
        Some(Command::Repl) => {
            eprintln!("nod-driver repl: not yet implemented (see Sprint 08).");
            ExitCode::from(2)
        }
        Some(Command::DumpTokens { input }) => run_dump_tokens(&input),
        Some(Command::DumpAst { input }) => run_dump_ast(&input),
        Some(Command::DumpGraph { input }) => run_dump_graph(&input),
        Some(Command::DumpDfm { input }) => run_dump_dfm(&input),
        Some(Command::DumpLlvm { input }) => run_dump_llvm(&input),
        Some(Command::Eval { expr }) => run_eval(&expr),
    }
}

// ─── Sprint 39a `build` subcommand ────────────────────────────────────────
//
// End-to-end: source.dylan → .obj → link.exe → exe. See PLAN.md /
// SPRINTS.md for the full Sprint 39 scope. The pipeline below stays
// minimal: no -O dial, no cross-compile, no incremental builds. A
// future sprint can layer those on without disturbing the shape here.

/// Default `<input stem>.exe` next to the input file. Mirrors `rustc`'s
/// behaviour when `-o` is omitted.
fn default_exe_path(input: &std::path::Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("a");
    let mut p = input.to_path_buf();
    p.set_file_name(format!("{stem}.exe"));
    p
}

/// Locate the `nod_runtime.lib` staticlib that Sprint 39a Phase A's
/// `[lib] crate-type = ["rlib", "staticlib"]` setting produces. We
/// look in the workspace's `target/<profile>/` directory.
///
/// **Profile selection**: prefer `target/debug/nod_runtime.lib` for
/// fastest iteration; a future sprint can add `--release`. The build
/// caller is responsible for ensuring nod-runtime has been compiled
/// (the easiest way: `cargo build -p nod-runtime` before invoking
/// `nod build`).
///
/// Returns `Err` if the staticlib isn't where we expect — a clearer
/// error than `link.exe` blowing up with "library not found".
fn locate_runtime_staticlib() -> Result<PathBuf, String> {
    // Allow override via env var so CI / tests can pin a specific
    // build directory.
    if let Ok(p) = std::env::var("NOD_RUNTIME_LIB") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!(
            "NOD_RUNTIME_LIB={} but file does not exist",
            p.display()
        ));
    }
    // The driver runs from anywhere; walk up from `current_exe` to the
    // workspace root. Cargo lays out test/CI/run binaries in the same
    // `target/<profile>/` directory, so `current_exe().parent()` is
    // where we expect to find `nod_runtime.lib` alongside the driver
    // itself.
    let exe = std::env::current_exe()
        .map_err(|e| format!("current_exe(): {e}"))?;
    let cargo_target = exe
        .parent()
        .ok_or_else(|| "current_exe has no parent".to_string())?;
    let direct = cargo_target.join("nod_runtime.lib");
    if direct.is_file() {
        return Ok(direct);
    }
    // Fall back to walking up: a manually-run `cargo run --bin nod` puts
    // the binary in `target/debug/`, the runtime artifact is right there;
    // `cargo test` puts the test binary in `target/debug/deps/` and the
    // runtime is one level up.
    let mut cursor = Some(cargo_target);
    while let Some(dir) = cursor {
        let candidate = dir.join("nod_runtime.lib");
        if candidate.is_file() {
            return Ok(candidate);
        }
        cursor = dir.parent();
    }
    Err(format!(
        "could not locate nod_runtime.lib (searched from {}). \
         Build it with: `cargo build -p nod-runtime` \
         (or set NOD_RUNTIME_LIB=/path/to/nod_runtime.lib).",
        cargo_target.display()
    ))
}

fn run_build(input: &std::path::Path, output: &std::path::Path, verbose: bool) -> ExitCode {
    use nod_llvm::LlvmContext as Context;
    use nod_llvm::OptimizationLevel;

    // Step 1 — front-end pipeline. `compile_file_for_aot` ensures
    // stdlib is loaded, parses, macro-expands, lowers.
    let lm = match nod_sema::compile_file_for_aot(input) {
        Ok(lm) => lm,
        Err(e) => {
            eprintln!("nod build: {e}");
            return ExitCode::from(1);
        }
    };
    // Sprint 39a doesn't support `define c-function` yet — that's
    // Sprint 39b. Fail fast with a clear message rather than letting
    // `link.exe` surface a cryptic "unresolved external".
    if !lm.c_function_stub_table.is_empty() {
        eprintln!(
            "nod build: `define c-function` is not supported in Sprint 39a \
             (the user's program declares {} c-function bindings). \
             Win32 imports are scheduled for Sprint 39b.",
            lm.c_function_stub_table.len()
        );
        return ExitCode::from(2);
    }

    // Sprint 39a: the user's `define function main` must be present
    // for `nod-llvm::aot::emit_aot_entry_stubs` to find it. Surface a
    // clear error before we kick off codegen if it's missing.
    if !lm.functions.iter().any(|f| f.name == "main") {
        eprintln!(
            "nod build: input file does not define `main` — Sprint 39a EXEs need \
             `define function main () => () ... end` as the entry point."
        );
        return ExitCode::from(1);
    }

    // Step 2 — codegen.
    let ctx = Context::create();
    let module_name = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dylan-module");
    let out = match nod_llvm::codegen_module(&ctx, &lm.functions, module_name) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("nod build: codegen: {e}");
            return ExitCode::from(1);
        }
    };
    let module = out.module;
    let manifest = out.manifest;

    // Step 3 — AOT entry-stub injection + object-file emission.
    // We co-locate the `.obj` next to the output EXE so the file system
    // shows the compile pipeline's intermediate artifact for debugging.
    // (A future sprint can route this through a temp directory if the
    // intermediate becomes noise.)
    let obj_path = {
        let mut p = output.to_path_buf();
        p.set_extension("obj");
        p
    };
    if let Err(e) =
        nod_llvm::aot::emit_aot_object(&module, &manifest, &obj_path, OptimizationLevel::Default)
    {
        eprintln!("nod build: {e}");
        return ExitCode::from(1);
    }

    // Step 4 — locate the staticlib and `link.exe`.
    let runtime_lib = match locate_runtime_staticlib() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("nod build: {e}");
            return ExitCode::from(1);
        }
    };

    let mut link_cmd = match cc::windows_registry::find("x86_64-pc-windows-msvc", "link.exe") {
        Some(c) => c,
        None => {
            eprintln!(
                "nod build: could not locate MSVC link.exe. \
                 Run from a Developer Command Prompt or install VS Build Tools."
            );
            return ExitCode::from(1);
        }
    };

    if verbose {
        eprintln!(
            "nod build: triple    = {}",
            nod_llvm::aot::default_triple_string()
        );
        eprintln!("nod build: input    = {}", input.display());
        eprintln!("nod build: object   = {}", obj_path.display());
        eprintln!("nod build: runtime  = {}", runtime_lib.display());
        eprintln!("nod build: output   = {}", output.display());
    }

    // Step 5 — invoke link.exe.
    //
    // Standard MSVC EXE link line:
    //   - User .obj (defines `nod_user_main` + `main`)
    //   - nod_runtime.lib (defines `nod_aot_main_wrapper` + the
    //     full Dylan runtime; transitively pulls in the user's
    //     `nod_user_main` reference from `aot.obj`)
    //   - CRT + system libs needed by Rust std I/O
    //   - /SUBSYSTEM:CONSOLE so format-out → stdout is visible
    //   - /ENTRY:mainCRTStartup the standard CRT entry; calls the
    //     `main()` stub we emitted in `emit_aot_entry_stubs`.
    //   - /NXCOMPAT /DYNAMICBASE /HIGHENTROPYVA — modern Windows
    //     security defaults; `link.exe` warns without these.
    link_cmd.arg(&obj_path);
    link_cmd.arg(&runtime_lib);
    link_cmd.arg(format!("/OUT:{}", output.display()));
    link_cmd.arg("/SUBSYSTEM:CONSOLE");
    link_cmd.arg("/ENTRY:mainCRTStartup");
    link_cmd.arg("/MACHINE:X64");
    link_cmd.arg("/NXCOMPAT");
    link_cmd.arg("/DYNAMICBASE");
    link_cmd.arg("/HIGHENTROPYVA");
    // The libs Rust's MSVC std + windows-sys need at link time. cc-rs's
    // discovered link.exe Command already has %LIB% set so these
    // resolve from the SDK's lib directory.
    for lib in [
        "kernel32.lib",
        "advapi32.lib",
        "userenv.lib",
        "ws2_32.lib",
        "ntdll.lib",
        "msvcrt.lib",
        "ucrt.lib",
        "vcruntime.lib",
        "legacy_stdio_definitions.lib",
        // Sprint 35 / 36's COM types pull in these even when the user
        // program doesn't touch them — the unused-symbol DCE doesn't
        // strip them because the `windows` crate uses `#[link]` attrs
        // that the staticlib's metadata propagates. Cheap to include
        // unconditionally.
        "ole32.lib",
        "oleaut32.lib",
        "uuid.lib",
        "user32.lib",
        "gdi32.lib",
        "dxgi.lib",
        "d3d11.lib",
        "d2d1.lib",
        "dwrite.lib",
        "bcrypt.lib",
        "synchronization.lib",
        // Sprint 39a — the `windows` crate's PROPVARIANT/VARIANT
        // helpers pulled in via Sprint 35's COM types reference
        // `PropVariantTo*` / `VariantTo*` which live in propsys.lib.
        // Adding here unconditionally because we have no way to know
        // which symbols the staticlib's transitively-included COM
        // types will reference; the linker DCE drops unused entries.
        "propsys.lib",
    ] {
        link_cmd.arg(lib);
    }
    if verbose {
        eprintln!("nod build: link.exe args: {:?}", link_cmd.get_args().collect::<Vec<_>>());
    }

    match link_cmd.output() {
        Ok(o) if o.status.success() => {
            println!("compiled: {}", output.display());
            ExitCode::SUCCESS
        }
        Ok(o) => {
            eprintln!("nod build: link.exe failed with status {}", o.status);
            if !o.stdout.is_empty() {
                eprintln!("link.exe stdout:");
                std::io::Write::write_all(&mut std::io::stderr(), &o.stdout).ok();
            }
            if !o.stderr.is_empty() {
                eprintln!("link.exe stderr:");
                std::io::Write::write_all(&mut std::io::stderr(), &o.stderr).ok();
            }
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("nod build: failed to invoke link.exe: {e}");
            ExitCode::from(1)
        }
    }
}

fn run_dump_dfm(input: &std::path::Path) -> ExitCode {
    match nod_sema::dump_dfm_for_file(input) {
        Ok(dump) => {
            print!("{dump}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nod-driver dump-dfm: {e}");
            ExitCode::from(2)
        }
    }
}

fn run_dump_llvm(input: &std::path::Path) -> ExitCode {
    match nod_sema::dump_llvm_for_file(input) {
        Ok(ir) => {
            print!("{ir}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nod-driver dump-llvm: {e}");
            ExitCode::from(2)
        }
    }
}

fn run_eval(expr: &str) -> ExitCode {
    match nod_sema::eval_expr_to_string(expr) {
        Ok(s) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nod-driver eval: {e}");
            ExitCode::from(1)
        }
    }
}

fn run_dump_tokens(input: &std::path::Path) -> ExitCode {
    use nod_reader::{SourceMap, format_tokens, lex};
    let src = match std::fs::read_to_string(input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "nod-driver dump-tokens: failed to read {}: {e}",
                input.display()
            );
            return ExitCode::from(2);
        }
    };
    let mut sm = SourceMap::new();
    let id = match sm.add(input.to_path_buf(), src.clone()) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("nod-driver dump-tokens: {e}");
            return ExitCode::from(2);
        }
    };
    let tokens = lex(&src, id);
    let dump = format_tokens(&tokens, id, &sm);
    print!("{dump}");
    ExitCode::SUCCESS
}

fn run_dump_ast(input: &std::path::Path) -> ExitCode {
    use nod_reader::{SourceMap, format_ast_module, lex, parse_module, scan_preamble};
    let src = match std::fs::read_to_string(input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nod-driver dump-ast: failed to read {}: {e}", input.display());
            return ExitCode::from(2);
        }
    };
    let mut sm = SourceMap::new();
    let id = match sm.add(input.to_path_buf(), src.clone()) {
        Ok(id) => id,
        Err(e) => {
            eprintln!("nod-driver dump-ast: {e}");
            return ExitCode::from(2);
        }
    };
    let tokens = lex(&src, id);
    let pre = scan_preamble(&src);
    match parse_module(&src, &tokens, pre.as_ref()) {
        Ok(m) => {
            print!("{}", format_ast_module(&m));
            ExitCode::SUCCESS
        }
        Err(diags) => {
            for d in &diags {
                eprintln!("error: {}", d.message);
            }
            ExitCode::from(1)
        }
    }
}

fn run_dump_graph(input: &std::path::Path) -> ExitCode {
    use nod_namespace::{Graph, dump_graph, load_lid_chain};
    let lid = match load_lid_chain(input) {
        Ok(lid) => lid,
        Err(e) => {
            eprintln!("nod-driver dump-graph: failed to load {}: {e}", input.display());
            return ExitCode::from(2);
        }
    };
    let mut g = Graph::new();
    g.add_library_from_lid(&lid);
    print!("{}", dump_graph(&g));
    ExitCode::SUCCESS
}
