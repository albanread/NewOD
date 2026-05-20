//! `nod-sema` — AST → DFM lowering for the Sprint 06 kernel subset,
//! plus Sprint 07 JIT entry points (`eval_expr_to_string`,
//! `dump_llvm_for_file`, `run_function_to_i64`).
//!
//! Out of scope (emits `LoweringError::Unsupported`):
//!   - Generics, methods, classes, macros.
//!   - Multi-binder `let (a, b) = …`, multi-value return.
//!   - `block` / `for` / `while` / `until`, closures, `local method`.
//!   - Keyword-arg synthetic calls (Sprint 04 carry-over).
//!   - `select` (Sprint 03 carry-over) and multi-cond `case` arms.
//!
//! Kernel subset (lowered):
//!   - `define constant` / `define function` (non-generic).
//!   - `Statement::Expr` and single-binder `Statement::Let`.
//!   - Literal exprs, idents (local + top-level direct-call), `Paren`,
//!     `BinOp` / `UnOp` (integer + float monomorphic), `If`, `Begin`,
//!     `Call` against an ident callee.

mod bench;
pub mod c3;
mod lower;
pub mod optimise;
pub mod stdlib;

pub use bench::{BenchResult, DispatchProfile, bench_fixture, dispatch_profile};
pub use optimise::{
    DispatchResolution, SealingFacts, dump_sealed, narrow_function, resolve_dispatches,
};

use std::path::Path;

use inkwell::context::Context;
use nod_dfm::TypeEstimate;
use nod_llvm::{Jit, codegen_module};

pub use lower::{
    BlockHandlerRegistration, BlockRegistration, LoweredModule, LoweringError, MethodRegistration,
    SealingViolation, dump_classes, lower_function, lower_module, lower_module_full,
};

/// Sprint 17: parse + macro-expand + lower in one shot. Existing
/// `lower_module_full(&Module)` remains for AST-direct testing; this
/// is the entry point all driver-facing helpers (`dump_dfm_for_file`,
/// `run_function_to_i64`, `eval_expr_to_string`) now route through so
/// `unless`-style macros expand before lowering.
///
/// Sprint 20b: ensures `stdlib.dylan` is loaded before lowering the
/// caller's module, and merges the stdlib's macros into the
/// per-call macro table so user code can use `for-each` etc.
pub fn expand_and_lower_module(
    module: &nod_reader::Module,
    source: &nod_reader::SourceMap,
) -> Result<LoweredModule, ExpandLowerError> {
    stdlib::ensure_loaded();
    let mut m = module.clone();
    expand_with_stdlib_macros(&mut m, source).map_err(ExpandLowerError::Macro)?;
    lower_module_full(&m).map_err(ExpandLowerError::Lower)
}

/// Sprint 20b: macro expansion that merges `stdlib_macros()` on top
/// of the user's per-call table. Same semantics as
/// `nod_macro::collect_and_expand` but with the stdlib's macros
/// pre-populated so user code can write `for-each (x in c) … end`.
fn expand_with_stdlib_macros(
    module: &mut nod_reader::Module,
    source: &nod_reader::SourceMap,
) -> Result<nod_macro::MacroTable, Vec<nod_macro::MacroError>> {
    let mut table = stdlib::stdlib_macros().clone();
    nod_macro::collect_macros(module, source, &mut table)?;
    nod_macro::expand_module(module, &table, source)?;
    Ok(table)
}

#[derive(Debug)]
pub enum ExpandLowerError {
    Macro(Vec<nod_macro::MacroError>),
    Lower(Vec<LoweringError>),
}

impl std::fmt::Display for ExpandLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Macro(es) => {
                write!(f, "macro expansion: {} error(s):", es.len())?;
                for e in es {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
            Self::Lower(es) => {
                write!(f, "lower: {} error(s):", es.len())?;
                for e in es {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ExpandLowerError {}

/// Sprint 17 driver helper: read a Dylan file, parse it, macro-expand,
/// return the formatted post-expansion AST. Wired into the future
/// `dump-expanded` CLI subcommand.
pub fn dump_expanded_for_file(path: &Path) -> Result<String, DumpError> {
    stdlib::ensure_loaded();
    let src = std::fs::read_to_string(path).map_err(DumpError::Io)?;
    let mut sm = nod_reader::SourceMap::new();
    let file_id = sm
        .add(path.to_path_buf(), src.clone())
        .map_err(DumpError::SourceMap)?;
    let toks = nod_reader::lex(&src, file_id);
    let pre = nod_reader::scan_preamble(&src);
    let mut module =
        nod_reader::parse_module(&src, &toks, pre.as_ref()).map_err(DumpError::Parse)?;
    expand_with_stdlib_macros(&mut module, &sm).map_err(DumpError::Macro)?;
    Ok(nod_reader::format_ast_module(&module))
}

/// Driver helper: read a Dylan file, parse it, lower it, return the
/// indented DFM dump. The driver will wire this into `dump-dfm` itself —
/// this is the smallest function-shaped entry point that hides the
/// SourceMap + parser plumbing from the driver.
pub fn dump_dfm_for_file(path: &Path) -> Result<String, DumpError> {
    stdlib::ensure_loaded();
    let src = std::fs::read_to_string(path).map_err(DumpError::Io)?;
    let mut sm = nod_reader::SourceMap::new();
    let file_id = sm.add(path.to_path_buf(), src.clone()).map_err(DumpError::SourceMap)?;
    let toks = nod_reader::lex(&src, file_id);
    let pre = nod_reader::scan_preamble(&src);
    let mut module =
        nod_reader::parse_module(&src, &toks, pre.as_ref()).map_err(DumpError::Parse)?;
    expand_with_stdlib_macros(&mut module, &sm).map_err(DumpError::Macro)?;
    let lm = lower_module_full(&module).map_err(DumpError::Lower)?;
    Ok(nod_dfm::format_dfm_module(&lm.functions))
}

/// Driver helper: read a Dylan file, parse + lower + codegen, return the
/// textual LLVM IR. Driver wires this into `dump-llvm`.
pub fn dump_llvm_for_file(path: &Path) -> Result<String, DumpError> {
    stdlib::ensure_loaded();
    let src = std::fs::read_to_string(path).map_err(DumpError::Io)?;
    let mut sm = nod_reader::SourceMap::new();
    let file_id = sm.add(path.to_path_buf(), src.clone()).map_err(DumpError::SourceMap)?;
    let toks = nod_reader::lex(&src, file_id);
    let pre = nod_reader::scan_preamble(&src);
    let mut module =
        nod_reader::parse_module(&src, &toks, pre.as_ref()).map_err(DumpError::Parse)?;
    expand_with_stdlib_macros(&mut module, &sm).map_err(DumpError::Macro)?;
    let lm = lower_module_full(&module).map_err(DumpError::Lower)?;
    let module_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dylan-module");
    let ctx = Context::create();
    let out = codegen_module(&ctx, &lm.functions, module_name).map_err(DumpError::Codegen)?;
    Ok(out.module.print_to_string().to_string())
}

/// Parse + lower + codegen + JIT-call a single Dylan expression. Wraps
/// the expression in a synthetic `<eval-entry>` function whose inferred
/// return type drives the call signature. Single-shot.
pub fn eval_expr_to_string(expr_src: &str) -> Result<String, EvalError> {
    // Wrap the expression in a single-function module so `lower_module`
    // and `codegen_module` can run untouched. The function has no
    // params and no `=> (…)` annotation; lowering infers the return
    // type from the body's final temp.
    //
    // Allow the `let X; expr end` form: callers may write a sequence
    // of statements terminated by `end` (as in the SPRINTS.md acceptance
    // case `let x = 41; x + 1 end`). The Dylan grammar reserves `end`
    // for compound forms, so when the expression *starts* with `let`
    // we strip a trailing `end`; the wrapper supplies its own.
    stdlib::ensure_loaded();
    let trimmed = expr_src.trim();
    let body = if trimmed.starts_with("let ") || trimmed.starts_with("let\t") {
        trimmed.strip_suffix("end").map(str::trim_end).unwrap_or(trimmed)
    } else {
        trimmed
    };
    let wrapped = format!(
        "Module: __eval__\n\
         define function <eval-entry> ()\n  {body}\nend;\n"
    );

    let mut sm = nod_reader::SourceMap::new();
    let file_id = sm
        .add("<eval>", wrapped.clone())
        .map_err(EvalError::SourceMap)?;
    let toks = nod_reader::lex(&wrapped, file_id);
    let pre = nod_reader::scan_preamble(&wrapped);
    let mut module = nod_reader::parse_module(&wrapped, &toks, pre.as_ref())
        .map_err(EvalError::Parse)?;
    expand_with_stdlib_macros(&mut module, &sm).map_err(EvalError::Macro)?;
    let lm = lower_module_full(&module).map_err(EvalError::Lower)?;

    let entry = lm
        .functions
        .iter()
        .find(|f| f.name == "<eval-entry>")
        .ok_or_else(|| EvalError::NoEntry("<eval-entry> missing after lowering".into()))?;
    let return_type = entry.return_type;

    let ctx = Context::create();
    let out = codegen_module(&ctx, &lm.functions, "__eval__").map_err(EvalError::Codegen)?;
    let mut jit = Jit::new(&ctx).map_err(EvalError::Jit)?;
    jit.add_module(out).map_err(EvalError::Jit)?;
    register_methods(&jit, &lm.methods)?;
    register_blocks(&jit, &lm.blocks)?;
    register_top_level_functions(&jit, &lm)?;

    // SAFETY: the JIT'd function takes no params; we transmute to the
    // exact signature dictated by `return_type` and call once. The JIT
    // engine outlives the call (held in `jit`).
    let ptr = unsafe { jit.get_function_ptr("<eval-entry>") }
        .ok_or_else(|| EvalError::NoEntry("<eval-entry>".into()))?;
    Ok(call_and_format(ptr, return_type))
}

/// Lower `source_path`, JIT it, look up `entry_name` (a `() => <integer>`
/// function), call once, return its `i64` result.
pub fn run_function_to_i64(
    source_path: &Path,
    entry_name: &str,
) -> Result<i64, EvalError> {
    stdlib::ensure_loaded();
    let src = std::fs::read_to_string(source_path).map_err(EvalError::Io)?;
    let mut sm = nod_reader::SourceMap::new();
    let file_id = sm
        .add(source_path.to_path_buf(), src.clone())
        .map_err(EvalError::SourceMap)?;
    let toks = nod_reader::lex(&src, file_id);
    let pre = nod_reader::scan_preamble(&src);
    let mut module = nod_reader::parse_module(&src, &toks, pre.as_ref())
        .map_err(EvalError::Parse)?;
    expand_with_stdlib_macros(&mut module, &sm).map_err(EvalError::Macro)?;
    let lm = lower_module_full(&module).map_err(EvalError::Lower)?;

    let target = lm
        .functions
        .iter()
        .find(|f| f.name == entry_name)
        .ok_or_else(|| EvalError::NoEntry(entry_name.to_string()))?;
    if !matches!(target.return_type, TypeEstimate::Integer) {
        return Err(EvalError::ReturnTypeMismatch {
            entry: entry_name.to_string(),
            expected: "<integer>",
            actual: target.return_type.name(),
        });
    }

    let module_name = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dylan-module");
    let ctx = Context::create();
    let out = codegen_module(&ctx, &lm.functions, module_name).map_err(EvalError::Codegen)?;
    let mut jit = Jit::new(&ctx).map_err(EvalError::Jit)?;
    jit.add_module(out).map_err(EvalError::Jit)?;
    register_methods(&jit, &lm.methods)?;
    register_blocks(&jit, &lm.blocks)?;
    register_top_level_functions(&jit, &lm)?;

    let ptr = unsafe { jit.get_function_ptr(entry_name) }
        .ok_or_else(|| EvalError::NoEntry(entry_name.to_string()))?;
    // SAFETY: target.return_type is Integer (checked above), no params
    // on the DFM function, and `<integer>` lowers to a tagged `Word`
    // (Sprint 09 ABI). Engine outlives the call.
    let f: extern "C" fn() -> u64 = unsafe { std::mem::transmute(ptr) };
    let w = nod_runtime::Word::from_raw(f());
    w.as_fixnum().ok_or_else(|| EvalError::ReturnTypeMismatch {
        entry: entry_name.to_string(),
        expected: "<integer> (fixnum)",
        actual: "<pointer-tagged> (Sprint 09 has no boxed integers)",
    })
}

/// Resolve every method's body function in the JIT and register the
/// resulting `(specialisers, fn_ptr)` pair in the runtime's dispatch
/// table. Runs once after `Jit::add_module` returns; the registrations
/// are process-global so subsequent calls just see them.
///
/// Sprint 13 passes the full specialiser list to
/// `nod_runtime::add_method_full` so multi-argument dispatch
/// (`intersect(<rect>, <circle>)` etc.) picks the right method.
/// Sprint 21: register every top-level Dylan function in the lowered
/// module with the runtime's function-ref registry, so that
/// `nod_make_function_ref(name, arity)` resolves to the JIT-emitted
/// address. Skips block-form lifted thunks (body / cleanup /
/// afterwards / handler) — those have a different ABI and aren't
/// callable from a `<function>` Word.
pub fn register_top_level_functions(
    jit: &Jit<'_>,
    lm: &LoweredModule,
) -> Result<(), EvalError> {
    // Build the set of names belonging to block-form lifted thunks
    // (the only Dylan-level functions that DON'T match the regular
    // `(u64, ..., u64) -> u64` calling convention; their leading
    // params are the captured-locals slots, not user args). Sprint 19
    // emits these with predictable names — we read them out of the
    // `blocks` registration list.
    let mut block_thunk_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for b in &lm.blocks {
        block_thunk_names.insert(b.body_fn_name.clone());
        if let Some(n) = &b.cleanup_fn_name {
            block_thunk_names.insert(n.clone());
        }
        if let Some(n) = &b.afterwards_fn_name {
            block_thunk_names.insert(n.clone());
        }
        for h in &b.handlers {
            block_thunk_names.insert(h.body_fn_name.clone());
        }
    }
    // Auto-generated slot accessors and method bodies belong to
    // generics; we register THEIR names ALSO so `\size` on a method-
    // name resolves to the generic dispatcher. But since those are
    // already in `lm.methods` and registered into the dispatch table
    // with `add_method_named`, we can rely on the generic registry
    // instead — `nod_make_function_ref` won't need a separate entry
    // for `size`.
    //
    // Sprint 21 simplification: register EVERY top-level function
    // whose name isn't a block thunk. The function-ref registry is
    // keyed on `(name, arity)`, so collisions are impossible across
    // arities.
    // Names whose addresses we register under the SOURCE name (vs the
    // mangled body symbol). For methods, the source name lives in
    // `lm.methods[i].generic_name` and the body name in
    // `lm.methods[i].body_fn_name`. Build a map from body name to
    // source name so the function pass below can register under both.
    let mut body_to_source: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for m in &lm.methods {
        body_to_source
            .entry(m.body_fn_name.clone())
            .or_insert_with(|| m.generic_name.clone());
    }
    for f in &lm.functions {
        if block_thunk_names.contains(&f.name) {
            continue;
        }
        // Skip the synthetic `<eval-entry>` so it isn't reachable via
        // `\<eval-entry>` from inside the evaluated body.
        if f.name == "<eval-entry>" {
            continue;
        }
        let arity = f.params.len();
        // SAFETY: get_function_ptr returns a valid JIT'd address;
        // the JIT engine outlives the registration (callers leak it).
        let ptr = unsafe { jit.get_function_ptr(&f.name) }.ok_or_else(|| {
            EvalError::NoEntry(format!("top-level function `{}` not JIT'd", f.name))
        })?;
        // SAFETY: ptr is JIT-emitted, signature `(u64*arity) -> u64`.
        unsafe {
            nod_runtime::register_jit_function(&f.name, arity, ptr as *const u8);
            // If this function is a method body (e.g. stdlib's
            // `size$<object>` body), also register under the
            // generic-source name so `\size` resolves.
            //
            // Sprint 22 fix: only register under the generic source name
            // if no entry exists yet — otherwise a later method body
            // (e.g. `size$<table>`) would overwrite the earlier
            // `<object>` body, and `\size` would call the wrong method
            // for non-table arguments. The first-registered method body
            // tends to be the most-general one (loader processes
            // `define function size` before `define method size(t ::
            // <table>)`), so first-wins matches "most general fallback".
            // The proper fix — generic-dispatcher trampoline — lands
            // when first-class dispatch wraps a `<function>` (DEFERRED).
            if let Some(src) = body_to_source.get(&f.name)
                && src != &f.name
                && nod_runtime::lookup_function_code(src, arity).is_none()
            {
                nod_runtime::register_jit_function(src, arity, ptr as *const u8);
            }
        }
    }
    Ok(())
}

pub fn register_methods(
    jit: &Jit<'_>,
    methods: &[MethodRegistration],
) -> Result<(), EvalError> {
    for m in methods {
        // SAFETY: the JIT engine outlives the registration; the body
        // function's `(u64, ..., u64) -> u64` signature is what the
        // dispatcher expects.
        let ptr = unsafe { jit.get_function_ptr(&m.body_fn_name) }.ok_or_else(|| {
            EvalError::NoEntry(format!(
                "method body `{}` not JIT'd",
                m.body_fn_name
            ))
        })?;
        // SAFETY: ptr is the live JIT'd function, matches `(u64, ..., u64) -> u64`.
        // Sprint 16: pass the JIT symbol name so the Sprint 15 dispatch
        // resolver can emit a `DirectCall` against the exact emitted
        // symbol — slot accessors (`<C>-getter-x`) don't follow the
        // `{generic}${specialisers}` convention `add_method_full`
        // assumes.
        unsafe {
            nod_runtime::add_method_named(
                &m.generic_name,
                m.specialisers.clone(),
                ptr as *const u8,
                m.param_count,
                &m.body_fn_name,
            );
        }
    }
    Ok(())
}

/// Sprint 19: resolve every `block` form's lifted thunks to JIT
/// addresses and register them with the runtime. Runs once after the
/// JIT finalises a module.
pub fn register_blocks(
    jit: &Jit<'_>,
    blocks: &[crate::lower::BlockRegistration],
) -> Result<(), EvalError> {
    for b in blocks {
        // SAFETY: JIT engine outlives the registration. The thunk
        // signatures match `extern "C-unwind" fn(u64, ..., u64) -> u64`.
        let body = unsafe { jit.get_function_ptr(&b.body_fn_name) }.ok_or_else(|| {
            EvalError::NoEntry(format!("block body `{}` not JIT'd", b.body_fn_name))
        })?;
        let cleanup = match &b.cleanup_fn_name {
            Some(n) => Some(
                unsafe { jit.get_function_ptr(n) }
                    .ok_or_else(|| EvalError::NoEntry(format!("block cleanup `{n}` not JIT'd")))?
                    as *const u8,
            ),
            None => None,
        };
        let afterwards = match &b.afterwards_fn_name {
            Some(n) => Some(
                unsafe { jit.get_function_ptr(n) }
                    .ok_or_else(|| EvalError::NoEntry(format!("block afterwards `{n}` not JIT'd")))?
                    as *const u8,
            ),
            None => None,
        };
        let handlers: Vec<nod_runtime::HandlerFn> = b
            .handlers
            .iter()
            .map(|h| {
                let p = unsafe { jit.get_function_ptr(&h.body_fn_name) }.ok_or_else(|| {
                    EvalError::NoEntry(format!("block handler `{}` not JIT'd", h.body_fn_name))
                })?;
                // Pin the class name as a static byte slice. Leaking is
                // intentional — these names live for the process.
                let pinned: &'static str = Box::leak(h.class_name.clone().into_boxed_str());
                Ok(nod_runtime::HandlerFn {
                    class_id: h.class_id,
                    class_name_ptr: pinned.as_ptr(),
                    class_name_len: pinned.len(),
                    body: p as *const u8,
                })
            })
            .collect::<Result<_, EvalError>>()?;
        let handlers_static: &'static [nod_runtime::HandlerFn] = Box::leak(handlers.into_boxed_slice());
        nod_runtime::register_block_fns(
            b.block_id,
            nod_runtime::BlockFns {
                body: body as *const u8,
                cleanup,
                afterwards,
                handlers: handlers_static,
            },
        );
    }
    Ok(())
}

fn call_and_format(ptr: *const (), ty: TypeEstimate) -> String {
    // SAFETY: each branch transmutes to the function signature implied
    // by the temp/return type the lowering pass produced. The JIT
    // memory backing `ptr` is kept alive by the caller's `Jit`.
    //
    // Sprint 10 ABI: `<integer>`, `<boolean>`, `<string>`, and Top/Bottom
    // returns are all a tagged `Word` packed into an `i64`.
    match ty {
        // Sprint 15: a `Class(_)` / `Singleton(_)` return value is still
        // a tagged `Word` packed into an `i64` (same ABI as `Top`); the
        // formatter walks the wrapper to surface the class name.
        TypeEstimate::Class(_) | TypeEstimate::Singleton(_) => {
            // SAFETY: ptr has signature `() -> u64`.
            let f: extern "C" fn() -> u64 = unsafe { std::mem::transmute(ptr) };
            let w = nod_runtime::Word::from_raw(f());
            format_pointer_word(w)
        }
        TypeEstimate::Integer | TypeEstimate::Top | TypeEstimate::Bottom => {
            // SAFETY: ptr has signature `() -> u64`.
            let f: extern "C" fn() -> u64 = unsafe { std::mem::transmute(ptr) };
            let w = nod_runtime::Word::from_raw(f());
            match w.as_fixnum() {
                Some(n) => n.to_string(),
                // Pointer-tagged return — surface the class.
                None => format_pointer_word(w),
            }
        }
        TypeEstimate::Boolean => {
            // SAFETY: ptr has signature `() -> u64`.
            let f: extern "C" fn() -> u64 = unsafe { std::mem::transmute(ptr) };
            let raw = f();
            let imm = nod_runtime::literal_pool_immediates();
            if raw == imm.false_.raw() {
                "#f".to_string()
            } else {
                "#t".to_string()
            }
        }
        TypeEstimate::SingleFloat => {
            // SAFETY: ptr has signature `() -> f32`.
            let f: extern "C" fn() -> f32 = unsafe { std::mem::transmute(ptr) };
            format!("{}", f() as f64)
        }
        TypeEstimate::DoubleFloat => {
            // SAFETY: ptr has signature `() -> f64`.
            let f: extern "C" fn() -> f64 = unsafe { std::mem::transmute(ptr) };
            format!("{}", f())
        }
        TypeEstimate::Character => {
            // SAFETY: ptr has signature `() -> u32`.
            let f: extern "C" fn() -> u32 = unsafe { std::mem::transmute(ptr) };
            match char::from_u32(f()) {
                Some(c) => format!("'{c}'"),
                None => "<bad-char>".to_string(),
            }
        }
        TypeEstimate::Unit => {
            // SAFETY: ptr has signature `()`.
            let f: extern "C" fn() = unsafe { std::mem::transmute(ptr) };
            f();
            "#unit".to_string()
        }
        TypeEstimate::String => {
            // SAFETY: ptr has signature `() -> u64`.
            let f: extern "C" fn() -> u64 = unsafe { std::mem::transmute(ptr) };
            let w = nod_runtime::Word::from_raw(f());
            format_pointer_word(w)
        }
    }
}

/// Render a pointer-tagged Word by reading its wrapper class. Sprint
/// 10 special-cases `<byte-string>` (print the contents) and the
/// pinned immediates; everything else prints as the class name.
fn format_pointer_word(w: nod_runtime::Word) -> String {
    if !w.is_pointer() {
        return format!("<non-pointer-word:{:#x}>", w.raw());
    }
    let imm = nod_runtime::literal_pool_immediates();
    if w == imm.true_ {
        return "#t".to_string();
    }
    if w == imm.false_ {
        return "#f".to_string();
    }
    if w == imm.nil {
        return "#()".to_string();
    }
    // SAFETY: every pointer-tagged Dylan Word in Sprint 10 either
    // points into the heap or into the pinned immediates region. The
    // wrapper-first invariant lets us read the wrapper directly.
    let Some(wrap) = (unsafe { nod_runtime::wrapper_of_unchecked(w) }) else {
        return format!("<bad-word:{:#x}>", w.raw());
    };
    if wrap.class() == nod_runtime::ClassId::BYTE_STRING {
        // SAFETY: class match implies <byte-string> layout.
        if let Some(bs) =
            unsafe { nod_runtime::try_byte_string(w, nod_runtime::ClassId::BYTE_STRING) }
        {
            // SAFETY: bs points at live allocation.
            return match unsafe { bs.as_str() } {
                Some(s) => format!("{s:?}"),
                None => format!("<non-utf8 byte-string len={}>", bs.len),
            };
        }
    }
    // Sprint 21: `<simple-object-vector>` prints as `#(elt0, elt1, …)`
    // matching Dylan's source-literal form. Used by the
    // `dylan_map_squares_three_element_list` headline test, which
    // produces an SOV via `map(...)`.
    if wrap.class() == nod_runtime::ClassId::SIMPLE_OBJECT_VECTOR {
        // SAFETY: class match implies SOV layout.
        if let Some(sov) = unsafe {
            nod_runtime::try_simple_object_vector(w, nod_runtime::ClassId::SIMPLE_OBJECT_VECTOR)
        } {
            // SAFETY: sov points at live allocation.
            let slots = unsafe { sov.slots() };
            let parts: Vec<String> = slots.iter().map(|s| format_element(*s)).collect();
            return format!("#({})", parts.join(", "));
        }
    }
    // Sprint 21: `<pair>` / `<empty-list>` cons-cell list pretty-print.
    if wrap.class() == nod_runtime::ClassId::PAIR
        || wrap.class() == nod_runtime::ClassId::EMPTY_LIST
    {
        return format_list(w);
    }
    format!("<{:?} @ {:#x}>", wrap.class(), w.raw() & !1)
}

/// Helper: render a single Word as it appears INSIDE a collection
/// literal. Fixnums print as their decimal value; pointer-tagged
/// values recurse through `format_pointer_word`.
fn format_element(w: nod_runtime::Word) -> String {
    if let Some(n) = w.as_fixnum() {
        return n.to_string();
    }
    format_pointer_word(w)
}

/// Render a `<pair>` / `<empty-list>` chain as `#(elt0, elt1, …)`.
fn format_list(w: nod_runtime::Word) -> String {
    let imm = nod_runtime::literal_pool_immediates();
    let mut parts: Vec<String> = Vec::new();
    let mut cur = w;
    while cur != imm.nil {
        // SAFETY: walking a Sprint 16 cons-cell chain; `try_pair` checks
        // the wrapper class and returns `None` if `cur` isn't a pair.
        let Some(p) = (unsafe { nod_runtime::try_pair(cur, nod_runtime::ClassId::PAIR) })
        else {
            break;
        };
        parts.push(format_element(p.head));
        cur = p.tail;
    }
    format!("#({})", parts.join(", "))
}

#[derive(Debug)]
pub enum DumpError {
    Io(std::io::Error),
    SourceMap(nod_reader::SourceMapError),
    Parse(Vec<nod_reader::Diagnostic>),
    Macro(Vec<nod_macro::MacroError>),
    Lower(Vec<LoweringError>),
    Codegen(nod_llvm::CodegenError),
}

impl std::fmt::Display for DumpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DumpError::Io(e) => write!(f, "io: {e}"),
            DumpError::SourceMap(e) => write!(f, "source map: {e}"),
            DumpError::Parse(d) => write!(f, "parse: {} diagnostic(s)", d.len()),
            DumpError::Macro(errs) => {
                write!(f, "macro: {} error(s):", errs.len())?;
                for e in errs {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
            DumpError::Lower(errs) => {
                write!(f, "lower: {} error(s):", errs.len())?;
                for e in errs {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
            DumpError::Codegen(e) => write!(f, "codegen: {e}"),
        }
    }
}

impl std::error::Error for DumpError {}

#[derive(Debug)]
pub enum EvalError {
    Io(std::io::Error),
    SourceMap(nod_reader::SourceMapError),
    Parse(Vec<nod_reader::Diagnostic>),
    Macro(Vec<nod_macro::MacroError>),
    Lower(Vec<LoweringError>),
    Codegen(nod_llvm::CodegenError),
    Jit(nod_llvm::JitError),
    NoEntry(String),
    ReturnTypeMismatch {
        entry: String,
        expected: &'static str,
        actual: &'static str,
    },
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::Io(e) => write!(f, "io: {e}"),
            EvalError::SourceMap(e) => write!(f, "source map: {e}"),
            EvalError::Parse(d) => write!(f, "parse: {} diagnostic(s)", d.len()),
            EvalError::Macro(errs) => {
                write!(f, "macro: {} error(s):", errs.len())?;
                for e in errs {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
            EvalError::Lower(errs) => {
                write!(f, "lower: {} error(s):", errs.len())?;
                for e in errs {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
            EvalError::Codegen(e) => write!(f, "codegen: {e}"),
            EvalError::Jit(e) => write!(f, "jit: {e}"),
            EvalError::NoEntry(n) => write!(f, "entry function not found: `{n}`"),
            EvalError::ReturnTypeMismatch { entry, expected, actual } => write!(
                f,
                "entry `{entry}` returns {actual}, expected {expected}"
            ),
        }
    }
}

impl std::error::Error for EvalError {}
