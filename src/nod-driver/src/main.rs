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
