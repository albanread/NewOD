# Next Steps — May 26 2026

## Current State

Both compiler fixes are in place:
- **GAP-008** (`lower_short_circuit` conservative GC merge): fixed and documented
- **GAP-009** (`lower_while_like` body/exit block ordering): fixed and documented

The GAP-009 fix is landing correctly — all `loop_exit` blocks in the new IR dump now
show `preds = %sc_join*` as expected. However T16 is **still failing** with the same
`phi.t41`/`phi.t51` dominance error, which means there is a **third instance of the
same root cause** in a different function — different block numbers, same pattern.

---

## Steps

1. **Find the failing function** — dump the LLVM IR cleanly and search for
   `phi.t51 = phi i64 [ %phi.t41, %then16 ], [ %phi.t41, %else17 ]`. The surrounding
   blocks will identify which Dylan function / loop / if combination is still broken.

2. **Determine if it's another `lower_while_like` instance or a different lowering site**
   — look at the predecessor chain: what is `then16`'s predecessor? If it's another
   loop exit that came before its sc_join, the fix didn't reach it because there may be
   a second call site (e.g. `until` is lowered differently, or a nested loop).

3. **Check `lower_until` or any other loop-like lowering** — grep `lower.rs` for any
   other place that calls `new_block` with a body/exit pattern before calling
   `lower_expr`.

4. **Apply same fix to any other affected lowering sites**, re-run nod-sema (23/23
   expected), re-run nod-od-suite (7/7 expected).

5. **Once T16 passes**, continue with T17+ Dylan workloads (word frequency counting
   using `<table>`, char frequency counting, full-file report object).

6. **Git commit** all session changes:
   - `src/nod-dfm/src/liveness.rs`
   - `src/nod-llvm/src/codegen.rs`
   - `src/nod-sema/src/lower.rs`
   - `tests/nod-od-suite/fixtures/gc-rope-file-load.dylan`
   - `docs/COMPILER_GAPS.md`

---

## Test Commands

```powershell
# nod-sema unit tests (expect 23/23)
& 'C:\Users\alban\.cargo\bin\cargo.exe' test --manifest-path e:\NewOpenDylan\NewOpenDylan\src\nod-sema\Cargo.toml -- --nocapture

# full integration suite (expect 7/7)
& 'C:\Users\alban\.cargo\bin\cargo.exe' test --manifest-path e:\NewOpenDylan\NewOpenDylan\Cargo.toml -p nod-od-suite -- --nocapture

# LLVM IR dump
& 'C:\Users\alban\.cargo\bin\cargo.exe' run --manifest-path e:\NewOpenDylan\NewOpenDylan\src\nod-driver\Cargo.toml -- dump-llvm tests\nod-od-suite\fixtures\gc-rope-file-load.dylan
```

> **Note**: After editing `lower.rs`, touch the file before running tests to force
> Cargo to detect it as changed:
> ```powershell
> (Get-Item e:\NewOpenDylan\NewOpenDylan\src\nod-sema\src\lower.rs).LastWriteTime = Get-Date
> ```
