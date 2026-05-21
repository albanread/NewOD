//! Sprint 28 — per-module API stub tables + Win64 marshaling
//! trampolines for `define c-function` calls.
//!
//! ## Architecture (from the Sprint 28 brief)
//!
//! Each Dylan module compiled at JIT time gets a **per-module API stub
//! table**. Each unique `(dll, symbol)` referenced in the module's
//! `define c-function`s gets ONE [`ApiStubEntry`]. Multiple call sites
//! for the same API share the same entry — PLT-like deduplication.
//!
//! At JIT-finalize, the runtime walks the table, `LoadLibrary`s each
//! unique DLL, `GetProcAddress`es each symbol, and populates the
//! entry's `fn_ptr` atomically. Lazy/PLT-style on-first-use is a
//! Sprint 38+ optimisation; eager init keeps Sprint 28 small.
//!
//! Per-call codegen lowers `Beep(440, 200)` to a `DirectCall` against
//! a synthetic `%winffi-call-N` callee (N = arg count). Codegen emits
//! `nod_winffi_call_N(entry_ptr_const, a0, …, aN-1)`. The trampoline
//! unboxes each arg per the entry's recorded [`ApiCallSignature`],
//! invokes the function pointer through an `extern "system"` (Win64)
//! signature, and reboxes the return as a Dylan [`Word`].
//!
//! ## Sprint 28 scope
//!
//! - Integer args/returns: `<c-bool>`, `<c-byte>`, `<c-short>`,
//!   `<c-int>`, `<c-long>`, `<c-dword>`, `<c-uint>`, `<c-ulong>`.
//! - Pointer/handle args/returns: `<c-pointer>`, `<c-handle>`.
//! - Up to **8 args per call** (Win64: RCX/RDX/R8/R9 + 4 stack slots).
//!
//! Out of scope (later sprints):
//!
//! - Strings (Sprint 30): no `<c-string>` / `<c-wide-string>` marshalling.
//! - Structs by value (Sprint 34).
//! - Callbacks / function pointers (Sprint 33).
//! - COM interfaces (Sprint 35).
//! - Variadics, structured `GetLastError`, auto-raise on failure.

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};

use crate::classes::ClassId;
use crate::conditions::condition_class_name;
use crate::make::rust_make;
use crate::word::Word;

// ─── Argument / return kinds ──────────────────────────────────────────────

/// Marshaling kind for a single C argument. Stored as `u8` inside
/// [`ApiCallSignature::arg_kinds`] so the whole signature stays
/// `Copy + #[repr(C)]`.
#[repr(u8)]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum CArgKind {
    Void = 0,
    Int8 = 1,
    Int16 = 2,
    Int32 = 3,
    Int64 = 4,
    UInt8 = 5,
    UInt16 = 6,
    UInt32 = 7,
    UInt64 = 8,
    /// `BOOL` is a Win32 32-bit integer; Dylan side maps from
    /// `<boolean>` singletons (`#t` → 1, `#f` → 0) or from a fixnum.
    Bool32 = 9,
    /// Raw `*mut u8` (opaque pointer).
    Pointer = 10,
    /// `HANDLE` — opaque pointer-sized handle. ABI-identical to
    /// [`CArgKind::Pointer`]; kept distinct so error messages /
    /// diagnostics can surface the source type.
    Handle = 11,
}

impl CArgKind {
    fn from_u8(b: u8) -> CArgKind {
        match b {
            0 => CArgKind::Void,
            1 => CArgKind::Int8,
            2 => CArgKind::Int16,
            3 => CArgKind::Int32,
            4 => CArgKind::Int64,
            5 => CArgKind::UInt8,
            6 => CArgKind::UInt16,
            7 => CArgKind::UInt32,
            8 => CArgKind::UInt64,
            9 => CArgKind::Bool32,
            10 => CArgKind::Pointer,
            11 => CArgKind::Handle,
            _ => panic!("nod-runtime/winffi: unknown CArgKind byte {b}"),
        }
    }

    /// Resolve a Dylan c-type class name (e.g. `<c-dword>`) to its
    /// marshaling kind. Sprint 28 panics on unknown names; the Sema
    /// layer is expected to validate names up front. None means the
    /// type isn't in the Sprint 28 supported set.
    pub fn from_c_type_name(name: &str) -> Option<CArgKind> {
        Some(match name {
            "<c-bool>" => CArgKind::Bool32,
            "<c-byte>" => CArgKind::UInt8,
            "<c-short>" => CArgKind::Int16,
            "<c-ushort>" => CArgKind::UInt16,
            "<c-int>" => CArgKind::Int32,
            "<c-uint>" => CArgKind::UInt32,
            "<c-long>" => CArgKind::Int32,
            "<c-ulong>" => CArgKind::UInt32,
            "<c-longlong>" => CArgKind::Int64,
            "<c-ulonglong>" => CArgKind::UInt64,
            "<c-dword>" => CArgKind::UInt32,
            "<c-word>" => CArgKind::Int64,
            "<c-pointer>" => CArgKind::Pointer,
            "<c-handle>" => CArgKind::Handle,
            _ => return None,
        })
    }
}

/// Marshaling kind for a C return value. Same shape as [`CArgKind`]
/// but with a narrower set — there's no return-by-value `Int8`/`Int16`
/// in any Win32 API the Sprint 28 acceptance tests touch (they're
/// promoted to Int32 by the C ABI anyway).
#[repr(u8)]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum CReturnKind {
    Void = 0,
    Int32 = 1,
    Int64 = 2,
    UInt32 = 3,
    UInt64 = 4,
    Bool32 = 5,
    Pointer = 6,
    Handle = 7,
}

impl CReturnKind {
    fn from_u8(b: u8) -> CReturnKind {
        match b {
            0 => CReturnKind::Void,
            1 => CReturnKind::Int32,
            2 => CReturnKind::Int64,
            3 => CReturnKind::UInt32,
            4 => CReturnKind::UInt64,
            5 => CReturnKind::Bool32,
            6 => CReturnKind::Pointer,
            7 => CReturnKind::Handle,
            _ => panic!("nod-runtime/winffi: unknown CReturnKind byte {b}"),
        }
    }

    /// Resolve a Dylan c-type class name to a return kind. Sprint 28
    /// only handles the kinds the acceptance tests use.
    pub fn from_c_type_name(name: &str) -> Option<CReturnKind> {
        Some(match name {
            "<c-bool>" => CReturnKind::Bool32,
            "<c-int>" => CReturnKind::Int32,
            "<c-uint>" => CReturnKind::UInt32,
            "<c-long>" => CReturnKind::Int32,
            "<c-ulong>" => CReturnKind::UInt32,
            "<c-longlong>" => CReturnKind::Int64,
            "<c-ulonglong>" => CReturnKind::UInt64,
            "<c-dword>" => CReturnKind::UInt32,
            "<c-word>" => CReturnKind::Int64,
            "<c-pointer>" => CReturnKind::Pointer,
            "<c-handle>" => CReturnKind::Handle,
            _ => return None,
        })
    }
}

/// Packed Win64 marshaling signature for a single c-function. Stored
/// in [`ApiStubEntry::signature`]; the trampoline reads it to decide
/// how to unbox each arg and rebox the return.
///
/// `#[repr(C)]` so the IR can bake field offsets if needed.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ApiCallSignature {
    /// Number of arguments (0..=8 in Sprint 28). The trampoline at
    /// arity N expects `arg_count == N`.
    pub arg_count: u8,
    /// Packed arg kinds; only the first `arg_count` entries are
    /// meaningful. Indices `arg_count..8` MUST be `CArgKind::Void` (0).
    pub arg_kinds: [u8; 8],
    /// Return value kind.
    pub return_kind: u8,
}

// ─── ApiStubEntry / ApiStubTable ──────────────────────────────────────────

/// One row in a module's API stub table. Pinned in the static area
/// for the module's lifetime so its address can be baked into JIT-
/// emitted constants.
///
/// `#[repr(C)]` keeps the field order stable across Rust versions.
/// The trampolines read `fn_ptr` and `signature` directly off this
/// struct via raw pointer arithmetic.
#[repr(C)]
pub struct ApiStubEntry {
    /// DLL name as a raw UTF-8 byte slice (NOT null-terminated; the
    /// resolver builds its own `CString` on first use). Static-area
    /// storage; valid for the process lifetime.
    pub dll_name_ptr: *const u8,
    pub dll_name_len: u32,
    /// Symbol name, same lifetime + non-null-terminated storage.
    pub symbol_name_ptr: *const u8,
    pub symbol_name_len: u32,
    /// Resolved function pointer, populated at module init via
    /// [`initialize_stub_table`]. Null until then.
    ///
    /// `AtomicPtr` for safe publication across threads. The
    /// trampoline does an `Acquire` load; init does a `Release` store.
    pub fn_ptr: AtomicPtr<u8>,
    /// Marshaling signature for this c-function.
    pub signature: ApiCallSignature,
}

// SAFETY: ApiStubEntry contains only POD + AtomicPtr; the `*const u8`
// pointers point at static-area UTF-8 bytes that live for the process.
unsafe impl Send for ApiStubEntry {}
unsafe impl Sync for ApiStubEntry {}

/// A module's complete API stub table. A `'static` slice of entries
/// (each pinned in the static area). The table itself is also pinned.
pub struct ApiStubTable {
    pub entries: &'static [ApiStubEntry],
}

// ─── Process-wide registry of resolved libraries ──────────────────────────

#[cfg(windows)]
mod resolver {
    use super::*;
    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

    /// (DLL name → HMODULE) cache. Once a DLL is loaded, the handle is
    /// kept alive for the process lifetime; the `Mutex` only protects
    /// the map's structure, not the handles themselves.
    static LOADED_LIBRARIES: OnceLock<Mutex<HashMap<String, isize>>> = OnceLock::new();

    fn libs() -> &'static Mutex<HashMap<String, isize>> {
        LOADED_LIBRARIES.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Look up `symbol` in `dll`, loading the DLL on first reference.
    /// Returns null on failure (callers raise `<c-ffi-error>`).
    ///
    /// On Windows this calls the raw `LoadLibraryA` /
    /// `GetProcAddress` Win32 APIs through `windows-sys` (which
    /// `nod-runtime` already depends on for `Win32_System_Memory`).
    /// We considered `libloading` per the brief; on Windows it adds a
    /// dependency for code that's a thin wrapper over the two APIs we
    /// need, so we use `windows-sys` directly. This keeps the
    /// build-deps footprint identical to Sprint 27. (Documented as a
    /// deliberate deviation from the brief.)
    pub fn resolve_symbol(dll: &str, symbol: &str) -> *const u8 {
        let mut guard = libs().lock().expect("winffi libs poisoned");
        let hmodule: isize = if let Some(&h) = guard.get(dll) {
            h
        } else {
            let Ok(c) = CString::new(dll) else { return std::ptr::null() };
            // SAFETY: LoadLibraryA takes a null-terminated ASCII string.
            // CString::as_ptr returns a stable pointer for the call's
            // duration.
            let h = unsafe { LoadLibraryA(c.as_ptr() as *const u8) };
            if h.is_null() {
                return std::ptr::null();
            }
            guard.insert(dll.to_string(), h as isize);
            h as isize
        };
        let Ok(c) = CString::new(symbol) else { return std::ptr::null() };
        // SAFETY: GetProcAddress takes an HMODULE + null-terminated
        // ASCII symbol name. The HMODULE is the just-cached handle.
        let p = unsafe { GetProcAddress(hmodule as HMODULE, c.as_ptr() as *const u8) };
        match p {
            Some(f) => f as *const u8,
            None => std::ptr::null(),
        }
    }
}

#[cfg(not(windows))]
mod resolver {
    /// Non-Windows builds: `resolve_symbol` always returns null. The
    /// Sprint 28 acceptance tests are `#[cfg(windows)]`; this keeps
    /// the workspace buildable on Linux for CI smoke runs.
    pub fn resolve_symbol(_dll: &str, _symbol: &str) -> *const u8 {
        std::ptr::null()
    }
}

pub use resolver::resolve_symbol;

// ─── Statistics — for the dedupe test ────────────────────────────────────

#[derive(Copy, Clone, Debug, Default)]
pub struct WinFfiStats {
    /// Cumulative number of stub-table entries allocated across every
    /// module the process has lowered.
    pub entries: usize,
    /// Cumulative number of successful `(dll, symbol)` resolutions
    /// performed by `initialize_stub_table`.
    pub total_resolved: usize,
    /// Cumulative number of unique `(dll, symbol)` pairs that have
    /// resolved. This is `<= entries`. For Sprint 28 (one module per
    /// JIT session, eager init) we typically have `entries ==
    /// total_resolved == unique_symbols`.
    pub unique_symbols: usize,
}

static STAT_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static STAT_RESOLVED: AtomicUsize = AtomicUsize::new(0);
static STAT_UNIQUE: AtomicUsize = AtomicUsize::new(0);

static UNIQUE_KEYS: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();

fn unique_keys() -> &'static Mutex<std::collections::HashSet<String>> {
    UNIQUE_KEYS.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// Snapshot the WinFFI stats counters. Used by the
/// `api_stub_table_deduplicates_call_sites` acceptance test.
pub fn winffi_stats() -> WinFfiStats {
    WinFfiStats {
        entries: STAT_ENTRIES.load(Ordering::Relaxed),
        total_resolved: STAT_RESOLVED.load(Ordering::Relaxed),
        unique_symbols: STAT_UNIQUE.load(Ordering::Relaxed),
    }
}

#[doc(hidden)]
pub fn _reset_winffi_stats_for_tests() {
    STAT_ENTRIES.store(0, Ordering::Relaxed);
    STAT_RESOLVED.store(0, Ordering::Relaxed);
    STAT_UNIQUE.store(0, Ordering::Relaxed);
    if let Some(m) = UNIQUE_KEYS.get() {
        m.lock().expect("unique_keys poisoned").clear();
    }
}

/// Record that one stub-table entry was allocated by the sema layer.
/// Called from `nod-sema` as part of building a per-module stub table.
pub fn record_stub_entry_allocated() {
    STAT_ENTRIES.fetch_add(1, Ordering::Relaxed);
}

/// Resolve one `(dll, symbol)` pair and store the result into `entry`.
/// Bumps the WinFFI stats counters. Returns `Ok(())` on success or
/// `Err(c_ffi_error_word)` on failure.
///
/// Used by the sema-side glue when finalising a JIT module — it walks
/// the per-module stub-table specs one-by-one rather than batch-init
/// through [`initialize_stub_table`] so it can plumb the resolved
/// entry pointers back into the lowering pass directly.
///
/// # Safety
/// `entry` must be a valid pointer to a static-area [`ApiStubEntry`].
pub unsafe fn resolve_into_entry(entry: *const ApiStubEntry, dll: &str, symbol: &str) -> Result<(), Word> {
    // SAFETY: caller's invariant — pinned in the static area for the
    // process lifetime.
    let e = unsafe { &*entry };
    if !e.fn_ptr.load(Ordering::Acquire).is_null() {
        // Already resolved; this is a no-op (the dedupe path means a
        // single entry can be re-visited if multiple modules reuse
        // the same (dll, symbol)).
        return Ok(());
    }
    let p = resolve_symbol(dll, symbol);
    if p.is_null() {
        let last_err = last_os_error_code();
        return Err(make_c_ffi_error(
            dll,
            symbol,
            last_err,
            &format!(
                "winffi: LoadLibrary/GetProcAddress failed for `{symbol}@{dll}` (OS error {last_err})"
            ),
        ));
    }
    e.fn_ptr.store(p as *mut u8, Ordering::Release);
    STAT_RESOLVED.fetch_add(1, Ordering::Relaxed);
    let key = format!("{dll}::{symbol}");
    let mut keys = unique_keys().lock().expect("unique_keys poisoned");
    if keys.insert(key) {
        STAT_UNIQUE.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

// ─── <c-ffi-error> condition class ────────────────────────────────────────

struct CFfiErrorClass {
    id: ClassId,
}

static C_FFI_ERROR_CLASS: OnceLock<CFfiErrorClass> = OnceLock::new();

/// Register the `<c-ffi-error>` condition class. Idempotent. Called
/// from `nod-sema` lowering when a `define c-function` is encountered.
pub fn ensure_c_ffi_error_registered() {
    crate::conditions::ensure_registered();
    let _ = C_FFI_ERROR_CLASS.get_or_init(|| {
        let error_id = crate::conditions::error_class_id();
        let (id, _) = crate::register_simple_user_class(
            "<c-ffi-error>",
            Some(error_id),
            vec![
                slot_str("dll-name", "dll-name"),
                slot_str("symbol-name", "symbol-name"),
                slot_int("os-error-code", "os-error-code"),
                slot_str("message", "message"),
            ],
        );
        CFfiErrorClass { id }
    });
}

fn slot_str(name: &str, init_kw: &str) -> crate::classes::SlotInfo {
    crate::classes::SlotInfo {
        name: name.to_string(),
        offset: 0,
        type_kind: crate::classes::SlotType::String,
        init_keyword: Some(init_kw.to_string()),
        required_init_keyword: false,
        default_init: crate::classes::SlotDefault::Unbound,
        has_setter: false,
    }
}

fn slot_int(name: &str, init_kw: &str) -> crate::classes::SlotInfo {
    crate::classes::SlotInfo {
        name: name.to_string(),
        offset: 0,
        type_kind: crate::classes::SlotType::Integer,
        init_keyword: Some(init_kw.to_string()),
        required_init_keyword: false,
        default_init: crate::classes::SlotDefault::Unbound,
        has_setter: false,
    }
}

/// `<c-ffi-error>` ClassId accessor. Lazily ensures registration.
pub fn c_ffi_error_class_id() -> ClassId {
    ensure_c_ffi_error_registered();
    C_FFI_ERROR_CLASS
        .get()
        .expect("c-ffi-error registered")
        .id
}

/// Build a `<c-ffi-error>` heap instance.
pub fn make_c_ffi_error(dll: &str, symbol: &str, os_code: i64, message: &str) -> Word {
    ensure_c_ffi_error_registered();
    let md = crate::class_metadata_for(c_ffi_error_class_id());
    let dll_w = crate::intern_string_literal(dll);
    let sym_w = crate::intern_string_literal(symbol);
    let msg_w = crate::intern_string_literal(message);
    let code_w = Word::fixnum_unchecked(os_code);
    // SAFETY: registered metadata + matching keyword names.
    unsafe {
        rust_make(
            md,
            &[
                ("dll-name", dll_w),
                ("symbol-name", sym_w),
                ("os-error-code", code_w),
                ("message", msg_w),
            ],
        )
    }
}

// ─── Initialize a module's stub table ────────────────────────────────────

/// Walk `table`'s entries, `LoadLibrary` each unique DLL,
/// `GetProcAddress` each symbol, populate `fn_ptr`. Returns `Ok(())` on
/// success or `Err(c_ffi_error_word)` on the first failure.
///
/// Re-running this on a table whose entries are already populated is
/// a no-op (the `fn_ptr` check short-circuits each entry).
///
/// # Safety
/// `table.entries` must point at static-area [`ApiStubEntry`] records
/// whose `dll_name_ptr` / `symbol_name_ptr` point at valid UTF-8 byte
/// runs of the recorded lengths.
pub unsafe fn initialize_stub_table(table: &ApiStubTable) -> Result<(), Word> {
    for entry in table.entries.iter() {
        // If already resolved, skip.
        if !entry.fn_ptr.load(Ordering::Acquire).is_null() {
            continue;
        }
        // SAFETY: caller's invariant — pointers + lengths describe
        // valid UTF-8 byte runs in the static area.
        let dll = unsafe { str_from_raw(entry.dll_name_ptr, entry.dll_name_len as usize) };
        let symbol =
            unsafe { str_from_raw(entry.symbol_name_ptr, entry.symbol_name_len as usize) };
        let p = resolve_symbol(dll, symbol);
        if p.is_null() {
            let last_err = last_os_error_code();
            return Err(make_c_ffi_error(
                dll,
                symbol,
                last_err,
                &format!(
                    "winffi: LoadLibrary/GetProcAddress failed for `{symbol}@{dll}` (OS error {last_err})"
                ),
            ));
        }
        entry.fn_ptr.store(p as *mut u8, Ordering::Release);
        STAT_RESOLVED.fetch_add(1, Ordering::Relaxed);
        let key = format!("{dll}::{symbol}");
        let mut keys = unique_keys().lock().expect("unique_keys poisoned");
        if keys.insert(key) {
            STAT_UNIQUE.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn last_os_error_code() -> i64 {
    // SAFETY: GetLastError is a thread-local read with no preconditions.
    unsafe { windows_sys::Win32::Foundation::GetLastError() as i64 }
}

#[cfg(not(windows))]
fn last_os_error_code() -> i64 {
    0
}

/// # Safety
/// `ptr` + `len` must describe a valid UTF-8 byte run. The returned
/// `&str` shares the input's lifetime; callers must not hold it
/// across mutations of the underlying bytes (the static area never
/// mutates).
unsafe fn str_from_raw<'a>(ptr: *const u8, len: usize) -> &'a str {
    if ptr.is_null() || len == 0 {
        return "";
    }
    // SAFETY: caller's invariant.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    // The sema layer only writes UTF-8 (Dylan source identifiers are
    // ASCII / UTF-8 anyway). On the off-chance of an upstream bug we
    // fall back to an empty string rather than panic — the resolver
    // will then fail benignly.
    std::str::from_utf8(bytes).unwrap_or("")
}

// ─── Building a stub table from sema-side metadata ────────────────────────

/// Sema-side description of one c-function reference in the module
/// being lowered. The lowerer collects one of these per *unique*
/// `(dll, symbol)` pair across every call site of every c-function;
/// the per-call-site lowering then looks up the entry index and
/// emits a `WinFfiCall` (lowered as a DirectCall to a synthetic
/// `%winffi-call-N` callee carrying the entry pointer as a constant).
#[derive(Clone, Debug)]
pub struct StubEntrySpec {
    pub dll: String,
    pub symbol: String,
    pub signature: ApiCallSignature,
}

/// Allocate a fresh module-level stub table in the static area from
/// the supplied entry specs. Returns the table address (which lives
/// for the process lifetime) plus a parallel `Vec` of per-entry
/// pointers — those pointers are what the codegen layer bakes into
/// each call site's IR constant.
pub fn allocate_stub_table(specs: &[StubEntrySpec]) -> (&'static ApiStubTable, Vec<*const ApiStubEntry>) {
    crate::with_literal_pool(|pool| {
        let mut entries: Vec<ApiStubEntry> = Vec::with_capacity(specs.len());
        let mut pinned_dlls: Vec<&'static [u8]> = Vec::with_capacity(specs.len());
        let mut pinned_syms: Vec<&'static [u8]> = Vec::with_capacity(specs.len());
        for spec in specs {
            // Pin the dll / symbol names in the static area as raw byte
            // boxes. We can't reuse the literal-pool string interning
            // because those allocations carry a `<byte-string>` header
            // — the trampolines want raw bytes only.
            let dll_box: Box<[u8]> = spec.dll.as_bytes().to_vec().into_boxed_slice();
            let sym_box: Box<[u8]> = spec.symbol.as_bytes().to_vec().into_boxed_slice();
            // SAFETY: Box::leak gives a 'static slice; we'll never drop
            // these (intentional process-lifetime leak).
            let dll_static: &'static [u8] = Box::leak(dll_box);
            let sym_static: &'static [u8] = Box::leak(sym_box);
            pinned_dlls.push(dll_static);
            pinned_syms.push(sym_static);
            entries.push(ApiStubEntry {
                dll_name_ptr: dll_static.as_ptr(),
                dll_name_len: dll_static.len() as u32,
                symbol_name_ptr: sym_static.as_ptr(),
                symbol_name_len: sym_static.len() as u32,
                fn_ptr: AtomicPtr::new(std::ptr::null_mut()),
                signature: spec.signature,
            });
            STAT_ENTRIES.fetch_add(1, Ordering::Relaxed);
        }
        // Leak the entries vec into the static area, then build the
        // table struct itself.
        let entries_boxed: Box<[ApiStubEntry]> = entries.into_boxed_slice();
        // SAFETY: Box::leak gives a 'static slice; static-area lifetime.
        let entries_static: &'static [ApiStubEntry] = Box::leak(entries_boxed);
        let entry_ptrs: Vec<*const ApiStubEntry> =
            entries_static.iter().map(|e| e as *const _).collect();
        let table = pool.static_area.alloc(ApiStubTable { entries: entries_static });
        (table, entry_ptrs)
    })
}

// ─── Win64 marshaling helpers ─────────────────────────────────────────────

/// Unbox one Dylan-side arg Word according to its `kind` to a `u64`
/// suitable for the Win64 register/stack-slot ABI. Integers are
/// sign- or zero-extended to 64 bits as required.
///
/// # Panics
/// On kinds outside the Sprint 28 supported set, or on a Word that
/// doesn't carry the expected payload (e.g. a non-fixnum for an
/// integer kind). The Sema layer is responsible for type-checking
/// before emitting the WinFfiCall.
fn unbox_arg(w: Word, kind: u8) -> u64 {
    let k = CArgKind::from_u8(kind);
    match k {
        CArgKind::Void => 0,
        CArgKind::Int8
        | CArgKind::Int16
        | CArgKind::Int32
        | CArgKind::Int64
        | CArgKind::UInt8
        | CArgKind::UInt16
        | CArgKind::UInt32
        | CArgKind::UInt64 => {
            // Dylan fixnum payload — extract the i64 value. The Win64
            // ABI takes integer args in 64-bit registers regardless of
            // declared width; truncation happens at the callee.
            let v = w.as_fixnum().unwrap_or_else(|| {
                panic!(
                    "winffi: expected fixnum-shaped integer arg for kind {k:?}; got raw {:#x}",
                    w.raw()
                )
            });
            v as u64
        }
        CArgKind::Bool32 => {
            // `<c-bool>` accepts either the Dylan boolean singletons
            // (`#t` / `#f`) OR a fixnum (0 = false, anything else =
            // true). Both forms encode to a u32 the Win32 ABI treats
            // as 0 or 1.
            let imm = crate::literal_pool_immediates();
            if w == imm.true_ {
                1
            } else if w == imm.false_ {
                0
            } else if let Some(n) = w.as_fixnum() {
                if n != 0 { 1 } else { 0 }
            } else {
                // Any other pointer-shaped Word counts as true (Dylan
                // truthiness — every non-#f value is true).
                1
            }
        }
        CArgKind::Pointer | CArgKind::Handle => {
            // Pointer payloads: a Dylan fixnum is treated as a raw
            // numeric handle (so callers can pass e.g. `null` as 0).
            // A pointer-tagged Word carries an 8-byte-aligned address;
            // we strip the tag bit and pass the raw address.
            if let Some(n) = w.as_fixnum() {
                n as u64
            } else if let Some(p) = w.as_ptr::<u8>() {
                p as u64
            } else {
                0
            }
        }
    }
}

/// Rebox a raw u64 return value as a Dylan Word per the call's
/// recorded return kind.
fn box_return(raw: u64, kind: u8) -> Word {
    let k = CReturnKind::from_u8(kind);
    match k {
        CReturnKind::Void => crate::literal_pool_immediates().nil,
        CReturnKind::Int32 => {
            let v = raw as i32 as i64;
            Word::fixnum_unchecked(v)
        }
        CReturnKind::Int64 => Word::fixnum_unchecked(raw as i64),
        CReturnKind::UInt32 => Word::fixnum_unchecked((raw as u32) as i64),
        CReturnKind::UInt64 => {
            // u64 may overflow a 63-bit signed fixnum. Sprint 28
            // truncates to fixnum range; callers that need the full
            // u64 should declare the return as `<c-pointer>` instead.
            // The mask drops the sign bit so the result is always
            // representable as a non-negative fixnum.
            let masked = (raw & ((1u64 << 62) - 1)) as i64;
            Word::fixnum_unchecked(masked)
        }
        CReturnKind::Bool32 => {
            let imm = crate::literal_pool_immediates();
            if (raw as u32) != 0 { imm.true_ } else { imm.false_ }
        }
        CReturnKind::Pointer | CReturnKind::Handle => {
            // Pointer-shaped returns come back as a raw u64. We
            // surface them as a Dylan fixnum (carrying the raw
            // numeric handle value) — the Dylan side compares
            // against zero / known pseudo-handles using integer
            // comparison. Pointer-tagging the raw address only works
            // if it's 8-byte-aligned and not zero; Win32 pseudo-
            // handles like `(HANDLE)-1` aren't aligned, so we use the
            // numeric form which is robust for every value.
            //
            // 63-bit fixnum range can hold any 0x0..=0x7FFFFFFFFFFFFFFF
            // address; values with the sign bit set (kernel addresses,
            // pseudo-handles like -1) sign-extend correctly because
            // `as i64` reinterprets the bit pattern.
            Word::fixnum_unchecked(raw as i64)
        }
    }
}

/// Common prelude for all trampolines: load the resolved function
/// pointer, validate it's been populated, and capture the signature.
/// Returns the function pointer or panics with a deliberate error
/// (the lowering layer guarantees module init happens before any
/// call).
#[inline(always)]
unsafe fn trampoline_prelude(entry: *const ApiStubEntry) -> (*mut u8, ApiCallSignature) {
    // SAFETY: caller's invariant — entry is the leaked static-area
    // pointer baked into the IR constant.
    let entry_ref = unsafe { &*entry };
    let fn_ptr = entry_ref.fn_ptr.load(Ordering::Acquire);
    if fn_ptr.is_null() {
        // SAFETY: ditto — we just need the names for the panic
        // message.
        let dll = unsafe {
            str_from_raw(entry_ref.dll_name_ptr, entry_ref.dll_name_len as usize)
        };
        let sym = unsafe {
            str_from_raw(entry_ref.symbol_name_ptr, entry_ref.symbol_name_len as usize)
        };
        panic!(
            "winffi: c-function `{sym}@{dll}` called before initialize_stub_table populated its entry"
        );
    }
    (fn_ptr, entry_ref.signature)
}

// ─── Trampolines: arity 0..=8 ─────────────────────────────────────────────
//
// One trampoline per arity. The codegen emits a DirectCall against
// `nod_winffi_call_N` with the entry pointer as the first arg (baked
// as an `i64` WordBits constant) followed by the user args.
//
// Each trampoline is `extern "C-unwind"` so a panic from the
// prelude can propagate via the Sprint 19 unwinder. The inner
// invocation of the resolved function uses `extern "system"` — the
// Win64 ABI uses `cdecl`-like rules with RCX/RDX/R8/R9 + stack
// slots, which `extern "system"` selects on Windows.

/// 0-arg trampoline: `nod_winffi_call_0(entry) -> u64`.
///
/// # Safety
/// `entry` must be the raw `u64` address of a fully-populated
/// [`ApiStubEntry`] in the static area (i.e. one that has gone
/// through `initialize_stub_table` / `resolve_into_entry`). The
/// entry's recorded signature must match this trampoline's arity.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_winffi_call_0(entry: u64) -> u64 {
    // SAFETY: `entry` is the static-area pointer the codegen baked.
    let (fn_ptr, sig) = unsafe { trampoline_prelude(entry as *const ApiStubEntry) };
    debug_assert_eq!(sig.arg_count, 0);
    // SAFETY: by sema invariant, sig.return_kind matches the actual
    // function's return shape, and arity 0 means no args.
    let raw = unsafe {
        let f: extern "system" fn() -> u64 = std::mem::transmute(fn_ptr);
        f()
    };
    box_return(raw, sig.return_kind).raw()
}

// Concat-paste meta-variable expressions aren't stable on the workspace
// edition; we expand each arity explicitly below.

/// 1-arg trampoline.
///
/// # Safety
/// See [`nod_winffi_call_0`].
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_winffi_call_1(entry: u64, a0: u64) -> u64 {
    // SAFETY: `entry` is the baked static-area pointer.
    let (fn_ptr, sig) = unsafe { trampoline_prelude(entry as *const ApiStubEntry) };
    debug_assert_eq!(sig.arg_count, 1);
    let c0 = unbox_arg(Word::from_raw(a0), sig.arg_kinds[0]);
    // SAFETY: sema-validated; Win64 ABI for one 64-bit arg.
    let raw = unsafe {
        let f: extern "system" fn(u64) -> u64 = std::mem::transmute(fn_ptr);
        f(c0)
    };
    box_return(raw, sig.return_kind).raw()
}

/// 2-arg trampoline.
///
/// # Safety
/// See [`nod_winffi_call_0`].
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_winffi_call_2(entry: u64, a0: u64, a1: u64) -> u64 {
    // SAFETY: `entry` is the baked static-area pointer.
    let (fn_ptr, sig) = unsafe { trampoline_prelude(entry as *const ApiStubEntry) };
    debug_assert_eq!(sig.arg_count, 2);
    let c0 = unbox_arg(Word::from_raw(a0), sig.arg_kinds[0]);
    let c1 = unbox_arg(Word::from_raw(a1), sig.arg_kinds[1]);
    // SAFETY: sema-validated; two-arg Win64 ABI.
    let raw = unsafe {
        let f: extern "system" fn(u64, u64) -> u64 = std::mem::transmute(fn_ptr);
        f(c0, c1)
    };
    box_return(raw, sig.return_kind).raw()
}

/// 3-arg trampoline.
///
/// # Safety
/// See [`nod_winffi_call_0`].
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_winffi_call_3(
    entry: u64,
    a0: u64,
    a1: u64,
    a2: u64,
) -> u64 {
    // SAFETY: `entry` is the baked static-area pointer.
    let (fn_ptr, sig) = unsafe { trampoline_prelude(entry as *const ApiStubEntry) };
    debug_assert_eq!(sig.arg_count, 3);
    let c0 = unbox_arg(Word::from_raw(a0), sig.arg_kinds[0]);
    let c1 = unbox_arg(Word::from_raw(a1), sig.arg_kinds[1]);
    let c2 = unbox_arg(Word::from_raw(a2), sig.arg_kinds[2]);
    // SAFETY: sema-validated; three-arg Win64 ABI.
    let raw = unsafe {
        let f: extern "system" fn(u64, u64, u64) -> u64 = std::mem::transmute(fn_ptr);
        f(c0, c1, c2)
    };
    box_return(raw, sig.return_kind).raw()
}

/// 4-arg trampoline.
///
/// # Safety
/// See [`nod_winffi_call_0`].
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_winffi_call_4(
    entry: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
) -> u64 {
    // SAFETY: `entry` is the baked static-area pointer.
    let (fn_ptr, sig) = unsafe { trampoline_prelude(entry as *const ApiStubEntry) };
    debug_assert_eq!(sig.arg_count, 4);
    let c0 = unbox_arg(Word::from_raw(a0), sig.arg_kinds[0]);
    let c1 = unbox_arg(Word::from_raw(a1), sig.arg_kinds[1]);
    let c2 = unbox_arg(Word::from_raw(a2), sig.arg_kinds[2]);
    let c3 = unbox_arg(Word::from_raw(a3), sig.arg_kinds[3]);
    // SAFETY: sema-validated; four-arg Win64 ABI.
    let raw = unsafe {
        let f: extern "system" fn(u64, u64, u64, u64) -> u64 = std::mem::transmute(fn_ptr);
        f(c0, c1, c2, c3)
    };
    box_return(raw, sig.return_kind).raw()
}

/// 5-arg trampoline.
///
/// # Safety
/// See [`nod_winffi_call_0`].
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_winffi_call_5(
    entry: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
) -> u64 {
    // SAFETY: `entry` is the baked static-area pointer.
    let (fn_ptr, sig) = unsafe { trampoline_prelude(entry as *const ApiStubEntry) };
    debug_assert_eq!(sig.arg_count, 5);
    let c0 = unbox_arg(Word::from_raw(a0), sig.arg_kinds[0]);
    let c1 = unbox_arg(Word::from_raw(a1), sig.arg_kinds[1]);
    let c2 = unbox_arg(Word::from_raw(a2), sig.arg_kinds[2]);
    let c3 = unbox_arg(Word::from_raw(a3), sig.arg_kinds[3]);
    let c4 = unbox_arg(Word::from_raw(a4), sig.arg_kinds[4]);
    // SAFETY: sema-validated; five-arg Win64 ABI (RCX/RDX/R8/R9 +
    // one stack slot above shadow space).
    let raw = unsafe {
        let f: extern "system" fn(u64, u64, u64, u64, u64) -> u64 =
            std::mem::transmute(fn_ptr);
        f(c0, c1, c2, c3, c4)
    };
    box_return(raw, sig.return_kind).raw()
}

/// 6-arg trampoline.
///
/// # Safety
/// See [`nod_winffi_call_0`].
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_winffi_call_6(
    entry: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
) -> u64 {
    // SAFETY: `entry` is the baked static-area pointer.
    let (fn_ptr, sig) = unsafe { trampoline_prelude(entry as *const ApiStubEntry) };
    debug_assert_eq!(sig.arg_count, 6);
    let c0 = unbox_arg(Word::from_raw(a0), sig.arg_kinds[0]);
    let c1 = unbox_arg(Word::from_raw(a1), sig.arg_kinds[1]);
    let c2 = unbox_arg(Word::from_raw(a2), sig.arg_kinds[2]);
    let c3 = unbox_arg(Word::from_raw(a3), sig.arg_kinds[3]);
    let c4 = unbox_arg(Word::from_raw(a4), sig.arg_kinds[4]);
    let c5 = unbox_arg(Word::from_raw(a5), sig.arg_kinds[5]);
    // SAFETY: sema-validated; six-arg Win64 ABI.
    let raw = unsafe {
        let f: extern "system" fn(u64, u64, u64, u64, u64, u64) -> u64 =
            std::mem::transmute(fn_ptr);
        f(c0, c1, c2, c3, c4, c5)
    };
    box_return(raw, sig.return_kind).raw()
}

/// 7-arg trampoline.
///
/// # Safety
/// See [`nod_winffi_call_0`].
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_winffi_call_7(
    entry: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
) -> u64 {
    // SAFETY: `entry` is the baked static-area pointer.
    let (fn_ptr, sig) = unsafe { trampoline_prelude(entry as *const ApiStubEntry) };
    debug_assert_eq!(sig.arg_count, 7);
    let c0 = unbox_arg(Word::from_raw(a0), sig.arg_kinds[0]);
    let c1 = unbox_arg(Word::from_raw(a1), sig.arg_kinds[1]);
    let c2 = unbox_arg(Word::from_raw(a2), sig.arg_kinds[2]);
    let c3 = unbox_arg(Word::from_raw(a3), sig.arg_kinds[3]);
    let c4 = unbox_arg(Word::from_raw(a4), sig.arg_kinds[4]);
    let c5 = unbox_arg(Word::from_raw(a5), sig.arg_kinds[5]);
    let c6 = unbox_arg(Word::from_raw(a6), sig.arg_kinds[6]);
    // SAFETY: sema-validated; seven-arg Win64 ABI.
    let raw = unsafe {
        let f: extern "system" fn(u64, u64, u64, u64, u64, u64, u64) -> u64 =
            std::mem::transmute(fn_ptr);
        f(c0, c1, c2, c3, c4, c5, c6)
    };
    box_return(raw, sig.return_kind).raw()
}

/// 8-arg trampoline — the Sprint 28 max.
///
/// # Safety
/// See [`nod_winffi_call_0`].
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_winffi_call_8(
    entry: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
    a7: u64,
) -> u64 {
    // SAFETY: `entry` is the baked static-area pointer.
    let (fn_ptr, sig) = unsafe { trampoline_prelude(entry as *const ApiStubEntry) };
    debug_assert_eq!(sig.arg_count, 8);
    let c0 = unbox_arg(Word::from_raw(a0), sig.arg_kinds[0]);
    let c1 = unbox_arg(Word::from_raw(a1), sig.arg_kinds[1]);
    let c2 = unbox_arg(Word::from_raw(a2), sig.arg_kinds[2]);
    let c3 = unbox_arg(Word::from_raw(a3), sig.arg_kinds[3]);
    let c4 = unbox_arg(Word::from_raw(a4), sig.arg_kinds[4]);
    let c5 = unbox_arg(Word::from_raw(a5), sig.arg_kinds[5]);
    let c6 = unbox_arg(Word::from_raw(a6), sig.arg_kinds[6]);
    let c7 = unbox_arg(Word::from_raw(a7), sig.arg_kinds[7]);
    // SAFETY: sema-validated; eight-arg Win64 ABI.
    let raw = unsafe {
        let f: extern "system" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64 =
            std::mem::transmute(fn_ptr);
        f(c0, c1, c2, c3, c4, c5, c6, c7)
    };
    box_return(raw, sig.return_kind).raw()
}

// ─── Sema helper: signature from c-type names ─────────────────────────────

/// Build an [`ApiCallSignature`] from a list of param c-type names
/// (e.g. `["<c-dword>", "<c-dword>"]`) and a return c-type name
/// (e.g. `"<c-bool>"`). Returns `Err(name)` if any name isn't in the
/// Sprint 28 supported set.
pub fn signature_from_names(
    arg_names: &[&str],
    return_name: Option<&str>,
) -> Result<ApiCallSignature, String> {
    if arg_names.len() > 8 {
        return Err(format!(
            "winffi: arity {} exceeds Sprint 28 cap of 8",
            arg_names.len()
        ));
    }
    let mut arg_kinds = [CArgKind::Void as u8; 8];
    for (i, n) in arg_names.iter().enumerate() {
        let k = CArgKind::from_c_type_name(n).ok_or_else(|| n.to_string())?;
        arg_kinds[i] = k as u8;
    }
    let return_kind = match return_name {
        None => CReturnKind::Void as u8,
        Some(n) => CReturnKind::from_c_type_name(n).ok_or_else(|| n.to_string())? as u8,
    };
    Ok(ApiCallSignature {
        arg_count: arg_names.len() as u8,
        arg_kinds,
        return_kind,
    })
}

// Silence unused-import warnings for non-Windows builds where the
// stat counter `AtomicU64` import isn't needed in practice.
const _: fn() = || {
    let _ = std::marker::PhantomData::<AtomicU64>;
};

// Suppress unused-helper warnings; `condition_class_name` is part of
// the `<c-ffi-error>` diagnostics chain referenced by tests through
// the public `condition_class_name` API in `conditions.rs`.
const _: fn(Word) -> Option<String> = condition_class_name;
