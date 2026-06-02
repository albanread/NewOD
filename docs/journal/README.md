# NewOpenDylan — Engineering Journal

A running lab notebook. Where `SPRINTS.md` records *what* shipped per
sprint and the commit log records *what changed per file*, this
journal records the part that otherwise evaporates: **what we were
trying to do, how we approached it, why we chose what we chose, and
what we discovered along the way** — including the wrong turns, the
"oh, it's actually simpler" moments, and the lessons that should
outlive the session they were learned in.

The audience is us, six months from now, trying to remember why the
architecture is shaped the way it is.

## Convention

- One file per session or coherent work-arc:
  `YYYY-MM-DD-short-slug.md`.
- Index below, newest first.
- Each entry, loosely:
  1. **Goal** — what we set out to do this session.
  2. **What we did** — the arc, with commit refs.
  3. **Why** — the decisions, especially the ones we reversed.
  4. **Discovered** — the lessons. This is the part that matters
     most; be honest about surprises and dead ends.
  5. **Where it leaves us** — state + the obvious next move.
- Keep prose over bullet-spam where the reasoning is the point. This
  is a notebook, not a changelog.

## Entries

- [2026-06-02 — The Dylan parser is the default front-end (51e.6)](2026-06-02-parser-is-the-default.md)
  — Sprint 51e complete. With the class-id drift fixed, the Dylan parser
  flips from opt-in to the default real-pipeline front-end (`--parse-with-rust`
  opts out; Rust = fall-back + verify oracle), gated on shim availability.
  Default full sweep green (35 binaries; only environmental `ide_shell_infra`
  fails). Lexer + parser are now both Dylan; the 8 remaining corpus
  fall-backs are macro-phase work that closes in Sprint 52.
- [2026-06-02 — Shim-AOT class-id drift: a great diagnosis, a rejected fix](2026-06-02-class-id-drift-attempt-rejected.md)
  — Task #7. A delegated fix produced an excellent dual-manifestation
  diagnosis (GAP-001's stdlib `<stream>` classes made the "no stdlib
  define class" premise stale) but an implementation rejected on review:
  it masked a *self-introduced* `LNK2005` with `/FORCE:MULTIPLE`, which
  poisoned its own green-sweep. Independently verified the clean baseline
  was healthy (c3_oracle + bench_richards pass, no LNK2005) and reverted.
  Lesson: a green gate obtained by silencing an error class is not green.
- [2026-06-02 — The Dylan parser enters the real pipeline; the shim-AOT class-id drift surfaces](2026-06-02-parser-in-the-pipeline-and-the-class-id-drift.md)
  — Sprint 51e.5. `--parse-with-dylan` wired into compile/eval/build via
  a `set_parse_override` hook (mirroring the lexer), with Rust fall-back
  + verify-mode; `eval "1 + 2 * 3"` → 9 through the Dylan parser. Surfaced
  the cross-cutting blocker: firing any front-end shim registers its
  `define class`es through the shared user-class-id counter, drifting the
  AOT-baked class ids. Gates 51e.6 default + all of 52/53/54; diagnosed
  with three fix directions, deferred as a back-end fix.
- [2026-06-02 — Parser parity push: 14 → 28/36, and two traps](2026-06-02-parser-parity-push-14-to-28.md)
  — Sprint 51e. Authored the 51e–54 migration specs, then drove the
  translation gate 14→28/36 (Precedence:c ladder, comment-aware
  operator extraction, HashLit/DefineBinding kinds, definition modifiers
  + DefineGeneric). Two traps documented: fall-back reasons are
  first-reported-reason artifacts, and the gate's self-build can measure
  a stale binary. Remaining 8 fall-backs are macro-phase (Sprint 52) work.
- [2026-06-01 — The translator payoff: Paren-transparent dump, `:=` precedence, 9→14/36](2026-06-01-translator-payoff-paren-and-assign.md)
  — Sprint 51e. Cashing in the flat-precedence migration by removing the
  translator's nested-binop guard. Took two more fixes: a `Paren`-transparent
  dump formatter (grouping is in the tree shape, not the marker) and a real
  `:=`-precedence bug in the Dylan-in-Dylan parser that the byte-identical
  gate caught (`i := i + 1` was parsing as `(i := i) + 1`). Plus two stale
  C-precedence unit tests the migration had missed.
- [2026-05-31 — DRM flat precedence by default, `Precedence: c` migration bridge](2026-05-31-flat-precedence-pragma.md)
  — Sprint 51e. The translate gate exposed the Rust parser's C-style
  precedence as a real bug (Dylan is flat per the DRM). Fixing it broke
  the whole C-precedence-assuming corpus (incl. the stdlib's char
  predicates); resolved with a `Precedence:` header pragma — default
  flat, legacy files opt into `c`.
- [2026-05-31 — DylanAst → ast::Module: the parser starts replacing parse_module](2026-05-31-dylan-to-ast-translator.md)
  — Sprint 51e, fork #2. Wire enrichment for function signatures
  (kinds 25–30), the `dylan_to_ast` translator, the `--parse-with-dylan`
  flag with fall-back, and a byte-identical translation gate. `hello.dylan`
  translates byte-identically (1/34); the gate immediately caught a
  too-empty-Module bug from unspanned `Error` nodes.
- [2026-05-31 — Parser kind-coverage: the extend-and-test grind begins](2026-05-31-parser-kind-coverage.md)
  — Sprint 51e. The coverage harness drives kind-by-kind extension:
  span backfill (and the finding that unspanned Errors are leaves, not
  containers), then DefineClass/Method/Generic. 77% → 79%; `slot`
  surfaces as the next target.
- [2026-05-31 — Front-end self-hosting: the breakthrough session](2026-05-31-front-end-self-hosting.md)
  — Sprints 51b–51e. The Dylan lexer and parser go live inside the
  driver; the architecture is reframed to a Dylan front-end on a
  permanent Rust+LLVM back-end; the parser coverage harness measures
  77% baseline and produces the extend-list.
