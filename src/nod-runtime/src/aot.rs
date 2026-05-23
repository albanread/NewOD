//! Sprint 39a — ahead-of-time (AOT) entry surface.
//!
//! When a Dylan program is linked as a standalone `.exe` (Sprint 39a's
//! goal, see `nod-driver`'s `build` subcommand), Rust's `cargo run`
//! lifecycle is no longer in the picture: the OS loader hands control to
//! `mainCRTStartup` which calls `int main(void)`. That `main` is emitted
//! by `nod-llvm` as a tiny LLVM-IR stub that does two things:
//!
//! 1. Call `nod_runtime_init()` (defined here) to eagerly run every
//!    initialisation the JIT path defers until first use — class
//!    registration, condition classes, the C-FFI error type, etc.
//! 2. Call the user's Dylan `main` (renamed to `nod_user_main` by the
//!    AOT post-processing pass in `nod-llvm::aot`) and propagate its
//!    `i64` return value as the process exit code.
//!
//! Both entry points are `extern "C-unwind"` so an uncaught Dylan
//! condition's panic-based NLX (Sprint 19) unwinds the stack normally
//! and Rust's default panic handler aborts the process with a
//! diagnostic — exactly the same observable behaviour as a panicking
//! Rust binary, which is the right default for a Dylan EXE that didn't
//! install its own top-level handler.
//!
//! ## Why this lives in `nod-runtime` (not `nod-driver`)
//!
//! The wrapper symbol (`nod_aot_main_wrapper`) must be reachable by
//! the linker when building the user's EXE. The linker pulls in
//! `nod_runtime.lib` (the Sprint 39a Phase A staticlib output), so
//! defining the wrapper here means the user's emitted `i32 @main()`
//! stub finds it via a normal static-library link.
//!
//! ## Idempotency
//!
//! `nod_runtime_init()` may be called more than once (e.g. a host
//! embedding the staticlib who isn't sure whether a previous Dylan EXE
//! linked into the same process already ran it). Every `ensure_*`
//! helper it calls is already idempotent — they use `OnceLock`,
//! `LazyLock`, or a `_REGISTERED` static — so double-calling here is
//! safe. The first call pays the cost; subsequent calls are O(1).
//!
//! ## Why no `catch_unwind`
//!
//! Sprint 19's `block`/`exception`/`cleanup` is implemented on top of
//! Rust's `panic_unwind` machinery: an unhandled `signal()` panics up
//! to the nearest `nod_run_block` frame. If `nod_aot_main_wrapper`
//! wrapped `nod_user_main()` in `catch_unwind`, an uncaught condition
//! would be swallowed and the EXE would exit with a misleading status
//! code. The right semantics — and the same as the JIT path — is to
//! let the panic propagate out of `main`, where the standard Rust
//! panic handler logs the message and aborts with exit code 101.

// ─── Sprint 39a — relocation resolvers ────────────────────────────────────
//
// The JIT path resolves Sprint 38's `RelocKind` entries via
// `LLVMAddGlobalMapping`: each named external global is bound to a
// current-process slot address at MCJIT-finalise time. The AOT path
// can't do that — the codegen-emitted `.obj` ships with strong storage
// for each global, and we populate that storage at startup via these
// C-ABI helpers. `nod-llvm::aot::emit_aot_entry_stubs` rewrites the
// IR to emit defining `i64 0` storage per entry, and adds a synthesised
// `nod_aot_resolve_relocs` LLVM function that calls one of these
// helpers per entry before `nod_user_main` runs.
//
// Each helper:
//   1. Computes the same per-process slot value the JIT path would
//      resolve via `resolve_reloc_kind`.
//   2. Loads that value (a `u64`).
//   3. Stores it into the user's `slot` storage.
//
// The user's IR then does `load i64, ptr @<sym>` against that storage
// and observes the same bits the JIT path would observe.

/// Sprint 39a — copy the runtime's `#t` Word bits into `slot`.
///
/// # Safety
/// `slot` must point at a writable, naturally-aligned `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_set_imm_true(slot: *mut u64) {
    // SAFETY: per caller.
    unsafe { *slot = *crate::imm_true_slot_addr() };
}

/// Sprint 39a — copy the runtime's `#f` Word bits into `slot`.
///
/// # Safety
/// `slot` must point at a writable, naturally-aligned `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_set_imm_false(slot: *mut u64) {
    // SAFETY: per caller.
    unsafe { *slot = *crate::imm_false_slot_addr() };
}

/// Sprint 39a — copy the runtime's `nil` Word bits into `slot`.
///
/// # Safety
/// `slot` must point at a writable, naturally-aligned `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_set_imm_nil(slot: *mut u64) {
    // SAFETY: per caller.
    unsafe { *slot = *crate::imm_nil_slot_addr() };
}

/// Sprint 39a — copy the runtime's `#f` untagged-wrapper bits into
/// `slot`. Used by codegen's branchless class-id read fallback.
///
/// # Safety
/// `slot` must point at a writable, naturally-aligned `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_set_imm_false_wrapper(slot: *mut u64) {
    // SAFETY: per caller.
    unsafe { *slot = *crate::imm_false_wrapper_slot_addr() };
}

/// Sprint 39a — copy the metadata pointer for `class_id` into `slot`.
///
/// # Safety
/// `slot` must point at a writable, naturally-aligned `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_set_class_md(slot: *mut u64, class_id: u32) {
    let id = crate::ClassId(class_id);
    // SAFETY: per caller.
    unsafe { *slot = *crate::class_metadata_slot_addr(id) };
}

/// Sprint 39a — intern `text` as a `<byte-string>` literal in the
/// runtime's literal pool, then store its tagged-Word bits into `slot`.
///
/// # Safety
/// `slot` must point at a writable, naturally-aligned `u64`. `text` +
/// `len` must describe a valid UTF-8 byte slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_set_strlit(slot: *mut u64, text: *const u8, len: usize) {
    // SAFETY: caller asserts the byte slice is valid UTF-8.
    let s = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(text, len)) };
    // SAFETY: per caller.
    unsafe { *slot = *crate::intern_string_literal_slot_addr(s) };
}

/// Sprint 39a — intern `name` as a `<symbol>` literal and store its
/// tagged-Word bits into `slot`.
///
/// # Safety
/// `slot` must point at a writable, naturally-aligned `u64`. `name` +
/// `len` must describe a valid UTF-8 byte slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_set_symlit(slot: *mut u64, name: *const u8, len: usize) {
    // SAFETY: caller asserts the byte slice is valid UTF-8.
    let s = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name, len)) };
    // SAFETY: per caller.
    unsafe { *slot = *crate::intern_symbol_literal_slot_addr(s) };
}

/// Sprint 39a — allocate (or look up) a cache slot keyed on
/// `(key_prefix, site_id)` and store its address into `slot`.
///
/// # Safety
///
/// `slot` must point at a writable, naturally-aligned `u64`.
/// `key_prefix` + `key_prefix_len` must describe a valid UTF-8 byte
/// slice (the 16-char hex prefix codegen embedded in symbol names).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_set_cache_slot(
    slot: *mut u64,
    key_prefix: *const u8,
    key_prefix_len: usize,
    site_id: u64,
) {
    // SAFETY: caller asserts the byte slice is valid UTF-8.
    let kp = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(key_prefix, key_prefix_len))
    };
    let v: &'static u64 = crate::cache_slot_slot_addr(kp, site_id);
    // SAFETY: per caller.
    unsafe { *slot = *v };
}

/// Sprint 39a — allocate (or look up) the `<generic>` function for
/// `name` and store its address into `slot`.
///
/// # Safety
/// `slot` must point at a writable, naturally-aligned `u64`. `name` +
/// `name_len` must describe a valid UTF-8 byte slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_set_generic(
    slot: *mut u64,
    name: *const u8,
    name_len: usize,
) {
    // SAFETY: caller asserts the byte slice is valid UTF-8.
    let s = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name, name_len)) };
    let v: &'static u64 = crate::generic_function_slot_addr(s);
    // SAFETY: per caller.
    unsafe { *slot = *v };
}

// ─── Sprint 39c — startup registration helpers ───────────────────────────────
//
// In the JIT path, the post-codegen glue in `nod-sema` resolves
// every method body / block thunk / top-level function to its JIT'd
// address and calls `add_method_named` / `register_block_fns` /
// `register_jit_function` immediately. The AOT path can't do that
// at compile time — the LLVM functions don't have addresses until
// the linker emits them into the EXE. Instead the codegen-emitted
// `nod_aot_resolve_relocs` calls these helpers from inside the EXE
// once per merged-stdlib (and user-defined) method / block / function;
// the helpers run inside the new process so they see the same
// process-global dispatch tables `nod_runtime_init` just populated.

/// Sprint 39c — register a Dylan method body with the global dispatch
/// table. Called from the codegen-emitted resolver per method in the
/// merged `LoweredModule`.
///
/// Arguments (all `(ptr, len)` for strings, raw fn ptr for the body,
/// raw `ClassId` array for the specialisers — kept as flat C-ABI
/// inputs because LLVM IR can pass each one as a `BasicMetadataValueEnum`
/// without needing to materialise a Rust struct):
///
/// - `generic_name_ptr`, `generic_name_len` — UTF-8 generic name.
/// - `specialisers_ptr`, `n_specialisers` — array of `u32` class IDs
///   (matching `ClassId(u32)`'s repr).
/// - `body_fn_ptr` — address of the method body's LLVM function
///   (linker-resolved at EXE-load time).
/// - `param_count` — Dylan-source arity (not the JIT body arity; the
///   dispatcher cares about the user-facing argument count).
/// - `body_fn_name_ptr`, `body_fn_name_len` — UTF-8 symbol name; the
///   dispatcher uses this for the Sprint 16 `DirectCall` path that
///   doesn't go through `{generic}${specialisers}` mangling.
///
/// # Safety
///
/// All `(ptr, len)` pairs must describe valid UTF-8 byte slices.
/// `specialisers_ptr` + `n_specialisers` must describe a contiguous
/// `[u32]` array. `body_fn_ptr` must be a valid function pointer of
/// signature `extern "C-unwind" fn(u64, …, u64) -> u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_register_method(
    generic_name_ptr: *const u8,
    generic_name_len: usize,
    specialisers_ptr: *const u32,
    n_specialisers: usize,
    body_fn_ptr: *const u8,
    param_count: usize,
    body_fn_name_ptr: *const u8,
    body_fn_name_len: usize,
) {
    // SAFETY: caller asserts the byte slices are valid UTF-8.
    let generic_name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(
            generic_name_ptr,
            generic_name_len,
        ))
    };
    let body_fn_name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(
            body_fn_name_ptr,
            body_fn_name_len,
        ))
    };
    // SAFETY: caller asserts the array is `[u32; n_specialisers]`.
    let raw_ids: &[u32] =
        unsafe { std::slice::from_raw_parts(specialisers_ptr, n_specialisers) };
    let specialisers: Vec<crate::ClassId> = raw_ids.iter().copied().map(crate::ClassId).collect();
    // SAFETY: body_fn_ptr is link-time-resolved; the JIT-style dispatcher
    // treats it as `*const u8` regardless of the underlying signature.
    unsafe {
        crate::add_method_named(
            generic_name,
            specialisers,
            body_fn_ptr,
            param_count,
            body_fn_name,
        );
    }
}

/// Sprint 39c — register a top-level Dylan function in the function-ref
/// registry so `\name` resolves to its body address.
///
/// # Safety
///
/// `name_ptr` + `name_len` must describe a valid UTF-8 byte slice.
/// `code_ptr` must be a valid function pointer of the signature
/// codegen emitted for `name` (the dispatcher's `nod_funcall_N`
/// trampolines interpret it via the registered arity).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_register_jit_function(
    name_ptr: *const u8,
    name_len: usize,
    arity: usize,
    code_ptr: *const u8,
) {
    // SAFETY: caller asserts the byte slice is valid UTF-8.
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len))
    };
    // SAFETY: code_ptr is link-time-resolved.
    unsafe {
        crate::register_jit_function(name, arity, code_ptr);
    }
}

/// Sprint 39c — a single `exception` clause's runtime metadata.
/// Mirrors [`crate::HandlerFn`]'s `#[repr(C)]` layout exactly so the
/// codegen-emitted handler array (a static `[HandlerFn; N]` in the
/// EXE's data section) can be passed by `(ptr, len)` and walked
/// directly without intermediate copies.
///
/// `class_name_ptr` / `class_name_len` reference a static UTF-8
/// byte slice the codegen emitted for the handler's specialiser
/// class name (used by the runtime for diagnostic dumps).
#[repr(C)]
pub struct AotHandlerEntry {
    pub class_id: u32,
    /// Padding to align the 8-byte fields below; the struct must
    /// match `HandlerFn`'s natural alignment.
    pub _pad: u32,
    pub class_name_ptr: *const u8,
    pub class_name_len: usize,
    pub body: *const u8,
}

// ─── Sprint 40a — user-defined class registration ─────────────────────────────
//
// Sprint 39c shipped registration for methods / top-level functions /
// blocks. Sprint 40a closes the last gap: user-defined `define class`
// in AOT user code. The JIT path calls `register_simple_user_class` /
// `register_mi_user_class` inline as `register_class` runs during
// lowering; the AOT path replays the same registrations inside the
// EXE's startup resolver so the freshly-allocated `ClassId`s match
// the values the compiler baked into IR.
//
// ## Layout-correctness
//
// Slot offsets are computed by sema during lowering (Sprint 12). The
// compiler-side persistence layer (`UserClassRegistration` in
// `nod-sema/src/lower.rs`) snapshots the canonical offsets that the
// JIT path's `register_user_class_metadata` pinned in the static area.
// The AOT shim re-uses those offsets verbatim via
// `register_user_class_metadata`, which trusts its `UserClassSpec`'s
// `slots` field rather than re-computing.
//
// ## Class-id determinism
//
// `allocate_user_class_id()` returns monotonic `ClassId(FIRST_USER + N)`
// in the order it's called. The EXE's `nod_aot_resolve_relocs` calls
// `nod_aot_register_user_class` once per merged-LM entry in the same
// order the compiler called `register_class`. With both processes
// starting from the same seeded `next_user_id` (since stdlib carries
// no `define class` today), the resulting IDs match. The shim asserts
// this — a panic here would be a codegen bug, never a user error.

/// Sprint 40a — encoding for [`crate::SlotType`] across the AOT C-ABI
/// boundary. Keep in lockstep with `nod-sema::encode_slot_type` (the
/// sender). The codegen layer emits an `i8` per slot; this module
/// decodes back into the runtime enum.
const AOT_SLOT_TYPE_INTEGER: u8 = 0;
const AOT_SLOT_TYPE_DOUBLE_FLOAT: u8 = 1;
const AOT_SLOT_TYPE_BOOLEAN: u8 = 2;
const AOT_SLOT_TYPE_CHARACTER: u8 = 3;
const AOT_SLOT_TYPE_STRING: u8 = 4;
const AOT_SLOT_TYPE_SYMBOL: u8 = 5;
const AOT_SLOT_TYPE_VECTOR: u8 = 6;
const AOT_SLOT_TYPE_OBJECT: u8 = 7;
const AOT_SLOT_TYPE_CLASS: u8 = 8;
/// Kept for documentation + decoder symmetry — the decoder's
/// catch-all arm uses this value implicitly via the `_` pattern, but
/// keeping the constant named makes the sender side (`nod-sema`'s
/// `encode_slot_type`) easier to read.
#[allow(dead_code)]
const AOT_SLOT_TYPE_TOP: u8 = 9;

/// Sprint 40a — one slot's worth of metadata, laid out for the
/// codegen-emitted resolver. `#[repr(C)]` matches what
/// `nod-llvm::aot::emit_user_class_registrations` bakes as a constant
/// `[AotSlotEntry; N]` array.
///
/// Strings (`name`, `init_keyword`) are `(ptr, len)` pairs pointing at
/// private LLVM globals in the EXE's read-only data section. A null
/// `init_keyword_ptr` (with `init_keyword_len == 0`) means "no init
/// keyword". The padding fields keep the struct size aligned with what
/// LLVM emits for the `struct_type` declared in `emit_user_class_registrations`.
#[repr(C)]
pub struct AotSlotEntry {
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub offset: usize,
    /// One of `AOT_SLOT_TYPE_*` above.
    pub type_tag: u8,
    /// Bools as `u8` (0/1) — easier for codegen than packing into a
    /// bit field.
    pub required_init_keyword: u8,
    /// `SlotDefault` encoding: 0 = Unbound, 1 = Value(default_value).
    pub default_init_tag: u8,
    pub has_setter: u8,
    /// 4-byte hole so `type_class_id` lands at a 4-byte boundary
    /// without LLVM tail-padding shenanigans.
    pub _pad: u32,
    /// Payload for `SlotType::Class(_)`; zero otherwise.
    pub type_class_id: u32,
    /// Padding so the next pointer (`init_keyword_ptr`) is 8-byte
    /// aligned regardless of struct base address.
    pub _pad2: u32,
    /// Raw `Word` bits for `SlotDefault::Value`; zero for `Unbound`.
    pub default_init_value: u64,
    pub init_keyword_ptr: *const u8,
    pub init_keyword_len: usize,
}

fn decode_slot_type(tag: u8, class_id: u32) -> crate::SlotType {
    use crate::SlotType;
    match tag {
        AOT_SLOT_TYPE_INTEGER => SlotType::Integer,
        AOT_SLOT_TYPE_DOUBLE_FLOAT => SlotType::DoubleFloat,
        AOT_SLOT_TYPE_BOOLEAN => SlotType::Boolean,
        AOT_SLOT_TYPE_CHARACTER => SlotType::Character,
        AOT_SLOT_TYPE_STRING => SlotType::String,
        AOT_SLOT_TYPE_SYMBOL => SlotType::Symbol,
        AOT_SLOT_TYPE_VECTOR => SlotType::Vector,
        AOT_SLOT_TYPE_OBJECT => SlotType::Object,
        AOT_SLOT_TYPE_CLASS => SlotType::Class(crate::ClassId(class_id)),
        // AOT_SLOT_TYPE_TOP and any out-of-range tag fall through to
        // Top, the safe over-conservative choice for the GC scanner.
        _ => SlotType::Top,
    }
}

/// Sprint 40a — register a user-defined Dylan class with the runtime.
/// Called from the codegen-emitted resolver once per
/// `LoweredModule::user_classes` entry, in the order the compiler
/// registered them, so class-id allocation in the EXE process mirrors
/// what the compiler observed.
///
/// Arguments mirror the C-ABI shapes the codegen layer can pass
/// directly (`(ptr, len)` byte slices, raw u32 arrays, a
/// `#[repr(C)]` slot-entry array).
///
/// # Safety
///
/// - `name_ptr` + `name_len` must describe a valid UTF-8 byte slice.
/// - `parents_ptr` + `n_parents` must describe a contiguous `[u32]`.
/// - `cpl_ptr` + `n_cpl` must describe a contiguous `[u32]` whose
///   first element equals `expected_class_id` (compiler-side self id).
/// - `slot_origin_ptr` + `n_slot_origin` must describe a contiguous
///   `[u32]` of length `n_slots`.
/// - `slots_ptr` + `n_slots` must describe a contiguous
///   `[AotSlotEntry]` array; each entry's string pointers (`name_ptr`,
///   `init_keyword_ptr`) must point at valid UTF-8 byte slices (or be
///   null for `init_keyword_ptr` when `init_keyword_len == 0`).
///
/// # Panics
///
/// Panics if `expected_class_id` differs from the freshly-allocated
/// `ClassId` returned by `register_user_class_metadata`. A mismatch
/// indicates the AOT registration order drifted from the compile-time
/// order — a hard codegen bug. Failing fast at startup beats silent
/// dispatch failure later.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_register_user_class(
    name_ptr: *const u8,
    name_len: usize,
    expected_class_id: u32,
    parents_ptr: *const u32,
    n_parents: usize,
    cpl_ptr: *const u32,
    n_cpl: usize,
    slots_ptr: *const AotSlotEntry,
    n_slots: usize,
    slot_origin_ptr: *const u32,
    n_slot_origin: usize,
    own_slot_count: usize,
    inherited_slot_count: usize,
) {
    // SAFETY: caller asserts the byte slice is valid UTF-8.
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len))
    };
    // SAFETY: caller asserts the arrays are valid for their lengths.
    // The codegen layer passes `null` for empty arrays (e.g. a class
    // with no own slots gets a null `slots_ptr` with `n_slots == 0`);
    // `slice::from_raw_parts` requires a non-null + aligned pointer
    // even at length zero, so we route empty arrays through an
    // explicit empty slice to avoid UB on the null-pointer path.
    let parents_raw: &[u32] = if n_parents == 0 || parents_ptr.is_null() {
        &[]
    } else {
        // SAFETY: caller guarantees the array is valid for `n_parents`.
        unsafe { std::slice::from_raw_parts(parents_ptr, n_parents) }
    };
    let cpl_raw: &[u32] = if n_cpl == 0 || cpl_ptr.is_null() {
        &[]
    } else {
        // SAFETY: caller guarantees the array is valid for `n_cpl`.
        unsafe { std::slice::from_raw_parts(cpl_ptr, n_cpl) }
    };
    let slot_origin_raw: &[u32] = if n_slot_origin == 0 || slot_origin_ptr.is_null() {
        &[]
    } else {
        // SAFETY: caller guarantees the array is valid for `n_slot_origin`.
        unsafe { std::slice::from_raw_parts(slot_origin_ptr, n_slot_origin) }
    };
    let slots_raw: &[AotSlotEntry] = if n_slots == 0 || slots_ptr.is_null() {
        &[]
    } else {
        // SAFETY: caller guarantees the array is valid for `n_slots`.
        unsafe { std::slice::from_raw_parts(slots_ptr, n_slots) }
    };

    let parents: Vec<crate::ClassId> =
        parents_raw.iter().copied().map(crate::ClassId).collect();
    let cpl: Vec<crate::ClassId> = cpl_raw.iter().copied().map(crate::ClassId).collect();
    let slot_origin: Vec<crate::ClassId> =
        slot_origin_raw.iter().copied().map(crate::ClassId).collect();
    let slots: Vec<crate::SlotInfo> = slots_raw
        .iter()
        .map(|e| {
            // SAFETY: caller asserts each entry's strings are valid UTF-8.
            let slot_name = unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(e.name_ptr, e.name_len))
            };
            let init_keyword = if e.init_keyword_len == 0 || e.init_keyword_ptr.is_null() {
                None
            } else {
                // SAFETY: caller asserts valid UTF-8.
                let s = unsafe {
                    std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                        e.init_keyword_ptr,
                        e.init_keyword_len,
                    ))
                };
                Some(s.to_string())
            };
            let default_init = match e.default_init_tag {
                1 => crate::SlotDefault::Value(crate::Word::from_raw(e.default_init_value)),
                _ => crate::SlotDefault::Unbound,
            };
            crate::SlotInfo {
                name: slot_name.to_string(),
                offset: e.offset,
                type_kind: decode_slot_type(e.type_tag, e.type_class_id),
                init_keyword,
                required_init_keyword: e.required_init_keyword != 0,
                default_init,
                has_setter: e.has_setter != 0,
            }
        })
        .collect();

    // Build the UserClassSpec. The CPL's first entry IS the compiler's
    // expected class id — `register_user_class_metadata` doesn't rewrite
    // the cpl[0] sentinel because we provided the real id. The
    // sema-side persistence already substituted the real id into cpl[0].
    let parent = parents.first().copied();
    let spec = crate::UserClassSpec {
        name: name.to_string(),
        parent,
        parents,
        cpl,
        slots,
        slot_origin,
        own_slot_count,
        inherited_slot_count,
    };
    let (assigned_id, _md_ptr) = crate::register_user_class_metadata(spec);
    assert_eq!(
        assigned_id.0, expected_class_id,
        "nod_aot_register_user_class: class id drift — compiler expected \
         {expected_class_id} but runtime allocated {} for class `{name}`. \
         This indicates the AOT registration sequence diverged from the \
         compile-time sequence; the codegen path is buggy.",
        assigned_id.0
    );
}

/// Sprint 39c — register a `block` form's lifted thunks with the
/// global block registry. Mirrors `register_block_fns` but accepts
/// raw inputs the codegen-emitted resolver can pass without
/// constructing a `BlockFns` struct.
///
/// `cleanup_ptr` and `afterwards_ptr` are `null` when the source
/// block omitted the corresponding clause. `handlers_ptr` +
/// `n_handlers` describe a static array of [`AotHandlerEntry`] —
/// codegen emits the array in the EXE's data section and the
/// runtime keeps the references alive for the process lifetime.
///
/// # Safety
///
/// `body_ptr` must be a valid function pointer; `cleanup_ptr` /
/// `afterwards_ptr` are either null or valid function pointers;
/// `handlers_ptr` + `n_handlers` must describe a valid array;
/// each handler's `class_name_ptr` / `class_name_len` must be a
/// valid UTF-8 byte slice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nod_aot_register_block(
    block_id: u64,
    body_ptr: *const u8,
    cleanup_ptr: *const u8,
    afterwards_ptr: *const u8,
    handlers_ptr: *const AotHandlerEntry,
    n_handlers: usize,
) {
    let cleanup = if cleanup_ptr.is_null() {
        None
    } else {
        Some(cleanup_ptr)
    };
    let afterwards = if afterwards_ptr.is_null() {
        None
    } else {
        Some(afterwards_ptr)
    };
    // Build a static `[HandlerFn]` from the codegen-emitted
    // `[AotHandlerEntry]` array. The handler entries themselves
    // live in the EXE's read-only data section so taking references
    // is sound; we materialise a fresh `Vec<HandlerFn>` and leak it
    // so the slice's lifetime is `'static` (matching what
    // `register_block_fns` expects from the JIT path).
    let handlers: Vec<crate::HandlerFn> = if n_handlers == 0 {
        Vec::new()
    } else {
        // SAFETY: caller asserts the array is `[AotHandlerEntry; n_handlers]`.
        let slice = unsafe { std::slice::from_raw_parts(handlers_ptr, n_handlers) };
        slice
            .iter()
            .map(|h| crate::HandlerFn {
                class_id: crate::ClassId(h.class_id),
                class_name_ptr: h.class_name_ptr,
                class_name_len: h.class_name_len,
                body: h.body,
            })
            .collect()
    };
    let handlers_static: &'static [crate::HandlerFn] =
        Box::leak(handlers.into_boxed_slice());
    crate::register_block_fns(
        block_id,
        crate::BlockFns {
            body: body_ptr,
            cleanup,
            afterwards,
            handlers: handlers_static,
        },
    );
}

/// Sprint 39a — eagerly perform every initialisation the JIT path defers
/// until first use. Called from the codegen-emitted `i32 @main()` stub
/// (via [`nod_aot_main_wrapper`]) before the user's Dylan body runs.
///
/// Idempotent. Each `ensure_*_registered` helper is independently
/// idempotent (backed by `OnceLock` / `LazyLock`); the outer
/// `LazyLock<()>` guard collapses repeated calls to a single atomic
/// load on the steady state.
///
/// # Why eager
///
/// In the JIT path each subsystem registers its classes lazily on first
/// Dylan use, threaded through `nod-sema` lowering. In the AOT path the
/// codegen-emitted `@main` enters the user's body directly — no
/// lowering happens at run time, so the lazy hooks never fire. Calling
/// every `ensure_*` here forces the same final state the JIT would
/// reach after touching every subsystem.
///
/// # Stability
///
/// The set of subsystems below mirrors what `nod-sema`'s lowering pass
/// touches when it sees `define class`, `define condition`, `define
/// c-function`, etc. If a future sprint adds a new subsystem with its
/// own `ensure_*_registered`, this list must grow to match — otherwise
/// the AOT path will diverge from the JIT path for programs using the
/// new feature. The Sprint 39a invariant is "every JIT-discoverable
/// runtime feature is eagerly registered by `nod_runtime_init`".
#[unsafe(no_mangle)]
pub extern "C-unwind" fn nod_runtime_init() {
    use std::sync::LazyLock;

    static INIT: LazyLock<()> = LazyLock::new(|| {
        // ORDER MATTERS — class IDs are content-deterministic only if
        // the registration sequence matches the codegen-time order. The
        // codegen process loads `stdlib.dylan` through `nod-sema`'s
        // `stdlib::load_stdlib`, which calls (in order):
        //   1. `ensure_conditions_registered`
        //   2. `ensure_collections_registered`
        //   3. `ensure_tables_registered`
        //   4. `ensure_structs_registered`
        // Then `lower_module_full` (called from inside the stdlib loader
        // for stdlib.dylan, and once more for the user's module) calls:
        //   5. `ensure_functions_registered`
        //   6. `ensure_closures_registered`
        //   7. `ensure_c_types_registered`
        // And `define c-function` lowering calls:
        //   8. `ensure_c_ffi_error_registered` (lazy — only if user code
        //       declares a c-function, which Sprint 39a forbids anyway)
        //
        // Float types, structs (extended), COM, operator shims, and
        // floats register additional seed classes; we replicate that
        // pre-Sprint-39c-stdlib-pre-compile order here so seed class IDs
        // align with what codegen baked into the manifest.
        //
        // Any drift from the codegen-time order produces silent
        // `ClassId` mismatches — `make(<range>, …)` resolves the wrong
        // class metadata, dispatch on `<range>` fails. This was Sprint
        // 39a's `aot_dispatch` red gate during initial bringup.
        crate::conditions::ensure_registered();
        crate::collections::ensure_registered();
        crate::tables::ensure_registered();
        crate::structs::ensure_structs_registered();
        crate::functions::ensure_registered();
        crate::closures::ensure_registered();
        crate::c_types::ensure_registered();
        // Float-type seeds + c-ffi-error are downstream of the above —
        // their IDs only matter if user code touches them, which Sprint
        // 39a's hello-world doesn't but `aot_arithmetic` /
        // `aot_dispatch` might via the `<float>` / `<c-ffi-error>`
        // baked into stdlib lowering paths.
        crate::c_types::ensure_float_types_registered();
        crate::winffi::ensure_c_ffi_error_registered();
        // Sprint 35 — COM-shim seed classes register AFTER c-ffi-error
        // because `<c-handle>` (a COM-shim seed) extends `<c-pointer>`
        // which is in `c_types`.
        #[cfg(windows)]
        crate::com_shim::ensure_com_types_registered();
        // Sprint 21 — operator shim *functions* (`+`, `*`, `<`, …) are
        // a registry of `<function>` instances, not classes. Order
        // doesn't affect ClassId allocation; run last.
        crate::functions::ensure_operator_shims_registered();
        // Touch the literal-pool singleton so `#t`/`#f`/`nil` Words
        // exist before the resolver populates the immediate slots.
        // SAFETY: `nod_nil` is `extern "C" fn() -> u64`, infallible.
        let _ = unsafe { crate::nod_nil() };
    });

    LazyLock::force(&INIT);
}

unsafe extern "C-unwind" {
    /// Sprint 39a — the user's Dylan top-level `main`, renamed by
    /// `nod-llvm::aot::emit_aot_entry_stubs` from the Dylan-source name
    /// (`main`) to a namespaced symbol the AOT-emitted `i32 @main()`
    /// stub can call without name-collision against the C `main`.
    ///
    /// Signature: `() -> i64`. The Dylan return value (`#t`, `#f`,
    /// `nil`, fixnum, or any tagged Word) is cast to `i32` and returned
    /// as the process exit code. Most Dylan `main` functions return
    /// `#f` (the unit-like value) which is a non-zero Word — but
    /// codegen emits a stub that **discards** the user's return value
    /// and returns 0 unconditionally, so the exit code is always
    /// success unless the user's `main` panics out.
    ///
    /// The actual link resolution happens at EXE-link time: `link.exe`
    /// pulls in the user's `.obj` (which defines `nod_user_main`) and
    /// `nod_runtime.lib` (which references it as `extern`).
    fn nod_user_main() -> i64;
}

/// Sprint 39a — the actual entry-point body invoked from the
/// codegen-emitted `i32 @main()` LLVM stub.
///
/// Runs eager initialisation, then calls the user's renamed Dylan
/// `main`. The return value is the process exit code; we return `0`
/// unconditionally because the Dylan `main` body's natural return is
/// a Word (e.g. `#f`'s bit pattern), which is meaningless as a Unix-
/// style exit code. A future sprint can extend the Dylan ABI to let
/// `main` declare an integer return type and surface it here; Sprint
/// 39a's `hello.dylan` returns `#f` and exits 0, which matches the
/// brief.
///
/// # Why no `catch_unwind`
///
/// Sprint 19's NLX panics out of unhandled `signal()` calls. Catching
/// the panic here would swallow the diagnostic — better to let Rust's
/// default panic handler abort the process with the usual `thread
/// 'main' panicked at ...` message. See the module-level doc for the
/// long-form rationale.
#[unsafe(no_mangle)]
pub extern "C-unwind" fn nod_aot_main_wrapper() -> i32 {
    nod_runtime_init();
    // SAFETY: `nod_user_main` is link-time-resolved from the user's
    // `.obj` produced by `nod-llvm::aot::emit_object_file`. The AOT
    // post-processing pass guarantees a symbol of that exact name + the
    // `() -> i64` signature is present in the linked object.
    let _rc = unsafe { nod_user_main() };
    // The Dylan return value is a tagged Word, not a Unix exit code.
    // Sprint 39a returns 0 on a normal (non-panic) exit; an uncaught
    // Dylan condition panics out of `nod_user_main` before this line.
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Double-call must be a no-op. The `LazyLock` guard collapses
    /// repeat calls to an atomic load; the individual `ensure_*`
    /// helpers each have their own idempotency story (covered by their
    /// own tests in their respective modules). This test just verifies
    /// the outer dispatch.
    #[test]
    fn nod_runtime_init_is_idempotent() {
        nod_runtime_init();
        nod_runtime_init();
        nod_runtime_init();
        // No panic, no double-registration — and the class table
        // observes every seed class. We probe one as a smoke check.
        assert!(
            crate::classes::find_class_id_by_name("<c-ffi-error>").is_some(),
            "expected <c-ffi-error> to be registered after nod_runtime_init"
        );
    }

    /// `nod_aot_main_wrapper` calls `nod_runtime_init` then
    /// `nod_user_main`; in tests `nod_user_main` is the stub above
    /// returning 0. End-to-end: wrapper returns 0.
    #[test]
    fn nod_aot_main_wrapper_returns_zero_via_stub() {
        let rc = nod_aot_main_wrapper();
        assert_eq!(rc, 0);
    }
}
