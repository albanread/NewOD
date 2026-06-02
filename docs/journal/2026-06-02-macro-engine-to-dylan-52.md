# 2026-06-02 — Porting the macro engine to Dylan (Sprint 52.1–52.5, 52.6 prep)

*Sprint 52 — the macro expander joins the lexer and parser in being
Dylan-written. Follows [the parser becoming the default front-end](2026-06-02-parser-is-the-default.md).
Spec: `specs/52-macro-expander-dylan.md`.*

## Goal

Port `nod-macro` (the ~1,900-line Rust macro engine) to Dylan, so the
front-end's third phase is self-hosted. Locus decision **(B)**: expand
entirely Dylan-side, before the AST-wire emit — no new wire.

## What we did

Drove the engine port sub-task by sub-task, each behind a Rust-parity or
hand-verified gate, committing every increment (reviewer pushes):

- **52.1** — locus decision (B) + `DYLAN_AST_WIRE.md` §7 "Parser+macro
  inputs" addendum. No new wire record/kind.
- **52.2** — promoted the engine out of the `dylan-macro-smoke` seed into
  `dylan-macro.dylan` (the production home) + a top-level `define macro …
  end macro` collector. Gate `macro_collect.rs`: name+rule-count parity
  with Rust `collect_macros` over stdlib (5 macros, cond=4 rules) + the
  fixtures.
- **52.3** — `match-pattern` to full parity: all seven `PatternKind`s
  (the seed had only expression/body). Gate `macro_match.rs`: 10 cases
  vs Rust `match_pattern`, identical bindings. Needed a tiny pub oracle
  helper `match_pattern_with_source` (the matcher reads literal text from
  a thread-local call-site source).
- **52.4** — substitution + **hygiene** (binder-only rename,
  `{name}__nod_hyg_{nonce}`). Gate `macro_expand.rs`: 4 cases incl. the
  real `for-each` (`%fip-state` renamed, `?var`/`?coll`/`?body` not), pinned
  nonce, byte-identical to `nod_macro::substitute`.
- **52.5** — multi-rule selection (`expand-call`) + the fragment-level
  module walk to fixpoint (`expand-fragments`/`expand-module-source`).
  Gates: `macro_expand.rs` extended to multi-rule `cond`; `macro_walk.rs`
  for embedded calls, passthrough, recursion-to-fixpoint, siblings.
- **52.6 prep** — strip `define macro` forms in the walk (compile-time
  only); call-shaped macro support (`name(args)`, no `end`).

## Why

The seed was already fragment-shaped (lex → fragments → match → substitute
→ re-lex), which is exactly locus (B)'s pipeline, so the "module walk over
the Dylan AST" the spec describes is really a **fragment-stream walk** —
simpler and the natural Dylan-side representation. Every gate drives the
SAME (def, call) cases through both engines so divergence is impossible to
miss; the cross-checks caught real things (the matcher's thread-local
source dependency; the lexer adapter silently dropping number/string
literals, which corrupts re-lexed expansions like `unless ?x (1) end`).

## Discovered

- **The lexer adapter was lossy.** `lex-token-to-tok` dropped every token
  kind it didn't explicitly handle (numbers, strings, chars, symbols).
  Harmless for collecting/matching the corpus (no literals in load-bearing
  positions) but it silently ate the `1` in a re-lexed `(1)`. Fixed by
  round-tripping all literal kinds as opaque `#"literal"` tokens.
- **Hygiene gensyms must be pinned to cross-check.** Rust's nonce is a
  per-expansion counter; the gate pins both sides to 42 so the rename text
  is deterministic. Neither corpus fixture actually has a template binder
  that isn't a pattern var, so the nonce never bites them — but the
  synthetic `let`/`method` cases exercise it.
- **The whole-file text round-trip is fidelity-limited — the real 52.6
  blocker.** `expand-module-source` renders expanded fragments back to
  text via `render-frags`, then the file would be re-parsed. Two leaks:
  the Dylan lexer keeps the `Module:` preamble (and `render-frags` drops
  the `:` off keyword-name tokens, so `Module: macros-unless` becomes
  `Module macros-unless`); and any keyword-name in the body (init-keywords,
  keyword args) loses its colon the same way. So rendering to text and
  re-parsing cannot be byte-faithful in general. The correct locus-(B)
  integration emits expanded **tokens** with synthesized spans (the
  span-rewrite piece deferred from 52.4) straight into the parser, never
  round-tripping through text.

## Where it leaves us

The engine is complete and parity-gated (52.1–52.5 + the strip/call-shaped
prep), all committed (`1c8cde4` … `1c4caf2`), five macro gates green. What
remains for 52.6/52.7 is the front-end **integration**, and it is the
substantial part:

1. Emit expanded tokens (not text) with synthesized spans — finish the
   52.4 span-rewrite, fragment→`<token>` flattening.
2. A shim entry that lexes → fragments → expands → feeds the parser, with
   the macro table seeded from the stdlib source (wire input (b)); bundle
   `dylan-macro.dylan` into `dylan-lex-shim.prj` (collision-free — checked)
   and rebuild the production shim `.lib.obj`.
3. `--expand-with-dylan` / `NOD_EXPAND_WITH_DYLAN` + verify-mode. Note the
   verify comparison must normalise out `define macro` items: locus (B)'s
   output is macro-free, but the Rust oracle's `format_ast_module` still
   prints `(DefineMacro …)`.
4. Flip the default + docs (52.7).

This rebuilds the production shim that the default parser uses, so it is
the point to confirm direction before proceeding.
