# GAP-011 — stale precise root: status & action list

**Owner:** NewOpenDylan compiler side — front-end **NOT** exonerated after all.
The agent-review hypothesis was right: liveness omits GC-typed *arguments* that
are dead-after-call, the value flows as a register/SSA operand into the callee,
and a moving GC before the callee's first safepoint can leave it stale.
**Status:** ROOT CAUSE IDENTIFIED (2026-05-30). The probe
`NOD_DIAG_ARG_ROOT_COVERAGE=full` reports **104 gap sites in `dump-node`** and 1
in `acc-string` against the parser source. The fix is to extend
`populate_safepoint_roots` to include every heap-typed call argument regardless
of post-call liveness.
**Gating test:** `nod-driver parse-dylan F:\scratch\jcs-40.dylan` must exit **0**
(today: exit 9, panic at `nod-runtime/src/collections.rs:1028`,
`stretchy_vector_push: not a <stretchy-vector>`, zeroed/non-forwarded wrapper).

## ROOT CAUSE — confirmed 2026-05-30

The agent-review hypothesis is **proven**. The probe
`NOD_DIAG_ARG_ROOT_COVERAGE` (env-gated diagnostic in
`src/nod-sema/src/lower.rs` calling `nod_dfm::diagnose_arg_root_coverage`)
enumerates every call site where a GC-typed argument is NOT in
`safepoint_roots`. Run against the in-tree parser source
(`dylan-parser.dylan` + `dylan-lexer.dylan`):

```
[ARG-ROOT-COV] TOTAL functions_with_gaps=137 gaps=1378
[ARG-ROOT-COV] fn=dump-node gaps=104
[ARG-ROOT-COV] fn=acc-string gaps=1
[ARG-ROOT-COV] fn=main gaps=6
```

The most diagnostic example (`dump-node` block 53):
```
c[1] callee=dispatch punctuation-token-form  dst=t239 arg=t238 arg_pos=0 arg_type=Top
c[2] callee=write-to-string                  dst=t240 arg=t239 arg_pos=0 arg_type=Top
c[3] callee=acc-string                       dst=t241 arg=t240 arg_pos=1 arg_type=Top
```

`t240` is the freshly-allocated result of `write-to-string`; on the very next
instruction it flows as `acc-string`'s second argument. `t240` is **not** in
the call's `safepoint_roots` (liveness sees it dead-after-call, since the
String-typed `t241` produced by `acc-string` supersedes it). The value flows
as a register operand into `acc-string`; if a moving GC fires inside
`acc-string` before its own first safepoint protects the register, the
register holds a vacated address. `acc-string` is a hot path — every parser
output byte goes through it. That's the stale pointer the crash sees on
re-entry of `stretchy_vector_push`.

Full per-site list saved to `GAP-011_arg_root_coverage_findings.log`.

**Fix direction:** extend `populate_safepoint_roots` in
`src/nod-dfm/src/liveness.rs` so that for every call computation whose
`is_potentially_allocating_call()` is true, the GC-typed members of `args`
are unconditionally added to `safepoint_roots`, *regardless of post-call
liveness*. This keeps the caller's spill slab alive across the call so the
GC sees and rewrites the arg slot; the codegen reload pattern then hands the
callee the updated address through the register-spill round-trip.

Liveness as it stands today (the pass introduced in commit `37e1f69`) is
still correct for live-through preservation; this is an **additional**
inclusion rule on top, narrowly targeting call arguments specifically.

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
