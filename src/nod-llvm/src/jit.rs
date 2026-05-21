//! Thin MCJIT engine wrapper around `codegen::CodegenOutput`.

use std::ffi::{CStr, CString};
use std::mem::size_of;
use std::sync::Once;

use inkwell::context::Context;
use inkwell::targets::{InitializationConfig, Target};
use inkwell::values::AsValueRef;
use llvm_sys::execution_engine::{
    LLVMAddGlobalMapping, LLVMCreateMCJITCompilerForModule, LLVMExecutionEngineRef,
    LLVMGetFunctionAddress, LLVMInitializeMCJITCompilerOptions, LLVMLinkInMCJIT,
    LLVMMCJITCompilerOptions,
};

use crate::codegen::{
    CodegenOutput, FORMAT_OUT_SYMBOL, NOD_APPLY_SYMBOL, NOD_CARD_MARK_SYMBOL,
    NOD_COLLECTION_CONCATENATE_SYMBOL, NOD_COLLECTION_SIZE_SYMBOL, NOD_CONDITION_MESSAGE_SYMBOL,
    NOD_DISPATCH_BINARY_SYMBOL, NOD_DISPATCH_SYMBOL, NOD_DISPATCH_UNARY_SYMBOL, NOD_EMPTY_P_SYMBOL,
    NOD_FIP_ADVANCE_SYMBOL, NOD_FIP_CURRENT_ELEMENT_SYMBOL, NOD_FIP_FINISHED_P_SYMBOL,
    NOD_FIP_INIT_SYMBOL, NOD_FUNCALL1_SYMBOL, NOD_FUNCALL2_SYMBOL, NOD_HAS_NEXT_METHOD_SYMBOL,
    NOD_INVOKE_EXIT_SYMBOL, NOD_IS_INSTANCE_OF_SYMBOL, NOD_MAKE_EXIT_PROCEDURE_SYMBOL,
    NOD_MAKE_FUNCTION_REF_SYMBOL, NOD_MAKE_RANGE_SYMBOL, NOD_MAKE_SOV_LEN_SYMBOL,
    NOD_MAKE_STRETCHY_VECTOR_SYMBOL, NOD_MAKE_SYMBOL, NOD_NEXT_METHOD_SYMBOL, NOD_NIL_SYMBOL, NOD_PAIR_ALLOC_SYMBOL,
    NOD_PAIR_HEAD_SYMBOL, NOD_PAIR_TAIL_SYMBOL, NOD_POP_SEALED_CHAIN_SYMBOL,
    NOD_PUSH_SEALED_CHAIN_SYMBOL, NOD_RANGE_BY_SYMBOL, NOD_RANGE_FROM_SYMBOL, NOD_RANGE_TO_SYMBOL,
    NOD_REGISTER_ROOT_SYMBOL, NOD_RUN_BLOCK_SYMBOL, NOD_SIGNAL_SYMBOL,
    NOD_SOV_ELEMENT_SETTER_SYMBOL, NOD_SOV_ELEMENT_SYMBOL, NOD_SOV_SIZE_SYMBOL,
    NOD_STRETCHY_VECTOR_ELEMENT_SETTER_SYMBOL, NOD_STRETCHY_VECTOR_ELEMENT_SYMBOL,
    NOD_STRETCHY_VECTOR_PUSH_SYMBOL, NOD_STRETCHY_VECTOR_SIZE_SYMBOL, NOD_UNREGISTER_ROOT_SYMBOL,
    // Sprint 22 — <table> + hashing.
    NOD_MAKE_TABLE_SYMBOL, NOD_OBJECT_EQUAL_P_SYMBOL, NOD_OBJECT_HASH_SYMBOL,
    NOD_TABLE_ELEMENT_OR_DEFAULT_SYMBOL, NOD_TABLE_ELEMENT_SETTER_SYMBOL, NOD_TABLE_ELEMENT_SYMBOL,
    NOD_TABLE_KEYS_SYMBOL, NOD_TABLE_REMOVE_KEY_SYMBOL, NOD_TABLE_SIZE_SYMBOL,
    NOD_TABLE_VALUES_SYMBOL,
    // Sprint 24 — closures.
    NOD_CELL_GET_SYMBOL, NOD_CELL_SET_SYMBOL, NOD_ENV_CELL_SYMBOL,
    NOD_MAKE_CELL_SYMBOL, NOD_MAKE_CLOSURE_SYMBOL, NOD_MAKE_ENVIRONMENT_SYMBOL,
};
use crate::jit_mm;

#[derive(Debug)]
pub enum JitError {
    Verify(String),
    Create(String),
    NoFunction(String),
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JitError::Verify(s) => write!(f, "LLVM verify: {s}"),
            JitError::Create(s) => write!(f, "JIT engine creation: {s}"),
            JitError::NoFunction(n) => write!(f, "JIT: function `{n}` not found"),
        }
    }
}

impl std::error::Error for JitError {}

fn init_native_target_once() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        unsafe { LLVMLinkInMCJIT() };
        Target::initialize_native(&InitializationConfig::default())
            .expect("Target::initialize_native");
    });
}

/// One JIT session over one LLVM `Context`. Holds engines alive for the
/// process lifetime — see `keep_forever` rationale in NewM2 / NCL.
pub struct Jit<'ctx> {
    _ctx: &'ctx Context,
    engines: Vec<LLVMExecutionEngineRef>,
}

impl<'ctx> Jit<'ctx> {
    pub fn new(ctx: &'ctx Context) -> Result<Self, JitError> {
        init_native_target_once();
        Ok(Self { _ctx: ctx, engines: Vec::new() })
    }

    /// Verify the codegen'd module, install it into a fresh MCJIT engine,
    /// and finalize so symbols become callable.
    pub fn add_module(&mut self, output: CodegenOutput<'ctx>) -> Result<(), JitError> {
        let CodegenOutput { module, .. } = output;
        module.verify().map_err(|e| JitError::Verify(e.to_string()))?;

        // Capture extern declarations BEFORE handing the module off to
        // the engine. After `LLVMCreateMCJITCompilerForModule` owns the
        // module pointer, `module.get_function` is no longer safe.
        let format_out_fn = module.get_function(FORMAT_OUT_SYMBOL);
        let make_fn = module.get_function(NOD_MAKE_SYMBOL);
        let is_inst_fn = module.get_function(NOD_IS_INSTANCE_OF_SYMBOL);
        let dispatch_fn = module.get_function(NOD_DISPATCH_UNARY_SYMBOL);
        let dispatch_binary_fn = module.get_function(NOD_DISPATCH_BINARY_SYMBOL);
        let dispatch_variadic_fn = module.get_function(NOD_DISPATCH_SYMBOL);
        let card_mark_fn = module.get_function(NOD_CARD_MARK_SYMBOL);
        let register_root_fn = module.get_function(NOD_REGISTER_ROOT_SYMBOL);
        let unregister_root_fn = module.get_function(NOD_UNREGISTER_ROOT_SYMBOL);
        let next_method_fn = module.get_function(NOD_NEXT_METHOD_SYMBOL);
        let has_next_method_fn = module.get_function(NOD_HAS_NEXT_METHOD_SYMBOL);
        let push_sealed_chain_fn = module.get_function(NOD_PUSH_SEALED_CHAIN_SYMBOL);
        let pop_sealed_chain_fn = module.get_function(NOD_POP_SEALED_CHAIN_SYMBOL);
        // Sprint 16: `<pair>` / `<list>` builtins. Each lowering emits a
        // `nod_pair_*` / `nod_empty_p` / `nod_nil` extern; we resolve the
        // five symbols to the runtime shims via `LLVMAddGlobalMapping`.
        let pair_alloc_fn = module.get_function(NOD_PAIR_ALLOC_SYMBOL);
        let pair_head_fn = module.get_function(NOD_PAIR_HEAD_SYMBOL);
        let pair_tail_fn = module.get_function(NOD_PAIR_TAIL_SYMBOL);
        let empty_p_fn = module.get_function(NOD_EMPTY_P_SYMBOL);
        let nil_fn = module.get_function(NOD_NIL_SYMBOL);
        // Sprint 19: conditions + block/exception/cleanup shims.
        let signal_fn = module.get_function(NOD_SIGNAL_SYMBOL);
        let run_block_fn = module.get_function(NOD_RUN_BLOCK_SYMBOL);
        let make_exit_fn = module.get_function(NOD_MAKE_EXIT_PROCEDURE_SYMBOL);
        let invoke_exit_fn = module.get_function(NOD_INVOKE_EXIT_SYMBOL);
        let condition_msg_fn = module.get_function(NOD_CONDITION_MESSAGE_SYMBOL);
        // Sprint 20b — collection / FIP / primitive-op shims.
        // Names mirror `nod-llvm::codegen::SPRINT_20B_PRIMITIVES`. Capture
        // the FunctionValues here before MCJIT takes ownership of the
        // module pointer.
        let sprint_20b_extern_decls: Vec<(Option<inkwell::values::FunctionValue<'_>>, *mut std::ffi::c_void)> = vec![
            (
                module.get_function(NOD_COLLECTION_SIZE_SYMBOL),
                nod_runtime::nod_collection_size as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_COLLECTION_CONCATENATE_SYMBOL),
                nod_runtime::nod_collection_concatenate as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_RANGE_FROM_SYMBOL),
                nod_runtime::nod_range_from as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_RANGE_TO_SYMBOL),
                nod_runtime::nod_range_to as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_RANGE_BY_SYMBOL),
                nod_runtime::nod_range_by as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_SOV_SIZE_SYMBOL),
                nod_runtime::nod_sov_size as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_SOV_ELEMENT_SYMBOL),
                nod_runtime::nod_sov_element as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_SOV_ELEMENT_SETTER_SYMBOL),
                nod_runtime::nod_sov_element_setter as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRETCHY_VECTOR_SIZE_SYMBOL),
                nod_runtime::nod_stretchy_vector_size as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRETCHY_VECTOR_ELEMENT_SYMBOL),
                nod_runtime::nod_stretchy_vector_element as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRETCHY_VECTOR_ELEMENT_SETTER_SYMBOL),
                nod_runtime::nod_stretchy_vector_element_setter as *const ()
                    as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRETCHY_VECTOR_PUSH_SYMBOL),
                nod_runtime::nod_stretchy_vector_push as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_FIP_INIT_SYMBOL),
                nod_runtime::nod_fip_init as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_FIP_FINISHED_P_SYMBOL),
                nod_runtime::nod_fip_finished_p as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_FIP_CURRENT_ELEMENT_SYMBOL),
                nod_runtime::nod_fip_current_element as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_FIP_ADVANCE_SYMBOL),
                nod_runtime::nod_fip_advance as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_MAKE_RANGE_SYMBOL),
                nod_runtime::nod_make_range as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_MAKE_STRETCHY_VECTOR_SYMBOL),
                nod_runtime::nod_make_stretchy_vector as *const () as *mut std::ffi::c_void,
            ),
            // Sprint 21 — first-class function values.
            (
                module.get_function(NOD_MAKE_FUNCTION_REF_SYMBOL),
                nod_runtime::nod_make_function_ref as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_FUNCALL1_SYMBOL),
                nod_runtime::nod_funcall1 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_FUNCALL2_SYMBOL),
                nod_runtime::nod_funcall2 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_APPLY_SYMBOL),
                nod_runtime::nod_apply as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_MAKE_SOV_LEN_SYMBOL),
                nod_runtime::nod_make_sov_len as *const () as *mut std::ffi::c_void,
            ),
            // Sprint 22 — <table> + hashing.
            (
                module.get_function(NOD_MAKE_TABLE_SYMBOL),
                nod_runtime::nod_make_table as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_TABLE_SIZE_SYMBOL),
                nod_runtime::nod_table_size as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_TABLE_ELEMENT_SYMBOL),
                nod_runtime::nod_table_element as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_TABLE_ELEMENT_OR_DEFAULT_SYMBOL),
                nod_runtime::nod_table_element_or_default as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_TABLE_ELEMENT_SETTER_SYMBOL),
                nod_runtime::nod_table_element_setter as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_TABLE_REMOVE_KEY_SYMBOL),
                nod_runtime::nod_table_remove_key as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_TABLE_KEYS_SYMBOL),
                nod_runtime::nod_table_keys as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_TABLE_VALUES_SYMBOL),
                nod_runtime::nod_table_values as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_OBJECT_HASH_SYMBOL),
                nod_runtime::nod_object_hash as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_OBJECT_EQUAL_P_SYMBOL),
                nod_runtime::nod_object_equal_p as *const () as *mut std::ffi::c_void,
            ),
            // Sprint 24 — closures.
            (
                module.get_function(NOD_MAKE_CELL_SYMBOL),
                nod_runtime::nod_make_cell as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_CELL_GET_SYMBOL),
                nod_runtime::nod_cell_get as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_CELL_SET_SYMBOL),
                nod_runtime::nod_cell_set as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_ENV_CELL_SYMBOL),
                nod_runtime::nod_env_cell as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_MAKE_ENVIRONMENT_SYMBOL),
                nod_runtime::nod_make_environment as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_MAKE_CLOSURE_SYMBOL),
                nod_runtime::nod_make_closure as *const () as *mut std::ffi::c_void,
            ),
        ];

        // Sprint 20b — capture any extern declarations whose names
        // match a registered method body. The codegen layer declares
        // these for any `DirectCall { callee: "<generic>$<spec>" }`
        // emitted by the dispatch resolver when the body lives in a
        // different JIT module (e.g. the auto-loaded stdlib). We
        // resolve each declared name against the dispatch registry's
        // body-fn-name → address table and bind via
        // `LLVMAddGlobalMapping` below.
        let mut cross_module_method_externs: Vec<(String, inkwell::values::FunctionValue<'_>)> =
            Vec::new();
        {
            let mut maybe = module.get_first_function();
            while let Some(f) = maybe {
                if f.count_basic_blocks() == 0 {
                    let name = f.get_name().to_string_lossy().into_owned();
                    // Skip well-known shim names — they're handled by
                    // the explicit mappings above.
                    if !name.is_empty()
                        && nod_runtime::find_method_body_ptr(&name).is_some()
                    {
                        cross_module_method_externs.push((name, f));
                    }
                }
                maybe = f.get_next_function();
            }
        }

        let mut opts: LLVMMCJITCompilerOptions = unsafe { std::mem::zeroed() };
        unsafe {
            LLVMInitializeMCJITCompilerOptions(&mut opts, size_of::<LLVMMCJITCompilerOptions>());
        }
        // Default O2 — Sprint 16 measurement showed O0 vs O2 makes only
        // a noise-level difference for the current IR shape (the per-call
        // mutex baseline in `nod_register_root`/`nod_unregister_root` is
        // opaque to LLVM and dominates the runtime). Keep at O2 since
        // that's the shipping default; structural perf wins land via
        // Sprint 11c lock-free roots, not opt-level dials.
        opts.OptLevel = 2;
        opts.MCJMM = unsafe { jit_mm::make_mm() };

        let mut engine: LLVMExecutionEngineRef = std::ptr::null_mut();
        let mut err_msg: *mut std::ffi::c_char = std::ptr::null_mut();
        let module_ptr = module.as_mut_ptr();
        let rc = unsafe {
            LLVMCreateMCJITCompilerForModule(
                &mut engine,
                module_ptr,
                &mut opts,
                size_of::<LLVMMCJITCompilerOptions>(),
                &mut err_msg,
            )
        };
        if rc != 0 || engine.is_null() {
            let msg = if err_msg.is_null() {
                "LLVMCreateMCJITCompilerForModule failed".to_string()
            } else {
                let s = unsafe { CStr::from_ptr(err_msg) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { llvm_sys::core::LLVMDisposeMessage(err_msg) };
                s
            };
            return Err(JitError::Create(msg));
        }

        // LLVM owns the module pointer after CreateMCJITCompilerForModule.
        // Forget the inkwell wrapper so it doesn't dispose underneath us.
        std::mem::forget(module);

        // Sprint 10: bind the `format-out` extern shim if the module
        // declared it. The mapping resolves the JIT-side
        // `nod_format_out` LLVM symbol to the runtime address.
        if let Some(f) = format_out_fn {
            let addr = nod_runtime::nod_format_out as *const () as *mut std::ffi::c_void;
            // SAFETY: `engine` was just created above and is non-null;
            // `f.as_value_ref()` is the `LLVMValueRef` of the extern
            // declaration in the module we just installed. The shim's
            // signature matches the IR's `(i64,i64,i64,i64) -> i64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        // Sprint 12: same dance for the class/method runtime shims.
        if let Some(f) = make_fn {
            let addr = nod_runtime::nod_make as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_make is `extern "C" fn(u64,…) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = is_inst_fn {
            let addr = nod_runtime::nod_is_instance_of as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_is_instance_of is `extern "C" fn(u64, u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = dispatch_fn {
            let addr = nod_runtime::nod_dispatch_unary as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_dispatch_unary is `extern "C" fn(u64, u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = dispatch_binary_fn {
            let addr = nod_runtime::nod_dispatch_binary as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_dispatch_binary is `extern "C" fn(u64, u64, u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        // Sprint 13: variadic dispatch shim. Takes
        // (generic_ptr, cache_slot_ptr, arity, a0..a7).
        if let Some(f) = dispatch_variadic_fn {
            let addr = nod_runtime::nod_dispatch as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_dispatch is `extern "C" fn(u64, u64, u64, u64*8) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = card_mark_fn {
            let addr = nod_runtime::nod_card_mark as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_card_mark is `extern "C" fn(u64)`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        // Sprint 11b: precise-roots brackets every potentially-
        // allocating call. The runtime exposes the two C-ABI shims;
        // codegen declares them and we resolve them here.
        if let Some(f) = register_root_fn {
            let addr = nod_runtime::nod_register_root as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_register_root is `extern "C" fn(*mut Word)`,
            // ABI-compatible with the LLVM-side `void (ptr)` signature.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = unregister_root_fn {
            let addr = nod_runtime::nod_unregister_root as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_unregister_root is `extern "C" fn(*mut Word)`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        // Sprint 14: `next-method()` lowers to a call into the runtime
        // shim, which pops the next applicable method from the
        // dispatch chain and invokes it with the parent method's args.
        if let Some(f) = next_method_fn {
            let addr = nod_runtime::nod_next_method as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_next_method is `extern "C-unwind" fn() -> u64`,
            // ABI-compatible with `i64 ()` at the LLVM level.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = has_next_method_fn {
            let addr = nod_runtime::nod_has_next_method as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_has_next_method is `extern "C" fn() -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        // Sprint 15: chain-frame push/pop shims used by SealedDirectCall
        // codegen so `next-method()` walks the fallback chain.
        if let Some(f) = push_sealed_chain_fn {
            let addr =
                nod_runtime::nod_push_sealed_chain_frame as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_push_sealed_chain_frame is
            // `extern "C" fn(*const u64, u64, *const *const u8, u64)`,
            // ABI-compatible with the LLVM-side `void (ptr, i64, ptr, i64)`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = pop_sealed_chain_fn {
            let addr =
                nod_runtime::nod_pop_sealed_chain_frame as *const () as *mut std::ffi::c_void;
            // SAFETY: nod_pop_sealed_chain_frame is `extern "C" fn()`,
            // ABI-compatible with the LLVM-side `void ()`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        // Sprint 16: `<pair>` / `<list>` runtime shims. Each is declared
        // by codegen on demand (`emit_list_builtin_call`) and resolved
        // here. All have `i64 (...)` signatures.
        if let Some(f) = pair_alloc_fn {
            let addr = nod_runtime::nod_pair_alloc as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_pair_alloc` is `extern "C" fn(u64, u64) -> u64`,
            // matching the codegen-side `i64 (i64, i64)` declaration.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = pair_head_fn {
            let addr = nod_runtime::nod_pair_head as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_pair_head` is `extern "C" fn(u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = pair_tail_fn {
            let addr = nod_runtime::nod_pair_tail as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_pair_tail` is `extern "C" fn(u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = empty_p_fn {
            let addr = nod_runtime::nod_empty_p as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_empty_p` is `extern "C" fn(u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = nil_fn {
            let addr = nod_runtime::nod_nil as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_nil` is `extern "C" fn() -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        // Sprint 19 — wire the conditions / block-orchestration shims.
        if let Some(f) = signal_fn {
            let addr = nod_runtime::nod_signal as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_signal` is `extern "C-unwind" fn(u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = run_block_fn {
            let addr = nod_runtime::nod_run_block as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_run_block` is
            // `extern "C-unwind" fn(u64, u64*8) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = make_exit_fn {
            let addr = nod_runtime::nod_make_exit_procedure as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_make_exit_procedure` is `extern "C-unwind" fn(u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = invoke_exit_fn {
            let addr = nod_runtime::nod_invoke_exit as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_invoke_exit` is `extern "C-unwind" fn(u64, u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        if let Some(f) = condition_msg_fn {
            let addr = nod_runtime::nod_condition_message as *const () as *mut std::ffi::c_void;
            // SAFETY: `nod_condition_message` is `extern "C-unwind" fn(u64) -> u64`.
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), addr) };
        }
        // Sprint 20b — bind every primitive shim whose extern was
        // declared in the module. All shims have `extern "C" fn(u64, …) -> u64`,
        // ABI-compatible with the LLVM-side `i64 (i64, …)` declarations.
        for (decl, addr) in &sprint_20b_extern_decls {
            if let Some(f) = decl {
                // SAFETY: `engine` is the live MCJIT engine; `f` is the
                // FunctionValue of the extern declaration; `addr` is the
                // address of a `nod_*` shim with matching ABI.
                unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), *addr) };
            }
        }
        // Sprint 20b — resolve cross-module method body externs. The
        // codegen layer declares any callee that's not in the local
        // function table but IS registered in `nod_runtime`'s dispatch
        // table (a stdlib method's `{generic}${specialisers}` body
        // symbol). Walk the captured externs and bind each via
        // `LLVMAddGlobalMapping` to the JIT'd body pointer.
        for f in &cross_module_method_externs {
            let name = f.0.clone();
            if let Some(ptr) = nod_runtime::find_method_body_ptr(&name) {
                let addr = ptr as *mut std::ffi::c_void;
                // SAFETY: `ptr` is the JIT'd body fn's live address
                // (kept alive by the stdlib's leaked JIT engine); the
                // declaration's `(i64, …) -> i64` signature matches
                // `extern "C" fn(u64, …) -> u64`.
                unsafe { LLVMAddGlobalMapping(engine, f.1.as_value_ref(), addr) };
            }
            // If `find_method_body_ptr` doesn't resolve the name (it
            // was declared but never had a method registered against
            // it), we leave the extern unbound. MCJIT will fail to
            // finalise if the symbol is actually called; this matches
            // the existing UnknownCallee path for callees that
            // don't appear in the dispatch table either.
        }

        self.engines.push(engine);
        Ok(())
    }

    /// Resolve a JIT'd symbol. The caller is responsible for transmuting
    /// the returned pointer to the correct function type.
    ///
    /// # Safety
    /// The returned pointer is only valid while `self` lives and the
    /// caller's transmuted signature must match the JIT'd function.
    pub unsafe fn get_function_ptr(&self, name: &str) -> Option<*const ()> {
        let cname = CString::new(name).ok()?;
        for &engine in &self.engines {
            // SAFETY: `engine` is a valid MCJIT engine; cname is a NUL-terminated string.
            let addr = unsafe { LLVMGetFunctionAddress(engine, cname.as_ptr()) };
            if addr != 0 {
                return Some(addr as *const ());
            }
        }
        None
    }
}
