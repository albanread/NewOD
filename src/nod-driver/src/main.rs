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
