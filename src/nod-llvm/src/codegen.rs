//! DFM -> LLVM IR lowering.
//!
//! **Sprint 09 ABI.** Every Dylan value of `<integer>` or `<boolean>`
//! type lowers to an `i64` holding a tagged `nod_runtime::Word`:
//!
//! ```text
//!   bit 0 = 0 → fixnum;  upper 63 bits = signed value shifted left by 1.
//!   bit 0 = 1 → pointer; bits [63:1] = 8-byte-aligned heap pointer.
//! ```
//!
//! For Sprint 09, `#t` and `#f` are *immediate* booleans encoded as
//! tagged fixnums 1 and 0 respectively (so `#f` = `Word(0)`). Sprint
//! 10+ may introduce a richer immediate scheme; today's encoding is
//! the minimum that gives `instance?(x, <boolean>)` something to test.
//!
//! **Tagged arithmetic.** `(a<<1) + (b<<1) = (a+b)<<1`, so integer
//! `add` / `sub` / `neg` need no untag/retag. `mul` is asymmetric:
//! `(a<<1) * (b<<1) = (a*b) << 2`, so we right-shift one operand
//! before the multiply to recover `(a*b)<<1`. `div` / `mod` / `rem`
//! untag both operands and retag the result — the cleanest lowering
//! given that signed-division identities don't survive the shift.
//!
//! **Comparisons** run directly on the tagged words (ordering is
//! preserved because both operands shift left by the same amount).
//! The `i1` from `icmp` is `zext`'d to i64 and shifted left by 1 to
//! match the boolean encoding.
//!
//! **Floats** are not tagged — Sprint 09 functions returning
//! `<double-float>` return raw `f64`. Sprint 10 boxes floats on the
//! heap; until then the calling convention for `<double-float>` is
//! the same as Sprint 07.
//!
//! **Sprint 10 changes.**
//!   - `#t` / `#f` are no longer fixnum-shaped; they're pinned heap
//!     wrappers whose addresses come from `nod_runtime::Immediates`.
//!     Codegen bakes those addresses into LLVM constants.
//!   - `<byte-string>` literals are interned in the process-global
//!     literal pool (`nod_runtime::intern_string_literal`); codegen
//!     bakes the resulting tagged Word as an `i64` constant.
//!   - `instance?` against the wrapper-tagged seed classes
//!     (`<byte-string>`, `<symbol>`, `<simple-object-vector>`,
//!     `<empty-list>`, `<boolean>`) reads the wrapper's class id and
//!     compares.
//!   - `format-out` is recognised as a builtin: codegen declares an
//!     `extern "C"` shim and binds `nod_format_out` via
//!     `LLVMAddGlobalMapping` at JIT-engine creation time.

use std::collections::HashMap;

use inkwell::FloatPredicate;
use inkwell::IntPredicate;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, PhiValue,
};
use nod_dfm::{
    BlockId, ClassCheck, Computation, ConstValue, Function as DfmFunction, PrimOp, SlotTypeKind,
    TempId, Terminator, TypeEstimate,
};
use nod_runtime::ClassId;

/// Name of the JIT-side `nod_make` external declaration.
pub const NOD_MAKE_SYMBOL: &str = "nod_make";
/// Name of the JIT-side `nod_is_instance_of` external declaration.
pub const NOD_IS_INSTANCE_OF_SYMBOL: &str = "nod_is_instance_of";
/// Name of the JIT-side `nod_dispatch_unary` external declaration.
pub const NOD_DISPATCH_UNARY_SYMBOL: &str = "nod_dispatch_unary";
/// Name of the JIT-side `nod_dispatch_binary` external declaration.
pub const NOD_DISPATCH_BINARY_SYMBOL: &str = "nod_dispatch_binary";
/// Sprint 13: variadic dispatch entry. Takes the generic pointer, a
/// cache-slot pointer, an arity, and up to 8 arguments.
pub const NOD_DISPATCH_SYMBOL: &str = "nod_dispatch";
/// Name of the JIT-side card-mark shim (`nod_card_mark`).
pub const NOD_CARD_MARK_SYMBOL: &str = "nod_card_mark";

/// Sprint 11b: Name of the JIT-side `nod_register_root` external. The
/// codegen layer brackets every potentially-allocating call with a
/// `register_root(slot)` ... call ... `unregister_root(slot)` pair for
/// each pointer-shaped live-across temp. The runtime walks registered
/// slots during GC and rewrites them if the target object moves.
pub const NOD_REGISTER_ROOT_SYMBOL: &str = "nod_register_root";
/// Sprint 11b: companion to `NOD_REGISTER_ROOT_SYMBOL`.
pub const NOD_UNREGISTER_ROOT_SYMBOL: &str = "nod_unregister_root";

/// Sprint 14: invoke the next-most-specific applicable method on the
/// current dispatch chain, forwarding the current method's args
/// verbatim. Lowered from Dylan-side `next-method()` calls.
pub const NOD_NEXT_METHOD_SYMBOL: &str = "nod_next_method";
/// Sprint 14: predicate `next-method?()` — `#t` iff there's a next
/// method in the chain.
pub const NOD_HAS_NEXT_METHOD_SYMBOL: &str = "nod_has_next_method";

/// Sprint 15: push a `next-method` chain frame on entry to a
/// sealed-direct multimethod call. Codegen emits a call to this just
/// before the resolved-method direct call so `next-method()` walks
/// the fallback chain (resolved at compile time).
pub const NOD_PUSH_SEALED_CHAIN_SYMBOL: &str = "nod_push_sealed_chain_frame";

/// Sprint 15: pop the chain frame after a sealed-direct multimethod
/// call returns. Paired with `NOD_PUSH_SEALED_CHAIN_SYMBOL`.
pub const NOD_POP_SEALED_CHAIN_SYMBOL: &str = "nod_pop_sealed_chain_frame";

/// Name of the JIT-side `format-out` external declaration. Resolved
/// to `nod_runtime::nod_format_out` via `LLVMAddGlobalMapping` at
/// engine-creation time (see `nod-llvm::jit`).
pub const FORMAT_OUT_SYMBOL: &str = "nod_format_out";

// ─── Sprint 16: `<pair>` / `<list>` builtins ───────────────────────────────
//
// Each Dylan-source builtin lowers to a synthetic `%pair-*` / `%empty?` /
// `%nil` callee in the DFM `DirectCall`. Codegen recognises the callee,
// declares the extern with the matching ABI, and emits the call. The
// JIT layer (`jit.rs`) resolves the extern's symbol to the runtime
// shim's address via `LLVMAddGlobalMapping`.

/// Sprint 16: `nod_pair_alloc(head, tail) -> <pair>`.
pub const NOD_PAIR_ALLOC_SYMBOL: &str = "nod_pair_alloc";
/// Sprint 16: `nod_pair_head(p) -> <object>`.
pub const NOD_PAIR_HEAD_SYMBOL: &str = "nod_pair_head";
/// Sprint 16: `nod_pair_tail(p) -> <object>`.
pub const NOD_PAIR_TAIL_SYMBOL: &str = "nod_pair_tail";
/// Sprint 16: `nod_empty_p(p) -> <boolean>`. Identity against `nil`.
pub const NOD_EMPTY_P_SYMBOL: &str = "nod_empty_p";
/// Sprint 16: `nod_nil() -> <empty-list>`.
pub const NOD_NIL_SYMBOL: &str = "nod_nil";

// ─── Sprint 19: conditions + block/exception/cleanup ───────────────────────
/// Sprint 19: `nod_signal(cond) -> u64`. Diverges via panic-based NLX.
pub const NOD_SIGNAL_SYMBOL: &str = "nod_signal";
/// Sprint 19: `nod_run_block(block_id, c0..c7) -> u64`. Drives the
/// block protocol.
pub const NOD_RUN_BLOCK_SYMBOL: &str = "nod_run_block";
/// Sprint 19: `nod_make_exit_procedure(block_id_word) -> u64`. Wraps
/// the Rust-side `make_exit_procedure` so codegen can mint exit
/// procedures from a `block (k)` site.
pub const NOD_MAKE_EXIT_PROCEDURE_SYMBOL: &str = "nod_make_exit_procedure";
/// Sprint 19: `nod_invoke_exit(ep, value) -> u64`. Diverges via NLX.
pub const NOD_INVOKE_EXIT_SYMBOL: &str = "nod_invoke_exit";
/// Sprint 19: `nod_condition_message(c) -> <byte-string>`.
pub const NOD_CONDITION_MESSAGE_SYMBOL: &str = "nod_condition_message";

// ─── Sprint 20b — collection / FIP / primitive-op shims ───────────────────
//
// Each shim mirrors a `%`-prefixed primitive callee emitted by
// `nod-sema/src/lower.rs::LOWER_PRIMITIVE_TABLE`. Codegen recognises the
// callee in `emit_direct_call`, declares the extern with `(i64, …) -> i64`,
// and `jit.rs` binds the symbol to the runtime shim address at engine
// creation. The shim sources live in `nod-runtime/src/collections.rs`.

pub const NOD_COLLECTION_SIZE_SYMBOL: &str = "nod_collection_size";
pub const NOD_COLLECTION_CONCATENATE_SYMBOL: &str = "nod_collection_concatenate";
pub const NOD_RANGE_FROM_SYMBOL: &str = "nod_range_from";
pub const NOD_RANGE_TO_SYMBOL: &str = "nod_range_to";
pub const NOD_RANGE_BY_SYMBOL: &str = "nod_range_by";
pub const NOD_SOV_SIZE_SYMBOL: &str = "nod_sov_size";
pub const NOD_SOV_ELEMENT_SYMBOL: &str = "nod_sov_element";
pub const NOD_SOV_ELEMENT_SETTER_SYMBOL: &str = "nod_sov_element_setter";
pub const NOD_STRETCHY_VECTOR_SIZE_SYMBOL: &str = "nod_stretchy_vector_size";
pub const NOD_STRETCHY_VECTOR_ELEMENT_SYMBOL: &str = "nod_stretchy_vector_element";
pub const NOD_STRETCHY_VECTOR_ELEMENT_SETTER_SYMBOL: &str = "nod_stretchy_vector_element_setter";
pub const NOD_STRETCHY_VECTOR_PUSH_SYMBOL: &str = "nod_stretchy_vector_push";
pub const NOD_FIP_INIT_SYMBOL: &str = "nod_fip_init";
pub const NOD_FIP_FINISHED_P_SYMBOL: &str = "nod_fip_finished_p";
pub const NOD_FIP_CURRENT_ELEMENT_SYMBOL: &str = "nod_fip_current_element";
pub const NOD_FIP_ADVANCE_SYMBOL: &str = "nod_fip_advance";
pub const NOD_MAKE_RANGE_SYMBOL: &str = "nod_make_range";
pub const NOD_MAKE_STRETCHY_VECTOR_SYMBOL: &str = "nod_make_stretchy_vector";

// ─── Sprint 21 — first-class function values ──────────────────────────────
//
// `nod_make_function_ref(name_bytestring, arity_fixnum) -> <function>`
// allocates (or returns the cached) `<function>` Word for the supplied
// Dylan-source name. The codegen emits a call to this shim whenever the
// lowerer sees an `Expr::Ident` resolving to a registered function used
// in expression (not call-head) position.
//
// `nod_funcall1`, `nod_funcall2`, `nod_apply` are the trampolines for
// invoking a `<function>` Word. See `nod-runtime::functions` for the
// implementation.
pub const NOD_MAKE_FUNCTION_REF_SYMBOL: &str = "nod_make_function_ref";
pub const NOD_FUNCALL0_SYMBOL: &str = "nod_funcall0";
pub const NOD_FUNCALL1_SYMBOL: &str = "nod_funcall1";
pub const NOD_FUNCALL2_SYMBOL: &str = "nod_funcall2";
pub const NOD_FUNCALL3_SYMBOL: &str = "nod_funcall3";
pub const NOD_FUNCALL4_SYMBOL: &str = "nod_funcall4";
pub const NOD_FUNCALL5_SYMBOL: &str = "nod_funcall5";
pub const NOD_APPLY_SYMBOL: &str = "nod_apply";
pub const NOD_MAKE_SOV_LEN_SYMBOL: &str = "nod_make_sov_len";

// ─── Sprint 28 — Win64 FFI trampolines ────────────────────────────────────
//
// One trampoline per arity 0..=8. Lowering emits a DirectCall against
// the synthetic `%winffi-call-N` callee; codegen recognises the prefix
// and declares the matching extern. The first arg of each trampoline
// is the static-area pointer of the entry's [`ApiStubEntry`] (baked as
// an `i64` constant by lowering); the remaining args are the Dylan
// caller's args, each passed as a tagged `i64` Word.
pub const NOD_WINFFI_CALL_0_SYMBOL: &str = "nod_winffi_call_0";
pub const NOD_WINFFI_CALL_1_SYMBOL: &str = "nod_winffi_call_1";
pub const NOD_WINFFI_CALL_2_SYMBOL: &str = "nod_winffi_call_2";
pub const NOD_WINFFI_CALL_3_SYMBOL: &str = "nod_winffi_call_3";
pub const NOD_WINFFI_CALL_4_SYMBOL: &str = "nod_winffi_call_4";
pub const NOD_WINFFI_CALL_5_SYMBOL: &str = "nod_winffi_call_5";
pub const NOD_WINFFI_CALL_6_SYMBOL: &str = "nod_winffi_call_6";
pub const NOD_WINFFI_CALL_7_SYMBOL: &str = "nod_winffi_call_7";
pub const NOD_WINFFI_CALL_8_SYMBOL: &str = "nod_winffi_call_8";

// ─── Sprint 24 — closures: <cell> and <environment> ───────────────────────
pub const NOD_MAKE_CELL_SYMBOL: &str = "nod_make_cell";
pub const NOD_CELL_GET_SYMBOL: &str = "nod_cell_get";
pub const NOD_CELL_SET_SYMBOL: &str = "nod_cell_set";
pub const NOD_ENV_CELL_SYMBOL: &str = "nod_env_cell";
pub const NOD_MAKE_ENVIRONMENT_SYMBOL: &str = "nod_make_environment";
pub const NOD_MAKE_CLOSURE_SYMBOL: &str = "nod_make_closure";

// ─── Sprint 22 — <table> + hashing ─────────────────────────────────────────
pub const NOD_MAKE_TABLE_SYMBOL: &str = "nod_make_table";
pub const NOD_TABLE_SIZE_SYMBOL: &str = "nod_table_size";
pub const NOD_TABLE_ELEMENT_SYMBOL: &str = "nod_table_element";
pub const NOD_TABLE_ELEMENT_OR_DEFAULT_SYMBOL: &str = "nod_table_element_or_default";
pub const NOD_TABLE_ELEMENT_SETTER_SYMBOL: &str = "nod_table_element_setter";
pub const NOD_TABLE_REMOVE_KEY_SYMBOL: &str = "nod_table_remove_key";
pub const NOD_TABLE_KEYS_SYMBOL: &str = "nod_table_keys";
pub const NOD_TABLE_VALUES_SYMBOL: &str = "nod_table_values";
pub const NOD_OBJECT_HASH_SYMBOL: &str = "nod_object_hash";
pub const NOD_OBJECT_EQUAL_P_SYMBOL: &str = "nod_object_equal_p";

/// Sprint 20b: `(dylan-name-as-emitted-by-lower, runtime-symbol, arity)`.
/// The lower pass emits the LHS name as the DirectCall callee; codegen
/// matches it here and emits a call into the RHS extern.
const SPRINT_20B_PRIMITIVES: &[(&str, &str, usize)] = &[
    ("nod_collection_size", NOD_COLLECTION_SIZE_SYMBOL, 1),
    ("nod_collection_concatenate", NOD_COLLECTION_CONCATENATE_SYMBOL, 2),
    ("nod_range_from", NOD_RANGE_FROM_SYMBOL, 1),
    ("nod_range_to", NOD_RANGE_TO_SYMBOL, 1),
    ("nod_range_by", NOD_RANGE_BY_SYMBOL, 1),
    ("nod_sov_size", NOD_SOV_SIZE_SYMBOL, 1),
    ("nod_sov_element", NOD_SOV_ELEMENT_SYMBOL, 2),
    ("nod_sov_element_setter", NOD_SOV_ELEMENT_SETTER_SYMBOL, 3),
    ("nod_stretchy_vector_size", NOD_STRETCHY_VECTOR_SIZE_SYMBOL, 1),
    ("nod_stretchy_vector_element", NOD_STRETCHY_VECTOR_ELEMENT_SYMBOL, 2),
    (
        "nod_stretchy_vector_element_setter",
        NOD_STRETCHY_VECTOR_ELEMENT_SETTER_SYMBOL,
        3,
    ),
    ("nod_stretchy_vector_push", NOD_STRETCHY_VECTOR_PUSH_SYMBOL, 2),
    ("nod_fip_init", NOD_FIP_INIT_SYMBOL, 1),
    ("nod_fip_finished_p", NOD_FIP_FINISHED_P_SYMBOL, 1),
    ("nod_fip_current_element", NOD_FIP_CURRENT_ELEMENT_SYMBOL, 1),
    ("nod_fip_advance", NOD_FIP_ADVANCE_SYMBOL, 1),
    ("nod_make_range", NOD_MAKE_RANGE_SYMBOL, 3),
    ("nod_make_stretchy_vector", NOD_MAKE_STRETCHY_VECTOR_SYMBOL, 1),
    // Sprint 21 — first-class function values.
    ("nod_make_function_ref", NOD_MAKE_FUNCTION_REF_SYMBOL, 2),
    ("nod_funcall0", NOD_FUNCALL0_SYMBOL, 1),
    ("nod_funcall1", NOD_FUNCALL1_SYMBOL, 2),
    ("nod_funcall2", NOD_FUNCALL2_SYMBOL, 3),
    ("nod_funcall3", NOD_FUNCALL3_SYMBOL, 4),
    ("nod_funcall4", NOD_FUNCALL4_SYMBOL, 5),
    ("nod_funcall5", NOD_FUNCALL5_SYMBOL, 6),
    ("nod_apply", NOD_APPLY_SYMBOL, 2),
    ("nod_make_sov_len", NOD_MAKE_SOV_LEN_SYMBOL, 1),
    // Sprint 22 — <table> + hashing.
    ("nod_make_table", NOD_MAKE_TABLE_SYMBOL, 1),
    ("nod_table_size", NOD_TABLE_SIZE_SYMBOL, 1),
    ("nod_table_element", NOD_TABLE_ELEMENT_SYMBOL, 2),
    ("nod_table_element_or_default", NOD_TABLE_ELEMENT_OR_DEFAULT_SYMBOL, 3),
    ("nod_table_element_setter", NOD_TABLE_ELEMENT_SETTER_SYMBOL, 3),
    ("nod_table_remove_key", NOD_TABLE_REMOVE_KEY_SYMBOL, 2),
    ("nod_table_keys", NOD_TABLE_KEYS_SYMBOL, 1),
    ("nod_table_values", NOD_TABLE_VALUES_SYMBOL, 1),
    ("nod_object_hash", NOD_OBJECT_HASH_SYMBOL, 1),
    ("nod_object_equal_p", NOD_OBJECT_EQUAL_P_SYMBOL, 2),
    // Sprint 24 — closures.
    ("nod_make_cell", NOD_MAKE_CELL_SYMBOL, 1),
    ("nod_cell_get", NOD_CELL_GET_SYMBOL, 1),
    ("nod_cell_set", NOD_CELL_SET_SYMBOL, 2),
    ("nod_env_cell", NOD_ENV_CELL_SYMBOL, 2),
    ("nod_make_environment", NOD_MAKE_ENVIRONMENT_SYMBOL, 1),
    ("nod_make_closure", NOD_MAKE_CLOSURE_SYMBOL, 3),
    // Sprint 28 — Win64 FFI trampolines. Arity here is the trampoline's
    // C-ABI arity (entry-pointer + user args), so `nod_winffi_call_N`
    // entry takes `N + 1` Dylan-side args.
    ("nod_winffi_call_0", NOD_WINFFI_CALL_0_SYMBOL, 1),
    ("nod_winffi_call_1", NOD_WINFFI_CALL_1_SYMBOL, 2),
    ("nod_winffi_call_2", NOD_WINFFI_CALL_2_SYMBOL, 3),
    ("nod_winffi_call_3", NOD_WINFFI_CALL_3_SYMBOL, 4),
    ("nod_winffi_call_4", NOD_WINFFI_CALL_4_SYMBOL, 5),
    ("nod_winffi_call_5", NOD_WINFFI_CALL_5_SYMBOL, 6),
    ("nod_winffi_call_6", NOD_WINFFI_CALL_6_SYMBOL, 7),
    ("nod_winffi_call_7", NOD_WINFFI_CALL_7_SYMBOL, 8),
    ("nod_winffi_call_8", NOD_WINFFI_CALL_8_SYMBOL, 9),
];

fn sprint_20b_primitive(name: &str) -> Option<(&'static str, usize)> {
    SPRINT_20B_PRIMITIVES
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, sym, ar)| (*sym, *ar))
}

pub type FunctionMap<'ctx> = HashMap<String, FunctionValue<'ctx>>;

/// Convert an `i1` to a Sprint 10 tagged-boolean Dylan value.
/// `#t` and `#f` are pointer-tagged Words referring to pinned heap
/// wrappers; we materialise them as `i64` constants via the literal
/// pool's `Immediates` and `select` between them on `i1`.
fn retag_bool<'ctx>(
    b: &Builder<'ctx>,
    i64ty: inkwell::types::IntType<'ctx>,
    i1: inkwell::values::IntValue<'ctx>,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
    let imm = nod_runtime::literal_pool_immediates();
    let true_c = i64ty.const_int(imm.true_.raw(), false);
    let false_c = i64ty.const_int(imm.false_.raw(), false);
    b.build_select(i1, true_c, false_c, "tag.bool.sel")
        .map_err(map_err)
}

/// Build an `i1` from a Sprint 10 tagged-boolean Word: `cond != #f`.
/// Used by `Terminator::If` and the boolean PrimOps. Dylan's truthiness
/// is "everything except `#f` is true", so the comparison is purely
/// pointer-identity against the pinned `#f` singleton.
fn untag_bool_to_i1<'ctx>(
    b: &Builder<'ctx>,
    i64ty: inkwell::types::IntType<'ctx>,
    v: inkwell::values::IntValue<'ctx>,
) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
    let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
    let imm = nod_runtime::literal_pool_immediates();
    let false_c = i64ty.const_int(imm.false_.raw(), false);
    b.build_int_compare(IntPredicate::NE, v, false_c, "untag.bool")
        .map_err(map_err)
}

pub struct CodegenOutput<'ctx> {
    pub module: Module<'ctx>,
    pub function_map: FunctionMap<'ctx>,
}

#[derive(Debug)]
pub enum CodegenError {
    UnknownCallee { in_function: String, callee: String },
    IndirectCallNotSupported { in_function: String },
    Builder(String),
    /// Sprint 11 stub. The `WriteBarrier` IR node exists for Sprint 12+
    /// slot setters; no lowering path emits it today.
    WriteBarrierNotEmitted { in_function: String },
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::UnknownCallee { in_function, callee } => {
                write!(f, "codegen: unknown callee `{callee}` in function `{in_function}`")
            }
            CodegenError::IndirectCallNotSupported { in_function } => write!(
                f,
                "codegen: indirect Call IR node not supported in Sprint 07 \
                 (function `{in_function}`)"
            ),
            CodegenError::Builder(e) => write!(f, "codegen: builder: {e}"),
            CodegenError::WriteBarrierNotEmitted { in_function } => write!(
                f,
                "codegen: WriteBarrier IR node emitted but Sprint 11 has no \
                 lowering path (function `{in_function}`); Sprint 12+ wires it"
            ),
        }
    }
}

impl std::error::Error for CodegenError {}

pub fn codegen_module<'ctx>(
    ctx: &'ctx Context,
    fns: &[DfmFunction],
    module_name: &str,
) -> Result<CodegenOutput<'ctx>, CodegenError> {
    let module = ctx.create_module(module_name);
    let builder = ctx.create_builder();

    // Pass 1: forward-declare every function so direct calls can resolve
    // regardless of declaration order (handles mutual recursion).
    let mut function_map: FunctionMap<'ctx> = HashMap::new();
    for f in fns {
        let fty = function_type(ctx, f);
        let fv = module.add_function(&f.name, fty, None);
        function_map.insert(f.name.clone(), fv);
    }

    // Pass 2: emit each body.
    for f in fns {
        let fv = function_map[&f.name];
        emit_function(ctx, &module, &builder, &function_map, f, fv)?;
    }

    Ok(CodegenOutput { module, function_map })
}

fn function_type<'ctx>(ctx: &'ctx Context, f: &DfmFunction) -> FunctionType<'ctx> {
    let param_types: Vec<BasicMetadataTypeEnum<'ctx>> = f
        .params
        .iter()
        .map(|p| {
            let ty = f.temp_type(*p);
            llvm_basic_type(ctx, ty).into()
        })
        .collect();
    match llvm_return_type(ctx, f.return_type) {
        Some(ret) => ret.fn_type(&param_types, false),
        None => ctx.void_type().fn_type(&param_types, false),
    }
}

fn llvm_basic_type<'ctx>(ctx: &'ctx Context, ty: TypeEstimate) -> BasicTypeEnum<'ctx> {
    match ty {
        // Sprint 09 ABI: `<integer>` and `<boolean>` are both tagged
        // `Word` values — a single `i64` per register/stack slot.
        // Sprint 10 promotes `<string>` to the same shape (tagged
        // pointer to a `<byte-string>` heap object).
        TypeEstimate::Integer | TypeEstimate::Boolean | TypeEstimate::String => {
            ctx.i64_type().into()
        }
        TypeEstimate::SingleFloat => ctx.f32_type().into(),
        TypeEstimate::DoubleFloat => ctx.f64_type().into(),
        TypeEstimate::Character => ctx.i32_type().into(),
        TypeEstimate::Unit | TypeEstimate::Top | TypeEstimate::Bottom => {
            // Top / Bottom default to i64 (kernel-subset choice; see DEFERRED).
            // Unit only appears as a return; values of Unit type never flow
            // through SSA, so this fallback never reads back.
            ctx.i64_type().into()
        }
        // Sprint 15: `Class(_)` is a tagged-pointer Word like `String`
        // — the lowering pass stores the runtime ClassId for narrowing
        // / dispatch purposes, but at the register level a user-class
        // instance is the same i64 tagged pointer as any other heap
        // object. `Singleton(_)` is reserved (not populated in Sprint
        // 15); same i64 fallback.
        TypeEstimate::Class(_) | TypeEstimate::Singleton(_) => ctx.i64_type().into(),
    }
}

fn llvm_return_type<'ctx>(
    ctx: &'ctx Context,
    ty: TypeEstimate,
) -> Option<BasicTypeEnum<'ctx>> {
    if matches!(ty, TypeEstimate::Unit) {
        None
    } else {
        Some(llvm_basic_type(ctx, ty))
    }
}

/// Per-function emission state. SSA values produced inside the function
/// are kept in `temps`; LLVM basic blocks are keyed on `BlockId`; phi
/// nodes are recorded in `phi_inputs` for a second-pass `add_incoming`.
struct Emit<'ctx, 'a> {
    ctx: &'ctx Context,
    module: &'a Module<'ctx>,
    builder: &'a Builder<'ctx>,
    function_map: &'a FunctionMap<'ctx>,
    func: &'a DfmFunction,
    llvm_fn: FunctionValue<'ctx>,
    blocks: HashMap<BlockId, BasicBlock<'ctx>>,
    block_phis: HashMap<BlockId, Vec<PhiValue<'ctx>>>,
    temps: HashMap<TempId, BasicValueEnum<'ctx>>,
    /// (target block, source block, args) — recorded as we emit each
    /// terminator. After all blocks are emitted, we walk this and call
    /// `add_incoming` on each phi node. Done in two phases so all
    /// predecessor SSA values are defined before phis read them.
    pending_incoming: Vec<(BlockId, BasicBlock<'ctx>, Vec<TempId>)>,
    /// Sprint 11b: a small pool of `i64` allocas in the entry block,
    /// reused across multiple safepoints. Indexed by allocation order.
    /// Each call's spill/reload sequence rents N slots starting at
    /// `safepoint_slots_used`, then returns them when the call
    /// finishes. Slots persist across calls; the pool grows as new
    /// peaks are reached.
    safepoint_slot_pool: Vec<inkwell::values::PointerValue<'ctx>>,
    /// Sprint 13: per-function counter for `Dispatch` call sites.
    /// Each `Computation::Dispatch` mints a fresh site id via this
    /// counter; the site id is baked into the cache slot at allocation
    /// time so `dump_dispatch` can identify which site fired.
    next_dispatch_site_id: u64,
}

fn emit_function<'ctx>(
    ctx: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    function_map: &FunctionMap<'ctx>,
    func: &DfmFunction,
    llvm_fn: FunctionValue<'ctx>,
) -> Result<(), CodegenError> {
    let mut state = Emit {
        ctx,
        module,
        builder,
        function_map,
        func,
        llvm_fn,
        blocks: HashMap::new(),
        block_phis: HashMap::new(),
        temps: HashMap::new(),
        pending_incoming: Vec::new(),
        safepoint_slot_pool: Vec::new(),
        next_dispatch_site_id: 0,
    };

    // Pre-create every LLVM basic block so terminators can branch
    // forward.
    for b in &func.blocks {
        let bb = ctx.append_basic_block(llvm_fn, &b.label);
        state.blocks.insert(b.id, bb);
    }

    // Bind function parameters to the entry block's SSA temps.
    for (i, p) in func.params.iter().enumerate() {
        let pv = llvm_fn
            .get_nth_param(i as u32)
            .expect("parameter index in range");
        state.temps.insert(*p, pv);
    }

    // For each non-entry block with `params`, create phi nodes at the
    // block's start. Phi values feed the block-arg temps.
    for b in &func.blocks {
        if b.params.is_empty() {
            continue;
        }
        let bb = state.blocks[&b.id];
        builder.position_at_end(bb);
        let mut phis = Vec::with_capacity(b.params.len());
        for &param in &b.params {
            let ty = func.temp_type(param);
            let llty = llvm_basic_type(ctx, ty);
            let phi = builder
                .build_phi(llty, &format!("phi.t{}", param.0))
                .map_err(|e| CodegenError::Builder(e.to_string()))?;
            state.temps.insert(param, phi.as_basic_value());
            phis.push(phi);
        }
        state.block_phis.insert(b.id, phis);
    }

    // Emit every block's computations + terminator.
    for b in &func.blocks {
        let bb = state.blocks[&b.id];
        builder.position_at_end(bb);
        for c in &b.computations {
            state.emit_computation(c)?;
        }
        state.emit_terminator(&b.terminator)?;
    }

    // Now that every block has been emitted (so every TempId is bound
    // to an LLVM value), wire up phi incomings.
    for (target_block, source_bb, args) in &state.pending_incoming {
        let phis = state.block_phis.get(target_block);
        let Some(phis) = phis else { continue };
        for (phi, arg_temp) in phis.iter().zip(args.iter()) {
            let v = *state
                .temps
                .get(arg_temp)
                .expect("phi incoming temp defined");
            phi.add_incoming(&[(&v, *source_bb)]);
        }
    }

    Ok(())
}

impl<'ctx, 'a> Emit<'ctx, 'a> {
    fn emit_computation(&mut self, c: &Computation) -> Result<(), CodegenError> {
        match c {
            Computation::Const { dst, value } => {
                let v = self.emit_const(*dst, value);
                self.temps.insert(*dst, v);
            }
            Computation::PrimOp { dst, op, args } => {
                let v = self.emit_primop(*op, args)?;
                self.temps.insert(*dst, v);
            }
            Computation::DirectCall {
                dst,
                callee,
                args,
                safepoint_roots,
            } => {
                let v = self.emit_direct_call(callee, args, *dst, safepoint_roots)?;
                if let Some(v) = v {
                    self.temps.insert(*dst, v);
                }
            }
            Computation::Call { .. } => {
                return Err(CodegenError::IndirectCallNotSupported {
                    in_function: self.func.name.clone(),
                });
            }
            Computation::TypeCheck { dst, value, class } => {
                let v = self.emit_type_check(*value, class)?;
                self.temps.insert(*dst, v);
            }
            // Sprint 11 stub: no lowering path emits WriteBarrier yet.
            // Sprint 12 (slot setters) is the first emitter; codegen
            // lowers `Computation::WriteBarrier` to a `*slot = value`
            // store plus a call into `nod_runtime::write_barrier`.
            Computation::WriteBarrier { .. } => {
                return Err(CodegenError::WriteBarrierNotEmitted {
                    in_function: self.func.name.clone(),
                });
            }
            Computation::LoadSlot { dst, instance, offset, slot_type } => {
                let v = self.emit_load_slot(*instance, *offset, *slot_type)?;
                self.temps.insert(*dst, v);
            }
            Computation::StoreSlot { dst, instance, offset, value, slot_type } => {
                let v = self.emit_store_slot(*instance, *offset, *value, *slot_type)?;
                self.temps.insert(*dst, v);
            }
            Computation::Dispatch {
                dst,
                generic_name,
                args,
                safepoint_roots,
            } => {
                let v = self.emit_dispatch(generic_name, args, *dst, safepoint_roots)?;
                if let Some(v) = v {
                    self.temps.insert(*dst, v);
                }
            }
            Computation::SealedDirectCall {
                dst,
                method,
                fallback_chain,
                generic_name,
                args,
                safepoint_roots,
            } => {
                let v = self.emit_sealed_direct_call(
                    method,
                    fallback_chain,
                    generic_name,
                    args,
                    *dst,
                    safepoint_roots,
                )?;
                if let Some(v) = v {
                    self.temps.insert(*dst, v);
                }
            }
        }
        Ok(())
    }

    /// Sprint 15 sealed-direct call codegen. Brackets the resolved
    /// direct call with a chain-frame push/pop pair so any
    /// `next-method()` inside the body walks the fallback chain
    /// identically to the runtime `nod_dispatch` path. The fallback
    /// chain's method body pointers are resolved by the JIT engine
    /// from the body-symbol names — we emit `ptrtoint(@symbol, i64)`
    /// constants and stash them in a stack-local array along with the
    /// args.
    ///
    /// For Sprint 15 the args are spilled to stack-local i64 slots; the
    /// chain frame's method pointers come from extern function
    /// declarations on the body symbols. The push shim memcpy's both
    /// into a heap-allocated chain frame; the pop shim drops it.
    fn emit_sealed_direct_call(
        &mut self,
        method: &str,
        fallback_chain: &[String],
        generic_name: &str,
        args: &[TempId],
        dst: TempId,
        safepoint_roots: &[TempId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        // Resolve the resolved-method body fn we want to call. The
        // body must be in the module's function table (it was added by
        // the lowering pass that registered the method definition).
        let Some(&callee_fn) = self.function_map.get(method) else {
            return Err(CodegenError::UnknownCallee {
                in_function: self.func.name.clone(),
                callee: method.to_string(),
            });
        };

        // Spill args into a stack-local i64 array.
        let arity = args.len();
        let args_arr_ty = i64ty.array_type(arity.max(1) as u32);
        let args_alloca = self
            .builder
            .build_alloca(args_arr_ty, "sd.args")
            .map_err(map_err)?;
        for (i, arg) in args.iter().enumerate() {
            let v = self.temp_val(*arg).into_int_value();
            let idx_const = i64ty.const_int(i as u64, false);
            // SAFETY (rust): GEP into a fixed-size i64 array.
            let gep = unsafe {
                self.builder.build_gep(
                    args_arr_ty,
                    args_alloca,
                    &[i64ty.const_zero(), idx_const],
                    &format!("sd.args.{i}"),
                )
            }
            .map_err(map_err)?;
            self.builder.build_store(gep, v).map_err(map_err)?;
        }

        // Stack-local fn-ptr array for the fallback chain. Each entry
        // is a ptrtoint of an extern declaration on the chain method
        // body symbol.
        let chain_len = fallback_chain.len();
        let chain_arr_ty = i64ty.array_type(chain_len.max(1) as u32);
        let chain_alloca = self
            .builder
            .build_alloca(chain_arr_ty, "sd.chain")
            .map_err(map_err)?;
        for (i, body_name) in fallback_chain.iter().enumerate() {
            // Ensure an extern function declaration exists for the
            // body symbol — same shape as `callee_fn`.
            let fn_val = match self.module.get_function(body_name) {
                Some(f) => f,
                None => {
                    // The chain method bodies are added to the module
                    // when their `Function` is lowered. If we don't
                    // see them, raise an error so we don't silently
                    // emit a broken sealed-direct.
                    return Err(CodegenError::UnknownCallee {
                        in_function: self.func.name.clone(),
                        callee: body_name.clone(),
                    });
                }
            };
            let fn_ptr_as_ptr = fn_val.as_global_value().as_pointer_value();
            let fn_ptr_as_int = self
                .builder
                .build_ptr_to_int(fn_ptr_as_ptr, i64ty, &format!("sd.chain.{i}.int"))
                .map_err(map_err)?;
            let idx_const = i64ty.const_int(i as u64, false);
            // SAFETY (rust): GEP into a fixed-size i64 array.
            let gep = unsafe {
                self.builder.build_gep(
                    chain_arr_ty,
                    chain_alloca,
                    &[i64ty.const_zero(), idx_const],
                    &format!("sd.chain.{i}"),
                )
            }
            .map_err(map_err)?;
            self.builder.build_store(gep, fn_ptr_as_int).map_err(map_err)?;
        }

        // Push the chain frame: nod_push_sealed_chain_frame(
        //   args_ptr, arity, methods_ptr, chain_len
        // ).
        let push_fn = match self.module.get_function(NOD_PUSH_SEALED_CHAIN_SYMBOL) {
            Some(f) => f,
            None => {
                let void_ty = self.ctx.void_type();
                let ty = void_ty.fn_type(
                    &[
                        ptr_ty.into(),
                        i64ty.into(),
                        ptr_ty.into(),
                        i64ty.into(),
                    ],
                    false,
                );
                self.module.add_function(NOD_PUSH_SEALED_CHAIN_SYMBOL, ty, None)
            }
        };
        let pop_fn = match self.module.get_function(NOD_POP_SEALED_CHAIN_SYMBOL) {
            Some(f) => f,
            None => {
                let void_ty = self.ctx.void_type();
                let ty = void_ty.fn_type(&[], false);
                self.module.add_function(NOD_POP_SEALED_CHAIN_SYMBOL, ty, None)
            }
        };

        self.builder
            .build_call(
                push_fn,
                &[
                    args_alloca.into(),
                    i64ty.const_int(arity as u64, false).into(),
                    chain_alloca.into(),
                    i64ty.const_int(chain_len as u64, false).into(),
                ],
                "sd.push",
            )
            .map_err(map_err)?;

        // Now the direct call. Bracket with the safepoint pair so any
        // allocation inside the method body is observed by GC.
        let rented = self.begin_safepoint(safepoint_roots)?;
        let arg_vals: Vec<BasicMetadataValueEnum<'ctx>> = args
            .iter()
            .map(|a| self.temp_val(*a).into())
            .collect();
        let name = format!("sd.t{}", dst.0);
        let site = self
            .builder
            .build_call(callee_fn, &arg_vals, &name)
            .map_err(map_err)?;
        self.end_safepoint(&rented)?;
        let result = site.try_as_basic_value().basic();

        // Pop the chain frame on the success path. (Panic-unwind from
        // the body would skip this — Sprint 19 wires structured
        // unwinding through `nod_resume`; for Sprint 15 the runtime
        // RAII guard isn't replicated here. Documented in DEFERRED.)
        self.builder
            .build_call(pop_fn, &[], "sd.pop")
            .map_err(map_err)?;

        let _ = generic_name;
        Ok(result)
    }

    fn emit_const(&self, dst: TempId, v: &ConstValue) -> BasicValueEnum<'ctx> {
        let ty = self.func.temp_type(dst);
        match v {
            ConstValue::Integer(n) => match ty {
                // Sprint 09: `<integer>` literals lower to *tagged*
                // fixnums. Bit 0 = 0, value shifted left by 1.
                TypeEstimate::Integer | TypeEstimate::Top | TypeEstimate::Bottom => {
                    let tagged = ((*n as i64) as u64).wrapping_shl(1);
                    self.ctx.i64_type().const_int(tagged, false).into()
                }
                TypeEstimate::Character => self
                    .ctx
                    .i32_type()
                    .const_int(*n as u64, true)
                    .into(),
                // Sprint 10: a `Boolean` temp arrived via
                // `ConstValue::Integer` — treat 0 as #f, anything else
                // as #t, materialising as the pinned singleton address.
                TypeEstimate::Boolean => {
                    let imm = nod_runtime::literal_pool_immediates();
                    let bits = if *n != 0 { imm.true_.raw() } else { imm.false_.raw() };
                    self.ctx.i64_type().const_int(bits, false).into()
                }
                _ => {
                    let tagged = ((*n as i64) as u64).wrapping_shl(1);
                    self.ctx.i64_type().const_int(tagged, false).into()
                }
            },
            ConstValue::Float(f) => match ty {
                TypeEstimate::SingleFloat => self.ctx.f32_type().const_float(*f).into(),
                _ => self.ctx.f64_type().const_float(*f).into(),
            },
            // Sprint 10: `#t` / `#f` are pinned heap-shape singletons.
            // Their addresses live in the process-global immediates
            // struct; bake the tagged-Word bit pattern as an i64 const.
            ConstValue::Bool(b) => {
                let imm = nod_runtime::literal_pool_immediates();
                let bits = if *b { imm.true_.raw() } else { imm.false_.raw() };
                self.ctx.i64_type().const_int(bits, false).into()
            }
            ConstValue::Char(c) => self
                .ctx
                .i32_type()
                .const_int(*c as u64, false)
                .into(),
            // Sprint 10: a `<byte-string>` literal is interned in the
            // process-global literal pool. The interned Word's raw bits
            // are baked as an i64 constant — JIT-loaded as a tagged
            // pointer to a `<byte-string>` heap object.
            ConstValue::String(s) => {
                let w = nod_runtime::intern_string_literal(s);
                self.ctx.i64_type().const_int(w.raw(), false).into()
            }
            // Sprint 10: `Unit` constants lower to `nil`'s pinned
            // singleton address. The DFM only uses Unit for the
            // never-returned value of a void function; baking nil
            // gives downstream callers a well-formed Word if they
            // accidentally read it.
            ConstValue::Unit => {
                let imm = nod_runtime::literal_pool_immediates();
                self.ctx.i64_type().const_int(imm.nil.raw(), false).into()
            }
            // Sprint 12: raw 64-bit constant — used by lowering to
            // bake `<class>` references, tagged class-metadata ptrs,
            // and interned symbol Words straight into the IR.
            ConstValue::WordBits(bits) => {
                self.ctx.i64_type().const_int(*bits, false).into()
            }
        }
    }

    fn emit_primop(
        &self,
        op: PrimOp,
        args: &[TempId],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let b = self.builder;
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let int2 = || -> (inkwell::values::IntValue<'ctx>, inkwell::values::IntValue<'ctx>) {
            (self.temp_val(args[0]).into_int_value(), self.temp_val(args[1]).into_int_value())
        };
        let float2 = || -> (inkwell::values::FloatValue<'ctx>, inkwell::values::FloatValue<'ctx>) {
            (
                self.temp_val(args[0]).into_float_value(),
                self.temp_val(args[1]).into_float_value(),
            )
        };
        let i64ty = self.ctx.i64_type();
        let one_i64 = i64ty.const_int(1, false);
        Ok(match op {
            // Tagged-stable: bit 0 of each operand is 0, so the sum's
            // bit 0 is also 0 and the value bits land exactly where
            // (a+b)<<1 expects them.
            PrimOp::AddInt => {
                let (l, r) = int2();
                b.build_int_add(l, r, "tag.add").map_err(map_err)?.into()
            }
            PrimOp::SubInt => {
                let (l, r) = int2();
                b.build_int_sub(l, r, "tag.sub").map_err(map_err)?.into()
            }
            // (a<<1) * (b<<1) = (a*b) << 2 — one bit too many. Shift
            // one operand right by 1 (arithmetic to preserve sign of
            // negative fixnums) before multiplying.
            PrimOp::MulInt => {
                let (l, r) = int2();
                let r_unshifted = b
                    .build_right_shift(r, one_i64, true, "tag.mul.untag")
                    .map_err(map_err)?;
                b.build_int_mul(l, r_unshifted, "tag.mul")
                    .map_err(map_err)?
                    .into()
            }
            // sdiv doesn't compose with shifted operands the way mul
            // does. Untag both, divide, retag: (a/b) << 1.
            PrimOp::DivInt => {
                let (l, r) = int2();
                let lu = b
                    .build_right_shift(l, one_i64, true, "tag.div.lu")
                    .map_err(map_err)?;
                let ru = b
                    .build_right_shift(r, one_i64, true, "tag.div.ru")
                    .map_err(map_err)?;
                let q = b
                    .build_int_signed_div(lu, ru, "tag.div.q")
                    .map_err(map_err)?;
                b.build_left_shift(q, one_i64, "tag.div.retag")
                    .map_err(map_err)?
                    .into()
            }
            PrimOp::ModInt | PrimOp::RemInt => {
                let (l, r) = int2();
                let lu = b
                    .build_right_shift(l, one_i64, true, "tag.rem.lu")
                    .map_err(map_err)?;
                let ru = b
                    .build_right_shift(r, one_i64, true, "tag.rem.ru")
                    .map_err(map_err)?;
                let m = b
                    .build_int_signed_rem(lu, ru, "tag.rem.m")
                    .map_err(map_err)?;
                b.build_left_shift(m, one_i64, "tag.rem.retag")
                    .map_err(map_err)?
                    .into()
            }
            // 0 - (a<<1) = (-a)<<1; bit 0 stays 0.
            PrimOp::NegInt => {
                let v = self.temp_val(args[0]).into_int_value();
                let zero = v.get_type().const_zero();
                b.build_int_sub(zero, v, "tag.neg").map_err(map_err)?.into()
            }
            PrimOp::AddFloat => {
                let (l, r) = float2();
                b.build_float_add(l, r, "fadd").map_err(map_err)?.into()
            }
            PrimOp::SubFloat => {
                let (l, r) = float2();
                b.build_float_sub(l, r, "fsub").map_err(map_err)?.into()
            }
            PrimOp::MulFloat => {
                let (l, r) = float2();
                b.build_float_mul(l, r, "fmul").map_err(map_err)?.into()
            }
            PrimOp::DivFloat => {
                let (l, r) = float2();
                b.build_float_div(l, r, "fdiv").map_err(map_err)?.into()
            }
            PrimOp::NegFloat => {
                let v = self.temp_val(args[0]).into_float_value();
                b.build_float_neg(v, "fneg").map_err(map_err)?.into()
            }
            // Comparisons run directly on tagged operands — the
            // shift-left-by-1 preserves the signed ordering. The i1
            // result is zext'd to i64 and shifted to land in the
            // tagged-boolean encoding (#t = 2, #f = 0).
            PrimOp::EqInt => {
                let (l, r) = int2();
                let i1 = b
                    .build_int_compare(IntPredicate::EQ, l, r, "tag.eq")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::NeInt => {
                let (l, r) = int2();
                let i1 = b
                    .build_int_compare(IntPredicate::NE, l, r, "tag.ne")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::LtInt => {
                let (l, r) = int2();
                let i1 = b
                    .build_int_compare(IntPredicate::SLT, l, r, "tag.lt")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::GtInt => {
                let (l, r) = int2();
                let i1 = b
                    .build_int_compare(IntPredicate::SGT, l, r, "tag.gt")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::LeInt => {
                let (l, r) = int2();
                let i1 = b
                    .build_int_compare(IntPredicate::SLE, l, r, "tag.le")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::GeInt => {
                let (l, r) = int2();
                let i1 = b
                    .build_int_compare(IntPredicate::SGE, l, r, "tag.ge")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::EqFloat => {
                let (l, r) = float2();
                let i1 = b
                    .build_float_compare(FloatPredicate::OEQ, l, r, "fcmp.eq")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::LtFloat => {
                let (l, r) = float2();
                let i1 = b
                    .build_float_compare(FloatPredicate::OLT, l, r, "fcmp.lt")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::GtFloat => {
                let (l, r) = float2();
                let i1 = b
                    .build_float_compare(FloatPredicate::OGT, l, r, "fcmp.gt")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::LeFloat => {
                let (l, r) = float2();
                let i1 = b
                    .build_float_compare(FloatPredicate::OLE, l, r, "fcmp.le")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            PrimOp::GeFloat => {
                let (l, r) = float2();
                let i1 = b
                    .build_float_compare(FloatPredicate::OGE, l, r, "fcmp.ge")
                    .map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), i1)?
            }
            // Sprint 10: booleans are pinned-pointer Words; pointer
            // identity (not bit patterns) carries truth. Untag to i1,
            // apply the LLVM bool op, retag.
            PrimOp::BoolAnd => {
                let (l, r) = int2();
                let li = untag_bool_to_i1(b, self.ctx.i64_type(), l)?;
                let ri = untag_bool_to_i1(b, self.ctx.i64_type(), r)?;
                let both = b.build_and(li, ri, "bool.and.i1").map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), both)?
            }
            PrimOp::BoolOr => {
                let (l, r) = int2();
                let li = untag_bool_to_i1(b, self.ctx.i64_type(), l)?;
                let ri = untag_bool_to_i1(b, self.ctx.i64_type(), r)?;
                let either = b.build_or(li, ri, "bool.or.i1").map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), either)?
            }
            PrimOp::BoolNot => {
                let v = self.temp_val(args[0]).into_int_value();
                let vi = untag_bool_to_i1(b, self.ctx.i64_type(), v)?;
                let one_i1 = self.ctx.bool_type().const_int(1, false);
                let not = b.build_xor(vi, one_i1, "bool.not.i1").map_err(map_err)?;
                retag_bool(b, self.ctx.i64_type(), not)?
            }
        })
    }

    fn emit_type_check(
        &mut self,
        value: TempId,
        class: &ClassCheck,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let b = self.builder;
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let v = self.temp_val(value).into_int_value();
        match class {
            // `<integer>` test: bit 0 == 0. AND with 1, compare to 0.
            ClassCheck::Integer => {
                let one = i64ty.const_int(1, false);
                let masked = b.build_and(v, one, "tcheck.int.mask").map_err(map_err)?;
                let zero = i64ty.const_zero();
                let i1 = b
                    .build_int_compare(IntPredicate::EQ, masked, zero, "tcheck.int.cmp")
                    .map_err(map_err)?;
                retag_bool(b, i64ty, i1)
            }
            // Wrapper-tagged class tests against seed classes. The helper
            // returns an i1 that's true iff `v` is pointer-tagged AND
            // its wrapper carries the target class id.
            ClassCheck::Boolean => self.emit_wrapper_class_check(v, ClassId::BOOLEAN),
            ClassCheck::String => self.emit_wrapper_class_check(v, ClassId::BYTE_STRING),
            ClassCheck::Symbol => self.emit_wrapper_class_check(v, ClassId::SYMBOL),
            ClassCheck::Vector => self.emit_wrapper_class_check(v, ClassId::SIMPLE_OBJECT_VECTOR),
            ClassCheck::Character => self.emit_wrapper_class_check(v, ClassId::CHARACTER),
            ClassCheck::EmptyList => self.emit_wrapper_class_check(v, ClassId::EMPTY_LIST),
            ClassCheck::UserClass { id, .. } => {
                // Sprint 12: call the runtime `nod_is_instance_of`
                // helper. Walks the value's class CPL — handles both
                // user classes and seed-class supers.
                let is_inst_fn = match self.module.get_function(NOD_IS_INSTANCE_OF_SYMBOL) {
                    Some(f) => f,
                    None => {
                        let ty = i64ty.fn_type(&[i64ty.into(), i64ty.into()], false);
                        self.module.add_function(NOD_IS_INSTANCE_OF_SYMBOL, ty, None)
                    }
                };
                let class_const = i64ty.const_int(*id as u64, false);
                let site = self
                    .builder
                    .build_call(is_inst_fn, &[v.into(), class_const.into()], "tcheck.user")
                    .map_err(map_err)?;
                Ok(site
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| CodegenError::Builder("nod_is_instance_of returned void".into()))?)
            }
            // Anything else: stub. Sprint 12 wires class-id dispatch.
            ClassCheck::Unsupported { .. } => Ok(i64ty.const_zero().into()),
        }
    }

    /// Read the runtime class id of a Word as an i64. Sprint 13's
    /// inline-cache code uses this to compute the cache key; the same
    /// logic appears in `emit_wrapper_class_check`, factored here so
    /// both stay in sync.
    ///
    /// Pseudocode:
    /// ```text
    ///   is_ptr = (w & 1) == 1
    ///   addr = select(is_ptr, w & ~1, fallback_addr)
    ///   wrapper = load i64, ptr addr
    ///   class_id = wrapper & 0xFFFF_FFFF
    ///   fixnum_class = <integer>'s class id (1)
    ///   result = select(is_ptr, class_id, fixnum_class)
    /// ```
    ///
    /// For fixnum inputs the wrapper load is redirected through a
    /// pinned safe address (the `#f` singleton) so it can't fault, and
    /// the final select substitutes the integer class id.
    fn emit_word_class_id(
        &self,
        v: inkwell::values::IntValue<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
        let b = self.builder;
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let one = i64ty.const_int(1, false);
        let not_one = i64ty.const_int(!1_u64, false);

        let tag_bits = b.build_and(v, one, "cls.id.tag").map_err(map_err)?;
        let is_ptr_i1 = b
            .build_int_compare(IntPredicate::EQ, tag_bits, one, "cls.id.isptr")
            .map_err(map_err)?;

        // Fallback address for fixnum inputs: pinned `#f` singleton's
        // wrapper. Used purely to keep the load fault-free; the result
        // is overwritten by the integer-class select below.
        let fallback = nod_runtime::literal_pool_immediates().false_.raw() & !1_u64;
        let fallback_const = i64ty.const_int(fallback, false);

        let masked = b.build_and(v, not_one, "cls.id.untag").map_err(map_err)?;
        let addr_i64 = b
            .build_select(is_ptr_i1, masked, fallback_const, "cls.id.addr")
            .map_err(map_err)?
            .into_int_value();
        let addr_ptr = b
            .build_int_to_ptr(
                addr_i64,
                self.ctx.ptr_type(inkwell::AddressSpace::default()),
                "cls.id.ptr",
            )
            .map_err(map_err)?;
        let wrapper = b
            .build_load(i64ty, addr_ptr, "cls.id.wrap")
            .map_err(map_err)?
            .into_int_value();
        let class_mask = i64ty.const_int(0xFFFF_FFFF, false);
        let ptr_class = b
            .build_and(wrapper, class_mask, "cls.id.ptr_class")
            .map_err(map_err)?;
        let integer_class = i64ty.const_int(ClassId::INTEGER.0 as u64, false);
        let result = b
            .build_select(is_ptr_i1, ptr_class, integer_class, "cls.id.value")
            .map_err(map_err)?
            .into_int_value();
        Ok(result)
    }

    /// Emit the wrapper-load-and-class-compare sequence.
    ///
    /// In LLVM-IR shape (the actual IR uses i64 throughout):
    ///
    /// ```text
    ///   ; v is the tagged Word.
    ///   is_ptr = (v & 1) == 1                           ; pointer-tag check
    ///   if !is_ptr -> result = 0 (false)
    ///   addr = v & ~1                                    ; untag
    ///   wrapper = load i64, i64* addr                    ; read header
    ///   class_id = wrapper & 0xFFFF_FFFF                 ; low 32 bits
    ///   class_eq = class_id == target_class_id
    ///   result = (is_ptr AND class_eq) << 1              ; tagged boolean
    /// ```
    ///
    /// The pointer-tag check is preserved with an `AND` so a fixnum
    /// input short-circuits to false — we deliberately do NOT branch
    /// (no new basic block) because every operand to `AND` is a pure
    /// computation. For fixnum inputs we redirect the load through a
    /// pinned fallback address (the `#f` singleton wrapper) so the
    /// load itself never faults; the AND with `is_ptr_i1` then drops
    /// whatever class the fallback happens to carry. This trades a
    /// load on the false path for branchless lowering.
    fn emit_wrapper_class_check(
        &self,
        v: inkwell::values::IntValue<'ctx>,
        target_class: ClassId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let b = self.builder;
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let one = i64ty.const_int(1, false);
        let not_one = i64ty.const_int(!1_u64, false);

        // is_ptr_i1 = (v & 1) == 1
        let tag_bits = b.build_and(v, one, "tcheck.cls.tag").map_err(map_err)?;
        let is_ptr_i1 = b
            .build_int_compare(IntPredicate::EQ, tag_bits, one, "tcheck.cls.isptr")
            .map_err(map_err)?;

        // For fixnum inputs we still need a valid address to load from.
        // Replace with a known-safe pinned address (the `#f` singleton
        // wrapper). The AND with `is_ptr_i1` at the end discards the
        // load result anyway, but the load itself must not fault.
        let fallback = nod_runtime::literal_pool_immediates().false_.raw() & !1_u64;
        let fallback_const = i64ty.const_int(fallback, false);

        let masked = b.build_and(v, not_one, "tcheck.cls.untag").map_err(map_err)?;
        let addr_i64 = b
            .build_select(is_ptr_i1, masked, fallback_const, "tcheck.cls.addr")
            .map_err(map_err)?
            .into_int_value();
        let addr_ptr = b
            .build_int_to_ptr(
                addr_i64,
                self.ctx.ptr_type(inkwell::AddressSpace::default()),
                "tcheck.cls.ptr",
            )
            .map_err(map_err)?;
        let wrapper = b
            .build_load(i64ty, addr_ptr, "tcheck.cls.wrap")
            .map_err(map_err)?
            .into_int_value();
        let class_mask = i64ty.const_int(0xFFFF_FFFF, false);
        let class_id = b
            .build_and(wrapper, class_mask, "tcheck.cls.id")
            .map_err(map_err)?;
        let target = i64ty.const_int(target_class.0 as u64, false);
        let class_eq_i1 = b
            .build_int_compare(IntPredicate::EQ, class_id, target, "tcheck.cls.eq")
            .map_err(map_err)?;
        let both = b.build_and(is_ptr_i1, class_eq_i1, "tcheck.cls.both").map_err(map_err)?;
        retag_bool(b, i64ty, both)
    }

    fn emit_direct_call(
        &mut self,
        callee: &str,
        args: &[TempId],
        dst: TempId,
        safepoint_roots: &[TempId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        // Sprint 10 builtin: `format-out` lowers to a call into the
        // `nod_format_out` extern shim. Args are padded with zeros so
        // the C ABI sees a fixed (u64, u64, u64, u64) -> u64.
        if callee == "format-out" {
            return self.emit_format_out_call(args, dst, safepoint_roots);
        }
        if callee == "%make" {
            return self.emit_make_call(args, dst, safepoint_roots);
        }
        // Sprint 14: runtime-resolved `next-method` shims. Take no args
        // and return a single `i64` (a Dylan Word).
        if callee == NOD_NEXT_METHOD_SYMBOL || callee == NOD_HAS_NEXT_METHOD_SYMBOL {
            return self.emit_next_method_call(callee, dst, safepoint_roots);
        }
        // Sprint 16: `<pair>` / `<list>` builtins. The synthetic
        // callee names emitted by `nod-sema::lower::lower_list_builtin`
        // map one-to-one onto the runtime shims declared above.
        if let Some((sym, arity)) = match callee {
            "%pair-alloc" => Some((NOD_PAIR_ALLOC_SYMBOL, 2)),
            "%pair-head" => Some((NOD_PAIR_HEAD_SYMBOL, 1)),
            "%pair-tail" => Some((NOD_PAIR_TAIL_SYMBOL, 1)),
            "%empty?" => Some((NOD_EMPTY_P_SYMBOL, 1)),
            "%nil" => Some((NOD_NIL_SYMBOL, 0)),
            _ => None,
        } {
            return self.emit_list_builtin_call(sym, arity, args, dst, safepoint_roots);
        }
        // Sprint 19: `signal` / `condition-message` / `block`
        // orchestration builtins. Each is a fixed-arity extern shim
        // resolved to a `nod_runtime` symbol at JIT-engine creation.
        if let Some((sym, arity)) = match callee {
            "%signal" => Some((NOD_SIGNAL_SYMBOL, 1)),
            "%condition-message" => Some((NOD_CONDITION_MESSAGE_SYMBOL, 1)),
            "%make-exit-procedure" => Some((NOD_MAKE_EXIT_PROCEDURE_SYMBOL, 1)),
            "%invoke-exit" => Some((NOD_INVOKE_EXIT_SYMBOL, 2)),
            "%run-block" => Some((NOD_RUN_BLOCK_SYMBOL, 9)), // block_id + 8 captured
            _ => None,
        } {
            return self.emit_list_builtin_call(sym, arity, args, dst, safepoint_roots);
        }
        // Sprint 20b: `%`-prefixed collection / FIP / primitive ops.
        // The lower pass emits the runtime symbol verbatim as the
        // DirectCall callee (see `LOWER_PRIMITIVE_TABLE` in
        // `nod-sema/src/lower.rs`); we match it against the
        // SPRINT_20B_PRIMITIVES table and emit the same fixed-arity
        // i64-shaped call shape used by the Sprint 16 list builtins.
        if let Some((sym, arity)) = sprint_20b_primitive(callee) {
            return self.emit_list_builtin_call(sym, arity, args, dst, safepoint_roots);
        }
        // Sprint 20b: when the callee isn't in this module's function
        // table, check the process-global dispatch registry — stdlib
        // methods (and any other JIT-resident method body) register a
        // body-fn-name → address mapping via `add_method_named`. If
        // one matches the callee, declare it as an extern in this
        // module with the standard `(i64, …) -> i64` ABI; `jit.rs`
        // resolves the symbol via `find_method_body_ptr` at engine
        // creation. This unblocks dispatch-resolver emissions of
        // `DirectCall { callee: "<generic>$<spec>" }` whose body lives
        // in a different JIT module (e.g. `nod-sema::stdlib`).
        let callee_fn = if let Some(&f) = self.function_map.get(callee) {
            f
        } else if let Some(existing) = self.module.get_function(callee) {
            // Already declared as an extern earlier in this emission.
            existing
        } else if nod_runtime::find_method_body_ptr(callee).is_some() {
            // Declare an extern with the standard method ABI. Methods
            // take their args as `u64` and return `u64`. The actual
            // arity matches `args.len()`.
            let i64ty = self.ctx.i64_type();
            let params: Vec<BasicMetadataTypeEnum<'ctx>> =
                (0..args.len()).map(|_| i64ty.into()).collect();
            let fty = i64ty.fn_type(&params, false);
            self.module.add_function(callee, fty, None)
        } else {
            return Err(CodegenError::UnknownCallee {
                in_function: self.func.name.clone(),
                callee: callee.to_string(),
            });
        };
        let arg_vals: Vec<BasicMetadataValueEnum<'ctx>> = args
            .iter()
            .map(|a| self.temp_val(*a).into())
            .collect();
        let name = format!("call.t{}", dst.0);
        // Sprint 11b: bracket the call with register/unregister pairs.
        // Sprint 12-shaped DirectCalls into user-defined Dylan functions
        // may transitively allocate (a callee that calls `make`); we
        // protect across every such call. Pure-arith Sprint 07-style
        // direct calls have an empty `safepoint_roots` list (the
        // liveness pass produced no live pointer-shaped temps) and the
        // bracketing is a no-op.
        let rented = self.begin_safepoint(safepoint_roots)?;
        let site = self
            .builder
            .build_call(callee_fn, &arg_vals, &name)
            .map_err(|e| CodegenError::Builder(e.to_string()))?;
        self.end_safepoint(&rented)?;
        Ok(site.try_as_basic_value().basic())
    }

    /// Sprint 12 builtin: `make` lowers to a call into the `nod_make`
    /// extern shim. Lowering produces args in the shape:
    /// `[class_metadata_addr_const, name_0, value_0, name_1, value_1, ...]`.
    /// We pad to the fixed `2 + 2*MAKE_MAX_KW_PAIRS` arity expected by
    /// `nod_make`, inserting `kw_count` as the second argument.
    fn emit_make_call(
        &mut self,
        args: &[TempId],
        dst: TempId,
        safepoint_roots: &[TempId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let max_pairs = nod_runtime::MAKE_MAX_KW_PAIRS;
        if args.is_empty() {
            return Err(CodegenError::Builder(
                "make: missing class argument".to_string(),
            ));
        }
        let pair_count = (args.len() - 1) / 2;
        if pair_count > max_pairs {
            return Err(CodegenError::Builder(format!(
                "make: Sprint 12 supports up to {max_pairs} keyword pairs, got {pair_count}"
            )));
        }
        let make_fn = match self.module.get_function(NOD_MAKE_SYMBOL) {
            Some(f) => f,
            None => {
                // Signature: (class_ptr, kw_count, [name, val] * max_pairs) -> u64
                let mut params: Vec<BasicMetadataTypeEnum<'ctx>> =
                    Vec::with_capacity(2 + 2 * max_pairs);
                params.push(i64ty.into());
                params.push(i64ty.into());
                for _ in 0..max_pairs {
                    params.push(i64ty.into());
                    params.push(i64ty.into());
                }
                let ty = i64ty.fn_type(&params, false);
                self.module.add_function(NOD_MAKE_SYMBOL, ty, None)
            }
        };
        let zero = i64ty.const_zero();
        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> =
            Vec::with_capacity(2 + 2 * max_pairs);
        // First arg: class metadata pointer (constant Word baked into the IR).
        let class_arg = self.temp_val(args[0]).into_int_value();
        call_args.push(class_arg.into());
        // Second arg: kw_count.
        call_args.push(i64ty.const_int(pair_count as u64, false).into());
        // Then 2*max_pairs slots for name/value pairs.
        for i in 0..max_pairs {
            if i < pair_count {
                let name_t = args[1 + 2 * i];
                let val_t = args[1 + 2 * i + 1];
                call_args.push(self.temp_val(name_t).into());
                call_args.push(self.temp_val(val_t).into());
            } else {
                call_args.push(zero.into());
                call_args.push(zero.into());
            }
        }
        let name = format!("call.t{}", dst.0);
        let rented = self.begin_safepoint(safepoint_roots)?;
        let site = self
            .builder
            .build_call(make_fn, &call_args, &name)
            .map_err(map_err)?;
        self.end_safepoint(&rented)?;
        Ok(site.try_as_basic_value().basic())
    }

    fn emit_format_out_call(
        &mut self,
        args: &[TempId],
        dst: TempId,
        safepoint_roots: &[TempId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        if args.is_empty() || args.len() > 4 {
            return Err(CodegenError::Builder(format!(
                "format-out: Sprint 10 supports arity 1..=4, got {}",
                args.len()
            )));
        }
        // Lookup or declare the extern.
        let fmt_fn = match self.module.get_function(FORMAT_OUT_SYMBOL) {
            Some(f) => f,
            None => {
                let ty = i64ty.fn_type(
                    &[i64ty.into(), i64ty.into(), i64ty.into(), i64ty.into()],
                    false,
                );
                self.module.add_function(FORMAT_OUT_SYMBOL, ty, None)
            }
        };
        // Pad to four i64 args, zero-filling missing slots.
        let zero = i64ty.const_zero();
        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(4);
        for i in 0..4 {
            if let Some(t) = args.get(i) {
                call_args.push(self.temp_val(*t).into());
            } else {
                call_args.push(zero.into());
            }
        }
        let name = format!("call.t{}", dst.0);
        let rented = self.begin_safepoint(safepoint_roots)?;
        let site = self
            .builder
            .build_call(fmt_fn, &call_args, &name)
            .map_err(map_err)?;
        self.end_safepoint(&rented)?;
        Ok(site.try_as_basic_value().basic())
    }

    /// Sprint 14: lower a call to one of the runtime `next-method`
    /// shims (`nod_next_method` / `nod_has_next_method`). Both take no
    /// args and return a single `i64` (Dylan Word). The dispatch
    /// chain frame the shim consults is pushed by `nod_dispatch`
    /// when the current method was reached through dispatch with more
    /// than one applicable method.
    fn emit_next_method_call(
        &mut self,
        callee: &str,
        dst: TempId,
        safepoint_roots: &[TempId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let fn_ = match self.module.get_function(callee) {
            Some(f) => f,
            None => {
                // Signature: `i64 ()`.
                let ty = i64ty.fn_type(&[], false);
                self.module.add_function(callee, ty, None)
            }
        };
        let name = format!("call.t{}", dst.0);
        let rented = self.begin_safepoint(safepoint_roots)?;
        let site = self
            .builder
            .build_call(fn_, &[], &name)
            .map_err(map_err)?;
        self.end_safepoint(&rented)?;
        Ok(site.try_as_basic_value().basic())
    }

    /// Sprint 16: lower a `<pair>` / `<list>` builtin to a call into the
    /// matching runtime shim. `sym` is the JIT-side symbol (one of
    /// `NOD_PAIR_ALLOC_SYMBOL` etc.); `arity` is the number of `i64`
    /// arguments the shim takes (and equals `args.len()` after the
    /// lowering pass's arity check). The return is always a single
    /// `i64` (a Dylan Word).
    ///
    /// All five shims observe the standard safepoint discipline so a
    /// minor GC fired during `pair(...)` finds every live pointer-shaped
    /// temp in the registered-roots table.
    fn emit_list_builtin_call(
        &mut self,
        sym: &str,
        arity: usize,
        args: &[TempId],
        dst: TempId,
        safepoint_roots: &[TempId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        debug_assert_eq!(args.len(), arity, "Sprint 16 builtin arity mismatch");
        let fn_ = match self.module.get_function(sym) {
            Some(f) => f,
            None => {
                let params: Vec<BasicMetadataTypeEnum<'ctx>> =
                    (0..arity).map(|_| i64ty.into()).collect();
                let ty = i64ty.fn_type(&params, false);
                self.module.add_function(sym, ty, None)
            }
        };
        let call_args: Vec<BasicMetadataValueEnum<'ctx>> = args
            .iter()
            .map(|a| self.temp_val(*a).into())
            .collect();
        let name = format!("call.t{}", dst.0);
        let rented = self.begin_safepoint(safepoint_roots)?;
        let site = self
            .builder
            .build_call(fn_, &call_args, &name)
            .map_err(map_err)?;
        self.end_safepoint(&rented)?;
        Ok(site.try_as_basic_value().basic())
    }

    /// Lower a `LoadSlot` IR node. Untag the instance Word, GEP to
    /// the slot byte offset, load 8 bytes.
    fn emit_load_slot(
        &self,
        instance: TempId,
        offset: usize,
        _slot_type: SlotTypeKind,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let b = self.builder;
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let i8ty = self.ctx.i8_type();
        let inst = self.temp_val(instance).into_int_value();
        let not_one = i64ty.const_int(!1_u64, false);
        let addr_i64 = b
            .build_and(inst, not_one, "slot.load.untag")
            .map_err(map_err)?;
        let base_ptr = b
            .build_int_to_ptr(
                addr_i64,
                self.ctx.ptr_type(inkwell::AddressSpace::default()),
                "slot.load.base",
            )
            .map_err(map_err)?;
        let offset_const = i64ty.const_int(offset as u64, false);
        // SAFETY-equivalent: GEP at byte offset.
        let slot_ptr = unsafe {
            b.build_in_bounds_gep(i8ty, base_ptr, &[offset_const], "slot.load.gep")
                .map_err(map_err)?
        };
        let val = b
            .build_load(i64ty, slot_ptr, "slot.load.val")
            .map_err(map_err)?;
        Ok(val)
    }

    /// Lower a `StoreSlot` IR node. Untag the instance, GEP to the
    /// slot, store the value, then call into `nod_runtime::write_barrier`
    /// (which marks the card if the slot is in old).
    fn emit_store_slot(
        &mut self,
        instance: TempId,
        offset: usize,
        value: TempId,
        _slot_type: SlotTypeKind,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let b = self.builder;
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let i8ty = self.ctx.i8_type();
        let inst = self.temp_val(instance).into_int_value();
        let val = self.temp_val(value).into_int_value();
        let not_one = i64ty.const_int(!1_u64, false);
        let addr_i64 = b
            .build_and(inst, not_one, "slot.store.untag")
            .map_err(map_err)?;
        let base_ptr = b
            .build_int_to_ptr(
                addr_i64,
                self.ctx.ptr_type(inkwell::AddressSpace::default()),
                "slot.store.base",
            )
            .map_err(map_err)?;
        let offset_const = i64ty.const_int(offset as u64, false);
        let slot_ptr = unsafe {
            b.build_in_bounds_gep(i8ty, base_ptr, &[offset_const], "slot.store.gep")
                .map_err(map_err)?
        };
        b.build_store(slot_ptr, val).map_err(map_err)?;
        // Card-mark via the runtime. The runtime helper takes the slot
        // pointer's raw address and the new value; we call the
        // `nod_runtime_card_mark` shim (defined alongside the others).
        let card_fn = match self.module.get_function(NOD_CARD_MARK_SYMBOL) {
            Some(f) => f,
            None => {
                let ty = self
                    .ctx
                    .void_type()
                    .fn_type(&[i64ty.into()], false);
                self.module.add_function(NOD_CARD_MARK_SYMBOL, ty, None)
            }
        };
        let slot_addr_const = b
            .build_ptr_to_int(slot_ptr, i64ty, "slot.store.addr")
            .map_err(map_err)?;
        b.build_call(card_fn, &[slot_addr_const.into()], "slot.store.barrier")
            .map_err(map_err)?;
        // The "value" of a store is the stored value (allows `slot := v`
        // to be used as an expression in Dylan).
        Ok(val.into())
    }

    /// Sprint 13: lower a `Dispatch` IR node into an inline-cache
    /// check + fast-path direct call + slow-path runtime dispatch.
    ///
    /// IR shape per call site (with N = args.len(), capped at 8):
    ///
    /// ```text
    ///   ; ----- inline cache check -----
    ///   %r           = args[0]                              ; receiver word
    ///   %r_class     = call <emit_word_class_id>(%r)        ; i64 class id
    ///   %cached_cls  = load atomic i64, ptr @cache_class_for_site_N, monotonic
    ///   %cached_mthd = load atomic i64, ptr @cache_method_for_site_N, monotonic
    ///   %cached_gen  = load atomic i64, ptr @cache_gen_for_site_N, monotonic
    ///   %gen         = load atomic i64, ptr @generic.GENERIC_NAME.generation, monotonic
    ///   %class_ok    = icmp eq i64 %r_class, %cached_cls
    ///   %gen_ok      = icmp eq i64 %gen, %cached_gen
    ///   %nonzero     = icmp ne i64 %cached_cls, 0
    ///   %cache_hit   = and i1 %class_ok, (and i1 %gen_ok, %nonzero)
    ///   br i1 %cache_hit, label %fast_call, label %slow_call
    ///
    /// fast_call:
    ///   ; bump hits counter
    ///   atomicrmw add ptr @cache_hits_for_site_N, i64 1 monotonic
    ///   %fn = inttoptr i64 %cached_mthd to ptr
    ///   %r_fast = call i64 %fn(args...)
    ///   br label %dispatch_done
    ///
    /// slow_call:
    ///   ; nod_dispatch bumps misses and updates the cache itself
    ///   %r_slow = call i64 @nod_dispatch(generic_ptr, cache_slot_ptr,
    ///                                    arity, a0..a7)
    ///   br label %dispatch_done
    ///
    /// dispatch_done:
    ///   %result = phi i64 [ %r_fast, %fast_call ], [ %r_slow, %slow_call ]
    /// ```
    ///
    /// The cache slot's address is baked into the IR as an `i64`
    /// constant. The slot lives in the runtime's static area (pinned
    /// for the process lifetime) so subsequent re-JITs of unrelated
    /// modules don't clobber it.
    ///
    /// Safepoint roots are spilled+registered ONCE before the diamond
    /// and unregistered+reloaded at the join — both paths share the
    /// same root protection, and the post-dispatch `temps[i]` mapping
    /// reflects any GC evacuation that ran in either branch.
    fn emit_dispatch(
        &mut self,
        generic_name: &str,
        args: &[TempId],
        dst: TempId,
        safepoint_roots: &[TempId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, CodegenError> {
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

        if args.is_empty() {
            return Err(CodegenError::Builder(format!(
                "dispatch: `{generic_name}` has no arguments (need at least one receiver)"
            )));
        }
        if args.len() > 8 {
            return Err(CodegenError::Builder(format!(
                "dispatch: `{generic_name}` arity {} exceeds Sprint 13 cap of 8 (lifted in Sprint 23 c-ffi)",
                args.len()
            )));
        }

        // Reserve a unique site id for this call site.
        let site_id = self.next_dispatch_site_id;
        self.next_dispatch_site_id += 1;

        // Bake addresses of the GenericFunction and the CacheSlot into
        // the IR as i64 constants. Both are pinned for the process
        // lifetime (generic: leaked Box; cache slot: StaticArea).
        let generic = nod_runtime::get_or_create_generic(generic_name);
        let generic_ptr_raw: u64 = (generic as *const _) as u64;
        let cache_slot_ptr = nod_runtime::allocate_cache_slot(site_id);
        let cache_slot_raw: u64 = cache_slot_ptr as u64;

        // Field offsets within `CacheSlot` — must match the
        // `#[repr(C)]` layout in `nod_runtime::dispatch`.
        let cache_class_addr =
            i64ty.const_int(cache_slot_raw + offset_of_cache_slot_class() as u64, false);
        let cache_method_addr =
            i64ty.const_int(cache_slot_raw + offset_of_cache_slot_method() as u64, false);
        let cache_gen_addr =
            i64ty.const_int(cache_slot_raw + offset_of_cache_slot_generation() as u64, false);
        let cache_hits_addr =
            i64ty.const_int(cache_slot_raw + offset_of_cache_slot_hits() as u64, false);
        // misses bumped by nod_dispatch itself.

        // GenericFunction.generation is the second field; compute its
        // address relative to the generic pointer.
        let generic_gen_addr =
            i64ty.const_int(generic_ptr_raw + offset_of_generic_generation() as u64, false);

        let arity_const = i64ty.const_int(args.len() as u64, false);
        let generic_ptr_const = i64ty.const_int(generic_ptr_raw, false);
        let cache_slot_const = i64ty.const_int(cache_slot_raw, false);

        // Snapshot arg SSA values BEFORE the safepoint (still valid
        // because spill doesn't invalidate the original temp mapping —
        // it only forces the temp to live through the call).
        let arg_vals: Vec<inkwell::values::IntValue<'ctx>> = args
            .iter()
            .map(|t| self.temp_val(*t).into_int_value())
            .collect();
        let receiver = arg_vals[0];

        // ---- Compute r_class (i64) for the cache key. ----
        let r_class = self.emit_word_class_id(receiver)?;

        // ---- Load cache + generic generation (monotonic atomics). ----
        let cache_class_ptr = self
            .builder
            .build_int_to_ptr(cache_class_addr, ptr_ty, &format!("disp.s{site_id}.cache_class.ptr"))
            .map_err(map_err)?;
        let cache_class_load = self
            .builder
            .build_load(i64ty, cache_class_ptr, &format!("disp.s{site_id}.cache_class"))
            .map_err(map_err)?;
        let cache_class_inst = cache_class_load
            .as_instruction_value()
            .expect("load is an instruction");
        cache_class_inst
            .set_alignment(8)
            .map_err(|e| CodegenError::Builder(format!("set_alignment: {e}")))?;
        cache_class_inst
            .set_atomic_ordering(inkwell::AtomicOrdering::Monotonic)
            .map_err(|e| CodegenError::Builder(format!("atomic ordering: {e}")))?;

        let cache_method_ptr = self
            .builder
            .build_int_to_ptr(cache_method_addr, ptr_ty, &format!("disp.s{site_id}.cache_method.ptr"))
            .map_err(map_err)?;
        let cache_method_load = self
            .builder
            .build_load(i64ty, cache_method_ptr, &format!("disp.s{site_id}.cache_method"))
            .map_err(map_err)?;
        let cache_method_inst = cache_method_load
            .as_instruction_value()
            .expect("load is an instruction");
        cache_method_inst
            .set_alignment(8)
            .map_err(|e| CodegenError::Builder(format!("set_alignment: {e}")))?;
        cache_method_inst
            .set_atomic_ordering(inkwell::AtomicOrdering::Monotonic)
            .map_err(|e| CodegenError::Builder(format!("atomic ordering: {e}")))?;

        let cache_gen_ptr = self
            .builder
            .build_int_to_ptr(cache_gen_addr, ptr_ty, &format!("disp.s{site_id}.cache_gen.ptr"))
            .map_err(map_err)?;
        let cache_gen_load = self
            .builder
            .build_load(i64ty, cache_gen_ptr, &format!("disp.s{site_id}.cache_gen"))
            .map_err(map_err)?;
        let cache_gen_inst = cache_gen_load
            .as_instruction_value()
            .expect("load is an instruction");
        cache_gen_inst
            .set_alignment(8)
            .map_err(|e| CodegenError::Builder(format!("set_alignment: {e}")))?;
        cache_gen_inst
            .set_atomic_ordering(inkwell::AtomicOrdering::Monotonic)
            .map_err(|e| CodegenError::Builder(format!("atomic ordering: {e}")))?;

        let generic_gen_ptr = self
            .builder
            .build_int_to_ptr(generic_gen_addr, ptr_ty, &format!("disp.s{site_id}.gen.ptr"))
            .map_err(map_err)?;
        let generic_gen_load = self
            .builder
            .build_load(i64ty, generic_gen_ptr, &format!("disp.s{site_id}.gen"))
            .map_err(map_err)?;
        let generic_gen_inst = generic_gen_load
            .as_instruction_value()
            .expect("load is an instruction");
        generic_gen_inst
            .set_alignment(8)
            .map_err(|e| CodegenError::Builder(format!("set_alignment: {e}")))?;
        generic_gen_inst
            .set_atomic_ordering(inkwell::AtomicOrdering::Monotonic)
            .map_err(|e| CodegenError::Builder(format!("atomic ordering: {e}")))?;

        let cached_class = cache_class_load.into_int_value();
        let cached_method = cache_method_load.into_int_value();
        let cached_gen = cache_gen_load.into_int_value();
        let generic_gen = generic_gen_load.into_int_value();

        // ---- Cache-hit predicate. ----
        let class_ok = self
            .builder
            .build_int_compare(IntPredicate::EQ, r_class, cached_class, &format!("disp.s{site_id}.class_ok"))
            .map_err(map_err)?;
        let gen_ok = self
            .builder
            .build_int_compare(IntPredicate::EQ, generic_gen, cached_gen, &format!("disp.s{site_id}.gen_ok"))
            .map_err(map_err)?;
        let zero_i64 = i64ty.const_zero();
        let nonzero_class = self
            .builder
            .build_int_compare(IntPredicate::NE, cached_class, zero_i64, &format!("disp.s{site_id}.nonzero_class"))
            .map_err(map_err)?;
        let cg = self
            .builder
            .build_and(class_ok, gen_ok, &format!("disp.s{site_id}.class_and_gen"))
            .map_err(map_err)?;
        let cache_hit = self
            .builder
            .build_and(cg, nonzero_class, &format!("disp.s{site_id}.cache_hit"))
            .map_err(map_err)?;

        // ---- Begin safepoint for both branches. ----
        let rented = self.begin_safepoint(safepoint_roots)?;

        // Create fast/slow/done blocks. Append AFTER the current end
        // (don't disturb pre-created DFM blocks).
        let fast_bb = self
            .ctx
            .append_basic_block(self.llvm_fn, &format!("disp.s{site_id}.fast_call"));
        let slow_bb = self
            .ctx
            .append_basic_block(self.llvm_fn, &format!("disp.s{site_id}.slow_call"));
        let done_bb = self
            .ctx
            .append_basic_block(self.llvm_fn, &format!("disp.s{site_id}.dispatch_done"));

        self.builder
            .build_conditional_branch(cache_hit, fast_bb, slow_bb)
            .map_err(map_err)?;

        // ---- Fast path: bump hits, transmute cached_method, call. ----
        self.builder.position_at_end(fast_bb);
        let cache_hits_ptr = self
            .builder
            .build_int_to_ptr(cache_hits_addr, ptr_ty, &format!("disp.s{site_id}.hits.ptr"))
            .map_err(map_err)?;
        let one_i64 = i64ty.const_int(1, false);
        let _hits_rmw = self
            .builder
            .build_atomicrmw(
                inkwell::AtomicRMWBinOp::Add,
                cache_hits_ptr,
                one_i64,
                inkwell::AtomicOrdering::Monotonic,
            )
            .map_err(|e| CodegenError::Builder(format!("atomicrmw: {e}")))?;

        // Build function-type for the cached method call.
        let mut fn_param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::with_capacity(args.len());
        for _ in 0..args.len() {
            fn_param_tys.push(i64ty.into());
        }
        let cached_fn_ty: inkwell::types::FunctionType<'ctx> =
            i64ty.fn_type(&fn_param_tys, false);
        let cached_fn_ptr = self
            .builder
            .build_int_to_ptr(cached_method, ptr_ty, &format!("disp.s{site_id}.fast.fn"))
            .map_err(map_err)?;
        let fast_call_args: Vec<BasicMetadataValueEnum<'ctx>> =
            arg_vals.iter().map(|v| (*v).into()).collect();
        let fast_call_site = self
            .builder
            .build_indirect_call(
                cached_fn_ty,
                cached_fn_ptr,
                &fast_call_args,
                &format!("disp.s{site_id}.fast.call"),
            )
            .map_err(map_err)?;
        let fast_result = fast_call_site
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodegenError::Builder("dispatch fast-call returned void".into()))?;
        // Snapshot the current block (fast_bb) for the phi's incoming.
        let fast_pred = self
            .builder
            .get_insert_block()
            .expect("builder positioned");
        self.builder
            .build_unconditional_branch(done_bb)
            .map_err(map_err)?;

        // ---- Slow path: call nod_dispatch with the cache slot. ----
        self.builder.position_at_end(slow_bb);
        let disp_fn = match self.module.get_function(NOD_DISPATCH_SYMBOL) {
            Some(f) => f,
            None => {
                // (generic_ptr, cache_slot_ptr, arity, 8 * args) -> i64
                let mut params: Vec<BasicMetadataTypeEnum<'ctx>> = Vec::with_capacity(11);
                params.push(i64ty.into()); // generic_ptr
                params.push(i64ty.into()); // cache_slot_ptr
                params.push(i64ty.into()); // arity
                for _ in 0..8 {
                    params.push(i64ty.into());
                }
                let ty = i64ty.fn_type(&params, false);
                self.module.add_function(NOD_DISPATCH_SYMBOL, ty, None)
            }
        };
        let zero = i64ty.const_zero();
        let mut slow_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(11);
        slow_args.push(generic_ptr_const.into());
        slow_args.push(cache_slot_const.into());
        slow_args.push(arity_const.into());
        for i in 0..8 {
            if let Some(v) = arg_vals.get(i) {
                slow_args.push((*v).into());
            } else {
                slow_args.push(zero.into());
            }
        }
        let slow_call_site = self
            .builder
            .build_call(disp_fn, &slow_args, &format!("disp.s{site_id}.slow.call"))
            .map_err(map_err)?;
        let slow_result = slow_call_site
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodegenError::Builder("dispatch slow-call returned void".into()))?;
        let slow_pred = self
            .builder
            .get_insert_block()
            .expect("builder positioned");
        self.builder
            .build_unconditional_branch(done_bb)
            .map_err(map_err)?;

        // ---- Done block: phi the result + unregister roots. ----
        self.builder.position_at_end(done_bb);
        let phi = self
            .builder
            .build_phi(i64ty, &format!("disp.s{site_id}.result"))
            .map_err(map_err)?;
        phi.add_incoming(&[(&fast_result, fast_pred), (&slow_result, slow_pred)]);

        self.end_safepoint(&rented)?;
        let _ = dst;

        Ok(Some(phi.as_basic_value()))
    }

    fn emit_terminator(&mut self, t: &Terminator) -> Result<(), CodegenError> {
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        match t {
            Terminator::Return { value: None } => {
                self.builder.build_return(None).map_err(map_err)?;
            }
            Terminator::Return { value: Some(t) } => {
                let v = self.temp_val(*t);
                self.builder.build_return(Some(&v)).map_err(map_err)?;
            }
            Terminator::If { cond, then_block, else_block } => {
                // Sprint 10: every Dylan value except `#f` is true.
                // Compare against the pinned `#f` singleton address.
                let c64 = self.temp_val(*cond).into_int_value();
                let c1 = untag_bool_to_i1(self.builder, self.ctx.i64_type(), c64)?;
                let then_bb = self.blocks[then_block];
                let else_bb = self.blocks[else_block];
                self.builder
                    .build_conditional_branch(c1, then_bb, else_bb)
                    .map_err(map_err)?;
            }
            Terminator::Jump { target, args } => {
                let target_bb = self.blocks[target];
                // Snapshot the actual source block at this exact insert
                // point — phi nodes need that, not the logical DFM
                // BlockId (which would resolve to its starting LLVM BB
                // even after intermediate splits).
                let current = self
                    .builder
                    .get_insert_block()
                    .expect("builder positioned");
                self.builder
                    .build_unconditional_branch(target_bb)
                    .map_err(map_err)?;
                self.pending_incoming
                    .push((*target, current, args.clone()));
            }
        }
        Ok(())
    }

    fn temp_val(&self, t: TempId) -> BasicValueEnum<'ctx> {
        *self
            .temps
            .get(&t)
            .unwrap_or_else(|| panic!("undefined TempId({})", t.0))
    }

    /// Sprint 11b: emit the pre-call GC root bracketing — spill each
    /// live pointer-shaped temp into an entry-block-resident `alloca`
    /// slot and call `nod_register_root(slot)`. Returns the list of
    /// `(temp_id, slot_ptr)` pairs the matching `end_safepoint` will
    /// pop. Empty input → empty return → no IR emitted at all.
    fn begin_safepoint(
        &mut self,
        roots: &[TempId],
    ) -> Result<Vec<SafepointSlot<'ctx>>, CodegenError> {
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let register_fn = self.get_or_declare_register_root();
        let mut rented: Vec<SafepointSlot<'ctx>> = Vec::with_capacity(roots.len());
        for (i, t) in roots.iter().enumerate() {
            // Snapshot the current LLVM value for the temp, then drop a
            // slot in the entry block and spill it.
            let cur = self.temp_val(*t);
            let slot = self.rent_safepoint_slot(i)?;
            self.builder.build_store(slot, cur).map_err(map_err)?;
            self.builder
                .build_call(register_fn, &[slot.into()], "gc.reg")
                .map_err(map_err)?;
            rented.push(SafepointSlot { temp: *t, slot });
        }
        // Save current insert position for the caller — the caller
        // continues emitting the actual call into the same block.
        Ok(rented)
    }

    /// Sprint 11b: emit the post-call GC root cleanup — for each
    /// rented slot, call `nod_unregister_root(slot)`, reload the Word,
    /// and rewire the temp's mapping to the reloaded SSA value.
    fn end_safepoint(&mut self, rented: &[SafepointSlot<'ctx>]) -> Result<(), CodegenError> {
        if rented.is_empty() {
            return Ok(());
        }
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        let unregister_fn = self.get_or_declare_unregister_root();
        // Reverse order matches the "stack discipline" intent —
        // register A,B then unregister B,A — even though the runtime
        // tolerates any order. Determinism in IR shape simplifies
        // tests.
        for slot_info in rented.iter().rev() {
            self.builder
                .build_call(unregister_fn, &[slot_info.slot.into()], "gc.unreg")
                .map_err(map_err)?;
        }
        // Reload (in forward order) and rewire each temp's mapping.
        for slot_info in rented.iter() {
            let reloaded = self
                .builder
                .build_load(i64ty, slot_info.slot, &format!("gc.reload.t{}", slot_info.temp.0))
                .map_err(map_err)?;
            self.temps.insert(slot_info.temp, reloaded);
        }
        Ok(())
    }

    /// Return the i-th alloca slot from the function's safepoint pool,
    /// growing the pool if needed. Allocas are placed in the entry
    /// block (LLVM prefers entry-block allocas so the register
    /// allocator can scalarise / promote them on the fast path).
    fn rent_safepoint_slot(
        &mut self,
        idx: usize,
    ) -> Result<inkwell::values::PointerValue<'ctx>, CodegenError> {
        if idx < self.safepoint_slot_pool.len() {
            return Ok(self.safepoint_slot_pool[idx]);
        }
        let map_err = |e: inkwell::builder::BuilderError| CodegenError::Builder(e.to_string());
        let i64ty = self.ctx.i64_type();
        // Stash the current insert position; insert allocas at the
        // start of the entry block so LLVM treats them as standard
        // mem2reg-eligible storage.
        let saved = self.builder.get_insert_block();
        let entry_bb = self
            .llvm_fn
            .get_first_basic_block()
            .expect("function has at least one block");
        // Position before the first non-alloca instruction in entry.
        // For simplicity (and matching what the rest of the codebase
        // assumes), we position at the start of entry's instruction
        // list. Phi nodes appear at the very start of non-entry
        // blocks but never in the entry block — so this is safe.
        if let Some(first_inst) = entry_bb.get_first_instruction() {
            self.builder.position_before(&first_inst);
        } else {
            self.builder.position_at_end(entry_bb);
        }
        let slot_name = format!("gc.root.slot.{idx}");
        let slot = self.builder.build_alloca(i64ty, &slot_name).map_err(map_err)?;
        self.safepoint_slot_pool.push(slot);
        if let Some(bb) = saved {
            self.builder.position_at_end(bb);
        }
        Ok(slot)
    }

    fn get_or_declare_register_root(&self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function(NOD_REGISTER_ROOT_SYMBOL) {
            return f;
        }
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let ty = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        self.module.add_function(NOD_REGISTER_ROOT_SYMBOL, ty, None)
    }

    fn get_or_declare_unregister_root(&self) -> FunctionValue<'ctx> {
        if let Some(f) = self.module.get_function(NOD_UNREGISTER_ROOT_SYMBOL) {
            return f;
        }
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let ty = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        self.module.add_function(NOD_UNREGISTER_ROOT_SYMBOL, ty, None)
    }
}

/// One rented entry from the function's safepoint slot pool, used by
/// `begin_safepoint` / `end_safepoint`.
struct SafepointSlot<'ctx> {
    temp: TempId,
    slot: inkwell::values::PointerValue<'ctx>,
}

// ─── CacheSlot / GenericFunction field offsets ─────────────────────────────
//
// Sprint 13 bakes cache-slot field addresses into the IR as i64
// constants. The offsets here MUST agree with the `#[repr(C)]` layout
// of `nod_runtime::dispatch::CacheSlot` (six `AtomicU64`s, 8 bytes each
// at 8-byte alignment). Static asserts in the runtime crate's tests
// guard against accidental drift.

const fn offset_of_cache_slot_class() -> usize {
    0
}
const fn offset_of_cache_slot_method() -> usize {
    8
}
const fn offset_of_cache_slot_generation() -> usize {
    16
}
const fn offset_of_cache_slot_hits() -> usize {
    24
}

/// Offset of the `generation` AtomicU64 inside `GenericFunction`. The
/// struct layout (Rust default-repr — *not* repr(C), so we read this
/// at runtime through a helper) starts with `name: String` (24 bytes)
/// plus `methods: RwLock<Vec<Method>>` (sized by std), then the
/// `AtomicU64`. Because Rust's struct layout is not guaranteed across
/// versions, the runtime exposes `GenericFunction::generation()` which
/// codegen can't easily call inline; instead we read through this
/// constant offset and assert in the runtime tests that it matches.
///
/// Implementation note: we ALWAYS go through the runtime path for the
/// generation read (the slow path is `nod_dispatch`; the fast path
/// does its OWN read via baked offset). To keep this stable across
/// rustc versions we wrap the GenericFunction inside a `#[repr(C)]`
/// shim. See `nod_runtime::dispatch::generation_offset()`.
fn offset_of_generic_generation() -> usize {
    nod_runtime::generic_generation_offset()
}
