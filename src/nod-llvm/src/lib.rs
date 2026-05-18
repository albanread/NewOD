//! `nod-llvm` — DFM -> LLVM IR codegen + MCJIT execution.
//!
//! Sprint 07: kernel-subset codegen (i64 / f32 / f64 / bool arithmetic,
//! branches, direct calls, returns) plus a thin JIT wrapper that hands
//! back raw function pointers. No `gc.statepoint`, no opt passes — those
//! land in Sprints 11 and 11/12 respectively.

pub mod codegen;
pub mod jit;
pub mod jit_mm;

pub use codegen::{CodegenError, CodegenOutput, FunctionMap, codegen_module};
pub use jit::{Jit, JitError};
