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

use crate::symbols::{ModuleManifest, RelocKind};
use crate::codegen::{
    CodegenOutput, FORMAT_OUT_SYMBOL, NOD_APPLY_SYMBOL, NOD_CARD_MARK_SYMBOL,
    NOD_COLLECTION_CONCATENATE_SYMBOL, NOD_COLLECTION_SIZE_SYMBOL, NOD_CONDITION_MESSAGE_SYMBOL,
    NOD_DISPATCH_BINARY_SYMBOL, NOD_DISPATCH_SYMBOL, NOD_DISPATCH_UNARY_SYMBOL, NOD_EMPTY_P_SYMBOL,
    NOD_FIP_ADVANCE_SYMBOL, NOD_FIP_CURRENT_ELEMENT_SYMBOL, NOD_FIP_FINISHED_P_SYMBOL,
    NOD_FIP_INIT_SYMBOL, NOD_FUNCALL0_SYMBOL, NOD_FUNCALL1_SYMBOL, NOD_FUNCALL2_SYMBOL,
    NOD_FUNCALL3_SYMBOL, NOD_FUNCALL4_SYMBOL, NOD_FUNCALL5_SYMBOL, NOD_HAS_NEXT_METHOD_SYMBOL,
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
    // Sprint 28 — Win64 FFI trampolines.
    NOD_WINFFI_CALL_0_SYMBOL, NOD_WINFFI_CALL_1_SYMBOL, NOD_WINFFI_CALL_2_SYMBOL,
    NOD_WINFFI_CALL_3_SYMBOL, NOD_WINFFI_CALL_4_SYMBOL, NOD_WINFFI_CALL_5_SYMBOL,
    NOD_WINFFI_CALL_6_SYMBOL, NOD_WINFFI_CALL_7_SYMBOL, NOD_WINFFI_CALL_8_SYMBOL,
    NOD_WINFFI_CALL_9_SYMBOL, NOD_WINFFI_CALL_10_SYMBOL, NOD_WINFFI_CALL_11_SYMBOL,
    NOD_WINFFI_CALL_12_SYMBOL,
    // Sprint 32 — closure-to-C-callback trampoline registration.
    NOD_REGISTER_WNDENUMPROC_SYMBOL, NOD_REGISTER_WNDPROC_SYMBOL,
    // Sprint 34 — <c-struct> field accessor primitives.
    NOD_STRUCT_GET_I32_SYMBOL, NOD_STRUCT_GET_I64_SYMBOL, NOD_STRUCT_GET_POINTER_SYMBOL,
    NOD_STRUCT_GET_U16_SYMBOL, NOD_STRUCT_GET_U32_SYMBOL, NOD_STRUCT_GET_U64_SYMBOL,
    NOD_STRUCT_SET_I32_SYMBOL, NOD_STRUCT_SET_I64_SYMBOL, NOD_STRUCT_SET_POINTER_SYMBOL,
    NOD_STRUCT_SET_U16_SYMBOL, NOD_STRUCT_SET_U32_SYMBOL, NOD_STRUCT_SET_U64_SYMBOL,
    // Sprint 35 — COM shim symbols.
    NOD_COM_RELEASE_SYMBOL, NOD_COM_REGISTRY_LEN_SYMBOL, NOD_COM_LAST_HRESULT_SYMBOL,
    NOD_COM_CLEAR_LAST_HRESULT_SYMBOL,
    NOD_DXGI_CREATE_FACTORY_SYMBOL, NOD_DXGI_DEVICE_FROM_D3D_DEVICE_SYMBOL,
    NOD_DXGI_CREATE_SURFACE_FROM_TEXTURE_SYMBOL,
    NOD_D3D11_CREATE_DEVICE_SYMBOL, NOD_D3D11_GET_IMMEDIATE_CONTEXT_SYMBOL,
    NOD_D3D11_CREATE_TEXTURE_2D_SYMBOL, NOD_D3D11_COPY_TO_STAGING_AND_MAP_SYMBOL,
    NOD_D3D11_LAST_STAGING_HANDLE_SYMBOL, NOD_D3D11_LAST_MAPPED_ROW_PITCH_SYMBOL,
    NOD_D3D11_UNMAP_SYMBOL,
    NOD_D2D_CREATE_FACTORY_SYMBOL, NOD_D2D_CREATE_DEVICE_SYMBOL,
    NOD_D2D_CREATE_DEVICE_CONTEXT_SYMBOL, NOD_D2D_CREATE_BITMAP_FOR_TARGET_SYMBOL,
    NOD_D2D_SET_TARGET_SYMBOL, NOD_D2D_BEGIN_DRAW_SYMBOL, NOD_D2D_END_DRAW_SYMBOL,
    NOD_D2D_CLEAR_SYMBOL, NOD_D2D_SET_TRANSFORM_IDENTITY_SYMBOL,
    NOD_D2D_CREATE_SOLID_COLOR_BRUSH_SYMBOL, NOD_D2D_DRAW_TEXT_LAYOUT_SYMBOL,
    NOD_D2D_DRAW_RECTANGLE_SYMBOL, NOD_D2D_FILL_RECTANGLE_SYMBOL,
    NOD_DWRITE_CREATE_FACTORY_SYMBOL, NOD_DWRITE_CREATE_TEXT_FORMAT_SYMBOL,
    NOD_DWRITE_CREATE_TEXT_LAYOUT_SYMBOL, NOD_DWRITE_GET_LAYOUT_METRICS_SYMBOL,
    NOD_COUNT_NON_ZERO_RED_SYMBOL,
    // Sprint 36 — IDE-shell symbols.
    NOD_DXGI_FACTORY_FROM_D3D_DEVICE_SYMBOL,
    NOD_DXGI_CREATE_SWAP_CHAIN_FOR_HWND_SYMBOL,
    NOD_D2D_CREATE_BITMAP_FROM_SWAP_CHAIN_SYMBOL,
    NOD_DXGI_SWAP_CHAIN_PRESENT_SYMBOL,
    NOD_DXGI_SWAP_CHAIN_RESIZE_BUFFERS_SYMBOL,
    NOD_REGISTER_WINDOW_CLASS_SYMBOL,
    NOD_CREATE_MESSAGE_ONLY_WINDOW_SYMBOL,
    NOD_CREATE_HIDDEN_WINDOW_SYMBOL,
    NOD_DESTROY_WINDOW_SYMBOL,
    NOD_POST_MESSAGE_SYMBOL,
    NOD_PUMP_ONE_MESSAGE_SYMBOL,
    NOD_DEF_WINDOW_PROC_SYMBOL,
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
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut sprint_20b_extern_decls: Vec<(Option<inkwell::values::FunctionValue<'_>>, *mut std::ffi::c_void)> = vec![
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
                module.get_function(NOD_FUNCALL0_SYMBOL),
                nod_runtime::nod_funcall0 as *const () as *mut std::ffi::c_void,
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
                module.get_function(NOD_FUNCALL3_SYMBOL),
                nod_runtime::nod_funcall3 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_FUNCALL4_SYMBOL),
                nod_runtime::nod_funcall4 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_FUNCALL5_SYMBOL),
                nod_runtime::nod_funcall5 as *const () as *mut std::ffi::c_void,
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
            // Sprint 28 — Win64 FFI trampolines.
            (
                module.get_function(NOD_WINFFI_CALL_0_SYMBOL),
                nod_runtime::nod_winffi_call_0 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_1_SYMBOL),
                nod_runtime::nod_winffi_call_1 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_2_SYMBOL),
                nod_runtime::nod_winffi_call_2 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_3_SYMBOL),
                nod_runtime::nod_winffi_call_3 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_4_SYMBOL),
                nod_runtime::nod_winffi_call_4 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_5_SYMBOL),
                nod_runtime::nod_winffi_call_5 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_6_SYMBOL),
                nod_runtime::nod_winffi_call_6 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_7_SYMBOL),
                nod_runtime::nod_winffi_call_7 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_8_SYMBOL),
                nod_runtime::nod_winffi_call_8 as *const () as *mut std::ffi::c_void,
            ),
            // Sprint 36b — trampoline family extended to arity 12
            // (CreateWindowExW + the rest of the IDE-shell Win32 surface).
            (
                module.get_function(NOD_WINFFI_CALL_9_SYMBOL),
                nod_runtime::nod_winffi_call_9 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_10_SYMBOL),
                nod_runtime::nod_winffi_call_10 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_11_SYMBOL),
                nod_runtime::nod_winffi_call_11 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_WINFFI_CALL_12_SYMBOL),
                nod_runtime::nod_winffi_call_12 as *const () as *mut std::ffi::c_void,
            ),
            // Sprint 32 — closure-to-C-callback trampoline registration.
            (
                module.get_function(NOD_REGISTER_WNDPROC_SYMBOL),
                nod_runtime::nod_register_wndproc as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_REGISTER_WNDENUMPROC_SYMBOL),
                nod_runtime::nod_register_wndenumproc as *const () as *mut std::ffi::c_void,
            ),
            // Sprint 34 — <c-struct> field accessor primitives.
            (
                module.get_function(NOD_STRUCT_GET_I32_SYMBOL),
                nod_runtime::nod_struct_get_i32 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_SET_I32_SYMBOL),
                nod_runtime::nod_struct_set_i32 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_GET_I64_SYMBOL),
                nod_runtime::nod_struct_get_i64 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_SET_I64_SYMBOL),
                nod_runtime::nod_struct_set_i64 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_GET_U16_SYMBOL),
                nod_runtime::nod_struct_get_u16 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_SET_U16_SYMBOL),
                nod_runtime::nod_struct_set_u16 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_GET_U32_SYMBOL),
                nod_runtime::nod_struct_get_u32 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_SET_U32_SYMBOL),
                nod_runtime::nod_struct_set_u32 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_GET_U64_SYMBOL),
                nod_runtime::nod_struct_get_u64 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_SET_U64_SYMBOL),
                nod_runtime::nod_struct_set_u64 as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_GET_POINTER_SYMBOL),
                nod_runtime::nod_struct_get_pointer as *const () as *mut std::ffi::c_void,
            ),
            (
                module.get_function(NOD_STRUCT_SET_POINTER_SYMBOL),
                nod_runtime::nod_struct_set_pointer as *const () as *mut std::ffi::c_void,
            ),
        ];
        // Sprint 35 — COM shim function-pointer mappings. Only built on
        // Windows; the shim symbols are `#[cfg(windows)]` in nod-runtime.
        // On non-Windows builds these mappings simply aren't added —
        // the test layer guards every COM-touching test with
        // `#![cfg(windows)]` so the symbols are never referenced.
        #[cfg(windows)]
        let com_mappings: Vec<(Option<inkwell::values::FunctionValue<'_>>, *mut std::ffi::c_void)> = vec![
            (module.get_function(NOD_COM_RELEASE_SYMBOL),
             nod_runtime::nod_com_release as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_COM_REGISTRY_LEN_SYMBOL),
             nod_runtime::nod_com_registry_len as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_COM_LAST_HRESULT_SYMBOL),
             nod_runtime::nod_com_last_hresult as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_COM_CLEAR_LAST_HRESULT_SYMBOL),
             nod_runtime::nod_com_clear_last_hresult as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DXGI_CREATE_FACTORY_SYMBOL),
             nod_runtime::nod_dxgi_create_factory as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DXGI_DEVICE_FROM_D3D_DEVICE_SYMBOL),
             nod_runtime::nod_dxgi_device_from_d3d_device as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DXGI_CREATE_SURFACE_FROM_TEXTURE_SYMBOL),
             nod_runtime::nod_dxgi_create_surface_from_texture as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D3D11_CREATE_DEVICE_SYMBOL),
             nod_runtime::nod_d3d11_create_device as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D3D11_GET_IMMEDIATE_CONTEXT_SYMBOL),
             nod_runtime::nod_d3d11_get_immediate_context as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D3D11_CREATE_TEXTURE_2D_SYMBOL),
             nod_runtime::nod_d3d11_create_texture_2d as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D3D11_COPY_TO_STAGING_AND_MAP_SYMBOL),
             nod_runtime::nod_d3d11_copy_to_staging_and_map as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D3D11_LAST_STAGING_HANDLE_SYMBOL),
             nod_runtime::nod_d3d11_last_staging_handle as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D3D11_LAST_MAPPED_ROW_PITCH_SYMBOL),
             nod_runtime::nod_d3d11_last_mapped_row_pitch as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D3D11_UNMAP_SYMBOL),
             nod_runtime::nod_d3d11_unmap as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_CREATE_FACTORY_SYMBOL),
             nod_runtime::nod_d2d_create_factory as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_CREATE_DEVICE_SYMBOL),
             nod_runtime::nod_d2d_create_device as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_CREATE_DEVICE_CONTEXT_SYMBOL),
             nod_runtime::nod_d2d_create_device_context as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_CREATE_BITMAP_FOR_TARGET_SYMBOL),
             nod_runtime::nod_d2d_create_bitmap_for_target as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_SET_TARGET_SYMBOL),
             nod_runtime::nod_d2d_set_target as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_BEGIN_DRAW_SYMBOL),
             nod_runtime::nod_d2d_begin_draw as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_END_DRAW_SYMBOL),
             nod_runtime::nod_d2d_end_draw as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_CLEAR_SYMBOL),
             nod_runtime::nod_d2d_clear as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_SET_TRANSFORM_IDENTITY_SYMBOL),
             nod_runtime::nod_d2d_set_transform_identity as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_CREATE_SOLID_COLOR_BRUSH_SYMBOL),
             nod_runtime::nod_d2d_create_solid_color_brush as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_DRAW_TEXT_LAYOUT_SYMBOL),
             nod_runtime::nod_d2d_draw_text_layout as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_DRAW_RECTANGLE_SYMBOL),
             nod_runtime::nod_d2d_draw_rectangle as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_FILL_RECTANGLE_SYMBOL),
             nod_runtime::nod_d2d_fill_rectangle as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DWRITE_CREATE_FACTORY_SYMBOL),
             nod_runtime::nod_dwrite_create_factory as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DWRITE_CREATE_TEXT_FORMAT_SYMBOL),
             nod_runtime::nod_dwrite_create_text_format as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DWRITE_CREATE_TEXT_LAYOUT_SYMBOL),
             nod_runtime::nod_dwrite_create_text_layout as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DWRITE_GET_LAYOUT_METRICS_SYMBOL),
             nod_runtime::nod_dwrite_get_layout_metrics as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_COUNT_NON_ZERO_RED_SYMBOL),
             nod_runtime::nod_count_non_zero_red as *const () as *mut std::ffi::c_void),
            // Sprint 36 — HWND-bound swap chain + IDE-shell window
            // primitives. Each entry maps the LLVM symbol declared by
            // codegen.rs to the nod_runtime extern address.
            (module.get_function(NOD_DXGI_FACTORY_FROM_D3D_DEVICE_SYMBOL),
             nod_runtime::nod_dxgi_factory_from_d3d_device as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DXGI_CREATE_SWAP_CHAIN_FOR_HWND_SYMBOL),
             nod_runtime::nod_dxgi_create_swap_chain_for_hwnd as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_D2D_CREATE_BITMAP_FROM_SWAP_CHAIN_SYMBOL),
             nod_runtime::nod_d2d_create_bitmap_from_swap_chain as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DXGI_SWAP_CHAIN_PRESENT_SYMBOL),
             nod_runtime::nod_dxgi_swap_chain_present as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DXGI_SWAP_CHAIN_RESIZE_BUFFERS_SYMBOL),
             nod_runtime::nod_dxgi_swap_chain_resize_buffers as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_REGISTER_WINDOW_CLASS_SYMBOL),
             nod_runtime::nod_register_window_class as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_CREATE_MESSAGE_ONLY_WINDOW_SYMBOL),
             nod_runtime::nod_create_message_only_window as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_CREATE_HIDDEN_WINDOW_SYMBOL),
             nod_runtime::nod_create_hidden_window as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DESTROY_WINDOW_SYMBOL),
             nod_runtime::nod_destroy_window as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_POST_MESSAGE_SYMBOL),
             nod_runtime::nod_post_message as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_PUMP_ONE_MESSAGE_SYMBOL),
             nod_runtime::nod_pump_one_message as *const () as *mut std::ffi::c_void),
            (module.get_function(NOD_DEF_WINDOW_PROC_SYMBOL),
             nod_runtime::nod_def_window_proc as *const () as *mut std::ffi::c_void),
        ];
        #[cfg(windows)]
        sprint_20b_extern_decls.extend(com_mappings);

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

    /// Sprint 38 — install a module from previously-saved LLVM bitcode
    /// plus its [`ModuleManifest`]. This is the cross-process-replay
    /// counterpart to [`Self::add_module`]: instead of receiving a
    /// freshly-codegen'd module, it parses bitcode and registers each
    /// manifest-declared external symbol against the **current
    /// process's** runtime addresses before MCJIT finalises.
    ///
    /// On success the caller can resolve the entry function via
    /// [`Self::get_function_ptr`] just as with a cold-compiled module.
    ///
    /// `bitcode` is the byte payload of a `.bc` file produced by
    /// [`inkwell::module::Module::write_bitcode_to_memory`] during a
    /// previous cold compile. `manifest` is the parallel
    /// `<key>.manifest.json` sidecar describing the per-bake-site
    /// relocation kinds.
    ///
    /// # Returns
    /// `Ok(())` on success. On any error (verify failure, MCJIT engine
    /// creation failure, relocation kind requiring an FFI symbol that
    /// can't be resolved in this process) returns the appropriate
    /// [`JitError`] and the engine isn't installed.
    pub fn add_module_from_bitcode(
        &mut self,
        ctx: &'ctx Context,
        bitcode: &[u8],
        module_name: &str,
        manifest: &ModuleManifest,
    ) -> Result<(), JitError> {
        // Parse bitcode into a fresh inkwell `Module` owned by `ctx`.
        let buffer = inkwell::memory_buffer::MemoryBuffer::create_from_memory_range_copy(
            bitcode,
            module_name,
        );
        let module = inkwell::module::Module::parse_bitcode_from_buffer(&buffer, ctx)
            .map_err(|e| JitError::Verify(e.to_string()))?;
        module
            .verify()
            .map_err(|e| JitError::Verify(format!("post-load verify: {e}")))?;

        // Compute the current-process address for each manifest entry
        // BEFORE we hand the module to MCJIT. After
        // `LLVMCreateMCJITCompilerForModule` we lose the inkwell
        // module accessor.
        //
        // For each entry, look up the named external global in the
        // module. If present, capture the FunctionValue/GlobalValue
        // along with its target address.
        let mut reloc_bindings: Vec<(inkwell::values::GlobalValue<'ctx>, *mut std::ffi::c_void)> =
            Vec::with_capacity(manifest.entries.len());
        for entry in &manifest.entries {
            let Some(global) = module.get_global(&entry.symbol) else {
                // Symbol not present in the bitcode — IR may have been
                // optimised in a way that eliminated the use. Skip
                // silently; the load won't crash because the global
                // isn't referenced.
                continue;
            };
            let addr = match resolve_reloc_kind(&entry.kind) {
                Ok(a) => a,
                Err(e) => return Err(JitError::Create(format!("reloc {}: {e}", entry.symbol))),
            };
            reloc_bindings.push((global, addr));
        }

        // Capture all the standard extern shims the cold path resolves
        // (FORMAT_OUT, nod_make, dispatch shims, etc.) so the loaded
        // module's external decls bind correctly. We reuse the same
        // resolver as `add_module` by collecting `(name, addr)` pairs
        // and re-walking after MCJIT engine creation.
        let standard_externs = standard_extern_addresses();
        let mut standard_bindings: Vec<(inkwell::values::FunctionValue<'ctx>, *mut std::ffi::c_void)> =
            Vec::new();
        for (name, addr) in &standard_externs {
            if let Some(f) = module.get_function(name) {
                standard_bindings.push((f, *addr));
            }
        }
        // Cross-module method body externs (mirrors `add_module`'s logic).
        let mut cross_module_externs: Vec<(inkwell::values::FunctionValue<'ctx>, *mut std::ffi::c_void)> =
            Vec::new();
        {
            let mut maybe = module.get_first_function();
            while let Some(f) = maybe {
                if f.count_basic_blocks() == 0 {
                    let name = f.get_name().to_string_lossy().into_owned();
                    if !name.is_empty()
                        && nod_runtime::find_method_body_ptr(&name).is_some()
                    {
                        let addr = nod_runtime::find_method_body_ptr(&name).unwrap() as *mut std::ffi::c_void;
                        cross_module_externs.push((f, addr));
                    }
                }
                maybe = f.get_next_function();
            }
        }

        // Install the module in a fresh MCJIT engine.
        let mut opts: LLVMMCJITCompilerOptions = unsafe { std::mem::zeroed() };
        unsafe {
            LLVMInitializeMCJITCompilerOptions(&mut opts, size_of::<LLVMMCJITCompilerOptions>());
        }
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
        std::mem::forget(module);

        // Register relocation globals against their current-process
        // addresses. The named globals declared in IR have their
        // physical addresses fixed at this point — `LLVMAddGlobalMapping`
        // makes `&@symbol == addr` at runtime.
        for (global, addr) in &reloc_bindings {
            // SAFETY: `engine` is the live MCJIT engine; `global` is
            // a GlobalValue captured before ownership transfer; `addr`
            // is a process-local address valid for the engine's
            // lifetime.
            unsafe { LLVMAddGlobalMapping(engine, global.as_value_ref(), *addr) };
        }
        for (f, addr) in &standard_bindings {
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), *addr) };
        }
        for (f, addr) in &cross_module_externs {
            unsafe { LLVMAddGlobalMapping(engine, f.as_value_ref(), *addr) };
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

/// Sprint 38 — compute the current-process address for one
/// [`RelocKind`]. The cold-compile path baked the resolved address
/// into IR as an `i64` constant; the cache-hit path calls this to
/// recompute it against the new process's runtime state.
fn resolve_reloc_kind(kind: &RelocKind) -> Result<*mut std::ffi::c_void, String> {
    let addr: u64 = match kind {
        RelocKind::ImmTrue => nod_runtime::literal_pool_immediates().true_.raw(),
        RelocKind::ImmFalse => nod_runtime::literal_pool_immediates().false_.raw(),
        RelocKind::ImmNil => nod_runtime::literal_pool_immediates().nil.raw(),
        RelocKind::ImmFalseWrapper => {
            nod_runtime::literal_pool_immediates().false_.raw() & !1_u64
        }
        RelocKind::ClassMetadata { class_id } => {
            let id = nod_runtime::ClassId(*class_id);
            nod_runtime::class_metadata_ptr(id) as u64
        }
        RelocKind::StringLiteral { text } => nod_runtime::intern_string_literal(text).raw(),
        RelocKind::SymbolLiteral { name } => nod_runtime::intern_symbol_literal(name).raw(),
        RelocKind::CacheSlot { site_id } => {
            nod_runtime::allocate_cache_slot(*site_id) as u64
        }
        RelocKind::Generic { name } => {
            let g = nod_runtime::get_or_create_generic(name);
            g as *const _ as u64
        }
        RelocKind::StubEntry { dll, symbol, signature_bytes } => {
            // Reconstruct the ApiCallSignature from the manifest bytes
            // (it's `#[repr(C)] Copy` — bytewise round-trips).
            if signature_bytes.len() != size_of::<nod_runtime::ApiCallSignature>() {
                return Err(format!(
                    "StubEntry signature byte length mismatch: got {} expected {}",
                    signature_bytes.len(),
                    size_of::<nod_runtime::ApiCallSignature>()
                ));
            }
            let mut sig = nod_runtime::ApiCallSignature {
                arg_count: 0,
                arg_kinds: [0; 12],
                return_kind: 0,
            };
            // SAFETY: `ApiCallSignature` is `#[repr(C)] Copy`. We
            // verified the length above. Source bytes come from the
            // manifest JSON's hex-encoded signature field.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    signature_bytes.as_ptr(),
                    &mut sig as *mut _ as *mut u8,
                    signature_bytes.len(),
                );
            }
            let specs = vec![nod_runtime::StubEntrySpec {
                dll: dll.clone(),
                symbol: symbol.clone(),
                signature: sig,
            }];
            let (_table, ptrs) = nod_runtime::allocate_stub_table(&specs);
            let entry_ptr = ptrs[0];
            // SAFETY: `entry_ptr` was just allocated by
            // `allocate_stub_table`; `dll`/`symbol` are valid UTF-8
            // strings the caller produced. `resolve_into_entry` is
            // idempotent — calling it on an already-resolved entry is
            // a documented no-op.
            unsafe {
                if let Err(_w) = nod_runtime::resolve_into_entry(entry_ptr, dll, symbol) {
                    // Resolution failure surfaces as a `<c-ffi-error>`
                    // Word in the cold path; we surface it as a
                    // JitError so the cache loader can fall back to
                    // recompile.
                    return Err(format!(
                        "StubEntry resolve failed for `{symbol}@{dll}` in current process"
                    ));
                }
            }
            entry_ptr as u64
        }
    };
    Ok(addr as *mut std::ffi::c_void)
}

/// Sprint 38 — list of `(symbol_name, runtime_address)` pairs for the
/// standard runtime shims every JIT-compiled module potentially calls.
/// Same set as the bindings installed inline by [`Jit::add_module`].
fn standard_extern_addresses() -> Vec<(&'static str, *mut std::ffi::c_void)> {
    use crate::codegen::*;
    let mut v: Vec<(&'static str, *mut std::ffi::c_void)> = vec![
        (FORMAT_OUT_SYMBOL, nod_runtime::nod_format_out as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_SYMBOL, nod_runtime::nod_make as *const () as *mut std::ffi::c_void),
        (NOD_IS_INSTANCE_OF_SYMBOL, nod_runtime::nod_is_instance_of as *const () as *mut std::ffi::c_void),
        (NOD_DISPATCH_UNARY_SYMBOL, nod_runtime::nod_dispatch_unary as *const () as *mut std::ffi::c_void),
        (NOD_DISPATCH_BINARY_SYMBOL, nod_runtime::nod_dispatch_binary as *const () as *mut std::ffi::c_void),
        (NOD_DISPATCH_SYMBOL, nod_runtime::nod_dispatch as *const () as *mut std::ffi::c_void),
        (NOD_CARD_MARK_SYMBOL, nod_runtime::nod_card_mark as *const () as *mut std::ffi::c_void),
        (NOD_REGISTER_ROOT_SYMBOL, nod_runtime::nod_register_root as *const () as *mut std::ffi::c_void),
        (NOD_UNREGISTER_ROOT_SYMBOL, nod_runtime::nod_unregister_root as *const () as *mut std::ffi::c_void),
        (NOD_NEXT_METHOD_SYMBOL, nod_runtime::nod_next_method as *const () as *mut std::ffi::c_void),
        (NOD_HAS_NEXT_METHOD_SYMBOL, nod_runtime::nod_has_next_method as *const () as *mut std::ffi::c_void),
        (NOD_PUSH_SEALED_CHAIN_SYMBOL, nod_runtime::nod_push_sealed_chain_frame as *const () as *mut std::ffi::c_void),
        (NOD_POP_SEALED_CHAIN_SYMBOL, nod_runtime::nod_pop_sealed_chain_frame as *const () as *mut std::ffi::c_void),
        (NOD_PAIR_ALLOC_SYMBOL, nod_runtime::nod_pair_alloc as *const () as *mut std::ffi::c_void),
        (NOD_PAIR_HEAD_SYMBOL, nod_runtime::nod_pair_head as *const () as *mut std::ffi::c_void),
        (NOD_PAIR_TAIL_SYMBOL, nod_runtime::nod_pair_tail as *const () as *mut std::ffi::c_void),
        (NOD_EMPTY_P_SYMBOL, nod_runtime::nod_empty_p as *const () as *mut std::ffi::c_void),
        (NOD_NIL_SYMBOL, nod_runtime::nod_nil as *const () as *mut std::ffi::c_void),
        (NOD_SIGNAL_SYMBOL, nod_runtime::nod_signal as *const () as *mut std::ffi::c_void),
        (NOD_RUN_BLOCK_SYMBOL, nod_runtime::nod_run_block as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_EXIT_PROCEDURE_SYMBOL, nod_runtime::nod_make_exit_procedure as *const () as *mut std::ffi::c_void),
        (NOD_INVOKE_EXIT_SYMBOL, nod_runtime::nod_invoke_exit as *const () as *mut std::ffi::c_void),
        (NOD_CONDITION_MESSAGE_SYMBOL, nod_runtime::nod_condition_message as *const () as *mut std::ffi::c_void),
        (NOD_COLLECTION_SIZE_SYMBOL, nod_runtime::nod_collection_size as *const () as *mut std::ffi::c_void),
        (NOD_COLLECTION_CONCATENATE_SYMBOL, nod_runtime::nod_collection_concatenate as *const () as *mut std::ffi::c_void),
        (NOD_RANGE_FROM_SYMBOL, nod_runtime::nod_range_from as *const () as *mut std::ffi::c_void),
        (NOD_RANGE_TO_SYMBOL, nod_runtime::nod_range_to as *const () as *mut std::ffi::c_void),
        (NOD_RANGE_BY_SYMBOL, nod_runtime::nod_range_by as *const () as *mut std::ffi::c_void),
        (NOD_SOV_SIZE_SYMBOL, nod_runtime::nod_sov_size as *const () as *mut std::ffi::c_void),
        (NOD_SOV_ELEMENT_SYMBOL, nod_runtime::nod_sov_element as *const () as *mut std::ffi::c_void),
        (NOD_SOV_ELEMENT_SETTER_SYMBOL, nod_runtime::nod_sov_element_setter as *const () as *mut std::ffi::c_void),
        (NOD_STRETCHY_VECTOR_SIZE_SYMBOL, nod_runtime::nod_stretchy_vector_size as *const () as *mut std::ffi::c_void),
        (NOD_STRETCHY_VECTOR_ELEMENT_SYMBOL, nod_runtime::nod_stretchy_vector_element as *const () as *mut std::ffi::c_void),
        (NOD_STRETCHY_VECTOR_ELEMENT_SETTER_SYMBOL, nod_runtime::nod_stretchy_vector_element_setter as *const () as *mut std::ffi::c_void),
        (NOD_STRETCHY_VECTOR_PUSH_SYMBOL, nod_runtime::nod_stretchy_vector_push as *const () as *mut std::ffi::c_void),
        (NOD_FIP_INIT_SYMBOL, nod_runtime::nod_fip_init as *const () as *mut std::ffi::c_void),
        (NOD_FIP_FINISHED_P_SYMBOL, nod_runtime::nod_fip_finished_p as *const () as *mut std::ffi::c_void),
        (NOD_FIP_CURRENT_ELEMENT_SYMBOL, nod_runtime::nod_fip_current_element as *const () as *mut std::ffi::c_void),
        (NOD_FIP_ADVANCE_SYMBOL, nod_runtime::nod_fip_advance as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_RANGE_SYMBOL, nod_runtime::nod_make_range as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_STRETCHY_VECTOR_SYMBOL, nod_runtime::nod_make_stretchy_vector as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_FUNCTION_REF_SYMBOL, nod_runtime::nod_make_function_ref as *const () as *mut std::ffi::c_void),
        (NOD_FUNCALL0_SYMBOL, nod_runtime::nod_funcall0 as *const () as *mut std::ffi::c_void),
        (NOD_FUNCALL1_SYMBOL, nod_runtime::nod_funcall1 as *const () as *mut std::ffi::c_void),
        (NOD_FUNCALL2_SYMBOL, nod_runtime::nod_funcall2 as *const () as *mut std::ffi::c_void),
        (NOD_FUNCALL3_SYMBOL, nod_runtime::nod_funcall3 as *const () as *mut std::ffi::c_void),
        (NOD_FUNCALL4_SYMBOL, nod_runtime::nod_funcall4 as *const () as *mut std::ffi::c_void),
        (NOD_FUNCALL5_SYMBOL, nod_runtime::nod_funcall5 as *const () as *mut std::ffi::c_void),
        (NOD_APPLY_SYMBOL, nod_runtime::nod_apply as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_SOV_LEN_SYMBOL, nod_runtime::nod_make_sov_len as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_TABLE_SYMBOL, nod_runtime::nod_make_table as *const () as *mut std::ffi::c_void),
        (NOD_TABLE_SIZE_SYMBOL, nod_runtime::nod_table_size as *const () as *mut std::ffi::c_void),
        (NOD_TABLE_ELEMENT_SYMBOL, nod_runtime::nod_table_element as *const () as *mut std::ffi::c_void),
        (NOD_TABLE_ELEMENT_OR_DEFAULT_SYMBOL, nod_runtime::nod_table_element_or_default as *const () as *mut std::ffi::c_void),
        (NOD_TABLE_ELEMENT_SETTER_SYMBOL, nod_runtime::nod_table_element_setter as *const () as *mut std::ffi::c_void),
        (NOD_TABLE_REMOVE_KEY_SYMBOL, nod_runtime::nod_table_remove_key as *const () as *mut std::ffi::c_void),
        (NOD_TABLE_KEYS_SYMBOL, nod_runtime::nod_table_keys as *const () as *mut std::ffi::c_void),
        (NOD_TABLE_VALUES_SYMBOL, nod_runtime::nod_table_values as *const () as *mut std::ffi::c_void),
        (NOD_OBJECT_HASH_SYMBOL, nod_runtime::nod_object_hash as *const () as *mut std::ffi::c_void),
        (NOD_OBJECT_EQUAL_P_SYMBOL, nod_runtime::nod_object_equal_p as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_CELL_SYMBOL, nod_runtime::nod_make_cell as *const () as *mut std::ffi::c_void),
        (NOD_CELL_GET_SYMBOL, nod_runtime::nod_cell_get as *const () as *mut std::ffi::c_void),
        (NOD_CELL_SET_SYMBOL, nod_runtime::nod_cell_set as *const () as *mut std::ffi::c_void),
        (NOD_ENV_CELL_SYMBOL, nod_runtime::nod_env_cell as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_ENVIRONMENT_SYMBOL, nod_runtime::nod_make_environment as *const () as *mut std::ffi::c_void),
        (NOD_MAKE_CLOSURE_SYMBOL, nod_runtime::nod_make_closure as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_0_SYMBOL, nod_runtime::nod_winffi_call_0 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_1_SYMBOL, nod_runtime::nod_winffi_call_1 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_2_SYMBOL, nod_runtime::nod_winffi_call_2 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_3_SYMBOL, nod_runtime::nod_winffi_call_3 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_4_SYMBOL, nod_runtime::nod_winffi_call_4 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_5_SYMBOL, nod_runtime::nod_winffi_call_5 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_6_SYMBOL, nod_runtime::nod_winffi_call_6 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_7_SYMBOL, nod_runtime::nod_winffi_call_7 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_8_SYMBOL, nod_runtime::nod_winffi_call_8 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_9_SYMBOL, nod_runtime::nod_winffi_call_9 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_10_SYMBOL, nod_runtime::nod_winffi_call_10 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_11_SYMBOL, nod_runtime::nod_winffi_call_11 as *const () as *mut std::ffi::c_void),
        (NOD_WINFFI_CALL_12_SYMBOL, nod_runtime::nod_winffi_call_12 as *const () as *mut std::ffi::c_void),
    ];
    // Struct field accessors (Sprint 34) — same shape, kept in a
    // second pass for clarity.
    use crate::codegen::{
        NOD_STRUCT_GET_I32_SYMBOL, NOD_STRUCT_GET_I64_SYMBOL, NOD_STRUCT_GET_POINTER_SYMBOL,
        NOD_STRUCT_GET_U16_SYMBOL, NOD_STRUCT_GET_U32_SYMBOL, NOD_STRUCT_GET_U64_SYMBOL,
        NOD_STRUCT_SET_I32_SYMBOL, NOD_STRUCT_SET_I64_SYMBOL, NOD_STRUCT_SET_POINTER_SYMBOL,
        NOD_STRUCT_SET_U16_SYMBOL, NOD_STRUCT_SET_U32_SYMBOL, NOD_STRUCT_SET_U64_SYMBOL,
    };
    v.extend([
        (NOD_STRUCT_GET_I32_SYMBOL, nod_runtime::nod_struct_get_i32 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_SET_I32_SYMBOL, nod_runtime::nod_struct_set_i32 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_GET_I64_SYMBOL, nod_runtime::nod_struct_get_i64 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_SET_I64_SYMBOL, nod_runtime::nod_struct_set_i64 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_GET_U16_SYMBOL, nod_runtime::nod_struct_get_u16 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_SET_U16_SYMBOL, nod_runtime::nod_struct_set_u16 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_GET_U32_SYMBOL, nod_runtime::nod_struct_get_u32 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_SET_U32_SYMBOL, nod_runtime::nod_struct_set_u32 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_GET_U64_SYMBOL, nod_runtime::nod_struct_get_u64 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_SET_U64_SYMBOL, nod_runtime::nod_struct_set_u64 as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_GET_POINTER_SYMBOL, nod_runtime::nod_struct_get_pointer as *const () as *mut std::ffi::c_void),
        (NOD_STRUCT_SET_POINTER_SYMBOL, nod_runtime::nod_struct_set_pointer as *const () as *mut std::ffi::c_void),
        (NOD_REGISTER_WNDPROC_SYMBOL, nod_runtime::nod_register_wndproc as *const () as *mut std::ffi::c_void),
        (NOD_REGISTER_WNDENUMPROC_SYMBOL, nod_runtime::nod_register_wndenumproc as *const () as *mut std::ffi::c_void),
    ]);
    v
}
