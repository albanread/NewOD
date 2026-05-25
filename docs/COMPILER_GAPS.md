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
* **Status**: **fixed in SHA TBD** (this commit). GAP-002's regression
  test still passes — constants stay immutable, variables are the
  only writable kind.

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
