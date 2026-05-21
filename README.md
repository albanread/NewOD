# NewOpenDylan

> ⚠️ **Work in progress — not a usable Dylan implementation yet.**
> Roughly Sprint 20 of ~32 planned sprints have landed (see
> [docs/SPRINTS.md](docs/SPRINTS.md)). Each sprint is nominally two
> weeks, but real-world ratios on the sister projects say plan for
> **several more years** before NewOpenDylan reaches a state a Dylan
> programmer can actually rely on. Treat this repo as a design diary
> with running code, not a release.
>
> ⚠️ **The GC is not yet correct.** The collector runs Sprint 16's
> Richards bench end-to-end, but it's still the Sprint 11 "option (b)"
> design: synchronous, only triggered at Rust-side allocation sites,
> with **no JIT-side safepoint polls and no precise stack roots via
> `gc.statepoint`**. That work — Sprint 11d — is queued but not in
> tree. The implication: any JIT'd Dylan code that holds a tagged
> reference in a register across a triggering allocation can lose it,
> and any program large enough to fragment the young space will
> eventually surface this. Don't put real data through it.

A from-scratch Rust + LLVM JIT for the [Dylan programming language](https://opendylan.org), with a graphical IDE, live inspection, and live incremental compilation. Windows-first; macOS second. 64-bit only.

This is a **true revival** — not a port, not a fork, not a preservation effort. We keep the language as the Dylan Reference Manual defines it; we replace the implementation, the IDE, the GC, the runtime, and the build chain. See [docs/MANIFESTO.md](docs/MANIFESTO.md) for the design commitments we won't move.

## Where to start

- **[docs/MANIFESTO.md](docs/MANIFESTO.md)** — design constraints. Read this first.
- **[docs/PLAN.md](docs/PLAN.md)** — language survey + 12-phase implementation plan.
- **[docs/SPRINTS.md](docs/SPRINTS.md)** — two-week sprint breakdown with deliverables, acceptance criteria, and demos.
- **[docs/DEFERRED.md](docs/DEFERRED.md)** — features explicitly out of scope (with rationale).
- **[docs/NCL_GC_FEEDBACK.md](docs/NCL_GC_FEEDBACK.md)** — cross-project notes on GC design feeding back from NewCormanLisp.
- **[docs/specs/](docs/specs/)** — per-sprint design specs drafted ahead of implementation (lexer, library/module graph, REPL, sealing/dispatch, …).

## Workspace layout

```
NewOpenDylan/
├── Cargo.toml              # workspace root, `unsafe_op_in_unsafe_fn = "deny"`
├── .cargo/config.toml      # LLVM 22.1 env (activated in Sprint 06)
├── .github/workflows/ci.yml
├── docs/
│   ├── MANIFESTO.md        # design constraints (read first)
│   ├── PLAN.md             # 12-phase implementation plan
│   ├── SPRINTS.md          # 2-week sprint breakdown
│   ├── DEFERRED.md         # explicitly out-of-scope features
│   ├── NCL_GC_FEEDBACK.md  # cross-project GC notes from NewCormanLisp
│   ├── GC.md               # GC design notes (full doc by Sprint 11)
│   ├── DFM.md              # IR design notes (full doc by Sprint 06)
│   ├── SEALING.md          # sealing notes (full doc by Sprint 14)
│   ├── MACROS.md           # macro expander notes (full doc by Sprint 17)
│   └── specs/              # per-sprint design specs
│       ├── 01-lexer.md
│       ├── 05-library-module-graph.md
│       ├── 08-repl-and-live-bindings.md
│       └── 15-sealing-and-dispatch-resolution.md
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

## Current status — WIP checkpoint through Sprint 20b

The workspace is real code, not placeholders. Roughly 30 kLOC of Rust
across the crates. What's in tree today, by area:

**Front end** (`nod-reader`, `nod-namespace`, `nod-macro`).
Lexer + AST (Sprint 02), fragment-based infix parser (Sprint 03),
definition forms + body parser (Sprint 04), LID files + library /
module dependency graph (Sprint 05), pattern-rule macro expander +
the twelve most-common macro shapes (Sprint 17-18). Approximately
6.2 kLOC reader, 0.7 kLOC namespace, 1.8 kLOC macro.

**Semantic analysis** (`nod-sema`). Classes + slots, single dispatch
placeholder (Sprint 12), multiple-inheritance slot layout (Sprint 14),
sealing analysis + compile-time dispatch resolution (Sprint 15),
forward-iteration protocol scaffolding (Sprint 20). ~5.9 kLOC.

**IR + codegen** (`nod-dfm`, `nod-llvm`). Dylan Flow Machine typed
SSA IR (Sprint 06) with dispatch nodes (Sprint 13); thin-slice LLVM
codegen (Sprint 07). ~1.6 kLOC IR, ~3.1 kLOC codegen.

**Runtime** (`nod-runtime`). Boxed `<integer>`, strings, symbols,
vectors (Sprints 09-10); generational copying collector (Sprint 11 —
see warning above); method-lookup runtime (Sprint 13); make
dispatch (Sprint 12); conditions + non-local exit skeleton
(Sprint 19); core collection types (Sprint 20). ~10.5 kLOC.

**Driver / REPL** (`nod-driver`). REPL loop, `dump-tokens`,
`dump-ast`, `dump-graph`, `dump-dfm`, `dump-llvm`, `eval`.

**FFI trajectory (Sprint 27 → Sprint 37).** Sprint 27 lands the FFI
*data* layer: the vendored Windows API metadata database lives at
`data/windows_api.db`, the `nod-winapi` crate embeds a 205 KB
zstd-compressed postcard projection (13,080 primitive-typed Win32
functions across 336 DLLs), and the reader / sema layer accepts
`define c-function NAME (PARAMS) => (RET); library: "kernel32.dll";
end;` declarations and records DLL provenance in
`nod-namespace::Binding`. **No API call executes yet — Sprint 28 lands
the per-module API stub table + the first `Beep(440, 1000)`
end-to-end.** Sprints 28-37 then build the `c-ffi` library port
(structs, callbacks, COM), the `io` / kernel library ports, the
Dylan-side IDE shell on top of the Sprint-27 FFI stack, and finally
the Cocoa-FFI variant for the macOS port.

**What's not yet in tree:** `gc.statepoint` precise-roots emission
(Sprint 11d), end-to-end FFI calls (Sprint 28), `format` / `print` /
`streams` (Sprint 28b), kernel library port (Sprint 28c), the
Dylan-side IDE (Sprints 29+), `common-dylan` library (Sprint 30),
multi-threaded mutator (Sprint 31), AOT mode (Sprint 33), Mac port
(Sprint 35).

Day-to-day: `cargo build --workspace` is green, `cargo run -p
nod-driver -- dump-tokens hello.dylan` works, and Sprint 16's
`simple-richards` benchmark subset runs end-to-end. See
[docs/SPRINTS.md](docs/SPRINTS.md) for the full per-sprint plan
and [bench/richards.md](bench/richards.md) for the Richards
sealing-vs-open performance trajectory.

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
