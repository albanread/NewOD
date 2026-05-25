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
  Dylan I/O abstraction — `force-output`, `write-character`,
  `write-string` all dispatch on stream class). No `<stream>` class
  exists in `nod-runtime`. The `stream` parameter had to be left
  untyped, and the actual work moved into a parallel
  `print-token-to-string(t, source) => <byte-string>` helper that
  `dump-tokens` calls. The `print-token` generic with the untyped
  stream slot stays as a placeholder for the future `<stream>` API.
* **Workaround**: `print-token-to-string` shape. Tokens know how to
  render themselves to a byte-string; `dump-tokens` concatenates the
  results.
* **Planned fix**: minimum-viable `<string-stream>` (write-only,
  accumulates bytes into a stretchy-vector, `as-byte-string` to
  materialise). Then `<stream>` abstract base, `<byte-stream>` for
  binary, `<text-stream>` for character-level. Once `<string-stream>`
  lands, the lexer's `print-token` generic gets its proper type and
  the helper-stream parallel can be retired.
* **Scope**: medium. The minimal `<string-stream>` is maybe a day;
  the full hierarchy with `force-output` semantics and real
  `<file-stream>` support is a sprint of its own.
* **Status**: open.

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
* **Status**: **fixed in SHA TBD** (this commit). `define variable`
  is a separate, deeper gap — see GAP-004.

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
* **Workaround**: avoid `define variable`. The lexer fixture uses
  `define constant` exclusively. Mutable module-level state isn't
  expressible in user Dylan today.
* **Planned fix**: complete the `Item::DefineVariable` lowering.
  The right shape is probably "zero-arg getter function + one-arg
  setter function", same pattern as slot accessors — store the
  current value in a process-global Word slot (similar to Sprint
  38c's literal slots), getter loads it, setter stores it (with
  write-barrier if heap pointer).
* **Scope**: medium. Touches the lowering pass, the AOT
  registration path (need a runtime slot per `define variable`),
  and possibly the JIT path for cross-module refs.
* **Status**: open.

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
