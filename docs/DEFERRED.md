# NewOpenDylan — Deferred Work

*Living list of work consciously deferred from a landed sprint. Each entry
records what is missing, the sprint that introduced the gap, and the
sprint (or condition) that lights it back up. Items move to `:closed:`
status when a follow-up sprint lands the implementation.*

Format per entry: `:status: title — owner-sprint → unblock-sprint. brief.`

---

## Carry-over from Sprint 02 (lexer)

- **:closed: `nod-ide` Win32 shell** — Sprint 02 → cancelled. The
  manifesto was revised to **compiler-first** (core decision 8): the
  IDE is no longer a Rust crate. It will be a Dylan program compiled by
  NewOpenDylan, calling Win32 directly through `c-ffi` over the Windows
  FFI stack borrowed from NCL (Sprint 23b). First IDE shell lands at
  Sprint 26 — *after* the compiler can JIT and run non-trivial Dylan.
  No leftover Rust GUI work remains.

## Carry-over from Sprint 03 (fragments + Pratt parser)

- **:open: `select` form** — Sprint 03 → Sprint 04 or 18. Parser emits a
  structured diagnostic instead of an AST node. `case` is fully implemented;
  `select` was the optional drop per Sprint 03 brief.
- **:open: `parse_expr(src, tokens)` extra `src` parameter** — Sprint 03 →
  ergonomics-only. The signature deviates from the brief sketch because
  `Token` is lifetime-free and identifier text must be recovered from spans.
  Either (a) keep `src` parameter, (b) add a `SourceMap` argument and read
  through it, or (c) carry `&str` slices on `Token`. Decide before Sprint 04
  builds on top.
- **:open: `case` arm separation heuristic** — Sprint 03 → Sprint 04.
  Parser uses a `;` + `=>` look-ahead to chunk arms; sufficient for
  expression-level grammar but the full statement-body parser in Sprint 04
  should revisit.

## Carry-over from Sprint 04 (definitions + body parser)

- **:open: Statement-macro forms** — Sprint 04 → Sprint 17 → Sprint 18.
  Calls like
  `with-lock (lock) body end;`, `printing-logical-block (stream) body end;`
  are syntactic-macro-defined statement forms whose body is delimited by
  the macro's `end`. `parse_stmt_body` accepts them by treating the
  un-`;`-terminated head as a complete statement and letting the macro's
  body be a sequence of follow-on statements; the resulting AST is
  *wrong* but the parser stays in sync. Sprint 17 ships the matching
  / substitution engine for `Expr::*` shapes; statement-position
  expansion still needs a `Statement::*` recogniser that consumes
  trailing follow-on statements as the macro's body — Sprint 18.
  Fixtures impacted include `io/tests/temp-files.dylan`,
  `io/tests/streams.dylan`, `io/tests/print.dylan`,
  `common-dylan/tests/macros-tests.dylan`.
- **:open: `define sealed domain` (sealing declaration)** — Sprint 04 →
  Sprint 15. Currently recognised via the catch-all
  `Item::DefineOther { keyword: "domain", ... }` path with body captured
  as fragments. Real semantics (sealing graph + sealedness checks) land
  with method dispatch in Sprint 13/15.
- **:open: `define test` / `define suite` / `define table`** — Sprint 04
  → Sprint 04 or library-internal. Same catch-all path; body captured as
  raw fragments. Testworks/suite forms re-expand to plain `define
  function` + `define method` once Sprint 17 macros work.
- **:open: Multi-value `define constant (a, b) = …`** — Sprint 04 →
  Sprint 06. Parser keeps the first binder name and drops the rest into
  the value-shape; full multi-value binding lands when DFM models
  multi-value flow.
- **:open: `case` arm with multiple cond values** — Sprint 04 → Sprint 17.
  Plain `case` with `cond1, cond2 => body` (as in `macros.dylan:312`)
  is shorthand the v1 parser doesn't accept; `select` form handles the
  same shape. Treat as `select` until macros land.
- **:open: Slot adjective combinations** — Sprint 04 → Sprint 13. Slots
  carry an `allocation: SlotAllocation` enum but the open/sealed/inherited
  adjectives are silently consumed without modelling. Add per-slot
  modifier vec when sealing semantics need it.
- **:open: Keyword-argument lowering uses synthetic `%kw-arg` call** —
  Sprint 04 → Sprint 06. `f(x: 1)` lowers to
  `Call(f, [Call(%kw-arg, [Symbol("x:"), 1])])`. The pretty-printer maps
  back to `x: 1` so the round trip is shape-stable, but the IR layer
  will want a real `KeywordArg` variant on `Expr` (or on `Call`).
- **:open: `let handler` / handler bindings** — Sprint 04 → Sprint 17 or
  whenever exception semantics land. The current `let` parser accepts
  the surface but the handler shape is just a plain Let; exception-clause
  installation isn't modelled.
- **:open: Hash-keyword indexing literal `#[ … ]` (limited-typed array)
  and ratio literals** — Sprint 04 → Sprint 06. Hash-prefixed grouping
  literals lower to `Call(#vector | #list | #set, args)`; full literal
  semantics + `<ratio>` numeric values come with DFM.
- **:open: 21 fixture files fail round-trip** — Sprint 04 → Sprint 17.
  Round-trip clean ratio: 45 of 66 swept fixtures (>= 20 acceptance
  threshold from the brief). Failing categories all reduce to
  statement-macros, multi-value `case` arms, or `define test` bodies
  whose nested grammar Sprint 04 doesn't fully model. Files:
  `dylan/tests/{control,macros,regressions,collections,specification}.dylan`,
  `cmu-test-suite/{dylan-test,dylan-test-extras,run-tests}.dylan`,
  `collections/tests/{bit-vector-*,bit-set-tests,collectors}.dylan`,
  `common-dylan/tests/{byte-vector,collection-test-utilities,
  common-extensions-tests,condition-test-utilities,format-tests,
  machine-words,macros-tests,number-test-utilities,numerics-tests,
  stream-test-utilities,transcendentals}.dylan`,
  `io/tests/{pprint,print,streams-benchmarks,streams,temp-files}.dylan`.

## Carry-over from Sprint 05 (LID + module graph)

- **:open: `use` / `import:` / `exclude:` / `rename:` / `prefix:` /
  `export:` resolution** — Sprint 05 → Sprint 06 (after Sprint 04 lands
  `define module` / `define library` parsing). Types from spec §7
  (`LibraryUse`, `ModuleUse`, `Import`, `Reexport`, `LibraryRef`,
  `ModuleRef`) all exist; `add_library_from_lid` and `add_module`
  populate `uses: Vec::new()`. Graph cannot answer cross-module name
  lookups until the AST forms are walked.
- **:open: `BindingId` allocator** — Sprint 05 → Sprint 13 (when
  inline-cache hooks need it). `Module::bindings` is an empty `HashMap`;
  no minting API yet.
- **:open: Per-library / per-module generation-counter bump logic** —
  Sprint 05 → Sprint 08 (REPL hot-reload trigger). Fields exist and stay
  `0`; the bump policy from `MANIFESTO.md` lines 172-196 lights up when
  the REPL exists.
- **:open: `.hdp` file integration test** — Sprint 05 → Sprint 23 or
  whenever a `.hdp`-bearing fixture is needed. `parse_lid` works on any
  path; no dedicated test exists.
- **:open: Platform-conditional LID selection** — Sprint 05 → Sprint 5.5
  follow-up. `Platforms:` field is parsed and recorded; the registry
  algorithm that picks the matching LID per host triple is unwritten. v1
  driver will need an explicit `--platform` flag.

## Carry-over from Sprint 06 (DFM IR skeleton + AST → DFM lowering)

- **:open: `Computation::Values` / `BindExit` / `UnbindExit` / `Closure` /
  `MakeEnvironment`** — Sprint 06 → Sprint 08 (`Values` for multi-value
  return) / Sprint 19 (`Bind/UnbindExit` for `block` + NLX) / Sprint 11+
  (`Closure` + `MakeEnvironment` for `local method` and lambda). The
  Sprint 06 brief enumerated them; the kernel-subset lowering does not
  emit any. They are documented as `TODO(sprint-NN)` comments in
  `nod-dfm::ir`; verifier will reject them if hand-built today (no
  variants exist).
- **:open: First-class reference to a top-level function** — Sprint 06
  → Sprint 11+ (closure conversion). `Expr::Ident(name)` where `name`
  resolves to a top-level function is currently rejected as `Unsupported`
  unless it appears in the callee position of a `Call`. Closures will
  package the call protocol once `MakeEnvironment` / `Closure` exist.
- **:open: Int↔Float implicit coercion** — Sprint 06 → Sprint 12+. Mixed
  `<integer> + <double-float>` produces a `LoweringError::TypeMismatch`.
  Strategy alternatives: (a) add `PrimOp::IntToFloat` / `FloatToInt`
  coercion nodes; (b) make `+` a generic and let Sprint 13 dispatch
  resolve it. Decision parked. Untyped-`<top>` operands default to int.
- **:open: `Expr::Call` against a non-ident callee** — Sprint 06 →
  Sprint 13. Today only `Call { callee: Ident, args }` lowers, to
  `DirectCall`. Higher-order calls (`Call` IR variant, callee in a
  temp) need the runtime function-value representation that Sprint 09
  introduces (`<wrapper>`+function pointer); kernel subset doesn't
  exercise it.
- **:open: Top-level function lookup is name-keyed within a module
  only** — Sprint 06 → Sprint 07. `TopNames` is a flat `HashSet<String>`
  populated from `Item::DefineFunction`. Cross-library resolution will
  need `nod-namespace`'s module graph; Sprint 06 was sized to one
  source file at a time.
- **:open: Single-binder `let` only** — Sprint 06 → whenever `Values`
  lands. `let (a, b) = …` is rejected. Multi-value `define constant`
  (DEFERRED Sprint 04 entry) blocks on the same machinery.
- **:closed: `Statement::While` / `Statement::Until` lowering** —
  Sprint 06 → Sprint 18. Both now lower to a three-block CFG
  (entry → header → body / exit) with proper phi/block-param
  threading for loop-carried mutable variables. `lower_while_like`
  pre-scans the body for assigned-to names and creates header
  block parameters for each; the back-edge supplies the post-body
  temps as Jump args. Local-variable reassignment via `:=`
  (previously only supported for slot setters) updates the env's
  name → temp mapping in place.
- **:open: `Statement::For` / `Block` / `Local`** —
  Sprint 06 → Sprint 25 (`for`) / Sprint 19 (`block` / NLX +
  `local method`). All three still emit `LoweringError::Unsupported`.
  Sprint 18 closes the loop subset (`while` / `until`); `for`
  needs the upstream macro expansion to land in Sprint 25.
- **:open: Sprint 06 verifier checks textual-order definedness, not
  SSA dominance — back-edges still pass via block params** —
  Sprint 06 → Sprint 18+ (full dominance analysis is optimiser
  work). Sprint 18 confirms the existing weakened invariant
  composes with back-edges: the loop header's block params are
  defined before any computation in the header, the body uses them
  in a successor visit (declaration order), and the back-edge
  jump's args refer to body-defined temps already visited. No
  verifier change was required for the kernel + loop subset; a
  proper RPO + dominator walk lands when the optimiser does.

## Carry-over from Sprint 07 (LLVM codegen + JIT thin slice)

- **:open: `TypeEstimate::Top` / `Bottom` → `i64` default** — Sprint 07 →
  Sprint 09+ (tagged-pointer ABI). The codegen maps both lattice
  extremes to `i64` for now. Once `<wrapper>` headers and the
  tagged-pointer `Value` ABI land in Sprint 09, every `<top>` value
  becomes a register-sized word with the same machine type — so this
  default coincides with the long-term ABI by accident; no SSA traffic
  in `<top>` actually flows through the kernel-subset functions today.
- **:open: No `gc.statepoint` / safepoint poll emission** — Sprint 07
  → Sprint 11. Codegen emits plain `call`s without statepoint
  bundles. Stack maps and cooperative parking light up when the GC
  bring-up sprint lands.
- **:open: Single-module JIT, no incremental install** — Sprint 07 →
  Sprint 08 (live REPL image) / Sprint 11 (generation discipline).
  `Jit::add_module` allocates a fresh MCJIT engine per call. Symbol
  resolution does not cross modules; a later module cannot call into
  an earlier one. Replace with one growing module + per-definition
  install when the REPL gains persistence.
- **:open: No optimisation passes** — Sprint 07 → Sprint 11+. The
  `LLVMCreateMCJITCompilerForModule` invocation pins `OptLevel = 0`.
  Inlining, dead-code elimination, and basic loop optimisations are
  deferred until the IR shape stabilises post-GC.
- **:open: `Computation::Call` (indirect) returns a codegen error** —
  Sprint 07 → Sprint 13. The kernel-subset DFM doesn't emit indirect
  calls; if it ever does, `codegen_module` reports
  `CodegenError::IndirectCallNotSupported` rather than silently
  miscompiling. Lights up with first-class functions / closures.
- **:open: `<string>` JIT result format** — Sprint 07 → Sprint 10.
  `eval_expr_to_string` returns a placeholder for `<string>` return
  types because the kernel JIT has no heap-allocated string layout
  yet. Strings get a real layout in Sprint 10.
- **:open: `inkwell` feature-set override in `nod-llvm` and `nod-sema`
  Cargo.toml** — Sprint 07 → indefinite (cosmetic). The workspace root
  pins `inkwell = { version = "=0.9.0", features = ["llvm22-1"] }` with
  default features (= every LLVM target). The local LLVM install at
  `C:\projects\LLVM\install` is built x86-only. `nod-llvm` and
  `nod-sema` re-declare the dep without `workspace = true` to set
  `default-features = false, features = ["llvm22-1", "target-x86"]`.
  When the workspace root migrates to the slimmer feature list (NewM2
  already runs that way), drop the override.
- **:open: `eval_expr_to_string` `let X; expr end` heuristic** —
  Sprint 07 → Sprint 08. To satisfy the acceptance case
  `eval_expr_to_string("let x = 41; x + 1 end")`, the wrapper strips a
  trailing `end` only when the expression starts with `let `. A real
  REPL pipeline (Sprint 08) will parse the input itself rather than
  re-wrap text.

## Carry-over from Sprint 09 (tagged pointers + bump heap + class metadata)

- **:open: No garbage collection — bump allocator only** — Sprint 09 →
  Sprint 11. `nod_runtime::Heap` is a one-way bump pointer over a
  VirtualAlloc / mmap reservation. Once the reservation fills, the
  next allocation panics. Sprint 11 turns this into a young-generation
  copying collector with `gc.statepoint`-driven precise roots.
- **:open: `<wrapper>` GC bits are zero** — Sprint 09 → Sprint 11. The
  16 high bits of the header are reserved for mark / age / pinned /
  has-finalizer flags. Sprint 09 leaves them all zero; Sprint 11
  populates them.
- **:open: Floats stay unboxed in JIT function returns** — Sprint 09 →
  Sprint 10. A function whose return type is `<double-float>` still
  returns a raw `f64` from the JIT — the same calling convention
  Sprint 07 committed to. Sprint 10 introduces a heap-allocated
  `<double-float>` box and routes float returns through it (or keeps
  the unboxed path for sealed-domain calls — decided in Sprint 15).
- **:open: `instance?` only handles `<integer>` and `<boolean>`
  directly** — Sprint 09 → Sprint 12. `<integer>` tests bit 0 of the
  word. `<boolean>` folds to a constant `#f` because Sprint 09's
  immediate scheme doesn't yet distinguish boolean fixnums from
  ordinary fixnums (`#t` = tagged 1, `#f` = tagged 0 share the
  fixnum tag). Other classes route through `ClassCheck::Unsupported`
  and constant-fold to `#f`. Sprint 12 fills in `<wrapper>`-based
  class-id comparisons for user classes; Sprint 10 may give booleans
  a distinct immediate sub-tag.
- **:open: `nil` representation is provisional** — Sprint 09 → Sprint
  10. `Word::NIL` is currently `Word(0)` — i.e. fixnum-tagged zero,
  indistinguishable from `0`. Dylan doesn't formally have `nil`
  (it has `#f` and `#()`) so this is mostly a placeholder for the
  C-FFI layer that Sprint 23b will need. Decide between (a) keep
  using `#f` everywhere `nil` is needed, or (b) carve out an
  immediate sub-tag when the encoding grows.
- **:open: Single-threaded heap** — Sprint 09 → Sprint 11+. `Heap`
  serialises allocations through a `Mutex`. Multi-threaded mutators
  need thread-local allocation buffers (TLABs); the TLAB design is
  inherited from NCL and lights up alongside the collector.
- **:open: `Heap::wrapper_of` takes two mutex acquisitions** — Sprint
  09 → Sprint 11. Cosmetic: the function locks once to read `base`,
  unlocks, locks again to read `capacity`. Single-threaded Sprint 09
  doesn't care; collapse during Sprint 11's heap rework.
- **:open: Fixnum overflow at compile time only** — Sprint 09 →
  Sprint 12. Integer *literals* outside the 63-bit signed range are
  rejected during lowering with `LoweringError::IntegerOverflow`.
  Runtime overflow (`huge * huge`) silently wraps modulo 2^62 —
  there is no overflow check on `MulInt`. Sprint 12's `<big-integer>`
  / `<double-integer>` adds the overflow-check fast path.
- **:open: `StaticArea::alloc` leaks on every call** — Sprint 09 →
  Sprint 11. The shadow `Vec<Box<dyn Any>>` keeps boxes alive for
  the area's lifetime but does so by reconstructing the Box from
  the leaked pointer and re-pushing. The `Drop` impl on `StaticArea`
  would free everything, but in practice the area lives for the
  process; tighten if Sprint 11 carves the area into per-library
  arenas.
- **:open: `define class` still rejected** — Sprint 09 → Sprint 12.
  User-defined classes don't lower yet; the seed table in
  `nod_runtime::classes` holds only the eight built-in classes that
  `instance?` and the dispatch caches need.

## Sprint 10 (heap objects, immediates, tracer, format-out)

### Closed by Sprint 10

- **:closed: `<wrapper>`-based `<boolean>` instance check** — Sprint 09
  item #4 (`instance?` only handles `<integer>` and `<boolean>`
  directly). `#t` / `#f` are now pinned heap-shape singletons whose
  wrapper carries `ClassId::BOOLEAN`; `instance?(#t, <boolean>)` and
  `instance?(#f, <boolean>)` both return `#t`, integers return `#f`.
  Implemented in `nod-runtime::immediates`, `nod-llvm::codegen`'s
  `emit_wrapper_class_check`, and the new `ClassCheck` variants in
  `nod-dfm::ir`.
- **:closed: `nil` representation** — Sprint 09 item #6. `nil` is no
  longer `Word(0)`; it's a pinned `<empty-list>`-wrapped singleton in
  the literal pool's `StaticArea`. `Word::NIL` retains its old value
  for back-compat but new codegen and `ConstValue::Unit` lower through
  `Immediates::nil`.
- **:closed: `<string>` JIT result format** — Sprint 07 carry-over.
  `eval_expr_to_string` now resolves `<string>`-returning entries to
  `<byte-string>` heap objects and prints them via the literal-pool
  lookup. `format-out("...")` round-trips end-to-end.

### Opened by Sprint 10 (still deferred)

- **:open: Floats stay unboxed in JIT function returns** — Sprint 09 →
  Sprint 12. The Sprint 09 deferred entry remains: `<single-float>` /
  `<double-float>` return raw `f32` / `f64`. Boxing decision is
  deferred to Sprint 12 (when richer types arrive) / Sprint 15 (sealed
  domains may keep the unboxed path).
- **:open: `<unicode-string>` (UTF-16 / wide)** — Sprint 10 → Sprint 27
  (`unicode` library port). The Sprint 10 byte-string is UTF-8 only.
- **:open: `make-string` / `make-vector` Dylan-callable constructors**
  — Sprint 10 → Sprint 12. Heap allocation paths exist
  (`Heap::alloc_byte_string`, `Heap::alloc_simple_object_vector`,
  `SymbolTable::intern`); the only call sites Sprint 10 wires are
  literal-driven (codegen interning). Generic `make` lands when
  Sprint 12 ships classes + `define class`.
- **:open: Hash-table (`<table>`)** — Sprint 10 → Sprint 21 per
  SPRINTS.md.
- **:open: `:inspect` REPL meta-command + `dump-heap` driver
  subcommand** — Sprint 10 → Sprint 26 (IDE) for the interactive
  inspector, Sprint 08 (live REPL) for the meta-command line form.
  The tracer + `HeapTrace::format` are ready; the CLI surface is not
  wired today because Sprint 08 is spec-only.
- **:open: `format-out` to anywhere but stdout (or the test thread-
  local writer)** — Sprint 10 → Sprint 24 (`streams` library). The
  Sprint 10 shim recognises `%d` / `%s` / `%%` only; full `format`
  / `print` directive set lands with the `io` library port.
- **:open: Mark / age / pinned bits on `Wrapper`** — Sprint 09 →
  Sprint 11. Still zero; the tracer reports them but doesn't write
  them. Sprint 11's collector populates them.
- **:open: Float printing format choice** — Sprint 10 → cosmetic.
  `eval_expr_to_string` still prints `6` for `3.0 * 2.0`; whether to
  surface `6.0` is a presentation decision parked for the streams
  port.
- **:open: `<character>` boxing** — Sprint 10 → Sprint 12. Characters
  remain raw `i32` in SSA; `ClassCheck::Character` therefore always
  returns `#f` (no wrapper to read). Sprint 12 boxes characters as
  pinned singletons (256-entry table for the BMP).
- **:open: Symbol literals (`#"foo"`) not lowered through codegen** —
  Sprint 10 → Sprint 17 (macros) / Sprint 25 (kernel library port).
  The `SymbolTable::intern` machinery exists; `Expr::Symbol` still
  emits a `LoweringError::Unsupported`. Hooking literal-pool intern
  into the lowering path is a one-line change once a fixture needs it.
- **:open: First-class function references through the literal
  pool** — Sprint 10 → Sprint 11+. The literal pool currently pins
  strings + symbols + immediates only; pinning JIT-baked function
  pointers (so closures can carry them) is a Sprint 11 task that
  rides alongside stack-map emission.
- **:open: Per-library / per-module literal pool** — Sprint 10 →
  Sprint 11. Today's `LITERAL_POOL` is a single process-global pool.
  When module retirement lands, codegen needs per-module pools so
  retired modules can free their string + symbol literals.
- **:open: Static area's double-leak shadow** — Sprint 09 carry-over,
  still parked. The `Box::from_raw` + push to vec pattern in
  `StaticArea::alloc` survives intact; revisit when Sprint 11 carves
  the static area into per-library arenas.
- **:open: `nod_format_out` arity 5+** — Sprint 10 → Sprint 24. Cap
  is currently four arguments (fmt + three). Beyond that, codegen
  errors. Real `format` machinery is in Sprint 24.

## Sprint 11 (generational copying GC + class-driven scanning + write barrier)

### Closed by Sprint 11

- **:closed: No garbage collection — bump allocator only** — Sprint 09
  carry-over. Sprint 11 replaces the bump heap with a semispace
  generational copying collector (young + 2-semispace old) lifted
  structurally from NCL's `ncl-runtime/src/heap.rs` and heavily adapted
  for Dylan's one-bit tag + `Wrapper`-with-`ClassId`. `Heap::alloc_object`
  routes into young; minor GC promotes survivors into old; full GC
  evacuates young + old.live into old.scratch and swaps.
- **:closed: `<wrapper>` GC bits are zero** — Sprint 09 carry-over.
  Sprint 11 carves 4 bits out of the 16-bit GC field on `Wrapper`:
  Mark, Tenured, Pinned, Forwarded. Each is set/cleared via
  `Wrapper::with_gc_bit` / `::without_gc_bit`. The Forwarded bit
  doubles as the encoding marker for a forwarding pointer; the new
  address occupies the class-id slot, shifted right by 8 to fit. See
  `wrapper.rs` for the encoding contract.
- **:closed: Mark / age / pinned bits on `Wrapper`** — Sprint 10 entry.
  Same change as above; explicitly tracked separately because the
  Sprint 10 brief noted "the tracer reports them but doesn't write
  them" — Sprint 11's collector now sets `Tenured` on every survivor
  copy and `Pinned` on every conservatively-pinned object.
- **:closed: Single-threaded heap** — Sprint 09 carry-over (TLAB
  requirement). Sprint 11 doesn't ship per-thread TLABs yet, but the
  `Heap` is `Send + Sync` and the inner state is guarded by a
  `Mutex`, so the single-mutator-with-single-collector cross-thread
  story is correct (i.e. it's no worse than Sprint 09 and is sound;
  multi-mutator TLABs land alongside `gc.statepoint` in Sprint 11b).
- **:closed: Per-library / per-module literal pool** — Sprint 10 entry
  (#"Per-library / per-module literal pool"). The literal pool now
  routes through the **static area** (pinned, never collected). Sprint
  11 doesn't carve per-library pools yet — that arrives with module
  retirement — but the moveability hazard the Sprint 10 entry warned
  about is gone: codegen-baked addresses can never move.

### Opened by Sprint 11 (still deferred)

- **:open: `gc.statepoint` precise stack roots** — Sprint 11 → Sprint
  11b. The brief explicitly allowed conservative stack scanning as
  the bring-up choice. `Heap::pin_stack_range(lo, hi)` walks an
  address range, decoding each 8-byte slot as a `Word` and pinning
  the target if it looks like a heap pointer. Sprint 11b will (a)
  emit `gc.statepoint` / `gc.relocate` bundles at every JIT call
  site, (b) lift NCL's stack-map decoder, (c) add the safepoint-poll
  lowering pass to `nod-llvm::codegen`. Until then, the JIT-side
  parking story is "the GC only runs at Rust-side allocation sites".
- **:open: JIT safepoint poll emission** — Sprint 11 → Sprint 11b.
  Same root cause. Today's codegen emits plain `call`s; the brief's
  option (b) lets us defer the poll-and-park machinery. Concretely
  this means a JIT'd function that runs in a tight loop without
  allocating never yields to the collector — but Sprint 11's stress
  test (Rust-side allocation loop) already exercises the path the
  Sprint 12+ Dylan-side loops will reach via primops that allocate.
- **:open: Multi-threaded mutator + per-thread TLABs** — Sprint 11 →
  Sprint 11b / 28. The `Heap` is mutex-guarded; allocation is single-
  threaded in practice. NCL's `mutator.rs` (TLAB design + cooperative
  park) is the reference; Sprint 28 picks it up alongside the threads
  library port.
- **:open: `Computation::WriteBarrier` IR variant exists but no
  lowering emits it** — Sprint 11 → Sprint 12. The IR node + the
  verifier/format support are wired; the codegen path returns
  `CodegenError::WriteBarrierNotEmitted` if any lowering emits one
  (none does today). Sprint 12's slot setters will be the first
  emitter.
- **:open: `nod_runtime::write_barrier` is the canonical Rust-side
  store path but isn't yet wired into vector slot writes** — Sprint 11
  → Sprint 12. The Sprint 10 `vectors.rs::slots_mut` callers still
  store directly. Sprint 11's `write_barrier` is in place for any
  caller that wants it; Sprint 12 retrofits the vector + symbol-table
  setters.
- **:open: Pinned young objects are promoted, not held in place** —
  Sprint 11 → Sprint 11b. The brief flagged this: a Pinned object
  "should" stay where it is. Sprint 11 takes the simpler path of
  treating Pinned as a precise root (copy to old, install
  forwarding). The conservative caller's pointer becomes stale once
  it next refers to the object — which is acceptable because the
  caller (a stack scan) is a frozen snapshot. Sprint 11b's precise
  roots eliminate the need for pinning in normal operation.
- **:open: Class-pointer pinning for JIT-baked function pointers** —
  Sprint 11+ → Sprint 13 or later. The literal-pool entries codegen
  bakes today are byte-string and symbol pointers — both routed
  through the static area, so they're pinned-by-construction. When
  Sprint 13 introduces first-class function references (and the
  closure layout) the literal pool will need to pin function-value
  Words the same way. The static-area path is ready for it.
- **:open: Sprint 11's stress test is scaled to 100,000 allocations,
  not the SPRINTS.md "1 M" figure** — Sprint 11 → cosmetic. The 1M
  acceptance criterion is reachable but slow under `cargo test`. The
  100,000-allocation test exercises the same GC cycling behaviour at
  10× lower time cost. Bump to 1M when CI runs benchmark mode.
- **:open: No back-edge GC poll** — Sprint 11 → Sprint 11b. A
  long-running JIT'd loop that doesn't allocate never yields. Sprint
  11b emits a poll-and-park check at every loop back-edge alongside
  the call-site statepoints.
- **:open: Old → old write barrier integration in the JIT** — Sprint
  11 → Sprint 12. `Heap::mark_card_for` is called from the Rust-side
  `write_barrier` shim; the JIT-emitted store path skips the card
  mark (because Sprint 11 JIT'd code doesn't yet emit slot stores).
  Sprint 12's slot setters wire the card mark into the codegen
  template.
- **:open: Sprint 09 `StaticArea::alloc` double-leak shadow** —
  Sprint 09 entry, still parked. The append-only shadow Vec still
  uses the Box-from-raw + push pattern; the GC has no opinion about
  the static area's internal bookkeeping (it never visits the
  pinned-buffer ranges as movable storage). Revisit when per-library
  arenas land.

## Sprint 12 (classes + slots + single-dispatch generics)

### Closed by Sprint 12

- **:closed: `Computation::WriteBarrier` IR variant has no emitter** —
  Sprint 11 carry-over. Slot setters now emit `StoreSlot` (which lowers
  to a heap store + a call into `nod_card_mark`); the codegen path
  for `WriteBarrier` is still present as a documented stub for
  arbitrary slot-pointer stores Sprint 14+ may want.
- **:closed: `instance?` only handles seed classes** — Sprint 09 item
  #4 (and its Sprint 10 carry-over). `instance?(x, <foo>)` for a user-
  defined `<foo>` now walks the target object's class CPL via
  `nod_runtime::nod_is_instance_of`. Subclass relations against seed
  supers (e.g. `<object>`) also work.
- **:closed: `define class` rejected at lowering** — Sprint 09 #11.
  Sprint 12 lands the full `define class` / `make` / slot getters /
  setters / single-dispatch flow; the `<point>` fixture round-trips
  `distance-squared(make(<point>, x: 3, y: 4)) → 25` end-to-end.
- **:closed: `make-string` / `make-vector` Dylan-callable
  constructors** — Sprint 10 entry; replaced by the generic `make`
  intrinsic which handles user classes (and, with a slot encoding
  that matches `<byte-string>`/`<simple-object-vector>`, could carry
  the seed-collection cases too — left as a Sprint 21 follow-up
  rather than retrofitting `make` for them today).

### Acceptance deviation

- **:open: `distance-squared` substituted for `distance`** — Sprint 12
  → Sprint 21 (or whenever float boxing lands). The brief's acceptance
  used `distance(p) → 5.0`, which needs `<double-float>` boxing on
  the JIT return path. Sprint 12 substitutes `distance-squared(p) → 25`
  (integer-only) so the acceptance is reachable with the current
  unboxed-float ABI. Float boxing is Sprint 09 carry-over item #3
  and stays open.

### Opened by Sprint 12 (still deferred)

- **:closed: Multiple inheritance + indirect slot lookup** — Sprint 12
  → Sprint 14 (landed). Lowering now accepts multi-super class
  definitions; runs C3 over the parent CPLs; merges parent slots in
  most-specific-first append order; rejects same-name-different-origin
  slot conflicts with `LoweringError::SlotConflict`. The
  "indirect slot lookup" question dissolved into Sprint 13's
  per-class dispatch: each MI subclass whose inherited slot has
  shifted offset gets an **override accessor** auto-registered on
  the slot's generic; dispatch picks per receiver. See Sprint 14
  closed list below.
- **:open: Inline caches + monomorphic-then-polymorphic dispatch** —
  Sprint 12 → Sprint 13. `Computation::Dispatch` lowers to a runtime
  call into `nod_dispatch_unary` / `nod_dispatch_binary` which walks
  the dispatch table linearly. Sprint 13 adds the per-call-site
  monomorphic cache + the IR shape (`<dispatch>` vs `<direct-call>`)
  the optimisation pass needs.
- **:open: Class redefinition** — Sprint 12 → unresolved. Sprint 12
  refuses redefinition via `LoweringError::ClassRedefinitionNotSupported`.
  Three paths are on the table for v2: (a) lazy per-instance migration
  (Open Dylan's choice), (b) whole-heap migration on redefine, (c)
  forbid forever and require a new class name. Pick a path in Sprint
  28 (multi-mutator GC) where the migration cost is bearable.
- **:open: Float-typed slots** — Sprint 12 → Sprint 21. Slots typed
  `<double-float>` / `<single-float>` are recorded with `SlotType::DoubleFloat`
  but treated as pointer-shaped (visited by the GC). Until float
  boxing lands, storing a raw `f64` into the slot would be a tagging
  violation; lowering writes the value as a Word so today's accesses
  treat the slot as `<top>`-style. Document and move on.
- **:open: `make` arity limit (8 keyword pairs)** — Sprint 12 →
  Sprint 23 (c-ffi). The JIT-side `nod_make` shim is fixed-arity to
  match `nod_format_out`'s shape. Once c-ffi gives us real variadic
  calling-convention support, lift to unlimited.
- **:open: `compute-applicable-methods` / full MOP** — Sprint 12 →
  Sprint 17+. Sprint 12's dispatch is unary-and-binary only; the full
  multimethod with method combinations + before/after/around methods
  lands with the macro work.
- **:closed: Sealed-class redefinition checks** — Sprint 12 →
  Sprint 15 (landed). `Modifier::Sealed` on `define class` flips
  `ClassMetadata::sealed` (an `AtomicBool`) after class registration.
  In-library subclassing of a sealed class still works (the seal flag
  flips AFTER every class in the current `lower_module_full` call is
  registered). Cross-library subclassing — simulated as "a later
  separate `lower_module_full` call" — surfaces
  `LoweringError::SealingViolation { ... SealedClassExtendedAcrossBoundary }`.
  Cross-library sealing back-reference invalidation lands in Sprint 29.
- **:closed: `next-method` calling convention** — Sprint 12 → Sprint 14
  (landed). Implemented via a thread-local stack of method-chain
  frames; `nod_dispatch` pushes a frame when 2+ methods are
  applicable; `nod_next_method` walks it. Preserves Sprint 13's
  method-body ABI exactly — no implicit chain parameter.
- **:open: Default-init-function (`init-function: foo`)** — Sprint 12
  → Sprint 13. `SlotDefault::Function` is not in the runtime enum;
  Sprint 12 only supports literal-value defaults. Add the function
  branch once a fixture needs it.
- **:open: `define generic` parameter signatures** — Sprint 12 →
  Sprint 13. Sprint 12 treats `define generic` as a name declaration
  only; the parameter types are recorded in the AST but not used. The
  full signature-checking lands with Sprint 13's dispatch IR.
- **:open: Non-first-parameter specialisers on methods** — Sprint 12
  → Sprint 13. A method `define method foo (a :: <c1>, b :: <c2>)`
  is registered against the first parameter's class only. The second
  specialiser is parsed but silently ignored. Sprint 13's full
  multimethod dispatch wires it.
- **:open: Slot `class` / `each-subclass` / `virtual` allocations** —
  Sprint 12 → Sprint 13+. These slot allocations surface
  `LoweringError::UnsupportedSlotAllocation` today. Instance allocation
  covers the fixture-shaped uses; the rarer kinds wait for a fixture.
- **:open: User-defined `<C>`-typed temporaries don't narrow the
  type lattice** — Sprint 12 → Sprint 13. The DFM's `TypeEstimate`
  enum has no `Class(ClassId)` variant; a `let p = make(<point>, …)`
  binding registers as `TypeEstimate::Top`. The setter-assign path
  always emits `Dispatch` rather than direct `StoreSlot`, even when
  the receiver is statically a known user class. Sprint 13 grows the
  lattice; for now we eat the dispatch overhead.

## Sprint 11b (precise GC roots — spill-to-runtime-slots)

### Closed by Sprint 11b

- **:closed: Pinned young objects are promoted, not held in place** —
  Sprint 11 entry. Sprint 11b's precise roots eliminate the need for
  pinning in normal operation entirely. The conservative pinner
  (`Heap::pin_stack_range`) is opt-in only — no production path calls
  it. `gc_runs_without_conservative_pinning` asserts `last_pinned_objects
  == 0` across a 10K-allocation stress run; the dedicated
  `conservative_stack_pin_keeps_object_alive` test in `gc.rs` retains
  the path for explicit verification of the rewinding-pinned-objects
  branch.
- **:closed: JIT-side latent unsoundness across two allocations** —
  Sprint 11 entry (the `NCL_GC_FEEDBACK.md` §2 finding). Codegen now
  brackets every potentially-allocating `DirectCall` / `Call` /
  `Dispatch` with `nod_register_root(slot)` ... call ...
  `nod_unregister_root(slot)` pairs around an entry-block `alloca` per
  live pointer-shaped temp. After the call, codegen reloads from the
  slot and rewires the temp's SSA mapping. The collector walks
  `Heap::roots` (already wired in Sprint 11) and rewrites the slot's
  Word during evacuation. `jit_ir_brackets_second_make_with_register_root`
  asserts the IR shape; `allocation_across_gc_keeps_first_instance_readable`
  drives the runtime path with a forced GC between two `rust_make`
  calls.
- **:closed: JIT stub "safepoint poll"** — Sprint 11 entry. Sprint
  11b's spill-to-runtime-slots is functionally precise without any
  poll-and-park machinery; the GC runs synchronously inside
  `nod_make`'s heap allocation, observes the registered slots, and
  evacuates. The cooperative-park protocol is still future work
  (Sprint 11c / 28); single-threaded mutator semantics are fine until
  Dylan-side threads land.
- **:closed: Rust shims allocating without rooting their args** —
  Sprint 11 latent. `nod_make` and `rust_make` now use a `RootGuard`
  RAII wrapper to register each `(name, value)` Word kwarg as a root
  before the `Heap::alloc_object` call, and read the rooted values
  back when writing slots. Without this, a kwarg pointing into young
  would go stale if `alloc_object` triggered a minor GC mid-call.

### Opened by Sprint 11b (still deferred)

- **:open: Full `gc.statepoint` upgrade** — Sprint 11b → Sprint 11d / 19.
  Sprint 11b's spill-to-runtime-slots ships forced `alloca` slots for
  every live pointer-shaped temp at every allocating call site. LLVM
  can't keep these in registers across the call (the
  `register_root(ptr)` shim forces the address to escape). The full
  upgrade is `llvm.experimental.gc.statepoint` bundles per safe point
  with a collector-side stack-map decoder; the NCL stack-map decoder
  was lifted into `nod-runtime/src/stack_map.rs` during Sprint 11b
  and remains ready for that work. Performance gain:
  register-allocated temps survive across calls, no forced spill.
  Sprint 11c was originally scheduled to land this but took the
  surgical path instead — see the Sprint 11c section below.
- **:open: Per-block (or full SSA) liveness analysis** — Sprint 11b →
  Sprint 18 (DFM optimisation passes). The Sprint 11b pass is a
  simple per-block "def-index ≤ call-index < last-use-index"
  computation, with "escapes-block" used as the live-out
  approximation. A control-flow-aware backward dataflow analysis (the
  standard live-in/live-out fixpoint) would tighten the over-spilling
  on multi-block functions. Sprint 11b's `nod_dfm::liveness` module
  is structured to host the upgrade.
- **:open: Safepoint poll at loop back-edges** — Sprint 11b →
  Sprint 11d / Sprint 17 (whichever lands first). A JIT'd loop that
  doesn't allocate still doesn't yield to the collector. Sprint 11b's
  allocating-call brackets cover every current code-shape; loop-only
  constructs land with Sprint 17's `for` macro and need the back-edge
  poll added then.
- **:closed: Multi-threaded mutator + cooperative park (mutex-shaped)** —
  Sprint 11b → reframed by Sprint 11c. Sprint 11c removed the
  `Heap::roots` mutex entirely; the root registry is now a thread-
  local `RefCell<Vec<*const Word>>`. The original entry stays
  conceptually open (see Sprint 11c entries below).
- **:open: Entry-block alloca pool is unbounded** — Sprint 11b →
  Sprint 11d cleanup. `safepoint_slot_pool` grows monotonically per
  function as new peak live-set sizes are observed. The pool isn't
  freed between calls in the same function (intentional — slots are
  reused), but a function with N>>0 allocating calls allocates O(N)
  stack slots. LLVM's mem2reg coalesces these for most cases, but the
  cleaner approach (one slot per allocating call) waits for the
  Sprint 11d / 19 statepoint upgrade.
- **:open: `Top` / `Bottom` over-protection** — Sprint 11b → Sprint
  13 (richer `TypeEstimate`). `TypeEstimate::Top` includes both
  pointer-shaped values AND `Top`-typed fixnums (e.g. a `let n = 1`
  where the type estimate lattice can't prove `Integer`). The
  liveness pass conservatively treats every `Top` as
  pointer-shaped — over-spilling but always sound. Sprint 13's
  user-class type narrowing tightens the lattice.
- **:open: `pin_stack_range` retirement** — Sprint 11b → Sprint 11d.
  Sprint 11b keeps the conservative pinner alive behind its `unsafe`
  signature for the dedicated GC test in `gc.rs`; production code
  doesn't call it. Once the `gc.statepoint` upgrade lands, the
  conservative path can be removed entirely (or kept behind a
  `cfg(feature = "conservative-fallback")` if a debug build mode wants
  it).

## Sprint 11c (lock-free root registry)

### Closed by Sprint 11c

- **:closed: `Heap::roots` Mutex on every register/unregister** —
  Sprint 11b entry. The root registry is now a thread-local
  `RefCell<Vec<*const Word>>` (see `heap.rs` `ROOT_STACK`); the
  Sprint 11c shim path also bypasses `with_literal_pool`'s mutex.
  Hot-path cost dropped from ~80 ns (two mutex acquisitions + push)
  to ~5-10 ns (one TLS lookup + Vec push). The new
  `lock_free_roots_no_mutex_acquisition` smoke test completes 1M
  register/unregister pairs in well under 500ms (~100ms release,
  ~330ms debug).
- **:closed: Sprint 16's 1.06× sealing speedup baseline mystery** —
  the dominant cost in the Richards-shape bench was indeed the
  per-call mutex, as theorised. Sprint 11c lifts the measured ratio
  from 1.06× to ~1.37-1.40× by removing it; both variants got
  ~2-4× faster end-to-end. The remaining gap to the brief's 5×
  target is documented under the Sprint 16 entry above.

### Opened by Sprint 11c (still deferred)

- **:open: Multi-threaded mutator + per-thread root registries
  enumerable by the collector** — Sprint 11c → Sprint 28. The
  thread-local design assumes single-threaded mutation. Sprint 28's
  threads library will need (a) per-thread root stacks (already the
  case — `thread_local!`), and (b) a mechanism for the collector to
  enumerate roots across all parked mutator threads. The current
  collector reads only the calling thread's local stack via
  `snapshot_roots`. Likely shape: register each mutator thread in a
  global `Mutex<Vec<*const RootStack>>`, walk the list at GC time
  with the safepoint-park protocol holding all threads still.
- **:open: `gc.statepoint` precise roots — eliminates per-call
  register_root entirely** — Sprint 11b → Sprint 11d / Sprint 19.
  The thread-local registry is much faster than the mutex, but
  every potentially-allocating JIT call still pays a function-call
  + Vec::push + Vec::pop pair. The full statepoint upgrade replaces
  these with a single LLVM intrinsic at the safe point, and the
  collector decodes the stack map. The stack-map decoder is already
  lifted (`nod-runtime/src/stack_map.rs`); the compiler-side emission
  is the remaining work.
- **:open: Single-threaded thread-confinement assertion deferred to
  Sprint 28** — Sprint 11c → Sprint 28. The brief asked for a
  `OnceLock<ThreadId>` debug-assert capturing the first runtime-init
  thread. Implementation deferred because the Rust test harness
  spawns one OS thread per `#[test]` (even with `#[serial]`, which
  only serialises ORDER, not threads), making a process-wide thread
  assertion fire on the second test. The thread-local design is
  self-enforcing for single-threaded mutation; Sprint 28 grows the
  global root registry described above and the assertion becomes
  superfluous.

## Sprint 13 (full multimethod dispatch + inline caches)

### Closed by Sprint 13

- **:closed: Inline caches + monomorphic-then-polymorphic dispatch** —
  Sprint 12 entry. Sprint 13 ships the full inline-cache machinery:
  every `Computation::Dispatch` call site gets a per-site `CacheSlot`
  (six `AtomicU64`s in the static area), the JIT-emitted IR loads
  the cache fields with monotonic atomics, compares against the
  receiver's class id + the generic's current generation, and either
  fast-path direct-calls the cached method or falls through to
  `nod_dispatch`. The slow-path shim writes the cache back. Hit/miss
  counters are bumped inline (fast path) and inside `nod_dispatch`
  (slow path); `dump_dispatch()` surfaces them.
- **:closed: Non-first-parameter specialisers on methods** — Sprint 12
  entry. `MethodRegistration` now carries `specialisers: Vec<ClassId>`
  (one per required parameter); `lower_method_item` walks every
  parameter and records its declared class (defaulting to `<object>`
  for unannotated params). `lookup_method` consults the full vector
  with the argument-major CPL-driven specificity rule.
- **:closed: `define generic` parameter signatures** — Sprint 12 entry.
  Closed indirectly: the runtime now uses the full specialiser list
  on every method, and `define generic`'s parameter types still
  surface as informational only (the matching machinery is on each
  `define method`, not on the bare generic declaration). Full
  signature-validation against the generic remains as future work
  (Sprint 17+ when conditions can carry diagnostics).

### Opened by Sprint 13 (still deferred)

- **:open: Polymorphic inline caches (PIC) for 2–4 receivers** —
  Sprint 13 → Sprint 18+. The cache slot holds ONE receiver class.
  Calls that flip between 2–3 receiver classes hit the slow path
  every time. A polymorphic cache with a small bounded array (the
  Self / Smalltalk / V8 design) is the right next step once the
  Sprint 16 Richards subset is up; the cache-slot struct can grow
  without breaking the IR shape.
- **:closed: Sealed-direct call lowering** — Sprint 13 → Sprint 15
  (landed). The Sprint 15 dispatch resolver rewrites
  `Computation::Dispatch` to `Computation::DirectCall` (single
  applicable method) or `Computation::SealedDirectCall` (2+
  applicable methods + chain preamble) when sealing facts plus the
  type-estimate lattice permit. Verified by 17 tests in
  `tests/nod-tests/tests/sealing.rs`. Sprint 13's inline cache is
  the fallback path for sites the resolver can't close.
- **:open: JIT-emitted `add-method` via `nod_add_method`** — Sprint 13
  → optional. Sprint 13 ships the `nod_add_method` C-ABI shim and
  registers it with the JIT engine, but the production lowering path
  (Sprint 12's Rust-side `register_methods` after `Jit::add_module`)
  still does the work. Lowering an in-JIT `define method` body that
  emits `nod_add_method(...)` at JIT time is a polish item — no
  current fixture exercises it.
- **:open: Variadic dispatch above 8 args** — Sprint 13 → Sprint 23
  (c-ffi). `nod_dispatch` is fixed-arity at 8 to match `nod_make`'s
  shape. True variadic calling-convention dispatch lifts the cap.
- **:open: Hit / miss counters are atomic-relaxed; perf-critical** —
  Sprint 13 → Sprint 18+. Every fast-path call does an
  `atomicrmw add` on the hit counter, which serialises on the
  cache-coherent bus. Release builds may drop these or shift to a
  per-CPU local counter once profiling shows the cost.
- **:open: `compute-applicable-methods` / full MOP** — carry-over
  from Sprint 12. Sprint 13's dispatch resolves to a single method
  per call; method combinations + before/after/around methods are
  still Sprint 17+ work.
- **:open: `<ambiguous-methods-error>` / `<no-applicable-methods-error>`
  signalled rather than panicked** — Sprint 13 → Sprint 19. Sprint
  13's runtime panics with a structured message; the surface
  visible to Dylan code today is process abort. Sprint 19 turns
  these into properly-signalled conditions.
- **:open: Cache fast-path branch-prediction hints** — Sprint 13 →
  Sprint 18+. The cache-hit branch is taken on the steady state;
  LLVM doesn't know that. A `llvm.expect` annotation on the
  conditional would let the back-end emit the fast path as the
  fall-through. Cosmetic until profiling.
- **:closed: `next-method` calling convention** — carry-over from
  Sprint 12 → closed in Sprint 14. `nod_dispatch` now calls
  `lookup_applicable_methods` (full sorted chain, not just the
  winner) and pushes a thread-local frame with the chain tail before
  invoking the head. `nod_next_method` peeks the frame and walks
  forward. See Sprint 14 closed list for details.

## Sprint 14 (multiple inheritance + slot layout + `next-method`)

### Closed by Sprint 14

- **:closed: Multiple inheritance + indirect slot lookup** — Sprint 12
  entry. Sprint 14 lifts the `MultipleInheritanceNotSupported` gate;
  `register_class` now resolves every direct super to a `ClassId`, runs
  C3 over the parent CPLs, merges parent slots in declaration order
  (the "most-specific-first append" policy), and registers the new
  class via `nod_runtime::register_mi_user_class`. The Sprint-14
  insight from the brief is that Sprint 13's per-class dispatch
  obviates a runtime "indirect slot lookup": every concrete class
  whose inherited slot has shifted offset gets a generated **override
  accessor** registered on the slot's generic specialised to that
  class. Dispatch picks the right method per receiver. Fixed-offset
  inherited slots (offset matches the defining parent's) get NO
  override — the parent's accessor works as-is. `ClassMetadata` grew
  `parents: Vec<ClassId>` and `slot_origin: Vec<ClassId>` to support
  this; the legacy `parent: Option<ClassId>` field is the first
  parent (back-compat for Sprint 12 callers).
- **:closed: `next-method` calling convention** — Sprint 12 / Sprint 13
  carry-over. Implemented via a thread-local stack of method-chain
  frames maintained in `nod-runtime::dispatch`. `nod_dispatch` pushes
  a frame (recording the args + the tail of the applicable-method
  list, most-specific first) when 2+ methods are applicable; calls
  the head; pops on return (via an RAII drop-guard so panics balance
  too). `next-method()` lowers to a JIT call into the runtime shim
  `nod_next_method`, which peeks the top frame, pops the next method,
  and re-invokes with the recorded args. `next-method?()` lowers to
  `nod_has_next_method`. This design preserves Sprint 13's
  `extern "C" fn(u64, ..., u64) -> u64` method-body ABI verbatim —
  no implicit chain parameter — so all 13 dispatch tests, 15 classes
  tests, and 13 gc_precise tests stay green untouched.

### Acceptance deviation

- None — the Sprint 14 brief's acceptance items all run end-to-end.

### Opened by Sprint 14 (still deferred)

- **:open: Polymorphic inline caches for overridden slot accessors** —
  Sprint 14 → Sprint 18. When an MI subclass generates an override
  accessor, the slot's generic now has 2+ methods. The Sprint 13
  monomorphic inline cache hits the slow path every time the
  receiver class flips between the parent and the subclass. A small
  PIC (2–4 entries) is the right fix; the cache-slot struct can grow
  without breaking the IR shape. Same deferred entry as Sprint 13's
  open list but with a concrete fixture now that MI is real.
- **:open: `next-method` with explicit arguments** — Sprint 14 →
  Sprint 17. The Sprint 14 lowering rejects
  `(next-method x y)` with a structured `Unsupported` diagnostic and
  forwards the parent method's args verbatim for the no-args form.
  Explicit-args `next-method` is a Dylan macro form that lands with
  the macro expander.
- **:open: Sealed-class redefinition checks for MI subclasses** —
  Sprint 14 → Sprint 15. The Sprint 12 sealed-class checks deferred
  to Sprint 15 already cover the SI shape; the MI shape adds the
  question of "is a multi-parent subclass of a sealed class still
  legal at all" which Sprint 15's sealing analysis must answer.
- **:open: Diamond `make` keyword conflict resolution** — Sprint 14 →
  unscoped. When two parents define init-keywords for slots with the
  same name (impossible with the SlotConflict gate, but possible
  with same-name same-origin-class diamonds), the Sprint 14 layout
  picks the first-parent's defaults. Document and revisit if a
  fixture forces a different resolution.
- **:open: `<no-next-method-error>` as a real signal** — Sprint 14 →
  Sprint 19. `nod_next_method` panics with a structured message
  containing `<no-next-method-error>` when the chain is exhausted.
  Sprint 19 turns this into a Dylan-signalled condition routed
  through the handler chain.
- **:open: `next-method` chain frames live across one dispatch** —
  Sprint 14 → unscoped. Method bodies that capture `next-method` as
  a closure for use AFTER the body returns would observe a popped
  frame and either panic or read the wrong chain. Dylan's semantics
  forbid this (the chain has dynamic extent), so the Sprint 14 design
  is correct under the language spec. If a future fixture wants to
  capture next-method first-class, the chain frame's representation
  needs to grow lifetime tracking.
- **:open: MI override accessor registration repeats per inherited
  slot** — Sprint 14 → cosmetic. Each inherited slot whose offset
  shifts generates one `<C>-override-getter-x` and one
  `<C>-override-setter-x`. For very wide MI hierarchies the number
  of override accessors grows linearly with `inherited_slot_count`
  per concrete class. Acceptable until Sprint 18's library-merge
  optimisation surfaces a problem.

## Sprint 15 (sealing analysis + dispatch resolution)

- **:open: Cross-library sealing back-reference invalidation** —
  Sprint 15 → Sprint 29 (library-merge optimisation). Sprint 15
  records `(call_site_id, generic_name, recorded_generation)` for
  every resolved Dispatch in
  `nod_runtime::resolved_dispatch_snapshot()`. Sprint 29 consults
  this index to invalidate sealed-direct sites when a cross-library
  redefinition advances the generic's generation past the recorded
  value. Sprint 15 only populates; no reader yet.
- **:open: `instance?` else-branch narrowing** — Sprint 15 → v2.
  The else-branch sees "not `<C>`", a negation requiring intersection
  types / co-typed-sets in the lattice. Sprint 15 over-conservatively
  skips narrowing on the else-branch (sound — matches spec 15 §9.2).
  Lighting this up needs a richer lattice.
- **:open: Inlining sealed-direct call bodies** — Sprint 15 →
  Sprint 18. Sprint 15's rewrite goes through a function-pointer
  call to the resolved method body symbol; the JIT engine resolves
  the symbol at link time. Full inlining of the body into the caller
  is Sprint 18 optimiser work.
- **:open: `define inline` methods + sealing combination** —
  Sprint 15 → Sprint 18. Sprint 04 captures the `inline` /
  `not-inline` modifiers; Sprint 15 reads but doesn't act. The body
  still goes through a direct call; inlining is Sprint 18's job.
- **:open: PIC bichotomy for almost-resolved cases** — Sprint 15 →
  Sprint 18. When two methods are both guaranteed applicable but
  neither is more specific (true ambiguity within the closure), the
  resolver could emit `if class == A: call M1 else: call M2`
  instead of falling back to Dispatch. That's a Sprint 18 PIC
  optimisation; Sprint 15 leaves the call as Dispatch.
- **:open: `TypeEstimate::Singleton(Word)` lattice variant
  unpopulated** — Sprint 15 → Sprint 17 / 19. The variant is
  defined; conditions where it'd matter (`if x == #f then …`) need
  pattern recognition in the analyser. Sprint 17 macros + Sprint 19
  conditions revisit.
- **:open: `define sealed method` (method-level sealing)** —
  Sprint 15 → revisit when a fixture exercises it. Dylan allows
  `define sealed method` to mark a single method against override;
  Sprint 15 parses the modifier but doesn't act.
- **:open: `define sealed domain` source-syntax parsing** —
  Sprint 04 / Sprint 15 → Sprint 04 follow-up. Sprint 04's
  `parse_define_other` consumes the head paren list (the specialiser
  tuple `(<A>, <B>)`) silently before capturing the body, so the
  specialiser fragments don't make it into
  `Item::DefineOther::body_fragments`. Sprint 15 installs sealed
  domains via the runtime API (`GenericFunction::register_sealed_domain`)
  for tests + REPL; full source-syntax support needs Sprint 04 to
  preserve the head paren as a fragment.
- **:open: `SealedDirectCall` panic-unwind chain-frame leak** —
  Sprint 15 → Sprint 19. The codegen-side `nod_pop_sealed_chain_frame`
  call runs on the success path only. A panic-unwind from inside
  the method body would skip the pop and leave a stale frame on the
  thread-local stack. Sprint 19 wires structured unwinding via
  `nod_resume` / cleanup landing pads; for Sprint 15 the runtime
  RAII `ChainFrameGuard` discipline from `nod_dispatch` isn't
  replicated at the JIT call site.
- **:open: Sealed-direct lattice join doesn't compute CPL-common
  ancestor** — Sprint 15 → Sprint 18. Per spec 15 §4, two distinct
  `Class(C1)` / `Class(C2)` estimates joined at an if-merge widen
  to `Top` in `TypeEstimate::join`. A richer join that walks both
  CPLs and returns the closest common ancestor is the right next
  step; soundness already holds (over-conservative join is safe).

## Sprint 16 (Richards-shape headline benchmark + `<pair>` / `<list>`)

- **:open: Upstream `simple-richards.dylan` doesn't compile yet** —
  Sprint 16 → Sprint 17–18 (statement macros). The 438-line
  `opendylan-tests/sources/testing/benchmarks/richards/simple-richards.dylan`
  fixture uses several forms NewOpenDylan doesn't lower yet: `while` /
  `until` loops (Sprint 06 deferred — `Statement::{While, Until, Block,
  For, Local}` route through `LoweringError::Unsupported`), `define
  variable` (Sprint 06 deferred), `<vector>` constructed with
  `make(<vector>, size: N, fill: x)` (Sprint 10's
  `<simple-object-vector>` constructor doesn't accept `size:` / `fill:`
  kwargs), and statement-macros (`for (…) end`, `with-*`). The Sprint 16
  fixture `richards-shape.dylan` ports the dispatch architecture (sealed
  task hierarchy + sealed multimethod) without these forms; the full
  upstream port lands once Sprint 17–18's macros + collection
  constructors close the gaps.
- **:closed: 5× speedup target — dropped as a sprint-acceptance gate.**
  Sprint 16's original brief asked for ≥ 5× speedup; project policy
  (2026-05-18) explicitly drops perf ratios as gates at this stage and
  reframes them as a trajectory tracked in `bench/richards.md`. The
  bench test asserts `ratio >= 0.95` only — a regression guard against
  re-introducing dispatch overhead, mode-agnostic. The 5× target was
  always achievable only after Sprint 18 (LLVM optimisation passes,
  cross-function inlining within the JIT module) and Sprint 11d/19
  (`gc.statepoint` precise roots eliminating per-call register/
  unregister); both will naturally land their contributions. See
  `feedback_correctness_before_perf.md` in user memory for the
  framing rule.
- **:open: Perf ratio history tracking in `bench/richards.md`.** Each
  measurement run appends a dated row (date, sprint, build mode,
  sealed/open ms, ratio, notes). The History table starts with the
  Sprint 16 baseline (1.06×) and the Sprint 11c lock-free measurement
  (1.39× release / 1.09× debug). Future sprints that move the ratio
  (Sprint 11d, Sprint 18) add their own rows so the trajectory is
  observable.
- **:open: `<pair>` is not yet hashable / equal-comparable beyond
  identity** — Sprint 16 → Sprint 17+. The Sprint 16 runtime registers
  `<pair>` as seed `ClassId::PAIR` with `head` / `tail` slots and the
  data-driven scanner walks both, but `==` against a pair returns
  identity-only — pairwise equality lands once `=` is generic.
- **:open: `<pair>` has no Dylan-source class definition** — Sprint 16
  → Sprint 17+. The runtime carries it as a seed class registered at
  startup; the Dylan-side `pair` / `head` / `tail` / `empty?` / `nil`
  identifiers are wired as compiler builtins (synthetic `%pair-*`
  callees recognised by `nod-sema::lower` and codegen'd as direct calls
  into runtime shims). Re-implementing `<pair>` in Dylan source via
  `define class <pair> (<list>) slot head; slot tail end` waits for the
  `<list>` abstract-class hierarchy + collection protocol.
- **:open: Bench measurement uses a single warmup pass + one timed
  run** — Sprint 16 → Sprint 18+. No statistical rigor, no warmup
  iteration count knob, no run-to-run variance reporting. Sprint 18 can
  promote to `criterion`-style measurement with histogram output.
- **:open: `<task>` and friends redefine fresh class IDs on every
  `_reset_user_classes_for_tests` invocation** — Sprint 16 → Sprint 28+
  (lazy class migration). The reset helper drops user-class entries
  from the registry but the pinned `ClassMetadata` allocations stay in
  the static area. Re-running a fixture mints fresh ids; obsolete ids
  are orphaned but not freed. Tolerable while user-class counts stay
  small; Sprint 28+'s class redefinition story replaces this.

## Carry-over from Sprint 17 (macro expander — pattern matching engine)

- **:closed: `define macro` body parsing (template + pattern)** —
  Sprint 04 → Sprint 17. Sprint 04 captured `body_fragments`
  verbatim; Sprint 17 parses them into `MacroDef::rules` with
  `PatternElem` / `TemplateElem` trees, registers them in a
  `MacroTable`, and rewrites recognised call sites before lowering.
- **:closed: Multi-rule macros + first-match selection** —
  Sprint 17 → Sprint 18. `parse_macro_def` now accepts multiple
  `{ pattern } => { template }` clauses; `expand_one` tries them
  left-to-right and picks the first match. A new
  `MacroError::NoApplicableRule` is raised when every rule fails.
  The legacy `MacroError::MultipleRulesNotSupported` variant is
  retained for source compatibility but is unreachable from the
  engine itself.
- **:open: Auxiliary `rule` clauses inside `define macro`** —
  Sprint 17 → Sprint 19. Kernel-library macros (`for`, `case`,
  `select`) use `rule` sub-clauses for the `clause` taxonomy;
  multi-rule + first-match (Sprint 18) doesn't fully replace
  auxiliary rules — the `clause` syntax inside a brace pattern is
  still unparsed.
- **:closed: Statement-position macro recognition (call-shape)** —
  Sprint 04 → Sprint 17 → Sprint 18. The matcher already worked
  on `Expr::Call { callee: Ident(name), … }` shaped call sites at
  any position (including `Statement::Expr(Call(…))`). Sprint 18
  documents that this is the supported statement-position form;
  the bare-keyword surface (`for-range (i from 1 to 10) body end`
  with its own `end`) needs the Sprint 19 statement-fragment
  pre-pass — it's tracked under "Full upstream `for` macro" below.
- **:open: `with-*` statement macros** — Sprint 17 → Sprint 19.
  `with-open-file` / `with-lock` / `printing-logical-block` etc.
  need `cleanup` semantics from Sprint 19's NLX/condition work;
  the pattern-matching side is ready (statement-position macros
  expand fine), but the lowering target doesn't exist yet.
- **:closed: Pattern-variable taxonomy widened** — Sprint 17 →
  Sprint 18. `PatternKind` now exposes `Variable`, `MacroArg`,
  `ParameterList`, `Constraint` in addition to the Sprint 17
  `Expression` / `Name` / `Body`. The new kinds match minimally
  (e.g. `Variable` accepts `Ident` and `Ident :: <type>`;
  `MacroArg` aliases `Expression`; `Constraint` is recognised but
  the constraint expression isn't evaluated yet — Sprint 19).
- **:open: Definition macros that expand into `define foo …`
  forms** — Sprint 17 → Sprint 25. The Sprint 18 expander rewrites
  `Expr::*` shapes only; `Item::DefineOther` (e.g. `define table`,
  `define inline function`) stays unrewritten because the
  expansion engine doesn't yet promote a substituted fragment list
  back into the `Item::DefineXxx` family. Sprint 25's stdlib port
  needs this; Sprint 18 keeps it scoped out.
- **:open: Cross-file / cross-module macro use** — Sprint 17 →
  Sprint 19 (depends on Sprint 05 module-graph resolution
  landing). `expand_module` assumes the macro definition and the
  call site share the same `SourceMap` / file; macros imported
  from another module aren't reachable to `collect_macros`.
- **:open: Full upstream `for` macro with `from`/`to`/`by`/`above`/
  `below`/`then` clauses** — Sprint 17 → Sprint 25 (kernel library
  port). Sprint 18 ships a SIMPLER `for-range(var, start, end,
  body)` call-shape macro in `stdlib-min.dylan` /
  `macro-for-range.dylan` to demonstrate the lowering. The
  upstream `for (i from 1 to 10) body end` shape is a heroic
  macro that needs auxiliary `rule` clauses + statement-position
  parsing of bare keywords with their own `end`; both deferred.
- **:open: `case` / `cond` macros** — Sprint 17 → Sprint 25. Need
  auxiliary `rule` clauses for the arm-by-arm patterns. Sprint 18's
  multi-rule selection doesn't substitute for the inner-arm
  taxonomy. Migrate from hardcoded `Expr::Case` here.
- **:open: `Statement::For` lowering** — Sprint 17 → Sprint 25.
  `Statement::For` errors as `Unsupported`; the upstream `for`
  macro will expand into `let` + `while` via the engine; until then
  hand-written `for (i from 1 to 10) … end` rejects.
- **:open: Expansion-trail-aware diagnostic formatter** —
  Sprint 17 → Sprint 19. Origin records track template-vs-call
  provenance per fragment, and `rewrite_spans_expr` anchors AST
  spans at their original source location. The error-formatter
  that walks the chain (`error: x at <template>; expanded from
  <call>`) lands with Sprint 19 conditions.
- **:closed: Hygiene policy refinement** — Sprint 17 → Sprint 18.
  Sprint 17's "rename every template Ident not in pattern vars" was
  over-conservative. Sprint 18 refines: only Idents in BINDING
  POSITION inside the template (the binder of a `let`, the param
  names inside a `method` / `local method` arg list) get a
  per-expansion suffix. Reference-position Idents flow through
  unchanged so user-visible names (`if`, `else`, type names, etc.)
  resolve against the surrounding scope. The
  `collect_template_binders` walk implements the conservative rule
  set; widen when a fixture exercises a corner case.
- **:open: Paren-less / bare-keyword macro call surface
  (`unless 1 = 0 42 end`, `for-range (i from 1 to 10) body end`,
  `with-open-file (s = path) … end`, …)** — Sprint 17 →
  Sprint 19. The current parser AST-ifies call sites eagerly, so a
  bare-keyword statement-macro with its own `end` doesn't form an
  AST node the engine can recognise. Sprint 18 ships the
  call-shape statement-position path (the engine sees a
  `Statement::Expr(Call(Ident, args))` and rewrites it in place);
  Sprint 19 needs to add a fragment-pre-pass that consumes the
  bare-keyword surface from the token stream before AST-ifying.
- **:open: Per-call-site expansion-count budget** — Sprint 17 →
  Sprint 19. `DEFAULT_EXPANSION_BUDGET = 256` is defined in
  `nod-macro` but the depth limit (`DEFAULT_DEPTH_LIMIT = 64`) is
  what actually guards termination in v1. Add the per-site
  counter when a real fixture exercises the difference.

## Carry-over from Sprint 18 (macro engine extensions + while/until lowering)

- **:open: Bare-keyword statement-macro surface** — Sprint 18 →
  Sprint 19. Sprint 18 ships call-shape statement-position macros
  (the macro is invoked as `Ident(args)` at a statement). The
  upstream `for-range (i from 1 to 10) body end` bare-keyword
  form — with its own opening keyword, paren-clauses, free body
  statements, and matching `end` — needs a fragment-pre-pass that
  consumes statement-macro tokens before the AST-ifying parser
  runs. Sprint 19 adds it alongside the NLX block parsing.
- **:open: Migration of hardcoded `Expr::Unless` / `Expr::Case` /
  `Expr::Begin` to stdlib macros** — Sprint 18 → Sprint 25.
  Per the `feedback_dylan_lang_defined_by_macros.md` policy,
  Sprint 03's hardcoded AST forms exist only until the macro
  engine + stdlib catch up. Sprint 18 lights the engine; Sprint 25's
  stdlib port migrates `unless`, `case`, `cond`, `select`, `when`
  out of the parser and into Dylan-defined macros, then strips
  the AST variants. The Sprint 17/18 `Expr::Unless`-shape macro
  recognition (`macro_call_name` returns `"unless"` for the AST
  variant) is a transitional bridge that's removed in Sprint 25.
- **:open: Stdlib-min auto-loaded at compiler startup** —
  Sprint 18 → Sprint 25. The Sprint 18 `stdlib-min.dylan` fixture
  lives under `tests/nod-tests/fixtures/`; the "real" `nod-dylan/
  stdlib.dylan` that auto-loads before user code lands when
  Sprint 25 wires the loader to seed the macro table from a
  pinned source file.
- **:open: `for-range` upstream-fidelity gap** — Sprint 18 →
  Sprint 25. Sprint 18's `for-range(var, start, end, body)` takes
  four call-shape args. Upstream Dylan's `for (i from 1 to 10
  by 2 then i + 1 below n) body end` accepts the rich clause
  taxonomy. Sprint 25 ports the kernel `collection-macros.dylan`
  faithfully once auxiliary `rule` clauses + bare-keyword surface
  are in.
- **:open: Sprint 11b liveness pass is conservative across
  back-edges** — Sprint 11b → Sprint 18 retrospective. The
  per-block live-across-call analysis's `escapes_block` set
  already over-approximates correctly for loop bodies: a temp
  defined in the header block (e.g. the loop variable's phi)
  used inside the body escapes the header, so it's protected
  across every call inside the loop. Confirmed end-to-end via
  the Sprint 18 `for-range` fixture; refine when measurements
  demand.
- **:open: Multi-statement `?body` in expression-position
  expansions** — Sprint 18 → Sprint 19. The macro engine's body
  matcher handles trailing-literal followers (`?body:body end`)
  and binds multi-statement remainders correctly when fed raw
  fragment streams. But the resulting substitution is re-parsed
  as a single `Expr`, so an inline-template `?body` substituted
  into `begin ?body end` works (the `begin` collects multiple
  statements); free-standing `?body` substitution into a
  comma-separated argument list does not.
- **:open: Auxiliary `rule` clauses inside `define macro`** —
  Sprint 17 → Sprint 19. The Sprint 18 multi-rule selector
  handles top-level `{ pat } => { tmpl }; …` but doesn't parse
  the inner `rule` clause used by upstream's `case` and `for`.

## Carry-over from Sprint 19 (conditions, NLX, restart stubs)

- **:open: Full restart semantics** — Sprint 19 → Sprint 22. Class
  `<simple-restart>` exists and can be instantiated via
  `make-restart(name, description)`; `invoke-restart` is a panic
  stub. Sprint 22 lands the active-restart chain (parallel to the
  handler chain), `with-restart` / `restart-query`, restart inheritance
  through nested signals, and the full DRM restart protocol.
- **:open: `<simple-error>` / `<simple-warning>` MI parents** —
  Sprint 19 → Sprint 22. DRM defines `<simple-error>` as an MI
  subclass of `<simple-condition>` and `<error>`; we ship them as SI
  subclasses of `<error>` / `<warning>` respectively carrying their
  own `message` slot. Reason: avoids a slot-name conflict against the
  inherited `message` from `<simple-condition>` in Sprint 14's MI
  merge path. Sprint 22 will rationalise either by allowing
  same-origin slot dedup in the merge or by re-rooting the class
  hierarchy. As a consequence `is_subclass(<simple-warning>,
  <simple-condition>)` is false in Sprint 19; class identity through
  `<warning>` / `<error>` / `<condition>` still holds for the signal
  walker.
- **:open: `<no-next-method-error>` raise site** — Sprint 19 → Sprint 22.
  The class is seeded but `next-method` doesn't signal it when no next
  method exists; it currently returns `#f` (Sprint 14 behaviour).
  Sprint 22 will route `nod_next_method` through `nod_signal` with a
  freshly-constructed `<no-next-method-error>` when applicable.
- **:open: REPL `:handlers` meta-command** — Sprint 19 → Sprint 19.5
  (driver follow-up). The runtime side ships `handlers_report()` and
  `nod_walk_handlers_dump()`; the `nod-driver` REPL needs a
  meta-command wiring to call them. Likely 30 lines in
  `src/nod-driver/src/main.rs`'s REPL dispatcher. Independently
  ship-able from Sprint 19's headline acceptance.
- **:open: AOT-mode condition unwinding** — Sprint 19 → Sprint 28
  (AOT). The Sprint 19 NLX transport is `std::panic::panic_any` +
  `catch_unwind`. AOT builds (when they land) need a strategy that
  doesn't depend on Rust's panic runtime: either (a) install a
  Win64-SEH personality function so an `__except` filter catches the
  NLX, mirroring the M2NEW approach; or (b) keep the panic-based
  transport and statically link `std`'s panic runtime into AOT
  binaries (size cost, but minimal engineering). Decide at Sprint 28
  scoping.
- **:open: `nod-runtime` `_reset_user_classes_for_tests` + condition
  classes interaction** — Sprint 19 → ergonomics-only. The Sprint 19
  conditions registry caches `&'static ClassMetadata` pointers in a
  `OnceLock`; if a test calls `_reset_user_classes_for_tests` (Sprint
  12's helper that drops user-class registry entries while keeping the
  metadata pinned in the static area), the cached pointers become
  stale because `class_metadata_ptr` returns null for the dropped ids.
  Tests work around this by not resetting user classes when they
  exercise conditions. Sprint 22 (when conditions live in stdlib
  rather than the runtime seed table) makes this moot.
- **:open: `block (k)` `MAX_BLOCK_CAPTURED = 8`** — Sprint 19 → when
  it bites. Lowering errors out at lift time if a `block` form would
  capture more than 8 surrounding locals. Real Dylan code rarely hits
  this, but a real `define method` body with many locals around a
  `block` would. Two ways out: (a) raise the fixed limit, (b) pack
  captures into a heap-allocated environment object and pass a single
  pointer through the thunk-arg slot. (b) is the right answer
  long-term and aligns with the closure-environment work in Sprint 24.
- **:open: Handler chain as GC roots** — Sprint 19 → Sprint 11d
  (precise roots). The thread-local handler stack is a `Vec<HandlerFrame>`
  on the Rust heap; the `var_slot` mention in the brief was punted
  because the Sprint 19 lowering doesn't allocate explicit `var_slot`s
  — the handler's `var` is a normal SSA temp passed as an argument to
  the handler thunk. When precise stack roots land (Sprint 11d /
  `gc.statepoint`), the temp will be registered as a root through the
  normal codegen path. Until then, the in-flight condition Word is
  reachable through the thread's stack frame and gets pin-scanned by
  the conservative-scan fallback.

## Carry-over from Sprint 20 (forward iteration protocol + core collections)

- **:open: Dylan-side stdlib for collections** — Sprint 20 → Sprint 22.
  The spec's preferred path was to define `forward-iteration-protocol`,
  `size`, `element`, `element-setter`, `do`, `map`, `reduce`,
  `concatenate`, and the per-class FIP methods in
  `src/nod-dylan/dylan-sources/stdlib.dylan`. That file is still empty
  as of Sprint 19: the stdlib loader that folds it into the lowering
  pass before user code lowers doesn't exist yet (it's a Sprint 22
  task — the spec hints "Dylan-defined-by-macros direction"). Sprint
  20's collection ops live in `nod-runtime/src/collections.rs` as Rust
  APIs that mirror the sealed-Dylan-generic shape; when the loader is
  alive, the API surface can move into Dylan unchanged (each op is a
  pure function of its inputs). The class hierarchy is already
  registered as user classes, so Dylan-side `define method` on each
  concrete class will compose with what's there.
- **:open: `for-each` macro consuming FIP** — Sprint 20 → Sprint 22.
  Deferred because the macro would need first-class higher-order
  arguments (the closure inside `for (x in coll) body end` and the
  `iter-state` mutation chain) plumbed through the JIT, and Sprint 20's
  spec explicitly permits dropping the macro and exposing
  `do(method (x) ..., coll)` as the workaround. The runtime API
  (`collection_do` / `collection_reduce` / `collection_map`) carries
  the same semantics; landing the macro is one of the first stdlib
  pieces in Sprint 22.
- **:open: True multiple values for FIP return** — Sprint 20 →
  Sprint 22+. DRM specifies `forward-iteration-protocol` returns seven
  values; Sprint 20 bundles them in a heap-allocated
  `<iteration-state>` slot record because the IR / runtime have only
  TODO placeholders for `Values` / `BindExit` / `UnbindExit`. The
  bundled shape is a Sprint 20 acceptance — see
  `nod-runtime/src/collections.rs` top doc. When `nod-dfm` grows real
  multi-value support, the FIP signature can move back to the seven
  individual returns and `<iteration-state>` becomes vestigial.
- **:open: `<mutable-sequence>` MI parentage** — Sprint 20 → Sprint 22.
  DRM has `<mutable-sequence>` as a multiple-inheritance subclass of
  both `<mutable-collection>` and `<sequence>`. Sprint 20 registers it
  as a single-inheritance child of `<sequence>` only, dodging Sprint
  14's MI slot-merge risk (the parents are slot-less so the merge
  would succeed; SI is the conservative pick while the rest of the
  hierarchy beds in). Restore full MI parentage when the stdlib port
  lands — the C3 walk and is_subclass check are already MI-correct;
  only the registration shape needs to change.
- **:open: `<string>` as a `<sequence>`** — Sprint 20 → Sprint 21.
  Spec explicitly defers — `<string>` does not join the collection
  protocol in Sprint 20. Sprint 21 ("`<table>`, hashing, `<string>`
  collection conformance") owns the work; the FIP shape from Sprint 20
  generalises directly (the state is an integer index, advance bumps it,
  current-element reads `bytes[state]`).
- **:open: `<table>`, `<deque>`, `<vector>` (unbounded), limited
  collections** — Sprint 20 → Sprint 21 / v1.x. The Sprint 20 brief
  defers all of these explicitly; Sprint 21 ships `<table>` with
  hashing.
- **:open: Full DRM `for` clause matrix** — Sprint 20 → Sprint 22+.
  Numeric ranges (`for (i from 1 to 10)`), multiple parallel clauses,
  `until:` / `while:` / `finally:` ride on the Sprint 18 `for-range`
  macro shape. The full grammar is its own grammar tree; landing it
  needs the statement-fragment pre-pass that's still in motion for
  Sprint 19. Track alongside the `for-each` macro work.
- **:open: `:inspect` truncated-preview rendering for collections** —
  Sprint 20 → driver follow-up. The spec listed this as a Sprint 20
  deliverable but called out that deferring to a driver follow-up was
  fine. Today the driver prints `<simple-object-vector @ 0x…>` and
  similar; the preview should render the first N elements plus a total
  count, and `:inspect 0` / `:inspect 1` should walk into elements.
- **:open: `<list>` not re-parented to `<sequence>`** — Sprint 20 →
  Sprint 22. The seed `<empty-list>` (ClassId 10) and `<pair>`
  (ClassId 11) still have `<object>` as their direct parent in the
  seed table. Sprint 20 brief asked for re-parenting to `<sequence>`,
  but the seed table is a fixed `[SeedSpec; 12]` array — patching it
  would mean either flipping the seed-table CPL builder to consult
  user-class metadata (still bootstrapping at that point) or
  duplicating `<list>` as a user-class wrapper. Sprint 20 instead has
  `collection_size` / `collection_element` / `is_collection` /
  `forward_iteration_protocol` handle both seed list classes
  explicitly. The CPL chain remains `<pair>, <object>` rather than
  `<pair>, <list>, <sequence>, <collection>, <object>`. Sprint 22 (or
  the Sprint 25 kernel-library port) can introduce `<list>` as a real
  abstract class once `<empty-list>` and `<pair>` migrate out of the
  seed table.
- **:open: `iter-state` allocations per FIP start** — Sprint 20 →
  Sprint 22 (sealing-driven inlining). Each `collection_do` / `_reduce`
  / `_map` allocates one `<iteration-state>` instance on entry. The
  Sprint 15 dispatch resolver should let the JIT inline the FIP
  primitives once they're proper Dylan generics (Sprint 22 stdlib
  port); after that, `<iteration-state>` becomes an SSA scalar bundle
  and the allocation disappears. Sprint 20 doesn't attempt the
  optimisation — it just lands the protocol.
- **:open: `current-element-setter` slot in `<iteration-state>`** —
  Sprint 20 → when in-place `map!`/`replace-elements!` lands. The
  seventh DRM FIP value is a setter closure for mutable collections.
  Sprint 20's `make_iter_state` always writes `nil` there because
  `collection_map` allocates a fresh result rather than mutating in
  place; mutable in-place variants would need to populate the slot
  with a per-collection closure or method pointer. Track with the
  `for-each` macro work.
- **:closed: SOV / list / stretchy-vector JIT externs unused by current
  IR** — Sprint 20 → Sprint 20b. Wired in Sprint 20b's
  `LOWER_PRIMITIVE_TABLE` + `SPRINT_20B_PRIMITIVES` codegen +
  JIT-mapping path. The primitives are reachable from Dylan source
  as `%vector-size`, `%vector-element`, `%vector-element-setter`,
  `%stretchy-vector-size`, `%stretchy-vector-element`,
  `%stretchy-vector-element-setter`, `%stretchy-vector-push`,
  `%range-from`, `%range-to`, `%range-by`, `%make-range`,
  `%make-stretchy-vector`, plus the new `%collection-size` /
  `%collection-concatenate` / `%fip-*` family. `nod_make_sov_literal`
  is still unused; the `vector(...)` Dylan callable bring-up lands
  with the rest of the stdlib SOV surface in Sprint 21.

## Carry-over from Sprint 20b (stdlib loader + primitives)

- **:open: Full collection generics in stdlib.dylan** — Sprint 20b →
  Sprint 21. `reduce`, `map`, `do`, `element`, `element-setter` all
  stay as Rust APIs (`collection_reduce`, `collection_map`, …) because
  Sprint 20b can't yet thread first-class function values through the
  JIT ABI: the function argument to `reduce(f, init, c)` needs to be
  callable from inside the JIT'd Dylan body, which requires either
  (a) anonymous-method lifting to a top-level function plus a
  `<function>` Word wrapping the JIT'd address, or (b) a function-
  pointer extern shim invoked via `%apply-1`/`%apply-2`. Sprint 21
  picks one. The FIP primitives wired in Sprint 20b (`%fip-init` /
  `%fip-finished?` / `%fip-current-element` / `%fip-advance!`) cover
  the iteration-protocol surface today, and the two headline Sprint 20b
  acceptance tests (`reduce(\+, 0, range(from: 1, to: 100))` /
  `map(method (x) x * x end, #(1, 2, 3))`) are marked `#[ignore]`
  with this blocker as the reason. The Rust-API equivalents +
  the new `dylan_fip_reduce_range_one_to_one_hundred_is_5050`
  test (FIP-form, same machinery, no first-class function) cover
  the same code paths.
- **:open: Body-shaped macro calls in expression position** —
  Sprint 20b → Sprint 21. The `for-each` macro IS defined in
  `src/nod-dylan/dylan-sources/stdlib.dylan`, parsed, collected into
  the process-global macro table, and confirmed reachable by
  `dylan_stdlib_loader_registers_for_each_macro`. But the
  expression-level parser (`nod-reader/src/parser.rs`) can't yet
  recognise `for-each (x in c) body end` as a macro call — that
  syntax doesn't fit the `Expr::Call` shape the macro engine matches.
  Sprint 21 extends the parser to detect `<ident> (...) body end`
  patterns when `<ident>` resolves to a known macro name. Today's
  workaround: write the expansion target directly
  (`let s = %fip-init(c); until (%fip-finished?(s)) … end`); the
  `dylan_fip_until_loop_*` tests exercise this shape.
- **:open: Cross-module dispatch resolution against legacy
  `{generic}${specialisers}` body name** — Sprint 20b → Sprint 21. The
  codegen layer's fallback path (`find_method_body_ptr` extern
  declaration when the callee isn't local) works for `add_method_named`-
  registered methods. The OLDER `add_method` API (no body-fn-name
  stash) is unaffected because Sprint 12 / Sprint 13 always carry the
  body name through `MethodRegistration`. Watching for any sema path
  that calls bare `add_method` is a Sprint 21 audit item.
- **:open: Slot-accessor-based FIP methods in stdlib.dylan** —
  Sprint 20b → Sprint 21. The brief sketched
  `define sealed method forward-iteration-protocol (c :: <list>) …`
  in stdlib.dylan, reading `<iteration-state>` slots via `%`-prefixed
  names. Two blockers prevent landing it today: (1)
  `<iteration-state>` is registered via Rust
  `register_simple_user_class`, NOT a Dylan `define class`, so no
  slot-accessor methods are auto-generated; (2) the slot names
  `%state` / `%limit` / etc. would need lexer carve-outs to be
  distinguishable from a primitive-op call. Sprint 21 lights up both
  — either by adding a `define dylan-class` syntactic form that
  declares the Dylan-level shape of a Rust-registered class, or by
  moving the registration into stdlib.dylan with a `define class`
  declaration.
- **:closed: `define function` stdlib functions reachable from user
  code** — Sprint 20b → Sprint 20b. The loader rewrites every
  multi-arg `define function f (params)` to `define method f (params
  ... :: <object>)` so the call resolves via the process-global
  dispatch table. 0-arg `define function`s stay as direct-call
  top-level functions and aren't reachable from a separate JIT
  module; the loader takes the safe path here.

## Cross-sprint, infrastructure-shaped

- **:closed: `cargo clippy` blocked by agent sandbox** — Sprint 03 + 05
  agents both reported their sandbox refused clippy invocations.
  Resolved Sprint 12 retrospective by adding `Bash(cargo clippy:*)` and
  `PowerShell(cargo clippy:*)` to project-level `.claude/settings.json`;
  agents now invoke clippy without prompting.
- **:closed: `nod-od-suite` curated regression set** — Sprint 01 →
  Sprint 12 retrospective. Crate now hosts five hand-curated
  OpenDylan-flavoured fixtures (`fibonacci`, `euclid-gcd`, `even-rec`,
  `area-shapes`, `point-3d-sum`) covering recursion, mutual recursion,
  `mod`, single-dispatch over a shape hierarchy, and inherited slot
  access. Runner in `tests/run.rs` drives each through `nod-sema::
  run_function_to_i64`. Richards (Sprint 16) will land alongside the
  remaining iteration-protocol pieces.
