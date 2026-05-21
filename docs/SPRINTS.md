# NewOpenDylan — Sprint Plan

*Drafted 2026-05-15. Companion to [`PLAN.md`](PLAN.md) (the 12-phase
roadmap) and [`MANIFESTO.md`](MANIFESTO.md) (the design commitments).*

## Preamble

The sprint cadence is **two weeks, one developer, one demo**. Each
sprint must end with something the user can run, not a milestone in
a tracker. The trajectory:

- The first sprint produces a `cargo run -p nod-driver -- --version`
  and a workspace skeleton — cheap, demonstrable, unblocks everything.
- Sprints 1–16 cover PLAN.md phases 0–6: workspace → reader → namespace
  → kernel JIT → GC bring-up → classes → sealed multimethod dispatch.
  Sprint 16 ends with a real Dylan example — a slice of
  `simple-richards` — running through sealed dispatch.
- Sprints 17–20 hand off into macros (phase 7) so that the rest of the
  Dylan stdlib can be ported as library code, not compiler code.
- Sprints 21+ (conditions, collections, FFI, stdlib, IDE polish, AOT)
  are sketched only — their detail depends on what falls out of the
  early sprints.

**Compiler first.** This is a manifesto commitment (core
decision 8): no IDE until the compiler can JIT and run non-trivial
Dylan code. Sprints 01–16 are headless — `nod-driver` subcommands
(`dump-tokens`, `dump-ast`, `dump-graph`, `dump-dfm`, `dump-llvm`,
`eval`) and `cargo test` are how we know we are alive. The first
IDE we ship is the first non-trivial Dylan program NewOpenDylan
compiles, calling Win32 directly through `c-ffi` (Sprint 23-ish:
after macros, FFI, and the Windows-FFI runtime stack borrowed
from NewCormanLisp). Open Dylan's `sources/environment/` tree is
the re-implementation target.

**Sibling-project leverage is the budget mechanism.** Where NewM2,
NewCP, NewCormanLisp (NCL), NewBCPL, or NewFB already solved a
problem, we lift the code with attribution rather than rewriting.
Each sprint flags what is lifted vs. what is fresh.

**Tests gate sprints.** The OpenDylan test corpus at
`E:\NewOpenDylan\opendylan-tests\` is the regression battery; sprints
declare which files they intend to make pass.

---

## Sprint 01 — Workspace Skeleton
**Goal:** Compile an empty `nod-driver` binary and print a version banner.
**Length:** 2 weeks
**Phase (from PLAN.md):** 0 — Workspace skeleton

### Deliverables
- [ ] Root `Cargo.toml` with `resolver = "3"`, `edition = "2024"`, workspace lints (`unsafe_op_in_unsafe_fn = "deny"`), shared `[workspace.dependencies]`.
- [ ] Empty crates: `nod-driver`, `nod-reader`, `nod-macro`, `nod-namespace`, `nod-sema`, `nod-dfm`, `nod-opt`, `nod-llvm`, `nod-loader`, `nod-runtime`, `nod-dylan` (source-only crate, no Rust), plus `tests/nod-tests` and `tests/nod-od-suite`.
- [ ] `nod-driver` CLI: `--version`, `--help`, no-op `compile` / `repl` subcommands stubbed.
- [ ] LICENSE files (`MIT OR Apache-2.0`).
- [ ] `docs/` skeleton: `GC.md`, `DFM.md`, `SEALING.md`, `MACROS.md` with one-paragraph stubs.
- [ ] GitHub Actions CI: build + clippy + fmt on Windows x86_64.
- [ ] `README.md` linking PLAN, MANIFESTO, SPRINTS.

### Acceptance criteria
- `cargo build --workspace` clean on `x86_64-pc-windows-msvc`.
- `cargo clippy --workspace -- -D warnings` clean.
- `cargo run -p nod-driver -- --version` prints `nod-driver 0.0.1 (LLVM <version>)`.
- CI green.

### Dependencies
- LLVM version pinned (match NewM2 / NCL major).
- Rust MSRV pinned in workspace `Cargo.toml`.

### Risks
- Bikeshedding crate names. Lock the table from PLAN.md §2.1 verbatim.
- `inkwell` not yet on the latest LLVM major; if so, pin the LLVM
  version to whatever NewM2 currently uses to stay coordinated.

### Sibling-project leverage
- Workspace `Cargo.toml` structure and lints lifted verbatim from
  NewCormanLisp.
- CI workflow lifted from NewM2's `.github/workflows/`.

### Demo
`cargo run -p nod-driver -- --version` from a fresh checkout.

---

## Sprint 02 — Lexer
**Goal:** Tokenise Dylan source into a typed token stream, exposed through `nod-driver dump-tokens`.
**Length:** 2 weeks
**Phase (from PLAN.md):** 1 — Reader + AST

### Deliverables
- [ ] `nod-reader::lex` — state-machine lexer producing `Token { kind, span, text }`. Token kinds: identifier, `<class-name>`, `#"symbol"`, `#:keyword`, `keyword:`, integer literal (decimal, hex `#x`, binary `#b`, octal `#o`), float literal, string literal with escapes, character literal `'a'`, all operator/punctuator forms documented in `E:\opendylan\sources\dfmc\reader\lexer.dylan`.
- [ ] `Span { file_id: u32, lo: u32, hi: u32 }` with an interner for file ids.
- [ ] `nod-reader::format_tokens(src) -> String` debug dump.
- [ ] `nod-driver dump-tokens <file>` subcommand.
- [ ] `tests/nod-tests/reader/`: unit tests using fixtures lifted from `opendylan-tests/sources/dfmc/reader/tests/` (start with the literal-form tests — they're hermetic).

### Acceptance criteria
- Lexer round-trips all `opendylan-tests/sources/dfmc/reader/tests/` fixture inputs (token kinds and text agree with hand-checked expectations for at least 50 fixtures).
- `nod-driver dump-tokens opendylan-tests/sources/testing/cmu-test-suite/dylan-test.dylan` produces a non-empty, schema-stable dump.

### Dependencies
- Sprint 01.

### Risks
- Dylan's lexer has edge cases (numeric prefixes, operator-as-identifier
  with `\+`, hash-keyword distinction). Time-box and defer obscure
  forms with `TODO` tokens.

### Sibling-project leverage
- Span/interner pattern from `newm2-reader`.

### Demo
`nod-driver dump-tokens opendylan-tests/sources/testing/cmu-test-suite/dylan-test.dylan` — schema-stable token stream.

---

## Sprint 03 — Fragments + Infix Parser Core
**Goal:** Parse the Dylan expression grammar into AST nodes, anchored on fragments that carry source locations.
**Length:** 2 weeks
**Phase (from PLAN.md):** 1 — Reader + AST

### Deliverables
- [ ] `nod-reader::Fragment` — token-tree with parentheses/braces/brackets grouped; matches the role of `dfmc/reader/fragments.dylan`.
- [ ] Pratt-style infix parser for: literals, identifiers, function calls, binary/unary operators with Dylan precedence (`+ - * / mod rem`, comparison, `& |`, `:=`), `if`/`unless`/`case`/`select`, `begin … end`, `let`, `local method`, anonymous `method (…) … end`, parenthesised groups.
- [ ] AST as an `enum Expr` with `Span` on every node.
- [ ] `nod-reader::format_ast(expr) -> String` indented dump.
- [ ] `nod-driver dump-ast <file>` subcommand.

### Acceptance criteria
- Round-trips at least 80% of expressions in `opendylan-tests/sources/testing/cmu-test-suite/dylan-test.dylan` to a stable AST dump.
- Operator precedence verified against `opendylan-tests/sources/dfmc/reader/tests/expressions.dylan`.

### Dependencies
- Sprint 02.

### Risks
- Dylan's grammar has context-sensitive bits (statement vs. expression
  position for macros). Defer macro positions to Sprint 17 — for now,
  parse `define …` heads only.

### Sibling-project leverage
- Pratt-parser skeleton from `newcp-reader` (Component Pascal has a
  similar expression-grammar shape).

### Demo
`nod-driver dump-ast` on a hand-written Dylan expression file produces a tree dump matching expectations.

---

## Sprint 04 — Definition Forms + Body Parser
**Goal:** Parse all top-level `define` forms and the body grammar (statements, locals, returns).
**Length:** 2 weeks
**Phase (from PLAN.md):** 1 — Reader + AST

### Deliverables
- [ ] Parser for: `define constant`, `define variable`, `define function`, `define method`, `define generic`, `define class`, `define library`, `define module`. Slot definitions, init keywords, specialisers, return types.
- [ ] `define macro` is parsed *as a fragment* (not expanded) — the body is captured raw for Sprint 17.
- [ ] Statement-level parsing: `for`, `while`, `until`, `block`, `local`, `let`, sequence-of-statements, multiple-value bindings `let (a, b) = …`.
- [ ] `Module:` header comment parsed and attached to the AST root.
- [ ] `nod-driver dump-ast` updated to cover top-level forms.
- [ ] AST round-trip pretty-printer (`format_dylan(ast) -> String`) that produces parseable Dylan; verified on a fixture.

### Acceptance criteria
- Parses every `.dylan` file in `opendylan-tests/sources/testing/cmu-test-suite/` without error (results are AST dumps — semantics not checked yet).
- AST → pretty-print → re-parse is a fixed point for at least 20 hand-picked files.

### Dependencies
- Sprint 03.

### Risks
- `define macro` body syntax (`=> { … }` template) is non-trivial; in this sprint we only need to *skip* past it correctly, not understand it.

### Sibling-project leverage
- Pretty-printer pattern from `ncl-reader`.

### Demo
`nod-driver dump-ast opendylan-tests/sources/dylan/tests/constants.dylan` prints a complete AST.

---

## Sprint 05 — LID Files + Library / Module Graph
**Goal:** Parse `.lid` and `dylan-package.json` manifests; build the library/module DAG; resolve `use` / `import` / `export` / `rename`.
**Length:** 2 weeks
**Phase (from PLAN.md):** 2 — Module graph

### Deliverables
- [ ] `nod-namespace::Lid` parser for the `Library:`, `Files:`, `Library-Pack:`, `Platforms:`, `Synopsis:` keys; `.hdp` parsed for legacy interop.
- [ ] `nod-namespace::Package` parser for `dylan-package.json`.
- [ ] Library / module DAG construction from parsed `define library` / `define module` forms.
- [ ] `use` resolution with `import:` / `exclude:` / `rename:` / `prefix:` / `export:`.
- [ ] Cycle detection with a structured diagnostic.
- [ ] `Binding` resolution: every `Module: foo` source file resolves identifiers against module `foo`'s import set.
- [ ] `nod-driver dump-graph <lid>` subcommand — emits a Graphviz-shaped dump.

### Acceptance criteria
- Loads `opendylan-tests/sources/dylan/dylan.lid` (91 files) and produces a complete module graph without error.
- Loads the kernel library + `common-dylan` test library and resolves cross-library references.
- `dump-graph` output validates with `graphviz` (`dot -Tpng`).

### Dependencies
- Sprint 04.

### Risks
- Platform conditionalisation in LID files (`Platforms: x86-win32`) — handle in a follow-up if not in v0.1.
- LID is line-oriented but tolerant of weird whitespace; budget some bug-hunting.

### Sibling-project leverage
- DAG + cycle-detection from `newcp-loader` / `ncl-loader` (the graph
  shape is identical).

### Demo
`nod-driver dump-graph dylan.lid | dot -Tpng > dylan.png` renders the kernel library's 91 source files grouped by module with `use` arrows.

---

## Sprint 06 — DFM IR Skeleton + Format Dump
**Goal:** Define the SSA IR shape (the "DFM-equivalent") and lower a trivial AST (arithmetic + `let` + direct calls) to it.
**Length:** 2 weeks
**Phase (from PLAN.md):** 3 — Minimal kernel

### Deliverables
- [ ] `nod-dfm`: `Computation` enum with `Call`, `DirectCall`, `PrimOp`, `Const`, `If`, `Return`, `Values`, `BindExit`, `UnbindExit`, `Closure`, `MakeEnvironment`. (Generic dispatch nodes come in Sprint 13.)
- [ ] `Temporary { id, type_estimate }` with a `TypeEstimate` lattice (just `<top>`, `<bottom>`, `<integer>`, `<single-float>`, `<double-float>`, `<character>`, `<boolean>`, `<string>` for now).
- [ ] `Block`-structured IR with explicit entry/exit; SSA invariants checked by a verifier.
- [ ] `nod-sema` (first appearance): AST → DFM for the kernel subset — integer/float literals, arithmetic, comparison, `if`, `let`, top-level `define constant`, `define function` (non-generic), direct function calls within one library.
- [ ] `nod-dfm::format_dfm` indented dump with type-estimate annotations.
- [ ] `nod-driver dump-dfm <file>` subcommand.

### Acceptance criteria
- Lowers a hand-written `kernel-arith.dylan` (a few `define function` doing integer/float arithmetic) to DFM whose dump is reviewable.
- Verifier passes on every emitted function.

### Dependencies
- Sprint 05 (we need binding resolution to know what calls bind to).

### Risks
- Premature commitment to IR opcodes. Keep the enum private to `nod-dfm` and accept that Sprints 10–13 will add fields.

### Sibling-project leverage
- IR shape from `newm2-ir` and `ncl-ir`. The SSA-block + Computation +
  Temporary structure is portfolio-wide.

### Demo
`nod-driver dump-dfm kernel-arith.dylan` shows annotated SSA.

---

## Sprint 07 — LLVM Codegen Thin Slice
**Goal:** Emit LLVM IR from DFM for the kernel subset; JIT-compile and execute a function that returns an integer.
**Length:** 2 weeks
**Phase (from PLAN.md):** 3 — Minimal kernel

### Deliverables
- [ ] `nod-llvm` crate: `inkwell`-based context, module, builder; pinned LLVM major matching NewM2.
- [ ] DFM → LLVM IR lowering for: i64 arithmetic, f64 arithmetic, comparisons, branches, direct calls, returns.
- [ ] JIT execution engine (lifted from `newm2-llvm/src/jit_mm.rs`).
- [ ] `nod-driver eval <expr>` REPL prototype: parse → DFM → LLVM IR → JIT → run → print result. Single-shot, no live image yet.
- [ ] `nod-driver dump-llvm <file>` subcommand prints textual LLVM IR.

### Acceptance criteria
- `nod-driver eval '1 + 2 * 3'` prints `7`.
- `nod-driver eval 'let x = 41; x + 1 end'` prints `42`.
- A hand-written `factorial.dylan` defining `factorial(10)` returns `3628800` when called from `nod-driver eval`.
- LLVM IR dump available for every example.

### Dependencies
- Sprint 06.
- JIT-MM port from NewM2 (one-time cost).

### Risks
- LLVM version drift across the portfolio — coordinate the pin.
- `inkwell` API churn on the targeted LLVM major.

### Sibling-project leverage
- `jit_mm.rs` (Win64 SEH `RtlAddFunctionTable` registration) lifted from NewM2.
- `inkwell` wrapper patterns from NCL.

### Demo
Type expressions at `nod-driver eval` and see them evaluate. `nod-driver dump-llvm` prints the emitted textual IR.

---

## Sprint 08 — REPL Loop + Live Image (no GC yet)
**Goal:** A persistent REPL that keeps defined functions/constants in an arena and lets later expressions call them.
**Length:** 2 weeks
**Phase (from PLAN.md):** 3 — Minimal kernel

### Deliverables
- [ ] `nod-driver repl` mode: read line → parse → lower → JIT → install in module → call. Module is persistent within the REPL session.
- [ ] `nod-loader` (first appearance): per-definition installation, generation counter, dirty-tracking placeholder (full retirement comes later). Lift the shape from `ncl-loader`.
- [ ] FFI stub `format-out(fmt, args …)` lowering to a call into a Rust `extern "C"` shim that prints to stdout. The `c-ffi` proper comes later; this is just one well-known intrinsic.
- [ ] `define constant` and `define function` work across REPL lines.
- [ ] `:dump-dfm <name>` and `:dump-llvm <name>` REPL meta-commands reprint the IR for a definition installed earlier in the session.
- [ ] Arena allocator for any heap data (still no GC); intentionally leaks.

### Acceptance criteria
- Multi-turn REPL: `define function sq (x :: <integer>) x * x end` then `sq(7)` → `49`.
- `format-out("%d\n", sq(7))` prints `49` to stdout.
- `:dump-llvm sq` after the definition prints the JITed IR.
- A "hello, world" via `format-out("hello, world\n")` works end-to-end through the JIT.

### Dependencies
- Sprint 07.

### Risks
- Linking JIT-compiled functions across separate LLVM modules is fiddly
  (symbol resolution). Use a single growing module for now; split
  later.

### Sibling-project leverage
- Loader shape from `ncl-loader`.
- REPL line-reader from NCL's `ncl-driver`.

### Demo
In the REPL: type a function definition, call it on the next line, run `:dump-llvm <name>` to see the JITed IR.

---

## Sprint 09 — GC Phase 1: Tagged Pointers + Allocator + Boxed `<integer>`
**Goal:** Replace the arena with a real heap. Tagged pointers, `<wrapper>` headers, bump allocator, no collection yet.
**Length:** 2 weeks
**Phase (from PLAN.md):** 4 — GC + heap objects

### Deliverables
- [ ] `nod-runtime::heap`: a Rust-side heap with a `PageAllocator` shim (Windows `VirtualAlloc`). Bump-pointer allocation. No collection.
- [ ] Tagged pointer representation (lowest bit: 0 = fixnum, 1 = pointer-to-header). 63-bit fixnum range.
- [ ] `<wrapper>` header (8 bytes): pointer to class metadata, GC info bits.
- [ ] `Value` ABI for the JIT: every Dylan value is one `i64` register-sized word.
- [ ] `nod-llvm` codegen updated to emit tag-check / tag-strip primitives for integer arithmetic. Inline the fast path.
- [ ] Class metadata table (statically interned): `<integer>`, `<single-float>`, `<double-float>`, `<boolean>`, `<character>`, `<symbol>`, `<string>` (still a placeholder layout).
- [ ] `instance?(x, <integer>)` primitive working.
- [ ] `nod-driver dump-heap` REPL meta-command (`:dump-heap`) prints a flat list of allocated objects.

### Acceptance criteria
- `1 + 2` is a fixnum-on-fixnum add with no allocation (verified by `:dump-heap` showing zero new allocations across the call).
- Allocating an out-of-tag-range integer (Sprint 12 territory) is flagged with a clear "not yet supported" diagnostic — not silently wrong.
- `instance?(42, <integer>)` returns true; `instance?(42, <boolean>)` returns false.

### Dependencies
- Sprint 08.

### Risks
- Tag-bit choice (0 = fixnum vs. 1 = fixnum) needs to match what
  arithmetic codegen wants; pick early and document in `docs/GC.md`.

### Sibling-project leverage
- Tagged-pointer scheme and `<wrapper>` header pattern from NCL.
  Adopt Dylan's choice of tag layout (matches `dfmc/runtime`).

### Demo
`:dump-heap` shows zero objects when running pure fixnum code, and starts listing allocations as soon as a `<string>` literal is touched.

---

## Sprint 10 — GC Phase 2: Strings, Symbols, Vectors, Static Roots
**Goal:** Allocate real heap objects (`<byte-string>`, `<symbol>`, `<simple-object-vector>`) and trace them from a static root set. Still no collection — just precise root identification.
**Length:** 2 weeks
**Phase (from PLAN.md):** 4 — GC + heap objects

### Deliverables
- [ ] `<byte-string>`, `<symbol>`, `<simple-object-vector>` layouts in `nod-runtime`. UTF-8 encoded byte-strings.
- [ ] Constructors emit `make-string`, `make-vector` primitive calls.
- [ ] Symbol intern table (lifted from NCL).
- [ ] Static root set: REPL module's top-level bindings.
- [ ] Tracer that walks the static roots and prints the heap graph (no collection yet).
- [ ] `nod-driver dump-heap` subcommand.
- [ ] `:inspect <root-name>` REPL meta-command walks heap references from a named root and prints class + slots; one screen at a time, navigable by typing follow-up reference indices.

### Acceptance criteria
- `format-out("%s\n", "hello")` allocates a `<byte-string>` and prints `hello`.
- `dump-heap` shows the live string and its `<wrapper>`.
- Tracer reports the static root reachability of every allocated object correctly (verified against hand-counted fixtures).

### Dependencies
- Sprint 09.

### Risks
- Symbol interning under JIT — symbols must be value-equal across
  separately-JITed call sites. Use a global table behind a mutex.

### Sibling-project leverage
- Symbol-intern table directly from NCL.
- String layout from NCL (Dylan and CL agree on UTF-8 byte-string).

### Demo
Allocate strings and vectors at the REPL, walk them with `:inspect`.

---

## Sprint 11 — GC Phase 3: Generational Copying Collector + Safe Points
**Goal:** A working stop-the-world generational copying GC, driven by `gc.statepoint`-emitted stack maps and a cooperative safepoint poll.
**Length:** 2 weeks
**Phase (from PLAN.md):** 4 — GC + heap objects

### Deliverables
- [ ] LLVM `gc.statepoint` / `gc.relocate` emission in `nod-llvm` codegen at every call site. `gc.statepoint`-example strategy or the custom strategy already used by NewM2/NCL.
- [ ] Stack-map decoder in `nod-runtime` (lifted from NCL).
- [ ] Safepoint-poll lowering pass: at function entry, loop back-edges, and call returns, emit a load-and-branch against a thread-local "should park" flag.
- [ ] Young-generation copying collector. Old generation as a separately-tracked region (promotion happens on the second survival).
- [ ] Card-marking write barrier in `nod-llvm` (one byte per 512-byte card).
- [ ] GC stress test: a fibonacci-style allocator that triggers thousands of minor GCs.
- [ ] `:gc-stats` REPL meta-command and a `nod-driver --gc-trace` flag that dumps live/used/free per generation, GC count, last-pause time after each collection.

### Acceptance criteria
- A loop that allocates 1M `<byte-string>` objects completes without OOM.
- GC count > 100 over that run, no leaks reported by tracing test.
- `:gc-stats` reflects pulses of allocation and collection across the run.
- `dump-heap` correctness preserved across collections (fixture-based).

### Dependencies
- Sprint 10.

### Risks
- This is the highest-risk single sprint. The `gc.statepoint` lowering
  is the de-risked part (NCL has done it); what's new is wiring it to
  a Dylan-shaped object layout. Budget a buffer week, or split into
  Sprints 11a/11b if needed.
- Win64 SEH interaction with safepoint polls.

### Sibling-project leverage
- **Heavy lift from NCL.** The `gc.statepoint` lowering pass,
  cooperative-park protocol, TLAB design, card-marking barrier, and
  stack-map decoder are all lifted with attribution.

### Demo
The fibonacci-allocator runs through `nod-driver --gc-trace`; the trace stream shows pulses of allocation and collection in real time.

---

## Sprint 12 — Classes + Slots, Single Dispatch Placeholder
**Goal:** `define class` produces real classes with slot layout, getters, setters, `make`, `initialize`. Generic functions exist but dispatch is single-receiver.
**Length:** 2 weeks
**Phase (from PLAN.md):** 5 — Classes, slots, single dispatch

### Deliverables
- [ ] `define class <foo> (<bar>) slot a :: <integer>, init-keyword: a:; … end;` parsed and lowered into class metadata.
- [ ] C3 linearisation algorithm in `nod-sema` (port of `dispatch.dylan`'s C3 implementation).
- [ ] Fixed-offset slot layout for single inheritance.
- [ ] Auto-generated getter and setter generics: `a(p)`, `a(p) := v`.
- [ ] `make(<foo>, a: 1, b: 2)` working through a `make` generic.
- [ ] `initialize(obj, #key)` user-overridable.
- [ ] Single-dispatch generic functions: look up methods by receiver class; ignore other specialisers for now.
- [ ] `instance?(x, <foo>)` exact + subclass.
- [ ] `nod-driver dump-classes` subcommand and `:classes` REPL meta-command — list classes, dump slots + getter/setter for one.

### Acceptance criteria
- A hand-written `point.dylan` defining `<point>` with `x`, `y`, plus a `distance` method, computes `distance(make(<point>, x: 3, y: 4))` → `5.0`.
- C3 linearisation matches Python's `mro()` on the same class graph for 10 fixtures (sanity check — same algorithm).
- `:classes` lists the kernel classes installed so far; `:classes <point>` dumps its slot table.

### Dependencies
- Sprint 11.

### Risks
- Slot inheritance under MI is non-trivial; defer the *MI* case to
  Sprint 14 — this sprint only needs single inheritance to work.

### Sibling-project leverage
- C3 algorithm is small and standard; port from any reputable
  reference (Python or `dispatch.dylan`).

### Demo
Define a `<point>` class at the REPL, instantiate it, call `distance`. `:classes <point>` dumps the slot table.

---

## Sprint 13 — DFM Dispatch Node + Method-Lookup Runtime
**Goal:** Introduce the `<dispatch>` IR node, a runtime method-lookup function, and inline caches at call sites for unsealed generics.
**Length:** 2 weeks
**Phase (from PLAN.md):** 5–6 — single → multimethod

### Deliverables
- [ ] `nod-dfm`: `Computation::Dispatch { generic, args }` and `Computation::DirectCall { method, args }`. Lowering chooses `Dispatch` for generic calls, `DirectCall` for `define function`.
- [ ] Multimethod method-lookup algorithm in `nod-runtime`: given a generic and argument tuple of classes, return the most-specific applicable method or signal `<no-applicable-methods-error>` (signalling fully proper is a Sprint 19 deliverable; for now, panic with a diagnostic).
- [ ] Per-call-site monomorphic inline cache: one-entry cache keyed on the receiver's `<wrapper>`; cache miss falls through to the lookup.
- [ ] `nod-llvm` emits the inline-cache check inline at each call site.
- [ ] `add-method` / `remove-method` operations on a generic; method table is a sorted `Vec<Method>` with a generation counter.
- [ ] Cache invalidation: bump the generation counter on `add-method` / `remove-method`; inline caches compare generation on each call.
- [ ] `:dispatch-stats <generic>` REPL meta-command + `nod-driver dump-dispatch` listing each call site for a generic with its current cache state (cold / monomorphic / polymorphic) and the generation it was last validated against.

### Acceptance criteria
- A two-method generic (`area(<circle>)`, `area(<square>)`) dispatches correctly on both receiver classes; the inline cache reports monomorphic after several calls on the same class.
- Adding a third method invalidates the cache; next call goes through the lookup; cache repopulates.
- `:dispatch-stats` reflects each transition.

### Dependencies
- Sprint 12.

### Risks
- Inline-cache thread-safety. Use atomic generation + relaxed loads
  for the cache fields; document the memory model in `docs/SEALING.md`.

### Sibling-project leverage
- Inline-cache scheme is conceptually portable from NCL's generic
  function caches, but the data shape is Dylan-specific.

### Demo
Define two methods, call the generic from the REPL on each receiver type, run `:dispatch-stats area` and see the cache transition cold → monomorphic.

---

## Sprint 14 — Multiple Inheritance + Slot Layout
**Goal:** Classes with multiple superclasses, C3-driven slot layout, fixed-offset access where possible, indirect fallback otherwise.
**Length:** 2 weeks
**Phase (from PLAN.md):** 5 — Classes, slots, single dispatch

### Deliverables
- [ ] MI in `define class … (<a>, <b>) …`; C3 linearisation produces the class precedence list (CPL).
- [ ] Slot layout algorithm: walk the CPL, assign fixed offsets when all paths agree, fall back to a per-class indirection table when they don't (matches Dylan's `slots-have-fixed-offsets?-bit`).
- [ ] Slot accessor codegen consults the layout decision and emits either a direct load or a hash-lookup.
- [ ] `next-method` machinery — methods can call the next-most-specific method.
- [ ] `:classes <name>` slot listing distinguishes fixed-offset slots ("`@N`") from indirect ones ("`[indirect]`").

### Acceptance criteria
- A diamond hierarchy `<top>` → `<a>`, `<b>` → `<d>` with slots in `<a>` and `<b>` works; `<d>` instances can read/write both.
- A more pathological hierarchy that forces indirect layout is exercised by a fixture, and the indirect access works.
- `next-method` chain in a 4-deep inheritance walk produces the right sequence.

### Dependencies
- Sprint 13.

### Risks
- The fixed-vs-indirect-layout decision is the trickiest piece of
  Dylan's class system. Cross-reference `E:\opendylan\sources\dylan\
  class.dylan` lines 19-39 closely and put the algorithm in
  `docs/CLASSES.md`.

### Sibling-project leverage
- None — this is Dylan-specific. NCL has single dispatch only.

### Demo
Diamond-inheritance fixture from `opendylan-tests/sources/dylan/tests/classes.dylan` (subset) runs at the REPL.

---

## Sprint 15 — Sealing Analysis + Compile-Time Dispatch Resolution
**Goal:** Honour `sealed` declarations on classes and generics; resolve dispatch at compile time when sealing permits; emit direct calls.
**Length:** 2 weeks
**Phase (from PLAN.md):** 6 — Multimethod dispatch + sealing analysis

### Deliverables
- [ ] Parse `sealed` class modifier, `sealed` generic modifier, `define sealed domain g (<a>, <b>);` declarations.
- [ ] `nod-sema` records sealing facts on class and generic objects.
- [ ] `nod-opt`: dispatch-resolution pass that consults sealing facts and the type-estimate lattice. For each `<dispatch>` node, if the static type estimates plus sealing imply a single applicable method, rewrite to `<direct-call>`. This is the analogue of `dfmc/optimization/dispatch.dylan`'s `guaranteed-joint?`.
- [ ] Type-estimate propagation strengthened: receiver-class narrowing through `instance?` guards, slot-type-implied narrowing.
- [ ] Inline caching becomes the *fallback* path; the optimised case is a direct call with no cache.
- [ ] `:dispatch-stats` and `dump-dispatch` add a column marking sealed-direct vs. cached; `nod-driver dump-sealed` lists which generics are sealed over which classes.
- [ ] Live-incremental compilation: if a redefinition would invalidate a sealing assumption, surface a structured diagnostic rather than silently miscompiling (MANIFESTO commitment).

### Acceptance criteria
- A generic with two methods over sealed-domain classes is compiled into two direct calls in the LLVM IR for two specialised call sites (verified by `dump-llvm`).
- Adding a method to a sealed generic from inside the defining library works; from outside, errors at parse/sema with a clear message.
- A redefinition that would break a sealed-domain assumption surfaces a structured diagnostic on stderr and refuses the patch.

### Dependencies
- Sprint 13, Sprint 14.

### Risks
- Sealing analysis is subtle. Limit v0.1 to: single-library sealing,
  no library-merge — that's a v2 deliverable per PLAN.md §2.5(f).

### Sibling-project leverage
- None — Dylan-specific. This is the keystone language feature.

### Demo
A two-method `area` generic over sealed `<circle>` and `<square>` compiles to direct calls; `dump-llvm` shows no dispatch overhead. Add a method from another library; see the sealing diagnostic.

---

## Sprint 16 — `simple-richards` Subset Runs End-to-End
**Goal:** A real Dylan benchmark — a curated slice of `simple-richards` — JIT-compiles and runs against sealed multimethod dispatch.
**Length:** 2 weeks
**Phase (from PLAN.md):** 6 — Multimethod dispatch + sealing analysis

### Deliverables
- [ ] Port enough of `opendylan-tests/sources/testing/benchmarks/richards/simple-richards.dylan` to run under our compiler. Where the source uses macros / collections we haven't ported, hand-rewrite the affected parts and document the deltas (the macros land in Sprint 17–19).
- [ ] Whatever runtime primitives are needed: `<list>` minimum API (cons, car, cdr, null?), basic `<integer>` arithmetic at full speed, sealed dispatch on the task-record class hierarchy.
- [ ] Performance: a dated row in `bench/richards.md`'s History table comparing sealed vs all-`<dispatch>` runs on the same source. The 5× speedup target from the original brief is **dropped** — at this project stage we measure correctness and track perf as a trajectory rather than gating on a ratio. The bench's `ratio >= 0.95` assertion is a regression guard against re-introducing dispatch overhead, not a target. Future ratio improvements land naturally as Sprint 11d (`gc.statepoint`) and Sprint 18 (LLVM optimisation passes) come in.
- [ ] `nod-driver --profile compile-and-run` writes a per-method call-count + resolved-direct/cache-hit/cache-miss summary at end of run.

### Acceptance criteria
- `nod-driver compile-and-run simple-richards-subset.dylan` (or the hand-written `richards-shape` substitute, given upstream Richards' unimplemented forms) produces the expected result count.
- `--profile` output confirms sealed-direct dispatch dominates the tallies.
- A dated measurement row is added to `bench/richards.md`. (No ratio target; the test asserts `>= 0.95` as a regression guard.)

### Dependencies
- Sprint 15.

### Risks
- Richards uses several language features (records, generics, basic
  iteration) that may surface bugs at integration time. Plan for a
  bug-hunting buffer.
- Original Richards depends on macros (`for`, `with-…`); the
  hand-rewritten subset avoids them.

### Sibling-project leverage
- The Richards benchmark itself is open-source under Open Dylan's
  licence; we use it as a fixture per MANIFESTO §13 (open inputs only).

### Demo
**The headline demo.** `nod-driver --profile compile-and-run simple-richards-subset.dylan` produces the expected output and a profile dominated by sealed-direct dispatch. A Dylan programmer reading the source would recognise this as Dylan.

---

## Sprint 17 — Macro Expander: Pattern Matching Engine
**Goal:** Match `define macro` pattern clauses against call-site fragments; substitute templates with hygiene.
**Length:** 2 weeks
**Phase (from PLAN.md):** 7 — Macros

### Deliverables
- [ ] `nod-macro`: pattern parser for `define macro foo { foo ?x:expression } => { … ?x … }` rules.
- [ ] Fragment-level matching: `?x:expression`, `?x:name`, `?x:variable`, `?x:body`, literal tokens.
- [ ] Template substitution preserving source locations.
- [ ] Hygiene: introduced identifiers freshened per expansion.
- [ ] Integration into the compiler pipeline: after parsing top-level forms, before namespace resolution, expand macros that the parser captured as fragments in Sprint 04.
- [ ] `nod-driver dump-expanded <file>` shows post-macro AST.

### Acceptance criteria
- Hand-written `define macro unless { unless ?cond ?body end } => { if (~ ?cond) ?body end };` works at the REPL.
- Source-location preservation verified: a runtime error inside an `unless` body points at the original source span, not the expansion.

### Dependencies
- Sprint 16.

### Risks
- This is the second-highest-risk sprint after GC. The fragment-tree
  matching grammar is non-trivial.

### Sibling-project leverage
- None directly — Dylan macros are unique. But the fragment data
  structure was set up in Sprint 03 precisely to make this possible.

### Demo
Define `unless` as a macro at the REPL; use it in a function.

---

## Sprint 18 — Twelve Most-Common Macro Shapes
**Goal:** Implement enough macro features for the kernel-library macros in `sources/dylan/` to expand.
**Length:** 2 weeks
**Phase (from PLAN.md):** 7 — Macros

### Deliverables
- [ ] Coverage for: `unless`, `when`, `case`, `cond`, `for`, `while`, `until`, `with-open-file`, `block`/`exception`/`cleanup` (parsed; signalling lands in Sprint 19), `let`-extensions, definition macros (`define table`, `define inline function`), function macros.
- [ ] Multiple-rule macros (multiple `{ … } => { … }` clauses with backtracking).
- [ ] Auxiliary rules (`rule` inside `define macro`).
- [ ] Cross-file macro use (definition in module A, use in module B).
- [ ] `nod-driver dump-expanded --trace <file>` prints the full expansion chain for each macro call site, source-location-anchored.

### Acceptance criteria
- `opendylan-tests/sources/dylan/tests/macros.dylan` test fixtures expand correctly (target: 80% of cases).
- `for (i from 1 to 10) format-out("%d\n", i) end` runs at the REPL.

### Dependencies
- Sprint 17.

### Risks
- Backtracking pattern matcher edge cases. Lots of fixture-driven
  debugging.

### Sibling-project leverage
- None.

### Demo
Run `for (i from 1 to 10) … end` at the REPL; `:expand <last>` (or `nod-driver dump-expanded --trace`) shows the expansion chain.

---

## Sprint 19 — Conditions, NLX, Restart Stubs
**Goal:** `block`/`exception`/`cleanup` works; `<condition>` hierarchy exists; `signal` and basic handlers work; restarts present but minimal.
**Length:** 2 weeks
**Phase (from PLAN.md):** 8 — Conditions, NLX, restarts

### Deliverables
- [ ] `<condition>`, `<warning>`, `<error>`, `<serious-condition>` class hierarchy.
- [ ] Handler stack in `nod-runtime` (heap-allocated handler frames, thread-local chain per MANIFESTO §risks(d)).
- [ ] `block (return) … return(v) … exception (<error>) … cleanup … end` codegen — non-local exits use a parallel Dylan-handler chain; OS unwinder runs only `cleanup` clauses.
- [ ] `signal(<my-error>)` walks the handler chain.
- [ ] `<simple-restart>` class and `make-restart` plumbing; `invoke-restart` works for a single bound restart (full restart semantics in v1.x).
- [ ] Replace the panic-on-no-applicable-methods from Sprint 13 with a real `<no-applicable-methods-error>` signal.
- [ ] `:handlers` REPL meta-command prints the live Dylan handler chain at the prompt (and at a breakpoint, once a debugger lands).

### Acceptance criteria
- `block () signal(make(<error>, message: "x")) exception (c :: <error>) c.condition-message end` returns `"x"`.
- `cleanup` clauses run on both normal exit and unwound exit.
- `:handlers` shows the right chain inside an unfinished `block`/`exception` form entered via a paused signal.

### Dependencies
- Sprint 18 (we need macros to write `block`/`exception` body parsing cleanly).

### Risks
- Win64 SEH interaction with the parallel handler chain. Reference
  the M2NEW NLX work; do not invent a new approach here.

### Sibling-project leverage
- NLX scheme from NewM2 (the Win64-SEH-bridged design from M2NEW).

### Demo
Throw and handle an exception at the REPL; `:handlers` prints the chain.

---

## Sprint 20 — Forward Iteration Protocol + Core Collection Types
**Goal:** The collection protocol plus `<list>`, `<simple-object-vector>`, `<stretchy-vector>`, `<range>` working through `for`.
**Length:** 2 weeks
**Phase (from PLAN.md):** 9 — Collections, iteration protocol

### Deliverables
- [ ] Forward iteration protocol (the seven-values contract).
- [ ] `<collection>`, `<sequence>`, `<explicit-key-collection>`, `<mutable-collection>` hierarchy.
- [ ] Concrete: `<list>` (proper + improper), `<simple-object-vector>`, `<stretchy-vector>`, `<range>`. Defer `<table>` (hash), `<deque>`, `<string>` collection-ness, limited collections to v1.x.
- [ ] `map`, `do`, `reduce`, `concatenate`, `size`, `element`, `element-setter`.
- [ ] `for` macro consumes the iteration protocol; sealed dispatch on `<simple-object-vector>` should inline the iterator.
- [ ] `:inspect` REPL meta-command grows a truncated-preview rendering for collections (first N elements, total size); follow-up indices walk into elements.

### Acceptance criteria
- `reduce(\+, 0, range(from: 1, to: 100))` returns `5050`.
- `map(method (x) x * x end, #(1, 2, 3))` returns `#(1, 4, 9)`.
- `opendylan-tests/sources/collections/tests/bit-vector-tests.dylan` runs (subset; full coverage in a later sprint).

### Dependencies
- Sprint 19.

### Risks
- The protocol is intentionally branchy; sealing-driven inlining
  needs to actually fire for performance, otherwise iteration is slow.

### Sibling-project leverage
- None — Dylan-specific iteration protocol.

### Demo
Run `reduce` + `map` at the REPL; profile shows the iterator inlined via sealing.

---

## Sprints 21–35 — Sketches (phases 7+ continuations and 8–12)

> Detail level intentionally lower: each is 2-3 sentences. Concrete
> deliverables are decided after Sprint 20 retrospective. Sprints
> 21–24 have **landed** and carry retrospective notes in place of the
> original sketch; downstream sprints have slid forward to make room.

### Sprint 21 — First-class function values (landed)
Shipped: `<function>` heap class + `<wrong-number-of-arguments-error>`, anonymous-method lifting pass, `nod_funcall_N` / `nod_apply` trampolines, operator-shim registry (`\+`, `\-`, …), top-level / JIT / generic function-ref resolution. `\name` and `method (…) … end` in expression position work as first-class values. Free-variable capture (closures) explicitly deferred to Sprint 24.

### Sprint 22 — `<table>` + hashing (landed)
Shipped: `<table>` heap class with open-addressing buckets, FNV-1a hash + `==` equality machinery via `%object-hash` / `%object-equal?`, `%make-table` / `%table-element` / `%table-element-setter` / `%table-keys` / `%table-values` / `%table-remove-key` primitives wired through the lowerer, stdlib generics over `<table>`. `<not-hashable-error>` lands as a Sprint 19-shaped condition. `<string>` collection conformance slides to a later sprint.

### Sprint 23 — NewGC swap-out (landed)
Replaced the bespoke semispace `Heap` with the sibling-project `PageHeap<DylanLayout>` from `E:\NewGC`. Default feature `newgc-backend`; escape hatch `semispace-backend` keeps the old heap reachable for one sprint of cohabitation. `DylanLayout` binds Dylan's class-driven scan/size machinery to NewGC's `HeapLayout` trait via per-class `LayoutFn` pointers stored on `ClassMetadata`. Card-marking write barrier and root-set discipline unchanged.

### Sprint 24 — Closures (free-variable capture) — landed
Shipped: `<cell>` and `<environment>` heap classes (`nod-runtime/src/closures.rs`), a cell-conversion pass in `nod-sema/src/lower.rs` that promotes captured locals to heap cells and wires per-closure environments through the existing `<function>` `env-ptr` slot, and an env-ptr-conditional dispatch in `nod_funcall_N` / `nod_apply` (ABI choice 1 from the brief — closure bodies grow a synthetic env first parameter; top-level functions keep their Sprint 21 ABI unchanged). The canonical Dylan idiom `let m = 10; map(method (x) x * m end, #(1, 2, 3))` returns `"#(10, 20, 30)"`. By-reference capture: `:=` inside a closure body mutates the underlying cell, and the outer scope reads through the same cell — the textbook ML/Scheme semantics. Captured parameters are cell-promoted alongside captured `let` bindings (so curried `method (a) method (b) a + b end end` works). Test count moves from 410 / 0 / 5 to 421 / 0 / 5 under the `newgc-backend` default; the `semispace-backend` escape hatch stays green. Deferred to follow-up: closure-body arity-0 calls (Sprint 21's `anonymous_method_zero_args` limitation still bites — covered by writing dummy-arg variants in the meantime), env-sharing between sibling closures created in the same scope (each currently allocates its own env even if the capture sets overlap exactly), and deep nesting beyond two levels (works in practice but no explicit acceptance test).

### Sprint 26 — Polish bundle (landed)
Three small surface-level cleanups closed before the c-ffi greenfield, each from a Sprint 21/22/24 DEFERRED bin.

**A. Arity-0 and arity-3+ closure calls.** Sprint 21 wired the env-bound funcall dispatch at arities 1 and 2; arity 0 surfaced a "not supported" lowering error and arity 3+ was implicit. Sprint 26 extends the direct-funcall family to arities 0..=5 (`nod_funcall0`, `nod_funcall3`, `nod_funcall4`, `nod_funcall5` join `nod_funcall1` / `nod_funcall2`), each dispatching on the `<function>`'s `env-ptr` slot exactly like the existing pair. The Sprint 24 brief's `closure_writes_captured_variable` test now exists in its canonical `method ()` form (no dummy arg needed); the new `funcall_arity.rs` test file pins arities 0/3/4/5 with both env-less and closure-with-capture variants. Arities 6+ continue to route through `nod_apply` (8-cap unchanged).

**B. `make(<range>, from:, to:)` keyword-init.** Sprint 21 had to use the `%make-range(1, 100, 1)` primitive workaround because the canonical Dylan spec form left the `by:` slot at zero and the range iterator never advanced. The fix is a one-line default: `<range>`'s `range-by` slot now defaults to fixnum `1` via the new `slot_integer_default` helper. The Sprint 21 headline test `dylan_reduce_plus_zero_range_one_to_hundred_is_5050` now uses `reduce(\+, 0, make(<range>, from: 1, to: 100))` end-to-end, closing the deferral.

**C. Generic-dispatch trampoline for `\name`.** Sprint 22's `register_top_level_functions` had a "first-registration-wins" hack: when `\size` was used as a value, Sprint 21's function-ref machinery had to pick ONE method body's code-ptr to register against the source name. That made `\size(<table>)` call the wrong body. Sprint 26 introduces `FUNCTION_KIND_GENERIC_TRAMPOLINE` (a fourth `<function>` kind-tag value, alongside top-level/lifted-anon/closure) and `make_generic_trampoline_ref`: when `make_function_ref(name, arity)` is asked for a name that already has at least one registered method (`is_generic_defined`), it returns a trampoline `<function>` Word whose `env-ptr` slot stashes the `&'static GenericFunction` pointer. Every `nod_funcall_N` checks the kind-tag first; on a match it routes to `dispatch_via_generic_trampoline`, which walks the applicable-method chain via `nod_dispatch` and tail-calls the most-specific body. `\size(<table>)` now selects the `(t :: <table>)` method, `\size(<list>)` selects the generic-fallback body, and `\size(<range>)` likewise — confirmed by `generic_function_ref.rs`. The Sprint 22 shadow-registration in `register_top_level_functions` is removed.

Test count moves from 425 / 0 / 5 to 441 / 0 / 5 under `newgc-backend` default; semispace escape hatch tracks from 417 to 438. Clippy clean. 5x flake check clean.

### Sprint 25 — Retire `Expr::Unless` in favor of stdlib macros — landed
Shipped: body-shaped macro call parser (`Expr::MacroCall { name, span }` recognised at parse time when `<name>(head…) body… end` appears and `<name>` is in the parser's known-macro set, seeded from the stdlib by `nod-sema::parse_user_module`). `define macro unless` joins `define macro for-each` in `nod-dylan/dylan-sources/stdlib.dylan`; the parser-hardcoded `parse_unless` arm and the `Expr::Unless` AST variant are deleted. `unless (cond) body end` parses to `Expr::MacroCall("unless", ...)`, the stdlib's `unless` macro expands it to `if (~ cond) body else #f end`, and the kernel `Expr::If` lowering handles the rest. As a bonus, `for-each (x in #(1, 2, 3)) total := total + x end` now works as a body-shaped surface — the Sprint 20b deferred call site that the parser couldn't recognise. Test count moves from 421 / 0 / 5 to 425 / 0 / 5 under `newgc-backend` default; semispace escape hatch from 413 / 0 / 5 to 417 / 0 / 5. Deferred: `Expr::Case` retirement (case's multi-arm `=>` syntax doesn't fit the body-shaped recogniser — needs auxiliary `rule` clauses inside `define macro`; tracked for Sprint 26). The `feedback_dylan_lang_defined_by_macros.md` direction is validated: the compiler shrinks by ~70 deleted lines of hardcoded `unless` machinery and the language surface grows by ~10 lines of Dylan macro source.

### Sprint 25b — `c-ffi` library port — first cut
`define interface` for C type marshalling: scalars, pointers, structs, callbacks, `__stdcall`/`__cdcall`. Lift from NewM2's FFI machinery. Demo: call a Win32 API from Dylan. Slid from the original Sprint 25 slot when Sprint 25 absorbed the `Expr::Unless` retirement work.

### Sprint 25c — Windows FFI runtime stack (borrowed from NewCormanLisp)
Port NCL's Windows FFI design (`E:\CL\NewCormanLisp\docs\WINDOWS_FFI.md`, phases 1–6) into `nod-runtime`: surface bootstrap, `%ffi-call` calling-convention dispatcher, Windows API metadata pack loader, foreign-buffer primitives, callback bridge. Files lift nearly verbatim: `win_ffi.rs`, `win_callback.rs`, `win_buffer.rs`, `win_surface.rs`, `win_metadata.rs`. Demo: `define interface user32, function CreateWindowExW … end` from Dylan and successfully open a Win32 window from REPL code.

### Sprint 27 — `format` + `print` + `streams` (`io` library kernel)
Port `opendylan-tests/sources/io/tests/format.dylan`, `print.dylan`, `streams.dylan` against ported `io` library code. Removes the `format-out` FFI shim.

### Sprint 28 — Kernel library port: arithmetic, characters, symbols
Port enough of `sources/dylan/` (`number.dylan`, `character.dylan`, `symbol.dylan`, `boolean.dylan`) that the runtime stops providing these directly and the language defines them in itself.

### Sprint 29 — Dylan-side IDE bring-up: window, message pump, editor surface
First IDE sprint. **All Dylan code**, written against the Sprint 25b Windows FFI stack. Module `nod-dylan/ide-shell` registers a top-level window class, runs the message pump, hosts a single editable text pane and a REPL transcript pane. No syntax colouring yet, no menus — just "the compiler can open a window and let you type into it". Re-implements the scaffolding of `E:\opendylan\sources\environment\framework\` in Dylan.

### Sprint 29b — Dylan-side inspector + dispatch visualisation
With the IDE shell up, port the existing `:inspect` / `:dispatch-stats` / `:classes` REPL commands into IDE panels written in Dylan. Inspector handles every kernel class. Time-travel REPL prototype.

### Sprint 30 — `common-dylan` library port
Port `byte-vector`, `simple-format`, `simple-io`, `simple-random`, `transcendentals`, `threads/`. Run `opendylan-tests/sources/common-dylan/tests/`.

### Sprint 31 — Multi-threaded mutator + cooperative GC across threads
Thread-local TLABs, parking protocol, lock primitives in Dylan-side code. Run `opendylan-tests/sources/app/thread-test/`.

### Sprint 32 — Library-merge optimisation (v2 candidate moved up if cheap)
DFM serialisation, cache-key extension with downstream library hashes, cross-library inlining gated on sealing. May slip to post-v1.

### Sprint 33 — AOT mode — emit a standalone Windows executable
JIT artefacts written out as a PE binary plus a shipped `nod-runtime` static lib. Cache key already covers it; mostly a packaging exercise.

### Sprint 34 — Dylan-side IDE polish: debugger, library browser, sealed-domain visualiser to v1.0 quality
All in Dylan, on top of the Win32 FFI stack: source-stepping debugger, library browser with cross-references, sealed-domain visualiser usable on real programs. Re-implements the feel of `E:\opendylan\sources\environment\debugger\`, `editor/deuce/`, and `commands/`.

### Sprint 35 — macOS port (aarch64-apple-darwin first)
The Dylan-side IDE re-implementation against a `nsapp` / Cocoa equivalent — same shape: Dylan code calling Cocoa through `c-ffi` over a macOS analogue of the Sprint 25b FFI stack. The non-runtime crates are already platform-clean; the cost is rewriting the IDE-side Win32 bindings as Cocoa bindings. Same `c-ffi` shape, different `define interface` declarations.

---

## Dependency Graph (parallelism windows)

If a second developer joins, here is what can run in parallel.

| Sprint | Depends on | Can run in parallel with |
|---|---|---|
| 01 Workspace Skeleton | — | — |
| 02 Lexer | 01 | — |
| 03 Fragments + Parser | 02 | — |
| 04 Definitions + Body Parser | 03 | 05 LID parser (LID is independent of body grammar) |
| 05 LID + Module Graph | 02 | 04 |
| 06 DFM IR Skeleton | 04, 05 | — |
| 07 LLVM Codegen | 06 | — |
| 08 REPL Loop | 07 | 09a tagged-pointer design doc |
| 09 GC: Tagged Pointers | 08 | — |
| 10 GC: Strings, Symbols, Vectors | 09 | 12a class-syntax parsing (already done in 04) |
| 11 GC: Generational Collector | 10 | — |
| 12 Classes + Slots, Single Dispatch | 11 | 17a macro-pattern parser draft |
| 13 DFM Dispatch + Method Lookup | 12 | — |
| 14 Multiple Inheritance | 13 | 15a sealing-syntax parsing |
| 15 Sealing Analysis | 13, 14 | 17 Macro Expander (different code paths) |
| 16 Richards Subset | 15 | — |
| 17 Macro Expander Engine | 04 (just the fragment shape) | 13, 14, 15 — independent of dispatch |
| 18 Twelve Macro Shapes | 17 | 19 Conditions/NLX (independent) |
| 19 Conditions, NLX, Restarts | 11 (GC), 18 | 20 Collections (different subsystem) |
| 20 Iteration + Collections | 19 | — |

The clear parallelism windows are:
- **02 ↔ 03 ↔ 05** — the LID/manifest parser is independent of the body grammar.
- **11 ↔ 17** — once the GC is up and macros only need fragments, a macro-track and a class/dispatch-track can advance in parallel from Sprint 12 through Sprint 18.
- **15 ↔ 17/18** — sealing and macro work touch disjoint crates.

With one developer the dependency chain is essentially linear with two
short branch-and-merge points around macros (17–18) and conditions
(19). With two developers the project completes Sprints 01–20 in
roughly the time one developer takes for Sprints 01–14.

---

*This sprint plan is committed against PLAN.md and MANIFESTO.md.
Sprint retrospectives may revise sprints 17+ — sprints 01–16 are
intentionally locked.*
