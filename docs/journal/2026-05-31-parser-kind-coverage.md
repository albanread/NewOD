# 2026-05-31 — Parser kind-coverage: the extend-and-test grind begins

*Sprint 51e. Commits `db0b082` (harness) … `b86d1de` (DefineClass), and
onward. Continuation of [the front-end self-hosting session](2026-05-31-front-end-self-hosting.md).*

## Goal

Start filling out the Dylan front-end for real, using the loop the
coverage harness enables: pick the highest-frequency `Error` construct,
write its `emit-node` method (Dylan) + `Kind` variant (Rust) + wire-doc
row, rerun the harness, watch coverage climb and the next target
surface. "Two compilers, measured."

## What we did

1. **Coverage harness (`db0b082`).** `dylan_parse_coverage.rs` sweeps
   the fixtures through `dump-dylan-ast`, classifies every `Error` node
   by the leading word of its source span, prints a ranked punch-list.
   Baseline: **77% of corpus AST nodes structured** (4622/5941), with
   the punch-list `<unspanned 0..0>` 923, `if` 192, `define-class` 104,
   `until` 86, tail.

2. **Span backfill for containers (`cf2c410`).** Container nodes
   (`<ast-body>`, `<ast-call>`, `<ast-binary-op>`) carry no leading
   `<token>`, so `span-of` returned `(0,0)`. Added
   `backfill-span-from-children`: after a container's children emit,
   recover its span as the union of descendant spans (bottom-up).
   `dump-dylan-ast hello.dylan` went from `Body 0..0` everywhere to
   `Body 8..547 … Body 527..547`.

3. **DefineClass / DefineMethod / DefineGeneric (`b86d1de`).** First
   real kind extension. `function`/`method` are `<ast-body-definition>`
   (body-word dispatch); `class`/`generic` are *dedicated* nodes. New
   emit-methods for all. Coverage **77% → 79%** (+696 structured
   nodes); `define-class` and `define-generic` left the punch-list.

## Why

The order — harness first, then span backfill, then a kind — was
deliberate: build the dashboard before the grind so it tells you what
to do and confirms each step. Span backfill went first among the fixes
because it's low-risk and improves every node's span (which the
eventual `ast::Module` build will need), and because measuring whether
it moved the Error count was itself a diagnostic (it didn't — see
below).

## Discovered

1. **Span backfill helps containers, not leaves — and that's a
   finding, not a disappointment.** The backfill left the coverage
   numbers *identical* (still 923 unspanned). That proved the 923
   unspanned `Error`s are **childless leaves**, not containers:
   nodes like `let`/`<ast-local-decl>` whose span lives in a
   type-specific slot (`ldecl-word`) the catch-all `emit-node` can't
   reach, and which the parser doesn't copy to `node-token`. So
   backfill-from-children structurally *cannot* declassify them.
   The reframe: **the unspanned bucket isn't a span bug to patch — it's
   the same missing-kind work as the spanned bucket, just invisible
   until each kind lands.** Declassifying ≡ structuring.

2. **`define class` / `define generic` are dedicated AST nodes, not
   body-definitions — and the harness caught me getting it wrong.**
   First cut mapped `class` as an `<ast-body-definition>` body-word.
   It compiled, ran, and did *nothing* — `define-class` stayed at 104
   in the punch-list. The harness surfaced the dead code immediately
   (same way `--verify-parse` caught the Rust parser's `cond` gap last
   session). Real dispatch: `parse-definition` routes `class` →
   `parse-class-definition` → `<ast-class-definition>`, a node with its
   own `class-supers`/`class-slots` slots. The dashboard pays for
   itself: a silent mistarget became a visible "number didn't move."

3. **Emit the children and the next punch-list item appears.** Emitting
   the class slot-specs as `DefineClass` children surfaced `slot` (188)
   — a target that had been *inside* the Error blob, invisible. The
   loop is self-revealing: each kind you structure exposes its
   substructure as the next ranked target. `<ast-slot-spec>` sets
   `node-token` (parser line 1259), so the slots come through spanned
   and cleanly classified.

4. **The lowerer rejects empty `begin` blocks.** First backfill cut had
   `if (cond) <comment-only> else … end`; the empty then-branch lowered
   to "empty `begin` block not lowered". Rewrote as a single positive
   condition (`if (hi > 0)` — a real span always has `hi > lo >= 0`).
   A Dylan-subset gotcha worth remembering: don't write comment-only
   branches.

## Where it leaves us

**Coverage: 79%**, punch-list now:

| Count | Construct | Note |
|------:|-----------|------|
| 874 | `<unspanned 0..0>` | leaf bucket — `let`/expr nodes, each needs its own emit-method (span from its own slot) |
| 218 | `if` | `<ast-statement>` — biggest single spanned win, **next** |
| 188 | `slot` | newly surfaced from DefineClass children |
| 87 | `until` | loop statement |
| ~10 | `define-method`×3, `while`, `cond`, `unless`, punct | tail |

**Next:** `if` (the `<ast-statement>` family — likely covers `until`/
`while`/`select` too once the statement node is handled, since they
share `<ast-statement>`). Then `slot` to finish the class story. Then
chip at the unspanned leaf bucket (`let` first — it's everywhere).
Eventually: the `DylanAst → ast::Module` translator that makes
`--parse-with-dylan` replace `parse_module`.

**Repeatable loop, confirmed working:** pick top punch-list item →
emit-method + Kind variant + wire-doc row → rebuild shim + relink
driver → rerun harness → number climbs, next target surfaces → commit.
~3 file edits per kind. The bottleneck is writing Dylan emit-methods,
which is exactly the right bottleneck.
