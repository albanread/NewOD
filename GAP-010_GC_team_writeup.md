# GAP-010 — precise root goes stale after a MAJOR collection (AOT)

**For:** NewGC team
**From:** NewOpenDylan compiler side
**GC under test:** `newgc-core` pinned at `b66726a17cab9d7bb9c0e836fd535549fd9de30e` (the rev in our root `Cargo.toml`).
**Status:** compiler-side hypotheses ruled out empirically; we need help deciding between **(a)** a major-collection root-rewrite issue in `newgc-core` and **(b)** a frame-active-timing bug on our runtime side. Honest summary: we cannot yet prove it is the GC — but we *have* proven it is **not** our LLVM codegen/optimizer, which is where three independent reviewers initially pointed.

---

## 1. Symptom

A heap reference (`<byte-string>`) held **live across an allocating call inside a loop** becomes a **stale pointer after a MAJOR collection**, in an **AOT-compiled** Dylan EXE. The next use of it faults (raw SIGSEGV). It never happens until enough allocation occurs to trigger an old-generation (major) collection; a minor-only workload is fine.

We are a **precise-roots client**: our JIT/AOT codegen spills live GC roots into a per-call-site stack slot slab, brackets each potentially-allocating call with begin/end-safepoint calls, and after the call **reloads** the (possibly relocated) value from the slot. The collector is expected to **rewrite each root slot in place** to the forwarded address when it evacuates.

## 2. Minimal repro

`F:\scratch\gap010.dylan`:
```dylan
Module: dylan-user
define function churn (k :: <integer>) => (r :: <byte-string>)
  make(<byte-string>, size: 512)            // allocates; result dead immediately
end function;
define function loopy (keep :: <byte-string>) => (total :: <integer>)
  let total = 0;
  let i = 0;
  until (i = 200000)
    let junk = churn(i);                     // allocation -> eventually a MAJOR GC
    total := total + size(keep);             // USE `keep` after the alloc each iter
    i := i + 1;
  end;
  total
end function;
define function main () => ()
  let keep = make(<byte-string>, size: 10);
  format-out(integer-to-string(loopy(keep)));
end function main;
main();
```
Build + run (from the workspace root):
```
cargo build -p nod-driver -p nod-runtime
target/debug/nod-driver.exe build F:/scratch/gap010.dylan -o F:/scratch/gap010.exe
F:/scratch/gap010.exe        # SIGSEGV (exit 139)
```
- `gap010.dylan` (200000 iters) → **SIGSEGV**.
- The 200-iteration variant (`gap010-small.dylan`) → prints the correct `2000` (no major GC).
- `keep` is the only heap value live across `churn`; it is promoted to the old gen long before the first major collection, so a **major** collection is what relocates it.

## 3. How our roots reach you (the integration contract)

On the AOT path, per allocating call site, codegen emits (in `nod-llvm/src/codegen.rs`):
- entry-block `alloca [N x i64] %gc.root.slots` (`init_safepoint_slot_slab`),
- before the call: `store keep -> &slab[0]`, then `nod_aot_begin_safepoint(site_id, root_count=1, slot_base=&slab[0])`,
- the call (`churn`), where allocation — and therefore GC — happens synchronously inside `nod_make`,
- after the call: `nod_aot_end_safepoint(site_id)`, then `reload = load &slab[0]`; all later uses read `reload`.

`nod_aot_begin_safepoint` pushes `{site_id, root_count, slot_base}` onto a thread-local `ACTIVE_AOT_SAFEPOINTS` stack (`nod-runtime/src/aot.rs::begin_aot_safepoint`). At collection time, `nod-runtime/src/heap.rs::snapshot_roots()` builds one `Vec<*const Word>` from the thread-local root stack + JIT maps + `snapshot_active_aot_roots()` (which enumerates `slot_base[0..root_count]` for each active frame) + the values buffer. Both `collect_minor` and `collect_full` then run the **same** closure:
```rust
fn visit_roots(evac: &mut newgc_core::page_heap::PageEvacuator<'_, DylanLayout>, roots: &[*const Word]) {
    for &slot in roots.iter() {
        unsafe {
            let ngc_slot = slot as *mut newgc_core::Word;
            evac.visit(&mut *ngc_slot);          // expected: relocate + rewrite *slot in place
        }
    }
}
```
Our expectation/contract: **`evac.visit(&mut *slot)` rewrites `*slot` to the forwarded address for every slot in the snapshot, during both minor and major collections.** The slot pointers we hand you are raw stack addresses (entry-block allocas in a frame that is still live below the allocation).

## 4. What we have ruled out (with evidence)

1. **Liveness / root selection is correct.** `dump-dfm` shows the `churn` call annotated `safepoint=[t4]` where `t4` is `keep`. The DFM is surface-independent (JIT == AOT).
2. **Codegen wiring is correct.** The store-before / begin / call / end / reload sequence is emitted, and the slot addresses are consistent: `begin`'s `slot_base`, the spill store, and the reload all GEP the **same** entry-block alloca (`&slab[0]`).
3. **It is NOT the LLVM optimizer.** Rebuilding the repro with `OptimizationLevel::None` (-O0) **still segfaults.** With optimization off there is no store-to-load forwarding / dead-store elimination, and the reload instruction provably executes — yet it reads a stale value. (This refutes the initial reviewer consensus, which blamed MemorySSA/DSE eliminating the reload.)
4. **Registration count is correct.** Running with `NOD_AOT_VERIFY_SAFEPOINTS=1` fires no assertion — the registered root count (1) matches at every safepoint.
5. **Single-threaded GC is synchronous.** `nod_safepoint_poll()` is a no-op while no stop is requested; GC runs *inside* `nod_make` during `churn`, i.e. while `loopy`'s safepoint frame is still on the native stack.
6. The fault prints **no Dylan crash dump** (empty stderr), unlike other access violations in our runtime that the SEH handler catches — hinting the fault may occur **inside the collector/evacuator**, or with an already-corrupted stack, rather than cleanly at `size(keep)`.

So: emission, slot addressing, root selection, and registration are all correct, *even at -O0*. The defect is at **runtime, during the major collection**.

## 5. The two remaining possibilities — and what we need from you

**(a) `newgc-core` major-collection root rewrite.** Does `PageHeap::collect_full`'s evacuation **rewrite externally-supplied transient root slots** (the raw `*mut Word` stack addresses we pass via `visit_roots`) the same way it rewrites internally-registered roots — specifically for an **old-generation object being evacuated by a major cycle**? A reviewer reading rev `b66726a` traced `collect_full` (≈ `page_heap/cycle.rs:354`) → `evacuate_with_roots` (≈ `page_heap/evac.rs:559`) → `need_internal_mark` true → mark closure runs `visit_roots`, then `phase2_rewrite` (≈ `:875–896`) re-runs the closure in Rewrite mode. We'd value your confirmation that, in a **major** cycle promoting/evacuating an old object, the slot we hand you is (i) visited in the rewrite phase and (ii) actually written back to the new address. Any path where a transient root is marked-through but not rewritten — or rewritten in a different pass than the one that moves an old-gen object — would explain this exactly.

**(b) Frame-active timing on our side.** It is still possible `loopy`'s AOT safepoint frame is not on `ACTIVE_AOT_SAFEPOINTS` (or `slot_base[0]` doesn't yet hold `keep`) at the precise moment the major collection runs. We will verify this on our side (see §6). We don't want to waste your time if it turns out to be ours.

## 6. Proposed disambiguating experiment (either side can run)

Instrument the collection path to print, **at the start of a MAJOR collection while `loopy` is live**: the active AOT safepoint frames (`site_id`, `root_count`, `slot_base`), the `Word` value at `slot_base[0]` **before** evacuation, and the value at `slot_base[0]` **after** evacuation.
- If the frame is **present** and `slot_base[0]` **changes** across the cycle → the GC rewrote it; the bug is elsewhere on our side (we'll dig).
- If the frame is present but `slot_base[0]` is **unchanged** while the object moved → root-rewrite gap in the major path (**GC side**).
- If the frame is **absent** at major-collection time → frame-active-timing bug (**our side**).

We can add this print in `nod-runtime/src/heap.rs` (the newgc `collect_full` path) since we drive `collect_full`; if you'd prefer to instrument inside `evacuate_with_roots`, the slot addresses we pass are stable for the duration of the cycle.

## 7. Pointers

- Repro: `F:\scratch\gap010.dylan` (+ `gap010-small.dylan`), prebuilt `F:\scratch\gap010.exe`.
- Our runtime: `nod-runtime/src/heap.rs` (`snapshot_roots`, `visit_roots`, `collect_minor`/`collect_full` newgc arms), `nod-runtime/src/aot.rs` (`begin_aot_safepoint`, `snapshot_active_aot_roots`).
- Our codegen: `nod-llvm/src/codegen.rs` (`begin_safepoint` ≈4288, `end_safepoint` ≈4385, slab helpers ≈4444–4512).
- GC (rev `b66726a`): `crates/newgc-core` — `page_heap/cycle.rs` (`collect_full`), `page_heap/evac.rs` (`evacuate_with_roots`, `phase2_rewrite`).
- We are a **precise-only** client (built `newgc-core` with `default-features = false`; conservative-pin compiled out).

---

## 8. UPDATE — disambiguating tests run (supersedes the §5 open question)

We ran the experiments from §6. The results sharpen this considerably and point onto the GC side. (We instrumented `collect_minor`/`collect_full` in our `heap.rs` to dump each snapshot root's slot value before/after every cycle, env-gated; reverted after.)

1. **The GC IS rewriting our root correctly.** The repro has exactly **one** snapshot root (`loopy`'s `keep` slot). Its value **changes across minor collections** — e.g. `0x..13350001 → 0x..13750001 → 0x..13740001 → 0x..13750001` (plausible relocated heap addresses, pointer tag `..0001` preserved), then stabilises once promoted. So the rewrite contract is honored and the frame **is** active at GC time. **This refutes both "root not rewritten" and "frame-active-timing."**
2. **No MAJOR collection runs before the crash** — `collect_full` is never entered. The relocation that matters is the **MINOR cycle promoting `keep` (G0→G1/tenured)**. (Our earlier "major GC" framing was wrong; the trigger is promotion during a minor.)
3. **A pure allocation loop is fine.** The identical 200000-iteration loop with `keep` made dead/unrooted (nothing survives across collections) runs to completion (exit 0). So allocation + minor GC under churn is healthy.
4. **The crash requires a SURVIVING, promoted object.** It happens only when `keep` is live-and-used across the allocating call — i.e. only when an object survives minor cycles and is promoted.

**Sharpened suspect:** the defect is exercised specifically by **promotion / old-generation handling of a surviving object** — not by root rewriting (works) and not by allocation churn (fine). Either the promoted object's relocation/copy is subtly wrong, or the root slot is rewritten to a plausible-but-incorrect address during promotion, so a later use faults. The crash is a raw SIGSEGV with no Dylan crash dump, consistent with a fault inside the collector or on a corrupted/half-promoted object.

**This is now squarely a `newgc-core` question** (rev `b66726a`): does the G0→G1/tenured promotion path correctly relocate a *surviving* object **and** rewrite the externally-supplied transient root slot to its true new address? Our `gc_stress` suite passes in JIT, so either it does not exercise this exact "one long-lived promoted root + heavy churn" shape, or there is an AOT/runtime-path difference worth checking.

**Most decisive next step:** after each minor cycle, deref the rewritten root slot and confirm it still points at a live `<byte-string>` of the expected class/size — the first cycle where that check fails pinpoints the bad promotion/forward. (A debugger on the faulting address would also do it; none is installed on our box, and our review agent was sandbox-blocked from executing.)

---

## 9. GC TEAM RESPONSE — root-caused, with a correction (this is NOT a `newgc-core` bug)

> **Correction — supersedes the earlier draft of this section.** My first
> pass blamed a `newgc-core` dirty-card scan and shipped a fix at
> `22ec0e7`. That fix is real and worth keeping (see *Unrelated hardening*
> below), **but it is not the cause of GAP-010** — your repro crashes
> identically with and without it (you noted this). After reproducing your
> exact shape in-process *and* running the EXE under `cdb`, the true cause
> is on the **compiler + runtime side, not the GC.**

**The crash is a STACK OVERFLOW (`STATUS_STACK_OVERFLOW`, code `0xC00000FD`) — not heap corruption.** Under `cdb`, `gap010.exe` faults with a **single ~992 KB stack frame** that consumes ~99 % of the 1 MB thread stack; the GC's heap allocation during a minor cycle is merely the last straw on an already-exhausted stack. Critically, a stack overflow leaves no stack for `SetUnhandledExceptionFilter` to run — which is exactly why there was **no Dylan crash dump** (your §8.6): the process died silently.

**Root cause — AOT codegen leaked stack inside the loop.** `emit_sealed_direct_call` (`nod-llvm/src/codegen.rs`) emitted its `sd.args` / `sd.chain` scratch `alloca`s at the **loop-body insert point**. At `-O0`, an `alloca` executed every iteration is not reclaimed until the function returns (LLVM never auto-inserts `stackrestore`). `loopy`'s `size(keep)` lowers to a sealed-generic direct call, so each of the 200 000 iterations leaked ~32 bytes; the frame crossed 1 MB at ~31 K iterations — the minor cycle your instrumentation labelled "#4". This explains everything: the 200-iteration variant is fine (tiny leak **and** no GC fires), it reproduces at `-O0` (the leak is structural, not an optimizer artifact), and dropping the `size(keep)` use (`gap010-deadkeep`) removes the only sealed call in the loop and so removes the leak.

**Why §8 pointed at the GC (and why that was a red herring).** Each §8 observation is true but innocent: the root slot *is* rewritten correctly, and `keep` *is* a promoted survivor — it just happens to be live across a loop whose **frame** is leaking. We confirmed the GC's innocence directly: two faithful in-process reproductions on the **real `DylanLayout` heap** — one driving the exact `nod_byte_string_allocate` primitive on the global literal-pool heap with full `nod_runtime_init` and the default 4 MB young — promote a rooted size-10 `<byte-string>` through **200 000** size-512 churns (G0→G1→Tenured, relocated 4×) and re-validate it every cycle: it stays byte-perfect, slot always rewritten. The collector never touches the survivor's payload; only the AOT loop overflows. (Also note `make(<byte-string>, size: N)` zero-fills the payload, and `0` classifies as an immediate — so there was never any pointer-shaped byte to alias in the first place.)

**Fix 1 — compiler (`nod-llvm`).** `emit_sealed_direct_call`'s scratch allocas now go through a new `build_entry_alloca`, which places them in the function **entry block** (executed once, safely reused each iteration since the buffer is consumed synchronously) — the same placement `init_safepoint_slot_slab` already used. `gap010.exe` now runs to completion and prints `2000000`.

**Fix 2 — runtime (`nod-runtime/src/crash_dump.rs`).** Stack overflows are no longer silent. We call `SetThreadStackGuarantee` (reserving 64 KiB so a handler can run after the guard page is hit) and install a **vectored** exception handler that fires first-chance on `STATUS_STACK_OVERFLOW` and prints a dump including the exception address, GC phase, and **safepoint depth** (which immediately fingers runaway recursion / a leaking frame — e.g. our recursion smoke test reports `AOT safepoint frames: 20248`). With the reserve in place the existing unhandled filter now fires for overflow too. Mutator threads can opt in via `ensure_stack_overflow_reserve_this_thread`.

**Unrelated hardening (kept).** The `22ec0e7` object-aware dirty-card scan (`PageEvacuator::visit_card_pointer_cells`) fixes a genuine latent unsoundness — a cell-by-cell card scan would misread a byte-string's opaque payload as candidate pointers. It could never have hit *this* repro (zero-filled payload, nothing to alias), but it is a real correctness fix and stays in, with its isolation test `gap010_card_scan_must_not_treat_byte_payload_as_pointer`.

**Regression coverage.** `nod-runtime` gains two in-process reproductions that promote a rooted `<byte-string>` survivor through 200 000 churns and re-validate it every cycle (`heap.rs::tests::gap010_surviving_bytestring_survives_promotion_under_churn` and `gap010_global_heap_runtime_init_bytestring_churn`); `gap010.dylan` itself now passes end-to-end (`2000000`); and a recursion fixture exercises the new stack-overflow reporter. Follow-up worth doing: a codegen assertion that no `alloca` is ever emitted outside the entry block, so a future loop-body alloca can't silently regress this.

**Action for the NewOpenDylan side:** bump the `newgc-core` pin in your root `Cargo.toml` past **`22ec0e7`** and re-run `gap010.exe` — the SIGSEGV should be gone. *Honest caveat:* our isolation repro manually dirtied the card and planted the aliasing bytes; the mechanism matches your symptom exactly, but the definitive confirmation is **your** repro going green against the fix. If it still faults, send us the §6 pointer-validation-probe output (deref + class/size check) from the *first* failing minor and we'll dig further — but we expect it resolved.

Thanks for the unusually clean report. It found a real coverage gap (byte-payload objects on dirty cards) that our synthetic + cons/vector tests structurally could not.
