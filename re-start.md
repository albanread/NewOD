# Sprint 45b restart instructions

You're picking up an in-progress Sprint 45b (real Dylan lexer in Dylan). An
agent ran for ~26 minutes and made a 734-line edit to the lexer fixture, but
crashed before committing. The work is **uncommitted on master** and the code
**builds but panics at runtime**.

Date this was written: 2026-05-25.

---

## 1. First thing on restart — survey state

```
cd /e/NewOpenDylan/NewOpenDylan
git status
git diff --stat
git log --oneline -5
```

Expected state:
- Branch: `master`, 9 commits ahead of `origin/master` (DO NOT push — reviewer pushes)
- Last commit: `8117630 docs(GAP-005, GAP-006): pin fix SHA in COMPILER_GAPS.md`
- Uncommitted change: `tests/nod-tests/fixtures/dylan-lexer.dylan` (+734 -10)
- No new files added

If git status doesn't match, the agent's work has been lost or partially
discarded — fall back to launching a fresh agent (see §5).

---

## 2. The bug to fix first

```
cargo run -p nod-driver --quiet -- dump-dylan-tokens tests/nod-tests/fixtures/dylan-lexer.dylan
```

This currently panics with:

```
thread '<unnamed>' panicked at src\nod-runtime\src\collections.rs:989:36:
stretchy_vector_push: not a <stretchy-vector>
```

So the agent's `lex` is calling `add!` (or whatever pushes onto the token
vector) with something that isn't a stretchy-vector — likely a wrong variable,
or it's overwriting the accumulator with something else (a token? a string?)
somewhere in the loop body.

### Investigation steps
1. `git diff tests/nod-tests/fixtures/dylan-lexer.dylan | head -200` — read the
   new `lex` body
2. Find every site that mutates the token accumulator (`add!`, `push`, or
   whatever the fixture uses) and the variable it threads through
3. Most likely culprit: agent forgot the `add!` on a stretchy-vector returns
   the new vector and didn't reassign, OR shadowed the accumulator inside a
   nested scope
4. Re-run dump-dylan-tokens after each candidate fix until the panic goes away

The build is clean — this is purely a Dylan-source logic bug.

---

## 3. Acceptance gate (Sprint 45b "done")

Once the panic is fixed:

```
cargo run -p nod-driver --quiet -- dump-dylan-tokens tests/nod-tests/fixtures/dylan-lexer.dylan
```

Must produce **zero `<error-token>` lines** when tokenising the lexer's own
source. Spot-check the output for `<identifier-token>`, `<keyword-token>`,
`<string-token>`, `<line-comment-token>`, `<whitespace-token>`, `<eof-token>`.

Then add ~30 unit tests in `tests/nod-tests/tests/dylan_lexer.rs` (or sibling
`dylan_lexer_units.rs` — agent's call). The existing test
`dump_dylan_tokens_for_hello_prints_eof_only` will need to be updated to match
the real lexer output.

Final gates (Dylan-only-change rule: skip the full test sweep):
```
cargo build
cargo test -p nod-tests dylan_lexer
```

---

## 4. Commit shape

The agent was told to split into small commits. If you finish the work
yourself, aim for:
1. `Sprint 45b: real lex function — tokenise <stretchy-vector>` (the fix + the
   bug-fix to the agent's draft)
2. `Sprint 45b: ~30 unit tests for lex` (the test file)
3. Any new GAP entries appended to `docs/COMPILER_GAPS.md` in their own commit

Every commit gets the standard Claude co-author trailer. **Do not push.**

---

## 5. If the agent's work is unsalvageable

The full Sprint 45b brief is in conversation history of this session (search
for "Sprint 45b real Dylan lexer"). Key inputs:
- Plan: `docs/SPRINT_45_DYLAN_LEXER.md` §3 (token kinds), §9 (deferred
  decisions: block comments DO NOT nest, negative integers lex as Minus +
  Integer)
- GAPs status: `docs/COMPILER_GAPS.md` — GAP-001..006 all fixed, so streams,
  `define constant`, `define variable`, `if` without `else`, and void calls in
  `if` arms are all available
- Stub state to restore: `git checkout master -- tests/nod-tests/fixtures/dylan-lexer.dylan`
  brings back the 555-line stub that just returns `[<eof-token>]`

To launch a fresh agent, re-send the Sprint 45b brief from conversation
history to a new `Agent` tool call (subagent_type `general-purpose`). Note:
the Anthropic API was returning 529 Overloaded repeatedly when this session
ended; the launches that *did* eventually start ran to completion but didn't
get to commit.

---

## 6. After 45b is committed

Pending tasks in priority order:
- **Sprint 45c** — lift character predicates (`digit?`, `alpha?`, etc.) into
  stdlib so the lexer stops carrying its own copies. Task #268.
- **Sprint 45d** — oracle harness: cross-check the Dylan lexer's output
  against the Rust lexer on a corpus. Task #269.
- **Sprint 45e** — wire the Dylan lexer into IDE syntax colouring (replacing
  the Rust-side colourer). Task #270.
- **GAP-003** — multi-value return + multi-binder let. Open, big, own sprint.
  Task #273.
- **Sprint 11d Step F** — JIT-fixture WNDPROC reproducer test. Task #245.

---

## 7. Standing rules (don't forget)

- `F:\scratch` is the only valid test root for external files
- No semispace tests — they waste time
- No interactive UI in routine `cargo test` (interactive tests are `#[ignore]`)
- Always check WinAPI return values
- Dylan-only changes (`.dylan`/test files) skip the full test sweep —
  `cargo build` + targeted `cargo test` is enough
- **DO commit before reporting**
- **DO NOT push** — reviewer pushes after sign-off
- Always review agent work — read diffs, re-run gates, never trust the summary
  alone

---

## 8. Useful commands

```bash
# See what the agent changed
git diff tests/nod-tests/fixtures/dylan-lexer.dylan | less

# Restore the pre-agent stub if needed
git checkout master -- tests/nod-tests/fixtures/dylan-lexer.dylan

# Run a single lexer test
cargo test -p nod-tests --test dylan_lexer -- --nocapture

# Manual smoke (after fixing the panic)
cargo run -p nod-driver --quiet -- dump-dylan-tokens tests/nod-tests/fixtures/dylan-lexer.dylan 2>/dev/null | head -50

# Build only (fast gate)
cargo build
```
