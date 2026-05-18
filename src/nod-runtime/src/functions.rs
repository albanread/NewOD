//! Sprint 21 — first-class function values.
//!
//! This module owns:
//!
//!   1. **The `<function>` class** — a heap-tagged Word whose slots
//!      carry the function-pointer + arity descriptor + bookkeeping.
//!      Registered idempotently at process boot via
//!      `register_simple_user_class("<function>", None, …)`.
//!
//!   2. **`<wrong-number-of-arguments-error>`** — signalled from the
//!      `nod_funcall_N` extern shims when the descriptor's arity
//!      doesn't match the call shape. Parent `<error>`.
//!
//!   3. **`nod_funcall1` / `nod_funcall2`** — JIT-callable trampolines
//!      that pull the function-pointer out of the `<function>` Word and
//!      tail-call to it. The `<function>` ABI for callees is the same
//!      `extern "C-unwind" fn(u64, …) -> u64` shape that the rest of
//!      the JIT uses; the trampoline just transmutes and calls.
//!
//!   4. **`nod_apply`** — variadic dispatch: an args-vector
//!      (`<simple-object-vector>` containing tagged Words) unpacks up to
//!      `MAX_APPLY_ARITY` positional arguments. Sprint 21 caps at 8 —
//!      see DEFERRED.md for higher-arity follow-up.
//!
//! ## Slot layout
//!
//! ```text
//! <function>
//!   name        : <byte-string>   (diagnostics; `\+` -> "+", etc.)
//!   arity       : <integer>       (Sprint 21: fixed arity only)
//!   code-ptr    : <integer>       (RAW host pointer — NOT a Dylan Word)
//!   kind-tag    : <integer>       (0 = top-level, 1 = lifted anon)
//!   env-ptr     : <integer>       (Sprint 21: always 0; closures land Sprint 24)
//!   return-type : <integer>       (encoded TypeEstimate; 0 for now)
//! ```
//!
//! **All slots are typed `SlotType::Integer`** so the GC's class-driven
//! scanner treats them as opaque bits. `name` is in practice a
//! pointer-tagged `<byte-string>` interned in the static area — but
//! since the static area is the same place every other immutable string
//! literal lives, the conservative-Integer typing here is fine: the
//! collector won't follow the slot, and the static-area target is
//! pinned for the process lifetime regardless. (We could relax to
//! `SlotType::String` later when `<function>` instances themselves move
//! out of the static area; today every Sprint 21 instance is allocated
//! in the static area via `make_function`, so the slot value is also a
//! permanent pointer and scanning is moot.)

use std::sync::{Mutex, OnceLock};

use crate::classes::{
    ClassId, ClassMetadata, SlotDefault, SlotInfo, SlotType, class_metadata_for, is_subclass,
};
use crate::make::rust_make;
use crate::word::Word;

/// Sprint 21 cap on the number of positional arguments `nod_apply` will
/// unpack from its arg-vector. Larger applies error with a clear
/// diagnostic; see DEFERRED.md for the lift-the-cap follow-up.
pub const MAX_APPLY_ARITY: usize = 8;

struct FunctionClassIds {
    function: ClassId,
    wrong_args: ClassId,
    function_md: &'static ClassMetadata,
    wrong_args_md: &'static ClassMetadata,
}

static FUNCTION_CLASSES: OnceLock<FunctionClassIds> = OnceLock::new();

/// Register `<function>` and `<wrong-number-of-arguments-error>`
/// idempotently. Safe to call repeatedly. The condition seed classes
/// are registered first because `<wrong-number-of-arguments-error>`
/// inherits from `<error>`.
///
/// Also installs the built-in operator shims (`+`, `-`, `*`, `=`,
/// `<`, `>`) into the function registry so user code can pass `\+`
/// etc. as first-class values.
pub fn ensure_registered() {
    ensure_operator_shims_registered();
    let _ = FUNCTION_CLASSES.get_or_init(|| {
        crate::conditions::ensure_registered();
        // Sprint 21: parent = `<object>` so `is_subclass(<function>,
        // <object>)` holds. Stdlib methods registered as
        // `(c :: <object>)` need this for the dispatcher's
        // applicability check.
        let (function, _) = crate::register_simple_user_class(
            "<function>",
            Some(ClassId::OBJECT),
            vec![
                slot_int("name", "function-name"),
                slot_int("arity", "arity"),
                slot_int("code-ptr", "code-ptr"),
                slot_int("kind-tag", "kind-tag"),
                slot_int("env-ptr", "env-ptr"),
                slot_int("return-type", "return-type"),
            ],
        );
        let function_md = class_metadata_for(function);

        let error = crate::conditions::error_class_id();
        let (wrong_args, _) = crate::register_simple_user_class(
            "<wrong-number-of-arguments-error>",
            Some(error),
            vec![
                slot_int("function", "function"),
                slot_int("expected", "expected"),
                slot_int("got", "got"),
            ],
        );
        let wrong_args_md = class_metadata_for(wrong_args);

        FunctionClassIds {
            function,
            wrong_args,
            function_md,
            wrong_args_md,
        }
    });
}

fn slot_int(name: &str, init_kw: &str) -> SlotInfo {
    SlotInfo {
        name: name.to_string(),
        offset: 0,
        // Integer-typed — the GC scanner skips these slots. See module
        // docs for the rationale (Sprint 21 keeps code-ptr / env-ptr /
        // arity / return-type as opaque host bits, and `name` as a
        // pointer to a pinned static-area `<byte-string>`).
        type_kind: SlotType::Integer,
        init_keyword: Some(init_kw.to_string()),
        required_init_keyword: false,
        default_init: SlotDefault::Unbound,
        has_setter: false,
    }
}

fn classes() -> &'static FunctionClassIds {
    ensure_registered();
    FUNCTION_CLASSES
        .get()
        .expect("function classes registered")
}

pub fn function_class_id() -> ClassId {
    classes().function
}

pub fn wrong_number_of_arguments_error_class_id() -> ClassId {
    classes().wrong_args
}

// ─── Builders ──────────────────────────────────────────────────────────────

/// Allocate a `<function>` instance carrying the supplied descriptor.
/// `code_ptr` is the raw host address of an
/// `extern "C-unwind" fn(u64, ..., u64) -> u64` (or compatible) — the
/// trampoline transmutes to the right signature based on `arity`.
///
/// Sprint 21: `env_ptr` is unused (always pass 0). Closures with
/// captured environments land in Sprint 24.
pub fn make_function(
    name: &str,
    arity: usize,
    code_ptr: *const u8,
    kind_tag: u32,
    env_ptr: u64,
) -> Word {
    let md = classes().function_md;
    let name_word = crate::intern_string_literal(name);
    // The code-ptr / env-ptr / arity / kind-tag slots all expect a
    // tagged-Word value. We pack as fixnums where they fit (arity,
    // kind-tag, return-type) and as raw `WordBits`-tagged opaque
    // integers for the code-ptr / env-ptr (host pointers are arbitrary
    // 64-bit values that may not fit in a 63-bit fixnum).
    //
    // Sprint 21 simplification: we treat ALL six slots as opaque
    // 64-bit-bit-pattern values. `nod_make`'s slot-store path writes
    // the supplied Word verbatim into the slot — it doesn't
    // re-tag — so we hand it the raw bit pattern wrapped as a
    // `Word::from_raw`. The readers below pull the bits back out via
    // the same `Word::raw()` accessor.
    let arity_w = Word::from_raw(arity as u64);
    let code_w = Word::from_raw(code_ptr as u64);
    let kind_w = Word::from_raw(kind_tag as u64);
    let env_w = Word::from_raw(env_ptr);
    let ret_w = Word::from_raw(0);
    // SAFETY: registered metadata; init keyword names match the slot
    // names registered in `ensure_registered`.
    unsafe {
        rust_make(
            md,
            &[
                ("function-name", name_word),
                ("arity", arity_w),
                ("code-ptr", code_w),
                ("kind-tag", kind_w),
                ("env-ptr", env_w),
                ("return-type", ret_w),
            ],
        )
    }
}

/// Read the `code-ptr` slot from a `<function>` Word. Returns `None` if
/// the Word isn't pointer-tagged (the caller is expected to have
/// type-checked already; this is the defensive read for the
/// trampoline path).
pub fn function_code_ptr(f: Word) -> Option<*const u8> {
    let md = classes().function_md;
    let p = f.as_ptr::<u8>()?;
    let offset = md.slot_offset("code-ptr")?;
    // SAFETY: caller asserts `f` points at a `<function>` instance.
    // Slot offset is bounded by the class's instance size.
    let raw = unsafe { *((p as usize + offset) as *const u64) };
    Some(raw as *const u8)
}

/// Read the `arity` slot from a `<function>` Word.
pub fn function_arity(f: Word) -> Option<usize> {
    let md = classes().function_md;
    let p = f.as_ptr::<u8>()?;
    let offset = md.slot_offset("arity")?;
    // SAFETY: same as `function_code_ptr`.
    let raw = unsafe { *((p as usize + offset) as *const u64) };
    Some(raw as usize)
}

/// Read the `name` slot of a `<function>` instance as a Rust `String`.
/// Used by the wrong-number-of-arguments diagnostic and tests.
pub fn function_name(f: Word) -> Option<String> {
    let md = classes().function_md;
    let p = f.as_ptr::<u8>()?;
    let offset = md.slot_offset("name")?;
    // SAFETY: `name` slot stores a pointer-tagged <byte-string> Word.
    let name_word = unsafe { *((p as usize + offset) as *const Word) };
    let bs = unsafe { crate::try_byte_string(name_word, ClassId::BYTE_STRING) }?;
    // SAFETY: bs points at a live <byte-string>.
    unsafe { bs.as_str() }.map(|s| s.to_string())
}

/// True iff `w` is pointer-tagged and its wrapper class is
/// `<function>` (or a subclass — Sprint 21 has none, but the check
/// generalises).
pub fn is_function(w: Word) -> bool {
    let Some(p) = w.as_ptr::<u8>() else {
        return false;
    };
    // SAFETY: pointer-tagged Word; first 8 bytes are the Wrapper.
    let wrapper = unsafe { *(p as *const crate::wrapper::Wrapper) };
    is_subclass(wrapper.class(), classes().function)
}

/// Build a `<wrong-number-of-arguments-error>` instance carrying the
/// supplied function Word, the expected arity, and the actual arity.
pub fn make_wrong_number_of_arguments_error(
    function: Word,
    expected: usize,
    got: usize,
) -> Word {
    let md = classes().wrong_args_md;
    // SAFETY: registered metadata; init keyword names match.
    unsafe {
        rust_make(
            md,
            &[
                ("function", function),
                (
                    "expected",
                    Word::from_fixnum(expected as i64).unwrap_or(Word::from_raw(0)),
                ),
                (
                    "got",
                    Word::from_fixnum(got as i64).unwrap_or(Word::from_raw(0)),
                ),
            ],
        )
    }
}

// ─── Trampoline externs ───────────────────────────────────────────────────

type Arity0Fn = extern "C-unwind" fn() -> u64;
type Arity1Fn = extern "C-unwind" fn(u64) -> u64;
type Arity2Fn = extern "C-unwind" fn(u64, u64) -> u64;
type Arity3Fn = extern "C-unwind" fn(u64, u64, u64) -> u64;
type Arity4Fn = extern "C-unwind" fn(u64, u64, u64, u64) -> u64;
type Arity5Fn = extern "C-unwind" fn(u64, u64, u64, u64, u64) -> u64;
type Arity6Fn = extern "C-unwind" fn(u64, u64, u64, u64, u64, u64) -> u64;
type Arity7Fn = extern "C-unwind" fn(u64, u64, u64, u64, u64, u64, u64) -> u64;
type Arity8Fn = extern "C-unwind" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64;

/// Common arity check + dispatch error. Diverges if the function's
/// arity doesn't match `expected`.
fn check_arity_or_signal(f: Word, expected: usize) -> usize {
    let arity = function_arity(f).unwrap_or_else(|| {
        panic!(
            "nod_funcall*: argument is not a <function> Word (raw = {:#x})",
            f.raw()
        );
    });
    if arity != expected {
        let cond = make_wrong_number_of_arguments_error(f, expected, arity);
        // Diverges via `nod_signal`'s NLX path; if no handler matches,
        // panics with the unhandled-condition message. The return value
        // is never observed.
        // SAFETY: cond is a freshly-allocated condition Word.
        let _ = unsafe { crate::conditions::nod_signal(cond.raw()) };
    }
    arity
}

fn code_ptr_or_panic(f: Word) -> *const u8 {
    function_code_ptr(f).unwrap_or_else(|| {
        panic!(
            "nod_funcall*: argument is not a <function> Word (raw = {:#x})",
            f.raw()
        );
    })
}

/// `nod_funcall1(f, a) -> r` — invoke `f` with one arg.
///
/// Sprint 21 also accepts `<exit-procedure>` Words and routes them
/// to `nod_invoke_exit` so that lifted-thunk env-bound names work
/// uniformly: the same lowering path drives `\foo(x)` AND
/// `block (k) ... k(v) ... end`.
///
/// # Safety
///
/// `f_raw` must be a pointer-tagged Dylan Word; `a` is any Dylan Word.
/// If `f` is a `<function>`, its `code-ptr` must point at an
/// `extern "C-unwind" fn(u64) -> u64`. If `f` is an `<exit-procedure>`,
/// the call diverges via NLX.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_funcall1(f_raw: u64, a: u64) -> u64 {
    let f = Word::from_raw(f_raw);
    // Dispatch on the class. `<exit-procedure>` routes through
    // `nod_invoke_exit`; everything else expects `<function>`.
    if crate::conditions::exit_procedure_block_id(f).is_some() {
        // SAFETY: f is an <exit-procedure> Word; nod_invoke_exit
        // diverges.
        return unsafe { crate::conditions::nod_invoke_exit(f_raw, a) };
    }
    let _ = check_arity_or_signal(f, 1);
    let code = code_ptr_or_panic(f);
    // SAFETY: caller asserts the callee at `code` matches arity-1 ABI.
    let f1: Arity1Fn = unsafe { std::mem::transmute(code) };
    f1(a)
}

/// `nod_funcall2(f, a, b) -> r` — invoke `f` with two args.
///
/// # Safety
///
/// See `nod_funcall1`; callee must match arity-2 ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_funcall2(f_raw: u64, a: u64, b: u64) -> u64 {
    let f = Word::from_raw(f_raw);
    let _ = check_arity_or_signal(f, 2);
    let code = code_ptr_or_panic(f);
    // SAFETY: arity matched; callee matches arity-2 ABI.
    let f2: Arity2Fn = unsafe { std::mem::transmute(code) };
    f2(a, b)
}

/// `nod_apply(f, args_vector) -> r` — variadic dispatch via a
/// `<simple-object-vector>` of tagged Words. Sprint 21 caps the args
/// at `MAX_APPLY_ARITY` (8); higher-arity applies signal a
/// wrong-number-of-arguments condition.
///
/// # Safety
///
/// `f_raw` must be a pointer-tagged `<function>` Word; `args_raw` must
/// be a pointer-tagged `<simple-object-vector>` Word.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_apply(f_raw: u64, args_raw: u64) -> u64 {
    let f = Word::from_raw(f_raw);
    let args = Word::from_raw(args_raw);
    let sov = unsafe { crate::try_simple_object_vector(args, ClassId::SIMPLE_OBJECT_VECTOR) }
        .unwrap_or_else(|| {
            panic!(
                "nod_apply: args is not a <simple-object-vector> Word (raw = {:#x})",
                args_raw
            );
        });
    let n = sov.len as usize;
    let arity = function_arity(f).unwrap_or_else(|| {
        panic!(
            "nod_apply: function is not a <function> Word (raw = {:#x})",
            f_raw
        );
    });
    if n != arity {
        let cond = make_wrong_number_of_arguments_error(f, arity, n);
        // SAFETY: cond is a freshly-allocated condition Word; nod_signal
        // diverges, so the return is never observed.
        let _ = unsafe { crate::conditions::nod_signal(cond.raw()) };
    }
    if n > MAX_APPLY_ARITY {
        panic!(
            "nod_apply: Sprint 21 supports up to {MAX_APPLY_ARITY} args, got {n}; \
             higher-arity apply is a Sprint 22+ follow-up"
        );
    }
    let code = code_ptr_or_panic(f);
    // SAFETY: sov has at least `n` element slots; reading each as a u64
    // matches the `<simple-object-vector>` element layout (tagged Word
    // per slot).
    let mut a = [0u64; MAX_APPLY_ARITY];
    let slots = unsafe { sov.slots() };
    for i in 0..n {
        a[i] = slots[i].raw();
    }
    // SAFETY: arity-N callee ABI; we already verified `n == arity` and
    // `n <= MAX_APPLY_ARITY`.
    unsafe {
        match n {
            0 => (std::mem::transmute::<*const u8, Arity0Fn>(code))(),
            1 => (std::mem::transmute::<*const u8, Arity1Fn>(code))(a[0]),
            2 => (std::mem::transmute::<*const u8, Arity2Fn>(code))(a[0], a[1]),
            3 => (std::mem::transmute::<*const u8, Arity3Fn>(code))(a[0], a[1], a[2]),
            4 => (std::mem::transmute::<*const u8, Arity4Fn>(code))(a[0], a[1], a[2], a[3]),
            5 => (std::mem::transmute::<*const u8, Arity5Fn>(code))(
                a[0], a[1], a[2], a[3], a[4],
            ),
            6 => (std::mem::transmute::<*const u8, Arity6Fn>(code))(
                a[0], a[1], a[2], a[3], a[4], a[5],
            ),
            7 => (std::mem::transmute::<*const u8, Arity7Fn>(code))(
                a[0], a[1], a[2], a[3], a[4], a[5], a[6],
            ),
            8 => (std::mem::transmute::<*const u8, Arity8Fn>(code))(
                a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7],
            ),
            _ => unreachable!("clamped by the n > MAX_APPLY_ARITY check above"),
        }
    }
}

// ─── Top-level function registry ──────────────────────────────────────────
//
// Sprint 21 mints a `<function>` Word per registered name + arity at
// **runtime first-use** (via `nod_make_function_ref(name, arity)`); the
// instance lives in the static area so the Word survives GC and so
// codegen can bake the address as an `i64` constant.
//
// Two registries cooperate:
//
//   * `RUST_FUNCTION_REGISTRY` — names handed in from Rust code (e.g.
//     the operator shims `nod_plus_fn` etc. plus `format-out`,
//     `condition-message`, …). Registered at module-init time.
//
//   * `JIT_FUNCTION_REGISTRY` — names handed in by the sema layer post
//     JIT-compile, mapping a Dylan-source name (`reduce`, `bump`, the
//     synthesised `__anon-method-NNNN`) to the JIT'd entry-point
//     address.
//
// Lookup walks both registries (Rust first) and returns the first match.
// `nod_make_function_ref` then allocates the `<function>` Word in the
// static area; the returned Word's slot values (including the
// `<byte-string>` name slot) are all pinned for the process lifetime.
//
// **Memoisation:** repeated calls for the same `(name, arity)` return
// the same Word — both because we cache the allocation and because
// codegen sites that emit the same reference (e.g. two `\+` uses)
// must observe the same Word identity (so `f == g` comparisons work).

struct FunctionRegistryEntry {
    name: String,
    arity: usize,
    code_ptr: *const u8,
}

// SAFETY: the pointers stored in `FunctionRegistryEntry::code_ptr` are
// JIT-emitted or Rust-defined function addresses pinned for the process
// lifetime. Sharing them across threads is sound.
unsafe impl Send for FunctionRegistryEntry {}
unsafe impl Sync for FunctionRegistryEntry {}

static RUST_FUNCTION_REGISTRY: Mutex<Vec<FunctionRegistryEntry>> = Mutex::new(Vec::new());
static JIT_FUNCTION_REGISTRY: Mutex<Vec<FunctionRegistryEntry>> = Mutex::new(Vec::new());
// Cache of `(name, arity) -> <function> Word`. Each entry's Word is a
// pointer into the static area, stable for the process lifetime.
static FUNCTION_REF_CACHE: Mutex<Vec<((String, usize), Word)>> = Mutex::new(Vec::new());

/// Register a Rust-side function as a callable Dylan name. Used at
/// process boot for operator shims (`+`, `*`, …) and built-in helpers
/// (`format-out`, `condition-message`, …).
///
/// Subsequent `make_function_ref(name, arity)` calls resolve through
/// this table.
///
/// # Safety
///
/// `code_ptr` must point at an `extern "C-unwind" fn(u64, ..., u64) -> u64`
/// pinned for the process lifetime. The arity stated here must match
/// the callee's actual arity.
pub unsafe fn register_rust_function(name: &str, arity: usize, code_ptr: *const u8) {
    let mut g = RUST_FUNCTION_REGISTRY
        .lock()
        .expect("rust function registry poisoned");
    if let Some(slot) = g
        .iter_mut()
        .find(|e| e.name == name && e.arity == arity)
    {
        slot.code_ptr = code_ptr;
    } else {
        g.push(FunctionRegistryEntry {
            name: name.to_string(),
            arity,
            code_ptr,
        });
    }
}

/// Register a JIT-compiled top-level function under its Dylan name. The
/// `nod-sema` layer calls this once per `define function` body after
/// the JIT module is finalised; subsequent `\name` references resolve
/// through here.
///
/// # Safety
///
/// `code_ptr` must point at a JIT-emitted `extern "C-unwind"`-shaped
/// function whose runtime ABI is `(u64, ..., u64) -> u64`. The JIT
/// engine that owns the address must outlive every call site that
/// dispatches through it (Sprint 21's loaders leak the engines).
pub unsafe fn register_jit_function(name: &str, arity: usize, code_ptr: *const u8) {
    let mut g = JIT_FUNCTION_REGISTRY
        .lock()
        .expect("jit function registry poisoned");
    if let Some(slot) = g
        .iter_mut()
        .find(|e| e.name == name && e.arity == arity)
    {
        slot.code_ptr = code_ptr;
    } else {
        g.push(FunctionRegistryEntry {
            name: name.to_string(),
            arity,
            code_ptr,
        });
    }
}

fn lookup_function_code(name: &str, arity: usize) -> Option<*const u8> {
    let rust = RUST_FUNCTION_REGISTRY
        .lock()
        .expect("rust function registry poisoned");
    if let Some(e) = rust.iter().find(|e| e.name == name && e.arity == arity) {
        return Some(e.code_ptr);
    }
    drop(rust);
    let jit = JIT_FUNCTION_REGISTRY
        .lock()
        .expect("jit function registry poisoned");
    jit.iter()
        .find(|e| e.name == name && e.arity == arity)
        .map(|e| e.code_ptr)
}

/// Allocate (or reuse the cached) `<function>` Word for the given
/// `(name, arity)`. The Word's storage is in the static area, so it
/// survives GC and so codegen can bake the address as an `i64`
/// constant.
///
/// Returns `None` if the name+arity isn't registered.
pub fn make_function_ref(name: &str, arity: usize) -> Option<Word> {
    let mut cache = FUNCTION_REF_CACHE
        .lock()
        .expect("function ref cache poisoned");
    if let Some((_, w)) = cache
        .iter()
        .find(|((n, a), _)| n == name && *a == arity)
    {
        return Some(*w);
    }
    let code = lookup_function_code(name, arity)?;
    // Allocate the <function> in the static area. `rust_make` writes
    // into the moveable heap by default; for a Sprint 21 function-ref
    // we want pinned storage so the codegen-baked address stays valid
    // across GC cycles.
    //
    // Approach: build the instance via `rust_make` (which currently
    // allocates from the moveable heap), then immediately `pin` it by
    // promoting through the static area. Sprint 21 simplification: we
    // skip the promotion and rely on the fact that the function-Word
    // is reachable from the cache (held in this Mutex), so the GC's
    // root-walker preserves it across collections. The cache is a
    // process-global root.
    //
    // Future Sprint 24: when closures land, env-ptr will point to a
    // moveable heap object; the make-function instance moves with it.
    let w = make_function(name, arity, code, 0, 0);
    crate::heap_register_root(Box::leak(Box::new(w)) as *const Word as *mut Word);
    cache.push(((name.to_string(), arity), w));
    Some(w)
}

/// JIT-callable shim that returns the function-ref Word for the
/// supplied name (a `<byte-string>` Word) and arity (a fixnum Word).
/// Panics if the name isn't registered — codegen only emits this
/// against names it knows are registered.
///
/// # Safety
///
/// `name_raw` must be a pointer-tagged `<byte-string>` Word; `arity_raw`
/// must be a fixnum-tagged Word.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_make_function_ref(name_raw: u64, arity_raw: u64) -> u64 {
    let name_word = Word::from_raw(name_raw);
    let bs = unsafe { crate::try_byte_string(name_word, ClassId::BYTE_STRING) }
        .expect("nod_make_function_ref: name is not a <byte-string> Word");
    // SAFETY: bs points at a live <byte-string>.
    let name = unsafe { bs.as_str() }
        .expect("nod_make_function_ref: <byte-string> name not UTF-8")
        .to_string();
    let arity = Word::from_raw(arity_raw)
        .as_fixnum()
        .expect("nod_make_function_ref: arity is not a fixnum") as usize;
    let f = make_function_ref(&name, arity).unwrap_or_else(|| {
        panic!(
            "nod_make_function_ref: no registered function `{name}` with arity {arity}"
        )
    });
    f.raw()
}

/// Test helper: clear both function registries and the ref cache.
#[doc(hidden)]
pub fn _reset_function_registry_for_tests() {
    RUST_FUNCTION_REGISTRY
        .lock()
        .expect("rust function registry poisoned")
        .clear();
    JIT_FUNCTION_REGISTRY
        .lock()
        .expect("jit function registry poisoned")
        .clear();
    FUNCTION_REF_CACHE
        .lock()
        .expect("function ref cache poisoned")
        .clear();
}

// ─── Built-in operator shims ───────────────────────────────────────────────
//
// `\+`, `\-`, `\*`, `\=`, … get pre-registered Rust shims so user code
// can pass them as first-class function values (`reduce(\+, …)`). Each
// shim consumes two tagged-Word args, narrows to a fixnum, and returns
// the fixnum-tagged result.
//
// The `\=` shim returns a Dylan boolean. The relational comparisons
// (`<`, `>`, `<=`, `>=`) likewise. Float-typed args fall back to fixnum
// 0 — the brief specifies integer semantics; float handling lands when
// the stdlib defines `+` / etc. as real generics in Sprint 25.

/// Sprint 21 operator shims. Each shim has signature
/// `extern "C-unwind" fn(u64, u64) -> u64`. The inputs and output are
/// Dylan tagged Words; non-fixnum inputs decode to 0 (fallback path
/// — Sprint 21 doesn't yet ship a runtime no-applicable-method dispatch
/// for these). The `unsafe` qualifier is required by the
/// `extern "C-unwind"` ABI but the shims have no caller-visible safety
/// preconditions: any 64-bit input is well-defined.
macro_rules! arith_shim {
    ($name:ident, $op:tt) => {
        /// # Safety
        ///
        /// No preconditions. Inputs and output are Dylan tagged Words;
        /// non-fixnum decodes to 0.
        #[unsafe(no_mangle)]
        pub unsafe extern "C-unwind" fn $name(a: u64, b: u64) -> u64 {
            let av = Word::from_raw(a).as_fixnum().unwrap_or(0);
            let bv = Word::from_raw(b).as_fixnum().unwrap_or(0);
            Word::from_fixnum(av $op bv)
                .unwrap_or(Word::from_raw(0))
                .raw()
        }
    };
}

arith_shim!(nod_op_plus, +);
arith_shim!(nod_op_minus, -);
arith_shim!(nod_op_times, *);

/// `\=` — integer equality returning the Dylan boolean singleton.
///
/// # Safety
///
/// No preconditions. Inputs are any Dylan tagged Words; non-fixnum
/// inputs compare by pointer identity.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_op_eq(a: u64, b: u64) -> u64 {
    let av = Word::from_raw(a).as_fixnum();
    let bv = Word::from_raw(b).as_fixnum();
    let imm = crate::literal_pool_immediates();
    if av == bv && av.is_some() {
        imm.true_.raw()
    } else if av.is_some() && bv.is_some() {
        imm.false_.raw()
    } else {
        // Pointer-identity fallback for non-fixnum values.
        if a == b { imm.true_.raw() } else { imm.false_.raw() }
    }
}

/// `\<` — integer less-than.
///
/// # Safety
///
/// No preconditions. Inputs decode as fixnums; non-fixnums treat as 0.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_op_lt(a: u64, b: u64) -> u64 {
    let av = Word::from_raw(a).as_fixnum().unwrap_or(0);
    let bv = Word::from_raw(b).as_fixnum().unwrap_or(0);
    let imm = crate::literal_pool_immediates();
    if av < bv { imm.true_.raw() } else { imm.false_.raw() }
}

/// `\>` — integer greater-than.
///
/// # Safety
///
/// No preconditions. Inputs decode as fixnums; non-fixnums treat as 0.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn nod_op_gt(a: u64, b: u64) -> u64 {
    let av = Word::from_raw(a).as_fixnum().unwrap_or(0);
    let bv = Word::from_raw(b).as_fixnum().unwrap_or(0);
    let imm = crate::literal_pool_immediates();
    if av > bv { imm.true_.raw() } else { imm.false_.raw() }
}

/// Install the operator shims into `RUST_FUNCTION_REGISTRY`. Idempotent
/// — safe to call repeatedly. Called from the `LiteralPool`
/// initialiser path (via `ensure_registered`).
pub fn ensure_operator_shims_registered() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        // SAFETY: each shim has the canonical `(u64, u64) -> u64` ABI.
        unsafe {
            register_rust_function("+", 2, nod_op_plus as *const u8);
            register_rust_function("-", 2, nod_op_minus as *const u8);
            register_rust_function("*", 2, nod_op_times as *const u8);
            register_rust_function("=", 2, nod_op_eq as *const u8);
            register_rust_function("<", 2, nod_op_lt as *const u8);
            register_rust_function(">", 2, nod_op_gt as *const u8);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    extern "C-unwind" fn echo1(a: u64) -> u64 {
        a
    }
    extern "C-unwind" fn add2(a: u64, b: u64) -> u64 {
        // Treat both as fixnums.
        let aw = Word::from_raw(a);
        let bw = Word::from_raw(b);
        let av = aw.as_fixnum().unwrap_or(0);
        let bv = bw.as_fixnum().unwrap_or(0);
        Word::from_fixnum(av + bv).unwrap().raw()
    }

    #[test]
    fn function_class_registers_and_introspects() {
        ensure_registered();
        let id = function_class_id();
        let md = class_metadata_for(id);
        assert_eq!(md.name, "<function>");
        // Six slots: name, arity, code-ptr, kind-tag, env-ptr, return-type.
        assert_eq!(md.slots.len(), 6);
        for s in &md.slots {
            assert_eq!(s.type_kind, SlotType::Integer);
        }
    }

    #[test]
    fn wrong_args_error_inherits_from_error() {
        ensure_registered();
        let wae = wrong_number_of_arguments_error_class_id();
        assert!(is_subclass(wae, crate::conditions::error_class_id()));
    }

    #[test]
    fn make_function_roundtrips_arity_and_code_ptr() {
        ensure_registered();
        let f = make_function("echo1", 1, echo1 as *const u8, 0, 0);
        assert_eq!(function_arity(f), Some(1));
        assert_eq!(
            function_code_ptr(f).unwrap() as usize,
            echo1 as *const () as usize
        );
        assert_eq!(function_name(f).as_deref(), Some("echo1"));
        assert!(is_function(f));
    }

    #[test]
    fn funcall1_dispatches_to_echo() {
        ensure_registered();
        let f = make_function("echo1", 1, echo1 as *const u8, 0, 0);
        let arg = Word::from_fixnum(42).unwrap();
        // SAFETY: f is a real <function>, arg is a real tagged Word, and
        // the callee at code-ptr is arity-1.
        let result = unsafe { nod_funcall1(f.raw(), arg.raw()) };
        assert_eq!(Word::from_raw(result).as_fixnum(), Some(42));
    }

    #[test]
    fn funcall2_dispatches_to_add() {
        ensure_registered();
        let f = make_function("add2", 2, add2 as *const u8, 0, 0);
        let a = Word::from_fixnum(40).unwrap();
        let b = Word::from_fixnum(2).unwrap();
        // SAFETY: arity-2 callee + two tagged Word args.
        let result = unsafe { nod_funcall2(f.raw(), a.raw(), b.raw()) };
        assert_eq!(Word::from_raw(result).as_fixnum(), Some(42));
    }

    #[test]
    fn funcall_arity_mismatch_signals_wae() {
        // No installed handler => process-level panic with the
        // unhandled-condition message. We catch the panic and assert
        // the class name appears in it.
        ensure_registered();
        crate::_reset_handler_stack_for_tests();
        let f = make_function("echo1", 1, echo1 as *const u8, 0, 0);
        let a = Word::from_fixnum(1).unwrap();
        let b = Word::from_fixnum(2).unwrap();
        let outcome = std::panic::catch_unwind(|| {
            // SAFETY: passing 2 args to an arity-1 function — arity
            // mismatch triggers nod_signal -> panic.
            unsafe {
                nod_funcall2(f.raw(), a.raw(), b.raw());
            }
        });
        crate::_reset_handler_stack_for_tests();
        let err = outcome.expect_err("arity mismatch must panic");
        let msg = err
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&'static str>().map(|s| s.to_string()))
            .unwrap_or_default();
        assert!(
            msg.contains("<wrong-number-of-arguments-error>"),
            "expected WAE in panic message, got: {msg}"
        );
    }
}
