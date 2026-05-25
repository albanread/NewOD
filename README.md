# NewOpenDylan

> ⚠️ **Work in progress — not a usable Dylan implementation yet.**
> Sprint 45a has landed (see [docs/SPRINTS.md](docs/SPRINTS.md) for
> the long form). Each sprint is nominally two weeks of effort, but
> real-world ratios on the sister projects say plan for **several more
> years** before NewOpenDylan reaches a state a Dylan programmer can
> actually rely on. Treat this repo as a design diary with running
> code, not a release.
>
> ⚠️ **The GC is not yet correct.** The collector now runs the Sprint
> 23 page-heap NewGC backend and survives a 200-op random-edit
> stress walk against a Dylan-side `<rope>` data structure
> (~thousands of small allocations) — but it's still the Sprint 11
> "option (b)" design at heart: synchronous, only triggered at
> Rust-side allocation sites, with **no JIT-side safepoint polls
> and no precise stack roots via `gc.statepoint`**. That work —
> Sprint 11d — is queued but not in tree. The implication: any JIT'd
> Dylan code that holds a tagged reference in a register across a
> triggering allocation can lose it, and any program large enough to
> fragment the working set will eventually surface this. Don't put
> real data through it.
>
> ⚠️ **AOT mode compiles real Win64 EXEs, but the whole stack is new
> and lightly tested.** `nod-driver build foo.dylan -o foo.exe`
> produces a standalone EXE that calls Win32 APIs (CreateWindowExW,
> the COM Direct2D / DirectWrite chain, GetOpenFileNameW, …) via
> linker-resolved IAT. Sprint 44 extended this to multi-file builds:
> `nod-driver build a.dylan b.dylan … -o foo.exe` merges ASTs before
> lowering so all definitions are visible across files. The
> `nod-ide.exe` Windows IDE is written entirely in Dylan across five
> source files (~2000 lines total) and AOT-builds this way. **Every
> layer — AOT, user-defined classes, FFI, COM, callbacks, IDE shell,
> byte-string stdlib — landed in the last thirty-odd sprints and has
> only been exercised by its own tests.** Expect bugs. Release-mode
> AOT currently hits an LNK2005 `nod_user_main` collision;
> debug-mode AOT works. The whole AOT trajectory is documented
> per-sprint in [docs/SPRINTS.md](docs/SPRINTS.md) (Sprints 39–44).

A from-scratch Rust + LLVM JIT for the [Dylan programming language](https://opendylan.org), with a graphical IDE, live inspection, and live incremental compilation. Windows-first; macOS second. 64-bit only.

This is a **true revival** — not a port, not a fork, not a preservation effort. We keep the language as the Dylan Reference Manual defines it; we replace the implementation, the IDE, the GC, the runtime, and the build chain. See [docs/MANIFESTO.md](docs/MANIFESTO.md) for the design commitments we won't move.

---

## About Dylan

### Vision

Dylan was designed to bridge the gap between dynamic and compiled languages: the flexibility and interactivity of Lisp, with performance suitable for shipping commercial software. The aims were to combine rapid prototyping, a rich object-oriented type system, and the ability to scale to large, high-performance applications — without forcing the programmer to choose one or the other up front.

The key mechanism was *controlled dynamism*: you can write fully generic, open-ended code first, then gradually add type declarations and sealing constraints to let the compiler specialise it — prototype to production in one language, on one codebase. And unlike traditional Lisps, Dylan adopted a conventional ALGOL-style infix syntax, so the code looks familiar to anyone who learned programming after 1985.

The short version: a Lisp-inspired language that reads like ordinary code and compiles to fast native binaries.

### History

Dylan was created in the early 1990s at Apple, working with Carnegie Mellon University and Harlequin. The original target was the Apple Newton PDA — Dylan was meant to be its primary development language. It wasn't ready in time; Newton shipped with other tools.

Through the mid-1990s the project pivoted toward general-purpose programming, and the syntax was redesigned from Lisp-style s-expressions to the infix form the language has today, explicitly to broaden its appeal. Apple ended its internal investment around 1995, releasing only a limited technology preview.

Development continued through Harlequin's commercial toolchain and CMU's Gwydion Dylan compiler, then transitioned into the community-driven Open Dylan project, where the language has persisted as a technically serious but niche system programming language ever since.

One-line takeaway: Dylan was an ambitious attempt to make Lisp-style dynamism practical for mainstream, high-performance software — it missed its commercial window, and it survived anyway.

### Why we're here

NewOpenDylan is our own take on that original ambition — brand new, experimental, and operational:

- **The compiler works.** It JITs and AOT-compiles non-trivial Dylan programs, backed by our own page-heap GC, and produces standalone Win64 EXEs via `nod-driver build`.
- **The IDE is written in Dylan.** `nod-ide.exe` — a graphical source editor with a real Win32 message pump, DirectWrite text rendering, and a full file menu — is itself a Dylan AOT-compiled binary. The editor and the compiler it will eventually host are built from the same language.
- **Everything is new.** This is not a resurrection of the old Harlequin or Open Dylan toolchain. It's a clean-room implementation in Rust + LLVM, with a fresh GC, a fresh IR, and a fresh FFI layer.

What draws us to Dylan specifically:

- **Interactive by design.** The REPL and hot-reload pipeline give you a live Lisp-style development loop — evaluate a form, inspect the result, redefine a method without restarting the process.
- **Lisp dynamism in conventional clothing.** Multiple dispatch, a real condition system, macros, first-class functions — but written in syntax that doesn't require unlearning everything you already know.
- **Full modern Windows API.** Our FFI layer covers ~15 000 Win32 functions across ~350 DLLs, COM via the `windows` crate, D2D/DirectWrite/DXGI, and callback trampolines. Writing a native Windows GUI in Dylan is not a research project here; it's what we do.
- **Not simple, but interesting.** Dylan rewards the effort. The object system has sealing and compile-time dispatch. The GC is precise (on the roadmap) and incremental. The macro system is hygienic. It's the kind of language where the more you put in, the more you get back.

---

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

## Current status — WIP checkpoint through Sprint 45a

The workspace is real code, not placeholders — and a lot more of it
than the original sprint plan anticipated. The compiler JITs and
AOT-compiles non-trivial Dylan programs, the IDE runs as a native
Win64 EXE written in Dylan, and multi-file AOT builds work. Headlines:

**`nod-ide.exe`** — a Windows source editor written **entirely in
Dylan**, AOT-built to a ~1 MB EXE via `nod-driver build`. The IDE
source is now split across five files in
`tests/nod-tests/fixtures/`:

| File | Lines | Content |
|---|---|---|
| `nod-ide.dylan` | ~917 | entry point, window setup, message pump |
| `ide_syntax.dylan` | ~507 | syntax-colouring engine |
| `ide_helpers.dylan` | ~251 | text utilities, layout helpers |
| `ide_rope.dylan` | ~282 | rope ↔ IDE bridge |
| `ide_win_calls.dylan` | ~91 | Win32 call wrappers |

The editor provides: File menu (Open / Save / Save As / Recent /
Exit), Help → About, DirectWrite text rendering, horizontal +
vertical scrolling, a `<rope>`-backed live editing buffer, a
blinking text cursor driven by `WM_TIMER`, full keyboard navigation
(arrow keys, Home / End, PgUp / PgDn, Ctrl+Home / Ctrl+End),
mouse-click cursor placement via `HitTestPoint`, syntax colouring
(Dylan keywords, comments, strings, numbers, and class names each
in a distinct colour), and a left gutter showing line numbers with
reserved columns for future fold/error markers. Sprints 41–43g
delivered this end-to-end.

**`<rope>` data structure** — a classical rope buffer lives in
`tests/nod-tests/fixtures/rope.dylan` (~717 lines of Dylan): abstract
`<rope>` parent, `<rope-leaf>` and `<rope-node>` subclasses with
cached size + newline counts, full O(log n) read + edit surface
(`rope-size`, `rope-element`, `rope-substring`, `rope-concatenate`,
`for-each-leaf`, `rope-split-at`, `rope-insert`, `rope-delete`,
`rope-line-count`, `rope-line-to-offset`, `rope-offset-to-line`). 24
self-tests pass under AOT including a 200-op random-edit GC-stress
walk — Sprints 43a–c. Sprint 43d wired the rope as the live editor
buffer in `nod-ide.exe`.

**Dylan lexer in Dylan** (Sprint 45a) — `tests/nod-tests/fixtures/
dylan-lexer.dylan` (~471 lines) defines the full `<token>` class
hierarchy (16 concrete subclasses including `<integer-token>`,
`<string-token>`, `<symbol-token>`, `<keyword-token>`,
`<operator-token>`, `<eof-token>`, …), `<span>`, and per-class
generics (`colour-of`, `token-kind-name`, `print-token`). A stub
`lex` returns `[<eof-token>]` only; real tokenisation lands in
45b. The new `nod-driver dump-dylan-tokens <path>` subcommand
AOT-compiles the embedded lexer source and prints a text-diff oracle
dump.

Roughly 60+ kLOC of Rust across the crates, plus the Dylan fixtures
above. What's in tree today, by area:

**Front end** (`nod-reader`, `nod-namespace`, `nod-macro`). Lexer
+ AST (Sprint 02), fragment-based infix parser (Sprint 03),
definition forms + body parser (Sprint 04), LID files + library /
module dependency graph (Sprint 05), pattern-rule macro expander +
common macro shapes including `unless` / `for-each` migrated to
stdlib macros (Sprints 17, 18, 25). Multi-file sema entry point —
`compile_files_for_aot` parses each file then merges ASTs before
lowering (Sprint 44A–B; AST-level merge hotfix post-44E).

**Semantic analysis** (`nod-sema`). Classes + slots, single dispatch
(Sprint 12), multiple-inheritance slot layout (Sprint 14), sealing
analysis + compile-time dispatch resolution (Sprint 15), forward-
iteration protocol (Sprint 20), `<table>` + content-based hashing
(Sprint 22), closures with cell promotion (Sprint 24), `<c-struct>`
field accessors (Sprint 34), universal `=` dispatch for non-numeric
operands (Sprint 42a Phase B), env-merge correctness in `lower_if`
(Sprint 42-pre — fixed a latent SSA dominance bug surfaced by the
rope work).

**IR + codegen** (`nod-dfm`, `nod-llvm`). Dylan Flow Machine typed
SSA IR (Sprint 06) with dispatch nodes (Sprint 13); LLVM codegen
(Sprint 07); LLVM `ObjectCache` per-module + sidecar manifest
(Sprint 37); cross-process bitcode replay with named symbol
resolution (Sprint 38 family — immediates, static-area pointers,
stub-entry pointers, cache slots, generic-function pointers all
converted from baked addresses to externally-resolved globals);
on-disk replay wired into the eval pipeline (Sprint 38f); cross-
module sealed-direct call fallback (Sprint 42a Phase B). Short-
circuit `|` and `&` via 3-block CFG lowering (Task #251 — no more
eager evaluation of the right-hand operand).

**Runtime** (`nod-runtime`). Tagged Words, boxed `<integer>`,
strings, symbols, vectors, lists, ranges, stretchy vectors
(Sprints 09–10, 16, 20); NewGC page-heap backend (Sprint 23 — see
warning above); method-lookup runtime + dispatch (Sprints 12–15);
conditions + non-local exit (Sprint 19); `<table>` open-addressing
+ FNV-1a content hash for byte-strings (Sprint 22); cells +
environments (Sprint 24); literal pool + static-area allocator;
Sprint 32 callback trampoline pool (closure → C fn-ptr for WNDPROC,
WNDENUMPROC); Sprint 35 COM handle registry via the `windows` crate
(DXGI, D3D11, D2D, DirectWrite); Sprint 42a byte-string primitives
(`%byte-string-allocate` / `-size` / `-element` / `-element-setter`
/ `-copy!`).

**Stdlib** (`nod-dylan/dylan-sources/stdlib.dylan`). Pre-compiled
into a long-lived JIT engine on first eval and merged into AOT
modules at build time (Sprint 39c). Provides `size`, `concatenate`,
`reduce`, `map`, `do` over the FIP; `<table>` generics; `<c-struct>`
field accessors; `as-wndproc-callback` / `as-wndenumproc-callback`;
**12 `<byte-string>` methods built on the Sprint 42a primitives**
(`size`, `element`, `concatenate`, `copy-sequence`, `subsequence`,
`starts-with?`, `ends-with?`, `find-substring`, `as-uppercase`,
`as-lowercase`); universal `=` for byte-strings via Sprint 22's
content-equal path; `for-each`, `unless` macros.

**FFI** (`nod-winapi`, FFI machinery in `nod-runtime` /
`nod-sema` / `nod-llvm`). Vendored Windows API metadata at
`data/windows_api.db`; `nod-winapi` embeds a zstd-compressed
postcard projection of ~15K primitive-typed Win32 functions
across ~350 DLLs (Sprint 27). Per-module API stub table + Win64
trampolines arity 0..=8 (Sprint 28). Win32 integer constants
auto-generated into stdlib (Sprint 29). `<c-string>` / `<c-wide-
string>` marshaling + `$NULL` (Sprint 30). JIT-time bare-name
materialization — call `Beep(440, 1000)` without an explicit
`define c-function` (Sprint 31). Callback trampolines (Sprint 32).
`<c-struct>` family for `POINT`, `RECT`, `MSG`, `WNDCLASSEXW`,
`PAINTSTRUCT` (Sprint 34). COM via `windows` crate (Sprint 35).

**AOT mode** (Sprints 39–44). `cargo run --bin nod-driver -- build
foo.dylan -o foo.exe` produces a standalone Win64 EXE. Sprint 39
wired the dual-output `nod-runtime` (rlib + staticlib), codegen
`main`-stub injection, object-file emission via
`inkwell::TargetMachine`, and the `nod-driver build` subcommand.
Sprint 40a brought user-defined classes into AOT; 40b–d brought
Win32 callbacks, the COM device chain, and bare-name Win32 calls.
Sprints 41a–g built the IDE shell in Dylan. Sprint 44 extended to
multi-file: `nod-driver build a.dylan b.dylan … -o foo.exe` parses
all files, merges their ASTs into a single module, then lowers once
— definitions are visible across files without any stub or import
ceremony. `nod-ide.dylan` was split into its five-file form using
this path in Sprint 44E.

**Driver / REPL** (`nod-driver`). REPL loop, `dump-tokens`,
`dump-ast`, `dump-graph`, `dump-dfm`, `dump-llvm`, `eval`, `build`
(AOT single-file), and `dump-dylan-tokens` (runs the Dylan-side
lexer fixture and prints the token dump — Sprint 45a).

**What's still warning-flagged:**

- `gc.statepoint` precise-roots emission (Sprint 11d). The GC has
  survived real workloads (Richards bench, table churn, the rope
  stress walk) but the safety story still depends on the mutator
  not stashing tagged references in registers across allocations.
- Release-mode AOT (LNK2005 `nod_user_main` collision in the staticlib
  build). Debug-mode AOT works.
- `MessageBoxW` from a WNDPROC inside a DirectX-rendered window
  (Sprint 41f investigation). Workaround: `SetWindowTextW` for
  in-app notifications.
- `empty?` can't be specialised on `<byte-string>` yet — Sprint 16's
  lower-time list-builtin shortcut intercepts the call before
  generic dispatch (use `size(s) = 0` for now).

**Test counts:** `cargo test --workspace --tests` runs ~130+ tests
across the workspace (the bulk are in `nod-tests`). The interactive
AOT IDE tests are `#[ignore]`'d and must be run with `--ignored`;
the rope and multi-file AOT tests carry cargo-build overhead (~10s
each). One pre-existing parallel-flake on
`api_stub_table_deduplicates_call_sites` passes in isolation.

Day-to-day: `cargo build --workspace` is green, `cargo run -p
nod-driver -- eval '1 + 1'` works, and a multi-file IDE build is:

```
cargo run --bin nod-driver -- build \
  tests/nod-tests/fixtures/nod-ide.dylan \
  tests/nod-tests/fixtures/ide_syntax.dylan \
  tests/nod-tests/fixtures/ide_helpers.dylan \
  tests/nod-tests/fixtures/ide_rope.dylan \
  tests/nod-tests/fixtures/ide_win_calls.dylan \
  -o F:/scratch/nod-ide.exe
```

See [docs/SPRINTS.md](docs/SPRINTS.md) for the full per-sprint
history and [bench/richards.md](bench/richards.md) for the Richards
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
