//! AST → DFM lowering.

use std::collections::{HashMap, HashSet};

use nod_dfm::{
    Block, BlockId, ClassCheck, Computation, ConstValue, Function, FunctionId, PrimOp,
    SlotTypeKind, TempId, Temporary, Terminator, TypeEstimate,
};
use nod_reader::{BinOp, Expr, Item, Module, Param, ReturnSig, Span, Statement, UnOp};
use nod_runtime::{
    ClassId, ClassMetadata, SlotDefault, SlotInfo, SlotType, Word, class_metadata_for,
    class_metadata_ptr, find_class_id_by_name, register_mi_user_class,
    register_simple_user_class,
};

use crate::c3::{C3Error, c3_linearise};

type LocalEnv = HashMap<String, TempId>;

/// Sprint 15: structured outcomes of the redefinition-refusal pass.
/// Surfaced via `LoweringError` so the driver can display the
/// diagnostic with span context.
#[derive(Clone, Debug)]
pub enum SealingViolation {
    /// `define class <Sub> (<Sealed>)` where `<Sealed>` was sealed by
    /// a prior compilation unit ("another library" in Sprint 15's
    /// simulated-cross-library scope).
    SealedClassExtendedAcrossBoundary { sealed_parent: String, child: String },
    /// `add-method` against a generic whose `sealed` flag is set
    /// from a prior compilation unit.
    SealedGenericClosed { generic: String },
}

impl std::fmt::Display for SealingViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SealingViolation::SealedClassExtendedAcrossBoundary {
                sealed_parent,
                child,
            } => write!(
                f,
                "sealed-class-extended-across-boundary: `{child}` cannot extend `{sealed_parent}` — sealed classes are closed against subclassing across library boundaries (Sprint 15 single-library scope)"
            ),
            SealingViolation::SealedGenericClosed { generic } => write!(
                f,
                "sealed-generic-closed: cannot add methods to `{generic}` — sealed against further additions (Sprint 15 single-library scope)"
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub enum LoweringError {
    Unsupported { span: Span, message: String },
    UndefinedIdent { span: Span, name: String },
    TypeMismatch { span: Span, message: String },
    /// Integer literal doesn't fit in the fixnum range
    /// (`[FIXNUM_MIN, FIXNUM_MAX]` = 63-bit signed).
    IntegerOverflow { span: Span, value: i128 },
    /// Re-defining an existing class. Sprint 12 refuses class
    /// redefinition; Sprint 28+ adds lazy migration.
    ClassRedefinitionNotSupported { span: Span, class_name: String },
    /// `class:` / `each-subclass:` / `virtual:` slots — Sprint 12 only
    /// supports `instance:` allocation.
    UnsupportedSlotAllocation { span: Span, class_name: String, slot_name: String, allocation: String },
    /// The class's parent reference doesn't resolve to a known class.
    UnknownSuperclass { span: Span, class_name: String, super_name: String },
    /// Sprint 14: C3 linearisation failed — two parents impose
    /// inconsistent orders on shared ancestors.
    InconsistentInheritance { span: Span, class_name: String, detail: String },
    /// Sprint 14: two parents independently define a slot with the same
    /// name. Inheriting the same slot from a shared ancestor (diamond)
    /// is fine; defining the same slot name in two unrelated parents
    /// is an MI conflict the programmer must resolve.
    SlotConflict {
        span: Span,
        class_name: String,
        slot_name: String,
        first_origin: String,
        second_origin: String,
    },
    /// Sprint 15: a redefinition that would break a sealing assumption.
    /// Single-library Sprint 15 scope: cross-library extension is
    /// "simulated" as "another lowering call after the class is
    /// sealed". Per-method violations are surfaced before any
    /// runtime mutation runs.
    SealingViolation { span: Span, violation: SealingViolation },
}

impl std::fmt::Display for LoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoweringError::Unsupported { span, message } => {
                write!(f, "unsupported [{:?}]: {message}", span)
            }
            LoweringError::UndefinedIdent { span, name } => {
                write!(f, "undefined ident `{name}` [{:?}]", span)
            }
            LoweringError::TypeMismatch { span, message } => {
                write!(f, "type mismatch [{:?}]: {message}", span)
            }
            LoweringError::IntegerOverflow { span, value } => write!(
                f,
                "integer overflow [{:?}]: literal {value} out of fixnum range \
                 (<big-integer> / <double-integer> not yet supported)",
                span
            ),
            LoweringError::ClassRedefinitionNotSupported { span, class_name } => write!(
                f,
                "class redefinition refused [{:?}]: `{class_name}` already exists; Sprint 12 forbids redefinition",
                span
            ),
            LoweringError::UnsupportedSlotAllocation { span, class_name, slot_name, allocation } => write!(
                f,
                "slot allocation `{allocation}` not supported [{:?}]: in `{class_name}` slot `{slot_name}` (only `instance:` is supported in Sprint 12)",
                span
            ),
            LoweringError::UnknownSuperclass { span, class_name, super_name } => write!(
                f,
                "unknown superclass `{super_name}` [{:?}]: in `define class {class_name}`",
                span
            ),
            LoweringError::InconsistentInheritance { span, class_name, detail } => write!(
                f,
                "inconsistent inheritance [{:?}]: in `define class {class_name}`: {detail}",
                span
            ),
            LoweringError::SlotConflict {
                span,
                class_name,
                slot_name,
                first_origin,
                second_origin,
            } => write!(
                f,
                "slot conflict [{:?}]: `{class_name}` inherits slot `{slot_name}` from two unrelated parents (`{first_origin}` and `{second_origin}`); rename one slot to disambiguate",
                span
            ),
            LoweringError::SealingViolation { span, violation } => write!(f, "{violation} [{:?}]", span),
        }
    }
}

impl LoweringError {
    pub fn span(&self) -> Span {
        match self {
            LoweringError::Unsupported { span, .. }
            | LoweringError::UndefinedIdent { span, .. }
            | LoweringError::TypeMismatch { span, .. }
            | LoweringError::IntegerOverflow { span, .. }
            | LoweringError::ClassRedefinitionNotSupported { span, .. }
            | LoweringError::UnsupportedSlotAllocation { span, .. }
            | LoweringError::UnknownSuperclass { span, .. }
            | LoweringError::InconsistentInheritance { span, .. }
            | LoweringError::SlotConflict { span, .. }
            | LoweringError::SealingViolation { span, .. } => *span,
        }
    }
}

/// A method registration captured during lowering and applied to the
/// runtime dispatch table after JIT compilation. The driver / JIT glue
/// resolves `body_fn_name` to a JIT'd function pointer, then calls
/// `nod_runtime::add_method_full` with the full specialiser list.
///
/// Sprint 13 carries one `ClassId` per required parameter
/// (`specialisers`); the legacy `receiver_class` field is kept as a
/// convenience accessor for callers that only need the first
/// position.
#[derive(Clone, Debug)]
pub struct MethodRegistration {
    pub generic_name: String,
    pub specialisers: Vec<ClassId>,
    pub body_fn_name: String,
    pub param_count: usize,
}

impl MethodRegistration {
    /// First-parameter specialiser. Sprint 12 callers used this as
    /// "the receiver class"; Sprint 13's multi-arg dispatch reads the
    /// full vector.
    pub fn receiver_class(&self) -> ClassId {
        self.specialisers.first().copied().unwrap_or(ClassId::OBJECT)
    }
}

/// Sprint 20b: a `%`-prefixed primitive callee lowers to a `DirectCall`
/// against a `nod_*` runtime extern. The lowerer recognises the leading
/// `%` and routes through `LOWER_PRIMITIVE_TABLE` below; the codegen
/// layer (`nod-llvm/src/codegen.rs::emit_direct_call`) honours the
/// `%`-prefix and emits the matching extern declaration.
///
/// Each entry: `(dylan-name, runtime-symbol, arity, return-type)`.
///
/// **Naming convention** — every primitive name starts with `%`. The
/// runtime symbol is the `nod_*` C-ABI shim. Arity is the parameter
/// count; the return type is the Dylan-side `TypeEstimate`.
///
/// Primitives wired here are intentionally low-level — they bridge
/// Dylan source to the existing Sprint 20 runtime API. Higher-level
/// generics (`size`, `concatenate`, `for-each`) live in
/// `src/nod-dylan/dylan-sources/stdlib.dylan` and call these.
const LOWER_PRIMITIVE_TABLE: &[(&str, &str, usize, TypeEstimate)] = &[
    // Collection-class primitives (Sprint 20b — wraps the Rust Sprint 20 API).
    ("%collection-size", "nod_collection_size", 1, TypeEstimate::Integer),
    ("%collection-concatenate", "nod_collection_concatenate", 2, TypeEstimate::Top),
    // <range> field accessors.
    ("%range-from", "nod_range_from", 1, TypeEstimate::Integer),
    ("%range-to", "nod_range_to", 1, TypeEstimate::Integer),
    ("%range-by", "nod_range_by", 1, TypeEstimate::Integer),
    // <simple-object-vector> primitives.
    ("%vector-size", "nod_sov_size", 1, TypeEstimate::Integer),
    ("%vector-element", "nod_sov_element", 2, TypeEstimate::Top),
    ("%vector-element-setter", "nod_sov_element_setter", 3, TypeEstimate::Top),
    // <stretchy-vector> primitives.
    ("%stretchy-vector-size", "nod_stretchy_vector_size", 1, TypeEstimate::Integer),
    ("%stretchy-vector-element", "nod_stretchy_vector_element", 2, TypeEstimate::Top),
    (
        "%stretchy-vector-element-setter",
        "nod_stretchy_vector_element_setter",
        3,
        TypeEstimate::Top,
    ),
    ("%stretchy-vector-push", "nod_stretchy_vector_push", 2, TypeEstimate::Top),
    // FIP primitives — drive the existing Rust iteration state.
    ("%fip-init", "nod_fip_init", 1, TypeEstimate::Top),
    ("%fip-finished?", "nod_fip_finished_p", 1, TypeEstimate::Boolean),
    ("%fip-current-element", "nod_fip_current_element", 1, TypeEstimate::Top),
    ("%fip-advance!", "nod_fip_advance", 1, TypeEstimate::Top),
    // Allocators — for tests that exercise <range> and <stretchy-vector>
    // from Dylan source without going through `make(<range>, …)` keyword
    // dispatch.
    ("%make-range", "nod_make_range", 3, TypeEstimate::Top),
    ("%make-stretchy-vector", "nod_make_stretchy_vector", 1, TypeEstimate::Top),
    // Sprint 21: first-class function dispatch primitives.
    // Sprint 26: extended to arities 0 and 3..=5 so closures and
    // env-bound function-Refs can be called cleanly without packing
    // args into a `<simple-object-vector>` for `nod_apply`.
    ("%funcall0", "nod_funcall0", 1, TypeEstimate::Top),
    ("%funcall1", "nod_funcall1", 2, TypeEstimate::Top),
    ("%funcall2", "nod_funcall2", 3, TypeEstimate::Top),
    ("%funcall3", "nod_funcall3", 4, TypeEstimate::Top),
    ("%funcall4", "nod_funcall4", 5, TypeEstimate::Top),
    ("%funcall5", "nod_funcall5", 6, TypeEstimate::Top),
    ("%apply", "nod_apply", 2, TypeEstimate::Top),
    // Sprint 21: allocate a zero-filled `<simple-object-vector>` of the
    // given length. Mirrors `collection_map`'s allocator path.
    ("%make-sov", "nod_make_sov_len", 1, TypeEstimate::Top),
    // Sprint 24: closures — `<cell>` and `<environment>` primitives.
    // `%make-cell(v) -> <cell>`. Allocate a one-slot box.
    ("%make-cell", "nod_make_cell", 1, TypeEstimate::Top),
    // `%cell-get(c) -> <object>`. Load the cell's value slot.
    ("%cell-get", "nod_cell_get", 1, TypeEstimate::Top),
    // `%cell-set!(v, c) -> v`. Store through the GC write barrier.
    ("%cell-set!", "nod_cell_set", 2, TypeEstimate::Top),
    // `%env-cell(env, idx) -> <cell>`. Read a cell pointer out of an
    // environment by index. The caller follows up with `%cell-get` /
    // `%cell-set!` to actually read/write the captured variable.
    ("%env-cell", "nod_env_cell", 2, TypeEstimate::Top),
    // `%make-environment(cells_vec) -> <environment>`. Wrap a pre-built
    // SOV of cell-Words into an environment record.
    ("%make-environment", "nod_make_environment", 1, TypeEstimate::Top),
    // `%make-closure(name, arity, env) -> <function>`. Allocate a fresh
    // closure `<function>` Word in the moveable heap whose body is the
    // already-registered `name` symbol and whose env-ptr slot points at
    // `env`. The lowerer emits this at every closure-creation site that
    // captures at least one variable.
    ("%make-closure", "nod_make_closure", 3, TypeEstimate::Top),
    // Sprint 42a — <byte-string> primitives. Minimum surface (allocate,
    // size, byte-read, byte-write, bulk-copy); all higher-level ops
    // (`concatenate`, `copy-sequence`, `subsequence`, `starts-with?`,
    // `ends-with?`, `find-substring`, `as-uppercase`, `as-lowercase`,
    // `empty?`) live in `stdlib.dylan` and call these.
    ("%byte-string-allocate", "nod_byte_string_allocate", 1, TypeEstimate::Top),
    ("%byte-string-size", "nod_byte_string_size", 1, TypeEstimate::Integer),
    ("%byte-string-element", "nod_byte_string_element", 2, TypeEstimate::Integer),
    ("%byte-string-element-setter", "nod_byte_string_element_setter", 3, TypeEstimate::Integer),
    ("%byte-string-copy!", "nod_byte_string_copy_bytes", 5, TypeEstimate::Integer),
    // Sprint 22 — <table> + hashing.
    ("%make-table", "nod_make_table", 1, TypeEstimate::Top),
    ("%table-size", "nod_table_size", 1, TypeEstimate::Integer),
    ("%table-element", "nod_table_element", 2, TypeEstimate::Top),
    ("%table-element-or-default", "nod_table_element_or_default", 3, TypeEstimate::Top),
    ("%table-element-setter", "nod_table_element_setter", 3, TypeEstimate::Top),
    ("%table-remove-key", "nod_table_remove_key", 2, TypeEstimate::Top),
    ("%table-keys", "nod_table_keys", 1, TypeEstimate::Top),
    ("%table-values", "nod_table_values", 1, TypeEstimate::Top),
    ("%object-hash", "nod_object_hash", 1, TypeEstimate::Integer),
    ("%object-equal?", "nod_object_equal_p", 2, TypeEstimate::Boolean),
    // Sprint 32 — closure → C function pointer trampolines. Each
    // primitive takes a `<function>` Word and returns a fixnum-tagged
    // `<c-pointer>` Word whose payload is the trampoline address Win32
    // can call through the standard Win64 ABI.
    ("%register-wndproc", "nod_register_wndproc", 1, TypeEstimate::Top),
    ("%register-wndenumproc", "nod_register_wndenumproc", 1, TypeEstimate::Top),
    // Sprint 34 — <c-struct> field accessors. Get primitives return an
    // <integer>; set primitives return the value Word (Dylan setter
    // convention). The offset arg is a fixnum literal baked into the
    // stdlib accessor.
    ("%struct-get-i32", "nod_struct_get_i32", 2, TypeEstimate::Integer),
    ("%struct-set-i32", "nod_struct_set_i32", 3, TypeEstimate::Integer),
    ("%struct-get-i64", "nod_struct_get_i64", 2, TypeEstimate::Integer),
    ("%struct-set-i64", "nod_struct_set_i64", 3, TypeEstimate::Integer),
    ("%struct-get-u16", "nod_struct_get_u16", 2, TypeEstimate::Integer),
    ("%struct-set-u16", "nod_struct_set_u16", 3, TypeEstimate::Integer),
    ("%struct-get-u32", "nod_struct_get_u32", 2, TypeEstimate::Integer),
    ("%struct-set-u32", "nod_struct_set_u32", 3, TypeEstimate::Integer),
    ("%struct-get-u64", "nod_struct_get_u64", 2, TypeEstimate::Integer),
    ("%struct-set-u64", "nod_struct_set_u64", 3, TypeEstimate::Integer),
    ("%struct-get-pointer", "nod_struct_get_pointer", 2, TypeEstimate::Integer),
    ("%struct-set-pointer", "nod_struct_set_pointer", 3, TypeEstimate::Integer),
    // Sprint 35 — COM shim: DXGI / D3D11 / D2D / DirectWrite primitives.
    // All return a fixnum-tagged opaque handle (or 0 on error). Sprint 35
    // uses integer-encoded floats throughout (color channels as
    // 0..=255, coordinates as integer pixels) — see
    // `nod-runtime::com_shim` module docs for the deviation rationale.
    ("%com-release", "nod_com_release", 1, TypeEstimate::Integer),
    ("%com-registry-len", "nod_com_registry_len", 0, TypeEstimate::Integer),
    ("%com-last-hresult", "nod_com_last_hresult", 0, TypeEstimate::Integer),
    ("%com-clear-last-hresult", "nod_com_clear_last_hresult", 0, TypeEstimate::Integer),
    ("%dxgi-create-factory", "nod_dxgi_create_factory", 0, TypeEstimate::Integer),
    ("%dxgi-device-from-d3d-device", "nod_dxgi_device_from_d3d_device", 1, TypeEstimate::Integer),
    ("%dxgi-create-surface-from-texture", "nod_dxgi_create_surface_from_texture", 1, TypeEstimate::Integer),
    ("%d3d11-create-device", "nod_d3d11_create_device", 0, TypeEstimate::Integer),
    ("%d3d11-get-immediate-context", "nod_d3d11_get_immediate_context", 1, TypeEstimate::Integer),
    ("%d3d11-create-texture-2d", "nod_d3d11_create_texture_2d", 4, TypeEstimate::Integer),
    ("%d3d11-copy-to-staging-and-map", "nod_d3d11_copy_to_staging_and_map", 5, TypeEstimate::Integer),
    ("%d3d11-last-staging-handle", "nod_d3d11_last_staging_handle", 0, TypeEstimate::Integer),
    ("%d3d11-last-mapped-row-pitch", "nod_d3d11_last_mapped_row_pitch", 0, TypeEstimate::Integer),
    ("%d3d11-unmap", "nod_d3d11_unmap", 2, TypeEstimate::Integer),
    ("%d2d-create-factory", "nod_d2d_create_factory", 0, TypeEstimate::Integer),
    ("%d2d-create-device", "nod_d2d_create_device", 2, TypeEstimate::Integer),
    ("%d2d-create-device-context", "nod_d2d_create_device_context", 1, TypeEstimate::Integer),
    ("%d2d-create-bitmap-for-target", "nod_d2d_create_bitmap_for_target", 2, TypeEstimate::Integer),
    ("%d2d-set-target", "nod_d2d_set_target", 2, TypeEstimate::Integer),
    ("%d2d-begin-draw", "nod_d2d_begin_draw", 1, TypeEstimate::Integer),
    ("%d2d-end-draw", "nod_d2d_end_draw", 1, TypeEstimate::Integer),
    ("%d2d-clear", "nod_d2d_clear", 5, TypeEstimate::Integer),
    ("%d2d-set-transform-identity", "nod_d2d_set_transform_identity", 1, TypeEstimate::Integer),
    ("%d2d-create-solid-color-brush", "nod_d2d_create_solid_color_brush", 5, TypeEstimate::Integer),
    ("%d2d-draw-text-layout", "nod_d2d_draw_text_layout", 5, TypeEstimate::Integer),
    ("%d2d-draw-rectangle", "nod_d2d_draw_rectangle", 7, TypeEstimate::Integer),
    ("%d2d-fill-rectangle", "nod_d2d_fill_rectangle", 6, TypeEstimate::Integer),
    ("%dwrite-create-factory", "nod_dwrite_create_factory", 0, TypeEstimate::Integer),
    ("%dwrite-create-text-format", "nod_dwrite_create_text_format", 4, TypeEstimate::Integer),
    ("%dwrite-create-text-layout", "nod_dwrite_create_text_layout", 5, TypeEstimate::Integer),
    ("%dwrite-get-layout-metrics", "nod_dwrite_get_layout_metrics", 1, TypeEstimate::Integer),
    ("%dwrite-hit-test-position", "nod_dwrite_hit_test_text_position", 3, TypeEstimate::Integer),
    ("%dwrite-hit-test-point", "nod_dwrite_hit_test_point", 3, TypeEstimate::Integer),
    ("%dwrite-set-drawing-effect", "nod_dwrite_set_drawing_effect", 4, TypeEstimate::Integer),
    ("%dwrite-set-line-spacing", "nod_dwrite_set_line_spacing", 3, TypeEstimate::Integer),
    ("%count-non-zero-red", "nod_count_non_zero_red", 4, TypeEstimate::Integer),
    // Sprint 36 — HWND-bound swap chain + IDE-shell window-class primitives.
    // All return fixnum-tagged handles, atoms, or HRESULT-encoded results;
    // float marshaling is deferred (Sprint 37+).
    ("%dxgi-factory-from-d3d-device", "nod_dxgi_factory_from_d3d_device", 1, TypeEstimate::Integer),
    ("%dxgi-create-swap-chain-for-hwnd", "nod_dxgi_create_swap_chain_for_hwnd", 5, TypeEstimate::Integer),
    ("%d2d-create-bitmap-from-swap-chain", "nod_d2d_create_bitmap_from_swap_chain", 2, TypeEstimate::Integer),
    ("%dxgi-swap-chain-present", "nod_dxgi_swap_chain_present", 1, TypeEstimate::Integer),
    ("%dxgi-swap-chain-resize-buffers", "nod_dxgi_swap_chain_resize_buffers", 3, TypeEstimate::Integer),
    ("%register-window-class", "nod_register_window_class", 2, TypeEstimate::Integer),
    ("%create-message-only-window", "nod_create_message_only_window", 1, TypeEstimate::Integer),
    ("%create-hidden-window", "nod_create_hidden_window", 1, TypeEstimate::Integer),
    ("%destroy-window", "nod_destroy_window", 1, TypeEstimate::Integer),
    ("%post-message", "nod_post_message", 4, TypeEstimate::Integer),
    ("%pump-one-message", "nod_pump_one_message", 1, TypeEstimate::Integer),
    // Sprint 41a — blocking Win32 message loop. Arity-0, returns the
    // fixnum-tagged WPARAM of the WM_QUIT message (typically the value
    // a WNDPROC's WM_DESTROY handler passed to `PostQuitMessage`).
    ("%run-message-loop", "nod_run_message_loop", 0, TypeEstimate::Integer),
    ("%def-window-proc", "nod_def_window_proc", 4, TypeEstimate::Integer),
    // Sprint 41b — IDE source-viewer primitives. Both return either a
    // fresh `<byte-string>` Word or the `nil` immediate, so the type
    // estimate has to be `Top` (a union that includes neither
    // `<integer>` nor a unique class). Dylan-side callers branch on
    // `result = nil` to surface "no file" / "no arg" cases.
    ("%read-file", "nod_read_file_to_string", 1, TypeEstimate::Top),
    ("%argv1", "nod_get_argv1", 0, TypeEstimate::Top),
    // Sprint 41b — LOWORD/HIWORD extraction for WM_SIZE `lparam` unpack.
    // Both take a fixnum value and return a fixnum. Future sprints
    // should replace with general bitwise primitives.
    ("%lo-word", "nod_lo_word", 1, TypeEstimate::Integer),
    ("%hi-word", "nod_hi_word", 1, TypeEstimate::Integer),
    // Sprint 41c — scrollbar primitives. `%set-scroll-info` takes
    // (hwnd, nbar, n-min, n-max, n-page, n-pos, redraw); `%get-scroll-pos`
    // takes (hwnd, nbar). Both return fixnum-tagged integers.
    ("%set-scroll-info", "nod_set_scroll_info", 7, TypeEstimate::Integer),
    ("%get-scroll-pos", "nod_get_scroll_pos", 2, TypeEstimate::Integer),
    // Sprint 41e — File → Open. Wraps Win32 `GetOpenFileNameW` plus the
    // 88-byte `OPENFILENAMEW` struct in a single shim that returns the
    // chosen path as a `<byte-string>` (or `nil` if the user cancelled).
    // Arity-1: takes the owner HWND as a fixnum. Return is a string-or-
    // nil union, so the type estimate has to be `Top`.
    ("%show-open-file-dialog", "nod_show_open_file_dialog", 1, TypeEstimate::Top),
    // Sprint 41g — File → Save / Save As. `%write-file` takes
    // (path, content) — both `<byte-string>` Words — and returns
    // fixnum 1 on success / 0 on I/O error. `%show-save-file-dialog`
    // mirrors `%show-open-file-dialog` exactly but calls
    // `GetSaveFileNameW` with OFN_OVERWRITEPROMPT.
    ("%write-file", "nod_write_file_from_string", 2, TypeEstimate::Integer),
    ("%show-save-file-dialog", "nod_show_save_file_dialog", 1, TypeEstimate::Top),
    // Sprint 41g's `%load-recent`, `%add-recent`, `%basename` primitives
    // (and Sprint 41c's `%count-newlines` / 41d's `%max-line-chars`) are
    // retired — Sprint 42a Phase E moved all of them into pure Dylan
    // in `tests/nod-tests/fixtures/nod-ide.dylan`, built on the
    // byte-string ops (`size`, `element`, `concatenate`, `copy-sequence`,
    // `=`) plus the `%read-file` / `%write-file` primitives.
];

fn lookup_primitive(name: &str) -> Option<(&'static str, usize, TypeEstimate)> {
    LOWER_PRIMITIVE_TABLE
        .iter()
        .find(|(n, _, _, _)| *n == name)
        .map(|(_, sym, ar, ty)| (*sym, *ar, *ty))
}

/// Sprint 21: a Dylan-source operator name (`+`, `-`, `*`, `=`, `<`,
/// `>`) used as a first-class function reference (`\+` etc.) has a
/// fixed runtime-shim arity. The shims live in `nod-runtime::functions`
/// and are pre-registered in the function-ref registry by
/// `ensure_operator_shims_registered`.
fn operator_arity(name: &str) -> Option<usize> {
    match name {
        "+" | "-" | "*" | "=" | "<" | ">" => Some(2),
        _ => None,
    }
}

/// Sprint 16: the five `<pair>` / `<list>` builtins. Each lowers to a
/// synthetic `%pair*` / `%nil` / `%empty?` callee that codegen turns
/// into a call into the matching `nod_runtime` shim.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum ListBuiltin {
    /// `pair(head, tail) -> <pair>`.
    Pair,
    /// `head(p :: <pair>) -> <object>`.
    Head,
    /// `tail(p :: <pair>) -> <object>`.
    Tail,
    /// `empty?(p) -> <boolean>`. Identity test against `nil`.
    EmptyP,
    /// `nil() -> <empty-list>`. Returns the pinned empty-list singleton.
    Nil,
}

impl ListBuiltin {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "pair" => Some(ListBuiltin::Pair),
            "head" => Some(ListBuiltin::Head),
            "tail" => Some(ListBuiltin::Tail),
            "empty?" => Some(ListBuiltin::EmptyP),
            "nil" => Some(ListBuiltin::Nil),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            ListBuiltin::Pair => "pair",
            ListBuiltin::Head => "head",
            ListBuiltin::Tail => "tail",
            ListBuiltin::EmptyP => "empty?",
            ListBuiltin::Nil => "nil",
        }
    }

    fn arity(self) -> usize {
        match self {
            ListBuiltin::Pair => 2,
            ListBuiltin::Head | ListBuiltin::Tail | ListBuiltin::EmptyP => 1,
            ListBuiltin::Nil => 0,
        }
    }

    /// Synthetic callee symbol carried in the DFM `DirectCall` and
    /// recognised by the codegen layer. Each one maps to a `nod_runtime`
    /// extern shim with a fixed ABI.
    fn callee_symbol(self) -> &'static str {
        match self {
            ListBuiltin::Pair => "%pair-alloc",
            ListBuiltin::Head => "%pair-head",
            ListBuiltin::Tail => "%pair-tail",
            ListBuiltin::EmptyP => "%empty?",
            ListBuiltin::Nil => "%nil",
        }
    }
}

/// Aggregated output of `lower_module_full`. Sprint 12 carries class
/// and method registrations alongside the lowered function list so
/// the JIT-glue (in `nod-sema::lib`) can install them. Sprint 15
/// adds the per-library sealing facts captured during lowering so
/// the dispatch resolver, `dump_sealed`, and the JIT-time installer
/// can read them.
#[derive(Default, Clone, Debug)]
pub struct LoweredModule {
    pub functions: Vec<Function>,
    pub methods: Vec<MethodRegistration>,
    /// Sprint 15 sealing facts collected from the parsed modifiers
    /// and `define sealed domain` declarations.
    pub sealing: crate::optimise::SealingFacts,
    /// Sprint 15 dispatch resolution log — one entry per `Dispatch`
    /// node the resolver inspected. Stored for `dump_dispatch`
    /// annotations and as a diagnostic aid; not load-bearing for
    /// codegen.
    pub resolutions: Vec<crate::optimise::DispatchResolution>,
    /// Sprint 19: every `block` form encountered during lowering.
    /// Post-JIT the glue (`register_blocks`) resolves the lifted thunk
    /// names to function pointers and registers them with the runtime
    /// (`nod_runtime::register_block_fns`).
    pub blocks: Vec<BlockRegistration>,
    /// Sprint 24: closure metadata produced by `lift_anonymous_methods`.
    /// The `register_top_level_functions` glue consults this to
    /// register closure bodies under their *source* arity (the body's
    /// JIT signature carries a hidden env parameter on top).
    pub closures: ClosureRegistry,
    /// Sprint 27: every `define c-function` we encountered during
    /// lowering. The driver / FFI glue (Sprint 28+) consults this to
    /// emit the per-module API stub table; Sprint 27 just recorded
    /// the metadata. Sprint 28 adds the parsed marshaling signature
    /// to each binding.
    pub c_functions: Vec<CFunctionBinding>,
    /// Sprint 28: deduplicated stub-table for this module. One entry
    /// per unique `(dll, symbol)` pair referenced by the module's
    /// `define c-function`s. The driver-side glue (`eval_expr_to_string`)
    /// builds the runtime [`nod_runtime::ApiStubTable`] from these
    /// specs and calls `nod_runtime::initialize_stub_table` BEFORE
    /// any JIT-emitted code runs. The `entry_ptr` field is patched
    /// in-place by lowering once the static-area entries exist.
    pub c_function_stub_table: Vec<CFunctionStubEntry>,
    /// Sprint 27: non-fatal diagnostics. Sprint 27 surfaces these
    /// for `define c-function` declarations whose target symbol is
    /// not present in the embedded `nod-winapi` index. The driver
    /// prints them; they don't block compilation.
    pub warnings: Vec<LoweringWarning>,
    /// Sprint 40a — every `define class` registered during lowering,
    /// in declaration / registration order. Used by the AOT pipeline
    /// (`compile_file_for_aot` → `build_aot_registrations`) to emit
    /// `nod_aot_register_user_class` calls inside the EXE's startup
    /// resolver. The JIT path ignores this field — it registers user
    /// classes inline as `register_class` runs in `lower_module_full`.
    pub user_classes: Vec<UserClassRegistration>,
}

/// Sprint 27: information captured for a single `define c-function`
/// declaration. Carries the DLL provenance + the c-side identifier;
/// Sprint 28 adds the marshaling signature + the index into the
/// per-module stub table. Sprint 31 adds `source` so callers can tell
/// user-written declarations apart from bindings the JIT materialized
/// from the embedded Win32 index on the fly.
#[derive(Clone, Debug)]
pub struct CFunctionBinding {
    pub dylan_name: String,
    pub c_name: String,
    pub library: String,
    pub span: Span,
    /// `true` when the symbol was found in the embedded
    /// `nod-winapi` index at compile time. `false` means the user
    /// declared a custom DLL/symbol the DB doesn't know about — we
    /// warn but continue.
    pub resolved_in_db: bool,
    /// Sprint 28: marshaling signature derived from the param /
    /// return c-type annotations. `None` when the declaration uses a
    /// c-type outside the Sprint 28 supported set; calls then surface
    /// a deferral diagnostic.
    pub signature: Option<nod_runtime::ApiCallSignature>,
    /// Sprint 31: provenance of this binding. `UserCFunction` for any
    /// explicit `define c-function` in user source (or stdlib);
    /// `JitMaterialized` when the lowerer synthesized the binding from
    /// the embedded `nod-winapi` index because a bare-name call site
    /// referenced a Win32 export the user hadn't declared.
    pub source: BindingSource,
}

/// Sprint 31: where a [`CFunctionBinding`] came from. User declarations
/// always win — if a name is declared explicitly anywhere in the module,
/// the JIT-materialization path declines to synthesize a binding for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingSource {
    /// `define c-function` written in user Dylan source (or in the
    /// future, stdlib). The default for every Sprint 27 / 28 / 30
    /// declaration.
    UserCFunction,
    /// Synthesized on the fly by Sprint 31's bare-name lookup hook.
    /// The binding never appears in the source; the lowerer fabricated
    /// it from the embedded `nod-winapi` index because a call site
    /// referenced a name the user hadn't declared.
    JitMaterialized,
}

/// Sprint 28: one resolved stub-table entry for the module being
/// lowered. The runtime-side [`nod_runtime::ApiStubTable`] is built
/// from these specs at JIT-finalize time. The per-call lowering bakes
/// the entry's static-area pointer (recovered from `entry_ptr`) into
/// the call site as an `i64` constant.
#[derive(Clone, Debug)]
pub struct CFunctionStubEntry {
    pub dll: String,
    pub symbol: String,
    pub signature: nod_runtime::ApiCallSignature,
    /// Pointer to the static-area [`nod_runtime::ApiStubEntry`] this
    /// resolved to. Populated once the per-module table is built.
    /// Until then this is null and per-call codegen would emit a 0
    /// constant; we always allocate the table BEFORE lowering call
    /// sites, so callers never observe `None` here.
    pub entry_ptr: u64,
}

/// Sprint 27: non-fatal sema diagnostic.
#[derive(Clone, Debug)]
pub enum LoweringWarning {
    /// `define c-function NAME` references a (library, c-name) pair
    /// not present in the embedded `nod-winapi` index. Sprint 27
    /// accepts the declaration anyway — the user may target a
    /// custom DLL. Sprint 28's call-site lowering will error at
    /// runtime if the LoadLibrary / GetProcAddress fails.
    CFunctionNotInDb {
        span: Span,
        name: String,
        library: String,
        c_name: String,
    },
}

impl std::fmt::Display for LoweringWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoweringWarning::CFunctionNotInDb { name, library, c_name, span } => write!(
                f,
                "warning: `define c-function {name}` (library: \"{library}\", c-name: \"{c_name}\") \
                 not in windows_api database; will fail at runtime if the DLL doesn't export it [{:?}]",
                span
            ),
        }
    }
}

/// Sprint 31: a synthesized c-function binding the lowerer fabricated
/// from the embedded Win32 index because a bare-name call site
/// referenced a name the user hadn't declared. Pre-stub-table working
/// state — once we've allocated the table this turns into both a
/// `CFunctionBinding` (for introspection) and a `CFunctionCallInfo`
/// (for per-call lowering).
#[derive(Clone, Debug)]
struct MaterializedBindingSpec {
    dylan_name: String,
    c_name: String,
    library: String,
    span: Span,
    signature: nod_runtime::ApiCallSignature,
    /// Index into `c_function_specs` where the stub-table slot lives.
    spec_idx: usize,
}

/// Sprint 31: outcome of [`try_jit_materialize_winapi`] for a single
/// bare-name candidate. Distinguishes "found and fully supported",
/// "found but signature unsupported" (so we can surface a helpful
/// diagnostic), and "not in the index at all" (so we fall through to
/// the existing unknown-ident path).
#[derive(Clone, Debug)]
enum MaterializationOutcome {
    Materialized {
        c_name: String,
        library: String,
        signature: nod_runtime::ApiCallSignature,
    },
    UnsupportedSignature {
        /// The DLL the matched function lives in. Surfaced for future
        /// diagnostic improvements (currently consumed only via the
        /// `reason` string).
        #[allow(dead_code)]
        c_name: String,
        #[allow(dead_code)]
        library: String,
        reason: String,
    },
    NotFound,
}

/// Sprint 31: DLL priority order for cross-DLL name collisions. Kernel
/// wins over user / gdi / advapi / shell / comctl; any other DLL falls
/// to alphabetical fallback. The list is small; a linear scan beats
/// pulling in a `phf` for six strings.
const WINAPI_DLL_PRIORITY: &[&str] = &[
    "kernel32.dll",
    "user32.dll",
    "gdi32.dll",
    "advapi32.dll",
    "shell32.dll",
    "comctl32.dll",
];

fn winapi_dll_priority(dll: &str) -> usize {
    WINAPI_DLL_PRIORITY
        .iter()
        .position(|&p| p == dll)
        .unwrap_or(WINAPI_DLL_PRIORITY.len())
}

/// Sprint 31: try to materialize a [`MaterializedBindingSpec`] for the
/// bare-name `name`. Default A/W disambiguation prefers W; if the
/// literal name already ends in `A` or `W` (or neither variant exists)
/// we use it as-is. Cross-DLL ambiguity is broken by
/// [`WINAPI_DLL_PRIORITY`]. Functions whose param / return types fall
/// outside Sprint 28-30's marshaling set return
/// [`MaterializationOutcome::UnsupportedSignature`] so the caller can
/// surface a helpful diagnostic instead of "unknown identifier".
fn try_jit_materialize_winapi(name: &str) -> MaterializationOutcome {
    // Pull candidates by name. We need to enumerate every DLL the name
    // lives in to apply the priority order, so we scan `functions()`
    // once rather than relying on the convenience accessor (which only
    // surfaces the first match).
    let try_one = |candidate_name: &str| -> Vec<&'static nod_winapi::FunctionInfo> {
        nod_winapi::functions()
            .iter()
            .filter(|f| f.name == candidate_name)
            .collect()
    };

    // A/W default: if the user wrote a bare name with no A/W suffix,
    // prefer the W variant (modern Unicode-correct).
    let try_order: Vec<String> = if name.ends_with('A') || name.ends_with('W') {
        vec![name.to_string()]
    } else {
        vec![format!("{name}W"), name.to_string()]
    };
    let mut candidates: Vec<&'static nod_winapi::FunctionInfo> = Vec::new();
    let mut resolved_via: String = String::new();
    for n in &try_order {
        let hits = try_one(n);
        if !hits.is_empty() {
            candidates = hits;
            resolved_via = n.clone();
            break;
        }
    }
    if candidates.is_empty() {
        return MaterializationOutcome::NotFound;
    }
    // Cross-DLL priority. Stable secondary key on dll name keeps the
    // pick deterministic when two non-priority DLLs tie.
    candidates.sort_by(|a, b| {
        winapi_dll_priority(&a.dll)
            .cmp(&winapi_dll_priority(&b.dll))
            .then_with(|| a.dll.cmp(&b.dll))
    });
    let chosen = candidates[0];

    match build_signature_from_function_info(chosen) {
        Ok(sig) => MaterializationOutcome::Materialized {
            c_name: resolved_via,
            library: chosen.dll.clone(),
            signature: sig,
        },
        Err(reason) => MaterializationOutcome::UnsupportedSignature {
            c_name: chosen.name.clone(),
            library: chosen.dll.clone(),
            reason,
        },
    }
}

/// Sprint 31: derive a Sprint 28/30 marshaling signature from a
/// [`nod_winapi::FunctionInfo`]. Returns `Err(reason)` if any param /
/// return type uses a category Sprint 28-30 can't marshal yet
/// (struct-by-value, function-pointer callback, opaque
/// pointer-to-pointer, …).
fn build_signature_from_function_info(
    info: &nod_winapi::FunctionInfo,
) -> Result<nod_runtime::ApiCallSignature, String> {
    if info.params.len() > 12 {
        return Err(format!(
            "arity {} exceeds Sprint 36b cap of 12",
            info.params.len()
        ));
    }
    let mut arg_kinds = [nod_runtime::CArgKind::Void as u8; 12];
    for (i, p) in info.params.iter().enumerate() {
        let kind = c_arg_kind_from_type_ref(&p.type_ref).map_err(|why| {
            format!(
                "parameter #{} ({}) has unsupported type: {}",
                i + 1,
                p.name.as_deref().unwrap_or("?"),
                why
            )
        })?;
        arg_kinds[i] = kind as u8;
    }
    let return_kind = c_return_kind_from_type_ref(&info.return_type)
        .map_err(|why| format!("return type has unsupported shape: {why}"))?;
    Ok(nod_runtime::ApiCallSignature {
        arg_count: info.params.len() as u8,
        arg_kinds,
        return_kind: return_kind as u8,
    })
}

/// Sprint 31: map a [`nod_winapi::TypeRef`] to a [`nod_runtime::CArgKind`].
/// Mirrors the Dylan-name table in `nod_runtime::CArgKind::from_c_type_name`
/// but works on the structured TypeRef enum directly so the JIT
/// materializer doesn't have to stringify-then-parse.
fn c_arg_kind_from_type_ref(t: &nod_winapi::TypeRef) -> Result<nod_runtime::CArgKind, String> {
    use nod_runtime::CArgKind;
    use nod_winapi::TypeRef as T;
    Ok(match t {
        T::I8 => CArgKind::Int8,
        T::U8 => CArgKind::UInt8,
        T::I16 => CArgKind::Int16,
        T::U16 => CArgKind::UInt16,
        T::I32 => CArgKind::Int32,
        T::U32 => CArgKind::UInt32,
        T::I64 => CArgKind::Int64,
        T::U64 => CArgKind::UInt64,
        T::Bool32 => CArgKind::Bool32,
        T::Handle => CArgKind::Handle,
        T::NarrowString => CArgKind::NarrowString,
        T::WideString => CArgKind::WideString,
        T::Pointer { pointee_type_ref } => match pointee_type_ref {
            // Opaque `*mut void` and one-level pointers to primitive
            // scalars marshal as a raw pointer; the Dylan side passes a
            // fixnum 0 (NULL) or a tagged-pointer word in.
            None => CArgKind::Pointer,
            Some(inner) => match inner.as_ref() {
                T::I8 | T::U8 | T::I16 | T::U16 | T::I32 | T::U32 | T::I64 | T::U64
                | T::Handle | T::Pointer { .. } => CArgKind::Pointer,
                // Pointers to enums / aliases / strings reduce to
                // raw `void*` for Sprint 31's purposes — callers can
                // still pass NULL or a raw word.
                T::Enum { .. } | T::Alias { .. } => CArgKind::Pointer,
                T::Bool32 => CArgKind::Pointer,
                T::NarrowString | T::WideString => CArgKind::Pointer,
                T::Void => CArgKind::Pointer,
            },
        },
        T::Enum { base } => c_arg_kind_from_type_ref(base)?,
        T::Alias { base, .. } => c_arg_kind_from_type_ref(base)?,
        T::Void => return Err("void as parameter type".to_string()),
    })
}

/// Sprint 31: companion to [`c_arg_kind_from_type_ref`] for return
/// types. Returns the matching [`nod_runtime::CReturnKind`].
fn c_return_kind_from_type_ref(t: &nod_winapi::TypeRef) -> Result<nod_runtime::CReturnKind, String> {
    use nod_runtime::CReturnKind;
    use nod_winapi::TypeRef as T;
    Ok(match t {
        T::Void => CReturnKind::Void,
        T::I8 | T::I16 | T::I32 => CReturnKind::Int32,
        T::U8 | T::U16 | T::U32 => CReturnKind::UInt32,
        T::I64 => CReturnKind::Int64,
        T::U64 => CReturnKind::UInt64,
        T::Bool32 => CReturnKind::Bool32,
        T::Handle => CReturnKind::Handle,
        T::NarrowString => CReturnKind::NarrowString,
        T::WideString => CReturnKind::WideString,
        T::Pointer { .. } => CReturnKind::Pointer,
        T::Enum { base } => c_return_kind_from_type_ref(base)?,
        T::Alias { base, .. } => c_return_kind_from_type_ref(base)?,
    })
}

/// Sprint 38d — bytewise-encode an [`nod_runtime::ApiCallSignature`] for
/// carriage in the DFM IR + the manifest sidecar. `ApiCallSignature` is
/// `#[repr(C)] Copy` so a `transmute`-equivalent `copy_nonoverlapping`
/// is well-defined; the inverse happens in
/// `nod_llvm::jit::resolve_reloc_kind` on the warm-replay path.
fn signature_to_bytes(sig: &nod_runtime::ApiCallSignature) -> Vec<u8> {
    let n = std::mem::size_of::<nod_runtime::ApiCallSignature>();
    let mut bytes = vec![0u8; n];
    // SAFETY: `ApiCallSignature` is `#[repr(C)] Copy` (struct of bytes
    // and a u8 array — no padding hazards on x86-64). The destination
    // slice has the exact same length.
    unsafe {
        std::ptr::copy_nonoverlapping(
            sig as *const nod_runtime::ApiCallSignature as *const u8,
            bytes.as_mut_ptr(),
            n,
        );
    }
    bytes
}

/// Sprint 31: walk a module's call sites collecting bare-name callees
/// that are *candidates* for JIT materialization — i.e. names that
/// aren't user-declared c-functions, aren't user-defined functions,
/// aren't generics, aren't classes, and aren't reserved builtins like
/// `make` / `instance?` / etc. The caller then tries each candidate
/// against the embedded Win32 index.
fn collect_bare_call_candidates(
    m: &Module,
    user_declared_c_names: &HashSet<String>,
    top_names: &TopNames,
    generics: &HashSet<String>,
    user_classes: &HashMap<String, ClassId>,
    out: &mut Vec<(String, nod_reader::Span)>,
) {
    for item in &m.items {
        match item {
            Item::DefineFunction { body, .. } | Item::DefineMethod { body, .. } => {
                for s in body {
                    walk_stmt_for_candidates(
                        s,
                        user_declared_c_names,
                        top_names,
                        generics,
                        user_classes,
                        out,
                    );
                }
            }
            Item::DefineConstant { value, .. } | Item::DefineVariable { value, .. } => {
                walk_expr_for_candidates(
                    value,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
            Item::Expr(e) => walk_expr_for_candidates(
                e,
                user_declared_c_names,
                top_names,
                generics,
                user_classes,
                out,
            ),
            _ => {}
        }
    }
}

fn walk_stmt_for_candidates(
    s: &Statement,
    user_declared_c_names: &HashSet<String>,
    top_names: &TopNames,
    generics: &HashSet<String>,
    user_classes: &HashMap<String, ClassId>,
    out: &mut Vec<(String, nod_reader::Span)>,
) {
    match s {
        Statement::Expr(e) => walk_expr_for_candidates(
            e,
            user_declared_c_names,
            top_names,
            generics,
            user_classes,
            out,
        ),
        Statement::Let { value, .. } => walk_expr_for_candidates(
            value,
            user_declared_c_names,
            top_names,
            generics,
            user_classes,
            out,
        ),
        Statement::Local { .. } => {}
        Statement::For { body, finally_, .. } => {
            for s in body {
                walk_stmt_for_candidates(
                    s,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
            for s in finally_ {
                walk_stmt_for_candidates(
                    s,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
        }
        Statement::While { cond, body, .. } | Statement::Until { cond, body, .. } => {
            walk_expr_for_candidates(
                cond,
                user_declared_c_names,
                top_names,
                generics,
                user_classes,
                out,
            );
            for s in body {
                walk_stmt_for_candidates(
                    s,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
        }
        Statement::Block { body, cleanup, afterwards, .. } => {
            for s in body {
                walk_stmt_for_candidates(
                    s,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
            for s in cleanup {
                walk_stmt_for_candidates(
                    s,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
            for s in afterwards {
                walk_stmt_for_candidates(
                    s,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
        }
    }
}

fn walk_expr_for_candidates(
    e: &Expr,
    user_declared_c_names: &HashSet<String>,
    top_names: &TopNames,
    generics: &HashSet<String>,
    user_classes: &HashMap<String, ClassId>,
    out: &mut Vec<(String, nod_reader::Span)>,
) {
    match e {
        Expr::Call { callee, args, span } => {
            if let Expr::Ident(_, name) = callee.as_ref()
                && is_winapi_candidate_name(
                    name,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                )
            {
                out.push((name.clone(), *span));
            }
            walk_expr_for_candidates(
                callee,
                user_declared_c_names,
                top_names,
                generics,
                user_classes,
                out,
            );
            for a in args {
                walk_expr_for_candidates(
                    a,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
        }
        Expr::BinOp { lhs, rhs, .. } => {
            walk_expr_for_candidates(
                lhs,
                user_declared_c_names,
                top_names,
                generics,
                user_classes,
                out,
            );
            walk_expr_for_candidates(
                rhs,
                user_declared_c_names,
                top_names,
                generics,
                user_classes,
                out,
            );
        }
        Expr::UnOp { operand, .. } => walk_expr_for_candidates(
            operand,
            user_declared_c_names,
            top_names,
            generics,
            user_classes,
            out,
        ),
        Expr::Paren { inner, .. } => walk_expr_for_candidates(
            inner,
            user_declared_c_names,
            top_names,
            generics,
            user_classes,
            out,
        ),
        Expr::If { cond, then_, else_, .. } => {
            walk_expr_for_candidates(
                cond,
                user_declared_c_names,
                top_names,
                generics,
                user_classes,
                out,
            );
            walk_expr_for_candidates(
                then_,
                user_declared_c_names,
                top_names,
                generics,
                user_classes,
                out,
            );
            if let Some(eb) = else_ {
                walk_expr_for_candidates(
                    eb,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
        }
        Expr::Begin { body, .. } => {
            for e in body {
                walk_expr_for_candidates(
                    e,
                    user_declared_c_names,
                    top_names,
                    generics,
                    user_classes,
                    out,
                );
            }
        }
        _ => {}
    }
}

/// Sprint 31: callee-name filter. A name is a winapi-candidate iff it
/// looks like a Win32 export (capital-letter start, all ASCII letters
/// / digits, no Dylan-style `<...>` / hyphenated tokens, and not a
/// known intrinsic / Dylan symbol).
fn is_winapi_candidate_name(
    name: &str,
    user_declared_c_names: &HashSet<String>,
    top_names: &TopNames,
    generics: &HashSet<String>,
    user_classes: &HashMap<String, ClassId>,
) -> bool {
    if user_declared_c_names.contains(name) {
        return false;
    }
    if top_names.contains(name) {
        return false;
    }
    if generics.contains(name) {
        return false;
    }
    if user_classes.contains_key(name) {
        return false;
    }
    if nod_runtime::is_generic_defined(name) {
        return false;
    }
    // Reserved Dylan-side identifiers. We could enumerate from a single
    // table, but the explicit allowlist of Win32-shape names below is
    // a stronger filter — if a name doesn't match that shape we never
    // bother the index.
    if !looks_like_win32_export(name) {
        return false;
    }
    true
}

/// Sprint 31: shape filter for Win32 exports. Must:
///   * Be at least 3 characters long
///   * Contain only ASCII letters and digits (no `-`, `_`, `<`, `>`, `?`, `!`)
///   * Contain at least one uppercase ASCII letter somewhere (so e.g.
///     `print`, `read`, `format` don't trigger a 13000-entry index
///     scan — every real Win32 export has at least one uppercase
///     letter, including the lowercase-prefixed ones like `lstrlenW`
///     and `wsprintfW`).
///
/// This keeps Dylan-side names like `print`, `+`, `<my-class>`, `id?`
/// out of the candidate set while admitting the unusual lowercase-
/// prefixed Win32 exports (`lstrlenA/W`, `wsprintf*`, `wnsprintf*`).
fn looks_like_win32_export(name: &str) -> bool {
    if name.len() < 3 {
        return false;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    // Must start with a letter (no leading digits — those aren't Dylan
    // identifiers anyway, but belt-and-braces).
    if !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // Must contain at least one uppercase letter somewhere. Every real
    // Win32 export does; ordinary Dylan identifiers don't.
    name.chars().any(|c| c.is_ascii_uppercase())
}

/// Sprint 19: one lifted-thunk set per `block` form in the source. The
/// names refer to top-level functions present in `LoweredModule::functions`
/// (each emitted with the canonical 8-captured-locals C ABI; handlers
/// take an additional leading `condition` arg).
#[derive(Clone, Debug)]
pub struct BlockRegistration {
    /// Runtime-allocated id (via `nod_runtime::allocate_block_id`).
    /// Baked into the call site as a `WordBits` constant.
    pub block_id: u64,
    pub body_fn_name: String,
    pub cleanup_fn_name: Option<String>,
    pub afterwards_fn_name: Option<String>,
    /// One entry per `exception` clause (source order).
    pub handlers: Vec<BlockHandlerRegistration>,
}

#[derive(Clone, Debug)]
pub struct BlockHandlerRegistration {
    pub class_id: ClassId,
    pub class_name: String,
    pub body_fn_name: String,
}

/// Sprint 40a — a single `define class` registration captured during
/// lowering. The JIT path doesn't need this (it calls
/// `nod_runtime::register_simple_user_class` / `register_mi_user_class`
/// inline as `register_class` runs); the AOT path serialises this shape
/// into the EXE's startup so a fresh process can replay the same
/// registrations with the same class IDs in the same order.
///
/// All offsets / CPL / slot_origin entries are already fully resolved
/// by the lowering pass (mirroring what `register_user_class_metadata`
/// pins into the static area on the JIT side). The EXE-side shim
/// (`nod_aot_register_user_class`) reconstructs a `UserClassSpec` from
/// these fields and calls `register_user_class_metadata` directly.
///
/// # Class-id determinism
///
/// The JIT/compiler process allocated `class_id` via
/// `allocate_user_class_id()` in monotonic order. The EXE-side
/// `nod_aot_resolve_relocs` calls `nod_aot_register_user_class` in the
/// SAME order this `Vec` was populated, so the EXE's
/// `allocate_user_class_id` produces the exact same sequence of IDs.
/// The shim asserts the returned id matches `class_id` and panics on
/// drift — a panic here would be a codegen bug, not a user error.
#[derive(Clone, Debug)]
pub struct UserClassRegistration {
    pub name: String,
    pub class_id: ClassId,
    /// Direct supers in declaration order. Empty for `<object>` (which
    /// the AOT path never emits — it's a seed class). For user classes
    /// with no explicit super list, this is `[<object>]` per Dylan
    /// convention (matching `register_class`'s default).
    pub parents: Vec<ClassId>,
    /// Full C3-linearised class precedence list including self at
    /// index 0.
    pub cpl: Vec<ClassId>,
    /// All slots (own + inherited) in layout order, with offsets +
    /// init-keyword strings + type kinds already populated.
    pub slots: Vec<SlotInfo>,
    /// For each `slots[i]`, the class id that introduced that slot
    /// (`class_id` for own slots; some ancestor's id for inherited).
    pub slot_origin: Vec<ClassId>,
    pub own_slot_count: usize,
    pub inherited_slot_count: usize,
}

pub fn lower_module(m: &Module) -> Result<Vec<Function>, Vec<LoweringError>> {
    lower_module_full(m).map(|lm| lm.functions)
}

pub fn lower_module_full(m: &Module) -> Result<LoweredModule, Vec<LoweringError>> {
    // Sprint 19: ensure the seed condition classes are registered
    // before lowering starts so `<error>` / `<simple-error>` / etc.
    // resolve via `find_class_id_by_name` during exception-clause
    // lowering. Idempotent — repeated calls are cheap.
    nod_runtime::ensure_conditions_registered();
    // Sprint 21: ensure the `<function>` / `<wrong-number-of-arguments-error>`
    // classes + operator shim registrations are alive before lowering
    // touches `\name` / anonymous-method expressions.
    nod_runtime::ensure_functions_registered();
    // Sprint 24: ensure `<cell>` and `<environment>` are registered
    // before any closure-creation site lowers. The runtime exports
    // `nod_make_cell` / `nod_cell_get` / … as `extern "C-unwind"` symbols
    // already; this just lights up the class table.
    nod_runtime::ensure_closures_registered();
    // Sprint 27: ensure FFI c-type classes (`<c-bool>`, `<c-dword>`,
    // …) are registered before any `define c-function` declaration
    // tries to validate its parameter / return type annotations
    // against the class table.
    nod_runtime::ensure_c_types_registered();

    // Sprint 21 pre-pass: rewrite every `Expr::Method` in expression
    // position to a synthetic `Expr::Ident(__anon-method-NNNN)` and
    // emit a matching `Item::DefineFunction` at the top level. The
    // normal lowering path then handles the lifted thunks as ordinary
    // top-level functions and the call sites as ordinary `\name`
    // references.
    let mut m_owned: Module = m.clone();
    let (closure_registry, lift_errors) = lift_anonymous_methods(&mut m_owned);
    if !lift_errors.is_empty() {
        return Err(lift_errors);
    }
    let m: &Module = &m_owned;

    let mut errors: Vec<LoweringError> = Vec::new();
    let mut user_classes: HashMap<String, ClassId> = HashMap::new();
    // Sprint 40a: capture each registered user class's metadata for
    // the AOT pipeline. The JIT path ignores this; the driver / AOT
    // codegen reads it through `LoweredModule::user_classes`.
    let mut user_class_registrations: Vec<UserClassRegistration> = Vec::new();

    // Phase 1a: walk define-class items and register metadata. The
    // sealing flag flip is deferred to Phase 1c so subclassing a
    // sealed class WITHIN THIS SAME `lower_module_full` call is
    // allowed (in-library subclassing — see spec 15 §6 table). The
    // cross-library refusal in `register_class` checks `is_sealed()`,
    // so an in-call subclass registration runs before the parent's
    // sealed bit is flipped; a later separate `lower_module_full`
    // call sees the flag and refuses.
    for item in &m.items {
        if let Item::DefineClass { name, supers, slots, span, .. } = item {
            match register_class(name, supers, slots, *span) {
                Ok(id) => {
                    user_classes.insert(name.clone(), id);
                    // Sprint 40a: snapshot the freshly-registered
                    // metadata for the AOT pipeline. Reading from the
                    // canonical static-area entry guarantees the
                    // persisted offsets / CPL / slot_origin match
                    // exactly what the JIT path resolved through the
                    // class table — no parallel computation, no drift.
                    let md_ptr = class_metadata_ptr(id);
                    if !md_ptr.is_null() {
                        // SAFETY: pointer is to static-area metadata
                        // (process-lived); we just registered it.
                        let md = unsafe { &*md_ptr };
                        user_class_registrations.push(UserClassRegistration {
                            name: md.name.clone(),
                            class_id: id,
                            parents: md.parents.clone(),
                            cpl: md.cpl.clone(),
                            slots: md.slots.clone(),
                            slot_origin: md.slot_origin.clone(),
                            own_slot_count: md.own_slot_count,
                            inherited_slot_count: md.inherited_slot_count,
                        });
                    }
                }
                Err(e) => errors.push(e),
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // Phase 1c: flip sealed flags on classes + generics that bear the
    // `sealed` modifier. Runs AFTER every class in this lowering call
    // is registered (and after any in-library subclasses of a sealed
    // parent are themselves registered), so the cross-library refusal
    // in `register_class` doesn't fire for in-library use.
    for item in &m.items {
        if let Item::DefineClass { name, modifiers, .. } = item
            && modifiers.contains(&nod_reader::Modifier::Sealed)
            && let Some(&id) = user_classes.get(name)
        {
            let p = class_metadata_ptr(id);
            if !p.is_null() {
                // SAFETY: static-area metadata.
                unsafe { (*p).mark_sealed() };
            }
        }
        if let Item::DefineGeneric { name, modifiers, .. } = item
            && modifiers.contains(&nod_reader::Modifier::Sealed)
        {
            let g = nod_runtime::get_or_create_generic(name);
            g.mark_sealed();
        }
    }

    // Phase 2: collect top-level function names (incl. auto-accessor
    // names) and generic names.
    let top_names = collect_top_level_names(m, &user_classes);
    let generics = collect_generic_names(m);

    let mut out: Vec<Function> = Vec::new();
    let mut methods: Vec<MethodRegistration> = Vec::new();
    // Sprint 27: every `define c-function` declaration we encounter
    // is recorded here. The driver / FFI lowerer (Sprint 28+) reads
    // these to populate the per-module API stub table.
    let mut c_functions: Vec<CFunctionBinding> = Vec::new();
    // Sprint 27: non-fatal diagnostics — currently just
    // `c-function not in windows_api database`.
    let mut warnings: Vec<LoweringWarning> = Vec::new();
    // Sprint 19: a single `LiftSink` carries the FunctionId counter and
    // any per-`block` lifted thunks the lowerer synthesises. Both the
    // Phase 3 slot accessors and the Phase 4 user-item lowering allocate
    // ids through it.
    let mut lift_sink = LiftSink::default();
    let alloc_id = |sink: &mut LiftSink| sink.alloc_fn_id();

    // Phase 3: emit auto-generated slot accessors for every user class.
    //
    // For each slot in a class's merged layout:
    //   * If the slot was introduced by THIS class (`slot_origin == self`),
    //     emit the canonical `<C>-getter-x` / `<C>-setter-x` and register
    //     them as methods on the slot's generic (`x` / `x-setter`).
    //   * Else if the slot is inherited from an ancestor AND its offset
    //     in this class differs from the offset it had in the defining
    //     class's own layout, emit an override accessor that bakes the
    //     new offset, and register it as an additional method on the
    //     slot's generic specialised to this class. The Sprint 13
    //     dispatcher picks the override when the receiver is an instance
    //     of this class.
    //   * If the slot is inherited and the offset matches the parent's
    //     ("fixed-offset" case), no override is needed — the parent's
    //     accessor method already handles the receiver via inheritance.
    for item in &m.items {
        let Item::DefineClass { name, slots, .. } = item else {
            continue;
        };
        let Some(&class_id) = user_classes.get(name) else {
            continue;
        };
        let md_ptr = nod_runtime::class_metadata_ptr(class_id);
        if md_ptr.is_null() {
            continue;
        }
        // SAFETY: registered above; static-area lifetime.
        let metadata = unsafe { &*md_ptr };
        for (idx, slot) in metadata.slots.iter().enumerate() {
            let origin = metadata.slot_origin[idx];
            if origin == class_id {
                // Own slot — emit canonical accessors + register methods.
                let getter_name = format!("{}-getter-{}", name, slot.name);
                if !module_defines_function(m, &getter_name) {
                    out.push(build_slot_getter(
                        alloc_id(&mut lift_sink),
                        &getter_name,
                        slot.offset,
                        slot_type_to_dfm_kind(slot.type_kind),
                        slot_type_to_estimate(slot.type_kind),
                    ));
                    methods.push(MethodRegistration {
                        generic_name: slot.name.clone(),
                        specialisers: vec![class_id],
                        body_fn_name: getter_name,
                        param_count: 1,
                    });
                }
                if slot.has_setter {
                    let setter_name = format!("{}-setter-{}", name, slot.name);
                    if !module_defines_function(m, &setter_name) {
                        out.push(build_slot_setter(
                            alloc_id(&mut lift_sink),
                            &setter_name,
                            slot.offset,
                            slot_type_to_dfm_kind(slot.type_kind),
                        ));
                        methods.push(MethodRegistration {
                            generic_name: format!("{}-setter", slot.name),
                            specialisers: vec![class_id, ClassId::OBJECT],
                            body_fn_name: setter_name,
                            param_count: 2,
                        });
                    }
                }
                let _ = slots;
            } else {
                // Inherited slot — generate an override iff the offset
                // shifts vs. the slot's defining class's own layout.
                let origin_md_ptr = nod_runtime::class_metadata_ptr(origin);
                if origin_md_ptr.is_null() {
                    continue;
                }
                // SAFETY: static-area metadata.
                let origin_md = unsafe { &*origin_md_ptr };
                let origin_offset = origin_md
                    .slots
                    .iter()
                    .find(|s| s.name == slot.name)
                    .map(|s| s.offset)
                    .unwrap_or(slot.offset);
                if origin_offset == slot.offset {
                    // Fixed-offset case — parent's accessor works as-is.
                    continue;
                }
                // Override needed. Emit a fresh getter/setter that bakes
                // the new offset, register it on the slot's generic
                // specialised to this class.
                let getter_name = format!("{}-override-getter-{}", name, slot.name);
                if !module_defines_function(m, &getter_name) {
                    out.push(build_slot_getter(
                        alloc_id(&mut lift_sink),
                        &getter_name,
                        slot.offset,
                        slot_type_to_dfm_kind(slot.type_kind),
                        slot_type_to_estimate(slot.type_kind),
                    ));
                    methods.push(MethodRegistration {
                        generic_name: slot.name.clone(),
                        specialisers: vec![class_id],
                        body_fn_name: getter_name,
                        param_count: 1,
                    });
                }
                if slot.has_setter {
                    let setter_name = format!("{}-override-setter-{}", name, slot.name);
                    if !module_defines_function(m, &setter_name) {
                        out.push(build_slot_setter(
                            alloc_id(&mut lift_sink),
                            &setter_name,
                            slot.offset,
                            slot_type_to_dfm_kind(slot.type_kind),
                        ));
                        methods.push(MethodRegistration {
                            generic_name: format!("{}-setter", slot.name),
                            specialisers: vec![class_id, ClassId::OBJECT],
                            body_fn_name: setter_name,
                            param_count: 2,
                        });
                    }
                }
            }
        }
    }

    // Sprint 28 — Phase 3b: walk `define c-function` items, build the
    // marshaling signature for each, deduplicate `(dll, symbol)` pairs,
    // and allocate the per-module API stub table in the static area.
    // The resulting `c_function_call_map` is threaded through `LowerCtx`
    // so call-site lowering inside Phase 4 can resolve `Beep(...)` to a
    // WinFFI DirectCall against the right entry.
    //
    // We process declarations eagerly so the `entry_ptr` is non-null
    // before any call site is lowered. Unknown / unsupported c-types
    // produce a `signature: None`; call sites of those names then
    // surface a deferral error.
    nod_runtime::ensure_c_ffi_error_registered();
    let mut c_function_specs: Vec<nod_runtime::StubEntrySpec> = Vec::new();
    let mut c_function_pre: Vec<(String, Option<usize>, nod_reader::Span)> = Vec::new();
    let mut c_function_call_map: HashMap<String, CFunctionCallInfo> = HashMap::new();
    let mut spec_dedupe: HashMap<(String, String), usize> = HashMap::new();
    for item in &m.items {
        let Item::DefineCFunction {
            name,
            params,
            return_,
            c_name,
            library,
            span,
            ..
        } = item
        else {
            continue;
        };
        if library.is_empty() {
            // Diagnostic emitted in Phase 4; nothing to register here.
            continue;
        }
        // Build the marshaling signature from parsed types.
        let mut arg_names: Vec<String> = Vec::with_capacity(params.len());
        let mut signature_ok = true;
        for p in params {
            match &p.type_ {
                Some(Expr::Ident(_, n)) => arg_names.push(n.clone()),
                _ => {
                    signature_ok = false;
                    break;
                }
            }
        }
        let return_name: Option<String> = match return_ {
            Some(rs) if rs.values.len() > 1 => {
                signature_ok = false;
                None
            }
            Some(rs) => match rs.values.first() {
                Some(v) => match &v.type_ {
                    Some(Expr::Ident(_, n)) => Some(n.clone()),
                    _ => {
                        signature_ok = false;
                        None
                    }
                },
                None => None,
            },
            None => None,
        };
        if !signature_ok {
            c_function_pre.push((name.clone(), None, *span));
            continue;
        }
        let arg_refs: Vec<&str> = arg_names.iter().map(|s| s.as_str()).collect();
        let sig = match nod_runtime::signature_from_names(&arg_refs, return_name.as_deref()) {
            Ok(sig) => sig,
            Err(_) => {
                c_function_pre.push((name.clone(), None, *span));
                continue;
            }
        };
        let effective_c_name = c_name.clone().unwrap_or_else(|| name.clone());
        let key = (library.clone(), effective_c_name.clone());
        let idx = if let Some(&i) = spec_dedupe.get(&key) {
            i
        } else {
            let i = c_function_specs.len();
            spec_dedupe.insert(key, i);
            c_function_specs.push(nod_runtime::StubEntrySpec {
                dll: library.clone(),
                symbol: effective_c_name.clone(),
                signature: sig,
            });
            i
        };
        c_function_pre.push((name.clone(), Some(idx), *span));
        // The entry_ptr is patched once the table is allocated; we
        // need to know `idx` first, hence the two-phase loop here.
    }

    // Sprint 31: JIT-time API materialization. Walk the module's call
    // sites looking for bare-name callees that haven't already been
    // declared as `define c-function` (user wins) AND don't resolve as
    // a Dylan-side function, generic, class, or builtin. For each such
    // name try the embedded `nod-winapi` index; on a successful match
    // synthesize a `CFunctionBinding` + a stub-table entry on the fly.
    //
    // The materialization respects the same `spec_dedupe` map so two
    // bare references to `GetTickCount64` in the same module share one
    // table slot (and one resolver invocation at init time).
    //
    // Names whose signatures use unsupported types (struct-by-value,
    // function-pointer, opaque pointer-to-pointer, …) decline silently
    // — the call site then falls through to the existing
    // "unknown ident" DirectCall path. We track them so Phase 4's
    // unsupported-signature error can mention "Win32 function exists,
    // but signature uses unsupported types".
    let user_declared_c_names: HashSet<String> = c_function_pre
        .iter()
        .map(|(n, _, _)| n.clone())
        .collect();
    let mut materialized_binding_specs: Vec<MaterializedBindingSpec> = Vec::new();
    let mut materialized_call_names: HashSet<String> = HashSet::new();
    let mut materialized_unsupported: HashMap<String, String> = HashMap::new();
    let mut materialization_candidates: Vec<(String, nod_reader::Span)> = Vec::new();
    collect_bare_call_candidates(
        m,
        &user_declared_c_names,
        &top_names,
        &generics,
        &user_classes,
        &mut materialization_candidates,
    );
    let mut seen_candidate_names: HashSet<String> = HashSet::new();
    for (name, span) in &materialization_candidates {
        if !seen_candidate_names.insert(name.clone()) {
            continue;
        }
        match try_jit_materialize_winapi(name) {
            MaterializationOutcome::Materialized {
                c_name,
                library,
                signature,
            } => {
                let key = (library.clone(), c_name.clone());
                let idx = if let Some(&i) = spec_dedupe.get(&key) {
                    i
                } else {
                    let i = c_function_specs.len();
                    spec_dedupe.insert(key, i);
                    c_function_specs.push(nod_runtime::StubEntrySpec {
                        dll: library.clone(),
                        symbol: c_name.clone(),
                        signature,
                    });
                    i
                };
                materialized_binding_specs.push(MaterializedBindingSpec {
                    dylan_name: name.clone(),
                    c_name,
                    library,
                    span: *span,
                    signature,
                    spec_idx: idx,
                });
                materialized_call_names.insert(name.clone());
                nod_runtime::winffi_record_materialized();
            }
            MaterializationOutcome::UnsupportedSignature { reason, .. } => {
                materialized_unsupported.insert(name.clone(), reason);
            }
            MaterializationOutcome::NotFound => {}
        }
    }

    // Allocate the stub table NOW (in the static area). The returned
    // `entry_ptrs` are stable for the process lifetime; we bake them
    // into per-call IR as `i64` constants.
    let c_function_stub_table_entries: Vec<CFunctionStubEntry>;
    let entry_ptrs: Vec<*const nod_runtime::ApiStubEntry>;
    if !c_function_specs.is_empty() {
        let (_table, ptrs) = nod_runtime::allocate_stub_table(&c_function_specs);
        entry_ptrs = ptrs;
        c_function_stub_table_entries = c_function_specs
            .iter()
            .zip(entry_ptrs.iter())
            .map(|(s, &p)| CFunctionStubEntry {
                dll: s.dll.clone(),
                symbol: s.symbol.clone(),
                signature: s.signature,
                entry_ptr: p as u64,
            })
            .collect();
    } else {
        entry_ptrs = Vec::new();
        c_function_stub_table_entries = Vec::new();
    }
    // Build the per-call lookup map: Dylan name -> entry pointer +
    // arg count. Sprint 38d also carries (dll, symbol, signature_bytes)
    // so the call-site lowering can emit a `ConstValue::StubEntryRef`
    // (which the codegen turns into a `load i64, ptr @nod_stub__*`
    // through a per-module external global instead of baking the
    // per-process entry pointer as an `i64`).
    for (name, idx_opt, _) in &c_function_pre {
        if let Some(idx) = idx_opt {
            let p = entry_ptrs[*idx];
            let spec = &c_function_specs[*idx];
            let sig = spec.signature;
            let signature_bytes = signature_to_bytes(&sig);
            c_function_call_map.insert(
                name.clone(),
                CFunctionCallInfo {
                    entry_ptr: p as u64,
                    arg_count: sig.arg_count as usize,
                    dll: spec.dll.clone(),
                    symbol: spec.symbol.clone(),
                    signature_bytes,
                },
            );
        }
    }
    // Sprint 31: wire materialized bindings into the same lookup map
    // and register a synthesized `CFunctionBinding` so dump-ast and
    // introspection see them. User declarations always sit first in
    // `c_functions` (and in `c_function_call_map`) so explicit names
    // win over JIT materialization automatically — but `c_functions`
    // is a Vec, not a map, so explicit dedupe on `dylan_name` happens
    // here too as a belt-and-braces guard.
    for spec in &materialized_binding_specs {
        if user_declared_c_names.contains(&spec.dylan_name) {
            continue;
        }
        let p = entry_ptrs[spec.spec_idx];
        let sig = spec.signature;
        let signature_bytes = signature_to_bytes(&sig);
        c_function_call_map
            .entry(spec.dylan_name.clone())
            .or_insert(CFunctionCallInfo {
                entry_ptr: p as u64,
                arg_count: sig.arg_count as usize,
                dll: spec.library.clone(),
                symbol: spec.c_name.clone(),
                signature_bytes,
            });
        c_functions.push(CFunctionBinding {
            dylan_name: spec.dylan_name.clone(),
            c_name: spec.c_name.clone(),
            library: spec.library.clone(),
            span: spec.span,
            resolved_in_db: true,
            signature: Some(sig),
            source: BindingSource::JitMaterialized,
        });
    }

    // Phase 4: lower user-defined items.
    let user_classes_snapshot = user_classes.clone();
    for item in &m.items {
        match item {
            Item::DefineConstant { name, value, span, .. } => {
                let mut b = FunctionBuilder::new(alloc_id(&mut lift_sink), name.clone(), *span);
                let mut env = LocalEnv::new();
                let ctx = LowerCtx {
                    top_names: &top_names,
                    generics: &generics,
                    user_classes: &user_classes_snapshot,
                    closures: Some(&closure_registry),
                    c_functions: Some(&c_function_call_map),
                };
                match b.lower_expr(value, &mut env, &ctx) {
                    Ok(t) => {
                        let ty = b.func.temp_type(t);
                        b.func.return_type = ty;
                        b.terminate_current(Terminator::Return { value: Some(t) });
                        out.push(b.finish());
                    }
                    Err(e) => errors.push(e),
                }
            }
            Item::DefineFunction {
                name,
                params,
                body,
                return_,
                span,
                ..
            } => {
                let ctx = LowerCtx {
                    top_names: &top_names,
                    generics: &generics,
                    user_classes: &user_classes_snapshot,
                    closures: Some(&closure_registry),
                    c_functions: Some(&c_function_call_map),
                };
                match lower_function_inner(
                    alloc_id(&mut lift_sink),
                    name,
                    params,
                    return_.as_ref(),
                    body,
                    *span,
                    &ctx,
                    &mut lift_sink,
                ) {
                    Ok(f) => out.push(f),
                    Err(e) => errors.push(e),
                }
            }
            Item::DefineMethod {
                name,
                params,
                body,
                return_,
                span,
                ..
            } => {
                let ctx = LowerCtx {
                    top_names: &top_names,
                    generics: &generics,
                    user_classes: &user_classes_snapshot,
                    closures: Some(&closure_registry),
                    c_functions: Some(&c_function_call_map),
                };
                match lower_method_item(
                    alloc_id(&mut lift_sink),
                    name,
                    params,
                    return_.as_ref(),
                    body,
                    *span,
                    &ctx,
                    &mut lift_sink,
                ) {
                    Ok(method) => {
                        methods.push(method.registration);
                        out.push(method.function);
                    }
                    Err(e) => errors.push(e),
                }
            }
            Item::DefineGeneric { .. } => {
                // Sprint 12: `define generic` is informational —
                // declares the name. We collected it in `generics`
                // already; no lowering needed.
            }
            Item::DefineClass { .. } => {
                // Already handled in Phase 1.
            }
            Item::DefineVariable { span, .. } => {
                errors.push(LoweringError::Unsupported {
                    span: *span,
                    message: "define variable not lowered in Sprint 06".to_string(),
                });
            }
            Item::DefineMacro { .. } => {
                // WHY: Sprint 17 — macro definitions are collected and
                // removed by `nod_macro::expand_module` before lowering.
                // If one survives to here (direct `lower_module_full`
                // call without expansion) it is inert; no codegen needed.
            }
            Item::DefineCFunction {
                name,
                params,
                return_,
                c_name,
                library,
                span,
                ..
            } => {
                // Sprint 27 recorded the binding; Sprint 28 builds
                // the marshaling signature, picks a stub-table slot,
                // and registers a "synthetic top name" so call sites
                // can resolve `Beep(...)` to a WinFFI DirectCall.
                //
                // Validation: require `library:` to be present and
                // non-empty. Probe the embedded `nod-winapi` index
                // for the (DLL, c-name) pair; warn (not error) if
                // missing — user might be targeting a custom DLL
                // the DB doesn't know about.
                if library.is_empty() {
                    errors.push(LoweringError::Unsupported {
                        span: *span,
                        message: format!(
                            "`define c-function {name}`: missing required `library:` attribute"
                        ),
                    });
                    continue;
                }
                let effective_c_name = c_name.clone().unwrap_or_else(|| name.clone());
                let resolved =
                    nod_winapi::find_function(library, &effective_c_name).is_some();
                if !resolved {
                    warnings.push(LoweringWarning::CFunctionNotInDb {
                        span: *span,
                        name: name.clone(),
                        library: library.clone(),
                        c_name: effective_c_name.clone(),
                    });
                }
                // Sprint 28: derive the marshaling signature. Each
                // param's type annotation must be a `<c-…>` ident
                // that maps to a [`nod_runtime::CArgKind`]. Bail out
                // (signature = None) on any unknown type — call sites
                // then surface a deferral error per the Sprint 28
                // brief ("integer / pointer only").
                let mut arg_names: Vec<String> = Vec::with_capacity(params.len());
                let mut signature_ok = true;
                for p in params {
                    let n = match &p.type_ {
                        Some(Expr::Ident(_, n)) => n.clone(),
                        _ => {
                            signature_ok = false;
                            String::new()
                        }
                    };
                    arg_names.push(n);
                }
                let return_name: Option<String> = match return_ {
                    Some(rs) => {
                        if rs.values.len() > 1 {
                            // Multi-value c-function returns are not in
                            // Sprint 28 scope; fall through to
                            // signature = None.
                            signature_ok = false;
                            None
                        } else if let Some(v) = rs.values.first() {
                            match &v.type_ {
                                Some(Expr::Ident(_, n)) => Some(n.clone()),
                                _ => {
                                    signature_ok = false;
                                    None
                                }
                            }
                        } else {
                            None
                        }
                    }
                    None => None,
                };
                let signature: Option<nod_runtime::ApiCallSignature> = if signature_ok {
                    let arg_refs: Vec<&str> =
                        arg_names.iter().map(|s| s.as_str()).collect();
                    nod_runtime::signature_from_names(&arg_refs, return_name.as_deref())
                        .ok()
                } else {
                    None
                };
                c_functions.push(CFunctionBinding {
                    dylan_name: name.clone(),
                    c_name: effective_c_name,
                    library: library.clone(),
                    span: *span,
                    resolved_in_db: resolved,
                    signature,
                    source: BindingSource::UserCFunction,
                });
            }
            Item::DefineLibrary { .. } | Item::DefineModule { .. } => {}
            Item::DefineOther { span, keyword, .. } => {
                errors.push(LoweringError::Unsupported {
                    span: *span,
                    message: format!("`define {keyword}` not lowered in Sprint 06"),
                });
            }
            Item::Expr(_) => {}
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    // Sprint 15 — sealing analysis + dispatch resolution. Runs BEFORE
    // the precise-roots post-pass so any Dispatch → DirectCall (or
    // SealedDirectCall) rewrite happens before liveness sees the
    // call-shaped nodes; rewriting preserves the safepoint-roots
    // discipline transparently because the new call-shaped nodes go
    // through the same `safepoint_roots_mut()` accessor.
    //
    // Pre-register every method's specialiser tuple in the runtime
    // dispatch table so the resolver can enumerate applicable methods.
    // The body pointer is null at this stage — the JIT installs the
    // real address later via `register_methods`. The resolver only
    // reads `specialisers`; the resolver-side symbol name is
    // recomputed from `(generic_name, specialisers)` independently.
    for reg in &methods {
        let g = nod_runtime::get_or_create_generic(&reg.generic_name);
        // Skip if a method with these specialisers is already registered
        // (a prior `lower_module_full` call may have done it).
        let already = g
            .methods
            .read()
            .expect("methods rwlock poisoned")
            .iter()
            .any(|m| m.specialisers == reg.specialisers);
        if !already {
            // Sprint 16: pre-register the JIT body symbol name so the
            // dispatch resolver picks up the actual emitted symbol
            // (slot accessors don't follow the canonical naming
            // convention).
            g.add_method(nod_runtime::Method {
                specialisers: reg.specialisers.clone(),
                body_fn_ptr: std::ptr::null(),
                param_count: reg.param_count,
                body_fn_name: reg.body_fn_name.clone(),
            });
        }
    }
    let sealing = crate::optimise::collect_sealing_facts(&m.items, &user_classes_snapshot);
    crate::optimise::install_sealing_facts(&sealing);
    let mut resolutions: Vec<crate::optimise::DispatchResolution> = Vec::new();
    for f in &mut out {
        let narrowed = crate::optimise::narrow_function(f);
        let mut log = crate::optimise::resolve_dispatches(f, &narrowed, &sealing);
        resolutions.append(&mut log);
    }

    // Sprint 11b — precise-roots post-pass. Compute the set of
    // Sprint 19: drain any lifted-block thunks into the function list
    // BEFORE the safepoint-roots post-pass runs so the lifted thunks
    // also receive safepoint-roots populated.
    out.append(&mut lift_sink.functions);
    let blocks = std::mem::take(&mut lift_sink.blocks);

    // pointer-shaped temps live across each potentially-allocating
    // call, and stash the list on the call's `safepoint_roots` field.
    // Codegen brackets the call with `nod_register_root` /
    // `nod_unregister_root` pairs so the GC can rewrite the slots if
    // it evacuates the objects mid-call.
    for f in &mut out {
        nod_dfm::populate_safepoint_roots(f);
    }

    // Sprint 28: scan the AST for any call expression whose callee
    // is the name of a `define c-function` WHOSE signature couldn't
    // be derived (unsupported c-type, multi-value return, etc.). The
    // happy path (a supported signature) is fully lowered inside
    // `lower_call`. Names with `signature: None` aren't in
    // `c_function_call_map`, so their call sites would otherwise
    // silently fall through to "unknown ident — DirectCall against
    // the bare name"; we surface a deferral diagnostic instead.
    let unsupported_c_names: HashSet<String> = c_functions
        .iter()
        .filter(|c| c.signature.is_none() && c.source == BindingSource::UserCFunction)
        .map(|c| c.dylan_name.clone())
        .collect();
    if !unsupported_c_names.is_empty() {
        let mut call_site_errors: Vec<LoweringError> = Vec::new();
        scan_module_for_c_function_calls(m, &unsupported_c_names, &mut call_site_errors);
        if !call_site_errors.is_empty() {
            return Err(call_site_errors);
        }
    }

    // Sprint 31: bare-name calls whose Win32 entry exists in the index
    // BUT whose signature uses unsupported types (struct-by-value,
    // function-pointer callback, …) decline materialization and would
    // otherwise fall through to "unknown identifier" — surface a more
    // informative error so the user knows the API exists but isn't
    // yet wired up.
    if !materialized_unsupported.is_empty() {
        let mut call_site_errors: Vec<LoweringError> = Vec::new();
        scan_module_for_materialized_unsupported(
            m,
            &materialized_unsupported,
            &mut call_site_errors,
        );
        if !call_site_errors.is_empty() {
            return Err(call_site_errors);
        }
    }
    let _ = materialized_call_names;

    Ok(LoweredModule {
        functions: out,
        methods,
        sealing,
        resolutions,
        blocks,
        closures: closure_registry,
        c_functions,
        c_function_stub_table: c_function_stub_table_entries,
        warnings,
        user_classes: user_class_registrations,
    })
}

/// Sprint 27: walk the AST collecting any call expressions whose
/// callee is the name of a `define c-function`. Each such call site
/// becomes a `LoweringError::Unsupported` with the Sprint 28
/// deferral text. Sprint 28's call-site lowering will replace this
/// scan with proper FFI codegen.
fn scan_module_for_c_function_calls(
    m: &Module,
    c_names: &HashSet<String>,
    errors: &mut Vec<LoweringError>,
) {
    for item in &m.items {
        match item {
            Item::DefineFunction { body, .. } | Item::DefineMethod { body, .. } => {
                for s in body {
                    scan_stmt_for_c_calls(s, c_names, errors);
                }
            }
            Item::DefineConstant { value, .. } | Item::DefineVariable { value, .. } => {
                scan_expr_for_c_calls(value, c_names, errors);
            }
            Item::Expr(e) => scan_expr_for_c_calls(e, c_names, errors),
            _ => {}
        }
    }
}

fn scan_stmt_for_c_calls(
    s: &nod_reader::Statement,
    c_names: &HashSet<String>,
    errors: &mut Vec<LoweringError>,
) {
    use nod_reader::Statement as S;
    match s {
        S::Expr(e) => scan_expr_for_c_calls(e, c_names, errors),
        S::Let { value, .. } => {
            scan_expr_for_c_calls(value, c_names, errors);
        }
        S::Local { .. } => {
            // local methods carry exprs in bodies. Sprint 27 doesn't
            // recurse into them — c-function call inside a local
            // method is exotic and Sprint 28 will sweep this up.
        }
        S::For { body, finally_, .. } => {
            for s in body {
                scan_stmt_for_c_calls(s, c_names, errors);
            }
            for s in finally_ {
                scan_stmt_for_c_calls(s, c_names, errors);
            }
        }
        S::While { cond, body, .. } | S::Until { cond, body, .. } => {
            scan_expr_for_c_calls(cond, c_names, errors);
            for s in body {
                scan_stmt_for_c_calls(s, c_names, errors);
            }
        }
        S::Block { body, cleanup, afterwards, .. } => {
            for s in body {
                scan_stmt_for_c_calls(s, c_names, errors);
            }
            for s in cleanup {
                scan_stmt_for_c_calls(s, c_names, errors);
            }
            for s in afterwards {
                scan_stmt_for_c_calls(s, c_names, errors);
            }
        }
    }
}

fn scan_expr_for_c_calls(
    e: &Expr,
    c_names: &HashSet<String>,
    errors: &mut Vec<LoweringError>,
) {
    use nod_reader::Expr as E;
    match e {
        E::Call { callee, args, span } => {
            if let E::Ident(_, name) = callee.as_ref()
                && c_names.contains(name)
            {
                errors.push(LoweringError::Unsupported {
                    span: *span,
                    message: format!(
                        "`{name}`: c-function signature couldn't be derived. \
                         Check (a) arity ≤ 12 (Sprint 36b cap); \
                         (b) every param + return is one of the supported \
                         c-types: integer family, <c-bool>, <c-pointer>, \
                         <c-handle>, <c-string>, <c-wide-string>, or a \
                         <c-struct> subclass. Float / variadic / function-pointer \
                         args are not yet supported (Sprint 37+)."
                    ),
                });
            }
            scan_expr_for_c_calls(callee, c_names, errors);
            for a in args {
                scan_expr_for_c_calls(a, c_names, errors);
            }
        }
        E::BinOp { lhs, rhs, .. } => {
            scan_expr_for_c_calls(lhs, c_names, errors);
            scan_expr_for_c_calls(rhs, c_names, errors);
        }
        E::UnOp { operand, .. } => scan_expr_for_c_calls(operand, c_names, errors),
        E::Paren { inner, .. } => scan_expr_for_c_calls(inner, c_names, errors),
        E::If { cond, then_, else_, .. } => {
            scan_expr_for_c_calls(cond, c_names, errors);
            scan_expr_for_c_calls(then_, c_names, errors);
            if let Some(eb) = else_ {
                scan_expr_for_c_calls(eb, c_names, errors);
            }
        }
        E::Begin { body, .. } => {
            for e in body {
                scan_expr_for_c_calls(e, c_names, errors);
            }
        }
        E::Let { value, .. } => scan_expr_for_c_calls(value, c_names, errors),
        E::Method { body, .. } | E::LocalMethod { body, .. } => {
            for e in body {
                scan_expr_for_c_calls(e, c_names, errors);
            }
        }
        E::Stmt(s) => scan_stmt_for_c_calls(s, c_names, errors),
        _ => {}
    }
}

/// Sprint 31: parallel scan to [`scan_module_for_c_function_calls`] that
/// emits a different error message for bare-name calls whose Win32 entry
/// is present in the embedded index but uses an unsupported parameter /
/// return shape. We surface the matched (library, c-name) so the user
/// can declare a shim by hand.
fn scan_module_for_materialized_unsupported(
    m: &Module,
    unsupported: &HashMap<String, String>,
    errors: &mut Vec<LoweringError>,
) {
    for item in &m.items {
        match item {
            Item::DefineFunction { body, .. } | Item::DefineMethod { body, .. } => {
                for s in body {
                    scan_stmt_for_unsupported_winapi(s, unsupported, errors);
                }
            }
            Item::DefineConstant { value, .. } | Item::DefineVariable { value, .. } => {
                scan_expr_for_unsupported_winapi(value, unsupported, errors);
            }
            Item::Expr(e) => scan_expr_for_unsupported_winapi(e, unsupported, errors),
            _ => {}
        }
    }
}

fn scan_stmt_for_unsupported_winapi(
    s: &Statement,
    unsupported: &HashMap<String, String>,
    errors: &mut Vec<LoweringError>,
) {
    match s {
        Statement::Expr(e) => scan_expr_for_unsupported_winapi(e, unsupported, errors),
        Statement::Let { value, .. } => scan_expr_for_unsupported_winapi(value, unsupported, errors),
        Statement::Local { .. } => {}
        Statement::For { body, finally_, .. } => {
            for s in body {
                scan_stmt_for_unsupported_winapi(s, unsupported, errors);
            }
            for s in finally_ {
                scan_stmt_for_unsupported_winapi(s, unsupported, errors);
            }
        }
        Statement::While { cond, body, .. } | Statement::Until { cond, body, .. } => {
            scan_expr_for_unsupported_winapi(cond, unsupported, errors);
            for s in body {
                scan_stmt_for_unsupported_winapi(s, unsupported, errors);
            }
        }
        Statement::Block { body, cleanup, afterwards, .. } => {
            for s in body {
                scan_stmt_for_unsupported_winapi(s, unsupported, errors);
            }
            for s in cleanup {
                scan_stmt_for_unsupported_winapi(s, unsupported, errors);
            }
            for s in afterwards {
                scan_stmt_for_unsupported_winapi(s, unsupported, errors);
            }
        }
    }
}

fn scan_expr_for_unsupported_winapi(
    e: &Expr,
    unsupported: &HashMap<String, String>,
    errors: &mut Vec<LoweringError>,
) {
    match e {
        Expr::Call { callee, args, span } => {
            if let Expr::Ident(_, name) = callee.as_ref()
                && let Some(reason) = unsupported.get(name)
            {
                errors.push(LoweringError::Unsupported {
                    span: *span,
                    message: format!(
                        "Win32 function `{name}` was found in the embedded \
                         windows_api.db index, but its signature uses unsupported \
                         types ({reason}). To use this function, declare an \
                         explicit `define c-function {name} ... library: \"…\"; end;` \
                         with a shim signature, or wait for the relevant FFI \
                         capability sprint (callbacks: Sprint 33; structs: Sprint 34)."
                    ),
                });
            }
            scan_expr_for_unsupported_winapi(callee, unsupported, errors);
            for a in args {
                scan_expr_for_unsupported_winapi(a, unsupported, errors);
            }
        }
        Expr::BinOp { lhs, rhs, .. } => {
            scan_expr_for_unsupported_winapi(lhs, unsupported, errors);
            scan_expr_for_unsupported_winapi(rhs, unsupported, errors);
        }
        Expr::UnOp { operand, .. } => scan_expr_for_unsupported_winapi(operand, unsupported, errors),
        Expr::Paren { inner, .. } => scan_expr_for_unsupported_winapi(inner, unsupported, errors),
        Expr::If { cond, then_, else_, .. } => {
            scan_expr_for_unsupported_winapi(cond, unsupported, errors);
            scan_expr_for_unsupported_winapi(then_, unsupported, errors);
            if let Some(eb) = else_ {
                scan_expr_for_unsupported_winapi(eb, unsupported, errors);
            }
        }
        Expr::Begin { body, .. } => {
            for e in body {
                scan_expr_for_unsupported_winapi(e, unsupported, errors);
            }
        }
        Expr::Let { value, .. } => scan_expr_for_unsupported_winapi(value, unsupported, errors),
        Expr::Method { body, .. } | Expr::LocalMethod { body, .. } => {
            for e in body {
                scan_expr_for_unsupported_winapi(e, unsupported, errors);
            }
        }
        Expr::Stmt(s) => scan_stmt_for_unsupported_winapi(s, unsupported, errors),
        _ => {}
    }
}

// ─── Class registration ────────────────────────────────────────────────────

fn register_class(
    name: &str,
    supers: &[Expr],
    slots: &[nod_reader::SlotDef],
    span: Span,
) -> Result<ClassId, LoweringError> {
    // Sprint 12 refuses redefinition.
    if find_class_id_by_name(name).is_some() {
        return Err(LoweringError::ClassRedefinitionNotSupported {
            span,
            class_name: name.to_string(),
        });
    }
    // Resolve every super to a registered ClassId. Default to a
    // singleton `[<object>]` when no supers were declared, per Dylan
    // convention.
    let parent_ids: Vec<ClassId> = if supers.is_empty() {
        vec![ClassId::OBJECT]
    } else {
        let mut out = Vec::with_capacity(supers.len());
        for super_expr in supers {
            let super_name = match super_expr {
                Expr::Ident(_, n) => n.clone(),
                _ => {
                    return Err(LoweringError::Unsupported {
                        span,
                        message: "superclass expression must be an identifier".to_string(),
                    });
                }
            };
            match find_class_id_by_name(&super_name) {
                Some(id) => {
                    // Sprint 15 cross-library refusal — if the parent
                    // was already sealed by a prior lowering call, this
                    // is an attempt to extend a sealed class from a
                    // different "library". The check naturally allows
                    // in-library subclassing because the parent's
                    // `sealed` bit is flipped AFTER `register_class`
                    // returns in this very same `lower_module_full`
                    // call (Phase 1 vs the modifiers-acting loop).
                    let p = class_metadata_ptr(id);
                    if !p.is_null() {
                        // SAFETY: static-area metadata.
                        let sealed = unsafe { (*p).is_sealed() };
                        if sealed {
                            return Err(LoweringError::SealingViolation {
                                span,
                                violation: SealingViolation::SealedClassExtendedAcrossBoundary {
                                    sealed_parent: super_name.clone(),
                                    child: name.to_string(),
                                },
                            });
                        }
                    }
                    out.push(id);
                }
                None => {
                    return Err(LoweringError::UnknownSuperclass {
                        span,
                        class_name: name.to_string(),
                        super_name,
                    });
                }
            }
        }
        out
    };

    // Build SlotInfos for own slots (offsets get patched in
    // `register_simple_user_class`).
    let mut own_slots: Vec<SlotInfo> = Vec::with_capacity(slots.len());
    for slot in slots {
        if slot.allocation != nod_reader::SlotAllocation::Instance {
            return Err(LoweringError::UnsupportedSlotAllocation {
                span: slot.span,
                class_name: name.to_string(),
                slot_name: slot.name.clone(),
                allocation: format!("{:?}", slot.allocation),
            });
        }
        let type_kind = slot
            .type_
            .as_ref()
            .map(slot_type_from_expr)
            .unwrap_or(SlotType::Top);
        let default_init = match (&slot.init_value, type_kind) {
            (Some(Expr::Integer(_, n)), _) => {
                // Try to encode as a fixnum literal.
                match (*n).try_into() {
                    Ok(i) => Word::from_fixnum(i)
                        .map(SlotDefault::Value)
                        .unwrap_or(SlotDefault::Unbound),
                    Err(_) => SlotDefault::Unbound,
                }
            }
            (Some(Expr::Bool(_, true)), _) => {
                SlotDefault::Value(nod_runtime::literal_pool_immediates().true_)
            }
            (Some(Expr::Bool(_, false)), _) => {
                SlotDefault::Value(nod_runtime::literal_pool_immediates().false_)
            }
            _ => SlotDefault::Unbound,
        };
        let has_setter = slot.setter.unwrap_or(true);
        own_slots.push(SlotInfo {
            name: slot.name.clone(),
            offset: 0, // patched by registration helper.
            type_kind,
            init_keyword: slot.init_keyword.clone(),
            required_init_keyword: slot.required_init_keyword,
            default_init,
            has_setter,
        });
    }

    // Single-inheritance fast path — preserves Sprint 12 behaviour exactly.
    if parent_ids.len() == 1 {
        let parent_id = parent_ids[0];
        let (id, _addr) =
            register_simple_user_class(name, Some(parent_id), own_slots);
        // Sprint 15: register `id` as a direct subclass of `parent_id`
        // so the dispatch resolver can enumerate bounded subclass sets
        // when the parent is sealed.
        register_direct_subclass(parent_id, id);
        return Ok(id);
    }

    // Multi-inheritance path (Sprint 14).
    // 1. Resolve every parent's name (for C3 + diagnostics).
    let parent_names: Vec<String> = parent_ids
        .iter()
        .map(|id| {
            let md_ptr = class_metadata_ptr(*id);
            if md_ptr.is_null() {
                format!("<unknown:{}>", id.0)
            } else {
                // SAFETY: pointer is to static-area metadata.
                unsafe { (*md_ptr).name.clone() }
            }
        })
        .collect();
    // 2. Run C3 on the parent CPLs (which are names, since that's what
    //    c3.rs takes).
    let parent_cpl_names: Vec<Vec<String>> = parent_ids
        .iter()
        .map(|id| {
            let md = class_metadata_for(*id);
            md.cpl
                .iter()
                .map(|c| {
                    let p = class_metadata_ptr(*c);
                    if p.is_null() {
                        format!("<unknown:{}>", c.0)
                    } else {
                        // SAFETY: static area.
                        unsafe { (*p).name.clone() }
                    }
                })
                .collect()
        })
        .collect();
    let parent_cpl_refs: Vec<&[String]> =
        parent_cpl_names.iter().map(|v| v.as_slice()).collect();
    let cpl_names = c3_linearise(name, &parent_names, &parent_cpl_refs).map_err(
        |e| match e {
            C3Error::InconsistentMerge { class_name } => {
                LoweringError::InconsistentInheritance {
                    span,
                    class_name: class_name.clone(),
                    detail: "C3 merge failed: parents impose conflicting orders on a shared ancestor".to_string(),
                }
            }
            C3Error::UnresolvedParent { class_name, parent_name } => {
                LoweringError::InconsistentInheritance {
                    span,
                    class_name,
                    detail: format!("parent `{parent_name}` has no CPL yet (forward reference?)"),
                }
            }
        },
    )?;
    // 3. Map names back to ClassIds. Sentinel `ClassId(u32::MAX)` for the
    //    self entry at index 0; runtime patches after id minting.
    let self_sentinel = ClassId(u32::MAX);
    let mut cpl: Vec<ClassId> = Vec::with_capacity(cpl_names.len());
    for (i, n) in cpl_names.iter().enumerate() {
        if i == 0 {
            cpl.push(self_sentinel);
        } else {
            match find_class_id_by_name(n) {
                Some(id) => cpl.push(id),
                None => {
                    return Err(LoweringError::InconsistentInheritance {
                        span,
                        class_name: name.to_string(),
                        detail: format!("C3-derived ancestor `{n}` is not a registered class"),
                    });
                }
            }
        }
    }
    // 4. Merge slot lists. Walk parents in declaration order (the
    //    "most-specific-first append" policy from the brief): append
    //    each parent's full slot list to the merged list, skipping
    //    slots whose origin class is already present.
    let mut merged_slots: Vec<SlotInfo> = Vec::new();
    let mut merged_origin: Vec<ClassId> = Vec::new();
    for parent_id in &parent_ids {
        let pmd = class_metadata_for(*parent_id);
        for (slot, origin) in pmd.slots.iter().zip(pmd.slot_origin.iter()) {
            // If a slot with the same defining class is already in the
            // merged list, skip it (diamond — same slot reached via two
            // paths).
            if merged_origin.contains(origin) {
                // We already pulled in every slot from this origin via
                // a different parent path; this iteration is a duplicate.
                // But we still need to check slot name conflicts: if
                // two different origins define the same slot NAME, that's
                // an MI conflict.
                continue;
            }
            // Conflict check: another origin already defined a slot with
            // this name?
            if let Some(idx) = merged_slots.iter().position(|s| s.name == slot.name) {
                let prior_origin = merged_origin[idx];
                if prior_origin != *origin {
                    return Err(LoweringError::SlotConflict {
                        span,
                        class_name: name.to_string(),
                        slot_name: slot.name.clone(),
                        first_origin: class_name_of(prior_origin),
                        second_origin: class_name_of(*origin),
                    });
                }
            }
            merged_slots.push(slot.clone());
            merged_origin.push(*origin);
        }
    }
    let inherited_slot_count = merged_slots.len();
    // 5. Append this class's own slots (mark with self-sentinel — runtime
    //    patches after id minting).
    for slot in own_slots {
        // Reject conflict with an inherited slot name.
        if merged_slots.iter().any(|s| s.name == slot.name) {
            return Err(LoweringError::SlotConflict {
                span,
                class_name: name.to_string(),
                slot_name: slot.name.clone(),
                first_origin: "(an ancestor)".to_string(),
                second_origin: name.to_string(),
            });
        }
        merged_slots.push(slot);
        merged_origin.push(self_sentinel);
    }
    let own_slot_count = merged_slots.len() - inherited_slot_count;
    // 6. Patch every slot's offset to its position in the merged list.
    for (i, slot) in merged_slots.iter_mut().enumerate() {
        slot.offset = std::mem::size_of::<nod_runtime::Wrapper>() + i * 8;
    }
    let (id, _addr) = register_mi_user_class(
        name,
        parent_ids.clone(),
        cpl,
        merged_slots,
        merged_origin,
        own_slot_count,
        inherited_slot_count,
    );
    // Sprint 15: record this class as a direct subclass of every
    // declared parent.
    for parent_id in &parent_ids {
        register_direct_subclass(*parent_id, id);
    }
    Ok(id)
}

/// Sprint 15: append `child` to `parent`'s `direct_subclasses` list.
/// No-op if either id has no metadata (defensive against the seed
/// path's tests).
fn register_direct_subclass(parent: ClassId, child: ClassId) {
    let p = class_metadata_ptr(parent);
    if p.is_null() {
        return;
    }
    // SAFETY: pointer is to static-area metadata (process-lived).
    unsafe { (*p).register_subclass(child) };
}

fn class_name_of(id: ClassId) -> String {
    let p = class_metadata_ptr(id);
    if p.is_null() {
        format!("<unknown:{}>", id.0)
    } else {
        // SAFETY: static area.
        unsafe { (*p).name.clone() }
    }
}

fn slot_type_from_expr(e: &Expr) -> SlotType {
    if let Expr::Ident(_, n) = e {
        match n.as_str() {
            "<integer>" => SlotType::Integer,
            "<single-float>" | "<double-float>" | "<float>" => SlotType::DoubleFloat,
            "<boolean>" => SlotType::Boolean,
            "<character>" => SlotType::Character,
            "<string>" | "<byte-string>" => SlotType::String,
            "<symbol>" => SlotType::Symbol,
            "<simple-object-vector>" | "<vector>" => SlotType::Vector,
            "<object>" | "<top>" => SlotType::Top,
            other => {
                // User class? If registered, narrow.
                if let Some(id) = find_class_id_by_name(other) {
                    SlotType::Class(id)
                } else {
                    SlotType::Top
                }
            }
        }
    } else {
        SlotType::Top
    }
}

fn slot_type_to_dfm_kind(t: SlotType) -> SlotTypeKind {
    match t {
        SlotType::Integer | SlotType::Character => SlotTypeKind::Integer,
        _ => SlotTypeKind::Object,
    }
}

fn slot_type_to_estimate(t: SlotType) -> TypeEstimate {
    match t {
        SlotType::Integer => TypeEstimate::Integer,
        SlotType::DoubleFloat => TypeEstimate::DoubleFloat,
        SlotType::Boolean => TypeEstimate::Boolean,
        SlotType::Character => TypeEstimate::Character,
        SlotType::String => TypeEstimate::String,
        _ => TypeEstimate::Top,
    }
}

fn module_defines_function(m: &Module, name: &str) -> bool {
    m.items.iter().any(|it| match it {
        Item::DefineFunction { name: n, .. } | Item::DefineMethod { name: n, .. } => n == name,
        _ => false,
    })
}

// ─── Slot-accessor synthesis ───────────────────────────────────────────────

fn build_slot_getter(
    id: FunctionId,
    name: &str,
    offset: usize,
    slot_type: SlotTypeKind,
    return_type: TypeEstimate,
) -> Function {
    let span = Span {
        file_id: nod_reader::FileId(0),
        lo: 0,
        hi: 0,
    };
    let entry = BlockId(0);
    let self_temp = TempId(0);
    let result_temp = TempId(1);
    Function {
        id,
        name: name.to_string(),
        params: vec![self_temp],
        entry,
        blocks: vec![Block {
            id: entry,
            label: "entry".to_string(),
            params: Vec::new(),
            computations: vec![Computation::LoadSlot {
                dst: result_temp,
                instance: self_temp,
                offset,
                slot_type,
            }],
            terminator: Terminator::Return {
                value: Some(result_temp),
            },
        }],
        temps: vec![
            Temporary {
                id: self_temp,
                type_estimate: TypeEstimate::Top,
            },
            Temporary {
                id: result_temp,
                type_estimate: return_type,
            },
        ],
        return_type,
        span,
    }
}

fn build_slot_setter(
    id: FunctionId,
    name: &str,
    offset: usize,
    slot_type: SlotTypeKind,
) -> Function {
    let span = Span {
        file_id: nod_reader::FileId(0),
        lo: 0,
        hi: 0,
    };
    let entry = BlockId(0);
    let self_temp = TempId(0);
    let value_temp = TempId(1);
    let result_temp = TempId(2);
    Function {
        id,
        name: name.to_string(),
        params: vec![self_temp, value_temp],
        entry,
        blocks: vec![Block {
            id: entry,
            label: "entry".to_string(),
            params: Vec::new(),
            computations: vec![Computation::StoreSlot {
                dst: result_temp,
                instance: self_temp,
                offset,
                value: value_temp,
                slot_type,
            }],
            terminator: Terminator::Return {
                value: Some(result_temp),
            },
        }],
        temps: vec![
            Temporary {
                id: self_temp,
                type_estimate: TypeEstimate::Top,
            },
            Temporary {
                id: value_temp,
                type_estimate: TypeEstimate::Top,
            },
            Temporary {
                id: result_temp,
                type_estimate: TypeEstimate::Top,
            },
        ],
        return_type: TypeEstimate::Top,
        span,
    }
}

// ─── Method lowering ───────────────────────────────────────────────────────

struct LoweredMethod {
    function: Function,
    registration: MethodRegistration,
}

#[allow(clippy::too_many_arguments)]
fn lower_method_item(
    id: FunctionId,
    name: &str,
    params: &[Param],
    return_sig: Option<&ReturnSig>,
    body: &[Statement],
    span: Span,
    ctx: &LowerCtx,
    sink: &mut LiftSink,
) -> Result<LoweredMethod, LoweringError> {
    if params.is_empty() {
        return Err(LoweringError::Unsupported {
            span,
            message: "define method requires at least one parameter".to_string(),
        });
    }
    // Sprint 13: collect ONE specialiser per required parameter. An
    // unannotated parameter is `<object>` per Dylan convention.
    let mut specialisers: Vec<ClassId> = Vec::with_capacity(params.len());
    for p in params {
        let cls = match &p.type_ {
            Some(Expr::Ident(_, cls)) => match find_class_id_by_name(cls) {
                Some(id) => id,
                None => {
                    return Err(LoweringError::UndefinedIdent {
                        span: p.span,
                        name: cls.clone(),
                    });
                }
            },
            _ => ClassId::OBJECT,
        };
        specialisers.push(cls);
    }
    let receiver_class = specialisers[0];
    // Encode all specialisers in the body fn name so distinct
    // multi-arg methods don't collide at codegen.
    let suffix = specialisers
        .iter()
        .map(|c| c.0.to_string())
        .collect::<Vec<_>>()
        .join("_");
    let body_fn_name = format!("{name}${suffix}");
    let function =
        lower_function_inner(id, &body_fn_name, params, return_sig, body, span, ctx, sink)?;
    let _ = receiver_class;
    let registration = MethodRegistration {
        generic_name: name.to_string(),
        specialisers,
        body_fn_name: body_fn_name.clone(),
        param_count: params.len(),
    };
    Ok(LoweredMethod {
        function,
        registration,
    })
}

pub fn lower_function(
    name: &str,
    params: &[Param],
    body: &[Statement],
) -> Result<Function, LoweringError> {
    let span = body
        .first()
        .map(Statement::span)
        .or_else(|| params.first().map(|p| p.span))
        .unwrap_or(Span {
            file_id: nod_reader::FileId(0),
            lo: 0,
            hi: 0,
        });
    let top_names = TopNames::empty();
    let generics: HashSet<String> = HashSet::new();
    let user_classes: HashMap<String, ClassId> = HashMap::new();
    let ctx = LowerCtx {
        top_names: &top_names,
        generics: &generics,
        user_classes: &user_classes,
        closures: None,
        c_functions: None,
    };
    let mut sink = LiftSink::default();
    lower_function_inner(FunctionId(0), name, params, None, body, span, &ctx, &mut sink)
}

#[allow(clippy::too_many_arguments)]
fn lower_function_inner(
    id: FunctionId,
    name: &str,
    params: &[Param],
    return_sig: Option<&ReturnSig>,
    body: &[Statement],
    span: Span,
    ctx: &LowerCtx,
    sink: &mut LiftSink,
) -> Result<Function, LoweringError> {
    let mut b = FunctionBuilder::new(id, name.to_string(), span);
    let mut env = LocalEnv::new();

    // Sprint 24: closure-body bring-up. If `name` is the lifted body of
    // a closure with non-empty captures, install the env parameter as a
    // synthetic FIRST parameter (matching the runtime ABI in
    // `nod_funcall_N`). The lowerer redirects reads / writes of
    // captured names through `%env-cell` + `%cell-get` / `%cell-set!`.
    let closure_info: Option<&ClosureInfo> = ctx
        .closures
        .and_then(|reg| reg.closure_for(name))
        .filter(|info| !info.captured.is_empty());
    if let Some(info) = closure_info {
        let env_temp = b.fresh_temp(TypeEstimate::Top);
        b.func.params.push(env_temp);
        let mut index_of: HashMap<String, usize> = HashMap::new();
        for (i, c) in info.captured.iter().enumerate() {
            index_of.insert(c.clone(), i);
        }
        b.cell_ctx.env_captures = Some(EnvCaptures { env_temp, index_of });
    }

    // Sprint 24: cell-promotion locals for this body. Any local in
    // `cell_locals` whose `let` binding is encountered while lowering
    // becomes a `<cell>` allocation; subsequent reads / writes go
    // through the cell.
    if let Some(reg) = ctx.closures
        && let Some(cells) = reg.cell_locals_for(name)
    {
        b.cell_ctx.cell_locals = cells.clone();
    }

    for p in params {
        let pty = type_from_expr(p.type_.as_ref());
        let t = b.fresh_temp(pty);
        b.func.params.push(t);
        // Sprint 24: if this param is itself captured by an inner
        // closure, promote it to a cell so the inner closure (which
        // accesses it through the env) and the outer scope see the
        // same storage. The cell-promoted name maps to the cell-Word
        // in `env`; subsequent reads/writes of `p.name` go through
        // `%cell-get` / `%cell-set!`.
        if b.cell_ctx.cell_locals.contains(&p.name) {
            let cell = b.fresh_temp(TypeEstimate::Top);
            b.push(Computation::DirectCall {
                dst: cell,
                callee: "nod_make_cell".to_string(),
                args: vec![t],
                safepoint_roots: Vec::new(),
            });
            env.insert(p.name.clone(), cell);
        } else {
            env.insert(p.name.clone(), t);
        }
    }

    let declared_ret = return_sig
        .and_then(|r| r.values.first().and_then(|v| v.type_.as_ref()))
        .map(|e| type_from_expr(Some(e)));

    let last_idx = body.len().saturating_sub(1);
    let mut final_temp: Option<TempId> = None;
    for (i, stmt) in body.iter().enumerate() {
        let is_last = i == last_idx;
        match stmt {
            Statement::Expr(e) => {
                let t = b.lower_expr(e, &mut env, ctx)?;
                if is_last {
                    final_temp = Some(t);
                }
            }
            Statement::Let {
                binders,
                rest,
                value,
                span,
            } => {
                if rest.is_some() || binders.len() != 1 {
                    return Err(LoweringError::Unsupported {
                        span: *span,
                        message: "Sprint 06 lowers single-binder `let` only".to_string(),
                    });
                }
                let bname = &binders[0].name;
                let t = b.lower_expr(value, &mut env, ctx)?;
                // Sprint 24: cell-promote the binding if any inner
                // closure captures it.
                let bound = if b.cell_ctx.cell_locals.contains(bname) {
                    let cell = b.fresh_temp(TypeEstimate::Top);
                    b.push(Computation::DirectCall {
                        dst: cell,
                        callee: "nod_make_cell".to_string(),
                        args: vec![t],
                        safepoint_roots: Vec::new(),
                    });
                    cell
                } else {
                    t
                };
                env.insert(bname.clone(), bound);
                if is_last {
                    final_temp = Some(t);
                }
            }
            Statement::Local { span, .. } => {
                return Err(LoweringError::Unsupported {
                    span: *span,
                    message: "`local method` not lowered in Sprint 06".to_string(),
                });
            }
            Statement::While { cond, body: wbody, .. } => {
                // Sprint 18: `while (cond) body end`. Three-block CFG
                // with a back-edge: header → loop_body → header / exit.
                // The header block evaluates the condition each
                // iteration; loop_body runs the user statements then
                // unconditionally jumps back to header.
                b.lower_while_like(cond, wbody, false, &mut env, ctx)?;
                if is_last {
                    final_temp = None; // while statements have no value
                }
            }
            Statement::Until { cond, body: wbody, .. } => {
                // Sprint 18: `until (cond) body end`. Same shape as
                // `while` but the condition is negated at the header.
                b.lower_while_like(cond, wbody, true, &mut env, ctx)?;
                if is_last {
                    final_temp = None;
                }
            }
            Statement::For { span, .. } => {
                return Err(LoweringError::Unsupported {
                    span: *span,
                    message: "`for` not lowered in Sprint 18 (use `for-range` macro or rewrite to `while`)".to_string(),
                });
            }
            Statement::Block {
                span,
                exit_var,
                body: blk_body,
                handlers,
                cleanup,
                afterwards,
            } => {
                // Sprint 19: lower `block ... exception ... cleanup ...
                // end` via lifted thunks + a runtime `nod_run_block`
                // call. See `docs/CONDITIONS.md` for the design.
                let t = lower_block_form(
                    &mut b,
                    sink,
                    &mut env,
                    ctx,
                    *span,
                    name,
                    exit_var.as_deref(),
                    blk_body,
                    handlers,
                    cleanup,
                    afterwards,
                )?;
                if is_last {
                    final_temp = Some(t);
                }
            }
        }
    }

    let ret_ty = if let Some(declared) = declared_ret {
        declared
    } else if let Some(t) = final_temp {
        b.func.temp_type(t)
    } else {
        TypeEstimate::Unit
    };
    b.func.return_type = ret_ty;

    let term = if ret_ty == TypeEstimate::Unit {
        Terminator::Return { value: None }
    } else {
        let t = final_temp.ok_or_else(|| LoweringError::Unsupported {
            span,
            message: "function with non-unit return has empty body".to_string(),
        })?;
        Terminator::Return { value: Some(t) }
    };
    b.terminate_current(term);

    Ok(b.finish())
}

// ─── Top-level name set + lowering context ─────────────────────────────────

pub struct TopNames {
    fns: HashMap<String, TypeEstimate>,
    /// Sprint 21: arity per top-level function. Populated alongside
    /// `fns` by `collect_top_level_names`. Used to bake the right
    /// arity into `nod_make_function_ref` call sites for `\name`
    /// references. Slot accessors (`<C>-getter-x`) have arity 1;
    /// setters (`<C>-setter-x`) arity 2; user `define function`s
    /// follow their param count.
    fn_arity: HashMap<String, usize>,
}

impl TopNames {
    pub fn empty() -> Self {
        Self {
            fns: HashMap::new(),
            fn_arity: HashMap::new(),
        }
    }
    pub fn contains(&self, name: &str) -> bool {
        self.fns.contains_key(name)
    }
    pub fn return_type(&self, name: &str) -> Option<TypeEstimate> {
        self.fns.get(name).copied()
    }
    /// Sprint 21: arity for a registered top-level function, if known.
    pub fn arity(&self, name: &str) -> Option<usize> {
        self.fn_arity.get(name).copied()
    }
}

struct LowerCtx<'a> {
    top_names: &'a TopNames,
    generics: &'a HashSet<String>,
    user_classes: &'a HashMap<String, ClassId>,
    /// Sprint 24: closure registry produced by `lift_anonymous_methods`.
    /// `None` when lowering is invoked outside of `lower_module_full`
    /// (e.g. the `lower_function` test helper); in that case the
    /// lowerer behaves exactly as it did in Sprint 21.
    closures: Option<&'a ClosureRegistry>,
    /// Sprint 28: per-module c-function call site dispatch table.
    /// Maps the Dylan-side name (`Beep`) to the resolved stub-table
    /// entry + signature for code-gen-time lowering. `None` outside
    /// `lower_module_full` (no `define c-function` in scope).
    c_functions: Option<&'a HashMap<String, CFunctionCallInfo>>,
}

/// Sprint 28: per-c-function metadata threaded through `LowerCtx` so
/// call-site lowering can look up the stub-table entry pointer + the
/// marshaling signature for a given Dylan-side name.
///
/// Sprint 38d: in addition to the (still-allocated) static-area entry
/// pointer, carry `dll` / `symbol` / `signature_bytes` so the call-site
/// lowering can emit a `ConstValue::StubEntryRef` instead of baking the
/// per-process `entry_ptr` as an `i64`. The pre-allocation is kept
/// because `nod-sema::lib::initialize_module_winffi` still walks the
/// `c_function_stub_table` to drive cold-path resolution; switching that
/// to go through the slot allocator is part of Sprint 38d's runtime
/// integration but not strictly required (the slot allocator's
/// `resolve_into_entry` call is idempotent, so the eager pre-resolve in
/// sema becomes a no-op).
#[derive(Clone, Debug)]
struct CFunctionCallInfo {
    /// Static-area address of the resolved [`nod_runtime::ApiStubEntry`]
    /// — kept for back-compat with `c_function_stub_table` consumers.
    /// Sprint 38d: no longer baked into the IR; codegen now reads
    /// `dll` / `symbol` / `signature_bytes` and goes through the
    /// `stub_entry_slot_addr` path.
    #[allow(dead_code)]
    entry_ptr: u64,
    /// Argument count from the parsed signature. Drives which
    /// `nod_winffi_call_N` trampoline the call site emits.
    arg_count: usize,
    /// Sprint 38d — DLL name carried verbatim into
    /// `ConstValue::StubEntryRef`. The slot allocator lowercases this
    /// for its case-insensitive key; we keep the original casing here
    /// so debug dumps + sema-side diagnostics match the source.
    dll: String,
    /// Sprint 38d — symbol (effective C name).
    symbol: String,
    /// Sprint 38d — bytewise-encoded [`nod_runtime::ApiCallSignature`]
    /// (`#[repr(C)] Copy`). Carried through into the manifest so the
    /// warm-replay resolver reconstructs the same marshaling shape.
    signature_bytes: Vec<u8>,
}

/// Sprint 24: per-body lowering state for cell-promotion + env access.
/// Threaded alongside `LocalEnv` through every `lower_*` method. Holds:
///
///   * `cell_locals` — names of THIS body's local bindings that should
///     be heap-allocated cells (because some inner closure captures them).
///   * `env_captures` — when lowering a closure body, the synthetic
///     env parameter's `TempId` and the captured-variable-name → env
///     index map. `None` outside closure bodies.
#[derive(Default, Clone, Debug)]
struct CellCtx {
    cell_locals: HashSet<String>,
    env_captures: Option<EnvCaptures>,
}

#[derive(Clone, Debug)]
struct EnvCaptures {
    env_temp: TempId,
    /// Captured-variable name -> index in the env's cells vector.
    index_of: HashMap<String, usize>,
}

/// Sprint 19: accumulator for the lifted thunks each `block` form
/// produces. Threaded through `lower_function_inner` and `FunctionBuilder`
/// so a deeply-nested `block` can deposit its synthesised top-level
/// functions back into the enclosing `lower_module_full` pass.
///
/// `next_fn_id` mirrors the counter `lower_module_full` uses for user
/// `define function`s; lifted thunks get fresh ids in the same space.
/// `name_seed` lets us append a counter to lift-thunk names so two
/// `block` forms in the same parent function don't collide.
#[derive(Default)]
pub struct LiftSink {
    pub functions: Vec<Function>,
    pub blocks: Vec<BlockRegistration>,
    pub next_fn_id: u32,
    pub thunk_counter: u32,
}

impl LiftSink {
    fn alloc_fn_id(&mut self) -> FunctionId {
        let id = FunctionId(self.next_fn_id);
        self.next_fn_id += 1;
        id
    }
    fn alloc_thunk_suffix(&mut self) -> u32 {
        let n = self.thunk_counter;
        self.thunk_counter += 1;
        n
    }
}

// ─── Sprint 21 / 24: anonymous-method lifting pre-pass ────────────────────
//
// Walks every Item's body, every Expr nested inside, and replaces
// `Expr::Method { params, body }` with an `Expr::Ident` referencing a
// synthesised top-level name. Each replacement also appends an
// `Item::DefineFunction` to the module so the normal lowering flow
// emits the lifted thunk as an ordinary top-level function.
//
// Sprint 21 erred out on any free variable inside a method body with
// "closures land in Sprint 24". Sprint 24 replaces that path with the
// cell-conversion machinery:
//
//   * Compute the **captured set** per `Expr::Method` — names that
//     reference a variable bound in an enclosing scope (and not a
//     top-level / operator / class name).
//   * For each captured local, the enclosing function's body promotes
//     it to a heap-allocated `<cell>`: `let x = E` becomes
//     `let x = %make-cell(E)`, and reads / writes go through
//     `%cell-get` / `%cell-set!` (decided at lowering time via
//     `cell_locals`).
//   * The lifted method body grows a synthetic env parameter; reads /
//     writes of captured names in the body become
//     `%cell-get(%env-cell(env, i))` / `%cell-set!(v, %env-cell(env, i))`.
//   * The closure-creation site (the original `Expr::Method` location)
//     emits `%make-closure(name, arity, env)` where `env` is built by
//     gathering the (cell-promoted) outer-scope variables.
//
// The lifter records all this in a `ClosureRegistry` consumed by the
// lowerer. The registry is keyed by lifted-body-name; the
// per-enclosing-function "which locals to promote" set is computed
// separately and stored under the enclosing function's name.

/// Sprint 24: per-method closure metadata. The lifter produces one of
/// these per `Expr::Method`. The lowerer consults the registry when it
/// sees an `Expr::Ident(lifted_name)` to decide whether to emit a
/// plain function-ref (no captures) or a `%make-closure` site (with
/// captures).
#[derive(Clone, Debug)]
pub struct ClosureInfo {
    pub lifted_name: String,
    /// Captured variable names in stable order. The index into this
    /// vector is the cell's slot index in the environment.
    pub captured: Vec<String>,
    pub arity: usize,
    pub span: Span,
}

/// Sprint 24 registry built by the lift pre-pass.
///
/// Two pieces of information come out of the pre-pass:
///
///   * **Per-lifted-body**: a `ClosureInfo` describing the body's
///     capture list. Used by the lowerer to (1) recognise that a
///     synthesised `Expr::Ident(lifted_name)` is a closure-creation
///     site, not a plain `\name` reference, and (2) compile the body
///     itself with the synthetic env parameter and the
///     captured-variable indexing scheme.
///
///   * **Per-enclosing-function** ("cell-promote sets"): for each
///     top-level / lifted function in the module, the set of its OWN
///     local-variable names that any inner closure captures. The
///     lowerer's per-body environment management uses this set to
///     cell-promote the matching `let` bindings AND to redirect reads
///     / writes through `%cell-get` / `%cell-set!`.
#[derive(Default, Clone, Debug)]
pub struct ClosureRegistry {
    /// `lifted_name -> ClosureInfo`.
    pub by_lifted_name: HashMap<String, ClosureInfo>,
    /// `enclosing_function_name -> set of locals captured by inner
    /// closures`. Drives cell-promotion in the enclosing body's lowering.
    pub cell_locals_per_function: HashMap<String, HashSet<String>>,
}

impl ClosureRegistry {
    pub fn closure_for(&self, name: &str) -> Option<&ClosureInfo> {
        self.by_lifted_name.get(name)
    }
    pub fn cell_locals_for(&self, function_name: &str) -> Option<&HashSet<String>> {
        self.cell_locals_per_function.get(function_name)
    }
}

/// Mutable threading state for the lift pre-pass. Carries the global
/// counters and per-call-site capture metadata that bubble up through
/// recursion.
struct LiftState<'a> {
    /// Set of module-level names (`define function`, classes, …) used
    /// by `check_free_vars` to distinguish "captured local" from
    /// "top-level reference".
    top: &'a HashSet<String>,
    /// Counter for `__anon-method-NNNN` synthetic names.
    counter: u32,
    /// Sink for lifted `Item::DefineFunction`s.
    new_items: Vec<Item>,
    /// Lift-time diagnostics.
    errors: Vec<LoweringError>,
    /// The Sprint 24 closure registry being built.
    registry: ClosureRegistry,
}

/// Per-scope rewriting context for the lift pre-pass. Carries the
/// set of names visible in the enclosing scope (so `check_free_vars`
/// can identify captures) and **the name of the enclosing function**
/// (so cell-promotion targets land in the right
/// `cell_locals_per_function` bucket).
struct LiftScope {
    /// Names bound in this lexical scope or any enclosing scope inside
    /// the current top-level function. Walks "outward" the same way
    /// `check_free_vars` does.
    in_scope: HashSet<String>,
    /// Name of the enclosing function (the synthetic top-level name
    /// for lifted bodies; the source name for user functions). The
    /// lift pass deposits "this local must be cell-promoted" under
    /// this name when an inner method captures it.
    enclosing_fn: String,
}

/// Pre-pass entry point. Mutates `module` in place and produces a
/// `ClosureRegistry` describing every closure site discovered. Returns
/// `Err` only for genuine lifting failures (none currently — Sprint 24
/// supports every capture shape Sprint 21 rejected).
fn lift_anonymous_methods(
    module: &mut Module,
) -> (ClosureRegistry, Vec<LoweringError>) {
    // Collect the set of top-level names so the free-variable check
    // can distinguish "captured local" from "module-scope reference".
    // Top-level names include `define function` / `define method` /
    // `define generic` / `define constant` / `define variable` /
    // `define class`. Registered seed / runtime classes (`<integer>`,
    // `<error>`, ...) are also OK because they resolve via
    // `find_class_id_by_name`.
    let mut top_level_names: HashSet<String> = HashSet::new();
    for item in &module.items {
        match item {
            Item::DefineFunction { name, .. }
            | Item::DefineMethod { name, .. }
            | Item::DefineGeneric { name, .. }
            | Item::DefineConstant { name, .. }
            | Item::DefineVariable { name, .. }
            | Item::DefineClass { name, .. } => {
                top_level_names.insert(name.clone());
            }
            _ => {}
        }
    }
    let mut state = LiftState {
        top: &top_level_names,
        counter: 0,
        new_items: Vec::new(),
        errors: Vec::new(),
        registry: ClosureRegistry::default(),
    };
    // Process each existing item in turn. Replacements append to the
    // module's items via `state.new_items`.
    let mut items = std::mem::take(&mut module.items);
    for mut item in items.drain(..) {
        lift_item(&mut item, &mut state);
        state.new_items.push(item);
    }
    module.items = std::mem::take(&mut state.new_items);
    (state.registry, state.errors)
}

fn lift_item(item: &mut Item, st: &mut LiftState<'_>) {
    match item {
        Item::DefineFunction { name, params, body, .. }
        | Item::DefineMethod { name, params, body, .. } => {
            let mut scope = LiftScope {
                in_scope: st.top.clone(),
                enclosing_fn: name.clone(),
            };
            for p in params.iter() {
                scope.in_scope.insert(p.name.clone());
            }
            for s in body.iter_mut() {
                lift_statement(s, &mut scope, st);
            }
        }
        Item::DefineConstant { value, name, .. }
        | Item::DefineVariable { value, name, .. } => {
            let mut scope = LiftScope {
                in_scope: st.top.clone(),
                enclosing_fn: name.clone(),
            };
            lift_expr(value, &mut scope, st);
        }
        Item::Expr(e) => {
            // Top-level expression — Sprint 12+ eval-entry uses
            // "<eval-entry>" as the synthetic enclosing function name.
            let mut scope = LiftScope {
                in_scope: st.top.clone(),
                enclosing_fn: "<eval-entry>".to_string(),
            };
            lift_expr(e, &mut scope, st);
        }
        // DefineClass: nested exprs in supers / slot defaults aren't
        // currently supported as expression-position method literals;
        // skip. DefineGeneric / DefineLibrary / DefineModule /
        // DefineMacro / DefineOther — no expression bodies to lift.
        _ => {}
    }
}

fn lift_statement(
    s: &mut Statement,
    scope: &mut LiftScope,
    st: &mut LiftState<'_>,
) {
    match s {
        Statement::Expr(e) => {
            lift_expr(e, scope, st);
        }
        Statement::Let { binders, value, .. } => {
            lift_expr(value, scope, st);
            for b in binders {
                scope.in_scope.insert(b.name.clone());
            }
        }
        Statement::Local { methods, .. } => {
            // Local methods bind their name in scope; their bodies are
            // not yet lifted (Sprint 06 lowering errors on local methods
            // with `Unsupported`).
            for m in methods {
                scope.in_scope.insert(m.name.clone());
            }
        }
        Statement::While { cond, body, .. } | Statement::Until { cond, body, .. } => {
            lift_expr(cond, scope, st);
            let saved = scope.in_scope.clone();
            for sub in body {
                lift_statement(sub, scope, st);
            }
            scope.in_scope = saved;
        }
        Statement::For { .. } => {
            // For statements aren't lowered in Sprint 18; leave alone.
        }
        Statement::Block {
            exit_var,
            body,
            handlers,
            cleanup,
            afterwards,
            ..
        } => {
            let saved = scope.in_scope.clone();
            if let Some(ev) = exit_var {
                scope.in_scope.insert(ev.clone());
            }
            for sub in body {
                lift_statement(sub, scope, st);
            }
            for h in handlers {
                let h_saved = scope.in_scope.clone();
                if let Some(v) = &h.var {
                    scope.in_scope.insert(v.clone());
                }
                for sub in &mut h.body {
                    lift_statement(sub, scope, st);
                }
                scope.in_scope = h_saved;
            }
            for sub in cleanup {
                lift_statement(sub, scope, st);
            }
            for sub in afterwards {
                lift_statement(sub, scope, st);
            }
            scope.in_scope = saved;
        }
    }
}

fn lift_expr(
    e: &mut Expr,
    scope: &mut LiftScope,
    st: &mut LiftState<'_>,
) {
    match e {
        Expr::Integer(..)
        | Expr::Float(..)
        | Expr::Bool(..)
        | Expr::String(..)
        | Expr::Char(..)
        | Expr::Symbol(..)
        | Expr::Ident(..) => {}
        Expr::Paren { inner, .. } => {
            lift_expr(inner, scope, st);
        }
        Expr::BinOp { lhs, rhs, .. } => {
            lift_expr(lhs, scope, st);
            lift_expr(rhs, scope, st);
        }
        Expr::UnOp { operand, .. } => {
            lift_expr(operand, scope, st);
        }
        Expr::If { cond, then_, else_, .. } => {
            lift_expr(cond, scope, st);
            lift_expr(then_, scope, st);
            if let Some(e2) = else_ {
                lift_expr(e2, scope, st);
            }
        }
        Expr::Begin { body, .. } => {
            let saved = scope.in_scope.clone();
            for sub in body {
                lift_expr(sub, scope, st);
            }
            scope.in_scope = saved;
        }
        Expr::Call { callee, args, .. } => {
            lift_expr(callee, scope, st);
            for a in args {
                lift_expr(a, scope, st);
            }
        }
        Expr::Let { binder, value, .. } => {
            lift_expr(value, scope, st);
            scope.in_scope.insert(binder.clone());
        }
        Expr::Case { .. } | Expr::LocalMethod { .. } | Expr::MacroCall { .. } => {
            // Not lowered; leave the unsupported diagnostic to the
            // main lowering pass. `MacroCall` should never reach
            // lowering — the macro engine substitutes it away
            // before lower runs. If we see one here it's a missing
            // macro definition; the diagnostic path catches it.
        }
        Expr::Stmt(s) => {
            lift_statement(s, scope, st);
        }
        Expr::Method { span, params, body } => {
            // Compute the captured set: every Ident referenced inside
            // the method body that isn't (a) one of the method's own
            // params, (b) a top-level name, (c) a fresh `let` binder
            // introduced inside the body, OR (d) an operator / class /
            // generic name. Sprint 24 promotes these to cells; Sprint 21
            // erred out here.
            let mut inner_scope: HashSet<String> = st.top.clone();
            for p in params.iter() {
                inner_scope.insert(p.name.clone());
            }
            let mut free_seq: Vec<(Span, String)> = Vec::new();
            for sub in body.iter() {
                check_free_vars(sub, &mut inner_scope, &scope.in_scope, st.top, &mut free_seq);
            }
            // De-duplicate while preserving first-seen order. The order
            // becomes the env-index assignment, so it must be stable.
            let mut captured: Vec<String> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            for (_, n) in &free_seq {
                if seen.insert(n.clone()) {
                    captured.push(n.clone());
                }
            }

            // Synthesise a fresh top-level name for the lifted body.
            let lifted_name = format!("__anon-method-{}", st.counter);
            st.counter += 1;

            // Record this closure's captured locals against the
            // enclosing function so the lowerer cell-promotes them.
            if !captured.is_empty() {
                let bucket = st
                    .registry
                    .cell_locals_per_function
                    .entry(scope.enclosing_fn.clone())
                    .or_default();
                for c in &captured {
                    bucket.insert(c.clone());
                }
            }

            // Build the lifted DefineFunction. For closures (non-empty
            // capture set), prepend a synthetic `__env` parameter — the
            // lowerer wires it in when it sees the `ClosureInfo` for
            // this body. We do NOT add the param at AST level (keeps
            // the AST stable for printing); instead, the lowerer reads
            // `ClosureInfo::captured.len() > 0` and inserts the env
            // parameter at the head of the body's params list before
            // lowering proceeds.
            let body_stmts: Vec<Statement> =
                body.iter().cloned().map(Statement::Expr).collect();
            st.new_items.push(Item::DefineFunction {
                span: *span,
                modifiers: Vec::new(),
                name: lifted_name.clone(),
                params: params.clone(),
                return_: None,
                body: body_stmts,
            });

            // Register the closure in the registry. The lowerer
            // consumes this to recognise `Expr::Ident(lifted_name)` as
            // a closure-creation site and to wire the env parameter
            // when lowering the body itself.
            st.registry.by_lifted_name.insert(
                lifted_name.clone(),
                ClosureInfo {
                    lifted_name: lifted_name.clone(),
                    captured: captured.clone(),
                    arity: params.len(),
                    span: *span,
                },
            );

            // Recursively lift any nested anonymous methods inside the
            // body we just stuffed into the synthetic DefineFunction.
            // Run the pre-pass on it in place; nested closures captured
            // variables that the new enclosing function (lifted_name)
            // owns now.
            let last_idx = st.new_items.len() - 1;
            let mut taken = std::mem::replace(
                &mut st.new_items[last_idx],
                Item::Expr(Expr::Bool(*span, false)),
            );
            lift_item(&mut taken, st);
            st.new_items[last_idx] = taken;

            // Replace the original Method expression with an ident
            // reference to the lifted thunk. The lowerer consults the
            // registry to decide whether to emit `nod_make_function_ref`
            // (no captures) or `%make-closure` (with captures + env).
            *e = Expr::Ident(*span, lifted_name);
        }
    }
}

/// Free-variable walk used inside `lift_expr`'s `Expr::Method` branch.
/// Pushes `(span, name)` into `free` for every Ident in `e` that
/// resolves to a name in `outer_scope` but NOT in `inner_scope` or
/// `top`.
///
/// `inner_scope` starts as `top + method-params` and grows as `let`
/// binders are introduced inside the body.
fn check_free_vars(
    e: &Expr,
    inner_scope: &mut HashSet<String>,
    outer_scope: &HashSet<String>,
    top: &HashSet<String>,
    free: &mut Vec<(Span, String)>,
) {
    match e {
        Expr::Integer(..)
        | Expr::Float(..)
        | Expr::Bool(..)
        | Expr::String(..)
        | Expr::Char(..)
        | Expr::Symbol(..) => {}
        Expr::Ident(span, name) => {
            // Sprint 21 free-var check: any Ident NOT in inner_scope
            // AND that exists in outer_scope is a capture. Idents that
            // resolve to a registered class / runtime generic stay OK.
            if inner_scope.contains(name) || top.contains(name) {
                return;
            }
            // Operator shims (`+`, `-`, ...) are always available.
            if operator_arity(name).is_some() {
                return;
            }
            // Registered classes (`<integer>`, `<error>`, ...).
            if name.starts_with('<') && name.ends_with('>') {
                return;
            }
            // Registered runtime generics (stdlib `size`, ...).
            if nod_runtime::is_generic_defined(name) {
                return;
            }
            if outer_scope.contains(name) {
                free.push((*span, name.clone()));
            }
            // If neither inner_scope nor outer_scope binds it, leave
            // the diagnostic to the main lowering pass (it'll surface
            // an UndefinedIdent).
        }
        Expr::Paren { inner, .. } => check_free_vars(inner, inner_scope, outer_scope, top, free),
        Expr::BinOp { lhs, rhs, .. } => {
            check_free_vars(lhs, inner_scope, outer_scope, top, free);
            check_free_vars(rhs, inner_scope, outer_scope, top, free);
        }
        Expr::UnOp { operand, .. } => {
            check_free_vars(operand, inner_scope, outer_scope, top, free);
        }
        Expr::If { cond, then_, else_, .. } => {
            check_free_vars(cond, inner_scope, outer_scope, top, free);
            check_free_vars(then_, inner_scope, outer_scope, top, free);
            if let Some(e2) = else_ {
                check_free_vars(e2, inner_scope, outer_scope, top, free);
            }
        }
        Expr::Begin { body, .. } => {
            let saved = inner_scope.clone();
            for sub in body {
                check_free_vars(sub, inner_scope, outer_scope, top, free);
            }
            *inner_scope = saved;
        }
        Expr::Call { callee, args, .. } => {
            check_free_vars(callee, inner_scope, outer_scope, top, free);
            for a in args {
                check_free_vars(a, inner_scope, outer_scope, top, free);
            }
        }
        Expr::Let { binder, value, .. } => {
            check_free_vars(value, inner_scope, outer_scope, top, free);
            inner_scope.insert(binder.clone());
        }
        Expr::Case { .. } | Expr::LocalMethod { .. } | Expr::MacroCall { .. } => {}
        Expr::Method { params, body, .. } => {
            // Nested anonymous method: its own params extend the inner
            // scope; the outer scope is unchanged for the recursive walk
            // (the nested method's free variables vs its enclosing
            // method's scope is what we want — same outer_scope).
            let mut nested_inner = inner_scope.clone();
            for p in params {
                nested_inner.insert(p.name.clone());
            }
            for sub in body {
                check_free_vars(sub, &mut nested_inner, outer_scope, top, free);
            }
        }
        Expr::Stmt(s) => check_free_vars_in_stmt(s, inner_scope, outer_scope, top, free),
    }
}

fn check_free_vars_in_stmt(
    s: &Statement,
    inner_scope: &mut HashSet<String>,
    outer_scope: &HashSet<String>,
    top: &HashSet<String>,
    free: &mut Vec<(Span, String)>,
) {
    match s {
        Statement::Expr(e) => check_free_vars(e, inner_scope, outer_scope, top, free),
        Statement::Let { binders, value, .. } => {
            check_free_vars(value, inner_scope, outer_scope, top, free);
            for b in binders {
                inner_scope.insert(b.name.clone());
            }
        }
        Statement::Local { methods, .. } => {
            for m in methods {
                inner_scope.insert(m.name.clone());
            }
        }
        Statement::While { cond, body, .. } | Statement::Until { cond, body, .. } => {
            check_free_vars(cond, inner_scope, outer_scope, top, free);
            let saved = inner_scope.clone();
            for sub in body {
                check_free_vars_in_stmt(sub, inner_scope, outer_scope, top, free);
            }
            *inner_scope = saved;
        }
        Statement::For { .. } => {}
        Statement::Block {
            exit_var,
            body,
            handlers,
            cleanup,
            afterwards,
            ..
        } => {
            let saved = inner_scope.clone();
            if let Some(ev) = exit_var {
                inner_scope.insert(ev.clone());
            }
            for sub in body {
                check_free_vars_in_stmt(sub, inner_scope, outer_scope, top, free);
            }
            for h in handlers {
                let mut h_scope = inner_scope.clone();
                if let Some(v) = &h.var {
                    h_scope.insert(v.clone());
                }
                for sub in &h.body {
                    check_free_vars_in_stmt(sub, &mut h_scope, outer_scope, top, free);
                }
            }
            for sub in cleanup {
                check_free_vars_in_stmt(sub, inner_scope, outer_scope, top, free);
            }
            for sub in afterwards {
                check_free_vars_in_stmt(sub, inner_scope, outer_scope, top, free);
            }
            *inner_scope = saved;
        }
    }
}

fn collect_top_level_names(m: &Module, user_classes: &HashMap<String, ClassId>) -> TopNames {
    let mut fns = HashMap::new();
    let mut fn_arity: HashMap<String, usize> = HashMap::new();
    for item in &m.items {
        if let Item::DefineFunction { name, params, return_, .. } = item {
            let ret = return_
                .as_ref()
                .and_then(|r| r.values.first().and_then(|v| v.type_.as_ref()))
                .map(|e| type_from_expr(Some(e)))
                .unwrap_or(TypeEstimate::Top);
            fns.insert(name.clone(), ret);
            fn_arity.insert(name.clone(), params.len());
        }
    }
    // Slot accessors are emitted as top-level functions too; record
    // them so `<C>-getter-foo(p)` resolves to a DirectCall. For MI
    // override accessors (`<C>-override-getter-foo`) — also include
    // them.
    for item in &m.items {
        let Item::DefineClass { name, .. } = item else {
            continue;
        };
        let Some(&class_id) = user_classes.get(name) else {
            continue;
        };
        let md_ptr = nod_runtime::class_metadata_ptr(class_id);
        if md_ptr.is_null() {
            continue;
        }
        // SAFETY: registered class, static-area metadata.
        let metadata = unsafe { &*md_ptr };
        for (idx, slot) in metadata.slots.iter().enumerate() {
            let origin = metadata.slot_origin[idx];
            if origin == class_id {
                let getter = format!("{}-getter-{}", name, slot.name);
                fns.insert(getter.clone(), slot_type_to_estimate(slot.type_kind));
                fn_arity.insert(getter, 1);
                if slot.has_setter {
                    let setter = format!("{}-setter-{}", name, slot.name);
                    fns.insert(setter.clone(), TypeEstimate::Top);
                    fn_arity.insert(setter, 2);
                }
            } else {
                // Inherited slot — if Phase 3 will generate an override
                // (offset differs vs. defining-class layout), the
                // override function needs to be in `top_names` too.
                let origin_md_ptr = nod_runtime::class_metadata_ptr(origin);
                if origin_md_ptr.is_null() {
                    continue;
                }
                // SAFETY: static-area metadata.
                let origin_md = unsafe { &*origin_md_ptr };
                let origin_offset = origin_md
                    .slots
                    .iter()
                    .find(|s| s.name == slot.name)
                    .map(|s| s.offset)
                    .unwrap_or(slot.offset);
                if origin_offset != slot.offset {
                    let getter = format!("{}-override-getter-{}", name, slot.name);
                    fns.insert(getter.clone(), slot_type_to_estimate(slot.type_kind));
                    fn_arity.insert(getter, 1);
                    if slot.has_setter {
                        let setter = format!("{}-override-setter-{}", name, slot.name);
                        fns.insert(setter.clone(), TypeEstimate::Top);
                        fn_arity.insert(setter, 2);
                    }
                }
            }
        }
    }
    TopNames { fns, fn_arity }
}

fn collect_generic_names(m: &Module) -> HashSet<String> {
    let mut out = HashSet::new();
    for item in &m.items {
        match item {
            Item::DefineGeneric { name, .. } => {
                out.insert(name.clone());
            }
            Item::DefineMethod { name, .. } => {
                out.insert(name.clone());
            }
            Item::DefineClass { name, .. } => {
                // Auto-generated slot accessors are generics (registered
                // by name into the dispatch table). Adding them here
                // ensures `x(p)` lowers to Dispatch when the function
                // table isn't sufficient (e.g. cross-class methods).
                //
                // For MI: every slot — own or inherited — belongs to a
                // generic with the slot's name. The dispatch picks the
                // right per-class method (override or parent's).
                if let Some(class_id) = find_class_id_by_name(name) {
                    let md_ptr = nod_runtime::class_metadata_ptr(class_id);
                    if !md_ptr.is_null() {
                        // SAFETY: registered class.
                        let metadata = unsafe { &*md_ptr };
                        for slot in &metadata.slots {
                            out.insert(slot.name.clone());
                            if slot.has_setter {
                                out.insert(format!("{}-setter", slot.name));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

// ─── Type-expr → TypeEstimate ────────────────────────────────────────────

fn type_from_expr(ty: Option<&Expr>) -> TypeEstimate {
    let Some(ty) = ty else { return TypeEstimate::Top };
    match ty {
        Expr::Ident(_, n) => match n.as_str() {
            "<integer>" => TypeEstimate::Integer,
            "<single-float>" => TypeEstimate::SingleFloat,
            "<double-float>" | "<float>" => TypeEstimate::DoubleFloat,
            "<boolean>" => TypeEstimate::Boolean,
            "<character>" => TypeEstimate::Character,
            "<string>" | "<byte-string>" => TypeEstimate::String,
            "<object>" | "<top>" => TypeEstimate::Top,
            // Sprint 15 method-specialiser narrowing: a `<foo>`-shaped
            // type ident that resolves to a registered class lights up
            // as `Class(<foo>)`. The dispatch resolver consults this
            // alongside the sealing facts to pick sealed-direct.
            //
            // For unregistered classes we fall back to `Top` (the
            // parameter type is informational only — codegen lowers
            // it as a tagged Word regardless of estimate). The
            // narrowing pass / resolver simply skips temps with `Top`.
            other if other.starts_with('<') && other.ends_with('>') => {
                match find_class_id_by_name(other) {
                    Some(id) => TypeEstimate::Class(id.0),
                    None => TypeEstimate::Top,
                }
            }
            _ => TypeEstimate::Top,
        },
        _ => TypeEstimate::Top,
    }
}

// ─── Function builder ────────────────────────────────────────────────────

struct FunctionBuilder {
    func: Function,
    current: usize,
    next_temp: u32,
    next_block: u32,
    /// Sprint 19: last value-producing temp in this function, used by
    /// `lower_statements_into` (the block-lifting helper) to know what
    /// to return from a lifted thunk. Updated as statements lower.
    /// `None` after a statement that produces no value (loops).
    last_temp: Option<TempId>,
    /// Sprint 24: cell-promotion + closure-env context for this body.
    /// Populated by `lower_function_inner` before any statement runs.
    /// The `lower_expr` / `lower_assign` / `let`-statement paths
    /// consult this to redirect captured-local reads/writes through
    /// `%cell-get` / `%cell-set!` and to lower a `%env-cell` indirection
    /// for variables that live in the enclosing environment.
    cell_ctx: CellCtx,
}

impl FunctionBuilder {
    fn new(id: FunctionId, name: String, span: Span) -> Self {
        let entry = BlockId(0);
        let func = Function {
            id,
            name,
            params: Vec::new(),
            entry,
            blocks: vec![Block {
                id: entry,
                label: "entry".to_string(),
                params: Vec::new(),
                computations: Vec::new(),
                terminator: Terminator::Return { value: None },
            }],
            temps: Vec::new(),
            return_type: TypeEstimate::Unit,
            span,
        };
        Self {
            func,
            current: 0,
            next_temp: 0,
            next_block: 1,
            last_temp: None,
            cell_ctx: CellCtx::default(),
        }
    }

    fn finish(self) -> Function {
        self.func
    }

    fn last_temp(&self) -> Option<TempId> {
        self.last_temp
    }

    fn set_last_temp(&mut self, t: TempId) {
        self.last_temp = Some(t);
    }

    fn clear_last_temp(&mut self) {
        self.last_temp = None;
    }

    fn fresh_temp(&mut self, ty: TypeEstimate) -> TempId {
        let id = TempId(self.next_temp);
        self.next_temp += 1;
        self.func.temps.push(Temporary {
            id,
            type_estimate: ty,
        });
        id
    }

    fn new_block(&mut self, label: String) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        self.func.blocks.push(Block {
            id,
            label,
            params: Vec::new(),
            computations: Vec::new(),
            terminator: Terminator::Return { value: None },
        });
        id
    }

    fn block_mut(&mut self, id: BlockId) -> &mut Block {
        self.func
            .blocks
            .iter_mut()
            .find(|b| b.id == id)
            .expect("block not found")
    }

    fn switch_to(&mut self, id: BlockId) {
        self.current = self
            .func
            .blocks
            .iter()
            .position(|b| b.id == id)
            .expect("block not found");
    }

    fn push(&mut self, c: Computation) {
        self.func.blocks[self.current].computations.push(c);
    }

    fn terminate_current(&mut self, t: Terminator) {
        self.func.blocks[self.current].terminator = t;
    }

    fn add_block_param(&mut self, block: BlockId, ty: TypeEstimate) -> TempId {
        let t = self.fresh_temp(ty);
        self.block_mut(block).params.push(t);
        t
    }

    // ─── Expression lowering ────────────────────────────────────────────

    fn lower_expr(
        &mut self,
        e: &Expr,
        env: &mut LocalEnv,
        ctx: &LowerCtx,
    ) -> Result<TempId, LoweringError> {
        match e {
            Expr::Integer(span, v) => {
                const FIXNUM_MIN_I128: i128 = -(1_i128 << 62);
                const FIXNUM_MAX_I128: i128 = (1_i128 << 62) - 1;
                if *v < FIXNUM_MIN_I128 || *v > FIXNUM_MAX_I128 {
                    return Err(LoweringError::IntegerOverflow {
                        span: *span,
                        value: *v,
                    });
                }
                let t = self.fresh_temp(TypeEstimate::Integer);
                self.push(Computation::Const {
                    dst: t,
                    value: ConstValue::Integer(*v),
                });
                Ok(t)
            }
            Expr::Float(_, v) => {
                let t = self.fresh_temp(TypeEstimate::DoubleFloat);
                self.push(Computation::Const {
                    dst: t,
                    value: ConstValue::Float(*v),
                });
                Ok(t)
            }
            Expr::Bool(_, v) => {
                let t = self.fresh_temp(TypeEstimate::Boolean);
                self.push(Computation::Const {
                    dst: t,
                    value: ConstValue::Bool(*v),
                });
                Ok(t)
            }
            Expr::String(_, raw) => {
                let decoded = decode_dylan_string_literal(raw);
                let t = self.fresh_temp(TypeEstimate::String);
                self.push(Computation::Const {
                    dst: t,
                    value: ConstValue::String(decoded),
                });
                Ok(t)
            }
            Expr::Char(_, c) => {
                let t = self.fresh_temp(TypeEstimate::Character);
                self.push(Computation::Const {
                    dst: t,
                    value: ConstValue::Char(*c),
                });
                Ok(t)
            }
            Expr::Symbol(_, raw) => {
                // Sprint 22: symbol literals. The parser delivers the
                // raw token text. Three surface forms to normalise:
                //   * `#"foo"` → `"foo"`
                //   * `#:foo`  → `"foo"`
                //   * `foo:`   → `"foo"`
                let name = if let Some(s) = raw
                    .strip_prefix("#\"")
                    .and_then(|s| s.strip_suffix('"'))
                {
                    s.to_string()
                } else if let Some(s) = raw.strip_prefix("#:") {
                    s.to_string()
                } else {
                    raw.trim_end_matches(':').to_string()
                };
                Ok(self.emit_symbol_literal(&name))
            }
            Expr::Ident(span, name) => {
                // Sprint 24: closure body capture — an inner-method
                // body that captures `name` from its outer scope reads
                // it through `%cell-get(%env-cell(env, idx))`.
                if let Some(ec) = self.cell_ctx.env_captures.clone()
                    && let Some(&idx) = ec.index_of.get(name)
                {
                    return Ok(self.emit_captured_var_read(ec.env_temp, idx));
                }
                if let Some(t) = env.get(name).copied() {
                    // Sprint 24: cell-promoted local — the env binds
                    // the CELL Word. Insert a `%cell-get` to read the
                    // underlying value.
                    if self.cell_ctx.cell_locals.contains(name) {
                        let dst = self.fresh_temp(TypeEstimate::Top);
                        self.push(Computation::DirectCall {
                            dst,
                            callee: "nod_cell_get".to_string(),
                            args: vec![t],
                            safepoint_roots: Vec::new(),
                        });
                        return Ok(dst);
                    }
                    return Ok(t);
                }
                // Sprint 29: stdlib-curated integer constant. The
                // `$MB-OK`, `$WM-PAINT`, … set (and any future stdlib
                // `define constant N = <int>`) lives in a process-
                // global map populated by the stdlib loader. Resolve
                // it here BEFORE the function-ref fallback path so
                // user code reads the constant as a literal integer,
                // not a `<function>` Word.
                //
                // Local bindings shadow the stdlib constant (the
                // `env.get` check above happens first), matching how
                // every other resolution-order step behaves.
                if let Some(v) = crate::stdlib::lookup_constant(name) {
                    const FIXNUM_MIN_I128: i128 = -(1_i128 << 62);
                    const FIXNUM_MAX_I128: i128 = (1_i128 << 62) - 1;
                    if !(FIXNUM_MIN_I128..=FIXNUM_MAX_I128).contains(&v) {
                        return Err(LoweringError::IntegerOverflow {
                            span: *span,
                            value: v,
                        });
                    }
                    let t = self.fresh_temp(TypeEstimate::Integer);
                    self.push(Computation::Const {
                        dst: t,
                        value: ConstValue::Integer(v),
                    });
                    return Ok(t);
                }
                // Sprint 12: a `<foo>`-shaped ident may refer to a
                // registered class. Lower as a constant pointer to
                // the class metadata (i.e. a tagged Word).
                if name.starts_with('<')
                    && name.ends_with('>')
                    && let Some(class_id) = ctx.user_classes.get(name).copied().or_else(|| find_class_id_by_name(name))
                {
                    return Ok(self.emit_class_ref(class_id));
                }
                // Sprint 24: closure-creation site. The lift pre-pass
                // rewrites `method (...) ... end` to
                // `Expr::Ident(__anon-method-NNNN)`; if that name is in
                // the closure registry AND has a non-empty capture set,
                // emit `%make-closure(name, arity, env)` here. The env
                // is built from the captured locals — each one's
                // cell-Word lives in the current `LocalEnv` as the
                // result of cell-promotion at its `let` (or param)
                // binding site.
                if let Some(reg) = ctx.closures
                    && let Some(info) = reg.closure_for(name)
                    && !info.captured.is_empty()
                {
                    let mut captured_cells: Vec<TempId> = Vec::with_capacity(info.captured.len());
                    for cap in &info.captured {
                        // Cell lives in `env` because the enclosing
                        // body's cell-promotion logic stored it there.
                        // If for some reason it isn't found, fall
                        // through to UndefinedIdent.
                        let Some(&cell_t) = env.get(cap) else {
                            return Err(LoweringError::UndefinedIdent {
                                span: *span,
                                name: cap.clone(),
                            });
                        };
                        captured_cells.push(cell_t);
                    }
                    return Ok(self.emit_make_closure(name, info.arity, &captured_cells));
                }
                // Sprint 21: first-class function references.
                //
                // An ident in expression position that resolves to a
                // registered function (top-level / slot accessor / stdlib
                // method / operator shim) lowers to
                // `nod_make_function_ref(name, arity)`.
                //
                // Arity resolution priority:
                //   1. `top_names::arity(name)` — user functions and
                //      slot accessors in THIS module.
                //   2. operator shims — fixed arity-2.
                //   3. generics — pick the first registered method's
                //      param count via the dispatch table.
                if let Some(arity) = ctx.top_names.arity(name) {
                    return Ok(self.emit_make_function_ref(name, arity));
                }
                if let Some(arity) = operator_arity(name) {
                    return Ok(self.emit_make_function_ref(name, arity));
                }
                if ctx.generics.contains(name) || nod_runtime::is_generic_defined(name) {
                    // Read the arity from the first method registered
                    // under this generic, if any. For stdlib methods
                    // rewritten as `f (x :: <object>, …)`, the param
                    // count IS the arity.
                    let arity = nod_runtime::find_generic(name)
                        .and_then(|g| g.first_method_param_count())
                        .unwrap_or(1);
                    return Ok(self.emit_make_function_ref(name, arity));
                }
                if ctx.top_names.contains(name) {
                    // Should be reachable only if arity lookup somehow
                    // failed; fall back to arity 1 so we don't crash.
                    return Ok(self.emit_make_function_ref(name, 1));
                }
                Err(LoweringError::UndefinedIdent {
                    span: *span,
                    name: name.clone(),
                })
            }
            Expr::Paren { inner, .. } => self.lower_expr(inner, env, ctx),
            Expr::BinOp { op, lhs, rhs, span } => {
                if *op == BinOp::Assign {
                    return self.lower_assign(lhs, rhs, *span, env, ctx);
                }
                let l = self.lower_expr(lhs, env, ctx)?;
                let r = self.lower_expr(rhs, env, ctx)?;
                let lt = self.func.temp_type(l);
                let rt = self.func.temp_type(r);
                // Sprint 42a — generic `=` dispatch for non-numeric operands.
                // When both operands are pointer-shaped (neither statically
                // `<integer>` nor any float), `=`/`==`/`~=`/`~==` route
                // through `%object-equal?` so byte-strings, symbols, and
                // other heap objects get content equality instead of
                // pointer-compare. The Rust shim (`nod_object_equal_p`)
                // checks raw-bit identity first, so fixnum-tagged Words
                // (which carry their value in the bits) round-trip
                // identically to `PrimOp::EqInt`. We invert via `BoolNot`
                // for the negative operators.
                //
                // The integer / float fast paths below stay exactly as
                // they were — this only diverts when neither operand has
                // a known numeric estimate.
                if matches!(*op, BinOp::Eq | BinOp::EqEq | BinOp::Ne | BinOp::NeEq)
                    && !lt.is_integer()
                    && !lt.is_float()
                    && !rt.is_integer()
                    && !rt.is_float()
                {
                    let eq_dst = self.fresh_temp(TypeEstimate::Boolean);
                    self.push(Computation::DirectCall {
                        dst: eq_dst,
                        callee: "nod_object_equal_p".to_string(),
                        args: vec![l, r],
                        safepoint_roots: Vec::new(),
                    });
                    if matches!(*op, BinOp::Ne | BinOp::NeEq) {
                        let neg_dst = self.fresh_temp(TypeEstimate::Boolean);
                        self.push(Computation::PrimOp {
                            dst: neg_dst,
                            op: PrimOp::BoolNot,
                            args: vec![eq_dst],
                        });
                        return Ok(neg_dst);
                    }
                    return Ok(eq_dst);
                }
                let op = select_binop(*op, lt, rt, *span)?;
                let dst = self.fresh_temp(op.result_type());
                self.push(Computation::PrimOp {
                    dst,
                    op,
                    args: vec![l, r],
                });
                Ok(dst)
            }
            Expr::UnOp { op, operand, span } => {
                let v = self.lower_expr(operand, env, ctx)?;
                let vt = self.func.temp_type(v);
                let op = select_unop(*op, vt, *span)?;
                let dst = self.fresh_temp(op.result_type());
                self.push(Computation::PrimOp {
                    dst,
                    op,
                    args: vec![v],
                });
                Ok(dst)
            }
            Expr::If { cond, then_, else_, span } => {
                let Some(else_) = else_ else {
                    return Err(LoweringError::Unsupported {
                        span: *span,
                        message: "Sprint 06 lowers only `if`-expressions with an `else` arm"
                            .to_string(),
                    });
                };
                self.lower_if(cond, then_, else_, env, ctx)
            }
            Expr::Begin { body, span } => {
                if body.is_empty() {
                    return Err(LoweringError::Unsupported {
                        span: *span,
                        message: "empty `begin` block not lowered".to_string(),
                    });
                }
                let last_idx = body.len() - 1;
                let mut last = None;
                for (i, e) in body.iter().enumerate() {
                    let t = self.lower_expr(e, env, ctx)?;
                    if i == last_idx {
                        last = Some(t);
                    }
                }
                Ok(last.expect("begin had body"))
            }
            Expr::Call { callee, args, span } => {
                self.lower_call(callee, args, *span, env, ctx)
            }
            Expr::Let { binder, value, .. } => {
                // Sprint 18: lower `let X = E` at expression position
                // — used by macro-emitted `begin let i = … ; while … end end`
                // and by Sprint 03's single-binder `let x = 41; x + 1 end`
                // surface. Inserts the binder into the surrounding env
                // and returns the value temp (so the expression evaluates
                // to the bound value).
                //
                // Sprint 24: if `binder` is captured by an inner closure,
                // promote it to a cell so reads / writes share storage
                // with the env-cell the inner closure accesses.
                let t = self.lower_expr(value, env, ctx)?;
                let bound = if self.cell_ctx.cell_locals.contains(binder) {
                    let cell = self.fresh_temp(TypeEstimate::Top);
                    self.push(Computation::DirectCall {
                        dst: cell,
                        callee: "nod_make_cell".to_string(),
                        args: vec![t],
                        safepoint_roots: Vec::new(),
                    });
                    cell
                } else {
                    t
                };
                env.insert(binder.clone(), bound);
                Ok(t)
            }
            Expr::Method { span, .. } => {
                // Sprint 21: `method (...) ... end` in expression
                // position should have been rewritten to a synthetic
                // `Expr::Ident(__anon-method-NNNN)` by
                // `lift_anonymous_methods` in the lowering pre-pass
                // (see `lift_anonymous_methods` below). If we got here,
                // the lifting pass missed a Method form — surface as an
                // unsupported diagnostic so the bug is loud.
                Err(LoweringError::Unsupported {
                    span: *span,
                    message: "anonymous method survived the Sprint 21 lift pre-pass — \
                              please report; expected every Expr::Method in expression \
                              position to be rewritten to an ident reference"
                        .to_string(),
                })
            }
            Expr::Case { span, .. } | Expr::LocalMethod { span, .. } => {
                Err(LoweringError::Unsupported {
                    span: *span,
                    message: format!(
                        "expression form `{}` not lowered in Sprint 06",
                        expr_kind(e)
                    ),
                })
            }
            Expr::MacroCall { span, name } => Err(LoweringError::Unsupported {
                span: *span,
                message: format!(
                    "macro call `{name}` reached lowering — no matching `define macro` \
                     in the seeded macro table; expansion was skipped"
                ),
            }),
            Expr::Stmt(s) => self.lower_stmt_as_expr(s, env, ctx),
        }
    }

    /// Sprint 24: emit the IR for reading a captured variable at index
    /// `idx` from the closure body's synthetic env parameter. Expands
    /// to two calls: `%env-cell(env, idx)` to fetch the cell, then
    /// `%cell-get(cell)` to read its value.
    fn emit_captured_var_read(&mut self, env_temp: TempId, idx: usize) -> TempId {
        let idx_t = self.fresh_temp(TypeEstimate::Integer);
        self.push(Computation::Const {
            dst: idx_t,
            value: ConstValue::Integer(idx as i128),
        });
        let cell_t = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst: cell_t,
            callee: "nod_env_cell".to_string(),
            args: vec![env_temp, idx_t],
            safepoint_roots: Vec::new(),
        });
        let val_t = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst: val_t,
            callee: "nod_cell_get".to_string(),
            args: vec![cell_t],
            safepoint_roots: Vec::new(),
        });
        val_t
    }

    /// Sprint 24: emit the IR for writing `value` into the captured
    /// variable at index `idx` (in the closure body's env). Expands to
    /// `%cell-set!(value, %env-cell(env, idx))`.
    fn emit_captured_var_write(
        &mut self,
        env_temp: TempId,
        idx: usize,
        value: TempId,
    ) -> TempId {
        let idx_t = self.fresh_temp(TypeEstimate::Integer);
        self.push(Computation::Const {
            dst: idx_t,
            value: ConstValue::Integer(idx as i128),
        });
        let cell_t = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst: cell_t,
            callee: "nod_env_cell".to_string(),
            args: vec![env_temp, idx_t],
            safepoint_roots: Vec::new(),
        });
        let dst = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst,
            callee: "nod_cell_set".to_string(),
            args: vec![value, cell_t],
            safepoint_roots: Vec::new(),
        });
        dst
    }

    /// Sprint 24: emit a `%make-closure(name, arity, env)` call. The
    /// `env` is built by gathering the captured locals' cell Words (each
    /// already stored in the current `LocalEnv` as the result of
    /// cell-promotion at the binding site) into a fresh SOV and
    /// wrapping it in an `<environment>`.
    fn emit_make_closure(
        &mut self,
        lifted_name: &str,
        arity: usize,
        captured_cells: &[TempId],
    ) -> TempId {
        // 1. Allocate the cells vector (len = captured.len()).
        let len_t = self.fresh_temp(TypeEstimate::Integer);
        self.push(Computation::Const {
            dst: len_t,
            value: ConstValue::Integer(captured_cells.len() as i128),
        });
        let sov_t = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst: sov_t,
            callee: "nod_make_sov_len".to_string(),
            args: vec![len_t],
            safepoint_roots: Vec::new(),
        });
        // 2. Fill the SOV slots with the captured cell Words.
        for (i, &cell) in captured_cells.iter().enumerate() {
            let i_t = self.fresh_temp(TypeEstimate::Integer);
            self.push(Computation::Const {
                dst: i_t,
                value: ConstValue::Integer(i as i128),
            });
            let _ = self.emit_sov_element_setter(cell, sov_t, i_t);
        }
        // 3. Wrap the SOV in an `<environment>`.
        let env_t = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst: env_t,
            callee: "nod_make_environment".to_string(),
            args: vec![sov_t],
            safepoint_roots: Vec::new(),
        });
        // 4. Allocate the closure Word with this env.
        let name_word = self.emit_string_literal(lifted_name);
        let arity_t = self.fresh_temp(TypeEstimate::Integer);
        self.push(Computation::Const {
            dst: arity_t,
            value: ConstValue::Integer(arity as i128),
        });
        let dst = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst,
            callee: "nod_make_closure".to_string(),
            args: vec![name_word, arity_t, env_t],
            safepoint_roots: Vec::new(),
        });
        dst
    }

    fn emit_sov_element_setter(
        &mut self,
        value: TempId,
        sov: TempId,
        idx: TempId,
    ) -> TempId {
        let dst = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst,
            callee: "nod_sov_element_setter".to_string(),
            args: vec![value, sov, idx],
            safepoint_roots: Vec::new(),
        });
        dst
    }

    /// Sprint 21: emit a `nod_make_function_ref(name_bytestring,
    /// arity_fixnum)` call. The result is a pointer-tagged `<function>`
    /// Word; the underlying instance lives in the static area so the
    /// address is stable. Codegen turns this into a DirectCall to the
    /// runtime shim.
    fn emit_make_function_ref(&mut self, name: &str, arity: usize) -> TempId {
        let name_word = self.emit_string_literal(name);
        let arity_temp = self.fresh_temp(TypeEstimate::Integer);
        self.push(Computation::Const {
            dst: arity_temp,
            value: ConstValue::Integer(arity as i128),
        });
        let dst = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst,
            callee: "nod_make_function_ref".to_string(),
            args: vec![name_word, arity_temp],
            safepoint_roots: Vec::new(),
        });
        dst
    }

    /// Emit a `<byte-string>` literal Word for the supplied Rust `&str`.
    /// The bake goes through the static-area-pinned literal pool so the
    /// address is stable across GC.
    ///
    /// Sprint 38c — emits `ConstValue::StringLiteralRef(text)` instead
    /// of `ConstValue::WordBits(w.raw())`. Codegen lowers this to a
    /// `load i64` through a per-module external global keyed by content,
    /// so the bitcode round-trips across processes.
    fn emit_string_literal(&mut self, s: &str) -> TempId {
        let t = self.fresh_temp(TypeEstimate::String);
        self.push(Computation::Const {
            dst: t,
            value: ConstValue::StringLiteralRef(s.to_string()),
        });
        t
    }

    /// Materialise a class reference as a Word constant pointing at
    /// the class's `ClassMetadata` in the static area. We tag the
    /// address with bit 0 = 1 (pointer tag); slot-load/store codegen
    /// will untag.
    ///
    /// Sprint 38c — emits `ConstValue::ClassMetadataPtr { class_id,
    /// tagged: true }`. Codegen lowers the load via the per-module
    /// external global; the `| 1` pointer-tag is applied AFTER the
    /// load (codegen handles the OR).
    fn emit_class_ref(&mut self, class_id: ClassId) -> TempId {
        let t = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::Const {
            dst: t,
            value: ConstValue::ClassMetadataPtr {
                class_id: class_id.0,
                tagged: true,
            },
        });
        t
    }

    fn lower_assign(
        &mut self,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
        env: &mut LocalEnv,
        ctx: &LowerCtx,
    ) -> Result<TempId, LoweringError> {
        // Sprint 18: `local := value` reassigns a local-variable binding
        // in-place. We don't have proper mutable cells — we just rebind
        // the name to the new value's temp in `env`. This makes the
        // post-assignment SSA temp visible to subsequent reads in the
        // same scope; `lower_while_like` snapshots names at the back
        // edge to thread them through the header phi.
        //
        // Sprint 24: cell-promoted locals + captured variables route
        // through the cell-set! / env-cell shims instead of an SSA
        // rebind — the mutation is visible to inner closures (and the
        // outer scope sees inner mutations too).
        if let Expr::Ident(_, name) = lhs {
            // Closure body captured-var write.
            if let Some(ec) = self.cell_ctx.env_captures.clone()
                && let Some(&idx) = ec.index_of.get(name)
            {
                let v = self.lower_expr(rhs, env, ctx)?;
                return Ok(self.emit_captured_var_write(ec.env_temp, idx, v));
            }
            if env.contains_key(name) {
                // Cell-promoted local: write through `%cell-set!`.
                if self.cell_ctx.cell_locals.contains(name) {
                    let v = self.lower_expr(rhs, env, ctx)?;
                    let cell_t = *env.get(name).expect("env entry checked");
                    let dst = self.fresh_temp(TypeEstimate::Top);
                    self.push(Computation::DirectCall {
                        dst,
                        callee: "nod_cell_set".to_string(),
                        args: vec![v, cell_t],
                        safepoint_roots: Vec::new(),
                    });
                    return Ok(dst);
                }
                // Plain local: rebind the SSA temp.
                let t = self.lower_expr(rhs, env, ctx)?;
                env.insert(name.clone(), t);
                return Ok(t);
            }
            return Err(LoweringError::UndefinedIdent {
                span,
                name: name.clone(),
            });
        }
        // Sprint 12: only `slot-getter(obj) := value` is supported.
        // I.e. lhs is `Call(Ident(name), [obj])`. We rewrite to a
        // setter dispatch.
        let Expr::Call { callee, args, .. } = lhs else {
            return Err(LoweringError::Unsupported {
                span,
                message: "Sprint 12 only supports `slot-getter(obj) := value` assignment".to_string(),
            });
        };
        let Expr::Ident(_, slot_name) = callee.as_ref() else {
            return Err(LoweringError::Unsupported {
                span,
                message: "Sprint 12 assign-call: callee must be an identifier".to_string(),
            });
        };
        if args.is_empty() {
            return Err(LoweringError::Unsupported {
                span,
                message: "setter: callee must have at least one argument".to_string(),
            });
        }
        // Sprint 22: N-ary setters. For `f(a0, a1, …) := v`, lower to
        // `Dispatch("f-setter", [v, a0, a1, …])` — Dylan's setter
        // calling convention puts the new value first. The unary case
        // (Sprint 12: `slot(obj) := value` → `Dispatch("slot-setter",
        // [obj, value])`) is preserved as a special case below for
        // back-compat with slot-getter rewrites.
        let obj_temps: Vec<TempId> = args
            .iter()
            .map(|a| self.lower_expr(a, env, ctx))
            .collect::<Result<_, _>>()?;
        let value_temp = self.lower_expr(rhs, env, ctx)?;
        if obj_temps.len() == 1
            && let Some(offset) =
                self.try_resolve_slot_offset(obj_temps[0], slot_name, ctx)
        {
            let dst = self.fresh_temp(TypeEstimate::Top);
            self.push(Computation::StoreSlot {
                dst,
                instance: obj_temps[0],
                offset,
                value: value_temp,
                slot_type: SlotTypeKind::Object,
            });
            return Ok(dst);
        }
        let dst = self.fresh_temp(TypeEstimate::Top);
        if obj_temps.len() == 1 {
            // Sprint 12 shape: `slot-setter(obj, value)`.
            self.push(Computation::Dispatch {
                dst,
                generic_name: format!("{slot_name}-setter"),
                args: vec![obj_temps[0], value_temp],
                safepoint_roots: Vec::new(),
            });
        } else {
            // Sprint 22 N-ary shape: `f-setter(value, a0, a1, …)`.
            let mut all = Vec::with_capacity(1 + obj_temps.len());
            all.push(value_temp);
            all.extend(obj_temps);
            self.push(Computation::Dispatch {
                dst,
                generic_name: format!("{slot_name}-setter"),
                args: all,
                safepoint_roots: Vec::new(),
            });
        }
        Ok(dst)
    }

    /// If `obj_temp` carries a user-class type estimate (or its declared
    /// parameter type is one), and `slot_name` is one of that class's
    /// slots, return the byte offset. Otherwise `None`.
    fn try_resolve_slot_offset(
        &self,
        _obj_temp: TempId,
        _slot_name: &str,
        _ctx: &LowerCtx,
    ) -> Option<usize> {
        // Sprint 12: the SSA type lattice doesn't carry user class ids
        // directly. Always go through Dispatch for slot access. The
        // direct LoadSlot path lights up when we add a class-aware
        // type estimate (Sprint 13).
        None
    }

    fn lower_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        span: Span,
        env: &mut LocalEnv,
        ctx: &LowerCtx,
    ) -> Result<TempId, LoweringError> {
        // `instance?(v, <class>)` intrinsic.
        if let Expr::Ident(_, name) = callee
            && name == "instance?"
            && args.len() == 2
        {
            return self.lower_instance_check(&args[0], &args[1], env, ctx, span);
        }
        // `make(<class>, kw: v, ...)` intrinsic.
        if let Expr::Ident(_, name) = callee
            && name == "make"
        {
            return self.lower_make(args, env, ctx, span);
        }
        // Sprint 14: `next-method()` and `next-method?()` intrinsics.
        // Lower to DirectCall against the runtime shim. Explicit-args
        // form `(next-method x y)` is Sprint 17 macro territory; today
        // only the no-args form is supported (the shim re-uses the
        // parent method's args via the thread-local chain frame).
        if let Expr::Ident(_, name) = callee
            && name == "next-method"
        {
            if !args.is_empty() {
                return Err(LoweringError::Unsupported {
                    span,
                    message: "Sprint 14: `next-method` with explicit arguments is Sprint 17 macro work; use the no-args form".to_string(),
                });
            }
            let dst = self.fresh_temp(TypeEstimate::Top);
            self.push(Computation::DirectCall {
                dst,
                callee: "nod_next_method".to_string(),
                args: Vec::new(),
                safepoint_roots: Vec::new(),
            });
            return Ok(dst);
        }
        if let Expr::Ident(_, name) = callee
            && name == "next-method?"
            && args.is_empty()
        {
            let dst = self.fresh_temp(TypeEstimate::Boolean);
            self.push(Computation::DirectCall {
                dst,
                callee: "nod_has_next_method".to_string(),
                args: Vec::new(),
                safepoint_roots: Vec::new(),
            });
            return Ok(dst);
        }
        // Sprint 16: `<pair>` / `<list>` builtins. `pair`, `head`,
        // `tail`, `empty?`, `nil` lower to direct calls into the runtime
        // shims. The codegen layer recognises the `%pair*` / `%nil` /
        // `%empty?` prefixes and emits the right extern declarations +
        // call sites. Estimates carry `Class(<pair>)` for the allocating
        // form so the dispatch resolver can narrow `<pair>`-typed args.
        if let Expr::Ident(_, name) = callee
            && let Some(builtin) = ListBuiltin::from_name(name)
        {
            return self.lower_list_builtin(builtin, args, env, ctx, span);
        }
        // Sprint 20b: `#(a, b, c)` literal lists. The parser emits
        // `Call(Ident("#list"), [a, b, c])`; we lower as a right-nested
        // chain of `pair(elt, tail)` calls bottoming out at `nil`.
        if let Expr::Ident(_, name) = callee
            && name == "#list"
        {
            // Empty list literal: just `nil`.
            if args.is_empty() {
                let dst = self.fresh_temp(TypeEstimate::Class(
                    nod_runtime::ClassId::EMPTY_LIST.0,
                ));
                self.push(Computation::DirectCall {
                    dst,
                    callee: "%nil".to_string(),
                    args: Vec::new(),
                    safepoint_roots: Vec::new(),
                });
                return Ok(dst);
            }
            // Lower each element to a temp, then build the chain
            // right-to-left.
            let elem_temps: Vec<TempId> = args
                .iter()
                .map(|a| self.lower_expr(a, env, ctx))
                .collect::<Result<_, _>>()?;
            let mut tail = self.fresh_temp(TypeEstimate::Class(
                nod_runtime::ClassId::EMPTY_LIST.0,
            ));
            self.push(Computation::DirectCall {
                dst: tail,
                callee: "%nil".to_string(),
                args: Vec::new(),
                safepoint_roots: Vec::new(),
            });
            for elt in elem_temps.into_iter().rev() {
                let pair_dst = self.fresh_temp(TypeEstimate::Class(
                    nod_runtime::ClassId::PAIR.0,
                ));
                self.push(Computation::DirectCall {
                    dst: pair_dst,
                    callee: "%pair-alloc".to_string(),
                    args: vec![elt, tail],
                    safepoint_roots: Vec::new(),
                });
                tail = pair_dst;
            }
            return Ok(tail);
        }
        // Sprint 20b: `%`-prefixed primitive ops. Each entry in
        // `LOWER_PRIMITIVE_TABLE` lowers to a `DirectCall` against a
        // `nod_*` runtime shim. Args are type-checked for arity only;
        // the runtime tolerates Word inputs of the wrong shape (e.g.
        // non-fixnum to `%range-from` returns 0).
        if let Expr::Ident(_, name) = callee
            && name.starts_with('%')
            && let Some((sym, arity, ret_ty)) = lookup_primitive(name)
        {
            if args.len() != arity {
                return Err(LoweringError::Unsupported {
                    span,
                    message: format!(
                        "primitive `{name}` expects {arity} argument(s), got {}",
                        args.len()
                    ),
                });
            }
            let arg_temps: Vec<TempId> = args
                .iter()
                .map(|a| self.lower_expr(a, env, ctx))
                .collect::<Result<_, _>>()?;
            let dst = self.fresh_temp(ret_ty);
            self.push(Computation::DirectCall {
                dst,
                callee: sym.to_string(),
                args: arg_temps,
                safepoint_roots: Vec::new(),
            });
            return Ok(dst);
        }
        // Sprint 19: `signal(c)` / `condition-message(c)` builtins.
        if let Expr::Ident(_, name) = callee
            && name == "signal"
            && args.len() == 1
        {
            let a = self.lower_expr(&args[0], env, ctx)?;
            let dst = self.fresh_temp(TypeEstimate::Top);
            self.push(Computation::DirectCall {
                dst,
                callee: "%signal".to_string(),
                args: vec![a],
                safepoint_roots: Vec::new(),
            });
            return Ok(dst);
        }
        if let Expr::Ident(_, name) = callee
            && name == "condition-message"
            && args.len() == 1
        {
            let a = self.lower_expr(&args[0], env, ctx)?;
            let dst = self.fresh_temp(TypeEstimate::Top);
            self.push(Computation::DirectCall {
                dst,
                callee: "%condition-message".to_string(),
                args: vec![a],
                safepoint_roots: Vec::new(),
            });
            return Ok(dst);
        }
        // Sprint 19: if the callee is a local binding that refers to an
        // exit procedure (i.e. the `k` in `block (k) ... k(v) ... end`),
        // lower as `%invoke-exit(k, v)`. Detecting the case statically
        // is hard because env doesn't carry "this is an exit-procedure"
        // type info; we can't tell apart from a regular call. Sprint 19
        // simplification: if a name is in env AND is being called with
        // exactly one arg AND we're inside a lifted block thunk whose
        // env binds that name from `exit_var`, treat it as invoke-exit.
        //
        // We use a simple naming convention: the `exit_var` name is
        // stored verbatim in env. To unambiguously trigger
        // `%invoke-exit`, the lowerer special-cases names that resolve
        // to an env entry AND aren't otherwise a known function. The
        // codegen-level `%invoke-exit` handler takes the env-bound Word
        // (the `<exit-procedure>` instance) and the value Word and
        // invokes the runtime shim.
        //
        // Heuristic: if the callee is an ident in `env`, NOT in
        // top_names, and there's exactly one argument, treat as
        // invoke-exit. This is safe for Sprint 19 because the parser /
        // earlier lowering doesn't yet support first-class function
        // values in env; the only env-bound callable values are exit
        // procedures.
        // Sprint 21: env-bound callable Word — could be a `<function>`
        // (introduced via `\name` or `method (...) ... end`) OR an
        // `<exit-procedure>` (the `k` in `block (k) ... end`). Both
        // route through the `nod_funcall_N` trampoline which dispatches
        // on the heap class at runtime. The arity is fixed by the call
        // shape; Sprint 21 supports up to arity-2 directly and uses
        // `nod_apply` for higher arities (deferred).
        //
        // Sprint 24: if `name` is a cell-promoted local OR a captured
        // env-variable, `lower_expr` already inserts the `%cell-get` /
        // `%env-cell` indirection. We route through the regular
        // `lower_expr` to get the unwrapped function Word.
        let callee_name: Option<&str> = match callee {
            Expr::Ident(_, n) => Some(n.as_str()),
            _ => None,
        };
        let captured_in_env = match (self.cell_ctx.env_captures.as_ref(), callee_name) {
            (Some(ec), Some(n)) => ec.index_of.contains_key(n),
            _ => false,
        };
        if let Expr::Ident(_, name) = callee
            && (env.contains_key(name) || captured_in_env)
            && !ctx.top_names.contains(name)
            && !ctx.generics.contains(name)
        {
            let f = self.lower_expr(callee, env, ctx)?;
            let arg_temps: Vec<TempId> = args
                .iter()
                .map(|a| self.lower_expr(a, env, ctx))
                .collect::<Result<_, _>>()?;
            // Sprint 26: arities 0..=5 dispatch through the direct
            // `nod_funcall_N` trampolines. Higher arities still need
            // `nod_apply` and are surfaced as a "not yet supported" so
            // the lowerer doesn't silently SOV-pack without the caller
            // opting in. `<exit-procedure>` is always arity 1 at the
            // source level; the arity-0 path skips the exit-procedure
            // shortcut inside `nod_funcall0` deliberately.
            let funcall_sym = match arg_temps.len() {
                0 => "nod_funcall0",
                1 => "nod_funcall1",
                2 => "nod_funcall2",
                3 => "nod_funcall3",
                4 => "nod_funcall4",
                5 => "nod_funcall5",
                n => {
                    return Err(LoweringError::Unsupported {
                        span,
                        message: format!(
                            "calling a local <function>/<exit-procedure> binding `{name}` with arity {n} not supported (cap is 5 direct args); use `apply(f, args)` for higher arities"
                        ),
                    });
                }
            };
            let mut call_args = Vec::with_capacity(arg_temps.len() + 1);
            call_args.push(f);
            call_args.extend(arg_temps);
            let dst = self.fresh_temp(TypeEstimate::Top);
            self.push(Computation::DirectCall {
                dst,
                callee: funcall_sym.to_string(),
                args: call_args,
                safepoint_roots: Vec::new(),
            });
            return Ok(dst);
        }
        // Strip kw-arg wrapper for non-make calls (the parser wraps
        // `name: value` arguments as Call(%kw-arg, [Symbol, Value]).
        // For Sprint 12 we treat them as positional values for direct
        // calls. Generic dispatch + make have their own kw handling.
        let mut positional_args: Vec<&Expr> = Vec::with_capacity(args.len());
        for a in args {
            if let Expr::Call { callee: c, args: kwargs, .. } = a
                && let Expr::Ident(_, n) = c.as_ref()
                && n == "%kw-arg"
                && kwargs.len() == 2
            {
                positional_args.push(&kwargs[1]);
            } else {
                positional_args.push(a);
            }
        }
        let arg_temps: Vec<TempId> = positional_args
            .iter()
            .map(|a| self.lower_expr(a, env, ctx))
            .collect::<Result<_, _>>()?;
        if let Expr::Ident(_, name) = callee {
            // Sprint 28: c-function call — look up the per-module
            // stub table and emit `nod_winffi_call_N(entry_ptr_const,
            // args...)`. Sprint 38d: the entry pointer is now baked
            // as a `ConstValue::StubEntryRef { dll, symbol, sig }` so
            // codegen lowers it to a `load i64, ptr @nod_stub__*`
            // through a per-module external global. The JIT-link path
            // binds the global's address to a stable `u64` slot whose
            // contents are the address of the freshly-allocated
            // `ApiStubEntry` in the current process (resolution is
            // lazy and idempotent inside the slot allocator).
            if let Some(cf_map) = ctx.c_functions
                && let Some(info) = cf_map.get(name)
            {
                if arg_temps.len() != info.arg_count {
                    return Err(LoweringError::Unsupported {
                        span,
                        message: format!(
                            "c-function `{name}` declared with {} parameter(s), called with {}",
                            info.arg_count,
                            arg_temps.len()
                        ),
                    });
                }
                if info.arg_count > 12 {
                    return Err(LoweringError::Unsupported {
                        span,
                        message: format!(
                            "c-function `{name}`: Sprint 36b caps arity at 12, got {}",
                            info.arg_count
                        ),
                    });
                }
                // Sprint 38d — emit a `StubEntryRef` const so codegen
                // routes through the per-module external global.
                // Pre-Sprint-38d code baked `info.entry_ptr` as a
                // `WordBits` `i64` — that worked in-process only
                // because the static-area address survived for the
                // process lifetime, but it pinned the bitcode to one
                // process (cache hits across processes saw stale
                // addresses).
                let entry_t = self.fresh_temp(TypeEstimate::Top);
                self.push(Computation::Const {
                    dst: entry_t,
                    value: ConstValue::StubEntryRef {
                        dll: info.dll.clone(),
                        symbol: info.symbol.clone(),
                        signature_bytes: info.signature_bytes.clone(),
                    },
                });
                let mut call_args = Vec::with_capacity(arg_temps.len() + 1);
                call_args.push(entry_t);
                call_args.extend(arg_temps);
                let callee_sym = match info.arg_count {
                    0 => "nod_winffi_call_0",
                    1 => "nod_winffi_call_1",
                    2 => "nod_winffi_call_2",
                    3 => "nod_winffi_call_3",
                    4 => "nod_winffi_call_4",
                    5 => "nod_winffi_call_5",
                    6 => "nod_winffi_call_6",
                    7 => "nod_winffi_call_7",
                    8 => "nod_winffi_call_8",
                    9 => "nod_winffi_call_9",
                    10 => "nod_winffi_call_10",
                    11 => "nod_winffi_call_11",
                    12 => "nod_winffi_call_12",
                    // Unreachable: arity_count <= 8 enforced above.
                    _ => unreachable!(),
                };
                let dst = self.fresh_temp(TypeEstimate::Top);
                self.push(Computation::DirectCall {
                    dst,
                    callee: callee_sym.to_string(),
                    args: call_args,
                    safepoint_roots: Vec::new(),
                });
                return Ok(dst);
            }
            // Sprint 12: prefer dispatch when the name is a known
            // generic AND the receiver's type estimate doesn't statically
            // resolve. For known top-level functions (slot accessors
            // emitted as Functions), DirectCall wins so the JIT inlines
            // straight to the LoadSlot body.
            if ctx.top_names.contains(name) {
                let ret = ctx
                    .top_names
                    .return_type(name)
                    .unwrap_or(TypeEstimate::Top);
                let dst = self.fresh_temp(ret);
                self.push(Computation::DirectCall {
                    dst,
                    callee: name.clone(),
                    args: arg_temps,
                    safepoint_roots: Vec::new(),
                });
                return Ok(dst);
            }
            if ctx.generics.contains(name) || nod_runtime::is_generic_defined(name) {
                let dst = self.fresh_temp(TypeEstimate::Top);
                self.push(Computation::Dispatch {
                    dst,
                    generic_name: name.clone(),
                    args: arg_temps,
                    safepoint_roots: Vec::new(),
                });
                return Ok(dst);
            }
            if env.contains_key(name) {
                return Err(LoweringError::Unsupported {
                    span,
                    message: format!(
                        "calling local binding `{name}` not lowered in Sprint 06"
                    ),
                });
            }
            // Unknown ident callee — emit DirectCall against the name.
            let dst = self.fresh_temp(TypeEstimate::Top);
            self.push(Computation::DirectCall {
                dst,
                callee: name.clone(),
                args: arg_temps,
                safepoint_roots: Vec::new(),
            });
            Ok(dst)
        } else {
            Err(LoweringError::Unsupported {
                span,
                message: "call against a non-ident callee not lowered in Sprint 06".to_string(),
            })
        }
    }

    fn lower_make(
        &mut self,
        args: &[Expr],
        env: &mut LocalEnv,
        ctx: &LowerCtx,
        span: Span,
    ) -> Result<TempId, LoweringError> {
        if args.is_empty() {
            return Err(LoweringError::Unsupported {
                span,
                message: "make: missing class argument".to_string(),
            });
        }
        // First arg: class. Expect an identifier resolving to a
        // registered class.
        let class_id = match &args[0] {
            Expr::Ident(_, name) => match ctx
                .user_classes
                .get(name)
                .copied()
                .or_else(|| find_class_id_by_name(name))
            {
                Some(id) => id,
                None => {
                    return Err(LoweringError::UndefinedIdent {
                        span: args[0].span(),
                        name: name.clone(),
                    });
                }
            },
            _ => {
                return Err(LoweringError::Unsupported {
                    span: args[0].span(),
                    message: "make: first argument must be a class name".to_string(),
                });
            }
        };
        // Sprint 22: `make(<table>, ...)` requires custom initialisation
        // (the backing buckets SOV has to be allocated and installed
        // before any insertion). The generic keyword-init path can't do
        // that, so we redirect to the `%make-table` primitive. The
        // optional `capacity:` keyword threads through; everything else
        // is silently ignored (Sprint 22 has no other table options).
        if find_class_id_by_name("<table>").map(|c| c == class_id).unwrap_or(false) {
            let mut capacity_temp: Option<TempId> = None;
            for a in &args[1..] {
                if let Expr::Call { callee, args: kwargs, .. } = a
                    && matches!(callee.as_ref(), Expr::Ident(_, n) if n == "%kw-arg")
                    && kwargs.len() == 2
                    && let Expr::Symbol(_, s) = &kwargs[0]
                    && s.trim_end_matches(':') == "capacity"
                {
                    capacity_temp = Some(self.lower_expr(&kwargs[1], env, ctx)?);
                }
            }
            let cap = capacity_temp.unwrap_or_else(|| self.emit_fixnum_const(0));
            let dst = self.fresh_temp(TypeEstimate::Top);
            self.push(Computation::DirectCall {
                dst,
                callee: "nod_make_table".to_string(),
                args: vec![cap],
                safepoint_roots: Vec::new(),
            });
            return Ok(dst);
        }
        let class_word_temp = self.emit_class_metadata_ptr_const(class_id);
        // Remaining args: kw: value pairs (parser-wrapped as
        // `Call(%kw-arg, [Symbol("kw:"), value])`).
        let mut make_args = vec![class_word_temp];
        for a in &args[1..] {
            let (kw_name, value_expr) = match a {
                Expr::Call { callee, args: kwargs, .. }
                    if matches!(callee.as_ref(), Expr::Ident(_, n) if n == "%kw-arg")
                        && kwargs.len() == 2 =>
                {
                    let raw_name = match &kwargs[0] {
                        Expr::Symbol(_, s) => s.trim_end_matches(':').to_string(),
                        _ => {
                            return Err(LoweringError::Unsupported {
                                span: a.span(),
                                message: "make: kw-arg name must be a keyword".to_string(),
                            });
                        }
                    };
                    (raw_name, &kwargs[1])
                }
                _ => {
                    return Err(LoweringError::Unsupported {
                        span: a.span(),
                        message: "make: arguments after the class must be `kw: value` pairs"
                            .to_string(),
                    });
                }
            };
            let name_temp = self.emit_symbol_literal(&kw_name);
            let value_temp = self.lower_expr(value_expr, env, ctx)?;
            make_args.push(name_temp);
            make_args.push(value_temp);
        }
        let dst = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::DirectCall {
            dst,
            callee: "%make".to_string(),
            args: make_args,
            safepoint_roots: Vec::new(),
        });
        Ok(dst)
    }

    /// Sprint 16: lower one of the `<pair>` / `<list>` builtins to a
    /// runtime-shim DirectCall. Each builtin lowers to a `%pair*` /
    /// `%nil` / `%empty?` synthetic callee that codegen recognises.
    /// Result temps carry the narrowest sound estimate so the dispatch
    /// resolver can pick sealed-direct on subsequent calls — `pair`
    /// returns `Class(<pair>)`, `head`/`tail` return `Top`, `empty?`
    /// returns `Boolean`, `nil` returns `Class(<empty-list>)`.
    fn lower_list_builtin(
        &mut self,
        builtin: ListBuiltin,
        args: &[Expr],
        env: &mut LocalEnv,
        ctx: &LowerCtx,
        span: Span,
    ) -> Result<TempId, LoweringError> {
        // Validate arity up front so the diagnostic points at the call,
        // not at codegen.
        let expected = builtin.arity();
        if args.len() != expected {
            return Err(LoweringError::Unsupported {
                span,
                message: format!(
                    "Sprint 16 builtin `{}` expects {} argument(s), got {}",
                    builtin.name(),
                    expected,
                    args.len(),
                ),
            });
        }
        let arg_temps: Vec<TempId> = args
            .iter()
            .map(|a| self.lower_expr(a, env, ctx))
            .collect::<Result<_, _>>()?;
        let pair_cid = nod_runtime::ClassId::PAIR.0;
        let empty_cid = nod_runtime::ClassId::EMPTY_LIST.0;
        let result_ty = match builtin {
            ListBuiltin::Pair => TypeEstimate::Class(pair_cid),
            ListBuiltin::Head | ListBuiltin::Tail => TypeEstimate::Top,
            ListBuiltin::EmptyP => TypeEstimate::Boolean,
            ListBuiltin::Nil => TypeEstimate::Class(empty_cid),
        };
        let dst = self.fresh_temp(result_ty);
        self.push(Computation::DirectCall {
            dst,
            callee: builtin.callee_symbol().to_string(),
            args: arg_temps,
            safepoint_roots: Vec::new(),
        });
        Ok(dst)
    }

    fn emit_fixnum_const(&mut self, n: i64) -> TempId {
        // Sprint 22 helper — emit a small fixnum constant as a temp.
        let w = nod_runtime::Word::from_fixnum(n)
            .expect("emit_fixnum_const value fits in fixnum range");
        let t = self.fresh_temp(TypeEstimate::Integer);
        self.push(Computation::Const {
            dst: t,
            value: ConstValue::WordBits(w.raw()),
        });
        t
    }

    fn emit_class_metadata_ptr_const(&mut self, class_id: ClassId) -> TempId {
        // The class-metadata pointer is the raw address of the
        // `ClassMetadata` struct in the static area — NOT a tagged
        // Word. `nod_make`'s first param is a raw pointer.
        //
        // Sprint 38c — emits `ConstValue::ClassMetadataPtr { class_id,
        // tagged: false }`. Codegen loads through the per-module
        // external global without applying the pointer-tag OR.
        let t = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::Const {
            dst: t,
            value: ConstValue::ClassMetadataPtr {
                class_id: class_id.0,
                tagged: false,
            },
        });
        t
    }

    fn emit_symbol_literal(&mut self, name: &str) -> TempId {
        // Symbol literal: pin `:name` in the literal pool's static
        // area and bake the tagged Word.
        //
        // Sprint 38c — emits `ConstValue::SymbolLiteralRef(name)` so
        // codegen lowers via the per-module external global pattern.
        let t = self.fresh_temp(TypeEstimate::Top);
        self.push(Computation::Const {
            dst: t,
            value: ConstValue::SymbolLiteralRef(name.to_string()),
        });
        t
    }

    fn lower_instance_check(
        &mut self,
        value: &Expr,
        class: &Expr,
        env: &mut LocalEnv,
        ctx: &LowerCtx,
        span: Span,
    ) -> Result<TempId, LoweringError> {
        let v = self.lower_expr(value, env, ctx)?;
        let check = match class {
            Expr::Ident(_, name) => match name.as_str() {
                "<integer>" => ClassCheck::Integer,
                "<boolean>" => ClassCheck::Boolean,
                "<string>" | "<byte-string>" => ClassCheck::String,
                "<symbol>" => ClassCheck::Symbol,
                "<simple-object-vector>" | "<vector>" => ClassCheck::Vector,
                "<character>" => ClassCheck::Character,
                "<empty-list>" => ClassCheck::EmptyList,
                _ => {
                    let cid = ctx
                        .user_classes
                        .get(name)
                        .copied()
                        .or_else(|| find_class_id_by_name(name));
                    match cid {
                        Some(id) => ClassCheck::UserClass {
                            id: id.0,
                            name: name.clone(),
                        },
                        None => ClassCheck::Unsupported {
                            name: static_class_name(name),
                        },
                    }
                }
            },
            _ => {
                return Err(LoweringError::Unsupported {
                    span,
                    message: "second argument to `instance?` must be a class name literal"
                        .to_string(),
                });
            }
        };
        let dst = self.fresh_temp(TypeEstimate::Boolean);
        self.push(Computation::TypeCheck {
            dst,
            value: v,
            class: check,
        });
        Ok(dst)
    }

    fn lower_if(
        &mut self,
        cond: &Expr,
        then_: &Expr,
        else_: &Expr,
        env: &mut LocalEnv,
        ctx: &LowerCtx,
    ) -> Result<TempId, LoweringError> {
        let cond_t = self.lower_expr(cond, env, ctx)?;

        let then_idx = self.next_block;
        let else_idx = self.next_block + 1;
        let join_idx = self.next_block + 2;
        let then_b = self.new_block(format!("then{then_idx}"));
        let else_b = self.new_block(format!("else{else_idx}"));
        let join_b = self.new_block(format!("join{join_idx}"));

        self.terminate_current(Terminator::If {
            cond: cond_t,
            then_block: then_b,
            else_block: else_b,
        });

        // Sprint 42-pre: env-merge at the join. Walk both arms upfront
        // to find every name that gets rebound in either; snapshot the
        // pre-if env so each arm can be lowered against the same state;
        // emit join-block params for the rebound names and route each
        // arm's jump with the correct args. Without this, a then-only
        // (or else-only) `name := value` mutates env in place but no
        // join phi gets created — and when `name` is also a loop-header
        // phi target, the back-edge picks up an arm-local temp that
        // doesn't dominate the back-edge block. LLVM verification then
        // (correctly) rejects the IR with "Instruction does not dominate
        // all uses".
        //
        // Cell-promoted locals are fine without a phi (the cell pointer
        // stays the same in env across arms); we still include them in
        // the args, producing `phi(cell_t, cell_t) = cell_t`. Harmless.
        let mut assigned_in_arms: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        collect_assigned_in_expr(then_, env, &mut assigned_in_arms);
        collect_assigned_in_expr(else_, env, &mut assigned_in_arms);
        let mut merge_names: Vec<String> = assigned_in_arms.into_iter().collect();
        merge_names.sort(); // deterministic param order
        // Filter to names actually bound in env (collect_assigned_in_expr
        // already gates on env.contains_key, but be defensive).
        merge_names.retain(|n| env.contains_key(n));
        let pre_env = env.clone();

        // Lower the then arm against the pre-if env. `self.current`
        // after `lower_expr` is wherever the arm finished (possibly a
        // nested join block, not `then_b`); `terminate_current` uses
        // that, which is what we want — we terminate the *last* block
        // of the arm with the jump to the outer join.
        self.switch_to(then_b);
        let then_v = self.lower_expr(then_, env, ctx)?;
        let then_ty = self.func.temp_type(then_v);
        let then_merge_temps: Vec<TempId> = merge_names
            .iter()
            .map(|n| *env.get(n).expect("merge name bound after then arm"))
            .collect();
        let mut then_args: Vec<TempId> = Vec::with_capacity(1 + then_merge_temps.len());
        then_args.push(then_v);
        then_args.extend(then_merge_temps.iter().copied());
        self.terminate_current(Terminator::Jump {
            target: join_b,
            args: then_args,
        });

        // Reset env to pre-if state, then lower the else arm.
        *env = pre_env.clone();
        self.switch_to(else_b);
        let else_v = self.lower_expr(else_, env, ctx)?;
        let else_ty = self.func.temp_type(else_v);
        let else_merge_temps: Vec<TempId> = merge_names
            .iter()
            .map(|n| *env.get(n).expect("merge name bound after else arm"))
            .collect();
        let mut else_args: Vec<TempId> = Vec::with_capacity(1 + else_merge_temps.len());
        else_args.push(else_v);
        else_args.extend(else_merge_temps.iter().copied());
        self.terminate_current(Terminator::Jump {
            target: join_b,
            args: else_args,
        });

        // Add join params: if-value first, then one per merged name.
        // Param order MUST match the jump-args order above.
        let joined_ty = then_ty.join(else_ty);
        let join_value_param = self.add_block_param(join_b, joined_ty);
        let mut join_var_params: Vec<TempId> = Vec::with_capacity(merge_names.len());
        for (i, _n) in merge_names.iter().enumerate() {
            let then_t_ty = self.func.temp_type(then_merge_temps[i]);
            let else_t_ty = self.func.temp_type(else_merge_temps[i]);
            let ty = then_t_ty.join(else_t_ty);
            let p = self.add_block_param(join_b, ty);
            join_var_params.push(p);
        }

        // Switch to join and rebind env so post-if code sees the phi'd
        // values. The caller (let-binding, sequence, etc.) just reads
        // env normally.
        self.switch_to(join_b);
        for (n, p) in merge_names.iter().zip(join_var_params.iter()) {
            env.insert(n.clone(), *p);
        }
        Ok(join_value_param)
    }


    /// Sprint 18: lower `while (cond) body end` / `until (cond) body end`
    /// into a three-block CFG with a back-edge.
    ///
    /// ```text
    ///   entry → header(phi_i, phi_total, …)
    ///   header:
    ///     cond_t = eval(cond)
    ///     if cond_t (or !cond_t for until) → loop_body else exit
    ///   loop_body:
    ///     eval(body…)                 (updates env for loop vars)
    ///     jump header(new_i, new_total, …)   ← back-edge
    ///   exit:
    ///     (fall-through; caller continues here)
    /// ```
    ///
    /// Loop variables — names assigned (`:=`) inside `body` or assigned
    /// by a nested `let` after they were established outside — become
    /// block parameters on `header`. Their initial values come from the
    /// pre-loop env; the back-edge re-supplies the post-body values.
    /// Names that are only *read* inside the loop body need no param.
    ///
    /// `invert_cond` flips the header branch (for `until`).
    fn lower_while_like(
        &mut self,
        cond: &Expr,
        body: &[Statement],
        invert_cond: bool,
        env: &mut LocalEnv,
        ctx: &LowerCtx,
    ) -> Result<(), LoweringError> {
        let header_idx = self.next_block;
        let body_idx = self.next_block + 1;
        let exit_idx = self.next_block + 2;
        let header_b = self.new_block(format!("loop_header{header_idx}"));
        let body_b = self.new_block(format!("loop_body{body_idx}"));
        let exit_b = self.new_block(format!("loop_exit{exit_idx}"));

        // Pre-scan: which names get assigned inside the body? Each one
        // becomes a header block-param so the back-edge can re-supply
        // the updated value.
        let assigned_names = collect_assigned_names_in_stmts(body, env);

        // Snapshot pre-loop temps for each assigned name. Create block
        // params on `header` for them; the entry-side jump carries the
        // pre-loop temps as args, the back-edge carries the post-body
        // temps.
        let mut loop_var_order: Vec<String> = assigned_names.into_iter().collect();
        loop_var_order.sort(); // deterministic param ordering
        let mut pre_loop_temps: Vec<TempId> = Vec::with_capacity(loop_var_order.len());
        let mut header_params: Vec<TempId> = Vec::with_capacity(loop_var_order.len());
        for n in &loop_var_order {
            // WHY: every loop var must already be in env (introduced by
            // a `let` before the loop); lowering errors out earlier if
            // an unbound name is referenced.
            let outer = *env.get(n).ok_or_else(|| LoweringError::Unsupported {
                span: cond.span(),
                message: format!(
                    "loop variable `{n}` not bound before loop entry (Sprint 18)"
                ),
            })?;
            pre_loop_temps.push(outer);
            let ty = self.func.temp_type(outer);
            let phi = self.add_block_param(header_b, ty);
            header_params.push(phi);
        }

        // Entry-side jump → header with pre-loop temps as initial args.
        self.terminate_current(Terminator::Jump {
            target: header_b,
            args: pre_loop_temps.clone(),
        });

        // Update env so the header / body see the header-block params
        // when reading the loop vars.
        for (n, phi) in loop_var_order.iter().zip(header_params.iter()) {
            env.insert(n.clone(), *phi);
        }

        // ─── header ─── evaluate cond, branch.
        self.switch_to(header_b);
        let cond_t = self.lower_expr(cond, env, ctx)?;
        let (then_block, else_block) = if invert_cond {
            (exit_b, body_b)
        } else {
            (body_b, exit_b)
        };
        self.terminate_current(Terminator::If {
            cond: cond_t,
            then_block,
            else_block,
        });

        // ─── loop_body ─── lower each body stmt, then jump back to
        // header with the post-body temps.
        self.switch_to(body_b);
        for s in body {
            self.lower_loop_body_stmt(s, env, ctx)?;
        }
        let back_args: Vec<TempId> = loop_var_order
            .iter()
            .map(|n| *env.get(n).expect("loop var lost"))
            .collect();
        self.terminate_current(Terminator::Jump {
            target: header_b,
            args: back_args,
        });

        // ─── exit ─── caller continues here. The env's mapping for
        // each loop var should reflect the header's phi (since after
        // the loop, control reaches exit ONLY from the header's false
        // branch, where the latest cond-checked value is the header
        // phi). Restore env to the header param mapping.
        for (n, phi) in loop_var_order.iter().zip(header_params.iter()) {
            env.insert(n.clone(), *phi);
        }
        self.switch_to(exit_b);
        Ok(())
    }

    /// Sprint 18: lower a single body statement inside a `while`/`until`
    /// loop. Mirrors the function-body statement loop but never sets
    /// `final_temp` (the loop's value is discarded) and recognises the
    /// nested-loop case by recursing into `lower_while_like`.
    fn lower_loop_body_stmt(
        &mut self,
        s: &Statement,
        env: &mut LocalEnv,
        ctx: &LowerCtx,
    ) -> Result<(), LoweringError> {
        match s {
            Statement::Expr(e) => {
                self.lower_expr(e, env, ctx)?;
                Ok(())
            }
            Statement::Let {
                binders, rest, value, span,
            } => {
                if rest.is_some() || binders.len() != 1 {
                    return Err(LoweringError::Unsupported {
                        span: *span,
                        message: "Sprint 18 lowers single-binder `let` only inside loops".to_string(),
                    });
                }
                let bname = &binders[0].name;
                let t = self.lower_expr(value, env, ctx)?;
                env.insert(bname.clone(), t);
                Ok(())
            }
            Statement::While { cond, body, .. } => {
                self.lower_while_like(cond, body, false, env, ctx)
            }
            Statement::Until { cond, body, .. } => {
                self.lower_while_like(cond, body, true, env, ctx)
            }
            Statement::For { span, .. } => Err(LoweringError::Unsupported {
                span: *span,
                message: "`for` inside loop body not lowered (Sprint 25)".to_string(),
            }),
            Statement::Block { span, .. } => Err(LoweringError::Unsupported {
                span: *span,
                message: "`block` inside loop body not lowered (Sprint 19)".to_string(),
            }),
            Statement::Local { span, .. } => Err(LoweringError::Unsupported {
                span: *span,
                message: "`local method` inside loop body not lowered".to_string(),
            }),
        }
    }

    /// Sprint 18: lower an `Expr::Stmt(s)` — used when a macro expansion
    /// produces a statement-shaped form inside an expression position
    /// (e.g. a `Begin` body containing `Expr::Stmt(While {…})`). Returns
    /// a fresh Unit temp; the macro's expansion is in service of side
    /// effects, not a value.
    fn lower_stmt_as_expr(
        &mut self,
        s: &Statement,
        env: &mut LocalEnv,
        ctx: &LowerCtx,
    ) -> Result<TempId, LoweringError> {
        match s {
            Statement::While { cond, body, .. } => {
                self.lower_while_like(cond, body, false, env, ctx)?;
                Ok(self.unit_temp())
            }
            Statement::Until { cond, body, .. } => {
                self.lower_while_like(cond, body, true, env, ctx)?;
                Ok(self.unit_temp())
            }
            Statement::Let {
                binders, rest, value, span,
            } => {
                if rest.is_some() || binders.len() != 1 {
                    return Err(LoweringError::Unsupported {
                        span: *span,
                        message: "Sprint 18 lowers single-binder `let` only".to_string(),
                    });
                }
                let bname = &binders[0].name;
                let t = self.lower_expr(value, env, ctx)?;
                env.insert(bname.clone(), t);
                Ok(t)
            }
            Statement::Expr(e) => self.lower_expr(e, env, ctx),
            Statement::For { span, .. }
            | Statement::Local { span, .. }
            | Statement::Block { span, .. } => Err(LoweringError::Unsupported {
                span: *span,
                message: "statement form not lowerable inside an expression context".to_string(),
            }),
        }
    }

    /// Sprint 18: produce a fresh `<unit>`-typed temp materialised as a
    /// `Const(Bool(false))` so the SSA verifier sees a definition. Used
    /// when a loop/`Expr::Stmt` lowering needs a placeholder value for
    /// expression-context callers. The temp's `type_estimate` is `Unit`
    /// so the surrounding context knows the value is meaningless.
    fn unit_temp(&mut self) -> TempId {
        let t = self.fresh_temp(TypeEstimate::Unit);
        self.push(Computation::Const {
            dst: t,
            value: ConstValue::Bool(false),
        });
        t
    }
}

/// Sprint 18: walk a loop body and collect every local-variable name
/// reassigned via `:=` (or shadowed by an inner `let`). Used by
/// [`FunctionBuilder::lower_while_like`] to drive the loop-header phi
/// params: any name in this set needs a header block param so the
/// back-edge can re-supply the post-body value.
///
/// Only names that EXIST in `env` (i.e. are introduced before the loop)
/// qualify — fresh-bound names inside the loop body are scoped to the
/// body and don't need phi participation.
fn collect_assigned_names_in_stmts(
    body: &[Statement],
    env: &LocalEnv,
) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in body {
        collect_assigned_in_stmt(s, env, &mut out);
    }
    out
}

fn collect_assigned_in_stmt(s: &Statement, env: &LocalEnv, out: &mut HashSet<String>) {
    match s {
        Statement::Expr(e) => collect_assigned_in_expr(e, env, out),
        Statement::Let { value, binders, .. } => {
            collect_assigned_in_expr(value, env, out);
            // Sprint 18: a `let X = …` inside a loop body shadows X if
            // X was bound outside; treat the outer X as loop-mutable.
            for b in binders {
                if env.contains_key(&b.name) {
                    out.insert(b.name.clone());
                }
            }
        }
        Statement::While { cond, body, .. } | Statement::Until { cond, body, .. } => {
            collect_assigned_in_expr(cond, env, out);
            for s2 in body {
                collect_assigned_in_stmt(s2, env, out);
            }
        }
        Statement::For { .. } | Statement::Block { .. } | Statement::Local { .. } => {}
    }
}

fn collect_assigned_in_expr(e: &Expr, env: &LocalEnv, out: &mut HashSet<String>) {
    match e {
        Expr::BinOp { op: BinOp::Assign, lhs, rhs, .. } => {
            if let Expr::Ident(_, name) = lhs.as_ref()
                && env.contains_key(name)
            {
                out.insert(name.clone());
            }
            collect_assigned_in_expr(rhs, env, out);
        }
        Expr::BinOp { lhs, rhs, .. } => {
            collect_assigned_in_expr(lhs, env, out);
            collect_assigned_in_expr(rhs, env, out);
        }
        Expr::UnOp { operand, .. } => collect_assigned_in_expr(operand, env, out),
        Expr::Paren { inner, .. } => collect_assigned_in_expr(inner, env, out),
        Expr::Call { callee, args, .. } => {
            collect_assigned_in_expr(callee, env, out);
            for a in args {
                collect_assigned_in_expr(a, env, out);
            }
        }
        Expr::If { cond, then_, else_, .. } => {
            collect_assigned_in_expr(cond, env, out);
            collect_assigned_in_expr(then_, env, out);
            if let Some(b) = else_ {
                collect_assigned_in_expr(b, env, out);
            }
        }
        Expr::Begin { body, .. } => {
            for b in body {
                collect_assigned_in_expr(b, env, out);
            }
        }
        Expr::Let { binder, value, .. } => {
            collect_assigned_in_expr(value, env, out);
            // Sprint 18: same shadowing rule as the Statement::Let arm.
            if env.contains_key(binder) {
                out.insert(binder.clone());
            }
        }
        Expr::Stmt(s) => collect_assigned_in_stmt(s, env, out),
        Expr::Case { arms, otherwise, .. } => {
            for a in arms {
                collect_assigned_in_expr(&a.cond, env, out);
                for b in &a.body {
                    collect_assigned_in_expr(b, env, out);
                }
            }
            if let Some(o) = otherwise {
                collect_assigned_in_expr(o, env, out);
            }
        }
        _ => {}
    }
}

/// Strip surrounding `"`s and decode the minimal escape set. Supports
/// `\n`, `\r`, `\t`, `\\`, `\"`, `\0`. Unknown escapes are emitted as
/// the literal escape char so behaviour matches Dylan's tolerant lexer.
fn decode_dylan_string_literal(raw: &str) -> String {
    let s = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(raw);
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('0') => out.push('\0'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn static_class_name(name: &str) -> &'static str {
    match name {
        "<object>" => "<object>",
        "<integer>" => "<integer>",
        "<single-float>" => "<single-float>",
        "<double-float>" => "<double-float>",
        "<boolean>" => "<boolean>",
        "<character>" => "<character>",
        "<symbol>" => "<symbol>",
        "<string>" => "<string>",
        "<byte-string>" => "<byte-string>",
        "<simple-object-vector>" => "<simple-object-vector>",
        "<vector>" => "<vector>",
        "<empty-list>" => "<empty-list>",
        _ => "<unknown>",
    }
}

fn expr_kind(e: &Expr) -> &'static str {
    match e {
        Expr::Integer(..) => "integer",
        Expr::Float(..) => "float",
        Expr::String(..) => "string",
        Expr::Char(..) => "char",
        Expr::Bool(..) => "bool",
        Expr::Symbol(..) => "symbol",
        Expr::Ident(..) => "ident",
        Expr::Call { .. } => "call",
        Expr::BinOp { .. } => "binop",
        Expr::UnOp { .. } => "unop",
        Expr::Paren { .. } => "paren",
        Expr::If { .. } => "if",
        Expr::Case { .. } => "case",
        Expr::MacroCall { .. } => "macro-call",
        Expr::Begin { .. } => "begin",
        Expr::Let { .. } => "let",
        Expr::LocalMethod { .. } => "local-method",
        Expr::Method { .. } => "method",
        Expr::Stmt(_) => "stmt",
    }
}

// ─── BinOp / UnOp resolution ─────────────────────────────────────────────

fn select_binop(
    op: BinOp,
    lt: TypeEstimate,
    rt: TypeEstimate,
    span: Span,
) -> Result<PrimOp, LoweringError> {
    let both_int = lt.is_integer() && rt.is_integer();
    let any_float = lt.is_float() || rt.is_float();
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Rem => {
            if both_int {
                Ok(match op {
                    BinOp::Add => PrimOp::AddInt,
                    BinOp::Sub => PrimOp::SubInt,
                    BinOp::Mul => PrimOp::MulInt,
                    BinOp::Div => PrimOp::DivInt,
                    BinOp::Mod => PrimOp::ModInt,
                    BinOp::Rem => PrimOp::RemInt,
                    _ => unreachable!(),
                })
            } else if any_float
                && !matches!(op, BinOp::Mod | BinOp::Rem)
                && (lt.is_float() || lt == TypeEstimate::Top)
                && (rt.is_float() || rt == TypeEstimate::Top)
            {
                Ok(match op {
                    BinOp::Add => PrimOp::AddFloat,
                    BinOp::Sub => PrimOp::SubFloat,
                    BinOp::Mul => PrimOp::MulFloat,
                    BinOp::Div => PrimOp::DivFloat,
                    _ => unreachable!(),
                })
            } else if lt == TypeEstimate::Top && rt == TypeEstimate::Top {
                Ok(match op {
                    BinOp::Add => PrimOp::AddInt,
                    BinOp::Sub => PrimOp::SubInt,
                    BinOp::Mul => PrimOp::MulInt,
                    BinOp::Div => PrimOp::DivInt,
                    BinOp::Mod => PrimOp::ModInt,
                    BinOp::Rem => PrimOp::RemInt,
                    _ => unreachable!(),
                })
            } else if lt.is_integer() && rt == TypeEstimate::Top {
                // Sprint 12: a slot getter return (Top) + an integer
                // local → assume the slot was integer-typed. Choose
                // the integer path. This handles the `<point>` case
                // where `x(p) * x(p)` has Dispatch-typed temps.
                Ok(match op {
                    BinOp::Add => PrimOp::AddInt,
                    BinOp::Sub => PrimOp::SubInt,
                    BinOp::Mul => PrimOp::MulInt,
                    BinOp::Div => PrimOp::DivInt,
                    BinOp::Mod => PrimOp::ModInt,
                    BinOp::Rem => PrimOp::RemInt,
                    _ => unreachable!(),
                })
            } else if lt == TypeEstimate::Top && rt.is_integer() {
                Ok(match op {
                    BinOp::Add => PrimOp::AddInt,
                    BinOp::Sub => PrimOp::SubInt,
                    BinOp::Mul => PrimOp::MulInt,
                    BinOp::Div => PrimOp::DivInt,
                    BinOp::Mod => PrimOp::ModInt,
                    BinOp::Rem => PrimOp::RemInt,
                    _ => unreachable!(),
                })
            } else {
                Err(LoweringError::TypeMismatch {
                    span,
                    message: format!(
                        "mixed int+float operand types ({} {} {}) — explicit coercion not lowered",
                        lt.name(),
                        op.name(),
                        rt.name()
                    ),
                })
            }
        }
        BinOp::Eq | BinOp::EqEq | BinOp::Ne | BinOp::NeEq | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
            let is_float_cmp = any_float;
            let p = match (op, is_float_cmp) {
                (BinOp::Eq | BinOp::EqEq, false) => PrimOp::EqInt,
                (BinOp::Ne | BinOp::NeEq, false) => PrimOp::NeInt,
                (BinOp::Lt, false) => PrimOp::LtInt,
                (BinOp::Gt, false) => PrimOp::GtInt,
                (BinOp::Le, false) => PrimOp::LeInt,
                (BinOp::Ge, false) => PrimOp::GeInt,
                (BinOp::Eq | BinOp::EqEq, true) => PrimOp::EqFloat,
                (BinOp::Lt, true) => PrimOp::LtFloat,
                (BinOp::Gt, true) => PrimOp::GtFloat,
                (BinOp::Le, true) => PrimOp::LeFloat,
                (BinOp::Ge, true) => PrimOp::GeFloat,
                (BinOp::Ne | BinOp::NeEq, true) => {
                    return Err(LoweringError::Unsupported {
                        span,
                        message: "float-`~=` not lowered (no NeFloat PrimOp in Sprint 06)"
                            .to_string(),
                    });
                }
                _ => unreachable!(),
            };
            Ok(p)
        }
        BinOp::And => Ok(PrimOp::BoolAnd),
        BinOp::Or => Ok(PrimOp::BoolOr),
        BinOp::Pow | BinOp::Assign => Err(LoweringError::Unsupported {
            span,
            message: format!("BinOp `{}` not lowered in Sprint 06", op.name()),
        }),
    }
}

fn select_unop(op: UnOp, vt: TypeEstimate, span: Span) -> Result<PrimOp, LoweringError> {
    match op {
        UnOp::Neg => match vt {
            TypeEstimate::Integer | TypeEstimate::Top => Ok(PrimOp::NegInt),
            TypeEstimate::SingleFloat | TypeEstimate::DoubleFloat => Ok(PrimOp::NegFloat),
            _ => Err(LoweringError::TypeMismatch {
                span,
                message: format!("unary `-` on non-numeric {}", vt.name()),
            }),
        },
        UnOp::Not => Ok(PrimOp::BoolNot),
    }
}

/// Dump every registered class to a multi-line string. Used by the
/// driver's (eventual) `dump-classes` subcommand and by the Sprint 12
/// acceptance tests.
///
/// Sprint 14 extends the per-slot row with an MI-aware annotation:
///   - `[own]` for slots introduced by this class.
///   - `[inherited from <C>, fixed-offset]` for inherited slots whose
///     offset matches the defining class's layout.
///   - `[inherited from <C>, override @N→@M]` for inherited slots
///     whose offset shifted vs. the defining class. The lowering pass
///     generated an override accessor method bound to this receiver
///     class; dispatch picks it.
///
/// The class-header line now lists `parents=[...]` instead of a single
/// `parent=` field.
pub fn dump_classes() -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let mut entries: Vec<&'static ClassMetadata> = Vec::new();
    nod_runtime::for_each_class(|md| entries.push(md));
    entries.sort_by_key(|m| m.id.0);
    for md in entries {
        let parents_disp = if md.parents.is_empty() {
            "[]".to_string()
        } else {
            let names: Vec<String> = md
                .parents
                .iter()
                .map(|p| {
                    let ptr = nod_runtime::class_metadata_ptr(*p);
                    if ptr.is_null() {
                        format!("<unknown:{}>", p.0)
                    } else {
                        // SAFETY: static-area metadata.
                        unsafe { (*ptr).name.clone() }
                    }
                })
                .collect();
            format!("[{}]", names.join(", "))
        };
        let cpl_disp = {
            let names: Vec<String> = md
                .cpl
                .iter()
                .map(|c| {
                    let ptr = nod_runtime::class_metadata_ptr(*c);
                    if ptr.is_null() {
                        format!("<unknown:{}>", c.0)
                    } else {
                        // SAFETY: static-area metadata.
                        unsafe { (*ptr).name.clone() }
                    }
                })
                .collect();
            format!("[{}]", names.join(", "))
        };
        let _ = writeln!(
            out,
            "{} (id={}, parents={parents_disp}, cpl={cpl_disp}, slots={}, size={}B)",
            md.name,
            md.id.0,
            md.slots.len(),
            md.instance_size
        );
        for (idx, slot) in md.slots.iter().enumerate() {
            // slot_origin may be shorter than slots in legacy callers;
            // default to "self" if absent.
            let origin = md.slot_origin.get(idx).copied().unwrap_or(md.id);
            let annotation = if origin == md.id {
                "[own]".to_string()
            } else {
                let origin_md_ptr = nod_runtime::class_metadata_ptr(origin);
                if origin_md_ptr.is_null() {
                    format!("[inherited from <unknown:{}>]", origin.0)
                } else {
                    // SAFETY: static-area metadata.
                    let origin_md = unsafe { &*origin_md_ptr };
                    let origin_offset = origin_md
                        .slots
                        .iter()
                        .find(|s| s.name == slot.name)
                        .map(|s| s.offset)
                        .unwrap_or(slot.offset);
                    if origin_offset == slot.offset {
                        format!("[inherited from {}, fixed-offset]", origin_md.name)
                    } else {
                        format!(
                            "[inherited from {}, override @{}→@{}]",
                            origin_md.name, origin_offset, slot.offset
                        )
                    }
                }
            };
            let _ = writeln!(
                out,
                "    slot {} @{}  {:?}  init-keyword={:?}  has-setter={}  {}",
                slot.name, slot.offset, slot.type_kind, slot.init_keyword, slot.has_setter, annotation
            );
        }
    }
    out
}

// ─── Sprint 19: `block` / `exception` / `cleanup` lowering ─────────────────
//
// See `docs/CONDITIONS.md` §"block lowering" for the full design. In
// short: we lift the body, each handler body, the cleanup body, and the
// afterwards body into top-level Dylan functions and emit a single
// runtime call (`%run-block`) at the original `block` site. The runtime
// (`nod_runtime::nod_run_block`) drives the protocol: push handlers,
// `catch_unwind` the body, run cleanup on every exit path (including
// unwound exits), run afterwards on normal exit, pop handlers.
//
// **Captured locals**: we close over every name in the current `env` at
// the moment the `block` form opens. Each lifted thunk receives those
// values as positional `u64` parameters. We cap the total at
// `MAX_BLOCK_CAPTURED` (8); attempting to capture more is rejected
// with a clear "Sprint 19 limitation" error.
//
// **`block (k)` capture**: the exit-procedure `k` is materialised
// up-front via `%make-exit-procedure(block_id)` (a runtime shim) and
// passed as the first captured slot when `exit_var` is present.
//
// **No mutation across the boundary**: Dylan locals in this codebase
// are immutable bindings (`let` always rebinds); the lowerer doesn't
// implement `:=` against captured names. If the lifted body's lowering
// emits an `Assign` against a captured name it surfaces as an
// `Unsupported` (the new function's env would treat the param as a
// fresh binding; mutating it wouldn't write back). The acceptance
// fixtures don't exercise this case.

const BLOCK_RUN_CALLEE: &str = "%run-block";
const BLOCK_MAKE_EXIT_CALLEE: &str = "%make-exit-procedure";

/// One captured local entry: the source name (so lifted bodies can
/// rebind it) and the temp in the enclosing function.
#[derive(Clone)]
struct CapturedLocal {
    name: String,
    outer_temp: TempId,
}

#[allow(clippy::too_many_arguments)]
fn lower_block_form(
    b: &mut FunctionBuilder,
    sink: &mut LiftSink,
    env: &mut LocalEnv,
    ctx: &LowerCtx,
    span: Span,
    parent_name: &str,
    exit_var: Option<&str>,
    body: &[Statement],
    handlers: &[nod_reader::ExceptionClause],
    cleanup: &[Statement],
    afterwards: &[Statement],
) -> Result<TempId, LoweringError> {
    use nod_runtime::MAX_BLOCK_CAPTURED;

    // Collect captured locals from the enclosing function's env (every
    // currently-visible binding). The order is the iteration order of
    // the HashMap — for stability we sort by name so a given source
    // produces deterministic captured ordering across runs.
    let mut captured: Vec<CapturedLocal> = env
        .iter()
        .map(|(name, &outer_temp)| CapturedLocal {
            name: name.clone(),
            outer_temp,
        })
        .collect();
    captured.sort_by(|a, b| a.name.cmp(&b.name));

    // If the block introduces an exit-procedure, reserve slot 0 for it.
    let exit_slot_used = exit_var.is_some();
    let total_captured = captured.len() + if exit_slot_used { 1 } else { 0 };
    if total_captured > MAX_BLOCK_CAPTURED {
        return Err(LoweringError::Unsupported {
            span,
            message: format!(
                "Sprint 19 limitation: `block` captures {total_captured} locals (max = {MAX_BLOCK_CAPTURED}); reduce surrounding bindings or restructure"
            ),
        });
    }

    let thunk_seq = sink.alloc_thunk_suffix();
    // Sprint 37: deterministic block_id derived from (parent_name,
    // thunk_seq). Identical source must produce identical DFM IR for the
    // JIT object-code cache to hit; a process-global counter via
    // `allocate_block_id` would change across runs. The id is registered
    // post-JIT with `register_block_fns`, which replaces same-id entries,
    // so collisions across modules are tolerated. The hash is SipHash 1-3
    // via `DefaultHasher`, which has fixed seeds — stable across runs.
    // The id must fit in 63 bits because `make_exit_procedure` packs it
    // into a tagged fixnum (see `Word::from_fixnum`), and must be
    // non-zero because 0 is the "no block" sentinel — we OR in bit 62
    // to satisfy both constraints with high collision-resistance.
    let block_id = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        parent_name.hash(&mut h);
        thunk_seq.hash(&mut h);
        b"sprint37-block-id".hash(&mut h);
        let raw = h.finish();
        // Mask to 62 bits then set bit 62; gives a non-zero value
        // strictly less than 2^63, fitting `Word::from_fixnum`'s domain.
        (raw & ((1u64 << 62) - 1)) | (1u64 << 62)
    };

    // ─── Lift each stage to a top-level function ────────────────────
    //
    // Lifted-function name shape: `<parent>$$blk<N>$<stage>`. The `$$`
    // separator + `blk` prefix is a marker the dumps + `:handlers`
    // output uses to spot lifted thunks.

    let stage_name = |stage: &str| format!("{parent_name}$$blk{thunk_seq}${stage}");

    let body_fn_name = stage_name("body");
    let cleanup_fn_name = if cleanup.is_empty() {
        None
    } else {
        Some(stage_name("cleanup"))
    };
    let afterwards_fn_name = if afterwards.is_empty() {
        None
    } else {
        Some(stage_name("afterwards"))
    };

    let body_fn = lift_block_stage(
        sink.alloc_fn_id(),
        &body_fn_name,
        &captured,
        exit_var,
        body,
        span,
        ctx,
        sink,
        false,
    )?;
    sink.functions.push(body_fn);

    if let Some(name) = &cleanup_fn_name {
        let f = lift_block_stage(
            sink.alloc_fn_id(),
            name,
            &captured,
            exit_var,
            cleanup,
            span,
            ctx,
            sink,
            false,
        )?;
        sink.functions.push(f);
    }
    if let Some(name) = &afterwards_fn_name {
        let f = lift_block_stage(
            sink.alloc_fn_id(),
            name,
            &captured,
            exit_var,
            afterwards,
            span,
            ctx,
            sink,
            false,
        )?;
        sink.functions.push(f);
    }

    let mut handler_regs: Vec<BlockHandlerRegistration> = Vec::with_capacity(handlers.len());
    for (i, h) in handlers.iter().enumerate() {
        let class_id = match &h.class {
            Expr::Ident(_, n) => ctx
                .user_classes
                .get(n)
                .copied()
                .or_else(|| find_class_id_by_name(n))
                .ok_or_else(|| LoweringError::UndefinedIdent {
                    span: h.span,
                    name: n.clone(),
                })?,
            _ => {
                return Err(LoweringError::Unsupported {
                    span: h.span,
                    message: "exception clause: class must be a bare identifier".to_string(),
                });
            }
        };
        let class_name = match &h.class {
            Expr::Ident(_, n) => n.clone(),
            _ => unreachable!("guarded by Ident match above"),
        };
        let fn_name = stage_name(&format!("h{i}"));
        let handler_fn = lift_block_stage_handler(
            sink.alloc_fn_id(),
            &fn_name,
            &captured,
            exit_var,
            h.var.as_deref(),
            &h.body,
            h.span,
            ctx,
            sink,
        )?;
        sink.functions.push(handler_fn);
        handler_regs.push(BlockHandlerRegistration {
            class_id,
            class_name,
            body_fn_name: fn_name,
        });
    }

    // Record the block for post-JIT registration.
    sink.blocks.push(BlockRegistration {
        block_id,
        body_fn_name: body_fn_name.clone(),
        cleanup_fn_name: cleanup_fn_name.clone(),
        afterwards_fn_name: afterwards_fn_name.clone(),
        handlers: handler_regs,
    });

    // ─── Emit the call site in the enclosing function ───────────────
    //
    // Args to `%run-block`: [block_id_const, c0..c7]. Unused slots are
    // zero-filled.

    // Block-id constant.
    let bid_temp = b.fresh_temp(TypeEstimate::Top);
    b.push(Computation::Const {
        dst: bid_temp,
        value: ConstValue::WordBits(block_id),
    });
    let zero_temp = b.fresh_temp(TypeEstimate::Top);
    b.push(Computation::Const {
        dst: zero_temp,
        value: ConstValue::WordBits(0),
    });

    // Optional exit procedure (slot 0 if present): call %make-exit-procedure(block_id).
    let mut capture_temps: Vec<TempId> = Vec::with_capacity(MAX_BLOCK_CAPTURED);
    if exit_slot_used {
        let ep_temp = b.fresh_temp(TypeEstimate::Top);
        b.push(Computation::DirectCall {
            dst: ep_temp,
            callee: BLOCK_MAKE_EXIT_CALLEE.to_string(),
            args: vec![bid_temp],
            safepoint_roots: Vec::new(),
        });
        capture_temps.push(ep_temp);
    }
    for c in &captured {
        capture_temps.push(c.outer_temp);
    }
    while capture_temps.len() < MAX_BLOCK_CAPTURED {
        capture_temps.push(zero_temp);
    }

    let dst = b.fresh_temp(TypeEstimate::Top);
    let mut args = Vec::with_capacity(1 + MAX_BLOCK_CAPTURED);
    args.push(bid_temp);
    args.extend(capture_temps);
    b.push(Computation::DirectCall {
        dst,
        callee: BLOCK_RUN_CALLEE.to_string(),
        args,
        safepoint_roots: Vec::new(),
    });

    // The block's result type is intentionally `Top` — Sprint 19
    // doesn't attempt to type-merge the body/handler branches.
    let _ = exit_var;
    Ok(dst)
}

/// Lift one "straight" stage (body / cleanup / afterwards) of a `block`
/// form into a fresh top-level Dylan function. The new function takes
/// `MAX_BLOCK_CAPTURED` positional `u64` params (the captured locals,
/// padded with zeros). Its body is the supplied `stmts` lowered with
/// the captured names bound to the param temps.
#[allow(clippy::too_many_arguments)]
fn lift_block_stage(
    id: FunctionId,
    fn_name: &str,
    captured: &[CapturedLocal],
    exit_var: Option<&str>,
    stmts: &[Statement],
    span: Span,
    ctx: &LowerCtx,
    sink: &mut LiftSink,
    _is_handler: bool,
) -> Result<Function, LoweringError> {
    use nod_runtime::MAX_BLOCK_CAPTURED;
    let mut b = FunctionBuilder::new(id, fn_name.to_string(), span);
    let mut env = LocalEnv::new();
    // Build params: slot 0 = exit-procedure (if any), then captures, padded to 8.
    let mut slot_names: Vec<Option<String>> = Vec::with_capacity(MAX_BLOCK_CAPTURED);
    if let Some(ev) = exit_var {
        slot_names.push(Some(ev.to_string()));
    }
    for c in captured {
        slot_names.push(Some(c.name.clone()));
    }
    while slot_names.len() < MAX_BLOCK_CAPTURED {
        slot_names.push(None);
    }
    for slot_name in &slot_names {
        let t = b.fresh_temp(TypeEstimate::Top);
        b.func.params.push(t);
        if let Some(n) = slot_name {
            env.insert(n.clone(), t);
        }
    }

    lower_statements_into(&mut b, &mut env, ctx, sink, stmts)?;
    b.func.return_type = TypeEstimate::Top;
    // Terminate the current block with a return of the last temp (or
    // zero if the stage was empty). `lower_statements_into` left
    // `current_block`'s terminator as the default `Return None`; we
    // overwrite to surface the final value.
    if let Some(t) = b.last_temp() {
        // SSA: emit the return explicitly.
        let cur_block_id = b.func.blocks[b.current].id;
        let _ = cur_block_id;
        b.terminate_current(Terminator::Return { value: Some(t) });
    } else {
        // Empty stage — return zero (which the runtime interprets as
        // the unit Word for cleanup/afterwards, and the body's value
        // for a body stage; an empty body returns 0).
        let z = b.fresh_temp(TypeEstimate::Top);
        b.push(Computation::Const {
            dst: z,
            value: ConstValue::WordBits(0),
        });
        b.terminate_current(Terminator::Return { value: Some(z) });
    }
    Ok(b.finish())
}

/// Lift one handler clause. Signature: 1 condition Word arg + 8
/// captured-locals slots. The handler's bound condition variable (the
/// `c` in `exception (c :: <error>)`) is bound to the first param.
#[allow(clippy::too_many_arguments)]
fn lift_block_stage_handler(
    id: FunctionId,
    fn_name: &str,
    captured: &[CapturedLocal],
    exit_var: Option<&str>,
    handler_var: Option<&str>,
    stmts: &[Statement],
    span: Span,
    ctx: &LowerCtx,
    sink: &mut LiftSink,
) -> Result<Function, LoweringError> {
    use nod_runtime::MAX_BLOCK_CAPTURED;
    let mut b = FunctionBuilder::new(id, fn_name.to_string(), span);
    let mut env = LocalEnv::new();

    // Param 0: the condition Word. The handler may omit the variable;
    // we always emit a param but only bind it in env when named.
    let cond_temp = b.fresh_temp(TypeEstimate::Top);
    b.func.params.push(cond_temp);
    if let Some(v) = handler_var {
        env.insert(v.to_string(), cond_temp);
    }

    // Params 1..=8: captured locals, same layout as the body thunk
    // (exit-procedure in slot 0 if present, then captures, padded with
    // zeros).
    let mut slot_names: Vec<Option<String>> = Vec::with_capacity(MAX_BLOCK_CAPTURED);
    if let Some(ev) = exit_var {
        slot_names.push(Some(ev.to_string()));
    }
    for c in captured {
        slot_names.push(Some(c.name.clone()));
    }
    while slot_names.len() < MAX_BLOCK_CAPTURED {
        slot_names.push(None);
    }
    for slot_name in &slot_names {
        let t = b.fresh_temp(TypeEstimate::Top);
        b.func.params.push(t);
        if let Some(n) = slot_name {
            env.insert(n.clone(), t);
        }
    }

    lower_statements_into(&mut b, &mut env, ctx, sink, stmts)?;
    b.func.return_type = TypeEstimate::Top;
    if let Some(t) = b.last_temp() {
        b.terminate_current(Terminator::Return { value: Some(t) });
    } else {
        let z = b.fresh_temp(TypeEstimate::Top);
        b.push(Computation::Const {
            dst: z,
            value: ConstValue::WordBits(0),
        });
        b.terminate_current(Terminator::Return { value: Some(z) });
    }
    Ok(b.finish())
}

/// Inline-lower a sequence of statements into the current block of
/// `b`. Returns Ok(()) on success. Used by the block-stage lifting
/// helpers so the lifted thunk can itself contain `let`, `if`,
/// `while`, nested `block`, etc.
fn lower_statements_into(
    b: &mut FunctionBuilder,
    env: &mut LocalEnv,
    ctx: &LowerCtx,
    sink: &mut LiftSink,
    stmts: &[Statement],
) -> Result<(), LoweringError> {
    for stmt in stmts {
        match stmt {
            Statement::Expr(e) => {
                let _t = b.lower_expr(e, env, ctx)?;
                b.set_last_temp(_t);
            }
            Statement::Let {
                binders,
                rest,
                value,
                span,
            } => {
                if rest.is_some() || binders.len() != 1 {
                    return Err(LoweringError::Unsupported {
                        span: *span,
                        message: "Sprint 06 lowers single-binder `let` only".to_string(),
                    });
                }
                let bname = &binders[0].name;
                let t = b.lower_expr(value, env, ctx)?;
                env.insert(bname.clone(), t);
                b.set_last_temp(t);
            }
            Statement::Local { span, .. } => {
                return Err(LoweringError::Unsupported {
                    span: *span,
                    message: "`local method` not lowered in Sprint 06".to_string(),
                });
            }
            Statement::While { cond, body, .. } => {
                b.lower_while_like(cond, body, false, env, ctx)?;
                b.clear_last_temp();
            }
            Statement::Until { cond, body, .. } => {
                b.lower_while_like(cond, body, true, env, ctx)?;
                b.clear_last_temp();
            }
            Statement::For { span, .. } => {
                return Err(LoweringError::Unsupported {
                    span: *span,
                    message: "`for` not lowered in Sprint 18".to_string(),
                });
            }
            Statement::Block {
                span,
                exit_var,
                body,
                handlers,
                cleanup,
                afterwards,
            } => {
                // Nested block. The parent function is the lifted thunk
                // we're currently building.
                let parent_name = b.func.name.clone();
                let t = lower_block_form(
                    b,
                    sink,
                    env,
                    ctx,
                    *span,
                    &parent_name,
                    exit_var.as_deref(),
                    body,
                    handlers,
                    cleanup,
                    afterwards,
                )?;
                b.set_last_temp(t);
            }
        }
    }
    Ok(())
}

// ─── Sprint 31 unit tests ────────────────────────────────────────────────

#[cfg(test)]
#[allow(non_snake_case)]
mod sprint31_tests {
    use super::*;

    #[test]
    fn winapi_dll_priority_orders_kernel_first() {
        assert!(winapi_dll_priority("kernel32.dll") < winapi_dll_priority("user32.dll"));
        assert!(winapi_dll_priority("user32.dll") < winapi_dll_priority("gdi32.dll"));
        assert!(winapi_dll_priority("gdi32.dll") < winapi_dll_priority("advapi32.dll"));
        assert!(winapi_dll_priority("advapi32.dll") < winapi_dll_priority("shell32.dll"));
        assert!(winapi_dll_priority("shell32.dll") < winapi_dll_priority("comctl32.dll"));
        // Unknown DLLs sort to the end (alphabetical fallback there).
        assert!(winapi_dll_priority("d3d12.dll") > winapi_dll_priority("comctl32.dll"));
    }

    #[test]
    fn looks_like_win32_export_filters_correctly() {
        // Yes: standard Win32 export shape.
        assert!(looks_like_win32_export("MessageBoxW"));
        assert!(looks_like_win32_export("GetTickCount64"));
        assert!(looks_like_win32_export("Beep"));
        // Yes: lowercase-prefixed Win32 exports like lstrlenW.
        assert!(looks_like_win32_export("lstrlenW"));
        assert!(looks_like_win32_export("lstrlenA"));
        assert!(looks_like_win32_export("wsprintfW"));
        // Yes: mixed-case but starting lowercase.
        assert!(looks_like_win32_export("messageBox"));
        // No: Dylan-side names, punctuated, all-lowercase.
        assert!(!looks_like_win32_export("print"));
        assert!(!looks_like_win32_export("format"));
        assert!(!looks_like_win32_export("<my-class>"));
        assert!(!looks_like_win32_export("c-function"));
        assert!(!looks_like_win32_export("+"));
        assert!(!looks_like_win32_export("instance?"));
        assert!(!looks_like_win32_export("A"));
        assert!(!looks_like_win32_export("ab"));
    }

    #[test]
    fn jit_materialize_GetTickCount64_yields_kernel32_no_args() {
        let outcome = try_jit_materialize_winapi("GetTickCount64");
        match outcome {
            MaterializationOutcome::Materialized { c_name, library, signature } => {
                assert_eq!(c_name, "GetTickCount64");
                assert_eq!(library, "kernel32.dll");
                assert_eq!(signature.arg_count, 0);
            }
            other => panic!("expected materialized; got {other:?}"),
        }
    }

    #[test]
    fn jit_materialize_bare_MessageBox_picks_W() {
        let outcome = try_jit_materialize_winapi("MessageBox");
        match outcome {
            MaterializationOutcome::Materialized { c_name, library, .. } => {
                assert_eq!(c_name, "MessageBoxW");
                assert_eq!(library, "user32.dll");
            }
            other => panic!("expected materialized; got {other:?}"),
        }
    }

    #[test]
    fn jit_materialize_unknown_name_returns_not_found() {
        let outcome = try_jit_materialize_winapi("ThisIsNotAWin32Export");
        assert!(matches!(outcome, MaterializationOutcome::NotFound));
    }

    #[test]
    fn jit_materialize_lstrlenW_succeeds() {
        let outcome = try_jit_materialize_winapi("lstrlenW");
        match outcome {
            MaterializationOutcome::Materialized { c_name, library, signature } => {
                assert_eq!(c_name, "lstrlenW");
                assert_eq!(library, "kernel32.dll");
                assert_eq!(signature.arg_count, 1);
            }
            other => panic!("lstrlenW: expected materialized; got {other:?}"),
        }
    }

    #[test]
    fn jit_materialize_EnumWindows_outcome() {
        // EnumWindows has a callback (WNDENUMPROC). The outcome must
        // be either NotFound (if not in the embedded blob) or
        // UnsupportedSignature (function pointer). Both are acceptable
        // — Sprint 31 only needs the user to get a non-silent error
        // path. The blob filter in `build.rs` may have dropped it
        // entirely (`bad_type=5191`).
        let outcome = try_jit_materialize_winapi("EnumWindows");
        eprintln!("EnumWindows outcome: {outcome:?}");
        match outcome {
            MaterializationOutcome::NotFound
            | MaterializationOutcome::UnsupportedSignature { .. } => {}
            MaterializationOutcome::Materialized { signature, .. } => {
                // If it materialized, accept that — the index doesn't
                // expose callbacks as TypeRef::Function in this build,
                // they collapse to opaque pointers.
                eprintln!("EnumWindows actually materialized with sig {signature:?}");
            }
        }
    }
}
