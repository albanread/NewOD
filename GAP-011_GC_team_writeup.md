# GAP-011 — stale precise root: status & action list

**Owner:** NewOpenDylan compiler side — front-end remains exonerated.
The agent-review "register-arg staleness" hypothesis was **tested and
REFUTED** by direct measurement (2026-05-30).
**Status:** OPEN. Root cause still unknown; two hypotheses now ruled out
(A1 `note_successor_entry_temps` stale-reload + agent's arg-root
coverage). The probe `NOD_DIAG_ARG_ROOT_COVERAGE` is now silent on the
parser source (0 gaps) yet the crash signature is byte-for-byte
identical.
**Gating test:** `nod-driver parse-dylan F:\scratch\jcs-40.dylan` must exit **0**
(today: exit 9, panic at `nod-runtime/src/collections.rs:1075`,
`stretchy_vector_push: not a <stretchy-vector>`, zeroed/non-forwarded wrapper).

## Agent hypothesis tested & REFUTED — 2026-05-30

The agent-review hypothesis from the previous session:
> liveness correctly omits args dead-after-call → value flows as a
> register operand to the callee → if the callee allocates and triggers
> moving GC before its own first safepoint, the arg becomes stale.

Tested in three steps:
1. Built env-gated diagnostic `NOD_DIAG_ARG_ROOT_COVERAGE` (see
   `nod-dfm::diagnose_arg_root_coverage`, wired in `nod-sema::lower`).
2. Probe lit up against parser source: **137 functions with 1378
   gaps**, including `dump-node` (104), `acc-string` (1 false-positive
   on a Top-typed byte), `main` (6). The "freshly-allocated value flows
   directly into next call" shape (e.g. `c[2]=write-to-string(...);
   c[3]=acc-string(buf, that-result)`) repeats dozens of times in
   `dump-node`.
3. Implemented the candidate fix: extended `populate_safepoint_roots`
   to also include every heap-typed call arg regardless of post-call
   liveness, updated the verifier to accept arg-only entries, ran
   `cargo clean` + full rebuild + `parse-dylan F:/scratch/gap011-jcs-
   min-crash.dylan`.

**Result:** probe now reports `TOTAL functions_with_gaps=0 gaps=0`.
Crash signature unchanged: same `stretchy_vector_push: not a
<stretchy-vector>` with `sv=0x...771 ptr=0x...770`. The fix had **zero
effect** on the runtime behavior. Hypothesis refuted.

**Why the hypothesis didn't pan out (post-mortem):** at -O0, LLVM
spills every incoming arg to a local stack slot on entry, and the
callee's own backward-liveness pass already includes that param in the
`safepoint_roots` of every internal call where the param is live
across. So the callee always re-loads its arg from a slot the GC has
already rewritten — the caller-side "the arg isn't in MY slab" gap
is closed by the callee's own slab, end-to-end. Adding the arg to the
caller's slab was redundant.

The liveness fix was **reverted**; the probe + `diagnose_arg_root_coverage`
stay as permanent diagnostics. Findings preserved in
`GAP-011_arg_root_coverage_findings.log`.

## Trace inspection nails the staleness pattern — 2026-05-30 (afternoon)

Once `nod-driver symbolicate` made the backtrace readable, ran the
gating crash with `NOD_GC_TRACE=/tmp/gc-trace.jsonl` and answered the
"did GC ever see this pointer?" question by grepping. **It did.**
Pattern (4 cycles, 393 events total):

- Crash: `sv=0x000002254a34d771 ptr=0x000002254a34d770`.
- Tagged form `4a34d771` appears 80× in the trace (16 `root`, 64
  `root_rewrite`). My first grep was for the untagged `770` — 0
  matches. Lesson: **the trace records the tagged Word**, search both
  forms.
- Cycle 2's multi-pass major collection rewrote the vector through
  `38d → 34d → 37d → 34d` (G0→G1, G1→Tenured, Tenured→Tenured
  defrag). End state: 11 distinct slots hold `4a34d771`.
- Cycle 4 (a later major) moved the vector again: `4a34d771 →
  4a39d771`. Of the 11 slots that held the value, only **8 got
  rewritten**. The remaining **3 forgotten slots** are:
  - `0x000000962bfef3b8` — cycle 1 src=stack, i=5
  - `0x000000962bfef648` — cycle 1 src=aot,   i=18
  - `0x000000962bfef710` — cycle 1 src=aot,   i=17
  None of them appear in cycle 3 or 4's root set. They were
  deregistered between cycles 2 and 3.
- After cycle 4: heap object is at `4a39d771`; those 3 stack
  addresses still hold `4a34d771`. **stretchy_vector_push panics
  with `sv=0x...4a34d771`.**

The killer side-by-side: cycle 4's AOT slab covers slot addresses
`f3e0 < f470 < f478 < f5b0 < f5b8 < f6f0 < f6f8 < f700 < f708` —
and the forgotten `f648` and `f710` **sit physically between those
addresses, in active stack memory the slab doesn't claim**. So this
isn't "the slab's gone, the memory's freed" — these are stack
words inside a live Dylan call's frame that the current safepoint's
slab doesn't include, but earlier cycle 2 did include.

That is a **codegen / IR bug**, not a collector bug. Some SSA temp
gets reloaded from one of those forgotten stack addresses (or a value
flows from one of them via an LLVM load) without going through the
safepoint slab's reload protocol. Across cycle 4 the SSA value is the
pre-move address; passing it into `add!` → `nod_stretchy_vector_push`
fails on the wrapper-class check.

## Smoking gun located — 2026-05-30 (late afternoon, peer-reviewed)

Reading `src/nod-llvm/src/codegen.rs` with the trace finding in mind,
and **scoping the impact carefully** (peer-review note: this is
parameter-specific, not "all temps" — block params take a separate,
working path):

```rust
// codegen.rs:2303-2308 — at the START of EVERY block:
for (i, p) in func.params.iter().enumerate() {
    let pv = llvm_fn
        .get_nth_param(i as u32)
        .expect("parameter index in range");
    state.temps.insert(*p, pv);
}
```

**Every block-entry resets the SSA binding of every function
parameter to its original `get_nth_param` value — the pre-GC value
from the function's prologue.**

The comment at 2299-2302 acknowledges this:
> Safepoint reloads intentionally rebind `state.temps[temp]` to a
> fresh SSA value, but that rebind is only valid within the block
> that performed the reload. When we move on to a sibling block,
> restore canonical block-entry bindings so later uses do not pick
> up a reload defined in a non-dominating predecessor.

It's an SSA-dominance fix, but for function params it's also the
**bug**: any safepoint reload of a function param that happened in
block A is thrown away the moment we move to block B. If block B
then has an allocating call where param `p` is in
`safepoint_roots`, `begin_safepoint` stores `temp_val(p)` =
`%fn.argN` (the original, pre-GC) into the slab slot. GC reads that
stale value out of the slab, tries to evac/follow it — but the page
was reclaimed by the cycle that moved p — wrapper check fails →
panic.

**Scope (peer-reviewed correction).** The bug is specifically the
**function-parameter** path:

- **Block params**: handled correctly. Lines 2310-2312 rebind them
  to their phi values; lines 2331-2338 wire predecessors' resolved
  `BasicValueEnum`s into the phis using snapshots from
  `pending_incoming` (deliberately NOT re-consulting `state.temps`).
  GAP-007's commentary at lines 2167-2171 explains exactly why this
  has to be a snapshot.
- **Function params**: NOT phi'd. They're the raw LLVM function-arg
  SSA values, accessible from every block, rebound unconditionally
  at every block entry by lines 2303-2308.
- **Other temps derived from function params**: also at risk —
  anything whose dataflow chain reads a function param post-GC will
  see the pre-GC value once the block boundary is crossed.

So: **parameter-centric stale-value clobber, with downstream
corruption impact on any value derived from a function param after
the first block transition.** The GAP-007 / merge-divergence
diagnostic at lines 2340-2370 already documents the general
"non-dominating-predecessor reload leakage" shape; this is the same
class of bug, narrowly specific to function params because they
bypass the phi machinery the diagnostic was written to protect.

This matches the trace 1-for-1:
- Cycles 1-2: dump-node is in a single recursive call. Within that
  block, safepoint reloads work. Slot 17 of cycle 1's slab holds the
  buffer, gets rewritten correctly.
- Between cycles 2 and 3: dump-node hits a block boundary (e.g., the
  body of an if, the next iteration of an embedded recursion).
  Rebind resets the buffer-param's binding to `%fn.arg_buf` =
  `4a34d771` (the cycle-2-reloaded value WAS this address, but
  `%fn.arg_buf` is the value passed in RCX at function entry, which
  is whatever the CALLER's slab had at the moment of call — and
  that's stable through cycles 1-2 by the caller's own slab
  protection).
- Cycle 4: dump-node's NEW block's allocating call stores
  `%fn.arg_buf` to the slab. But cycle 4 has already moved the
  object to `4a39d771`; `%fn.arg_buf` is still `4a34d771` (the
  value the caller PASSED, set at function entry). GC tries to follow
  `4a34d771` to the new address — but `4a34d771`'s page has been
  reused. Wrapper-class check on next call to `add!` →
  `nod_stretchy_vector_push` fails on entry.

## Proposed fix — with non-negotiable invariants

Make function parameters use a stable per-param home alloca, so they
survive block transitions. Fits the existing entry-block-only alloca
discipline (codegen.rs:2390 GAP-010 guard) and the `build_entry_alloca`
helper at codegen.rs:4547.

Steps:

1. **Entry-block prelude.** For each GC-typed function param `p`,
   allocate a Word-sized alloca `%p.home` in the entry block and
   `store %fn.argN, %p.home` once.
2. **Every read of `p` loads from `%p.home`.** No code path may
   reach for `get_nth_param` again (or for a previously-cached SSA
   value) — this is **invariant A** and must be enforceable, not
   just intentional.
3. `begin_safepoint` keeps its current shape — it stores
   `temp_val(p)` to the slab slot. Since `temp_val(p)` now resolves
   to `load %p.home` (per step 2), it sees the current value.
4. **`end_safepoint` writes the reloaded value back to `%p.home`,
   not just to `state.temps`.** This is **invariant B**: the home
   alloca is the source of truth across block boundaries, so it
   MUST be refreshed by every safepoint reload that returns a moved
   address. Updating only `state.temps` reintroduces the bug we
   just fixed.
5. **Delete the unconditional `state.temps.insert(*p, pv)` at
   codegen.rs:2303-2308.** With the home alloca, there's nothing
   to "restore" — every block-entry use reads from the alloca,
   which already holds the freshest value.

Cost: one alloca + one store at entry + one load per param read at
-O0. mem2reg eliminates the home for non-relocating params at
higher opt levels.

**Open question** (the peer review flagged this): block params'
phi route is correct for SSA-style temps, but anything *derived*
from a function param via a chain that survives a block boundary
needs the same property. If the lowering emits an intermediate temp
`t = p` (assignment, alias) that gets used cross-block, today that
inherits the same bug through the same mechanism. The fix above
breaks the chain at the param itself; the verifier (below) catches
the rest.

## The alloca tracker — the "verifier" that closes this off

Post-codegen pass over the emitted LLVM module. For each function:

- Identify every Word-typed alloca in the entry block (excluding
  the safepoint slab itself).
- For each load from such an alloca, walk backward through the CFG
  to the most-recent dominating store to that alloca. On every
  potentially-allocating call between the store and the load,
  assert the alloca was either (a) reachable through a safepoint
  slab slot whose post-call reload also stored back to the alloca,
  or (b) we cross a fresh store first.
- Failure = name the function, the load instruction, and the
  unprotected call in between.

This makes the invariant **structural**: any future codegen change
that reintroduces the bug shape fails the gate at compile time.
For the fix above, the verifier should report zero violations on
the entire stdlib + parser corpus after invariants A and B are
honored.

## Open dependencies (peer-review notes)

- All moving / relocating opportunities are safepoint-mediated in
  this pipeline — newgc's evacuation runs only at allocation
  failure, which only happens inside the bracketed call. No
  background relocation, no signal-driven safepoints.
- No hidden direct uses of `%fn.argN` after the fix. The verifier
  enforces this — but during the transition, the rebind delete on
  line 2303-2308 needs careful audit: any code path that previously
  relied on `state.temps[p]` being the LLVM function-arg value
  (e.g., for non-GC scalar params, or for ABI thunks) needs to
  either share the alloca pattern or stay explicit.
- `end_safepoint` reaches every relocating call path — verified by
  the existing safepoint-roots dataflow already producing the slab
  for each `is_potentially_allocating_call`. The fix doesn't add
  new call shapes; it just makes the existing protocol persistent.

With two front-end hypotheses ruled out, the remaining suspects shift
toward the runtime / collector layer (and were already listed in the
"Narrowed hypothesis space" section below). Concrete next steps:

1. **`NOD_GC_TRACE` zoomed on the failing vector.** The crash prints
   `sv=0x...771 ptr=0x...770` — capture that exact `ptr` value at every
   `NOD_GC_TRACE` cycle and grep for a `root_rewrite old=ptr new=?` or
   `collect_begin` event. If `ptr` never appears as a root → GC didn't
   know about this object (a slot-map or class-layout bug);
   if it appears but never gets a `new` → GC saw the slot but didn't
   forward it (an evacuator bug). Use `NOD_GC_TRACE_WATCH` to focus on
   the exact pointer.
2. **`NOD_AOT_VERIFY_SAFEPOINTS=1` mid-run.** The existing safepoint
   verifier ran clean at AOT-emit time, but does it run live? If not,
   add per-frame slab-content checking before each
   `nod_aot_begin_safepoint` (the slab should hold non-zero `ptr`
   values that GC can chase). Catch the moment a stale value enters
   the slab.
3. **Class-slot-map audit for `<token>` / AST-node classes.** If a node
   holds the vector in a slot whose `slot-map` doesn't mark it as a
   heap reference, GC won't trace through the node → vector reclaimed.
   Walk the parser's class declarations and confirm every
   `<byte-string>` / `<stretchy-vector>` / `<object>` slot ends up with
   a heap-tagged slot-map entry.

Repro in one command (from workspace root):
```sh
head -n 43 tests/nod-tests/fixtures/jit_cache_sample_items.dylan > F:/scratch/jcs-40.dylan
cargo run -q --bin nod-driver -- parse-dylan F:/scratch/jcs-40.dylan
```

What's already done: global backward live-in/out liveness fixpoint
(`src/nod-dfm/src/liveness.rs`, commit `37e1f69`) — necessary, verified
correct, but not sufficient.

---

## P0 — close the crash (correctness, blocks the parser corpus)

> **A1 (the `note_successor_entry_temps` stale-reload) is REFUTED by direct
> measurement.** Do not implement the SSA-renaming / per-temp-slot rewrite — it
> targets a mechanism that does not fire here. See "What measurement showed"
> below.

### What measurement showed (this round)

Added an env-gated codegen diagnostic `NOD_DIAG_MERGE_DIVERGENCE`
(`src/nod-llvm/src/codegen.rs`, in `note_successor_entry_temps` + post-block
analysis; inert when the env var is unset). It records, per CFG edge, the LLVM
value each predecessor would carry into a successor, then reports any GC-typed
temp that arrives at a block from ≥2 predecessors with **different** values yet
is **not** a block param — the exact A1 stale-reload signature.

Built the parser fresh (cleared `target/nod-jit-cache` + the temp parser EXE;
confirmed `dylan-parser.obj` re-emitted, so `emit_function` ran for every
parser+lexer+stdlib function), ran it on `jcs-40` with the diagnostic on:

- **Zero** GC merge-divergence sites across the entire parser. → A1 refuted.
- `NOD_AOT_VERIFY_SAFEPOINTS=1` on the same run: **no** verifier failure (root
  counts consistent at every site).
- `dump-dfm` of the lexer: every push site protects the vector
  (`nod_stretchy_vector_push(t7, …) safepoint=[t7]`, etc.); `scan-string`'s
  byte-string copy loop carries `safepoint=[t27, t28]`. Roots are **complete**
  where the vector is live.
- Crash is at `collections.rs:1028`, the **first line** of
  `stretchy_vector_push` (`stretchy_vector_fields(sv).expect(...)`): `sv` is
  stale **on entry**. The runtime grow path (1035–1088) correctly re-reads
  `sv_local` after its grow alloc, so the runtime push is **not** at fault — the
  caller handed `push` a dead vector.

### Narrowed hypothesis space (all front-end root-completeness ruled out)

Liveness is complete, the type filter is comprehensive, no merge-divergence, the
verifier passes, the spill/reload slab is symmetric, no `is_no_alloc`
suppression. So the stale `sv` is **not** a missing/ stale front-end root. The
remaining live suspects, in rough order:

1. **A registered root the collector doesn't rewrite in some shape**
   (`newgc-core` evacuation, or an AOT active-frame slab that isn't scanned).
   Needs a `newgc`-side check: was the reclaimed vector's address presented as a
   root at the last collection? If yes → collector didn't rewrite it; if no →
   it was reachable only through a path precise roots don't cover (next item).
2. **The vector lives in a heap object's slot whose class slot-map is wrong**
   (collector doesn't trace/rewrite that slot), so a node holding the vector is
   evacuated but the slot keeps the stale address. A runtime class-layout bug,
   not codegen. The parser builds many AST nodes via `%make`.
3. **`nod_make`/`rust_make` don't root the fresh instance across user
   `initialize`** (A4) — only bites classes with a user `initialize`.

### Runtime ground truth — DONE (vendored GC + `NOD_GC_TRACE` tracer)

We vendored `newgc-core` in-tree (`src/newgc-core`, NewGC HEAD `15b50c6`) and
added an env-gated JSONL collection tracer (`NOD_GC_TRACE=<path>`, commit
`d2b489d`) plus an evacuator rewrite hook (this round). It records, per
collection: the full registered root set (provenance + slot addr + Word),
every **root** rewrite (`visit`), and every **object-field / dirty-card**
rewrite (`visit_cell`) — the last distinguishes a heap-resident slot (object
field) from a native-stack slot (root/safepoint slab). `stretchy_vector_push`
prints the stale `sv` on the failure path so the trace can be correlated.
Conditional `NOD_GC_TRACE_WATCH`/`_FOLLOW` zoom in on one object.

Findings on `jcs-40` (refreshed GC, byte-identical crash):

1. **The collector is faithful.** The stale vector IS a registered root (≈8
   slots). As it relocates across the multi-pass majors, the collector rewrites
   **every** registered slot to the final location (`moved:true` on all). After
   the cycle, all registered roots agree on the new address.
2. **The crash uses the *vacated* address**, held by a reference that was NOT in
   the registered root set at the moving collection.
3. **That reference is NOT a heap object field.** Across the whole run, the
   vector-family value appears in **32 rewrite events, every one with a
   native-stack slot** (`0x71…`, the AOT safepoint slabs); **zero** appear with
   a heap-resident slot (`0x19…`, object fields). The 20 object-field rewrites
   in the trace carry *other* objects.

**Conclusion — the residual is a missing/stale STACK-SLOT or REGISTER root, on
the compiler/runtime side. Refuted by this trace:** (#1) a `newgc-core`
evacuation bug — it faithfully rewrites every registered root; (#2) a wrong
`DylanLayout` slot-map / untraced object field — the vector never lives in an
object field. So the fix is **not** in the GC and **not** in the class layout.

### Bug site located — DONE (`/MAP` + `RtlCaptureStackBackTrace`)

Added two probes:

- **`/MAP` linker flag** in `nod-driver`'s `build` subcommand
  (`src/nod-driver/src/main.rs`) — every AOT EXE now gets a `dylan-parser.exe.map`
  symbol-to-RVA listing alongside it.
- **`RtlCaptureStackBackTrace` probe** at the `stretchy_vector_push` failure
  path (`src/nod-runtime/src/collections.rs`) — when push panics it dumps the
  raw frame IPs (the std `Backtrace` API only emits `<unknown>` without a
  PDB).

Subtracting the ASLR slide (`runtime_IP_in_push - preferred_addr_of_push`
rounded to the 16 KiB page) and looking each IP up in the `.map` produces the
clean call chain from the panic site, top-down:

```
0: stretchy_vector_push + 0x247      [nod_runtime]   ← panic
1: nod_stretchy_vector_push + 0x57   [nod_runtime]   ← C-ABI shim
2: acc-string + 0x144                [dylan-parser]  ← Dylan caller of push
3-7: dump-node + …                   [dylan-parser]  ← recursive AST dump
8: dump-ast + 0xea                   [dylan-parser]
9: nod_user_main + 0x252             [dylan-parser]
10: nod_aot_main_wrapper + 0x18      [nod_runtime]
11: main + 0xe
12: __scrt_common_main_seh
```

**Bug site:** `dump-node` in `tests/nod-tests/fixtures/dylan-parser.dylan`
holds the stretchy-vector accumulator it passes to `acc-string`, and that
local is not registered as a precise root across the `acc-string` call. A
collection fired by `acc-string`'s allocations relocates the vector; the
collector rewrites every *registered* root, but the unregistered stack slot
in `dump-node` keeps the vacated address. Next `dump-node → acc-string →
push` reads that slot and hands push the dead `Word`.

The crash is in the AST *dump* path (after a successful parse of `jcs-40`),
not the parse path — so a workaround is to skip the dump for files that
trigger this, but the fix is in `dump-node`'s lowering / safepoint coverage.

### Fix direction

`dump-dfm` of the parser fixture, find the `acc-string` call site inside
`dump-node`'s IR, inspect its `safepoint_roots` — the stretchy-vector
accumulator should be in there, and isn't. From there:

- If the IR shows the temp **is** live across `acc-string` but absent from
  `safepoint_roots` → liveness gap (verify the global fixpoint sees it).
- If the IR shows the temp is **not** live by liveness's reckoning (e.g.
  rematerialised, or split through a path the dataflow misses) → lowering
  gap; the accumulator needs to be threaded through every block where the
  call can trigger GC.
- If both look correct → codegen reload after the call.

Tooling for any of these is now in place. See `docs/tracing_guide.md` for the
investigation recipe.

### What we learned trying to make a small repro

Three reproducer attempts, saved as fixtures so future investigation can
iterate without re-deriving them (`tests/nod-tests/fixtures/gap011-*.dylan`):

- **`gap011-repro.dylan`** — recursive function passing a `<stretchy-vector>`
  through 1000 levels of recursion, each level pushing 64 bytes into the
  vector twice. **Does NOT crash**, even though it triggers 10 collections
  and 10 k root rewrites. Pure "recursive call + vector accumulator + many
  pushes across collections" is not enough.
- **`gap011-repro2.dylan`** — walks a tree of `<node>` instances (classes
  with slots, generic dispatch via `instance?` + slot accessors),
  pushing each node's label into a buffer. **Does NOT crash** either.
- **`gap011-jcs-min-crash.dylan`** — the smallest *parser-driven* input we
  found that still crashes: the first 35 `s00…s34` functions from
  `jit_cache_sample_items.dylan` (38 lines). At **32 functions the parser
  succeeds; at 35 it crashes** — a 10 %-wide threshold. This is the new
  gating test (smaller and faster than `jcs-40`).

That threshold tells us the bug isn't "lots of pushes across collections"
in general — the simple-pattern repros above clear that bar without
incident. It needs the *specific allocation interleaving* the parser's
`dump-ast → dump-node → acc-string → add!` chain produces, timed so a
moving major collection fires at exactly the wrong point. So the next
session's working hypothesis should be a **codegen/LLVM interaction that
only manifests under that specific interleaving** (a register-allocation
choice that survives the spill/reload contract, an `alloca` that LLVM
treats as non-escaping, or a stdlib runtime path the parser hits at scale
that the small repros don't).

### Echo from the NewGC side (worth keeping in mind)

The Lisp team's `c500539` ("Move clear_all_pins from per-evac to
per-logical-cycle") landed while this investigation was in flight. The
bug there: a per-pass cleanup wiped pin state needed across the **multi-
pass cascade** of a logical major collection — a G1 pin set at the start
of the cycle was empty by the cascade boundary and a live page got
released. The conservative path doesn't affect us (we build
`default-features = false`), but the **pattern** — state that's correct
at every individual pass boundary but wrong across the multi-pass cycle —
is exactly the shape of bug we've been hunting from the other side, and
worth holding next to our findings: every registered root is correctly
rewritten on every pass; one reference still ends up stale. We have not
yet ruled out an analogous "between-pass state loss" on the embedder
(our) side.

**A2. Make the safepoint verifier able to catch a real stale root.**
`NOD_AOT_VERIFY_SAFEPOINTS` (`src/nod-runtime/src/aot.rs:294-320`) checks root
*counts* only — it passed this bug clean. Even a completeness/value-dominance
check wouldn't have caught this (roots ARE complete), so the higher-value add is
a **post-collection slot sanity check**: after evacuation, assert every
registered root slot holds either an immediate or a forwarded/valid wrapper (not
a zeroed/from-space cell). That would fire AT the collection that strands `sv`,
naming the slot.

---

## P1 — correctness-adjacent (latent stale roots, currently masked)

**A3. Give the FFI closure `<environment>` real Minor-GC root visibility, then
delete the band-aid.** Callback registration currently forces a full
`collect_full` to tenure the closure environment so Minor GC won't reclaim it
(`src/nod-runtime/src/callbacks.rs:527,530,601`; Sprint 11d Step E). Same family
as GAP-011 — a live root the collector can't track. Fix the root visibility so
Minor GC preserves it, then remove the `collect_full` (also a real perf win on
every callback register). Do **not** remove the band-aid before the fix.

**A4. Root the fresh instance across user `initialize`.** `nod_make`/`rust_make`
(`src/nod-runtime/src/make.rs:243-253,342-348`) guard the keyword *values* but
not the freshly-allocated *instance* across the `initialize` dispatch, which can
allocate + collect. Narrow scope (only classes with a user `initialize`;
`<stretchy-vector>` has none, so it's not the jcs-40 crash) but a genuine
alloc-while-unrooted hole. RootGuard the instance across the call.

---

## P2 — hardening & robustness (cheap, prevents future regressions)

**A5. Replace `collection_reduce`'s comment-enforced rooting with a structural
guard.** `src/nod-runtime/src/collections.rs:1450-1472`: correctness depends on
the closure reading/writing through `acc_slot` (a RootGuarded stack slot), never
a local copy — enforced only by the comment at 1456-1457. A future refactor to a
local would silently reintroduce a stale-root window. Make the API shape force
slot access (or add a debug assert).

**A6. Decide `mid_evac_oom` policy.** When live data exceeds the 16 MiB
reservation, the GC raises `GcStallError::mid_evac_oom`
(`newgc-core/src/page_heap/evac.rs:908`) and our runtime **aborts**. Decide:
embedder grow-and-retry (raise/auto-grow the reservation) vs. intentional hard
cap. Today it's an unhandled abort masquerading as a crash.

---

## P3 — footprint / leak backlog (real, but not correctness)

**A7. Callback unregister/release semantics.** Trampoline registrations have no
unregistration path (`src/nod-runtime/src/callbacks.rs:496,497`) → unbounded
growth under repeated registration. Define a release/cleanup hook.

**A8. Bound the JIT/dispatch caches.** LLVM engines/contexts and `u64` dispatch
slots are intentionally `Box::leak`'d on cache miss
(`src/nod-sema/src/stdlib.rs:36,311,351`; `src/nod-runtime/src/lib.rs`). Steady
growth under churn; add bounded-LRU or periodic sweep if footprint matters.
(Function-ref / generic-trampoline cells at `src/nod-runtime/src/functions.rs:961,1008`
are also leaked roots, but **bounded** — deduped by `(name,arity)`, one leak per
distinct ref — so likely no action.)

---

## Verified clean — no action needed

- **nod-dfm liveness/passes** — global fixpoint correct on all operand
  categories; no pass drops `safepoint_roots` (`dispatch.rs` does
  `mem::take`+reinstall; `merge_modules` clones whole computations).
- **nod-sema lowering** — all *lexically-named* GC values live across joins/
  back-edges are threaded as block params; cells always reloaded. (The gap is
  *unnamed* intermediates → A1, a codegen issue.)
- **nod-runtime grow path** — `stretchy_vector_push` RootGuards `sv`+`value` and
  re-reads the backing store after the grow alloc; `sv` arrives stale from the
  caller (→ A1).
- **GAP-010 alloca guard** — entry-block alloca placement enforced
  (`codegen.rs:2329,2333`, task #293); safepoint slot slabs don't leak stack.
- **Rc/Arc** — no GC-ownership cycle; `Arc` only for shared immutable text/
  bitmaps (`nod-reader/src/span.rs:50`, `nod-runtime/src/heap_common.rs:114`).

---

## Appendix — crash signature (for confirming a fix is real, not masked)

`gc phase: idle`, modest heap (~0.9 MiB young at the smallest crashing file —
**not** OOM, **not** stack overflow). The faulting `sv` is a valid tagged
pointer whose target wrapper is **all-zero and not a forwarding pointer** =
reclaimed-and-zeroed memory a dead precise root still points at. A correct fix
must make `jcs-40` (and the 6 corpus offenders: `rope`, `ide_rope`,
`ide_syntax`, `nod-ide`, `jit_cache_sample_items`, `gc-rope-file-load`) exit 0,
not merely change the symptom. Earlier minimal repro `F:\scratch\gc-livethrough.dylan`
(missing-root layer) already exits 0 after `37e1f69`.
