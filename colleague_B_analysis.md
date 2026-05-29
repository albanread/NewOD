Based on an independent review of the AOT safepoint codegen path, the AOT runtime root exposure path, and the NewGC major-collection root visitation path, my best judgment is that the bug is more likely in AOT-generated code preservation/reload than in the GC's root-rewrite logic.

The key reason is structural: the GC does not appear to distinguish between "argument roots" and "non-argument roots" anywhere in the runtime path I reviewed. In the NewGC backend, major collection snapshots roots through `snapshot_roots()`, which merges the thread-local root stack, active JIT safepoints, and active AOT safepoints. For AOT, `snapshot_active_aot_roots()` simply enumerates the active safepoint frame's contiguous slot range `[slot_base, slot_base + expected_root_count)`. Then `visit_roots()` hands each slot to the page evacuator, which rewrites the pointed-at `Word` in place if it evacuates. That machinery is uniform: once a slot address is in the root snapshot, the collector treats it identically regardless of whether the underlying value was also a call argument.

That makes the observed behavioral split highly informative. If the held value survives when passed as an argument to the allocating call, but dies when merely live across that same call, the likely difference is not in the GC's visitor but in the native code shape around the call boundary. Arguments are forced through the platform ABI and are much more constrained in register/stack placement. A non-argument live temp depends entirely on the compiler honoring the safepoint spill/reload contract.

On the codegen side, I did not find any explicit logic that says only call arguments are protected. `begin_safepoint()` spills every temp listed in `safepoint_roots` into the safepoint slot slab, and `end_safepoint()` reloads every rented slot back into the temp map. `emit_direct_call()` and the builtin call helpers all route through the same bracketing path. So if the DFM/IR says `safepoint=[t4]`, the intended contract is clearly that `t4` gets spilled and reloaded whether or not it is an argument.

That said, the AOT path does have one suspicious asymmetry: it publishes the active AOT safepoint frame with `nod_aot_begin_safepoint(site_id, root_count, slot_base)` before it emits the stores of the live temps into the slot slab. The JIT path does the opposite: it spills first, then publishes the active frame. There is also a comment/order mismatch in the AOT codegen path: the comment says "begin -> spill roots -> call -> verify -> end", but the actual emission order is "begin -> spill roots -> verify -> call -> end". I consider that a real bug in instrumentation discipline, but probably not the primary cause of your crash if GC only runs synchronously inside the allocating call itself. It is worth cleaning up because it makes the AOT invariants harder to trust.

So my independent conclusion is:

1. I do not see code evidence that the GC major-collection rewrite is selectively mishandling non-argument transient roots.
2. I do see a plausible failure mode where the AOT/backend-generated machine code fails to preserve or reload a safepoint-spilled non-argument temp correctly at optimization time, while the "passed as argument" version works because the ABI forces the value through a more stable path.
3. The AOT begin/store/verify ordering should still be treated as suspicious and cleaned up, but it looks more like a diagnostic/invariant bug than the main crash trigger.

The next checks I would trust most are:

- Build the repro with AOT optimization disabled. If the crash disappears at `OptimizationLevel::None`, that strongly supports a backend/optimizer issue.
- Compare the native code or object dump for the two variants: "held temp not passed as arg" versus "same temp also passed as arg". The decisive question is whether the safepoint slot store and post-call reload are both materially present in the failing variant.
- Add a focused AOT regression that forces a major GC while a non-argument root is live across an allocating call. The current AOT safepoint tests prove that the runtime frame opens/verifies/closes, but not that a relocated non-argument temp is reloaded correctly after a major collection.

If I had to put money on one side today, I would put it on the AOT codegen/backend side, not on the GC major-rewrite walker.