# NewOpenDylan

> ⚠️ **Work in progress — not a usable Dylan implementation yet.**
> Sprint 49 has landed (see [docs/SPRINTS.md](docs/SPRINTS.md) for
> the long form). Each sprint is nominally two weeks of effort, but
> real-world ratios on the sister projects say plan for **several more
> years** before NewOpenDylan reaches a state a Dylan programmer can
> actually rely on. Treat this repo as a design diary with running
> code, not a release.
>
> ⚠️ **The GC is precise but not yet verified at compile time.**
> The collector runs the Sprint 23 page-heap NewGC backend and uses
> a precise-roots client: the AOT/JIT codegen spills live GC roots
> to per-call-site stack-slot slabs, brackets allocating calls with
> `nod_aot_begin_safepoint` / `nod_aot_end_safepoint`, and reloads
> the slabs after the call so GC-driven object relocation is
> observed. Sprint 48b (the GAP-011 marathon) closed a several-week
> precise-root staleness bug — function parameters now live in
> stable home allocas across block boundaries, so safepoint
> reloads of params survive into sibling blocks. The whole parser
> corpus now parses without GC crashes; the IDE's rope buffer
> survives heavy churn.
>
> *What's still on the door:* the post-codegen "alloca tracker"
> verifier that would catch this class of bug at compile time
> forever after is queued, not built. Today the invariant ("every
> load from a Word-typed alloca is dominated by either a fresh
> store or a post-safepoint reload+writeback") is true by
> construction; a future codegen change could regress it and the
> first sign would be a stale-pointer panic in a heavy-alloc
> workload. Don't ship anything you wouldn't be willing to debug.
>
> ⚠️ **AOT mode compiles real Win64 EXEs, but the whole stack is new
> and lightly tested.** `nod-driver build foo.dylan -o foo.exe`
> produces a standalone EXE that calls Win32 APIs (CreateWindowExW,
> the COM Direct2D / DirectWrite chain, GetOpenFileNameW, …) via
> linker-resolved IAT. Sprint 44 extended this to multi-file builds:
> `nod-driver build a.dylan b.dylan … -o foo.exe` merges ASTs before
> lowering so all definitions are visible across files. Sprint 49
> added `.prj` project files (TOML, three fields: `name` / `sources`
> / `output`) so the file list lives next to the sources instead of
> getting retyped per invocation — `nod-driver build --project
> foo.prj`. The `nod-ide.exe` Windows IDE is written entirely in
> Dylan across five source files (~2000 lines total) and AOT-builds
> this way. **Every layer — AOT, user-defined classes, FFI, COM,
> callbacks, IDE shell, byte-string stdlib — landed in the last
> thirty-odd sprints and has only been exercised by its own
> tests.** Expect bugs. Release-mode AOT currently hits an LNK2005
> `nod_user_main` collision; debug-mode AOT works. The whole AOT
> trajectory is documented per-sprint in
> [docs/SPRINTS.md](docs/SPRINTS.md) (Sprints 39–44, 49).

A from-scratch Rust + LLVM JIT for the [Dylan programming language](https://opendylan.org), with a graphical IDE, live inspection, and live incremental compilation. Windows-first; macOS second. 64-bit only.

**Architecture: a Dylan front-end on a Rust + LLVM back-end, split at
the DFM IR.** The front-end — lexer, parser, macros, sema, AST → DFM
lowering — migrates to Dylan and self-hosts (the lexer and parser run
inside the driver today, as of Sprint 51). The back-end — DFM → LLVM
codegen, the garbage collector, the JIT, the AOT linker, the runtime,
the FFI plumbing — is Rust + LLVM and stays that way permanently. It's
the division `rustc` and GHC draw: the language hosts everything above
the IR; the systems substrate hosts codegen and the collector. Full
statement and migration roadmap in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

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

## Current status — WIP checkpoint through Sprint 49

The workspace is real code, not placeholders — and a lot more of it
than the original sprint plan anticipated. The compiler JITs and
AOT-compiles non-trivial Dylan programs, the IDE runs as a native
Win64 EXE written in Dylan, the **Dylan-in-Dylan parser parses the
test corpus end-to-end** (Sprint 46, post-GAP-011), multi-file AOT
builds via `.prj` project files work, and the language surface is
incrementally migrating into stdlib macros (`unless`, `when`,
`for-each`, `with-cleanup`, `cond`) instead of growing AST nodes.
Headlines:

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

**Dylan lexer in Dylan** (Sprints 45a, 45b) —
`tests/nod-tests/fixtures/dylan-lexer.dylan` defines the full
`<token>` class hierarchy (16 concrete subclasses including
`<integer-token>`, `<string-token>`, `<symbol-token>`,
`<keyword-token>`, `<operator-token>`, `<eof-token>`, …), `<span>`,
and per-class generics (`colour-of`, `token-kind-name`,
`print-token`). 45b shipped the real `lex` function; 49c retired the
O(N²) walk-from-byte-0 anti-pattern in `offset-to-line-col` for an
O(N) sliding cursor. `nod-driver dump-dylan-tokens <path>`
AOT-compiles the embedded lexer source and prints a text-diff oracle
dump for any input file.

**Dylan parser in Dylan** (Sprint 46) — adjacent
`tests/nod-tests/fixtures/dylan-parser.dylan` parses the full
test corpus to an AST. `define class` (with superclass list +
slot specs), `define generic`, multi-clause statements
(`if/elseif/else`, `block/cleanup`, `select/otherwise`),
`for` iteration headers, infix word-operators `mod` / `rem`,
and method signatures all parse cleanly. `nod-driver parse-dylan
<path>` builds + caches the parser EXE and prints the AST dump.
The parser-self-host milestone (parse the whole corpus + GC-stress
with heavy parsing) was the gating use case that surfaced GAP-011;
post-fix, the milestone is unblocked.

Roughly 60+ kLOC of Rust across the crates, plus the Dylan fixtures
above. What's in tree today, by area:

**Front end** (`nod-reader`, `nod-namespace`, `nod-macro`). Lexer
+ AST (Sprint 02), fragment-based infix parser (Sprint 03),
definition forms + body parser (Sprint 04), LID files + library /
module dependency graph (Sprint 05), pattern-rule macro expander +
common macro shapes (Sprints 17, 18). Stdlib-macro migrations:
`unless` / `for-each` (Sprint 25), `cond` (Sprint 49b). Multi-file
sema entry point — `compile_files_for_aot` parses each file then
merges ASTs before lowering (Sprint 44A–B; AST-level merge hotfix
post-44E). Sprint 46 added `define class` (superclass list + slot
specs), `define generic`, multi-clause statements (`if/elseif/else`,
`block/cleanup`, `select/otherwise`), `for` iteration headers, and
method signatures — enough for the Dylan-in-Dylan parser to parse
the full test corpus. Sprint 47 lit up multi-value return /
multi-binder `let (a, b) = …` (GAP-003 closure).

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

- **Compile-time precise-roots verifier** (the "alloca tracker") not
  in tree. Sprint 48b's GAP-011 fix makes the invariant true by
  construction today; the static verifier that would catch a future
  codegen regression at build time is queued. See
  `GAP-011_GC_team_writeup.md` for the spec.
- **Sprint 48 Phase B / C unshipped.** The `is_no_alloc` field on
  `Computation::DirectCall` and `SealedDirectCall` exists and
  codegen reads it, but nothing sets it to `true` in production —
  the annotation pass + tests + docs are queued as task #288. Cost
  is felt as broken test-crate `Computation::DirectCall`
  constructors (missing field) that block `cargo test --workspace`.
- **`*` repetition in the macro engine.** `cond` ships with a
  fixed arity cap (1–4 test/body pairs + `otherwise`) because the
  pattern language doesn't yet have a `*` postfix. Adding it
  unblocks N-arm `case`, `select`, `for`, and removes the cap on
  `cond`. Queued.
- **Release-mode AOT** (LNK2005 `nod_user_main` collision in the
  staticlib build). Debug-mode AOT works.
- **`MessageBoxW` from a WNDPROC inside a DirectX-rendered window**
  (Sprint 41f investigation). Workaround: `SetWindowTextW` for
  in-app notifications.
- **`empty?` can't be specialised on `<byte-string>` yet** —
  Sprint 16's lower-time list-builtin shortcut intercepts the call
  before generic dispatch (use `size(s) = 0` for now).

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

## Where we're going

No fixed schedule — this project moves at whatever cadence is honest
for a from-scratch language implementation maintained alongside seven
others. But the *direction* is concrete.

**Next few sprints** (Sprint 50 and adjacent):

- **Close Sprint 46 properly.** Run `parse-dylan` over every fixture
  in the corpus + a heavy-alloc GC stress pass; declare the
  parser-self-host milestone done with a one-paragraph headline
  retro. The GAP-011 fix unblocked it; the work left is mostly
  bookkeeping.
- **Sprint 48 follow-up (#288).** Annotate stdlib primitives with
  `is_no_alloc`, add a fixed-point analysis that propagates it
  through user-defined functions, write the tests, fix the broken
  test-crate constructors. Restores `cargo test --workspace` to a
  clean run and saves a real number of slab slots on hot paths.
- **The alloca tracker.** Post-codegen LLVM-IR verifier that proves
  every load from a Word-typed alloca is dominated by either a
  fresh store or a post-safepoint reload+writeback. Catches the
  GAP-011 bug class at compile time forever after. Spec is in
  `GAP-011_GC_team_writeup.md`.

**Front-end self-hosting** (the "year-3" trajectory — arrived early,
at Sprint 51). The plan was a slow march; the lexer and parser turned
out to work as soon as they were written, so the front-end migration
is real and shipping. The permanent target is the **Dylan front-end /
Rust+LLVM back-end split at DFM** ([docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)):

- ✅ **Lexer (live).** `nod-driver --lex-with-dylan` runs the
  Dylan-written lexer (`dylan-lexer.dylan`) for the whole front-end,
  byte-identical to the Rust lexer on the corpus. Statically linked
  into the driver as an AOT `.obj`.
- ✅ **Parser (verify + AST emit).** `--verify-parse` runs the
  Dylan parser alongside the Rust one and asserts they agree on every
  fixture (it caught a *Rust* parser gap on its first run).
  `dump-dylan-ast` has the Dylan parser emit a real AST across a wire
  format ([docs/DYLAN_AST_WIRE.md](docs/DYLAN_AST_WIRE.md)) that the
  Rust side decodes.
- ⏳ **Macro expander, sema, AST → DFM lowering → Dylan** (Sprints
  52+). Each migrates by the same proven pattern: write in Dylan,
  AOT-compile `--library`, static-link into the driver, bridge across
  a committed wire format, validate in verify-mode against the Rust
  phase, then default.
- **The back-end never moves.** Codegen, GC, JIT, and the linker are
  Rust + LLVM for the life of the project. "Front-end in Dylan" is the
  goal, not a step toward "everything in Dylan" — DFM is the floor.
  The Rust front-end phases remain as the verify-mode reference oracle
  until each Dylan phase is the proven default.

**Language-surface-in-stdlib direction** (continuing Sprints 25, 49b):

- **Sprint 49e — `*` repetition in the macro engine.** Removes the
  fixed-arity cap on `cond` and unlocks clean N-arm `case`,
  `select`, `for`, `with-*`. This is the single biggest unlock for
  growing the language without growing the AST.
- After `*` lands: migrate `case`, `select`, and the various `with-*`
  forms from hardcoded parser arms to stdlib macros. The AST gets
  smaller; the stdlib gets a few hundred more lines of Dylan
  source.

**Stretch goals** (named, not committed):

- `gc.statepoint` precise-roots emission via LLVM intrinsics
  (replaces the current per-call-site slab pattern). Currently the
  precise-roots machinery is hand-rolled; routing through
  `gc.statepoint` would let LLVM's register allocator know about
  GC roots natively. Bigger surgery, much cleaner result.
- Release-mode AOT (resolve the `nod_user_main` LNK2005).
- macOS port (`aarch64-apple-darwin`). The non-runtime crates are
  already platform-clean; the cost is replacing the Win32 surface
  with a Cocoa one. Same `c-ffi` shape, different `define interface`
  declarations.

## Sibling-compiler portfolio

All projects listed here are under active development. None are
finished or production-ready.

This is a family of from-scratch language implementations — mostly
Rust + LLVM JITs, plus one native-assembly Forth — maintained in
parallel by the same developer. They share GC design, runtime
conventions, calling-convention rules, and JIT infrastructure, but
not AST, IR, or semantic analysis.

**How the portfolio works in practice:** focus shifts between projects
freely, driven by what's interesting, what's blocked, and frankly by
boredom with any one stream. That turns out to be a feature: each
language exercises the shared GC and runtime from a different angle.
A GC bug found while stress-testing the Dylan rope allocator gets
fixed once and benefits every runtime. A Forth word that tickles a
stack-scanning edge case becomes a regression fixture for the Lisp
collector. Running eight languages through the same collector means
there's rarely a shortage of workloads to find the next crash.

| Project | Language / notes | Implementation |
|---|---|---|
| NewM2 | Modula-2 (PIM 4 + ISO 10514-1) | Rust + LLVM JIT/AOT |
| NewCP | Component Pascal | Rust + LLVM JIT/AOT |
| NewCormanLisp | Common Lisp — page-heap GC, Win64 GUI | Rust + LLVM JIT |
| NewBCPL | BCPL | Rust + LLVM JIT |
| NewFB | FasterBASIC | Rust + LLVM JIT/AOT |
| WF64 | 64-bit subroutine-threaded Forth, Win64 native | Hand-written x64 asm |
| FactorForth | Factor-style concatenative / ANS Forth research | Rust + LLVM |
| **NewOpenDylan** | **Dylan — JIT + AOT + Win64 IDE written in Dylan** | **Rust + LLVM JIT/AOT** |

## Licence

Dual-licensed under MIT and Apache-2.0. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
