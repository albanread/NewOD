//! `nod-llvm` — DFM -> LLVM IR codegen + MCJIT execution.
//!
//! Sprint 07: kernel-subset codegen (i64 / f32 / f64 / bool arithmetic,
//! branches, direct calls, returns) plus a thin JIT wrapper that hands
//! back raw function pointers. No `gc.statepoint`, no opt passes — those
//! land in Sprints 11 and 11/12 respectively.

pub mod cache;
pub mod codegen;
pub mod jit;
pub mod jit_mm;

pub use cache::{
    CacheKey, JitCacheStats, JitReplayResult, NOD_RUNTIME_ABI_VERSION, OPT_LEVEL, ReplayFn,
    cache_entry_count, cache_key, cache_key_for_dfm, cache_max_bytes, cache_size_on_disk,
    clear_cache_dir, default_cache_dir, evict_to, in_process_clear, in_process_contains,
    in_process_get, in_process_insert, read_cache_entry, read_stats, record_hit, record_miss,
    reset_stats, target_triple, write_cache_entry,
};
pub use codegen::{CodegenError, CodegenOutput, FunctionMap, codegen_module};
pub use jit::{Jit, JitError};
