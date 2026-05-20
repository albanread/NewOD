//! Sprint 20b — Dylan stdlib auto-loader.
//!
//! The first call to any public `nod-sema` entry point
//! (`eval_expr_to_string`, `lower_module`, `lower_module_full`,
//! `run_function_to_i64`, the various `dump_*_for_file` helpers)
//! routes through [`ensure_loaded`]. The loader parses
//! `src/nod-dylan/dylan-sources/stdlib.dylan` once, runs the
//! macro engine over it, lowers it to DFM, and JIT-compiles every
//! function. Side effects on `nod_runtime`'s process-global
//! registries (macro table, dispatch table) make the stdlib's
//! definitions visible to subsequent user-code lowering / JITting
//! within the same process.
//!
//! ## How user code reaches stdlib symbols
//!
//! Sprint 20b doesn't yet link separate JIT modules together via
//! shared symbol resolution — the stdlib's JIT engine resides
//! behind a `OnceLock`, but user-code `Jit` instances created in
//! `eval_expr_to_string` etc. are independent. To make
//! `size(c)` in user code resolve, the loader rewrites every
//! `define function` in `stdlib.dylan` to `define method <name>
//! (param :: <object>, …)` BEFORE lowering. This registers each
//! function as a single-method generic against the most-general
//! specialisers. Generic dispatch lives in `nod_runtime` and is
//! process-global, so user code's `Dispatch` IR node (emitted by
//! `nod-sema/src/lower.rs` line ~2138 for known generic names)
//! finds the stdlib method through the same path it uses for
//! user-defined generics.
//!
//! Macros from `stdlib.dylan` populate a process-global macro
//! registry (`stdlib_macros`) which `expand_and_lower_module`
//! merges into the per-call `MacroTable` before expansion.
//!
//! ## Lifetime story
//!
//! The stdlib `Context` is leaked (`Box::leak`) so it lives for
//! the process. The stdlib `Jit` is moved into a static `OnceLock`
//! and never dropped — same pattern Sprint 13/19 used for runtime
//! helpers. Method body pointers registered with
//! `nod_runtime::add_method_named` reference the leaked JIT's
//! memory, so dispatch finds them forever.

use std::sync::OnceLock;

use inkwell::context::Context;
use nod_llvm::{Jit, codegen_module};
use nod_macro::MacroTable;
use nod_reader::{Item, Param, ReturnSig, Span};

use crate::lower::{MethodRegistration, lower_module_full};
use crate::{register_blocks, register_methods, register_top_level_functions};

/// Static-area for the stdlib LLVM context + JIT. Leaking the
/// engine is deliberate — Sprint 20b doesn't reclaim it, and the
/// addresses registered with `nod_runtime` must outlive every user
/// JIT.
static STDLIB_ARTEFACTS: OnceLock<&'static StdlibArtefacts> = OnceLock::new();

/// What the loader hands back. Mostly informational — the
/// dispatch-table / macro-registry side effects are the real
/// payload.
#[derive(Debug)]
pub struct StdlibArtefacts {
    /// Names of every function lowered from `stdlib.dylan`.
    pub function_names: Vec<String>,
    /// Method registrations the loader installed (post-rewrite of
    /// `define function` → `define method ... <object>`).
    pub method_registrations: Vec<MethodRegistration>,
    /// Names of every macro registered (so user-code expansion can
    /// find them via the process-global table merge).
    pub macro_names: Vec<String>,
}

/// Process-global macro table populated from `stdlib.dylan`. Read
/// (without modification) by `expand_and_lower_module` and merged
/// on top of each call's local macro table so user code can use
/// `for-each`, etc.
static STDLIB_MACROS: OnceLock<MacroTable> = OnceLock::new();

/// Sprint 20b: macro entries the loader collected. User-side
/// expansion merges these into the per-call `MacroTable`.
pub(crate) fn stdlib_macros() -> &'static MacroTable {
    STDLIB_MACROS.get().expect(
        "stdlib_macros() called before ensure_loaded(); \
         the lib.rs entry points call ensure_loaded() first.",
    )
}

/// Top-level entry: parse + expand + lower + JIT `stdlib.dylan`
/// exactly once. Idempotent; subsequent calls return the cached
/// artefacts. Errors during the first call are panicked on — the
/// stdlib is a compile-time-bundled source, so failure indicates
/// an internal-inconsistency bug, not a user error.
pub fn ensure_loaded() -> &'static StdlibArtefacts {
    if let Some(a) = STDLIB_ARTEFACTS.get() {
        return a;
    }
    // Single-shot load. `OnceLock::get_or_init` would race with
    // re-entrancy through nod-sema's lowering helpers; we use a
    // manual fast-then-slow path with `set` on first success.
    let artefacts = load_stdlib().expect("stdlib.dylan failed to load — internal bug");
    let leaked: &'static StdlibArtefacts = Box::leak(Box::new(artefacts));
    let _ = STDLIB_ARTEFACTS.set(leaked);
    STDLIB_ARTEFACTS
        .get()
        .copied()
        .expect("STDLIB_ARTEFACTS was just set")
}

#[derive(Debug)]
enum LoadError {
    Parse(Vec<nod_reader::Diagnostic>),
    Macro(Vec<nod_macro::MacroError>),
    Lower(Vec<crate::lower::LoweringError>),
    Codegen(nod_llvm::CodegenError),
    Jit(nod_llvm::JitError),
    NoEntry(String),
    SourceMap(nod_reader::SourceMapError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Parse(d) => write!(f, "stdlib parse: {} diagnostic(s)", d.len()),
            LoadError::Macro(es) => {
                write!(f, "stdlib macro: {} error(s)", es.len())?;
                for e in es {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
            LoadError::Lower(es) => {
                write!(f, "stdlib lower: {} error(s)", es.len())?;
                for e in es {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
            LoadError::Codegen(e) => write!(f, "stdlib codegen: {e}"),
            LoadError::Jit(e) => write!(f, "stdlib jit: {e}"),
            LoadError::NoEntry(n) => write!(f, "stdlib: function `{n}` missing post-JIT"),
            LoadError::SourceMap(e) => write!(f, "stdlib source map: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

fn load_stdlib() -> Result<StdlibArtefacts, LoadError> {
    // Sprint 20b: register the seed collection + condition classes
    // up-front. The stdlib references them by name (`<collection>`,
    // `<error>`, …); if they aren't registered yet `find_class_id_by_name`
    // misses and lowering fails. `ensure_*_registered` is idempotent.
    nod_runtime::ensure_conditions_registered();
    nod_runtime::ensure_collections_registered();
    nod_runtime::ensure_tables_registered();

    let src = include_str!("../../nod-dylan/dylan-sources/stdlib.dylan");
    let mut sm = nod_reader::SourceMap::new();
    let file_id = sm.add("<stdlib>", src.to_string()).map_err(LoadError::SourceMap)?;
    let toks = nod_reader::lex(src, file_id);
    let pre = nod_reader::scan_preamble(src);
    let mut module = nod_reader::parse_module(src, &toks, pre.as_ref()).map_err(LoadError::Parse)?;

    // Collect macros from stdlib INTO the process-global table.
    // Don't expand stdlib's own macro uses here — Sprint 20b's
    // stdlib doesn't call its own macros internally, so the table
    // population is the only side effect we need.
    let mut macro_table = MacroTable::default();
    nod_macro::collect_macros(&module, &sm, &mut macro_table).map_err(LoadError::Macro)?;
    // Drop the macro definitions from the module so lowering doesn't
    // see them again. Mirrors `expand_module`'s cleanup step.
    module.items.retain(|it| !matches!(it, Item::DefineMacro { .. }));
    let macro_names: Vec<String> = macro_table.defs.keys().cloned().collect();
    let _ = STDLIB_MACROS.set(macro_table);

    // Rewrite every `define function` in stdlib into a `define method
    // f (p1 :: <object>, p2 :: <object>, …)` so user code's
    // `Dispatch` IR resolves to it via the process-global dispatch
    // table. This is the cheapest way to make stdlib symbols
    // callable from a separate JIT engine without wiring
    // cross-module symbol resolution (deferred to Sprint 21).
    rewrite_define_function_to_method(&mut module);

    let lm = lower_module_full(&module).map_err(LoadError::Lower)?;

    // Codegen + JIT — leak the Context so engine pointers stay live
    // for the process. The Jit value itself is moved into the leaked
    // artefacts box so engines persist.
    let ctx_box: &'static Context = Box::leak(Box::new(Context::create()));
    let out = codegen_module(ctx_box, &lm.functions, "__nod_stdlib__").map_err(LoadError::Codegen)?;
    let mut jit = Jit::new(ctx_box).map_err(LoadError::Jit)?;
    jit.add_module(out).map_err(LoadError::Jit)?;

    // Wire methods + blocks into the process-global registries.
    register_methods(&jit, &lm.methods).map_err(|e| match e {
        crate::EvalError::NoEntry(n) => LoadError::NoEntry(n),
        crate::EvalError::Jit(e) => LoadError::Jit(e),
        other => LoadError::NoEntry(format!("stdlib method registration: {other}")),
    })?;
    register_blocks(&jit, &lm.blocks).map_err(|e| match e {
        crate::EvalError::NoEntry(n) => LoadError::NoEntry(n),
        crate::EvalError::Jit(e) => LoadError::Jit(e),
        other => LoadError::NoEntry(format!("stdlib block registration: {other}")),
    })?;
    // Sprint 21: register every stdlib `define function` body in the
    // process-global function-ref registry so `\size`, `\reduce`, etc.
    // are reachable as first-class function values from user code.
    register_top_level_functions(&jit, &lm).map_err(|e| match e {
        crate::EvalError::NoEntry(n) => LoadError::NoEntry(n),
        crate::EvalError::Jit(e) => LoadError::Jit(e),
        other => LoadError::NoEntry(format!("stdlib top-level fn registration: {other}")),
    })?;

    // Leak the Jit so engine + emitted code live forever. The
    // Box::leak yields a `&'static mut Jit<'static>`; we drop the
    // reference because we only need the side effects.
    let _: &'static mut Jit<'static> = Box::leak(Box::new(jit));

    let function_names = lm.functions.iter().map(|f| f.name.clone()).collect();

    Ok(StdlibArtefacts {
        function_names,
        method_registrations: lm.methods,
        macro_names,
    })
}

/// Pre-lowering rewrite: every `Item::DefineFunction { name, params,
/// body, … }` becomes `Item::DefineMethod { name, params (typed as
/// <object>), body, … }`. This makes the stdlib's functions
/// dispatchable as single-method generics on the maximally-general
/// specialisers, so user code's `Dispatch` IR resolves to them.
///
/// Sprint 20b: a small, surgical transform. The brief authorises
/// stdlib-side judgment calls; cross-module symbol linkage is the
/// principled fix and is deferred to Sprint 21 (see DEFERRED.md).
fn rewrite_define_function_to_method(module: &mut nod_reader::Module) {
    let span_dummy = Span {
        file_id: nod_reader::FileId(0),
        lo: 0,
        hi: 0,
    };
    for item in &mut module.items {
        let new = match item {
            Item::DefineFunction {
                span,
                name,
                modifiers,
                params,
                return_,
                body,
            } => {
                if params.is_empty() {
                    // 0-arg functions can't become methods (Dylan
                    // generics require at least one specialiser).
                    // Leave them as direct-call top-level functions —
                    // user code can't reach them via dispatch, which
                    // is fine for stdlib internals.
                    None
                } else {
                    // Synthesise `<object>` type annotations on every
                    // unannotated parameter. Already-annotated params
                    // stay as-is (the stdlib doesn't currently do this,
                    // but keeps the loader robust against future edits).
                    let typed_params: Vec<Param> = params
                        .iter()
                        .map(|p| {
                            let t = p.type_.clone().unwrap_or_else(|| {
                                nod_reader::Expr::Ident(span_dummy, "<object>".to_string())
                            });
                            Param {
                                span: p.span,
                                name: p.name.clone(),
                                type_: Some(t),
                            }
                        })
                        .collect();
                    Some(Item::DefineMethod {
                        span: *span,
                        name: name.clone(),
                        modifiers: modifiers.clone(),
                        params: typed_params,
                        return_: clone_return_sig(return_.as_ref()),
                        body: body.clone(),
                    })
                }
            }
            _ => None,
        };
        if let Some(replacement) = new {
            *item = replacement;
        }
    }
}

fn clone_return_sig(sig: Option<&ReturnSig>) -> Option<ReturnSig> {
    sig.cloned()
}
