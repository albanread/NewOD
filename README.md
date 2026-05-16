# NewOpenDylan

A from-scratch Rust + LLVM JIT for the [Dylan programming language](https://opendylan.org), with a graphical IDE, live inspection, and live incremental compilation. Windows-first; macOS second. 64-bit only.

This is a **true revival** — not a port, not a fork, not a preservation effort. We keep the language as the Dylan Reference Manual defines it; we replace the implementation, the IDE, the GC, the runtime, and the build chain. See [MANIFESTO.md](../MANIFESTO.md) for the design commitments we won't move.

## Where to start

- **[MANIFESTO.md](../MANIFESTO.md)** — design constraints. Read this first.
- **[PLAN.md](../PLAN.md)** — language survey + 12-phase implementation plan.
- **[SPRINTS.md](../SPRINTS.md)** — two-week sprint breakdown with deliverables, acceptance criteria, and demos.
- **[../opendylan-tests/INVENTORY.md](../opendylan-tests/INVENTORY.md)** — copied upstream test corpus, with bootstrap-validation candidates flagged.
- **[../specs/](../specs/)** — per-sprint design specs as they get drafted ahead of implementation.

## Workspace layout

```
NewOpenDylan/
├── Cargo.toml              # workspace root, `unsafe_op_in_unsafe_fn = "deny"`
├── .cargo/config.toml      # LLVM 22.1 env (activated in Sprint 06)
├── .github/workflows/ci.yml
├── docs/
│   ├── GC.md               # GC design (full doc by Sprint 11)
│   ├── DFM.md              # IR design (full doc by Sprint 06)
│   ├── SEALING.md          # sealing analysis (full doc by Sprint 14)
│   └── MACROS.md           # macro expander (full doc by Sprint 17)
├── src/
│   ├── nod-driver/         # CLI + REPL entrypoint
│   ├── nod-reader/         # lexer + AST
│   ├── nod-macro/          # pattern-rule macro expander
│   ├── nod-namespace/      # library/module graph
│   ├── nod-sema/           # type checking + sealing analysis
│   ├── nod-dfm/            # Dylan Flow Machine IR (typed SSA)
│   ├── nod-opt/            # IR-level optimisation passes
│   ├── nod-llvm/           # LLVM codegen
│   ├── nod-loader/         # incremental loader + hot reload
│   ├── nod-runtime/        # GC + runtime + IDE shell
│   └── nod-dylan/          # ported Dylan kernel library (sources only)
└── tests/
    ├── nod-tests/          # end-to-end JIT regression tests
    └── nod-od-suite/       # OpenDylan-compatibility test runner
```

## Current status

**Sprint 01 — Workspace Skeleton.** `cargo build --workspace` is green; `cargo run -p nod-driver -- --version` prints the banner. No real functionality yet — every crate is a placeholder. See [SPRINTS.md](../SPRINTS.md) for what each subsequent sprint adds.

## Sibling-compiler portfolio

NewOpenDylan is the sixth in a family of from-scratch Rust + LLVM compilers we maintain together. We share runtime, GC, JIT-MM, and conventions; we do not share AST/IR/sema across languages.

| Project | Language | Workspace |
|---|---|---|
| NewM2 | Modula-2 (PIM 4 + ISO 10514-1) | `E:\NewM2` |
| NewCP | Component Pascal | `E:\NewCP\NewCP` |
| NewCormanLisp | Common Lisp | `E:\CL\NewCormanLisp` |
| NewBCPL | BCPL | `E:\NewBCPL` |
| NewFB | FreeBASIC | `E:\NewFB` |
| **NewOpenDylan** | **Dylan** | `E:\NewOpenDylan\NewOpenDylan` |

## Licence

Dual-licensed under MIT and Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
