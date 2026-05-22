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

### Sprint 27 — FFI Phase A: data fork + `nod-winapi` crate + Binding DLL provenance + `define c-function` parser — landed
Opens the 10-sprint FFI trajectory that ends at the Dylan-side IDE shell. **Data plumbing only — no API calls execute yet.** Three deliverables:

**A. Vendored Windows API metadata + `nod-winapi` crate.** Forked `E:\windows_api\windows_api.db` (29 MB SQLite, schema v5 — `kind ∈ {primitive, reference, pointer, enum, struct, union, interface, delegate, apis-container, type}`) into the workspace at `data/windows_api.db`. New crate `src/nod-winapi/` with `build.rs` projecting primitive-typed function signatures into a compact `WinApiIndex` struct, `postcard`-serialising, and `zstd`-19 compressing into `$OUT_DIR/winapi_data.bin.zst`. `lib.rs` includes the blob via `include_bytes!(env!("WINAPI_DATA_BIN"))`, decompresses + parses on first access through a `LazyLock`-wrapped `HashMap` index. Projected subset: **13,080 functions across 336 DLLs** (kernel32 contributes ~1165 — every primitive-typed function from `Beep` to `WaitForSingleObject`). Embedded blob: **205,118 bytes** — 6.7% of the Sprint 27 3 MB budget. The schema's `reference` kind (BOOL, HRESULT, HANDLE, DWORD, …) carries no `target_type_id`, so we resolve well-known Windows typedefs by NAME against a static table in `build.rs`. Constants table is hand-curated for now (`MB_OK`, `INVALID_HANDLE_VALUE`, …) — the upstream DB doesn't model constants yet.

**B. `Binding` struct + `BindingId` table in `nod-namespace::graph`.** The Sprint 04 `BindingId(u32)` was scaffolding; this sprint populates it for the first time. New `Binding { id, name, kind: BindingKind::CFunction, dll: Option<String> }` and `Graph::record_c_function_binding(module, name, dll) -> BindingId`. Dylan-to-Dylan bindings still live in the flat sema tables — they'll migrate in a future namespace-consolidation sprint. For Sprint 27 the `Binding` table is the single source of truth for c-function DLL provenance.

**C. `define c-function` parser surface + sema validation.** Parser arm dispatches `define c-function NAME (PARAMS) => (RET); library: "STR"; [c-name: "STR";] end;` at `nod-reader/src/parser.rs:1722` (sibling to `define function`). New AST variant `Item::DefineCFunction { name, params, return_, c_name, library, span }`. Sema (`nod-sema/src/lower.rs`) lowers each declaration into a `CFunctionBinding` carried on `LoweredModule::c_functions`, probes the embedded `nod-winapi` index for the (DLL, c-name) pair, sets `resolved_in_db: bool`, and surfaces a non-fatal `LoweringWarning::CFunctionNotInDb` for unresolved symbols (user might target a custom DLL). **Crucially: any call site that invokes a c-function name errors with `Sprint 28` deferral text** — the AST scan in `scan_module_for_c_function_calls` walks `Expr::Call { callee: Ident(name) }` against the c-function name set. This locks in the deferral. Headline test (`tests/nod-tests/tests/c_function_parse.rs::c_function_call_site_errors_in_sprint27`) parses + sema-lowers `define c-function Beep ... end; define function call-beep() Beep(440, 1000); end;` and asserts the diagnostic exists.

**Plus:** 14 new c-type seed classes (`<c-bool>`, `<c-dword>`, `<c-int>`, `<c-uint>`, `<c-short>`, `<c-ushort>`, `<c-long>`, `<c-ulong>`, `<c-word>`, `<c-byte>`, `<c-pointer>`, `<c-handle>`, `<c-string>`, `<c-wide-string>`) in `nod-runtime/src/c_types.rs` — registered via the same `ensure_*_registered` pattern as Sprint 19 conditions and Sprint 22 tables. No marshaling behavior yet; the classes exist so sema can resolve their names without erroring. Sprint 28 will give them real ABI behavior.

Test count moves from 441 / 0 / 5 to **455 / 0 / 5** under `newgc-backend` default (+7 from `tests/nod-winapi/tests/lookup.rs`, +7 from `tests/nod-tests/tests/c_function_parse.rs`); semispace escape hatch from 438 / 0 / 5 to 452 / 0 / 5. Clippy `--all-targets -- -D warnings` clean (only the deliberate build-script `cargo:warning=...` lines, which is intended diagnostic output). 5x flake check clean.

Deviation from the brief: the upstream `windows_api.db` schema doesn't carry a `constants` table; Sprint 27 ships with a hand-curated list of ~10 well-known constants (`MB_OK`, etc.) to keep the Phase A smoke test honest. Sprint 28 (or a separate DB-extension task) can widen this.

### Sprint 28 — FFI Phase B: per-module API stub table + first end-to-end `Beep(440, 50)` — landed
Headline acceptance: `Beep(440, 50)` runs through `eval_expr_with_items_to_string`, produces an audible 50ms beep (when an audio device is present — returns `#t` regardless), and the test passes.

**A. `<c-ffi-error>` condition + WinFFI types (Phase A).** New `nod-runtime/src/winffi.rs` (~900 lines including the per-arity trampolines) carries `ApiStubEntry { dll_name_ptr, dll_name_len, symbol_name_ptr, symbol_name_len, fn_ptr: AtomicPtr<u8>, signature: ApiCallSignature }`, `ApiStubTable { entries: &'static [ApiStubEntry] }`, `CArgKind` / `CReturnKind`, plus `<c-ffi-error>` as a subclass of `<error>` with `dll-name`, `symbol-name`, `os-error-code`, `message` slots.

**B. Win64 trampolines for arity 0..=8 (Phase B).** Nine `#[unsafe(no_mangle)] pub unsafe extern "C-unwind" fn nod_winffi_call_N(entry: u64, a0: u64, …) -> u64`. Each loads the resolved fn-ptr from the entry (Acquire), unboxes each arg per the recorded signature, transmutes to `extern "system" fn(…)` (Win64 ABI: RCX/RDX/R8/R9 + stack slots beyond shadow space), invokes, reboxes the return as a Dylan Word.

**C. Eager LoadLibrary + GetProcAddress (Phase C).** `resolve_symbol(dll, symbol)` caches HMODULEs in a process-wide `Mutex<HashMap<String, isize>>`. `resolve_into_entry(entry_ptr, dll, symbol)` populates one entry, bumps WinFFI stats. **Deviation from the brief**: we use `windows-sys`'s raw `LoadLibraryA` / `GetProcAddress` instead of `libloading`. `nod-runtime` already depends on `windows-sys` for `Win32_System_Memory`; adding `Win32_System_LibraryLoader` is a one-feature bump rather than a whole new dependency.

**D. Lowering + codegen (Phase D).** `nod-sema/src/lower.rs` gets a Phase 3b pre-pass that walks `Item::DefineCFunction` declarations, builds the marshaling signature from the `<c-…>` ident annotations, deduplicates `(dll, symbol)` pairs, and allocates a single per-module `ApiStubTable` in the static area. The per-call lowering (in `lower_call`) emits `Computation::DirectCall { callee: "nod_winffi_call_N" }` against the synthetic trampoline name; the first arg is a `ConstValue::WordBits` carrying the raw static-area pointer to the entry. Codegen (`nod-llvm/src/codegen.rs`) gets 9 new symbol constants + a 9-row entry in `SPRINT_20B_PRIMITIVES`; the JIT layer (`nod-llvm/src/jit.rs`) binds them to the runtime trampoline addresses via `LLVMAddGlobalMapping`.

Module init: `nod-sema::initialize_module_winffi` walks `LoweredModule::c_function_stub_table` after the JIT engine finalises and calls `resolve_into_entry` for each spec; failures surface as `EvalError::WinFfiInit { class_name: "<c-ffi-error>", dll, symbol }` so tests can pattern-match without parsing a rendered message.

**E. Acceptance tests (Phase E).** `tests/nod-tests/tests/c_function_call.rs` (Windows-only, all `#[serial]`):
- `headline_beep_call_returns_true` — `Beep(440, 50)` → `"#t"`.
- `get_tick_count_returns_increasing_value` — `GetTickCount` + `Sleep` + `GetTickCount`, asserts delta ≥ 0.
- `get_current_process_id_returns_integer` — PID > 0, fits in u32.
- `sleep_zero_returns_without_crashing` — void-return `Sleep(0)` surfaces as `"#()"`.
- `get_current_process_returns_handle` — pseudo-handle (-1) returned, asserts non-zero.
- `api_stub_table_deduplicates_call_sites` — two call sites of `GetTickCount` → `winffi_stats().entries == 1`.
- `unknown_dll_signals_c_ffi_error` — `block`-free expectation via `EvalError::WinFfiInit { class_name: "<c-ffi-error>", dll: "nosuchmodule_sprint28.dll", … }`.
- `unknown_symbol_signals_c_ffi_error` — same shape, `kernel32.dll` + bogus symbol name.

Plus a rewritten `c_function_call_site_lowers_in_sprint28` test in `c_function_parse.rs` (was the Sprint 27 deferral test), plus `c_function_with_unsupported_type_still_defers` for `<c-string>` (Sprint 30 territory).

Test count: **455 → 464 / 0 / 5** under `newgc-backend` default (+9 tests). Semispace escape hatch: **452 → 461 / 0 / 5**. Clippy `--all-targets -- -D warnings` clean. 5x flake-check clean.

Sprint 28 scope is integer/pointer args/returns up to arity 8. Strings (Sprint 30), structs (34), callbacks (33), COM (35), and variadics remain deferred. Per-call `GetLastError` is available manually; auto-raise on Win32 failure (the `set-last-error:` plumbing) waits for Sprint 30+.

Deviation from the brief's wrapper API: the `eval_expr_with_items_to_string` wrap requires a blank line between `Module:` and the first item because `scan_preamble` greedily consumes lines with continuation indents (an indented `(args)` line on a `define c-function` would otherwise get eaten). Documented inline.

### Sprint 29 — Win32 constants generator (`$MB-OK`, `$WM-PAINT`, …) — landed
Replaces magic-number FFI call sites with idiomatic named constants. `MessageBoxW(NULL, "hi", "title", $MB-OK)` and `PostMessageW(hwnd, $WM-CLOSE, 0, 0)` now resolve at lowering time without a single function-ref hop.

**A. Database investigation (Phase A).** Confirmed schema v5 of `windows_api.db` carries 7,773 `enum`-kind type rows (`MESSAGEBOX_STYLE`, `WIN32_ERROR`, `SHOW_WINDOW_CMD`, …) but **NOT** their member values — no `enum_members` table, no `is_const=1` rows in `types`, no rows that reference the enum type via `target_type_id`. The upstream WinMD importer didn't project member integers into the SQLite shape. Falling back to a hand-curated source of truth: `data/win32_constants.txt`, 300 entries covering the most-used Win32 constants (MessageBox flags, window messages, window styles, ShowWindow commands, GetWindowLong offsets, standard cursors/icons, system metrics, GDI ROP codes, process/file access rights, VirtualAlloc flags, standard handles, WaitFor* returns, HRESULT codes, Win32 error codes).

**B. Build-time extraction (Phase B).** `src/nod-winapi/build.rs::project_constants` now reads `data/win32_constants.txt` (parsed by a stdlib-only INI-style parser — no new dep) and emits 300 `ConstantInfo` rows into the embedded blob. Each entry carries name, i64 value (parsed from decimal or `0x…` hex, sign-extended), and optional source-DLL annotation. Duplicate names allowed only if values agree (e.g., `MB_ICONERROR == MB_ICONSTOP == 0x10` — three Win32 spellings for the same flag value). Build-time `cargo:warning` reports the actual count: `nod-winapi: 13080 functions, 300 constants, 336 dlls`.

**C. `nod_winapi::iter_constants` (Phase C).** New public API surface for walking the embedded constant set. `find_constant` (Sprint 27) stays as the random-access lookup; `iter_constants` covers the generator and the regression test that locks in the 50-constant floor.

**D. Generator binary (Phase D).** `src/nod-winapi/src/bin/generate_constants.rs` reads `data/win32_constants.txt` (preserving category headers so the generated Dylan file stays grouped) and emits `src/nod-dylan/dylan-sources/win32-constants.dylan` — 300 `define constant $NAME = value;` lines, with `_` → `-` transformation and `$` prefix per Dylan convention. Values < 256 emit as decimal, larger values as `#xHEX` (Dylan hex literal), negatives as signed decimal. Run via `cargo run --quiet -p nod-winapi --bin generate_constants`; idempotent against unchanged source.

**E. Stdlib loader picks up `win32-constants.dylan` (Phase E).** `nod-sema/src/stdlib.rs` refactored to a multi-file `STDLIB_FILES` list. The loader parses each file, merges items into a single module, then strips `Item::DefineConstant { value: Expr::Integer(_, n) }` entries into a process-global `STDLIB_CONSTANTS: HashMap<String, i128>`. User-code lowering (`Expr::Ident` resolution in `lower.rs`) consults this map BEFORE the function-ref fallback path so `$MB-OK` becomes `ConstValue::Integer(0)` — not a `<function>` Word. The 300 constants never become functions in the stdlib JIT engine; they're pure compile-time values.

**F. Sprint 28 headline test wired through a constant (Phase F).** `tests/nod-tests/tests/c_function_call.rs::flash_window_with_named_constants` evaluates `$WM-NULL + GetTickCount()` and asserts the sum is the (positive) tick count, proving `$WM-NULL` resolves to 0 in the same expression context as a real Win32 call.

**G. Acceptance tests (Phase G).** New `tests/nod-tests/tests/win32_constants.rs` with 9 tests:
- `mb_ok_resolves_to_zero` — small zero flag round-trips.
- `wm_paint_resolves_to_15` — `0x000F` hex source surfaces as decimal `"15"`.
- `mb_iconerror_resolves_to_16` — `0x10` round-trips.
- `ws_overlappedwindow_is_complex_mask` — `0x00CF0000 == 13565952`, the union of OVERLAPPED|CAPTION|SYSMENU|THICKFRAME|MINIMIZEBOX|MAXIMIZEBOX.
- `gwl_style_resolves_to_minus_16` — negative offset round-trips through the curated `-16` spelling.
- `unknown_constant_errors_at_lower` — `$NOT-A-REAL-CONSTANT` produces `EvalError::Lower` with an `undefined ident` diagnostic.
- `constant_usable_in_arithmetic` — `$MB-OK + $MB-ICONERROR == 16`, proving both names resolve as integers in the same expression.
- `stdlib_constants_count_at_least_50` — locks the lower bound on coverage by inspecting `nod_sema::stdlib::constants_table()`.
- `winapi_iter_constants_count_at_least_50` — same lower bound at the `nod-winapi` layer.

Test count: **464 → 475 / 0 / 5** under `newgc-backend` default (+10 tests, including the new `flash_window_with_named_constants` in `c_function_call.rs` and 9 acceptance tests in `win32_constants.rs`). Semispace escape hatch: **461 → 472 / 0 / 5**. Clippy `--all-targets -- -D warnings` clean. 5x flake check clean.

Deviation from the brief: the brief considered a TOML-formatted curated file as one option for the hand-curated set; we went with a simpler `key = value` line-based format (`data/win32_constants.txt`) so `build.rs` could parse it with no new dep. The generator binary (Rust, not Python — keeps the toolchain story to "just `cargo`") preserves category headers from the source file so the emitted Dylan stays grouped by feature area.

Closes the Sprint 27 deferred entry about the upstream constants table; opens a new deferred entry about reviving enum-member type-checking (Sprint 30+) and string constants (`IID_*`, `CLSID_*` — Sprint 30+ territory).

### Sprint 30 — FFI Phase C: `<c-string>` + `<c-wide-string>` + `$NULL` — landed
Empirical headline: `lstrlenW("héllo") → "5"`. 'é' (U+00E9) is two UTF-8 bytes (0xC3 0xA9) but one UTF-16 code unit; only correct UTF-8 → UTF-16 transcoding produces 5. A byte-copy implementation would return 6, and any test that just checks ASCII strings would never spot the bug. The non-ASCII assertion *is* the proof that string marshaling works.

**A. `TempBuf` infrastructure + per-call buffer lifetimes (Phase A).** New `enum TempBuf { Narrow(Vec<u8>), Wide(Vec<u16>) }` in `nod-runtime/src/winffi.rs`. Each arity-N trampoline (`nod_winffi_call_1` .. `nod_winffi_call_8`) now allocates `let mut temps: Vec<TempBuf> = Vec::new();` on its stack frame before the unbox phase; `unbox_arg(w, kind, &mut temps)` pushes one `TempBuf` per string arg, returns the buffer's `as_ptr() as u64`, and the `Vec` drops at end of scope — *after* the C call returns. No leaks; lifetime is exactly the call.

**B. `CArgKind::NarrowString` + `CArgKind::WideString` (Phase A continued).** Two new discriminants on `#[repr(u8)] enum CArgKind` (values 12 and 13), plus `CReturnKind::NarrowString` (8) and `CReturnKind::WideString` (9). `signature_from_names` recognises `"<c-string>"` and `"<c-wide-string>"` for both arg and return positions. The receive-side path (Win32 API returns an LPCSTR/LPCWSTR — e.g. `GetCommandLineW`) scans the returned pointer to its null terminator (capped at 1MiB) and copies into a fresh Dylan `<byte-string>` via `intern_string_literal`.

**C. Wide-string transcoding (Phase A continued).** Uses `s.encode_utf16().collect::<Vec<u16>>()` + push(0) — std-lib only, no new transitive deps. The narrow path is intentionally pass-through bytes (UTF-8 → LPSTR with terminator) — this matches CP_ACP on the ASCII subset and avoids pulling in `WideCharToMultiByte` for the headline test. CP_ACP conversion for non-ASCII narrow strings is a deferred polish item.

**D. `$NULL` constant + null-pointer marshaling (Phase B).** New "Pointer / handle sentinels" category in `data/win32_constants.txt`: `NULL = 0`. The Sprint 29 generator picks this up, so `src/nod-dylan/dylan-sources/win32-constants.dylan` now exposes `define constant $NULL = 0;`. The marshaling change is one branch in `marshal_narrow_string` / `marshal_wide_string` / `unbox_arg`'s `Pointer|Handle` arm: a Dylan fixnum 0 → C `null` pointer. Callers can write `MessageBoxW($NULL, "hi", "title", $MB-OK)` idiomatically.

**E. Stats accounting (Phase A continued).** `WinFfiStats` gains a `tempbufs_allocated_lifetime: usize` counter, bumped by `marshal_narrow_string` / `marshal_wide_string`. Useful for the per-call allocation regression test and for any future cost-watch. Reset alongside the other counters by `_reset_winffi_stats_for_tests`.

**F. Sema lift of the Sprint 28 deferral (Phase C).** The Sprint 28-era `c_function_with_unsupported_type_still_defers` test is replaced by `c_function_with_string_arg_lowers_in_sprint30` in `tests/nod-tests/tests/c_function_parse.rs`: lowering a `MessageBoxA(<c-handle>, <c-string>, <c-string>, <c-dword>) => (<c-int>)` declaration now succeeds and produces an `ApiCallSignature` with arg kinds `[Handle, NarrowString, NarrowString, UInt32]` and return kind `Int32`.

**G. Acceptance tests in a new file (Phase C+).** `tests/nod-tests/tests/winffi_strings.rs` — 9 value-asserting tests + 1 ignored-by-default interactive demo:
- `lstrlen_w_returns_correct_wide_length` — `lstrlenW("hello world") → "11"`.
- `lstrlen_w_handles_unicode_correctly` — **`lstrlenW("héllo") → "5"`** (the empirical UTF-16 proof).
- `lstrlen_a_returns_correct_narrow_length` — `lstrlenA("hello world") → "11"`.
- `lstrlen_a_handles_utf8_as_bytes` — `lstrlenA("café") → "5"` (5 UTF-8 bytes; proves narrow path doesn't transcode).
- `lstrlen_w_empty_string` — `lstrlenW("") → "0"`.
- `null_constant_evaluates_to_zero` — `$NULL → "0"`.
- `null_pointer_via_dollar_null` — `lstrlenW($NULL) → "0"` (NULL pointer reaches the API per MSDN's documented contract).
- `mixed_args_string_and_int` — `lstrcmpW("abc", "abc") → "0"` (two wide-string args, separate temp buffers).
- `tempbuf_allocation_count_tracks_string_args` — two `lstrlenW` calls bump `tempbufs_allocated_lifetime` from 0 to exactly 2.
- `message_box_w_pops_real_dialog` — **`#[ignore]`-gated** opt-in developer demo; run manually via `cargo test --test winffi_strings -- --ignored`. NOT invoked by routine `cargo test`.

The brief originally suggested `IsBadStringPtrW($NULL, 10)` for the NULL-marshaling test; we substituted `lstrlenW($NULL)` because IsBadStringPtrW is deprecated and behaves unreliably on modern Windows, while `lstrlenW`'s NULL contract is documented and stable. Same shape of proof — fixnum 0 must reach the API as a real null pointer or the API would crash / return garbage instead of 0.

Test count: **475 → 484 / 0 / 6** under `newgc-backend` default (+9 passing string tests; +1 ignored MessageBoxW). Clippy `--all-targets -- -D warnings` clean. 5x flake check clean. The semispace backend is no longer routinely exercised — newgc default is the only verification path now.

**Out of scope for Sprint 30 (deferred):**
- C → Dylan string returns at the headline level (basic LPCSTR/LPCWSTR scan-and-copy is wired up via `CReturnKind::NarrowString` / `WideString`, but the out-buffer pattern — caller-allocated buffer + length, e.g. `GetWindowTextW(hwnd, buf, len)` — needs a separate sprint).
- CP_ACP encoding conversion for `<c-string>` (currently pass-through UTF-8 bytes).
- True wide-character Dylan-side storage (`<unicode-string>` Dylan class for UTF-16 payloads — currently we transcode at the boundary).
- BSTR / OLEStr handling (Sprint 35 — COM territory).

Sprint 30's "Dylan-side IDE bring-up" slot from the prior plan shifts forward; the FFI Phase C string-marshaling work moves into the Sprint 30 slot, and the IDE work tracks behind the Sprint 31 (`common-dylan` port) entry as Sprint 32+. Renumbering of downstream slots is deferred to the next sprint-plan review.

### Sprint 31 — JIT-time Win32 API materialization (bare-name calls) — landed

**Goal:** `GetTickCount64()` returns the system uptime via `eval_expr_to_string` **without any `define c-function` declaration above it**. Sprint 28 wired the table; Sprint 31 makes the table populate itself from the embedded `nod-winapi` index when Dylan source references a bare Win32 name.

**A. Sema lookup hook (`nod-sema/src/lower.rs`).** After the existing Phase 3b walk that builds the per-module stub table from explicit `define c-function` declarations, a new pre-Phase-4 pass walks the AST for `Expr::Call { callee: Expr::Ident(name), ... }` and collects every name that (a) isn't user-declared, (b) isn't a Dylan top-level function, generic, or class, and (c) passes a shape filter for Win32 exports (`looks_like_win32_export`: at least one uppercase letter, all ASCII alphanumerics, ≥ 3 chars). For each such candidate `try_jit_materialize_winapi(name)` consults `nod_winapi::functions()`:

1. **A/W default to W.** Bare `MessageBox` is rewritten to `MessageBoxW` first; the literal name is the fallback. Bare `MessageBoxA` keeps the explicit A suffix.
2. **Cross-DLL priority.** When a name resolves in multiple DLLs, `WINAPI_DLL_PRIORITY` breaks the tie: `kernel32.dll` > `user32.dll` > `gdi32.dll` > `advapi32.dll` > `shell32.dll` > `comctl32.dll` > alphabetical fallback.
3. **Signature derivation.** `build_signature_from_function_info` walks `FunctionInfo::params` + `return_type` (`nod_winapi::TypeRef`) and maps each to `nod_runtime::CArgKind` / `CReturnKind`. Unmappable shapes (`Void` as a param, `Pointer { pointee: Function }`, struct-by-value — none of which actually reach the embedded blob because `build.rs` filters them — and arity > 8) return `Err(reason)`; the materialization declines and the caller surfaces a "Win32 function found but signature uses unsupported types" diagnostic.

**B. `BindingSource` enum (`nod-sema/src/lower.rs`).** New `BindingSource::{UserCFunction, JitMaterialized}` on `CFunctionBinding`. User declarations always carry `UserCFunction`; the bare-name fallback decline-on-collision rule guarantees JIT materialization never overwrites an explicit binding. Introspection (`introspect_bindings`) exposes the field directly.

**C. Stub-table integration.** Synthesized bindings feed into the existing `c_function_specs` / `spec_dedupe` machinery — two bare references to `GetTickCount64` in the same module share one slot, and the resolver / trampoline path is unchanged from Sprint 28. No new IR variants, no new runtime infrastructure, no marshaling changes. Sprint 31 is sema-only.

**D. Diagnostics.** Bare-name calls whose Win32 entry exists in the index BUT whose signature uses unsupported categories now emit `LoweringError::Unsupported { message: "Win32 function `X` was found in the embedded windows_api.db index, but its signature uses unsupported types (...); declare an explicit `define c-function X ... library: ...; end;` with a shim signature, or wait for Sprint 33 (callbacks) / Sprint 34 (structs)." }`. Bare names that aren't in the index at all fall through to the existing `Codegen(UnknownCallee)` path — same behavior as before Sprint 31.

**E. Stats.** `WinFfiStats::materialized_lifetime` (process-global counter, bumped from `nod_runtime::winffi_record_materialized()` once per synthesized binding) surfaces materialization activity for tests and the future `winffi-stats` diagnostic command.

**F. Tests (`tests/nod-tests/tests/winffi_materialize.rs`).** 10 acceptance tests + 1 ignored marker, all `#[serial]` (`#[cfg(windows)]`):
- `bare_GetTickCount64_resolves_to_kernel32` — **headline**: bare-name uptime call, asserts > 1000 ms.
- `bare_GetCurrentProcessId_resolves_correctly` — positive u32.
- `bare_Sleep_resolves_to_void_returning` — void-return materialization works.
- `bare_lstrlenW_resolves_with_string_marshaling` — bare `lstrlenW("héllo")` → "5" (UTF-16 transcoding through a synthesized binding).
- `bare_MessageBox_resolves_to_W_variant` — introspection: bare `MessageBox` materializes as `MessageBoxW` from `user32.dll`, no dialog popped.
- `bare_MessageBoxA_resolves_explicitly` — introspection: explicit A suffix kept.
- `user_define_c_function_overrides_materialization` — user-declared `GetTickCount` carries `BindingSource::UserCFunction`, exactly one binding (no duplicate JIT-materialized entry).
- `unsupported_signature_declines_materialization` — `CreateProcessW` (10 params, > 8 arity cap) either errors with "unsupported types" or falls through to `UnknownCallee`; never silently succeeds.
- `stats_show_materialization_count` — two distinct bare calls bump `materialized_lifetime` by 2.
- `duplicate_bare_calls_share_one_materialization` — two calls to the same bare name share one slot (one materialization counter bump).
- `ambiguous_name_picks_kernel32_first` — `#[ignore]` marker: no genuine cross-DLL collisions in the current embedded blob; priority order covered by the pure-function unit test in `nod-sema` below.

Plus 7 unit tests in `nod-sema::lower::sprint31_tests`: `winapi_dll_priority_orders_kernel_first`, `looks_like_win32_export_filters_correctly`, `jit_materialize_GetTickCount64_yields_kernel32_no_args`, `jit_materialize_bare_MessageBox_picks_W`, `jit_materialize_unknown_name_returns_not_found`, `jit_materialize_lstrlenW_succeeds`, `jit_materialize_EnumWindows_outcome`.

**Headline acceptance:** `eval_expr_to_string("GetTickCount64()")` returns a positive integer > 1000 with no `define c-function` declaration. The bare-name `lstrlenW("héllo")` returns "5" through the same path Sprint 30 proved out for explicit declarations.

**Gate results:** 501 / 0 / 7 under newgc default (484 → 501; +17 new tests, +1 ignored marker). 5x sequential flake clean. `cargo clippy --workspace --all-targets -- -D warnings` clean.

**Out of scope (deferred):**
- Process-global materialized-binding cache (each eval-module re-materializes; not yet measurably hot).
- Better ambiguity fix-it hints (currently the message just names the colliding DLLs; no auto-suggest).
- Materialize-by-pattern (`Get*`, `*A` / `*W` family expansions) for IDE auto-completion.
- A/W resolution that walks back to a single canonical Dylan-name (currently `MessageBox` materializes to `MessageBoxW` and the synthesized binding's `dylan_name` stays `MessageBox` — the user code sees the bare name they wrote).

### Sprint 32 — Callbacks: closure → C function pointer — landed

**Goal:** `EnumWindows(callback, $NULL)` enumerates every top-level window on the test machine, invoking a Dylan closure (`method (hwnd, lp) ... #t end`) once per window, with the closure incrementing a captured-variable counter that survives across calls. The keystone IDE-essential FFI capability — `WNDPROC` for window procedures, `WNDENUMPROC` for `EnumWindows`. Sprint 28 wired Win32 → Dylan calls; Sprint 32 closes the reverse direction, Dylan-as-callback → Win32.

**A. Pre-allocated trampoline pool per signature class (`nod-runtime/src/callbacks.rs`).** Each Win32 callback signature has a fixed pool of 32 slot trampolines, one `extern "system" fn` per slot. A macro generates `wndproc_slot_0` … `wndproc_slot_31` (and the `wndenumproc_slot_N` family); each slot trampoline knows its slot ID at compile time and forwards to a per-signature dispatcher that looks up the registered closure Word for that slot, marshals C args → Dylan `Word`s, calls the closure via Sprint 24's `nod_funcall_N`, and rebox the return.

Sprint 32 ships two signatures:
- `Wndproc`: `extern "system" fn(HWND, UINT, WPARAM, LPARAM) -> LRESULT` (the WNDPROC contract for `RegisterClass(W)`).
- `Wndenumproc`: `extern "system" fn(HWND, LPARAM) -> BOOL` (the WNDENUMPROC contract for `EnumWindows` and family).

The fixed pool of 32 slots per signature is the Sprint 32 cap; tunable later via build-time const. The slot trampolines occupy 64 `extern "system"` symbols in `nod-runtime` (32 × 2 signatures), each with `#[unsafe(no_mangle)]` so the linker pins their addresses.

**B. Registry per signature (`OnceLock<Mutex<Registry>>`).** Each `Registry` holds `Box<[UnsafeCell<Word>; 32]>` — one stable closure-cell address per slot — plus an `occupied: [bool; 32]` bitmap. The slab address is stable for the process lifetime; the cell pointers are valid GC root targets.

**C. GC root discipline — per-thread.** Sprint 11c's `register_root` is thread-local (each mutator's `ROOT_STACK` is its own `RefCell<Vec<*const Word>>`). The callback registry's cells must be in EVERY mutator thread's root stack — otherwise a collection on a thread that didn't install them misses the registered closures. `install_gc_roots_for_this_thread(sig, registry)` registers all 32 cells on first touch from each thread, idempotent-guarded via `thread_local! { static WNDPROC_ROOTS_INSTALLED: Cell<bool>; }`. Both `register_callback` and the dispatchers call it on entry — covering the mutator and the OS-callback thread.

**D. JIT-callable externs.** `nod_register_wndproc(closure_word) -> Word` and `nod_register_wndenumproc(closure_word) -> Word`. Each is `unsafe extern "C-unwind"` to match the rest of the runtime's JIT ABI; the return is the slot's trampoline address packed into a fixnum-tagged `<c-pointer>` Word (the Sprint 28+ ABI for raw addresses). On pool exhaustion, surfaces a `<c-ffi-error>` via `nod_signal` (diverges).

**E. Lowering wiring (`nod-sema/src/lower.rs::LOWER_PRIMITIVE_TABLE`).** Two new primitives:
- `%register-wndproc(closure)` → `nod_register_wndproc`, arity 1, returns `<top>` (the `<c-pointer>` Word).
- `%register-wndenumproc(closure)` → `nod_register_wndenumproc`, arity 1.

**F. Stdlib wrappers (`nod-dylan/dylan-sources/stdlib.dylan`).** Two thin functions:

```dylan
define function as-wndproc-callback (closure) => (ptr)
  %register-wndproc(closure)
end function;

define function as-wndenumproc-callback (closure) => (ptr)
  %register-wndenumproc(closure)
end function;
```

A unified `as-c-callback(closure, signature-symbol)` form is deferred until Dylan-side `select` lowers cleanly (the current macro layer doesn't reach `select` at stdlib-load time).

**G. Codegen + JIT symbol bindings (`nod-llvm/src/codegen.rs`, `jit.rs`).** Two new symbol constants (`NOD_REGISTER_WNDPROC_SYMBOL`, `NOD_REGISTER_WNDENUMPROC_SYMBOL`) added to the `SPRINT_20B_PRIMITIVES` table; matching `LLVMAddGlobalMapping` entries in `jit.rs::add_module`. No new IR variants — the call lowers as a plain `DirectCall` against a `%`-prefixed primitive name, the same shape as every other runtime extern.

**H. Tests.** Six integration tests in `tests/nod-tests/tests/winffi_callbacks.rs` (`#![cfg(windows)]`, all `#[serial]`):

- **`enum_windows_invokes_callback_for_each_top_level_window`** — **the Sprint 32 headline**. `EnumWindows(callback, $NULL)` invokes a Dylan closure that increments a captured `count := count + 1` once per top-level desktop window; counter ends up positive (asserted `> 0` and `< 100_000` for sanity).
- `register_wndenumproc_returns_non_null_pointer` — non-zero trampoline address.
- `register_wndproc_returns_non_null_pointer` — same for WNDPROC (arity 4 closure body).
- `two_callbacks_get_distinct_addresses` — two registrations land in distinct slots → distinct trampoline addresses → Dylan-side `a = b` is `#f`.
- `callback_pool_full_signals_error` — 32 registrations succeed via the Rust API, the 33rd returns `Err(PoolFull)`; pool reset clears state for subsequent tests.
- `closure_survives_gc_pressure` — register a closure, force two minor GCs, invoke the trampoline directly via an `extern "system"` fn-pointer transmute; the closure body still runs.

Plus five in-module unit tests in `nod-runtime/src/callbacks.rs::tests` (`#[serial]`): synthetic dispatch via direct trampoline call (one each for WNDPROC and WNDENUMPROC), distinct-slot-addresses Rust check, pool-full Rust check, rebox-helper truth-table coverage.

**Headline acceptance:** the EnumWindows test returned a count of top-level windows on the test machine — a positive integer reflecting the actual Windows shell state at test time.

**Gate results:** 512 / 0 / 7 under newgc default (501 → 512; +11 new tests). 5x sequential flake clean. `cargo clippy --workspace --all-targets -- -D warnings` clean.

**Out of scope (deferred — see DEFERRED.md):**

- **Callback unregistration.** Sprint 32 registrations are leak-by-design: once a closure is registered, its slot stays occupied for the process lifetime. A future sprint adds `release-c-callback(ptr)` semantics with safe-point coordination so the OS isn't holding a stale trampoline mid-callback.
- **Additional signatures.** `TIMERPROC` (`SetTimer`), `THREADPROC` (`CreateThread`), `DLGPROC` (`DialogBox*`), Win32 hook procs (`SetWindowsHookEx`), CRT `qsort`/`bsearch`, and the various `EnumXxx` family beyond windows.
- **JIT-emitted per-callback trampolines.** Alternative to the fixed pool — each registration emits a fresh trampoline via the JIT, eliminating the pool-size cap. Memory cost per callback grows; freed-trampoline reclamation interacts with MCJIT engine lifetime. Sprint 32 ships the simpler fixed-pool architecture; a JIT-emitted variant becomes valuable when 32-slot saturation actually bites.
- **Cross-thread callback semantics.** If the OS invokes our trampoline on a thread different from the mutator that registered the closure, Sprint 32's per-thread root-installation handles GC root reachability, but a future cross-thread Sprint will need to lock the closure's environment frames (for closures with captured state in the moveable heap touched concurrently with mutator allocations).
- **Unified `as-c-callback(closure, sig-symbol)` surface.** Pending Dylan-side `select` lowering for the symbol-dispatched form.
- **`extern "system"` panic-on-unwind discipline.** Sprint 32 has the same UB exposure as Sprint 28's `nod_winffi_call_N` — a Dylan signal that crosses the OS callback boundary aborts (on Windows MSVC, panic crossing `extern "system"` is structurally unwound with a `STATUS_STACK_BUFFER_OVERRUN` abort). The mitigation is the same as Sprint 28's: trust callers to handle conditions before the closure returns. Tightening this (catching unwinds inside the dispatcher and returning a default value with a side-channel error) is a Sprint 33+ task.

### Sprint 34 — Structs: `<c-struct>` family for IDE-essential Win32 shapes — landed

**Goal:** `let pt = make(<point>); GetCursorPos(pt); point-x(pt) + point-y(pt)` runs through `eval_expr_to_string` and returns a real screen-cursor coordinate sum — empirical proof that struct allocation, address-of marshaling, field setters (by the C function via pointer), and field getters (Dylan reading the buffer back) all work end-to-end. The keystone IDE-essential FFI capability: `GetMessageW(LPMSG, …)`, `BeginPaint`, `GetCursorPos`, `GetClientRect`, `SetRect`, `GetSystemTime`, `GetLocalTime`, and every other Win32 API that takes a pointer to a caller-allocated struct.

**A. `<c-struct>` infrastructure (`nod-runtime/src/structs.rs`).** A new module registers a `<c-struct>` parent class at process boot via `ensure_structs_registered`, then six concrete subclasses (`<point>`, `<rect>`, `<size>`, `<filetime>`, `<systemtime>`, `<msg>`). Each concrete class:

- has `instance_size = 8 (wrapper) + struct_byte_size` matching the Win64 `sizeof` (POINT=8, RECT=16, SIZE=8, FILETIME=8, SYSTEMTIME=16, MSG=48);
- carries `is_byte_payload: true` on the `ClassMetadata` so the GC's `DylanLayout` reports an opaque payload (same pattern as `<byte-string>`);
- has a per-field layout table (`StructFieldInfo { name, offset, kind }`) accessible via `struct_layout_for(class_id)` for diagnostics.

Concrete struct classes bypass `register_simple_user_class` (which fixes instance size at `8 + 8*slot_count` and forces `is_byte_payload = false`) and go through a Sprint 34–local `register_struct` helper that allocates a custom `ClassMetadata` directly in the static area.

**B. Field accessor primitives (`nod-runtime/src/structs.rs`).** Each `nod_struct_get_*` / `nod_struct_set_*` pair takes a struct Word and a fixnum-tagged byte offset; the get returns a tagged fixnum, the set returns the value Word (Dylan setter convention). Sprint 34 wires six widths: i32, i64, u16, u32, u64, pointer. Unaligned reads/writes throughout so packed fields (e.g. `WPARAM` at MSG offset 16) need no extra alignment care.

The primitives' `offset` arg is itself a fixnum-tagged Word (`n << 1`) — JIT-emitted code passes Dylan integer literals which lower to tagged Words; a `decode_offset` helper unpacks the tag. Sprint 34 caught this convention mismatch the hard way (initial implementation treated the raw u64 as the offset, which silently doubled every access; field roundtrips passed because set and get cancelled out, but Win32 calls — which write at the true offsets — surfaced the bug as wrong field positions when read back).

**C. Stdlib field accessors (`nod-dylan/dylan-sources/stdlib.dylan`).** One getter and one setter per field of every seed struct, ~60 functions total, hand-generated. The setter signature follows the Sprint 12 unary-setter calling convention (`slot-getter(obj) := v` → `slot-getter-setter(obj, v)`): `point-x-setter(p, v)` forwards to `%struct-set-i32(v, p, 0)`. Sprint 35+ adds a `define c-struct` Dylan-side parser surface that emits these automatically.

**D. Auto-coerce in marshaling (`nod-runtime/src/winffi.rs::unbox_arg`).** When a `<c-function>` parameter is declared `<c-pointer>` or `<c-handle>` AND the actual Dylan arg is a pointer-tagged `<c-struct>` subclass instance, the marshaler passes `wrapper_ptr + 8` (the byte-payload start) instead of the wrapper address itself. The recognition test is `is_c_struct_instance(w)`, which walks the wrapper's class through `is_subclass(class, <c-struct>)`. The walk is short (Sprint 34 seed structs have a 3-entry CPL: self → `<c-struct>` → `<object>`), and Sprint 34 measured no observable hot-path impact — the `OnceLock::get` for the c-struct class id is non-locking, and `is_subclass` is a linear scan of a 3-entry Vec.

**E. Tests (`tests/nod-tests/tests/winffi_structs.rs`, plus inline `#[cfg(test)]` in `structs.rs`).**

Pure-Dylan field roundtrips (no Win32):
- `point_alloc_zeroes_fields` — `make(<point>)` zero-fills the payload; reading both fields returns `"0"`.
- `point_field_setter_roundtrip` — `point-x(p) := 42; point-y(p) := 99; point-x(p) + point-y(p)` returns `"141"`.
- `rect_all_four_fields` — set all four `<rect>` fields, compute width + height, expect `"270"`.
- `systemtime_u16_field_roundtrip` — `<systemtime>` u16 fields roundtrip through `2026 + 5 + 22 = "2053"`.
- `msg_mixed_width_fields_roundtrip` — `<msg>` exercises pointer, u32, u64, i64, i32 widths in one expression; sum `"15377"`.
- `point_is_subclass_of_c_struct` — `instance?(make(<point>), <c-struct>)` returns `"#t"`.

Rust-side metadata + GC:
- `instance_sizes_match_win64_sizeof` — POINT=16, RECT=24, SIZE=16, FILETIME=16, SYSTEMTIME=24, MSG=56 (including 8-byte wrapper).
- `point_survives_minor_gc` — root-installed `<point>` survives a `collect_minor()` cycle with its field intact.

Win32 headlines:
- **`get_cursor_pos_returns_screen_coords`** — **the Sprint 34 headline**. `GetCursorPos(pt)` writes real cursor x/y; Dylan reads them and the sum lands in a sensible `[−100k, 100k)` range. Run output: `[Sprint34 headline] GetCursorPos x+y = 44`.
- **`get_system_time_returns_current_year`** — `GetSystemTime(st); systemtime-year(st)` returns the current UTC year. Run output: `[Sprint34 headline] GetSystemTime year = 2026`.
- **`set_rect_populates_all_four_fields`** — `SetRect(r, 10, 20, 30, 40)` writes left/top/right/bottom; Dylan packs them as `left + top*10 + right*100 + bottom*1000 = 43210`. Run output: `[Sprint34 headline] SetRect packed sum = 43210`.
- `get_local_time_returns_sensible_month_and_day` — month ∈ [1,12], day ∈ [1,31].

**F. Verification.** 532 / 0 / 7 under `newgc-backend` default (512 → 532; +20 new tests across `winffi_structs.rs` integration suite and `structs.rs` inline unit tests). 5x sequential flake clean. `cargo clippy --workspace --all-targets -- -D warnings` clean.

**Headline acceptance:**
- `GetCursorPos x+y = 44` — actual desktop cursor coordinates rendered through Dylan.
- `GetSystemTime year = 2026` — Win32 wrote `2026` as a u16 at SYSTEMTIME offset 0; Dylan read it back.
- `SetRect packed sum = 43210` — all four `<rect>` fields populated by the Win32 API and read back by Dylan in correct positions.

**One existing fixture rename.** `tests/nod-tests/fixtures/point.dylan` defined a user-class `<point>` for the Sprint 12 distance-squared regression. With `<point>` now a seed struct, the fixture's class collided at lowering time (the `<point>` name resolves to the seed class, and a fresh `define class <point>` triggers `ClassRedefinitionNotSupported`). Renamed the fixture's class to `<user-point>` — purely a name change; no behavioural impact on the regression test.

**Out of scope (deferred — see DEFERRED.md):**

- `define c-struct` Dylan-side parser surface. Sprint 34 seeds six structs in Rust; the user surface for declaring new structs from Dylan source is a Sprint 35+ task.
- **Struct-by-value marshaling.** Win64 ABI rules for ≤8-byte structs (passed in register) and >8-byte structs (passed via hidden pointer) are real but every IDE-essential Win32 API uses pointer parameters (`LPMSG`, `LPRECT`, `LPPOINT`). Defer until a real use case demands it.
- **Nested struct field syntax.** MSG.pt is a POINT; Sprint 34 surfaces `msg-pt-x(m)` / `msg-pt-y(m)` as flat-offset accessors. Dotted-notation `msg.pt.x` access lands with the `define c-struct` parser.
- **Variable-length structs.** `BITMAPINFO`'s `bmiColors[1]` header-trick layout (and similar APIs) requires a different allocation model. Sprint 35+.
- **C → Dylan struct view.** Sprint 34 auto-coerces Dylan-struct → C-pointer one way only. A Win32 API that returns `LPRECT` and the Dylan caller wants to read its fields requires explicit `wrap-as-rect(ptr)` in Sprint 34 (deferred — no IDE-track API in the seed set returns a struct pointer that Dylan needs to read).
- **Per-bucket "is-c-struct" Wrapper flag.** Sprint 34 uses `is_subclass(class, <c-struct>)` for the auto-coerce decision; the CPL walk is 3 entries deep so the cost is negligible. If a future profile shows the test as hot, switching to a dedicated bit on the Wrapper (parallel to Sprint 22's bucket-state byte) is a one-line change.

### Sprint 35 — COM via `windows` crate: DXGI / D3D11 / D2D / DirectWrite infrastructure — landed

**Goal:** an offscreen D2D + DirectWrite text-rendering chain reachable from Dylan source. The Sprint 35 brief originally sketched a hand-rolled C++ shim DLL wrapping COM as plain C; we instead use the official Microsoft [`windows` crate](https://docs.rs/windows) as the COM-aware layer. The shim lives in Rust at `nod-runtime::com_shim`, uses the `windows` crate's typed interfaces for refcount-correct COM, and exposes ~30 `%`-primitive entries through the Sprint 20b primitive-call path. Dylan source builds a 256×256 BGRA8 texture, renders "hello, dylan" with DirectWrite + a red brush, reads back the pixel buffer through a CPU-mapped staging texture, and asserts text glyphs produced non-zero red pixels.

**Architectural shape.**

A. **COM handle registry.** A process-global `Mutex<HashMap<u64, ComObject>>` in `com_shim.rs` owns one typed `windows`-crate interface per Dylan-held handle. Cloning a `windows` COM type bumps refcount (`AddRef`); dropping calls `Release`. The registry's `register(obj) → u64` hands out monotonic counter handles which Dylan treats as opaque `<c-handle>` tokens. `release(handle)` removes the entry, which drops the typed wrapper, which calls `Release`. No manual AddRef/Release in our shims.

B. **Typed accessors per variant.** A `typed_accessor!($name, $variant, $ty)` macro generates `get_dxgi_factory`, `get_d3d11_device`, etc. — each takes a u64 handle, untags the fixnum tag bit, and returns `Option<TypedInterface>` cloned out of the registry. Cloning gives the caller an owned reference that survives `Drop` independently of the registry's entry.

C. **`windows` crate feature flags.** Sprint 35 enables `Win32_Foundation`, `Win32_System_Com`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`, `Win32_Graphics_Direct3D`, `Win32_Graphics_Direct3D11`, `Win32_Graphics_Direct2D`, `Win32_Graphics_Direct2D_Common`, `Win32_Graphics_DirectWrite`, and `Foundation_Numerics` (the last for `Matrix3x2`). Build cost: one-time ~3-minute clean compile of the windows crate; incremental builds are sub-second. The `windows` crate itself adds ~300MB to `target/`.

D. **Float-marshaling deviation.** The brief sketches `<c-float>` / `<c-double>` Dylan args feeding float-aware trampoline variants. Sprint 35 instead routes the whole COM surface through **integer-encoded scalars** — color channels are 0..=255 Dylan integers (the shim divides by 255 to get f32), pixel coordinates are integer Dylan values (the shim casts to f32 in stride). This eliminates the trampoline restructure entirely: every shim signature is `extern "C-unwind" fn(u64, u64, …) -> u64`, lowered through the existing Sprint 28 mechanism without change. The `<c-float>` / `<c-double>` Dylan classes are still registered (Phase A acceptance item), and `CArgKind::Float32` / `CArgKind::Float64` exist in the enum and `from_c_type_name` mapping — sema accepts `define c-function` declarations using these types — but the trampoline path for them panics with a deliberate "Sprint 36+" message. Sprint 36+ wires the trampoline shape that actually marshals native floats when a real use case demands it.

E. **`%`-primitive routing, not `define c-function`.** Sprint 28's `define c-function` path goes through `LoadLibrary`/`GetProcAddress` to look up Win32 DLL exports. The COM shim functions live in our own process, not in a DLL, so the Sprint 28 path doesn't apply. Sprint 35 wires every shim as a `%`-primitive in `LOWER_PRIMITIVE_TABLE` (the same mechanism as `%struct-get-i32`, `%nod_make_table`, etc.) — codegen emits a `DirectCall { callee: "nod_*", … }`, the JIT layer binds the runtime symbol via `LLVMAddGlobalMapping`, and the call returns straight through the standard primitive ABI. Dylan source uses `%dxgi-create-factory()` style invocation directly.

F. **Fixnum-tag discipline at the FFI boundary.** Sprint 28 primitives that return raw u64 values (handles, counts, HRESULTs) are passed back as Dylan Words through the primitive-call result temp. The Word tag bit must be 0 (fixnum) — a raw odd integer like 1 would parse as a pointer-tagged Word and trigger a null-pointer-dereference in the formatter. Sprint 35 introduces `tag()` and `untag()` helpers in `com_shim.rs`: every shim entry untags its u64 args before use and wraps every successful return in `tag()`. The macro-generated typed accessors do the untagging once at the lookup boundary.

G. **String marshaling.** DirectWrite expects UTF-16. Sprint 35 shims that take string args (font family, locale, text content) accept Dylan `<byte-string>` Words and convert UTF-8 → UTF-16 on the stack via `utf16_from_dylan_byte_string` helpers. This reuses `winffi::read_dylan_byte_string` (made `pub(crate)` for the dependency). No new trampoline path required.

**Shim surface (32 entries).** All `extern "C-unwind" fn(u64, …) -> u64`.

*Lifecycle / diagnostics:*

- `nod_com_release(handle)` → drop the registry entry, refcount goes to zero.
- `nod_com_registry_len()` → diagnostic count of live entries.
- `nod_com_last_hresult()` / `nod_com_clear_last_hresult()` → thread-local last-error.

*DXGI (3):* `nod_dxgi_create_factory`, `nod_dxgi_device_from_d3d_device`, `nod_dxgi_create_surface_from_texture`.

*D3D11 (3):* `nod_d3d11_create_device` (tries hardware, falls back to WARP), `nod_d3d11_get_immediate_context`, `nod_d3d11_create_texture_2d` (USAGE_DEFAULT + BIND_RENDER_TARGET + BIND_SHADER_RESOURCE).

*D2D (10):* `nod_d2d_create_factory` (`ID2D1Factory1` for device interop), `nod_d2d_create_device`, `nod_d2d_create_device_context`, `nod_d2d_create_bitmap_for_target` (wraps a DXGI surface as an `ID2D1Bitmap1`), `nod_d2d_set_target`, `nod_d2d_begin_draw`, `nod_d2d_end_draw` (returns HRESULT), `nod_d2d_clear`, `nod_d2d_set_transform_identity`, `nod_d2d_create_solid_color_brush`.

*Drawing primitives (3):* `nod_d2d_draw_text_layout`, `nod_d2d_draw_rectangle`, `nod_d2d_fill_rectangle`.

*DirectWrite (4):* `nod_dwrite_create_factory`, `nod_dwrite_create_text_format`, `nod_dwrite_create_text_layout`, `nod_dwrite_get_layout_metrics` (returns packed width+height).

*Pixel readback (4):* `nod_d3d11_copy_to_staging_and_map` (creates a CPU-readable staging texture, copies GPU→staging, calls `Flush`, then `Map`s for read), `nod_d3d11_last_staging_handle` / `nod_d3d11_last_mapped_row_pitch` (companions returning the staging handle + row pitch from the last copy), `nod_d3d11_unmap`, plus `nod_count_non_zero_red` (scans BGRA8 pixels at byte+2 of each 4-byte pixel and counts non-zero).

**ID2D1RenderTarget cast trick.** The `windows` crate doesn't auto-deref `ID2D1DeviceContext` to its parent `ID2D1RenderTarget`. Many drawing methods (`BeginDraw`, `EndDraw`, `Clear`, `SetTransform`, `CreateSolidColorBrush`, `DrawRectangle`, `FillRectangle`, `DrawTextLayout`) live on the parent. A `dc_as_render_target(dc_handle) -> Option<ID2D1RenderTarget>` helper does an `IUnknown::cast()` on the device-context interface (which is the same underlying COM object) to obtain the typed render-target view. The cast is a vtable lookup, essentially free.

**Headline acceptance — `d2d_offscreen_renders_text_glyphs`.** Dylan source builds the entire device chain, clears the texture to opaque black (so non-zero-red pixels can only be from the red brush), draws "hello, dylan" with DirectWrite at (10, 50) in 24-DIP Segoe UI, maps the staging texture, and counts red-channel pixels.

Run output: **`717`** red pixels rendered (out of 65 536 total) — proof that text glyphs, not background fill, produced the red. The chain exercises every layer: DXGI factory, D3D11 device, D3D11 texture allocation, DXGI surface cast, D2D factory + device + device context + bitmap, DirectWrite factory + text format + text layout, solid-color brush, BeginDraw/EndDraw bracketing, CPU readback through a staging texture, and pixel-level pointer reading. **Sprint 35's headline goal — text glyphs rendered into pixels we read back — is met.**

**Refcount discipline acceptance.** `ten_handles_released_clears_registry` creates 10 DXGI factories from Dylan source, walks `%com-registry-len()` to confirm the count grows to 10, releases each one, walks the count back to 0. Asserts `before - after == 10` — and gets exactly 10. The `windows` crate's `Drop` discipline propagates through our registry: removing a `HashMap` entry drops the typed wrapper which calls `Release`. No leaks observed across 5 sequential test runs.

**Refcount registry empty-after-reset acceptance.** `refcount_registry_starts_empty_after_reset` calls `%com-registry-len()` immediately after `_reset_com_registry_for_tests()` and asserts 0. Proves the test-side reset path zeros the registry cleanly.

**EndDraw success acceptance.** `d2d_clear_and_end_draw_succeed` builds the chain to the device-context level, calls `BeginDraw → Clear(128,64,200,255) → EndDraw`, and asserts the HRESULT return is 0 (S_OK). This is the closest Sprint 35 comes to "float-marshaling proof" — the clear's 4 color channels are Dylan integer args, the shim converts each to f32, the call succeeds.

**Tests added (14).** `winffi_d2d.rs` ships 11 `#[serial]` tests covering the headline, every factory creation, the refcount discipline, and EndDraw success, plus 2 non-serial `<c-float>` / `<c-double>` class-registration sanity checks. The com_shim module's `#[cfg(test)] mod tests` adds 3 unit tests proving the registry's COM-Drop discipline at the Rust level. Test counts: baseline 532 → 546 (+14). All `#[serial]` because the COM handle registry is process-global.

**Verification.**
- `cargo test --workspace --no-fail-fast`: 546 passed, 0 failed, 7 ignored.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean (only warnings are in the external `newgc-core` crate, same as baseline).
- 5x sequential flake check: 546/0/7 every run.

**Out of scope for Sprint 35 (deferred — see DEFERRED.md):**

- **Float-marshaling trampoline shape.** `<c-float>` / `<c-double>` are registered for sema-acceptance only; no Sprint 35 shim takes a native float arg. Sprint 36+ ships per-shape trampolines for Win64 floats-in-XMM marshaling when a use case demands it (e.g. Direct2D animation curves with real-valued time arguments).
- **HWND-bound swap chains.** Sprint 35 ships offscreen-only rendering — no `CreateSwapChainForHwnd`, no `IDXGISwapChain1`, no `Present()`. Lights up in Sprint 37 once the IDE window exists.
- **Linear/radial gradient brushes, geometries, paths.** Solid-color brush only.
- **WIC bitmap interop.** Image loading from PNG/JPEG via the Windows Imaging Component is a follow-on.
- **D2D effect graphs and animations.** Useful for IDE polish; not Sprint 35.
- **Device-loss recovery.** When the GPU is reset (driver crash, monitor change), every D3D11 / D2D resource is invalidated. Production polish.
- **Compositional swap chains.** `CreateSwapChainForComposition` enables IDE panes embedded in non-Win32 hosts (XAML islands, etc.). Later.
- **Hand-rolled C++ shim DLL (the original Sprint 35 brief).** Superseded by the `windows`-crate approach. The C++ shim is no longer on the roadmap.

### Sprint 29b — `format` + `print` + `streams` (`io` library kernel)
Slipped from the old Sprint 27 slot when Sprint 27 absorbed the FFI Phase A work. Port `opendylan-tests/sources/io/tests/format.dylan`, `print.dylan`, `streams.dylan` against ported `io` library code. Removes the `format-out` FFI shim.

### Sprint 29c — Kernel library port: arithmetic, characters, symbols
Port enough of `sources/dylan/` (`number.dylan`, `character.dylan`, `symbol.dylan`, `boolean.dylan`) that the runtime stops providing these directly and the language defines them in itself.

### Sprint 30 (planned, slipped) — Dylan-side IDE bring-up: window, message pump, editor surface
**Slipped:** the Sprint 30 slot was reclaimed by FFI Phase C (string marshaling — see "Sprint 30 — FFI Phase C" above). This IDE-bring-up plan stays on the roadmap and runs after Sprint 31's `common-dylan` port. First IDE sprint. **All Dylan code**, written against the Sprint 25b Windows FFI stack. Module `nod-dylan/ide-shell` registers a top-level window class, runs the message pump, hosts a single editable text pane and a REPL transcript pane. No syntax colouring yet, no menus — just "the compiler can open a window and let you type into it". Re-implements the scaffolding of `E:\opendylan\sources\environment\framework\` in Dylan.

### Sprint 30b — Dylan-side inspector + dispatch visualisation
With the IDE shell up, port the existing `:inspect` / `:dispatch-stats` / `:classes` REPL commands into IDE panels written in Dylan. Inspector handles every kernel class. Time-travel REPL prototype.

### Sprint 31 — `common-dylan` library port
Port `byte-vector`, `simple-format`, `simple-io`, `simple-random`, `transcendentals`, `threads/`. Run `opendylan-tests/sources/common-dylan/tests/`.

### Sprint 32 — Multi-threaded mutator + cooperative GC across threads
Thread-local TLABs, parking protocol, lock primitives in Dylan-side code. Run `opendylan-tests/sources/app/thread-test/`.

### Sprint 33 — Library-merge optimisation (v2 candidate moved up if cheap)
DFM serialisation, cache-key extension with downstream library hashes, cross-library inlining gated on sealing. May slip to post-v1.

### Sprint 34b — AOT mode — emit a standalone Windows executable
(Renumbered: Sprint 34 was reclaimed by the `<c-struct>` family work above — see "Sprint 34 — Structs".) JIT artefacts written out as a PE binary plus a shipped `nod-runtime` static lib. Cache key already covers it; mostly a packaging exercise.

### Sprint 35b — Dylan-side IDE polish: debugger, library browser, sealed-domain visualiser to v1.0 quality
(Renumbered: Sprint 35 was reclaimed by the COM / DXGI / D3D11 / D2D / DirectWrite infrastructure work above — see "Sprint 35 — COM via `windows` crate".) All in Dylan, on top of the Win32 FFI stack: source-stepping debugger, library browser with cross-references, sealed-domain visualiser usable on real programs. Re-implements the feel of `E:\opendylan\sources\environment\debugger\`, `editor/deuce/`, and `commands/`.

### Sprint 36 — macOS port (aarch64-apple-darwin first)
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
