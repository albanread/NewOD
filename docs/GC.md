# NewOpenDylan GC — design stub

*Sprint 01 placeholder. Full design lands ahead of Sprint 11 (GC bring-up).*

The garbage collector for NewOpenDylan is a **precise, generational copying
collector** written in pure Rust, inheriting from
[`E:\CL\NewCormanLisp\docs\GC.md`](../../CL/NewCormanLisp/docs/GC.md). Headlines
(per [MANIFESTO.md](../MANIFESTO.md) §The garbage collector):

- Precise root finding via LLVM `gc.statepoint` (no conservative scanning past
  bring-up).
- Generational copying: young + old + pinned static area for compiled code,
  sealed-class metadata, and the loaded image.
- Multi-threaded mutator, stop-the-world collector. Per-thread TLABs for a
  lock-free allocation fast path.
- Software card-marking write barrier.
- Class metadata is pinned; sealed classes live forever.
- Multimethod dispatch caches participate in collection.

Open questions for the full doc:
- Cons-equivalent headerless layout — what proves "monomorphic at the call site"?
- Interaction with Dylan's `<class>` versus instance object headers.
- Sealing-driven inline allocation optimisation — how aggressively?

See SPRINTS.md Sprints 09–11 for the bring-up sequence.
