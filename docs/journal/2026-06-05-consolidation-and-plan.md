# 2026-06-05 — Consolidation after the hacking week: state, loose ends, and the plan

*A deliberate pause to stabilise and plan, after a run of late-night
sessions (Sprints 52–53) that moved fast. Nothing here is new feature
work — it's an honest inventory and a prioritised path back to a calm,
all-green baseline before the next push.*

## Where we actually are

The whole week's work is **committed and pushed** (origin/master in sync
through `31d3218`). The driver builds. The front-end self-hosting ladder:

| Layer    | Status | Evidence |
|----------|--------|----------|
| Lexer    | ✅ self-hosted | oracle gate green |
| Parser   | ✅ self-hosted | translate gate **34/44** byte-identical, zero divergences |
| Macros   | ✅ self-hosted (opt-in) | `NOD_EXPAND_WITH_DYLAN`, fidelity proven vs the 6k-line compiler source |
| Sema     | ◐ oracle done, Dylan port **WIP** | `dump-sema` + `DYLAN_SEMA_WIRE.md`; Dylan walk checkpointed, not wired |
| Lowering | ⬜ still Rust | (next after sema) |

The shape is exactly the ratified architecture: a Dylan front-end being
grown layer-by-layer on the permanent Rust+LLVM back-end, each layer
validated by a byte-identical "two compilers, one truth" gate. **The
trajectory is healthy; this is progress, not drift.**

## Loose ends the fast week left (triaged)

**P0 — correctness, blocks a clean sweep**
1. **GAP-011 liveness (#300, in progress).** Global live-in/out fixpoint
   landed in 48b, but the symptom (`stretchy_vector_push` stale-root
   panic) still fires on heavy parse loops. This is the one real
   correctness hole — precise GC roots going stale.
2. **`short_circuit_ops` JIT tests hang.** Deadlock under parallel test
   threads AND at least one test (`and_short_circuits_past_out_of_range_array_index`)
   does not terminate even serialised/in-isolation. Means the full
   `cargo test -p nod-tests` sweep can't complete unattended. **Strong
   suspicion this and GAP-011 share a root** (a liveness/CFG fixpoint not
   converging on the `&`/`|` join blocks). Worth investigating together.
   (A background-task chip was raised for this.)

**P1 — completeness, no green/red impact yet**
3. **Sprint 53.2 sema walk is unfinished** (`dylan-sema.dylan`, just
   checkpointed). Still has `DBG` `format-out` diagnostics that pollute
   stdout; the gate it names (`sema_topnames.rs`) isn't written, so
   nothing builds or runs it. To finish: strip the DBG lines, write the
   gate to byte-match `--parse-with-rust dump-sema`'s top-names section on
   the class-free fixtures (`hello`, `factorial`), iterate.
4. **53.1 shim class-id drift.** `dump-sema` panics through the shim
   (`aot.rs:1037`) but works with `--parse-with-rust`. The in-process
   gate path is clean, so it's latent — but it's the same class-id-drift
   smell flagged back in the 52.6 rollout notes. Worth root-causing once,
   since it'll bite again as more layers route through the shim.

**P2 — cosmetic / deferred-by-design (leave alone for now)**
5. **`newgc-core` build warnings** (unused imports in `evac.rs`/`mark.rs`/
   `pin.rs`/`lisp_layout.rs`, one never-read `dest_gen` field).
   Deliberately NOT touched: the collector is mid-evolution and the
   standing rule is to leave GC code alone unless a sprint requires it.
   Clear these only as part of a GC-touching sprint.
6. **52.x expander is ~50× slower** under its opt-in flag. By design —
   per the locked decision, front-end perf waits for full consolidation
   (one DFM handoff when lex→parse→expand→sema→lower are all Dylan), not
   hybrid tuning. Not a regression; the default path is unaffected.
7. **Stale harness task list** (~300 entries, almost all completed
   sprints). Noise, not risk.

## The plan (recommended order)

The theme: **get back to an unattended all-green sweep before adding new
surface.** Correctness first, then finish what's half-built, then resume.

1. **Make the test sweep completable again (P0-2).** Decide the
   `short_circuit_ops` story: is it a genuine non-termination (fix it) or
   purely a parallel-JIT-global-state deadlock (mark the suite
   `--test-threads=1` / `harness=false` and document the convention, à la
   the other JIT projects)? Confirm by reproducing at an earlier commit to
   date any regression. *This unblocks every future "is it green?" check.*
2. **Land GAP-011 (P0-1).** Likely shares a root with (1). Get the
   `stretchy_vector_push` stale-root panic to stop firing on heavy parse
   loops; close #300.
3. **Finish or formally park Sprint 53.2 (P1-3).** Either strip the DBG
   and write `sema_topnames.rs` to green, or, if sema is paused, leave the
   checkpoint as-is (it's inert) and note it parked. Don't leave it
   half-visible.
4. **Root-cause the shim class-id drift once (P1-4).** Before more layers
   depend on the shim path.
5. **Then, and only then, resume the sema Dylan port** (53.3+: classes,
   slot accessors) on a calm baseline.

## Process notes (the "professional way", for the calmer week)

- **Push at the end of every session.** A week of work sat 19 commits
  deep with no remote copy — one disk failure from gone. Cheap insurance.
- **Keep the two-compilers gate sacred.** It's caught precedence bugs,
  `:=` mis-parsing, and representation mismatches that neither parser
  reported alone. It's the safety net that makes fast weeks survivable.
- **Don't tidy the collector for cosmetics.** GC churn has wrecked
  progress before; warnings in `newgc-core` wait for a GC sprint.
- **Perf stays deferred until consolidation.** Resist hybrid-perf
  rabbit holes; the win falls out of the single DFM handoff.
