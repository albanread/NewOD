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
  `1000000` appears at three sites with comments).
* **Symptom**: a module-level `define constant $line-multiplier =
  1000000;` declaration is correctly parsed and accepted, but
  references to `$line-multiplier` from inside a `define function`
  body (in the same module / same file) fail to resolve. Workaround
  was to substitute the literal integer at every call site with a
  comment marker.
* **Workaround**: the literal `1000000` is repeated at three sites
  in `offset-to-line-col-packed` / `unpack-line` / `unpack-col`,
  each with a `// SPRINT 45a COMPILER-GAP-002` comment.
* **Planned fix**: investigate sema's handling of `Item::DefineConstant`.
  Likely either:
  (a) constants are parsed but never lowered into the global name
      resolution table, OR
  (b) they are, but function-body name resolution doesn't consult
      that table.
  Either way the fix is bounded — a few lines of sema. Comes with
  a unit test that defines a constant and uses it from a function.
* **Scope**: small. Probably a one-evening fix once the sema
  control flow is understood.
* **Status**: in-progress (investigation starting now).

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
