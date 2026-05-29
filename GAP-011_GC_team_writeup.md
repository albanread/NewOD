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
