//! `nod-runtime` — tagged-pointer ABI, `<wrapper>` headers, generational
//! copying heap, class metadata table, write barrier. The Sprint 09
//! foundation grew into the Sprint 11 GC.
//!
//! Sprint 11 lights up:
//!   - **Generational copying GC** (semispace young + 2-semispace
//!     old). Structural lift from NCL's `ncl-runtime/src/heap.rs`,
//!     adapted for Dylan's one-bit tag + Wrapper-with-ClassId.
//!   - **Class-driven scanning** via `ClassMetadata::scan` /
//!     `::size_of` function pointers. Both the tracer and the
//!     collector go through this single data-driven path.
//!   - **Card-marking write barrier** (software, 512-byte cards).
//!     `write_barrier(dst, src)` is the canonical store path for
//!     Rust-side mutations of heap-resident Words.
//!   - **Conservative stack scanning** via `Heap::pin_stack_range`.
//!     Sprint 11b will upgrade to precise stack roots via
//!     `gc.statepoint`.
//!   - **Literal pool moved to static area.** Sprint 10's
//!     `intern_string_literal` / `intern_symbol_literal` now allocate
//!     in pinned storage so JIT-baked addresses survive every GC.
//!
//! Sprint 11 design choice (per the brief's option-(b) allowance):
//! **synchronous GC triggered only at Rust-side allocation sites**, no
//! JIT-side safepoint polls. Threading the JIT into the parking
//! protocol is Sprint 11b.

// Sprint 23: feature-gated GC backend. Exactly one must be enabled.
#[cfg(all(feature = "newgc-backend", feature = "semispace-backend"))]
compile_error!(
    "nod-runtime: features `newgc-backend` and `semispace-backend` are mutually exclusive. \
     The default activates `newgc-backend`; pass `--no-default-features --features semispace-backend` \
     to use the legacy escape-hatch semispace heap."
);
#[cfg(not(any(feature = "newgc-backend", feature = "semispace-backend")))]
compile_error!(
    "nod-runtime: one of `newgc-backend` (default) or `semispace-backend` must be enabled."
);

mod c_types;
mod classes;
mod closures;
mod collections;
mod conditions;
mod dispatch;
mod winffi;
#[cfg(feature = "newgc-backend")]
mod dylan_layout;
mod format_out;
mod functions;
mod heap;
mod heap_common;
mod immediates;
mod lists;
mod make;
mod roots;
mod stack_map;
mod static_area;
mod strings;
mod symbols;
mod tables;
mod tracer;
mod vectors;
mod word;
mod wrapper;

pub use classes::{
    ClassId, ClassMetadata, ClassTable, LayoutFn, ScanFn, SizeFn, SlotDefault, SlotInfo,
    SlotType, _reset_user_classes_for_tests, allocate_user_class_id, class_metadata_for,
    class_metadata_ptr, find_class_id_by_name, for_each_class, is_subclass,
    register_user_class, user_class_layout_fn, user_class_scan_fn, user_class_size_fn,
};
pub use closures::{
    cell_class_id, ensure_registered as ensure_closures_registered, environment_class_id,
    is_cell, is_environment, make_cell, make_environment, nod_cell_get, nod_cell_set,
    nod_env_cell, nod_make_cell, nod_make_environment,
};
pub use collections::{
    FipKind, IterStateSnapshot, OutOfRange, collection_class_id, collection_concatenate,
    collection_do, collection_element, collection_element_setter, collection_map,
    collection_reduce, collection_size, ensure_registered as ensure_collections_registered,
    explicit_key_collection_class_id, forward_iteration_protocol,
    forward_iteration_protocol_init, is_collection, iter_state_advance, iter_state_snapshot,
    iteration_state_class_id, make_out_of_range_error, make_range, make_stretchy_vector,
    mutable_collection_class_id, mutable_sequence_class_id,
    // Sprint 20b primitive-op shims (called from JIT-emitted DirectCalls
    // against `%`-prefixed names; see `nod-sema/src/lower.rs` LOWER_PRIMITIVES).
    nod_collection_concatenate, nod_collection_size, nod_fip_advance,
    nod_fip_current_element, nod_fip_finished_p, nod_fip_init, nod_make_range,
    nod_make_stretchy_vector, nod_range_by, nod_range_from, nod_range_to,
    nod_stretchy_vector_element, nod_stretchy_vector_element_setter,
    nod_stretchy_vector_push, nod_stretchy_vector_size, out_of_range_error_class_id,
    range_class_id, range_fields, sequence_class_id, stretchy_collection_class_id,
    stretchy_vector_class_id, stretchy_vector_fields, stretchy_vector_push,
};
pub use c_types::{
    c_bool_class_id, c_dword_class_id, c_handle_class_id, c_int_class_id, c_pointer_class_id,
    c_string_class_id, c_wide_string_class_id,
    ensure_registered as ensure_c_types_registered,
};
pub use conditions::{
    BlockFns, HandlerFn, HandlerFrame, MAX_BLOCK_CAPTURED, NlxPayload, _reset_block_registry_for_tests,
    _reset_handler_stack_for_tests, allocate_block_id, condition_class_id, condition_class_name,
    condition_message, error_class_id, ensure_registered as ensure_conditions_registered,
    exit_procedure_block_id, exit_procedure_class_id, for_each_handler, handler_stack_snapshot,
    handlers_report, invoke_restart, make_exit_procedure, make_no_applicable_methods_error,
    make_simple_condition, make_simple_error, make_simple_restart, make_simple_warning,
    no_applicable_methods_error_class_id, no_next_method_error_class_id, nod_condition_message,
    nod_invoke_exit, nod_make_exit_procedure, nod_pop_handler, nod_push_handler, nod_run_block,
    nod_signal, nod_walk_handlers_dump, register_block_fns, serious_condition_class_id,
    simple_condition_class_id, simple_error_class_id, simple_restart_class_id,
    simple_warning_class_id, warning_class_id,
};
pub use dispatch::{
    CacheSlot, GenericFunction, Method, MethodPtr, MethodTableError, ResolvedDispatchEntry,
    _reset_for_tests as _reset_dispatch_for_tests,
    _reset_method_chain_stack_for_tests, add_method, add_method_full, add_method_named,
    dump_dispatch, find_generic, find_initialize_method, find_method_body_ptr,
    for_each_generic, generic_generation_offset, get_or_create_generic, has_next_method,
    invoke_method_with_self, is_generic_defined, lookup_applicable_methods, lookup_method,
    lookup_method_by_receiver, nod_add_method, nod_dispatch, nod_dispatch_binary,
    nod_dispatch_unary, nod_has_next_method, nod_next_method, nod_pop_sealed_chain_frame,
    nod_push_sealed_chain_frame, record_resolved_dispatch, remove_method,
    resolved_dispatch_snapshot, try_add_method_full, word_class_id,
};
pub use make::{
    MAKE_MAX_KW_PAIRS, RootGuard, nod_card_mark, nod_is_instance_of, nod_is_instance_of_word,
    nod_make, nod_register_root, nod_unregister_root, rust_make,
};
pub use format_out::{
    install_test_writer, nod_format_out, take_test_writer, uninstall_test_writer,
};
pub use functions::{
    FUNCTION_KIND_CLOSURE, FUNCTION_KIND_GENERIC_TRAMPOLINE, FUNCTION_KIND_LIFTED_ANON,
    FUNCTION_KIND_TOP_LEVEL, MAX_APPLY_ARITY, _reset_function_registry_for_tests,
    ensure_operator_shims_registered, ensure_registered as ensure_functions_registered,
    function_arity, function_class_id, function_code_ptr, function_env_ptr, function_kind_tag,
    function_name, is_function, lookup_function_code, make_function, make_function_ref,
    make_generic_trampoline_ref, make_wrong_number_of_arguments_error, nod_apply, nod_funcall0,
    nod_funcall1, nod_funcall2, nod_funcall3, nod_funcall4, nod_funcall5, nod_make_closure,
    nod_make_function_ref, nod_op_eq, nod_op_gt, nod_op_lt, nod_op_minus, nod_op_plus,
    nod_op_times, register_jit_function, register_rust_function,
    wrong_number_of_arguments_error_class_id,
};
pub use heap::{
    DEFAULT_OLD_BYTES, DEFAULT_RESERVATION_BYTES, DEFAULT_YOUNG_BYTES, GcConfig, HEAP_ALIGN, Heap,
    HeapRanges, for_each_root, register_root as heap_register_root, root_count as heap_root_count,
    unregister_root as heap_unregister_root,
};
pub use immediates::{Immediates, WrapperCell, wrapper_of_unchecked};
pub use lists::{
    PAIR_HEAD_OFFSET, PAIR_TAIL_OFFSET, Pair, nod_empty_p, nod_list_size, nod_nil,
    nod_pair_alloc, nod_pair_head, nod_pair_tail, try_pair,
};
pub use roots::RootSet;
pub use stack_map::{LiveSlot, ParkedFrame, StackMap, StackMapEntry, walk_parked_frame};
pub use static_area::StaticArea;
pub use strings::{ByteString, try_byte_string};
pub use symbols::{Symbol, SymbolTable, try_symbol};
pub use tables::{
    ensure_registered as ensure_tables_registered, is_table, make_not_hashable_error, make_table,
    nod_make_table, nod_object_equal_p, nod_object_hash, nod_table_element,
    nod_table_element_or_default, nod_table_element_setter, nod_table_keys, nod_table_remove_key,
    nod_table_size, nod_table_values, not_hashable_error_class_id, table_class_id, table_element,
    table_element_setter, table_keys, table_remove_key, table_size, table_values,
};
pub use tracer::{HeapObjectInfo, HeapTrace, trace_heap};
pub use vectors::{
    SimpleObjectVector, nod_make_sov_len, nod_make_sov_literal, nod_sov_element,
    nod_sov_element_setter, nod_sov_size, try_simple_object_vector, try_simple_object_vector_mut,
};
pub use winffi::{
    ApiCallSignature, ApiStubEntry, ApiStubTable, CArgKind, CReturnKind, StubEntrySpec,
    WinFfiStats, _reset_winffi_stats_for_tests, allocate_stub_table, c_ffi_error_class_id,
    ensure_c_ffi_error_registered, initialize_stub_table, make_c_ffi_error,
    nod_winffi_call_0, nod_winffi_call_1, nod_winffi_call_2, nod_winffi_call_3,
    nod_winffi_call_4, nod_winffi_call_5, nod_winffi_call_6, nod_winffi_call_7,
    nod_winffi_call_8, record_stub_entry_allocated, resolve_into_entry, resolve_symbol,
    signature_from_names, winffi_record_materialized, winffi_stats,
};
pub use word::{FIXNUM_MAX, FIXNUM_MIN, FixnumOverflow, Word};
pub use wrapper::{GcBit, Wrapper};

use std::sync::{LazyLock, Mutex};

/// Process-global literal pool. Sprint 11 pins string + symbol literals
/// in the `StaticArea` so JIT-baked addresses (the `i64` constants
/// codegen emits) survive every GC cycle. Booleans and `nil` live in
/// the same static area for the same reason.
///
/// The pool also exposes a moveable `Heap` — that's the process-global
/// young generation the Sprint 11 collector mutates. JIT'd code
/// allocates there through the same `nod-sema` shim path it used in
/// Sprint 10.
pub struct LiteralPool {
    pub heap: Heap,
    pub symbols: SymbolTable,
    pub static_area: StaticArea,
    pub classes: ClassTable,
    pub immediates: Immediates,
}

static LITERAL_POOL: LazyLock<Mutex<LiteralPool>> = LazyLock::new(|| {
    let heap = Heap::new();
    let symbols = SymbolTable::new();
    let static_area = StaticArea::new();
    let classes = ClassTable::new();
    let immediates = Immediates::new(&static_area, &classes);
    let pool = LiteralPool {
        heap,
        symbols,
        static_area,
        classes,
        immediates,
    };
    // Sprint 19: register the seed condition classes once the seed
    // class table is alive. `ensure_registered` is idempotent and
    // routes through `register_simple_user_class` (which itself takes
    // the literal-pool mutex), so we have to schedule it AFTER this
    // initialiser returns. We do that by deferring to first-use of
    // any condition accessor — the first `signal()` or `make
    // <error>` from Dylan code triggers `ensure_registered` lazily.
    // Tests that want the classes present unconditionally call
    // `ensure_conditions_registered` from `nod-sema` lowering.
    let _ = (); // doc comment anchor
    Mutex::new(pool)
});

/// Take a brief lock on the process-global literal pool.
pub fn with_literal_pool<R>(f: impl FnOnce(&LiteralPool) -> R) -> R {
    let guard = LITERAL_POOL.lock().expect("literal pool poisoned");
    f(&guard)
}

/// Intern a Dylan string literal in the process-global literal pool
/// and return its tagged `Word`. Sprint 11: allocation goes through
/// the **static area**, not the moveable heap, so the returned
/// address is stable across every GC cycle. Codegen bakes these
/// addresses into LLVM constants.
pub fn intern_string_literal(s: &str) -> Word {
    with_literal_pool(|pool| pool.static_area.alloc_byte_string(s, &pool.classes))
}

/// Intern a Dylan symbol literal in the process-global literal pool
/// and return its tagged `Word`. Sprint 11: allocation goes through
/// the **static area**. Repeated calls with the same `name` return
/// the same Word (the symbol table dedups across heap + static).
pub fn intern_symbol_literal(name: &str) -> Word {
    with_literal_pool(|pool| {
        pool.symbols
            .intern_static(name, &pool.static_area, &pool.classes)
    })
}

/// The process-global boolean / nil singletons. Codegen bakes these
/// addresses into LLVM constants so `#t`, `#f`, and `nil` round-trip
/// through the JIT as stable pointer-tagged words.
pub fn literal_pool_immediates() -> Immediates {
    with_literal_pool(|pool| pool.immediates)
}

/// Sprint 13: mint a fresh inline-cache slot in the static area and
/// return its raw pointer. Each JIT-emitted `Dispatch` call site
/// receives one via `dispatch::CacheSlot::cold(site_id)` baked into
/// the IR as an `i64`. The slot's address is stable for the process
/// lifetime; the slot's contents are atomically read/written by both
/// the JIT-emitted fast path and the slow-path shim.
pub fn allocate_cache_slot(site_id: u64) -> *const CacheSlot {
    with_literal_pool(|pool| {
        let slot: &'static CacheSlot = pool.static_area.alloc(CacheSlot::cold(site_id));
        slot as *const CacheSlot
    })
}

/// Description of a user class to be registered. Sprint 12 expects
/// callers (the sema layer) to compute slot offsets + CPL up-front and
/// hand them over; the runtime just pins the metadata in the static
/// area. Returns the stable `ClassId` and the static-area address of
/// the new `ClassMetadata`, so the codegen layer can bake the address
/// into LLVM constants.
///
/// Sprint 14 adds `parents` (multiple direct supers) and `slot_origin`
/// (the defining class per slot). The legacy `parent` field is the
/// first parent (`parents[0]` or `None` for `<object>`).
pub struct UserClassSpec {
    pub name: String,
    pub parent: Option<ClassId>,
    pub parents: Vec<ClassId>,
    pub cpl: Vec<ClassId>,
    pub slots: Vec<SlotInfo>,
    pub slot_origin: Vec<ClassId>,
    pub own_slot_count: usize,
    pub inherited_slot_count: usize,
}

/// Pin a fresh `ClassMetadata` for a user class in the static area and
/// register it in the global class table. Returns the assigned
/// `ClassId` and the address of the pinned metadata.
///
/// Both addresses are stable for the process lifetime — the codegen
/// layer can bake them into LLVM `i64` constants.
pub fn register_user_class_metadata(spec: UserClassSpec) -> (ClassId, *const ClassMetadata) {
    let id = allocate_user_class_id();
    let instance_size = std::mem::size_of::<Wrapper>() + 8 * spec.slots.len();
    let md = ClassMetadata {
        id,
        name: spec.name,
        parent: spec.parent,
        parents: spec.parents,
        cpl: spec.cpl,
        slots: spec.slots,
        own_slot_count: spec.own_slot_count,
        inherited_slot_count: spec.inherited_slot_count,
        slot_origin: spec.slot_origin,
        instance_size,
        scan: user_class_scan_fn(),
        size_of: user_class_size_fn(),
        layout: user_class_layout_fn(),
        is_byte_payload: false,
        // Sprint 15: every class starts open; the lowering pass flips
        // `sealed = true` post-registration when the source carries the
        // `sealed` modifier. The atomic store there pairs with reads on
        // the dispatch resolver path.
        sealed: std::sync::atomic::AtomicBool::new(false),
        direct_subclasses: std::sync::RwLock::new(Vec::new()),
    };
    let static_ref: &'static ClassMetadata =
        with_literal_pool(|pool| pool.static_area.alloc(md));
    // SAFETY: static_ref lives in the static area (process-lived).
    unsafe { register_user_class(static_ref) };
    (id, static_ref as *const ClassMetadata)
}

/// Builder-style helper: register a single-inheritance user class given
/// its name, parent, and own slots. The slot offsets are computed
/// automatically (own slots appended after the parent's). Sprint 14:
/// for multi-parent classes, use `register_mi_user_class` which takes
/// the merged slot list directly.
///
/// Sprint 21: a `parent = None` arg is reinterpreted as `parent =
/// Some(<object>)` so the CPL chain reaches `<object>` and
/// `is_subclass(c, <object>)` holds for every user-registered class.
/// This restores the Dylan semantics that every class is implicitly a
/// subclass of `<object>` — required for stdlib methods declared as
/// `(p :: <object>)` to dispatch on user-class instances.
pub fn register_simple_user_class(
    name: &str,
    parent: Option<ClassId>,
    own_slots: Vec<SlotInfo>,
) -> (ClassId, *const ClassMetadata) {
    let parent = parent.or(Some(ClassId::OBJECT));
    let parents: Vec<ClassId> = parent.into_iter().collect();
    register_mi_user_class_simple(name, parent, &parents, own_slots)
}

/// SI fast path used internally — same shape as the Sprint 12 helper.
fn register_mi_user_class_simple(
    name: &str,
    parent: Option<ClassId>,
    parents: &[ClassId],
    own_slots: Vec<SlotInfo>,
) -> (ClassId, *const ClassMetadata) {
    // Inherit parent's slot list, then append our own at the next
    // offset. For SI this matches the Sprint 12 behaviour.
    let (inherited, inherited_origin): (Vec<SlotInfo>, Vec<ClassId>) = match parent {
        Some(p) => {
            let pmd = class_metadata_for(p);
            (pmd.slots.clone(), pmd.slot_origin.clone())
        }
        None => (Vec::new(), Vec::new()),
    };
    let inherited_slot_count = inherited.len();
    let mut all_slots = inherited;
    let mut slot_origin = inherited_origin;
    // Placeholder for "self id" — patched after registration when we know
    // the freshly minted ClassId. Until then, use a sentinel; the post-
    // registration step rewrites both `cpl[0]` and any `slot_origin[i]
    // == sentinel` entries.
    let self_sentinel = ClassId(u32::MAX);
    for (i, mut slot) in own_slots.into_iter().enumerate() {
        let slot_idx = inherited_slot_count + i;
        slot.offset = std::mem::size_of::<Wrapper>() + slot_idx * 8;
        all_slots.push(slot);
        slot_origin.push(self_sentinel);
    }
    let own_slot_count = all_slots.len() - inherited_slot_count;
    // CPL: [self, parent.cpl...]
    let mut cpl = vec![ClassId(0)]; // placeholder for self, filled below
    if let Some(p) = parent {
        let pmd = class_metadata_for(p);
        cpl.extend(pmd.cpl.iter().copied());
    }
    let spec = UserClassSpec {
        name: name.to_string(),
        parent,
        parents: parents.to_vec(),
        cpl,
        slots: all_slots,
        slot_origin,
        own_slot_count,
        inherited_slot_count,
    };
    let (id, md_ptr) = register_user_class_metadata(spec);
    // Patch the CPL's first entry + any `slot_origin == sentinel` entries
    // to point to the freshly minted id.
    // SAFETY: md_ptr points at the just-registered metadata in the
    // static area. We hold exclusive access (registration is the only
    // writer; no GC can touch this metadata).
    unsafe {
        let md_mut = md_ptr as *mut ClassMetadata;
        (&mut (*md_mut).cpl)[0] = id;
        for origin in (*md_mut).slot_origin.iter_mut() {
            if *origin == self_sentinel {
                *origin = id;
            }
        }
    }
    (id, md_ptr)
}

/// Sprint 14: register a user class with explicit MI shape — caller
/// supplies the C3-computed CPL, the merged slot list (one entry per
/// slot in layout order, offsets already patched), the per-slot
/// `slot_origin` vector, and the count split (own vs inherited).
///
/// Used by `nod-sema::lower` for MI classes, which run C3 and the
/// merge-slots pass themselves so the runtime stays algorithm-free.
pub fn register_mi_user_class(
    name: &str,
    parents: Vec<ClassId>,
    cpl: Vec<ClassId>,
    slots: Vec<SlotInfo>,
    slot_origin: Vec<ClassId>,
    own_slot_count: usize,
    inherited_slot_count: usize,
) -> (ClassId, *const ClassMetadata) {
    let parent = parents.first().copied();
    // The supplied CPL must begin with a `ClassId(0)` placeholder for
    // self at index 0 — we patch it after the id is minted. This mirrors
    // the SI helper above so both paths share the post-patch step.
    let spec = UserClassSpec {
        name: name.to_string(),
        parent,
        parents,
        cpl,
        slots,
        slot_origin,
        own_slot_count,
        inherited_slot_count,
    };
    let (id, md_ptr) = register_user_class_metadata(spec);
    // SAFETY: md_ptr points at the just-registered metadata in the
    // static area. We hold exclusive access (registration is the only
    // writer; no GC can touch this metadata).
    unsafe {
        let md_mut = md_ptr as *mut ClassMetadata;
        if let Some(slot0) = (*md_mut).cpl.first_mut()
            && (slot0.0 == 0 || *slot0 == ClassId(u32::MAX))
        {
            *slot0 = id;
        }
        // Patch any `slot_origin` sentinels (for own slots whose origin
        // is "self" — caller can use `ClassId(u32::MAX)` as the sentinel
        // or just hand back `id` directly).
        let self_sentinel = ClassId(u32::MAX);
        for origin in (*md_mut).slot_origin.iter_mut() {
            if *origin == self_sentinel {
                *origin = id;
            }
        }
    }
    (id, md_ptr)
}

/// Atomically store `src` into `*dst_ptr` and mark the corresponding
/// card. The canonical Rust-side write path for storing a Word into a
/// heap-resident slot. Use this anywhere the runtime mutates a Word
/// slot inside an old-generation object — including vector slot writes,
/// symbol intern-table updates, and any future class slot setter.
///
/// JIT-emitted code stores directly (no barrier) until Sprint 12 wires
/// `Computation::WriteBarrier` into the codegen path.
///
/// # Safety
///
/// `dst_ptr` must point at a valid, writable `Word` slot. If the slot
/// is inside the moveable heap (old.live), the write is recorded in
/// the card table; if it isn't, the card mark is a no-op. The caller
/// must not race other writers on the same slot.
pub unsafe fn write_barrier(dst_ptr: *mut Word, src: Word) {
    // Mark first, then store. The reverse ordering would create a brief
    // window in which the new pointer is visible without the card being
    // dirty — fine for synchronous GC (Sprint 11) but the right
    // discipline now for when concurrent GC arrives.
    with_literal_pool(|pool| pool.heap.mark_card_for(dst_ptr));
    // SAFETY: per caller's contract.
    unsafe { *dst_ptr = src };
}

/// Public-facing snapshot of GC counters. Returned by `gc_stats()`.
#[derive(Copy, Clone, Debug, Default)]
pub struct GcStats {
    pub minor_collections: u64,
    pub major_collections: u64,
    pub young_bytes_allocated: u64,
    pub young_bytes_live: u64,
    pub old_bytes_live: u64,
    pub last_minor_pause_ns: u64,
    pub last_major_pause_ns: u64,
    pub last_pinned_objects: u64,
    pub heap_backend: &'static str,
}

/// Snapshot the process-global heap's GC stats.
pub fn gc_stats() -> GcStats {
    with_literal_pool(|pool| {
        let s = pool.heap.stats_snapshot();
        GcStats {
            minor_collections: s.minor_collections,
            major_collections: s.major_collections,
            young_bytes_allocated: s.young_bytes_allocated,
            young_bytes_live: s.young_bytes_live,
            old_bytes_live: s.old_bytes_live,
            last_minor_pause_ns: s.last_minor_pause_ns,
            last_major_pause_ns: s.last_major_pause_ns,
            last_pinned_objects: s.last_pinned_objects,
            heap_backend: HEAP_BACKEND_NAME,
        }
    })
}

/// Backend-name string surfaced by `gc_stats().heap_backend`. Sprint 23:
/// `"page-mark-evacuate"` under the default `newgc-backend` feature;
/// `"semispace"` under the `--no-default-features --features
/// semispace-backend` escape hatch.
#[cfg(feature = "newgc-backend")]
const HEAP_BACKEND_NAME: &str = "page-mark-evacuate";
#[cfg(feature = "semispace-backend")]
const HEAP_BACKEND_NAME: &str = "semispace";

/// Trigger a minor GC of the process-global heap. Used by `:gc-stats`,
/// stress tests, and `--gc-trace` callers.
pub fn collect_minor() {
    with_literal_pool(|pool| pool.heap.collect_minor());
}

/// Trigger a full GC of the process-global heap.
pub fn collect_full() {
    with_literal_pool(|pool| pool.heap.collect_full());
}

/// Multi-line text rendering of `gc_stats()` for `:gc-stats` /
/// `--gc-trace`. Stable shape; suitable for assertion in tests.
pub fn gc_stats_report() -> String {
    let s = gc_stats();
    format!(
        "GC stats (backend = {})\n  \
         minor collections : {}\n  \
         major collections : {}\n  \
         young allocated   : {} bytes\n  \
         young live        : {} bytes\n  \
         old live          : {} bytes\n  \
         last minor pause  : {} ns\n  \
         last major pause  : {} ns\n  \
         last pinned objs  : {}\n",
        s.heap_backend,
        s.minor_collections,
        s.major_collections,
        s.young_bytes_allocated,
        s.young_bytes_live,
        s.old_bytes_live,
        s.last_minor_pause_ns,
        s.last_major_pause_ns,
        s.last_pinned_objects,
    )
}

// -- Tracing flag ------------------------------------------------------------
//
// Set by `--gc-trace` (driver-side). When true, GC entry/exit and pause
// times are logged to stderr. Sprint 11 exposes the toggle; the driver
// wires `--gc-trace` in a follow-up commit.

use std::sync::atomic::{AtomicBool, Ordering};

static GC_TRACE_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_gc_trace(enabled: bool) {
    GC_TRACE_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn gc_trace_enabled() -> bool {
    GC_TRACE_ENABLED.load(Ordering::Relaxed)
}
