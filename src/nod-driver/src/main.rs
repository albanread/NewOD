//! NewOpenDylan compiler driver.
//!
//! Sprint 01: stub CLI. `--version` and `--help` work; `compile` and
//! `repl` are recognised but do nothing yet. Real functionality lands
//! in subsequent sprints (lexer 02 → parser 03/04 → namespace 05 → DFM
//! IR 06 → LLVM codegen 07 → REPL loop 08 → …).

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
        input: Option<String>,
    },
    /// Start an interactive REPL. Not yet implemented.
    Repl,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        None => {
            // No subcommand: print a one-line banner that proves the
            // build is reachable. Matches the Sprint 01 acceptance
            // criterion for plain `nod-driver` (and is roughly what a
            // future bare `nod-driver` invocation will print before
            // dropping into the REPL).
            println!(
                "nod-driver {} (LLVM {LLVM_VERSION})",
                env!("CARGO_PKG_VERSION")
            );
        }
        Some(Command::Compile { input }) => {
            let target = input.as_deref().unwrap_or("<no input>");
            eprintln!("nod-driver compile: not yet implemented (input: {target})");
            std::process::exit(2);
        }
        Some(Command::Repl) => {
            eprintln!("nod-driver repl: not yet implemented (see Sprint 08).");
            std::process::exit(2);
        }
    }
}
