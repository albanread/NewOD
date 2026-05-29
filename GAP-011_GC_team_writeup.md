# GAP-011 — precise root goes stale after a MAJOR collection (AOT), redux

**For:** NewGC team
**From:** NewOpenDylan compiler side
**GC under test:** `newgc-core` pinned at `22ec0e7e692ac468fc7682f0aacb9683d01c1688` (the rev in our root `Cargo.toml`).
**Heap config:** young 4 MiB, old 12 MiB, reservation 16 MiB (`nod-runtime/src/heap.rs`: `DEFAULT_YOUNG_BYTES` / `DEFAULT_OLD_BYTES`).
**Status:** real, deterministic, reproduced from a tiny AOT program. We have **not** yet root-caused the layer (our nod-llvm precise-root codegen vs `newgc-core` evacuation root-fixup). This writeup gives the evidence and — importantly — the **negative reproducers** that rule out the simple shapes.

> Relationship to GAP-010: the *symptom* here is exactly what GAP-010 was originally filed under ("heap ref held across an allocating call goes stale after a MAJOR GC"). GAP-010 was ultimately closed by an unrelated fix — AOT sealed-call scratch `alloca`s leaking stack per loop iteration → `STATUS_STACK_OVERFLOW` (the `build_entry_alloca` hoist). That fix made the *byte-string* repro pass, but it addressed a stack-overflow, **not** a stale heap pointer. The crash below is a genuine **stale heap pointer** (zeroed wrapper, `gc phase: idle`, modest heap — **not** a stack overflow, **not** an OOM). So the original precise-root hypothesis appears to be real after all, just masked for the old repro.

---

## 1. How we found it

Sprint 46 brought up a Dylan-in-Dylan **parser**. As a stress exercise we ran the AOT parser EXE (`nod-driver parse-dylan <file>`) over the whole `.dylan` corpus (42 files). Result: 22 parse clean, 14 raise a normal parser `parse-error` (grammar gaps, exit 99), and **6 abort with a Rust panic in the runtime (exit 9)** — all at the *same* site, deterministically.

## 2. Symptom / signature

Faulting site (100% reproducible, identical across all 6 files):

```
thread '<unnamed>' panicked at src\nod-runtime\src\collections.rs:1028:36:
stretchy_vector_push: not a <stretchy-vector>
... -> panic in a function that cannot unwind -> abort  (exit 9)
```

`collections.rs:1028` is the **first** line of `stretchy_vector_push`:
```rust
let (length, capacity, storage) =
    stretchy_vector_fields(sv).expect("stretchy_vector_push: not a <stretchy-vector>");
```
i.e. the `sv` argument is rejected by the class check on entry. The push primitive is already defensive about *mid-call* evacuation (it RootGuards `sv`/`value`/`new_storage` and re-reads `storage` after the grow alloc) — but here `sv` is **already bad on entry**, so the **caller** handed it a stale Word.

### Instrumented `sv` (temporary `eprintln`, since reverted)

```
STRETCHY-PUSH-BAD sv=0x00000237e8e12761 ptr=0x237e8e12760
                  wrapper=0x0000000000000000 class=ClassId(0)
                  expected stretchy=ClassId(1043)
```

Reading it out:
- `sv` is a **valid tagged heap pointer** (`…761`, pointer tag; target `…760`).
- The **wrapper word at the target is all-zero** → `class = ClassId(0)`, and it is **NOT** a forwarding pointer (the `Forwarded` gc-bit is clear).
- Expected class is `<stretchy-vector>` = `ClassId(1043)`.

A **zeroed, non-forwarded** wrapper means `sv` points at memory the collector **reclaimed and zeroed** — the object that was there is gone, and `sv` is a **dangling precise root** that was never fixed up. (If it had been *evacuated*, we'd expect a forwarding wrapper, not zeros.)

### Heap state at the crash (from the crash dumper)

`gc phase: idle` (the bad access happens *after* a collection, not during one), and for the smallest crashing file:
```
minor collections : 3      major collections : 3
young allocated    : 904528 bytes      bytes promoted : 773424 bytes
```
→ **~0.9 MiB** total — *far* under the 16 MiB reservation. **Not OOM.** The corruption correlates with the **major** (evacuating) collections.

## 3. Reproducer (small, deterministic, one command)

The minimal *input* we have is the first **40** functions of an existing fixture — all trivial (`define function sNN (x :: <integer>) => (<integer>) x + sN-1(x) end;`, plus some `if/else`). No exotic syntax; every construct is individually tested and works. It is pure **volume** that trips it.

```sh
# from workspace root
head -n 43 tests/nod-tests/fixtures/jit_cache_sample_items.dylan > F:/scratch/jcs-40.dylan
cargo run -q --bin nod-driver -- parse-dylan F:/scratch/jcs-40.dylan
# -> exit 9, panic at collections.rs:1028 (stretchy_vector_push)
```
Bisected: 40 functions crash; the full 160 crash; all 6 corpus offenders (`rope`, `ide_rope`, `ide_syntax`, `nod-ide`, `jit_cache_sample_items`, `gc-rope-file-load`) hit the identical site.

### Second symptom on the same input (likely same corruption)

`nod-driver dump-dylan-tokens F:/scratch/jcs-40.dylan` (the Dylan **lexer**, run alone) does **not** crash — it **spins** (observed ~1500 CPU-seconds before we killed it). Consistent with a stale/zeroed length field being read as a garbage-large loop bound rather than faulting. Same family, different manifestation.

## 4. What we ruled out — negative reproducers

We tried to reduce the crash to a standalone (no-parser) AOT program. **Three** natural shapes all **run to completion (exit 0)** even at high iteration counts and with multiple major GCs — so the bug is **not** any of these in isolation:

1. **Single-frame loop, shallow inline alloc** — hold a `<stretchy-vector>` local, push `make(<tok>)` directly, 300 000 iters. *Passes.* (This is the GAP-007 shape; its fix holds.)
2. **Local held across a DEEP transient-churn call** — `acc` held across `churn(100)` (a callee that allocates 100 transient toks and returns a fixnum), 20 000 iters, modest live set. *Passes.*
3. **Slot-indirection across a deep call** — hold an *object* `b` whose slot holds the vector, push via `box-vec(b)` across the same deep churn. *Passes.*

(A fourth attempt that *retained* 19 MiB of live data crashed in `newgc-core/src/page_heap/evac.rs:908` = `GcStallError::mid_evac_oom` — that is a **legitimate OOM** (live > 16 MiB reservation), a *separate* robustness gap: our runtime aborts on the GC's stall/grow signal instead of growing the reservation. Not this bug.)

**Conclusion from the negatives:** the trigger needs the parser's *combined* structure that none of the toy shapes have — recursive descent (`parse-body → parse-constituent → parse-definition → parse-body …`) holding **several** heap roots live simultaneously (token-stream, tokens vector, the body being built, the current node) across allocating calls that span a major GC. That points at either:
- **(a) nod-llvm precise-root codegen** under-spilling/under-reloading one of multiple simultaneous roots across a call that triggers a major collection (register pressure / phi-wiring at the recursion edges), or
- **(b) newgc-core** not rewriting a registered root slot during old-gen evacuation in some case the toy repros don't exercise.

`NOD_AOT_VERIFY_SAFEPOINTS=1` did **not** flag a bad root at the failing safepoint, which mildly favours (b) or a root that's missing from the table entirely rather than mis-valued — but that's not conclusive.

## 5. What we are a (precise-roots) client of

Our JIT/AOT codegen spills live GC roots to per-call-site stack-slot slabs, brackets allocating calls with `nod_aot_begin_safepoint` / `nod_aot_end_safepoint`, and reloads (possibly relocated) values after the call. The collector is expected to rewrite each registered root slot in place to the forwarded address during evacuation. The zeroed (non-forwarded) wrapper says the object at `sv` was reclaimed without that root being updated.

## 6. Ask

1. With the repro in §3 (and `NOD_GC_DIAG=1` available), can you tell from the `newgc-core` side whether the failing object's address was ever presented as a root at the last major collection? If yes → it's our codegen missing a spill/reload of *that* root; if no → it's a missing root in our table (also ours) **or** an evacuation that zeroed a from-space cell a live root still pointed at.
2. Is `mid_evac_oom` (§4 parenthetical) meant to be caught and handled by the embedder (grow + retry), or is hitting it always a client bug (over-retention)? Our runtime currently lets it abort.

Repro files staged under `F:\scratch\`: `jcs-40.dylan` (crashes), `gc-major-churn.dylan` / `gc-stale-root.dylan` / `gc-stale-slot.dylan` (the three negatives), `gc-deep-churn.dylan` (the OOM).

---

## ADDENDUM — ROOT CAUSE CONFIRMED: it is neither (a) nor (b). It is us — the liveness pass.

Follow-up analysis (credit: a colleague's review) located the bug, and a
**compile-time `dump-dfm`** check plus a **minimal standalone reproducer**
confirm it. Neither the collector nor nod-llvm spill is at fault: **a root is
simply never registered** because `nod-dfm`'s liveness pass cannot see
*live-through* temps.

### The bug

`populate_safepoint_roots` (`src/nod-dfm/src/liveness.rs`) is a **per-block
approximation** with **no global live-in/live-out fixpoint** (see its own
module doc, and `compute_escaping_temps`, which only extends a temp's range
within its *defining* block). In Pass 3 it iterates only `def_index`, which
holds: function params, block params, and temps **defined or used in that
block**. A temp that is **live-in and live-out of a block but neither
mentioned nor threaded through it** is invisible there, so it is omitted from
the `safepoint_roots` of every allocating call in that block.

The GAP-008 merge-threading (`lower_if` / `lower_while_like`) hides this for
straight-line and single-branch code by threading env bindings through
**join/header** block params — but **arm blocks never get those params**, so an
allocating call in an arm, followed by nested control flow, with an outer GC
local live across it, falls straight into the hole. Recursive-descent parsers
are saturated with that shape; the toy repros above were not — which is
exactly why they passed.

### Compile-time proof (no runtime, no crash)

`F:\scratch\gc-livethrough.dylan` — `acc` is a GC local defined in the entry
block, first used only *after* an outer `if`; the then-arm makes a heavy
allocating call (`churn`) then branches again:

```
nod-driver dump-dfm F:\scratch\gc-livethrough.dylan
```
```
fn step (t0: <integer>) -> <integer>:
  entry:
    t2: <top> = DirectCall nod_make_stretchy_vector(t1)   ; t2 = acc
    ...
    If t4 then1 else2
  then1:
    t6: <integer> = DirectCall churn(t5)                  ; ALLOCATES — no safepoint=, t2 omitted
    ...
    If t8 then3 else4
  ...
  join6(t16: <top>, t17: <top>):                          ; t17 = acc, threaded in
    t21: <top> = DirectCall %make(...)  safepoint=[t17]
    t22: <top> = DirectCall nod_stretchy_vector_push(t17, t21)  safepoint=[t17]
```

`t2` (acc) is live across `churn` in `then1` (it reappears as `t17` in
`join6`), yet `churn`'s `safepoint_roots` is empty — `t2` is not a param of
`then1`, not defined there, not used there. **Missing root, proven at compile
time.** (Merge blocks `then3`/`join6` *do* list it — `safepoint=[t2, t7]` /
`[t17]` — confirming the hole is specifically the arm with the alloc.)

### Runtime seal

Building and running `gc-livethrough.dylan` crashes at
**`collections.rs:1028`** (`stretchy_vector_push` on stale `acc`) — the *same
site* as the parser — at **1 minor / 0 major collections, ~4 MB young**.
Removing the alloc-then-nested-branch shape makes it exit 0.

> **Refinement to §2:** the fault is **not** major-collection-specific. It
> bites on the **first relocating collection (minor included)** that fires in
> the unprotected window. The parser showed majors only because it allocates
> more before hitting an unprotected window; the mechanism is the same.

### Why every earlier signature still matches

* **Zeroed, non-forwarded wrapper / clean `NOD_AOT_VERIFY_SAFEPOINTS`:** the
  slot was *never registered*, so the collector never forwarded it (correct
  behaviour), and the verifier (which only checks registered ⊆ live) cannot
  detect a *missing* root. Both consistent.
* **(a) ruled out:** nod-llvm faithfully spills/reloads the roots it is given;
  it was given too few. **(b) ruled out:** newgc-core correctly leaves
  unregistered cells alone.

### Fix direction (our side, `nod-dfm`)

Replace the per-block approximation in `liveness.rs` with a real backward
**live-in/live-out dataflow fixpoint**, and feed *that* into
`safepoint_roots`. (Also worth: have `NOD_AOT_VERIFY_SAFEPOINTS` additionally
assert *completeness* — every GC-typed temp live across a call is registered —
so a too-small set can't pass silently.)

New repro: `F:\scratch\gc-livethrough.dylan` (confirmed crash + the `dump-dfm`
above).

---

## ADDENDUM 2 — liveness fix LANDED and verified, but a SECOND layer remains (codegen). Full subsystem sweep.

The fix from ADDENDUM 1 is done and is **necessary but not sufficient**. A
read-only sweep of every subsystem on the root lifecycle (four independent
reviewers: nod-dfm, nod-sema, nod-runtime, nod-llvm) has now triangulated the
residual to a specific codegen line. **No code has been changed in this round —
this is a findings dump for triage.**

### What the liveness fix did (committed `37e1f69`, `src/nod-dfm/src/liveness.rs`)

Replaced the per-block approximation with a real **global backward
live-in/live-out dataflow fixpoint** (`compute_global_live_out` +
`live_after_per_computation`), feeding `live_after` into `populate_safepoint_roots`.
Verified:
- `gc-livethrough.dylan` (ADDENDUM 1's compile-time + runtime repro) now exits **0**.
- nod-dfm unit tests pass (`fixnum_args_do_not_register_as_roots`,
  `live_pointer_across_call_gets_protected`).
- `dylan_parser` regression suite green (25/25) — no codegen regression.

### …but `jcs-40` still crashes — identical `collections.rs:1028` signature

After the fix, with **both** caches cleared and a fresh parser EXE rebuilt,
`F:\scratch\jcs-40.dylan` still aborts (exit 9) at `stretchy_vector_push: not a
<stretchy-vector>`, zeroed wrapper. So the missing-root layer is fixed; a
**stale-reload** layer remains.

### Sweep result — the four subsystems, ranked

The reviewers independently converged. Three subsystems are **clean for this
signature**; the fourth holds the residual.

| Subsystem | Verdict for GAP-011 residual | Evidence |
|---|---|---|
| **nod-dfm** liveness/passes | **CLEAN** — fixpoint correct on all operand categories, block-params-in-kill, back-edge re-iteration, live-after semantics, unreachable blocks. No later pass drops `safepoint_roots` (`dispatch.rs` does `mem::take`+reinstall at 113/141/155; `merge_modules` clones whole computations). | `liveness.rs:78,124-127,162-213`; `dispatch.rs:113,141,155`; `lib.rs:1222-1230` |
| **nod-sema** lowering | **CLEAN** — every *lexically-named* GC value live across a join/back-edge is threaded as a block param; cells always reloaded (`nod_cell_get`/`set` on every access). Lowering never sets `safepoint_roots` (always `Vec::new()`). NB: `for` is *rejected* at lowering, so the parser's new `for` can't reach codegen. | `lower.rs:6240-6341,6090-6201,6367-6506,4466-4488,4994-5034` |
| **nod-runtime** root bookkeeping | **CLEAN at the crash site** — `stretchy_vector_push` RootGuards `sv`+`value` and **re-reads** the backing store after the grow alloc (`collections.rs:1031,1033,1046,1048`). `snapshot_active_aot_roots` correct (`aot.rs:341-356`). `sv` is therefore stale **on entry** — the caller's frame handed it a dead Word. | `collections.rs:1026-1112`; `aot.rs:341-356`; `closures.rs:289-320` |
| **nod-llvm** codegen | **RESIDUAL LIVES HERE** — see below | `codegen.rs:2157,2282-2302,2359-2367,4266-4267,4290,4300,4448` |

### Root cause of the residual: non-dominating reload propagated to successors

The first guess (a function-wide `self.temps` never reset) was **refuted**:
`emit_function` rebuilds `self.temps` per block from `block_entry_temps`
(`codegen.rs:2282-2302`), re-pins params and phis, and phi incomings are
captured eagerly at jump-emit time (`pending_incoming`, the GAP-007 fix). The
`Jump`-with-args/phi path is sound.

The actual hole is **`note_successor_entry_temps`** (`codegen.rs:2359-2367`,
called for both `If` arms at `4266-4267` and `Jump` targets at `4290`). After
`end_safepoint` rebinds a root to its reload SSA value (`codegen.rs:4448`), this
helper snapshots the **entire** `self.temps` — including that reload — into each
successor's `block_entry_temps`, for **all** temps, not just declared block
params (first-writer-wins `or_insert`). Consequence:

1. Block A allocates; `end_safepoint` binds `t` → `gc.sN.reload.tN` (an instr in A).
2. A ends in `If`/`Jump` to B; `note_successor_entry_temps(B)` copies A's reload SSA into `block_entry_temps[B][t]`.
3. B installs it as its binding for `t`. If `t` is **not** a block param of B, the phi re-pin (`2298-2302`) does *not* overwrite it, so B's uses read A's reload directly.
4. If B is **not strictly dominated by A** (any join reachable from another predecessor, or a back-edge — exactly what a 40-function recursive-descent parser produces), that's a dominance-invalid reference. After a *later* collection between A and B's use, it's a pre-relocation (stale) `Word` → zeroed wrapper → the crash.

This is the **unnamed-intermediate-SSA-temp** class: sema threads all *named*
(lexical) live values as block params, but a transient like a freshly-`make`'d
object held while the next argument is evaluated is **not** env-named and **not**
threaded — so it leaks across a block boundary via this fallback. That reconciles
"sema is clean" with "codegen has the bug": the scheme is correct **only if**
lowering threads *every* cross-GC-live heap temp as an explicit block param; the
`note_successor_entry_temps` fallback silently masks a non-threaded temp with a
non-dominating reload.

### Why the verifier still passes it

`NOD_AOT_VERIFY_SAFEPOINTS` (`aot.rs:471-512,519-581`) asserts root **counts** and
stack-discipline site-ids only — never which SSA value the consumer reads. A
dominance-invalid/stale reload passes verification clean. (ADDENDUM 1 already
recommended adding *completeness*; this round adds: it should also check reload
**value** dominance, or it will keep missing this class.)

### Fix direction (our side — DEFERRED, not done this round)

Two viable paths:
1. **Preferred / matches in-progress #300 + #298:** make lowering thread every
   `live_in` heap temp (including unnamed intermediates) as an explicit block
   param/phi, **and** make codegen's `note_successor_entry_temps` refuse to
   propagate non-param temps (fail loud rather than silently install a
   non-dominating reload).
2. **Codegen-local:** reload each root from a stable per-temp frame slot via a
   `load` at the **entry of every block whose `live_in` contains the temp**, not
   just the block that made the call — so every use loads a dominance-valid
   post-relocation value.

### Adjacent findings from the sweep (not this crash; logged for triage)

These are real but **do not** corrupt the heap — they are footprint/perf, except
where noted:

1. **(Correctness-adjacent) FFI closure `<environment>` root visibility** —
   callback registration currently forces a full `collect_full` to tenure the
   closure environment so Minor GC won't reclaim it (a band-aid, also Sprint 11d
   Step E). This implies the FFI environment is **not** a Minor-GC-visible root.
   Same *family* as GAP-011 (a live root the collector can't track), different
   site. Removing the band-aid without first fixing root visibility would
   reintroduce a dangling environment. `callbacks.rs`.
2. **FFI callback trampolines never unregister** (leak by design — no cleanup
   path for Dylan closures used as C-callable trampolines). Unbounded growth
   under callback churn. `callbacks.rs`.
3. **`alloca` in loop bodies (GAP-010 family)** — native stack leak per
   iteration at -O0; the per-call-site safepoint slot slabs are allocas, so worth
   confirming the hoist (`build_entry_alloca` / task #293 guard) covers the slot
   machinery too. Footprint, not heap corruption. `codegen.rs`.
4. **JIT/dispatch cache `Box::leak`** — LLVM engines/contexts and `u64` dispatch
   slots are intentionally leaked on cache miss; steady growth under churn.
   Bounded-LRU/periodic-sweep is the fix. `nod-sema/lib.rs`, `nod-runtime/lib.rs`.
5. **`nod_make`/`rust_make` don't root the fresh instance across user
   `initialize`** (`make.rs:243-253,342-348`) — a genuine alloc-while-unrooted
   hole, but only for classes with a user `initialize`; `<stretchy-vector>` has
   none, so it cannot produce *this* signature. Separate correctness item.

`jcs-40.dylan` remains the live repro. No further code touched this round.

### Supplement — second source-validated sweep (line-level corroboration + new items)

A second, independent re-sweep validated findings directly in source. It
corroborates the above and adds two items worth tracking. **Status
correction first:**

> **GAP-011 is NOT closed.** The second sweep, reading only
> `nod-dfm/src/liveness.rs`, marked GAP-011 "fixed." That is true of the
> *liveness layer* (the global fixpoint landed and is correct) but **not** of
> the end-to-end bug: `jcs-40.dylan` still aborts at `collections.rs:1028`. The
> codegen residual above (`note_successor_entry_temps`) is the open layer. Do
> not treat GAP-011 as resolved until `jcs-40` exits 0.

New / corroborated items (verified by reading the cited lines):

- **(Medium, new) `collection_reduce` rooting invariant is comment-enforced.**
  `collections.rs:1450-1472`: `acc_slot` is a stack slot RootGuarded at 1459;
  the closure must read/write *through the slot*, never a local copy, or the
  collector's pointer-rewrite is defeated. Enforced only by the comment at
  1456-1457. A future refactor to `let acc = ...; f(acc, ...)` would silently
  reintroduce a stale-root window. Worth a structural guard, not just a comment.
- **(Medium, new but bounded) Function-ref / generic-trampoline cells are
  leaked roots.** `functions.rs:961` (`make_function_ref`) and `:1008`
  (`make_generic_trampoline_ref`) both `heap_register_root(Box::leak(Box::new(w)))`.
  This is **bounded**, not a churn leak: `FUNCTION_REF_CACHE` dedupes by
  `(name, arity)`, so each distinct ref leaks exactly once for process life.
  Retention contributor + manual-rooting lifecycle risk, but not unbounded.
- **(Corroborates) `NOD_AOT_VERIFY_SAFEPOINTS` is off by default outside
  tests.** `aot.rs:294-320`: env-gated in release, on-by-default only in the
  test binary (`VERIFY_ENABLED_FOR_TESTS`), because the check costs a global
  lock + Vec clone per Dylan call (~20k allocs/WM_PAINT). And it verifies root
  *counts* only — so it cannot catch the codegen residual even when enabled.
  Recommendation: keep it on in CI/debug lanes; add value-dominance checking if
  we want it to catch the residual class.
- **(Info, confirms) GAP-010 entry-block alloca guard is present and strong.**
  `codegen.rs:2329,2333` (task #293) enforces entry-block alloca placement, so
  the per-call-site safepoint slot slabs do not leak native stack per loop
  iteration. No action.
- **(Info) No Rc/Arc GC-ownership cycle risk.** `Arc` is used only for shared
  immutable text/bitmaps (`nod-reader/src/span.rs:50`,
  `nod-runtime/src/heap_common.rs:114`), never for Dylan-object ownership graphs
  (those are GC-traced). No reference-cycle leak path.

Net: the second sweep changes no conclusion — the open correctness item is the
codegen residual; everything else is footprint/perf or comment-discipline
fragility. `jcs-40.dylan` is still the gating repro.
