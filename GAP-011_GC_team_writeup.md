# GAP-011 — stale precise root: status & action list

**Owner:** NewOpenDylan compiler side (root cause is ours, not `newgc-core`).
**Status:** OPEN. The liveness layer is fixed; a codegen layer still crashes.
**Gating test:** `nod-driver parse-dylan F:\scratch\jcs-40.dylan` must exit **0**
(today: exit 9, panic at `nod-runtime/src/collections.rs:1028`,
`stretchy_vector_push: not a <stretchy-vector>`, zeroed/non-forwarded wrapper).

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

**A1. Fix the codegen stale-reload residual.**
`note_successor_entry_temps` (`src/nod-llvm/src/codegen.rs:2359-2367`, called at
`4266-4267`/`4290`) snapshots the *entire* `self.temps` — including a
post-`end_safepoint` reload SSA value (`codegen.rs:4448`) — into every
successor's entry temps, for **all** temps, not just declared block params. For
an *unnamed intermediate* heap temp that lowering didn't thread as a block
param, a successor not strictly dominated by the call block reads a
non-dominating, pre-relocation `Word` → stale root after the next collection.
Pick one:
- **(preferred, matches #298/#300)** make lowering thread every `live_in` heap
  temp (incl. unnamed intermediates) as an explicit block param, **and** make
  `note_successor_entry_temps` refuse non-param temps (fail loud instead of
  installing a non-dominating reload).
- **(codegen-local)** reload each root from a stable per-temp frame slot via a
  `load` at the entry of every block whose `live_in` contains the temp — not
  only the block that made the call.

Done-test: `jcs-40` exits 0 **and** `dylan_parser` suite stays 25/25.

**A2. Make the safepoint verifier able to catch this class.**
`NOD_AOT_VERIFY_SAFEPOINTS` (`src/nod-runtime/src/aot.rs:294-320`) checks root
*counts* only and is off by default outside tests, so it passed this bug clean.
Add reload-**value dominance** checking, and keep verification on in CI/debug
lanes. (Cheap insurance against A1 regressing.)

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
