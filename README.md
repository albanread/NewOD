# NewOpenDylan

> ⚠️ **Work in progress — not a usable Dylan implementation yet.**
> Sprint 55b is complete; Sprint 56 is designed (see
> [docs/SPRINTS.md](docs/SPRINTS.md) and
> [docs/journal/](docs/journal/) for the running log).
> Each sprint is nominally two weeks of effort, but real-world ratios
> on the sister projects say plan for **several more years** before
> NewOpenDylan reaches a state a Dylan programmer can actually rely on.
> Treat this repo as a design diary with running code, not a release.
>
> ⚠️ **The GC is precise but not yet verified at compile time.**
> The collector runs the Sprint 23 page-heap NewGC backend with a
> precise-roots client: codegen spills live GC roots to per-call-site
> stack-slot slabs, brackets allocating calls with
> `nod_aot_begin_safepoint` / `nod_aot_end_safepoint`, and reloads
> the slabs after the call so GC-driven relocation is observed.
> Sprint 48b (GAP-011) closed a precise-root staleness bug — function
> parameters now live in stable home allocas across block boundaries.
> The whole parser corpus parses without GC crashes; the IDE's rope
> buffer survives heavy churn.
>
> *What's still on the door:* the post-codegen "alloca tracker"
> verifier that would catch this class of bug at compile time is
> queued but not built. Today the invariant is true by construction;
> a future codegen change could regress it silently.
> See `GAP-011_GC_team_writeup.md`.
>
> ⚠️ **AOT mode compiles real Win64 EXEs, but the whole stack is new
> and lightly tested.** `nod-driver build foo.dylan -o foo.exe`
> produces a standalone EXE that calls Win32 APIs (CreateWindowExW,
> the COM Direct2D / DirectWrite chain, GetOpenFileNameW, …) via
> linker-resolved IAT. Multi-file builds (`nod-driver build a.dylan
> b.dylan … -o foo.exe`) merge ASTs before lowering so all definitions
> are visible across files. `.prj` project files (TOML — `name` /
> `sources` / `output`) keep the file list with the sources.
> `nod-ide.exe` — the graphical Windows IDE written in Dylan — builds
> this way. Release-mode AOT currently hits an LNK2005
> `nod_user_main` collision; debug-mode AOT works.

A from-scratch Rust + LLVM JIT for the [Dylan programming language](https://opendylan.org), with a graphical IDE, live inspection, and live incremental compilation. Windows-first; macOS second. 64-bit only.

**Architecture: a Dylan front-end on a Rust + LLVM back-end, split at
the DFM IR.** The front-end — lexer, parser, macros, sema, AST → DFM
lowering — migrates to Dylan and self-hosts (the lexer, parser, macro
expander, sema, and lowering are all in Dylan today at various stages
of opt-in integration). The back-end — DFM → LLVM codegen, the garbage
collector, the JIT, the AOT linker, the runtime, the FFI plumbing —
is Rust + LLVM and stays that way permanently. Full statement and
migration roadmap in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

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
- **The front-end is migrating to Dylan — and it works.** The lexer, parser, macro expander, sema, and AST→DFM lowering are all written in Dylan and running inside the driver today, three years ahead of the original self-hosting estimate.
- **Everything is new.** This is not a resurrection of the old Harlequin or Open Dylan toolchain. It's a clean-room implementation in Rust + LLVM, with a fresh GC, a fresh IR, and a fresh FFI layer.

What draws us to Dylan specifically:

- **Interactive by design.** The REPL and hot-reload pipeline give you a live Lisp-style development loop — evaluate a form, inspect the result, redefine a method without restarting the process.
- **Lisp dynamism in conventional clothing.** Multiple dispatch, a real condition system, macros, first-class functions — but written in syntax that doesn't require unlearning everything you already know.
- **Full modern Windows API.** Our FFI layer covers ~15 000 Win32 functions across ~350 DLLs, COM via the `windows` crate, D2D/DirectWrite/DXGI, and callback trampolines. Writing a native Windows GUI in Dylan is not a research project here; it's what we do.
- **Not simple, but interesting.** Dylan rewards the effort. The object system has sealing and compile-time dispatch. The GC is precise and generational. The macro system is hygienic. It's the kind of language where the more you put in, the more you get back.

---

## Where to start

- **[docs/manual/](docs/manual/index.md)** — the diagram-rich **manual** of the language and compiler. Start here for an explanatory tour; browse it offline with `pwsh tools/doccrate/Browse-Docs.ps1`.
- **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** — the Dylan-front-end / Rust+LLVM-back-end split. Read this to understand the migration shape. The phase-by-phase status table is kept current here.
- **[docs/MANIFESTO.md](docs/MANIFESTO.md)** — design constraints. Read this first.
- **[docs/PLAN.md](docs/PLAN.md)** — language survey + 12-phase implementation plan.
- **[docs/SPRINTS.md](docs/SPRINTS.md)** — two-week sprint breakdown with deliverables, acceptance criteria, and demos.
- **[docs/journal/](docs/journal/)** — per-session engineering log. The `journal/README.md` has an index; the most recent entries cover Sprints 53–56.
- **[docs/DEFERRED.md](docs/DEFERRED.md)** — features explicitly out of scope (with rationale).
- **[docs/specs/](docs/specs/)** — per-sprint design specs drafted ahead of implementation.

## Workspace layout

```
NewOpenDylan/
├── Cargo.toml              # workspace root, `unsafe_op_in_unsafe_fn = "deny"`
├── .cargo/config.toml      # LLVM 22.1 env (activated in Sprint 06)
├── .github/workflows/ci.yml
├── latest_status.md        # authoritative session handoff (always current)
├── docs/
│   ├── ARCHITECTURE.md     # Dylan-front-end / Rust+LLVM-back-end split (source of truth)
│   ├── MANIFESTO.md        # design constraints (read first)
│   ├── PLAN.md             # 12-phase implementation plan
│   ├── SPRINTS.md          # 2-week sprint breakdown
│   ├── DEFERRED.md         # explicitly out-of-scope features
│   ├── GC.md               # GC design notes
│   ├── DFM.md              # IR design notes
│   ├── SEALING.md          # sealing / dispatch design
│   ├── MACROS.md           # macro expander design
│   ├── DYLAN_TOKEN_WIRE.md # wire format: Dylan lexer → Rust
│   ├── DYLAN_AST_WIRE.md   # wire format: Dylan parser → Rust
│   ├── DYLAN_SEMA_WIRE.md  # wire format: Dylan sema → Rust
│   ├── NCL_GC_FEEDBACK.md  # cross-project GC notes from NewCormanLisp
│   ├── journal/            # per-session engineering log (index in journal/README.md)
│   ├── manual/             # diagram-rich language + compiler manual (24 pages)
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
│   ├── nod-dylan/          # ported Dylan kernel library (sources only)
│   └── newgc-core/         # vendored GC core (shared with sibling portfolio)
└── tests/
    ├── nod-tests/          # end-to-end JIT regression tests
    │   └── fixtures/       # Dylan source fixtures (incl. front-end self-host)
    └── nod-od-suite/       # OpenDylan-compatibility test runner
```

## Current status — Sprint 55b complete, Sprint 56 designed

The workspace is real code — a working JIT, a working AOT compiler, and
a graphical IDE written entirely in Dylan. More significantly: the
**Dylan front-end is live**, three years ahead of the original plan.
The lexer, parser, macro expander, sema, and AST→DFM lowering are all
written in Dylan and running inside the driver, at various stages of
opt-in integration on the path to becoming the default.

### Front-end self-hosting (the headline)

| Phase | Status | Dylan source |
|---|---|---|
| **Lexer** | ✅ live (`--lex-with-dylan`) | `fixtures/dylan-lexer.dylan` |
| **Parser** | ✅ **default** (Rust is the fall-back; `--parse-with-rust` opts out) | `fixtures/dylan-parser.dylan` |
| **Macro expander** | ✅ live (opt-in `NOD_EXPAND_WITH_DYLAN`, Sprint 52) | `fixtures/dylan-macro*.dylan` (6 files) |
| **Sema / namespace** | ✅ **load-bearing opt-in** (`--sema-with-dylan` / `NOD_SEMA_WITH_DYLAN`, Sprint 54): back-end consumes Dylan `SemaModel`, 38/38 byte-match gate | `fixtures/dylan-sema.dylan`, `dylan-c3.dylan` |
| **AST → DFM lowering** | ◐ **load-bearing opt-in** (`--lower-with-dylan` / `NOD_LOWER_WITH_DYLAN`, Sprint 55): stmts, exprs, control flow, slot accessors, `instance?`, `make`, sealed dispatch — 27 fixtures, 0 mismatches; closures/blocks still fall back to Rust | `fixtures/dylan-lower.dylan` |

The migration pattern is the same across every phase: write in Dylan →
AOT-compile to a static `.obj` → link into the driver → bridge across a
locked wire format ([DYLAN_TOKEN_WIRE.md](docs/DYLAN_TOKEN_WIRE.md),
[DYLAN_AST_WIRE.md](docs/DYLAN_AST_WIRE.md),
[DYLAN_SEMA_WIRE.md](docs/DYLAN_SEMA_WIRE.md)) → verify-mode against
the Rust phase → default. The Rust phase stays as the oracle until the
Dylan phase is the proven default.

### `nod-ide.exe`

A Windows source editor written **entirely in Dylan**, AOT-built to a
~1 MB EXE via `nod-driver build`. Source lives in `fixtures/`:

| File | Lines | Content |
|---|---|---|
| `nod-ide.dylan` | ~917 | entry point, window setup, message pump |
| `ide_syntax.dylan` | ~507 | syntax-colouring engine |
| `ide_helpers.dylan` | ~251 | text utilities, layout helpers |
| `ide_rope.dylan` | ~282 | rope ↔ IDE bridge |
| `ide_win_calls.dylan` | ~91 | Win32 call wrappers |

The editor provides: File menu, Help → About, DirectWrite text
rendering, scrolling, a `<rope>`-backed live editing buffer, a blinking
cursor via `WM_TIMER`, full keyboard navigation (arrows, Home/End,
PgUp/PgDn, Ctrl+Home/End), mouse-click cursor placement via
`HitTestPoint`, syntax colouring (keywords, comments, strings, numbers,
class names each in a distinct colour), and a line-number gutter.

### What's in tree, by area

**Front end** (`nod-reader`, `nod-namespace`, `nod-macro`). Lexer +
AST, fragment-based infix parser, definition forms, LID files + library
/ module dependency graph (Sprints 02–05), pattern-rule macro expander
(Sprints 17–18), stdlib-macro migrations (`unless`, `when`, `for-each`,
`with-cleanup`, `cond`). Multi-file sema entry point (Sprint 44).

**Semantic analysis** (`nod-sema`). Classes + slots, single dispatch
(Sprint 12), multiple-inheritance slot layout (Sprint 14), sealing
analysis + compile-time dispatch resolution (Sprint 15),
forward-iteration protocol (Sprint 20), `<table>` + content-based
hashing (Sprint 22), closures with cell promotion (Sprint 24),
`<c-struct>` field accessors (Sprint 34), universal `=` dispatch,
env-merge correctness in `lower_if`.

**IR + codegen** (`nod-dfm`, `nod-llvm`). Dylan Flow Machine typed SSA
IR with dispatch nodes; LLVM codegen; `ObjectCache` per-module + sidecar
manifest; cross-process bitcode replay with named symbol resolution
(Sprint 38 family); cross-module sealed-direct call fallback; short-
circuit `|` and `&` via 3-block CFG lowering.

**Runtime** (`nod-runtime`, `newgc-core`). Tagged Words, boxed
`<integer>`, strings, symbols, vectors, lists, ranges, stretchy vectors
(Sprints 09–10, 16, 20); NewGC page-heap backend (Sprint 23);
method-lookup runtime + dispatch (Sprints 12–15); conditions + non-local
exit (Sprint 19); `<table>` open-addressing + FNV-1a hash (Sprint 22);
cells + environments (Sprint 24); literal pool + static-area allocator;
callback trampoline pool (Sprint 32); COM handle registry (Sprint 35);
byte-string primitives (Sprint 42a).

**Stdlib** (`nod-dylan/dylan-sources/stdlib.dylan`). Pre-compiled into a
long-lived JIT engine; merged into AOT modules at build time. Provides
`size`, `concatenate`, `reduce`, `map`, `do`, `<table>` generics,
`<c-struct>` field accessors, `<byte-string>` methods (12 ops including
`starts-with?`, `find-substring`, `as-uppercase`), `for-each`, `unless`,
`when`, `cond` macros.

**FFI** (`nod-winapi`, FFI machinery in `nod-runtime` / `nod-sema` /
`nod-llvm`). Vendored Windows API metadata at `data/windows_api.db`;
~15K primitive-typed Win32 functions across ~350 DLLs; per-module API
stub table + Win64 trampolines arity 0..=8; Win32 integer constants;
`<c-string>` / `<c-wide-string>` marshaling + `$NULL`; JIT-time
bare-name materialisation (call `Beep(440, 1000)` without `define
c-function`); callback trampolines; `<c-struct>` family; COM via
`windows` crate.

**AOT mode** (Sprints 39–44, 51–55). `nod-driver build foo.dylan -o
foo.exe` produces a standalone Win64 EXE. Multi-file, `.prj` project
files, user-defined classes, Win32 callbacks, the COM device chain,
bare-name Win32 calls all work. The Dylan front-end phases (lexer,
parser, macro, sema, lowering) themselves compile and link back into the
driver this way — AOT is the bootstrap mechanism for self-hosting.

**Driver / REPL** (`nod-driver`). REPL loop, `dump-tokens`, `dump-ast`,
`dump-graph`, `dump-dfm`, `dump-llvm`, `eval`, `build` (AOT). Dylan
front-end subcommands: `dump-dylan-tokens` (Sprint 45a),
`dump-dylan-ast` (Sprint 51d), `dump-dylan-sema` (Sprint 54), plus
flags `--lex-with-dylan`, `--parse-with-rust` (opt-out from default
Dylan parser), `NOD_EXPAND_WITH_DYLAN`, `--sema-with-dylan` /
`NOD_SEMA_WITH_DYLAN`, `--lower-with-dylan` / `NOD_LOWER_WITH_DYLAN`.

### What's still warning-flagged

- **Compile-time precise-roots verifier** (the "alloca tracker") not
  in tree. Sprint 48b's GAP-011 fix makes the invariant true by
  construction today; the static verifier that would catch a future
  codegen regression at build time is queued.
  See `GAP-011_GC_team_writeup.md`.
- **Latent codegen first-writer-wins hole (deferred, no live repro).**
  When a GC-typed temp is live across a merge from ≥2 edges carrying
  different reloaded SSA values, `note_successor_entry_temps`'
  first-writer-wins can in principle install the wrong one. No
  reproducer found across the parser corpus, rope, and `pick` 40k-iter
  control. Recommended fix if one surfaces: the `legalize_block_params`
  DFM post-pass (prototype in the journal).
- **`dylan-sema` oracle byte-match gate** (`sema_topnames.rs`) still
  planned but not wired in. The output format is clean and stable, so
  the byte-match against `format_sema_model` is unblocked.
- **Release-mode AOT** (LNK2005 `nod_user_main` collision in the
  staticlib build). Debug-mode AOT works.

**Test counts:** `cargo test --workspace --tests` runs ~130+ tests
across the workspace. Interactive AOT IDE tests are `#[ignore]`'d; run
with `--ignored`. Rope and multi-file AOT tests carry ~10s cargo-build
overhead each.

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

See [docs/SPRINTS.md](docs/SPRINTS.md) for the full per-sprint history
and [bench/richards.md](bench/richards.md) for the Richards
sealing-vs-open performance trajectory.

## Where we're going

No fixed schedule — this project moves at whatever cadence is honest
for a from-scratch language implementation maintained alongside seven
others. But the *direction* is concrete.

**Sprint 56 — consolidation and flip-to-default** (current focus):

The Dylan lowering (`--lower-with-dylan`) byte-matches 27 fixtures with
0 mismatches, but still falls back to Rust for closures/blocks and for
four ownership areas the Rust phase still manages:

- **Class table ownership.** The class-id / slot-descriptor table is
  currently written by the Rust sema; the Dylan lowering needs to
  produce and own it so the back-end can consume it from either front-end
  interchangeably.
- **Function-side table ownership.** Four tables (generic-function
  descriptors, sealed-method dispatch entries, cache slots, stub table
  indices) are similarly Rust-owned today.
- **Side-effect replay.** Some per-eval Rust-side actions (intern pools,
  static-area registrations) need a Dylan-side replay path before the
  flag can be the default.
- **Flip to default, retire Phase 3/4.** Once the above are done, the
  `--frontend-with-dylan` umbrella flag becomes the default and the
  per-stage `--…-with-dylan` flags are retained only as opt-out escape
  hatches. The Rust Phase 3 (macro) and Phase 4 (sema) implementations
  are retired.

Design and adversarial review for Sprint 56 are in
[docs/journal/](docs/journal/) (2026-06-07 entry).

**After Sprint 56 — Phase 5 (lowering) maturity:**

- Complete 55c: closures and blocks in `dylan-lower.dylan`.
- Extend the byte-match gate to cover the full corpus.
- Retire the Rust Phase 5 (lowering) once the Dylan version is the
  proven default for all constructs.

**Language-surface direction** (continuing Sprints 25, 49b):

- **`*` repetition in the macro engine.** The macro engine is now
  Dylan-side (`dylan-macro*.dylan`); `*` repetition can be added there
  without touching Rust. Removes the fixed-arity cap on `cond` and
  unlocks clean N-arm `case`, `select`, `for`, `with-*`.
- After `*` lands: migrate `case`, `select`, and the various `with-*`
  forms from hardcoded parser arms to stdlib macros. The AST shrinks;
  the stdlib gets a few hundred more lines of Dylan source.

**Stretch goals** (named, not committed):

- `gc.statepoint` precise-roots emission via LLVM intrinsics (replaces
  the current per-call-site slab pattern). Bigger surgery, much cleaner
  result; currently the `newgc-core` crate handles this.
- Release-mode AOT (resolve the `nod_user_main` LNK2005).
- macOS port (`aarch64-apple-darwin`). The non-runtime crates are
  already platform-clean; the cost is replacing the Win32 surface with
  a Cocoa one.

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
fixed once and benefits every runtime.

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
