# NewOpenDylan

> ⚠️ **Work in progress — not a usable Dylan implementation yet.**
> Sprint 43c has landed (see [docs/SPRINTS.md](docs/SPRINTS.md) for
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
> linker-resolved IAT. The `nod-ide.exe` Windows IDE binary is written
> entirely in Dylan and builds this way. **Every layer of that —
> AOT, user-defined classes in AOT, FFI, COM, callbacks, the IDE
> shell, the byte-string stdlib methods — landed in the last
> twenty-odd sprints and has only been exercised by its own tests.**
> Expect bugs. Release-mode AOT currently hits an LNK2005
> `nod_user_main` collision; debug-mode AOT works. The whole AOT
> trajectory is documented per-sprint in
> [docs/SPRINTS.md](docs/SPRINTS.md) (Sprints 39–41).

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

## Current status — WIP checkpoint through Sprint 43c

The workspace is real code, not placeholders — and a lot more of it
than the original sprint plan anticipated. The compiler now JITs and
AOT-compiles non-trivial Dylan programs. Headlines:

**`nod-ide.exe`** — a small Windows IDE (read-only source viewer +
File menu with Open / Save / Save As / Recent / Exit, Help → About,
horizontal + vertical scrolling, DirectWrite text rendering, real
Win32 message pump) is written **entirely in Dylan** in
`tests/nod-tests/fixtures/nod-ide.dylan` (~700 lines) and AOT-builds
to a ~1 MB EXE via `cargo run --bin nod-driver -- build … -o ….exe`.
Sprints 41a–g delivered this end-to-end.

**`<rope>` data structure** — a classical Boehm-style rope buffer
lives in `tests/nod-tests/fixtures/rope.dylan` (~650 lines of
Dylan): abstract `<rope>` parent, `<rope-leaf>` and `<rope-node>`
subclasses with cached size + newline counts, full O(log n) read +
edit surface (`rope-size`, `rope-element`, `rope-substring`,
`rope-concatenate`, `for-each-leaf`, `rope-split-at`, `rope-insert`,
`rope-delete`, `rope-line-count`, `rope-line-to-offset`,
`rope-offset-to-line`). 24 self-tests pass under AOT including a
200-op random-edit GC-stress walk — Sprints 43a–c.

Roughly 60+ kLOC of Rust across the crates, plus the two non-trivial
Dylan fixtures above. What's in tree today, by area:

**Front end** (`nod-reader`, `nod-namespace`, `nod-macro`). Lexer
+ AST (Sprint 02), fragment-based infix parser (Sprint 03),
definition forms + body parser (Sprint 04), LID files + library /
module dependency graph (Sprint 05), pattern-rule macro expander +
common macro shapes including `unless` / `for-each` migrated to
stdlib macros (Sprints 17, 18, 25).

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
module sealed-direct call fallback (Sprint 42a Phase B).

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

**AOT mode** (Sprints 39 + 40 + 41). `cargo run --bin nod-driver
-- build foo.dylan -o foo.exe` produces a standalone Win64 EXE.
Sprint 39 wired the dual-output `nod-runtime` (rlib + staticlib),
the codegen `main`-stub injection, object-file emission via
`inkwell::TargetMachine`, and the `nod-driver build` subcommand.
Sprint 40a brought user-defined classes into AOT (registered at
EXE start-up via `nod_aot_register_user_class`); 40b–d brought
Win32 callbacks, the COM device chain, and bare-name Win32 calls
into AOT. Sprints 41a–g built the IDE shell entirely from Dylan
on top of all of the above — message pump, scrollbars, source
viewer, file menu, recent-files persistence (initially in Rust
shims; Sprint 42a Phase E retired those shims to pure Dylan once
byte-string methods were available).

**Driver / REPL** (`nod-driver`). REPL loop, `dump-tokens`,
`dump-ast`, `dump-graph`, `dump-dfm`, `dump-llvm`, `eval`, and
`build` (the AOT subcommand).

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

**Test counts:** `cargo test --workspace --tests` runs ~600+ tests;
all green modulo one pre-existing parallel-flake on
`api_stub_table_deduplicates_call_sites` that passes in isolation.
The interactive AOT IDE tests are `#[ignore]`'d; the rope tests
take ~10s under `--ignored` because of the cargo-build overhead.

Day-to-day: `cargo build --workspace` is green, `cargo run -p
nod-driver -- eval '1 + 1'` works, `cargo run --bin nod-driver --
build tests/nod-tests/fixtures/nod-ide.dylan -o F:/scratch/nod-ide.exe`
produces the IDE binary. See [docs/SPRINTS.md](docs/SPRINTS.md) for
the full per-sprint history and [bench/richards.md](bench/richards.md)
for the Richards sealing-vs-open performance trajectory.

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
