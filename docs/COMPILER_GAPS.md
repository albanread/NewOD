# Compiler gaps log

Living record of Dylan-language features or compiler bugs surfaced by
**dogfooding** — writing real Dylan to drive real Dylan tooling. The
mission of every dogfooding sprint (the IDE, the in-Dylan lexer, the
eventual in-Dylan parser/sema) is exactly to flush these out.

Each gap stays here until it ships a fix. Workarounds are recorded
verbatim so we can audit "still hacking around it?" at any time and
remove them once the underlying issue is closed.

## Format

```
## GAP-NNN — short title

* **Discovered**: sprint + commit SHA + file:line of the workaround.
* **Symptom**: minimal code that fails / unexpected behaviour.
* **Workaround**: what the dogfooder did instead (still in tree).
* **Planned fix**: what the compiler should ultimately do.
* **Scope**: rough size estimate (small / medium / large).
* **Status**: open | in-progress | fixed in SHA.
```

Sort by ID. New gaps append. Don't renumber.

---

## GAP-001 — No `<stream>` class in the runtime

* **Discovered**: Sprint 45a, commit `29e1040`,
  `tests/nod-tests/fixtures/dylan-lexer.dylan` around line 130
  (the `print-token` generic).
* **Symptom**: wanted to write
  `define generic print-token (t :: <token>, source :: <byte-string>, stream :: <stream>) => ()`
  so each token class can print itself to a stream (the canonical
  Dylan I/O abstraction). No stream classes existed in stdlib.
* **Workaround**: `print-token-to-string` shape. Tokens knew how to
  render themselves to a byte-string; `dump-tokens` concatenated the
  results. Retired with the fix.
* **Fix**: added a minimum-viable stream surface to
  `src/nod-dylan/dylan-sources/stdlib.dylan`:
  - `<stream>` abstract base class
  - `<string-stream> (<stream>)` concrete subclass with a single
    `stream-bytes :: <stretchy-vector>` slot
  - `make-string-stream() => <string-stream>` constructor
  - `write-byte(stream, b)` / `write-string(stream, s)` /
    `as-byte-string(stream)` generics + methods specialised on
    `<string-stream>`
  
  The write-side methods append bytes into the stretchy-vector;
  `as-byte-string` materialises a fresh `<byte-string>` of the
  right size and copies the accumulated bytes in. Future
  subclasses (`<file-stream>`, `<console-stream>`, `<input-stream>`)
  slot in via the same generics.

  End-to-end smoke test confirms `make-string-stream() →
  write-string + write-byte → as-byte-string` round-trips byte-exact
  through the AOT pipeline.

  This is also the **first time stdlib defines a class user code
  uses** — earlier classes like `<rope>` were always in the user's
  own AST. The class-resolution path
  (`find_class_id_by_name(name)` at `lower.rs:4317`) already
  worked for this — the stdlib lowering registers classes via
  `register_simple_user_class` and the user-side lookup falls
  through to the runtime registry. No compiler change needed,
  just the stdlib addition.
* **Regression test**: `tests/nod-tests/tests/sema.rs::
  gap_001_string_stream_round_trips`.
* **Scope**: small. ~70 lines of Dylan in stdlib.dylan.
* **Status**: **fixed in SHA `a689fcd`** (this commit). The full stream
  hierarchy (`<file-stream>`, `<input-stream>` for the parser, etc.)
  is its own future sprint when the IDE / Sprint 46+ parser need
  them.

## GAP-002 — `define constant` names don't resolve from function bodies

* **Discovered**: Sprint 45a, commit `29e1040`,
  `tests/nod-tests/fixtures/dylan-lexer.dylan` (the literal
  `1000000` appeared at three sites with comments).
* **Symptom**: a module-level `define constant $line-multiplier =
  1000000;` declaration is correctly parsed and lowered (as a
  zero-arg function returning the value), but `collect_top_level_names`
  in `nod-sema/src/lower.rs` only looked at `Item::DefineFunction`
  entries — never registered the constant in the name-resolution
  table. So bareword references from inside a `define function`
  body raised `LoweringError::UndefinedIdent` even though the
  constant was right there in scope.
* **Workaround**: the literal `1000000` was repeated at three sites
  in `offset-to-line-col-packed` / `unpack-line` / `unpack-col`.
  Retired with the fix.
* **Fix**: two changes in `nod-sema/src/lower.rs`:
  1. `collect_top_level_names` now also walks `Item::DefineConstant`
     and `Item::DefineVariable`, registering them with arity 0 and
     adding them to a new `TopNames::constants_and_variables` set.
  2. The `Expr::Ident` arm of `lower_expr` checks
     `is_constant_or_variable(name)` BEFORE the existing
     make-function-ref paths. When true, it emits a zero-arg
     `Computation::DirectCall` that evaluates the constant's body
     and returns its value — the right Dylan semantics (constants
     are values, not callable refs).
* **Regression test**: `tests/nod-tests/tests/sema.rs::
  gap_002_define_constant_resolves_from_function_body`.
* **Scope**: small. ~30 lines of sema.
* **Status**: **fixed in SHA `59e6f9f`**. `define variable` is a
  separate, deeper gap — see GAP-004.

## GAP-004 — `define variable` not lowered

* **Discovered**: Sprint 45a follow-up while fixing GAP-002 (this
  commit). The repro
  ```
  define variable *counter* = 41;
  define function main () => () *counter* := *counter* + 1; ... end;
  ```
  surfaces `unsupported [Span ...]: define variable not lowered in
  Sprint 06`. The `Item::DefineVariable` arm of the per-item
  lowering loop emits an `Unsupported` lowering error rather than
  generating a function body, so the variable's name is never bound
  to anything callable.
* **Symptom**: `define variable foo = expr;` fails to lower at all
  — fails BEFORE the GAP-002 name-resolution path is even reached.
* **Workaround**: avoid `define variable`. The lexer fixture used
  `define constant` exclusively. Retired with the fix.
* **Fix**: full `<cell>`-backed read/write/init pipeline in 7 steps:
  1. **Runtime storage** — `variable_cell_slot_addr(name) ->
     &'static AtomicU64` slot-allocator pattern (Sprint 38c shape,
     mutable variant) in `nod-runtime/src/lib.rs`. Slots hold the
     cell-pointer Word, registered as GC roots on first allocation
     so the cell itself stays reachable across GC cycles.
  2. **Runtime API** — `nod_aot_register_variable(name, name_len,
     init_fn_ptr)` (in `aot.rs`) calls the init function to compute
     the initial value, allocates a fresh `<cell>` via `nod_make_cell`,
     stores the cell pointer in the slot. `nod_var_get_by_name` /
     `nod_var_set_by_name` (in `closures.rs`) read/write through the
     slot lookup + cell deref.
  3. **Lower `Item::DefineVariable`** — emits THREE bodies: a
     `__init-<name>()` zero-arg function with the init expression,
     a getter `<name>()` that calls `nod_var_get_by_name`, and a
     setter `<name>-setter(v)` that calls `nod_var_set_by_name`.
  4. **Setter wiring** — `lower_assign` (lower.rs:4798) gained a
     module-variable branch: when the LHS resolves to a `define
     variable`, emit a DirectCall to `nod_var_set_by_name` with the
     interned variable name + RHS. `TopNames` split into separate
     `constants` and `variables` sets so assignment to a `define
     constant` correctly errors out.
  5. **AOT registration** — `LoweredModule` gained a `variables:
     Vec<VariableRegistration>` field; codegen emits one
     `nod_aot_register_variable(name, len, &__init-name)` call per
     variable inside `nod_aot_resolve_relocs` AFTER class/method/
     block registration (variables can call any registered function
     during init).
  6. **JIT path** — the JIT-side initialisation mirror runs after
     the engine materialises; calls each `__init-*` function and
     stores the result via `nod_var_set_by_name`. Symmetric with
     the AOT resolver.
  7. **GC discipline** — the cell pointer in the slot is reachable
     because the slot is registered as a heap root; the cell's
     `value` slot is `SlotType::Object` so the contained Word is
     traced via the existing Sprint 24 machinery.
* **Regression tests**:
  - `tests/nod-tests/tests/sema.rs::gap_004_define_variable_lowers_to_getter_and_init`
    — lowering-side check.
  - End-to-end smoke (manual): build `define variable *counter* = 41;`
    program, run, observe `initial = 41` → `*counter* := *counter* + 1`
    → `after-bump = 42` → `*counter* := 99` → `after-set = 99`.
    Verified byte-exact through the AOT EXE pipeline.
* **Scope (actual)**: medium-large. ~600 lines across nod-runtime,
  nod-sema, nod-llvm. 7 commits worth of independently-verifiable
  steps merged here into one for atomicity.
* **Status**: **fixed in SHA `74e6221`** (this commit). GAP-002's regression
  test still passes — constants stay immutable, variables are the
  only writable kind.

## GAP-005 — `if` without `else` arm refuses to lower

* **Discovered**: Sprint 45a rework (commit `1d32575`),
  `tests/nod-tests/fixtures/dylan-lexer.dylan` print-token method.
* **Symptom**: writing `if (cond) write-string(stream, "  ") end;`
  raised `unsupported [Span ...]: Sprint 06 lowers only
  if-expressions with an else arm`. Dylan supports the else-less
  form; the compiler rejected it.
* **Workaround**: explicit `else #f` arm. Retired with the fix.
* **Fix**: in `Expr::If` lowering, when `else_` is `None` synthesise
  an `Expr::Bool(span, false)` and pass it to `lower_if`. Semantically
  correct (Dylan: missing else returns `#f`). Same 3-block CFG, no
  special-case in lower_if.
* **Regression test**: `tests/nod-tests/tests/sema.rs::
  gap_005_if_without_else_lowers`.
* **Scope**: small. ~10 lines of sema.
* **Status**: **fixed in SHA `8e153b2`** (this commit). Note GAP-006 still
  applies if the synthesised else's `#f` doesn't shape-match the
  then-arm's last-expression type — see below.

## GAP-006 — void-returning calls in if-arms panic codegen

* **Discovered**: Sprint 45a rework (commit `1d32575`), print-token
  method using `if (cond) write-string(stream, "  ") end`.
* **Symptom**: codegen panics with `phi incoming temp defined` at
  `src/nod-llvm/src/codegen.rs:1233` when an if-arm's last
  expression is a void-returning generic call (return type `()`)
  AND the if's value flows into a join-block phi.
* **Root cause**: the `Computation::DirectCall` / `Dispatch` /
  `SealedDirectCall` codegen arms in `nod-llvm/src/codegen.rs`
  guarded `self.temps.insert(*dst, v)` behind `if let Some(v) = v`.
  When the called function returned void (`v == None`), the dst
  TempId was NEVER inserted into `state.temps`. But the lowering
  pass allocates a dst TempId regardless of the call's return
  arity. When that orphan TempId then appeared as a Jump arg into
  a join block, the phi-incoming wiring step at codegen.rs:1233
  panicked because `state.temps.get(arg_temp)` returned None.
  Not a type-system issue — a missing-binding issue.
* **Workaround**: ensure both arms produce a same-shape value,
  e.g. add a trailing `#f` sentinel after void calls. Retired
  with the fix.
* **Fix**: all three call-flavour Computation arms now insert
  `load_imm_nil()?.into()` for the dst TempId when the underlying
  emit returns None. Phi joins get a real i64 LLVM value (Dylan's
  canonical "no meaningful value" — `nil`). Consumers that ever
  use the value see `nil`, which is the right semantics for a void
  call's "result".
* **Regression test**: `tests/nod-tests/tests/sema.rs::
  gap_006_void_call_in_if_arm_does_not_panic`, plus the end-to-end
  smoke that the Sprint 45a `print-token` method now uses the bare
  `if (~instance?(...)) ... end` shape without any sentinel `#f`.
* **Scope**: small. ~15 lines of codegen.
* **Status**: **fixed in SHA `8e153b2`** (this commit).

## GAP-007 — Function-local heap references go stale across heavy allocation loops

* **Discovered**: Sprint 45b,
  `tests/nod-tests/fixtures/dylan-lexer.dylan` — the lex+dump path
  for the Dylan-in-Dylan lexer.
* **Symptom**: a function holds a heap-object reference in a `let`
  local (a `<stretchy-vector>`, a `<string-stream>`, a `<byte-string>`)
  and threads it through a loop that calls into other functions that
  allocate. After ~92–650 iterations (depending on the per-iteration
  allocation pressure) the local's word turns into garbage and the
  next use trips one of:
  - `stretchy_vector_push: not a <stretchy-vector>` in
    `src/nod-runtime/src/collections.rs:989`
  - `<no-applicable-methods-error>: no applicable method for
    \`write-byte\` on (<unknown:NNN>, <integer>)` raised by sema's
    method dispatch
  Class id `NNN` in the second form is a different small integer on
  every run — classic stale-pointer behaviour. Function parameters
  show the same failure as `let` locals; passing the vector/stream
  through a helper function's parameter slot does NOT save it.
  Module-level `define variable` cells DO survive because they live
  in cell-backed slots registered as GC roots (the Sprint 24 / GAP-004
  machinery).
* **Minimal reproducer**:
  ```dylan
  define class <tok> (<object>) end class;
  define function dump (vec :: <stretchy-vector>) => ()
    let stream = make-string-stream();
    let n = %stretchy-vector-size(vec);
    let i = 0;
    until (i = n)
      let t = %stretchy-vector-element(vec, i);
      write-string(stream, "abcdef");
      write-byte(stream, 10);
      i := i + 1;
    end;
  end function;
  ```
  Run with `n > ~92`. `vec` and `stream` both become garbage between
  iterations once `write-string` triggers enough allocations to grow
  the stream's backing storage. The `lex_count2.dylan` variant on
  this shape FAILS AT BUILD with a verifier error
  `Instruction does not dominate all uses!` involving a `gc.reload`
  PHI — same root cause surfacing as ill-formed LLVM IR instead of
  runtime corruption.
* **Workaround in tree**: the lexer fixture stashes its three
  heaviest-trafficked heap roots as module variables:
  - `*tokens* :: <object>` — the `<stretchy-vector>` accumulator
  - `*dump-stream* :: <object>` — the dump-tokens output stream
  - `print-token` writes through `*dump-stream*` directly (with
    helpers `write-line-col-to-dump-stream` and
    `write-escaped-source-text-to-dump-stream`)
  This pushes the failure envelope from ~92 lines to ~650 lines of
  the lexer's own source — enough for sprint 45b's working corpus
  (hello.dylan, the Sprint 45-era tests) but NOT enough to dump the
  lexer fixture on itself (~1265 lines, 38 KB). The workaround
  surface is documented in-source where it lives.
* **Root cause (verified by reading codegen + liveness)**: the bug is
  NOT in the GC liveness pass and NOT in the safepoint runtime — both
  are correct. The bug is in **phi-incoming wiring in
  `src/nod-llvm/src/codegen.rs`**:

  - `pending_incoming` (line 1140) is typed as
    `Vec<(BlockId, BasicBlock, Vec<TempId>)>` — it records the
    symbolic TempIds of jump args, not the resolved SSA values.
  - `emit_terminator` for `Terminator::Jump` (line 3092-3107) pushes
    the TempIds onto `pending_incoming` and moves on.
  - At end-of-function (line 1226-1236), the phi-wiring loop calls
    `state.temps.get(arg_temp)` to resolve each TempId → SSA value.
  - **But `end_safepoint` (line 3175) MUTATES `state.temps` every
    time it runs**:
    ```rust
    self.temps.insert(slot_info.temp, reloaded);
    ```
    Every safepoint reload overwrites `temps[t]` with a fresh
    `%gc.reload.tN` SSA value defined IN the current block.

  By the time phi-wiring runs at the end, `state.temps[t]` holds the
  **last** reload SSA value across the entire function — typically
  defined deep inside the loop body. The phi for `t` at the loop
  header ends up taking that same body-block SSA value on BOTH
  incoming edges. The entry-edge then can't possibly dominate it.

  This matches the GAP-007 IR-verifier error pattern exactly: the
  phi name `phi.t{}` (line 1206) and the gc.reload name `gc.reload.t{}`
  (line 3173) appear together in the failure message as the same
  TempId. Both incomings use the same value.

* **Symptom matrix explained by the root cause**:
  - **Build-time `Instruction does not dominate all uses!`** — both
    phi incomings reference `%gc.reload.tN` defined inside the body
    block. Entry-edge dominance violated.
  - **Runtime stale-pointer "after N iterations"** — when LLVM block
    layout happens to satisfy dominance, the IR is valid but
    semantically wrong: entry edge reads from a slot that wasn't
    initialised this call. Different `<unknown:NNN>` per run because
    the alloca slot pool is per-function-instance and the residual
    bits drift across runs.
  - **Function parameters fail identically to `let`-locals** —
    params skip entry-block phi creation but, once threaded into a
    downstream phi, go through the same `temps[p]` lookup that
    `end_safepoint` clobbered.
  - **Module-level `define variable` cells survive** — they bypass
    phi-wiring entirely. Each read calls `nod_var_get_by_name`
    against a registered cell slot.
  - **Workaround "envelope" of ~650 lines** — only because the most
    heavily allocating temp was hoisted into a module slot; the
    bug still bites every other `let`-local that's loop-carried.

* **The fix (small, surgical, three locations in `codegen.rs`)**:
  Snapshot SSA values at jump-emit time instead of resolving at
  phi-wiring time.

  1. Change `pending_incoming` type (line 1140) from
     `Vec<(BlockId, BasicBlock, Vec<TempId>)>` to
     `Vec<(BlockId, BasicBlock, Vec<BasicValueEnum<'ctx>>)>`.
  2. In `Terminator::Jump` (line 3092-3107), resolve `args` to SSA
     values BEFORE the branch, then push them:
     ```rust
     let arg_vals: Vec<BasicValueEnum<'ctx>> =
         args.iter().map(|t| self.temp_val(*t)).collect();
     ```
  3. In the wiring loop (line 1226-1236), iterate over the
     pre-resolved values directly — drop the `state.temps.get`
     lookup.

  Net ~10 lines. Snapshotting at emit-time captures the SSA value
  as it flowed out of the actual predecessor — which is exactly
  what a phi-incoming wants.

* **Related-bug bonus**: Sprint 11d's WNDPROC callback hang chase
  (Tasks #239 / #243 / #244 / §10.1 closure-graph tenuring) is the
  same root cause in a different shape — the callback frame's
  closure cells were threading through dispatch loops with
  loop-carried phis. The §10.1 tenuring hack worked around the
  symptom by pinning the cells in old-gen so reloads stopped
  mattering. If this fix lands cleanly, Sprint 11d Step F (#245)
  should be retire-able without the tenuring hack.

* **Regression test**: minimal reproducer above lands as
  `tests/nod-tests/tests/gap_007_stale_locals.rs` with fixture
  `tests/nod-tests/fixtures/gap-007-repro.dylan`. Add a focused
  unit test in `src/nod-llvm/src/codegen.rs::tests` that builds
  a 2-block function with a Jump-args phi-incoming, runs a fake
  safepoint between them, and asserts the resulting LLVM IR's
  phi incomings reference the pre-safepoint SSA value (NOT the
  reload).

* **Scope** (revised): SMALL — ~10 lines of code change + 2-3
  regression tests. Hot path though: this code runs for every
  Dylan function with a Jump terminator carrying args, so the
  full `cargo test` sweep IS required (one of the exceptions to
  the "Dylan-only changes skip the sweep" rule). One-day sprint.

* **Workaround retirement**: once the fix is in, revert the
  `*tokens*` / `*dump-stream*` module-var stash in
  `tests/nod-tests/fixtures/dylan-lexer.dylan` (Sprint 45b
  workaround) back to natural `let`-locals; add a sanity test
  that `dump-dylan-tokens` on the lexer fixture itself produces
  no errors (currently impossible — see §"Workaround in tree"
  above).

* **Status**: **open, diagnosis pinned**. Fix is a ~10-line patch
  in `src/nod-llvm/src/codegen.rs`. Workaround in tree at
  `tests/nod-tests/fixtures/dylan-lexer.dylan` (Sprint 45b).

## GAP-003 — No multi-value return / no multi-binder `let`

* **Discovered**: Sprint 45a, commit `29e1040`,
  `tests/nod-tests/fixtures/dylan-lexer.dylan`
  (the `offset-to-line-col-packed` function shape).
* **Symptom**: wanted to write
  ```dylan
  define function offset-to-line-col (off, source)
   => (line :: <integer>, col :: <integer>)
    ...
    values(line, col)
  end function;
  ...
  let (line, col) = offset-to-line-col(off, source);
  ```
  Neither the multiple-value return nor the multi-binder `let`
  form is implemented. Per nod-sema's "Out of scope" doc-comment,
  multi-value is a recognised future feature.
* **Workaround**: pack `line * 1_000_000 + col` into one integer
  return. Paired `unpack-line` / `unpack-col` accessors decode it
  at call sites. Works because both line and col are bounded
  small integers, but is ugly and would be wrong for anything else.
* **Planned fix**: real multi-value return as a first-class Dylan
  feature. Touches parser (multi-binder `let` form), sema (lower
  `values(...)` and the receiving destructure), DFM IR (multi-temp
  return), and codegen (multi-register return convention or
  caller-spilled slots).
* **Scope**: large. Plan it as its own sprint. Not blocking.
* **Status**: open.

---

## Notes

* The IDE (Sprint 41+) and the in-Dylan lexer (Sprint 45+) are
  collectively the **highest-pressure correctness tests** the
  compiler has — every gap they surface is a gap that real users
  will hit. Time spent fixing these gaps is time well spent.
* When a gap is **fixed**, leave its entry in this file but flip
  `Status` to `fixed in SHA xxxxxxx`, and remove the workaround
  marker comments from the source. The entry stays as historical
  context (and as a regression-test reminder).
