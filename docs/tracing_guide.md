# GC tracing & AOT-crash symbolication guide

This guide describes the diagnostic infrastructure built for **GAP-011** —
specifically, how to investigate a "stale precise root" crash where the
collector reclaims an object that some live reference still points to. It is
generic to any future bug with the same shape (`stretchy_vector_push: not a
<stretchy-vector>` is the headline, but the same probes work for any
zeroed-wrapper / dangling-pointer crash).

The infrastructure is **inert by default** — nothing fires unless you set an
env var or read a `.map` file. Normal build/test gates are unaffected.

## What's built

### 1. `NOD_GC_TRACE` — JSONL collection tracer

- File: `src/nod-runtime/src/gc_trace.rs` (sink) and `src/nod-runtime/src/heap.rs` (collection seams).
- Activation: set `NOD_GC_TRACE=<path>` in the env of the EXE that runs the
  collections (the AOT parser EXE, an integration test binary — *not* the
  driver; the env propagates to spawned children).
- Output: one JSON object per line, flushed after every line (so an
  abort/`exit 9` mid-cycle never loses a record).

Events:

| `ev`              | Meaning                                                       | Fields                                                  |
| ----------------- | ------------------------------------------------------------- | ------------------------------------------------------- |
| `collect_begin`   | start of a cycle                                              | `seq`, `kind` (`minor`/`major`), `young_alloc`         |
| `root`            | one registered root slot at cycle begin                       | `seq`, `i`, `src` (`stack`/`jit`/`aot`/`values`), `slot`, `word` |
| `root_rewrite`    | the evacuator's `visit` updated a root slot                   | `seq`, `slot`, `old`, `new`, `moved` (bool)            |
| `rewrite`         | *(if the evacuator field-hook is installed)* any pointer rewrite — roots AND object payload fields | same as `root_rewrite`                                  |
| `collect_end`     | end of a cycle                                                | `seq`, `kind`, `minor`, `major`, `young_live`, `old_live`, `promoted` |

Records sharing a `seq` belong to the same cycle.

The `root` event's `src` field distinguishes thread root-stack vs JIT
active-frame slabs vs AOT active-frame slabs vs the multi-values buffer.
This is the provenance — useful for knowing which subsystem the slot belongs
to.

### 2. `NOD_GC_TRACE_WATCH` + `NOD_GC_TRACE_FOLLOW` — zoom-in filtering

Setting `NOD_GC_TRACE_WATCH=0xADDR[,0xADDR…]` restricts `root` and
`root_rewrite` (and `rewrite`) emission to records that touch one of the
watched addresses. Matching is **untagged** (the low tag bit is masked), so
both a tagged Word and a bare pointer hit. Watching a *slot* address also
works. `collect_begin`/`collect_end` are always emitted as scaffolding.

Setting `NOD_GC_TRACE_FOLLOW=1` extends the watch set: any rewrite touching a
watched address adds its `old` and `new` addresses, so a move chain
(`A→B→C…`) stays tracked across passes and cycles without pre-listing every
relocation.

> **ASLR caveat.** Heap addresses differ per process launch. The follow seed
> must come from the **same** run. Within one run, the panic's stale-`sv`
> print *is* in the same process as the trace — seed from there.

### 3. AOT EXE address symbolication

The AOT linker emits a `.map` file (`/MAP` flag added in `nod-driver`'s
`build` subcommand) alongside every `dylan-parser.exe`. This file is the
only way to symbolicate AOT addresses — the EXE itself has no PDB.

To resolve a runtime IP (e.g. from a crash backtrace):

1. **Find the slide.** The map header lists `Preferred load address`; runtime
   is offset by ASLR. Frame 0 of any backtrace inside `stretchy_vector_push`
   is in `nod_runtime::collections::stretchy_vector_push`. Look up its
   preferred address in the map (`grep stretchy_vector_push`), subtract from
   frame 0's runtime IP, round down to the page boundary (16 KiB) → that's
   the slide.

2. **Look up each IP.** `RVA = IP - slide`; find the largest map symbol
   whose preferred address ≤ `RVA`; offset is `RVA - sym_addr`. A small
   offset (`< 0x500`) is a reliable match; a huge offset means the IP is
   inside a smaller function not in your symbol list, falling back to the
   previous symbol — treat it as "somewhere here, but unnamed".

The repo's investigation history has a working perl one-liner for this
(see commit `a4...` notes / `GAP-011_GC_team_writeup.md`). The shape is:

```perl
# parse map → %sym{preferred_addr} = [name, owner]
# for each IP: linear scan @sorted, take max addr ≤ (IP - slide)
```

### 4. `stretchy_vector_push` failure probe

`src/nod-runtime/src/collections.rs` — when push's entry-check fails (the
GAP-011 crash), the failure path prints:

```
[GAP-011] stretchy_vector_push: not a <stretchy-vector>: sv=0xXXXX ptr=0xYYYY
[GAP-011] push caller backtrace (N frames):
  frame  0: 0x...
  frame  1: 0x...
  ...
```

The `sv` hex is what to seed `NOD_GC_TRACE_WATCH` with. The backtrace IPs
(captured via `RtlCaptureStackBackTrace`) symbolicate via the `.map` file —
the first non-runtime frame names the immediate AOT caller.

This is reusable for any other stale-precise-root crash: change the panic
site, leave the structure.

## Worked example: GAP-011 on `jcs-40.dylan`

The headline. From a clean state:

```sh
# 1. clear caches so the parser EXE rebuilds with the current driver
rm -rf "$TEMP"/nod-dylan-parser-* target/nod-jit-cache

# 2. run with full GC trace
NOD_GC_TRACE=F:/scratch/gc.jsonl \
  nod-driver parse-dylan F:/scratch/jcs-40.dylan 2> /tmp/err

# 3. grab the bad sv from the panic
grep "GAP-011.*sv=" /tmp/err
# → sv=0x000001cf568f09e9 ptr=0x000001cf568f09e8
```

The JSONL log has ~400 records over 4 cycles. To zoom in on the stale
vector's lifecycle:

```sh
# (NOT a re-run — addresses change per launch. Grep the SAME file.)
grep -E '(old|new|word)":"0x[0-9a-f]+09e9' F:/scratch/gc.jsonl
# → shows every slot that ever held the vector family, in every cycle,
#   with provenance. Stack slots = registered roots; heap slots = object fields.
```

For the GAP-011 case this revealed: **every** slot that held the vector was
in the native-stack region (`0x71…`, the AOT safepoint slabs), **zero** in
the heap region (`0x19…`, object fields). So the residual was a missing
stack-slot root, not a slot-map / object-field issue.

To name the missing-root frame, the panic's backtrace gets symbolicated
against the `.map`:

```sh
# Map file lives next to the parser EXE.
PDIR=$(ls -td "$TEMP"/nod-dylan-parser-* | head -1)
MAP="$PDIR/dylan-parser.exe.map"
# … perl resolution …
```

That gave the chain (top to bottom of stack):

```
stretchy_vector_push          ← panic
nod_stretchy_vector_push      ← C-ABI shim
acc-string                    ← Dylan caller of push
dump-node (recursive)
dump-ast
nod_user_main
nod_aot_main_wrapper
main
```

So the buggy frame is `dump-node` — it holds the stretchy-vector accumulator
it passes to `acc-string`, and that local isn't kept registered across the
`acc-string` call that triggers a moving collection. From here the fix is a
`dump-dfm` of the parser fixture, finding the `acc-string` call in
`dump-node`'s IR, and inspecting its `safepoint_roots` set.

## Limitations / future work

- **No symbols in the AOT EXE.** Backtrace IPs only symbolicate via the
  `.map` file. Sub-function granularity (line numbers, inline frames) needs
  `/DEBUG` + PDB, which we don't generate yet.
- **Map symbols are sparse.** Some functions don't appear; lookups for those
  IPs land on the previous symbol with a huge `+0x…` offset. Treat
  offsets > a few hundred bytes as unreliable.
- **ASLR caveat for `NOD_GC_TRACE_WATCH`.** The follow seed must come from
  the same process. The push failure's `sv` print is in the same process as
  the trace — that's the canonical seed.
- **Object-field tracing (Step 3) is documented in the GAP-011 writeup but
  the code is not currently committed.** It was used as a one-shot
  experiment to refute the "wrong slot-map" hypothesis (the vendored
  evacuator's `visit`/`visit_cell` got a hook, and `gc_trace` emitted a
  `rewrite` event). The hook trips a Rust-CGU/MSVC archive-member edge case
  that needs a more robust separation of the `nod_user_main` stub before it
  can land as a permanent build. If a future investigation needs per-object
  field rewrites, the recipe is in the writeup.

## Related files

- `src/nod-runtime/src/gc_trace.rs` — JSONL sink + watch/follow.
- `src/nod-runtime/src/heap.rs` — collection seams (begin/end cycle, root
  emit, root-rewrite hook in `visit_roots`).
- `src/nod-runtime/src/collections.rs` — `stretchy_vector_push` failure
  probe (sv hex + `RtlCaptureStackBackTrace`).
- `src/nod-driver/src/main.rs` — `/MAP` linker flag.
- `GAP-011_GC_team_writeup.md` — the investigation narrative, findings,
  refuted hypotheses.
