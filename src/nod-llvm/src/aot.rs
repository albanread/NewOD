//! Sprint 39a — ahead-of-time (AOT) build helpers.
//!
//! Two responsibilities, both invoked by `nod-driver`'s `build`
//! subcommand:
//!
//! 1. [`emit_aot_entry_stubs`] — post-process a fresh codegen'd
//!    [`inkwell::module::Module`] in place: rename the user's
//!    Dylan-source `main` function to `nod_user_main` and inject a
//!    fresh `i32 @main()` C entry point that calls
//!    `@nod_aot_main_wrapper`. The JIT path never calls this; only the
//!    AOT driver does.
//!
//! 2. [`emit_object_file`] — write the post-processed module to disk
//!    as a Windows COFF (or ELF on `*nix`) `.obj` file via LLVM's
//!    `TargetMachine::write_to_file`. The output is what `link.exe`
//!    consumes alongside `nod_runtime.lib` to produce the user EXE.
//!
//! ## Why post-process instead of teaching codegen
//!
//! Sprint 39a wants the JIT path untouched. Routing through a thin
//! post-codegen step (vs threading an `aot: bool` flag through
//! `codegen_module_with_key`) keeps the JIT's hot path noise-free and
//! confines AOT-specific symbol manipulation to a single 50-line
//! function. The trade-off — re-walking the module to find `main` —
//! is negligible because module sizes are small at this sprint stage.

use std::path::Path;

use inkwell::OptimizationLevel;
use inkwell::AddressSpace;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::values::BasicMetadataValueEnum;

use crate::jit::JitError;
use crate::symbols::{ModuleManifest, RelocKind};

/// The renamed user `main` symbol the staticlib's
/// `nod_aot_main_wrapper` (in `nod-runtime/src/aot.rs`) calls. Must
/// agree with the `extern "C-unwind"` declaration there.
pub const NOD_USER_MAIN_SYMBOL: &str = "nod_user_main";

/// The Rust-side wrapper exposed by `nod_runtime.lib`. The codegen-
/// injected `i32 @main()` stub forwards to this symbol; the linker
/// resolves it to the static-library object at AOT link time.
pub const NOD_AOT_MAIN_WRAPPER_SYMBOL: &str = "nod_aot_main_wrapper";

/// Sprint 39a — the synthesised function that walks the manifest at
/// startup and fills every `nod_*` global with its runtime-resolved
/// bits. Emitted by [`emit_aot_entry_stubs`] from the manifest.
const NOD_AOT_RESOLVE_RELOCS_SYMBOL: &str = "nod_aot_resolve_relocs";

/// Errors emitted during the AOT post-processing + object emission
/// pipeline. Wraps [`JitError`] for the LLVM-side failures and adds
/// AOT-specific variants for missing entry points.
#[derive(Debug)]
pub enum AotError {
    /// The codegen'd module didn't contain a function named `main` —
    /// the user's source is missing `define function main () … end`.
    MissingMain,
    /// `inkwell` complained while creating the target machine or
    /// emitting the object file.
    Llvm(String),
    /// Stand-in for a structural problem the post-processing pass
    /// can't recover from (e.g. an existing `nod_user_main` symbol
    /// collision).
    Conflict(String),
    /// Underlying JIT engine plumbing failure (target init, etc.).
    Jit(JitError),
}

impl std::fmt::Display for AotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingMain => write!(
                f,
                "AOT: source file must define `main () => () end` for EXE entry"
            ),
            Self::Llvm(s) => write!(f, "AOT/LLVM: {s}"),
            Self::Conflict(s) => write!(f, "AOT/conflict: {s}"),
            Self::Jit(e) => write!(f, "AOT/JIT: {e}"),
        }
    }
}

impl std::error::Error for AotError {}

impl From<JitError> for AotError {
    fn from(e: JitError) -> Self {
        Self::Jit(e)
    }
}

/// Sprint 39a — post-process a codegen'd module in place to add the C
/// `main` entry point an EXE needs.
///
/// Steps:
///   1. Look up the function named `main`. If absent, error out
///      ([`AotError::MissingMain`]).
///   2. Rename it to `nod_user_main` (the symbol the runtime wrapper
///      declares as `extern`).
///   3. Add a fresh `i32 @main()` whose body is:
///         ```llvm
///         %rc = call i32 @nod_aot_main_wrapper()
///         ret i32 %rc
///         ```
///      so the CRT's `mainCRTStartup` calls our `main`, which calls
///      the Rust wrapper, which runs `nod_runtime_init()` and then
///      `nod_user_main()`.
///
/// The injected `main` is declared `External` (not `LinkOnceODR` /
/// `WeakODR`) so the linker treats it as the strong definition of
/// `main` for the EXE.
///
/// # Why the renamed user `main` keeps its original signature
///
/// The brief specifies `nod_user_main() -> i64` on the Rust side. The
/// Dylan-level `main` body is lowered with whatever return type sema
/// inferred (typically `Unit` → no return value at the LLVM level, or
/// `Boolean`/`Integer` → an i64 Word). The Rust wrapper discards the
/// return value, so any signature works at the LLVM level — but the
/// `extern "C-unwind" fn nod_user_main() -> i64` declaration in
/// `nod-runtime` requires the symbol to either have an `i64` return
/// or no return-value site at all. We satisfy this by NOT changing the
/// function's existing signature here; the Dylan-emitted body returns
/// whatever its inferred type lowers to (most commonly `void` for a
/// Unit-returning `main`), and the Rust extern's `i64` return is
/// "what's in `rax` after the call" — a void function leaves `rax`
/// untouched, which the wrapper happens to discard anyway.
///
/// A future sprint could tighten this by inserting an i64-cast
/// trampoline. Sprint 39a accepts the loose contract — the wrapper
/// throws the value away and the hello-world test asserts only on
/// stdout + exit code.
pub fn emit_aot_entry_stubs<'ctx>(
    module: &Module<'ctx>,
    manifest: &ModuleManifest,
) -> Result<(), AotError> {
    // Resist the temptation to rename `<eval-entry>` here — that name
    // is reserved for the JIT path. AOT users write `define function
    // main`.
    let user_main = module.get_function("main").ok_or(AotError::MissingMain)?;

    // Guard against a pre-existing `nod_user_main` (would be unusual —
    // user shouldn't pick that name — but a clear error beats silent
    // overwriting).
    if module.get_function(NOD_USER_MAIN_SYMBOL).is_some() {
        return Err(AotError::Conflict(format!(
            "module already declares a function named `{NOD_USER_MAIN_SYMBOL}` — \
             user source must not collide with the AOT entry-stub renaming"
        )));
    }

    // Step 1+2: rename the user's `main` to `nod_user_main`. inkwell
    // exposes `set_name` on `FunctionValue` via `LLVMSetValueName2`.
    user_main.as_global_value().set_name(NOD_USER_MAIN_SYMBOL);
    // External linkage so the staticlib's extern declaration finds it.
    user_main.set_linkage(Linkage::External);

    let ctx = module.get_context();

    // Step 3: convert every manifest-mentioned external global into a
    // defining `i64 0` global with internal linkage. The runtime
    // resolver (synthesised below) populates each at startup.
    convert_externals_to_defining_storage(module, manifest)?;

    // Step 4: emit the resolver function. It calls a per-RelocKind
    // C-ABI helper for each manifest entry, passing the global's
    // address and any per-kind parameters.
    let resolver_fn = emit_resolve_relocs_function(module, manifest)?;

    // Step 5: emit `i32 @main()` that calls the resolver, then the
    // wrapper, then returns the wrapper's rc.
    let i32_ty = ctx.i32_type();
    let main_ty = i32_ty.fn_type(&[], false);

    // Wrapper extern decl. `nod_runtime.lib` provides the definition.
    let wrapper_fn = match module.get_function(NOD_AOT_MAIN_WRAPPER_SYMBOL) {
        Some(f) => f,
        None => module.add_function(NOD_AOT_MAIN_WRAPPER_SYMBOL, main_ty, Some(Linkage::External)),
    };

    let main_fn = module.add_function("main", main_ty, Some(Linkage::External));
    let entry = ctx.append_basic_block(main_fn, "entry");
    let builder = ctx.create_builder();
    builder.position_at_end(entry);
    builder
        .build_call(resolver_fn, &[], "")
        .map_err(|e| AotError::Llvm(format!("build_call resolver: {e}")))?;
    let call = builder
        .build_call(wrapper_fn, &[], "rc")
        .map_err(|e| AotError::Llvm(format!("build_call wrapper: {e}")))?;
    let rc = call
        .try_as_basic_value()
        .basic()
        .ok_or_else(|| AotError::Llvm("wrapper call returned void".into()))?;
    builder
        .build_return(Some(&rc))
        .map_err(|e| AotError::Llvm(format!("build_return: {e}")))?;

    // Re-verify so a botched IR change here surfaces early (before the
    // driver hands the module to TargetMachine).
    module
        .verify()
        .map_err(|e| AotError::Llvm(format!("post-AOT-stub verify: {e}")))?;
    Ok(())
}

/// Sprint 39a — walk the manifest and convert each external global into
/// a defining `i64 0` global with internal linkage. The runtime-side
/// resolver (emitted by [`emit_resolve_relocs_function`]) populates
/// each at startup.
///
/// Skips symbols that aren't actually present in the module (this
/// happens when optimisation eliminates a load through the global).
fn convert_externals_to_defining_storage<'ctx>(
    module: &Module<'ctx>,
    manifest: &ModuleManifest,
) -> Result<(), AotError> {
    let ctx = module.get_context();
    let i64_ty = ctx.i64_type();
    for entry in &manifest.entries {
        let Some(g) = module.get_global(&entry.symbol) else {
            continue;
        };
        // The global was declared `external` + `externally_initialized`
        // by codegen. Switch to internal storage with a zero initialiser.
        // `set_initializer` removes the external flag at the IR level
        // for the global to be a definition.
        g.set_initializer(&i64_ty.const_zero());
        g.set_linkage(Linkage::Internal);
        g.set_externally_initialized(false);
    }
    Ok(())
}

/// Sprint 39a — emit the `void @nod_aot_resolve_relocs()` function that
/// the `main` stub calls before the user's `main`. The function iterates
/// over every manifest entry and calls the corresponding `nod_aot_set_*`
/// runtime helper to populate the slot with its in-process bits.
fn emit_resolve_relocs_function<'ctx>(
    module: &Module<'ctx>,
    manifest: &ModuleManifest,
) -> Result<inkwell::values::FunctionValue<'ctx>, AotError> {
    let ctx = module.get_context();
    let void_ty = ctx.void_type();
    let i64_ty = ctx.i64_type();
    let i32_ty = ctx.i32_type();
    let isize_ty = ctx.ptr_sized_int_type(
        &inkwell::targets::TargetData::create(""),
        Some(AddressSpace::default()),
    );
    let ptr_ty = ctx.ptr_type(AddressSpace::default());

    // Type signatures for each helper. Use C ABI: `void (...)`.
    let helper_set_imm = void_ty.fn_type(&[ptr_ty.into()], false);
    let helper_set_class_md = void_ty.fn_type(&[ptr_ty.into(), i32_ty.into()], false);
    // `(slot, text_ptr, text_len)` — `len` is `size_t`. We use the
    // target's pointer-sized int to stay portable; on x86_64 Windows
    // that's u64.
    let helper_set_lit = void_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), isize_ty.into()], false);
    // `(slot, key_prefix_ptr, key_prefix_len, site_id)`.
    let helper_set_cache_slot = void_ty.fn_type(
        &[ptr_ty.into(), ptr_ty.into(), isize_ty.into(), i64_ty.into()],
        false,
    );
    // `(slot, name_ptr, name_len)`.
    let helper_set_generic = helper_set_lit;

    // Declare or recover each helper as an external.
    let get_or_add =
        |name: &str, ty: inkwell::types::FunctionType<'ctx>| -> inkwell::values::FunctionValue<'ctx> {
            module
                .get_function(name)
                .unwrap_or_else(|| module.add_function(name, ty, Some(Linkage::External)))
        };

    let set_imm_true = get_or_add("nod_aot_set_imm_true", helper_set_imm);
    let set_imm_false = get_or_add("nod_aot_set_imm_false", helper_set_imm);
    let set_imm_nil = get_or_add("nod_aot_set_imm_nil", helper_set_imm);
    let set_imm_false_wrapper = get_or_add("nod_aot_set_imm_false_wrapper", helper_set_imm);
    let set_class_md = get_or_add("nod_aot_set_class_md", helper_set_class_md);
    let set_strlit = get_or_add("nod_aot_set_strlit", helper_set_lit);
    let set_symlit = get_or_add("nod_aot_set_symlit", helper_set_lit);
    let set_cache_slot = get_or_add("nod_aot_set_cache_slot", helper_set_cache_slot);
    let set_generic = get_or_add("nod_aot_set_generic", helper_set_generic);

    // Declare the `nod_runtime_init` extern. The resolver calls it
    // first so class metadata, condition classes, generics, etc., are
    // registered BEFORE we try to populate the slots that reference
    // them. Pre-Sprint-39a's first hello-world attempt called the
    // resolver before the init wrapper, which left `class_metadata_ptr
    // (<range>)` returning null because `ensure_collections_registered`
    // hadn't run yet.
    let runtime_init = module
        .get_function("nod_runtime_init")
        .unwrap_or_else(|| {
            module.add_function(
                "nod_runtime_init",
                void_ty.fn_type(&[], false),
                Some(Linkage::External),
            )
        });

    let resolver_ty = void_ty.fn_type(&[], false);
    let resolver_fn =
        module.add_function(NOD_AOT_RESOLVE_RELOCS_SYMBOL, resolver_ty, Some(Linkage::Internal));
    let entry = ctx.append_basic_block(resolver_fn, "entry");
    let builder = ctx.create_builder();
    builder.position_at_end(entry);

    // Run init first. Idempotent, so safe even if `nod_aot_main_wrapper`
    // also calls it after the resolver returns (which it does — the
    // wrapper's signature is "init then user_main"). The second call
    // is an atomic load of the `LazyLock` guard, negligible cost.
    builder
        .build_call(runtime_init, &[], "")
        .map_err(|e| AotError::Llvm(format!("call runtime_init: {e}")))?;

    // Per-manifest-entry call emission. Each entry resolves its slot
    // and invokes the appropriate helper. Entries that reference symbols
    // not present in the module (eliminated by codegen / opt) are
    // skipped silently.
    for entry in &manifest.entries {
        let Some(g) = module.get_global(&entry.symbol) else {
            continue;
        };
        let slot_ptr = g.as_pointer_value();
        match &entry.kind {
            RelocKind::ImmTrue => {
                builder
                    .build_call(set_imm_true, &[slot_ptr.into()], "")
                    .map_err(|e| AotError::Llvm(format!("call set_imm_true: {e}")))?;
            }
            RelocKind::ImmFalse => {
                builder
                    .build_call(set_imm_false, &[slot_ptr.into()], "")
                    .map_err(|e| AotError::Llvm(format!("call set_imm_false: {e}")))?;
            }
            RelocKind::ImmNil => {
                builder
                    .build_call(set_imm_nil, &[slot_ptr.into()], "")
                    .map_err(|e| AotError::Llvm(format!("call set_imm_nil: {e}")))?;
            }
            RelocKind::ImmFalseWrapper => {
                builder
                    .build_call(set_imm_false_wrapper, &[slot_ptr.into()], "")
                    .map_err(|e| AotError::Llvm(format!("call set_imm_false_wrapper: {e}")))?;
            }
            RelocKind::ClassMetadata { class_id } => {
                let id = i32_ty.const_int(*class_id as u64, false);
                let args: Vec<BasicMetadataValueEnum<'ctx>> =
                    vec![slot_ptr.into(), id.into()];
                builder
                    .build_call(set_class_md, &args, "")
                    .map_err(|e| AotError::Llvm(format!("call set_class_md: {e}")))?;
            }
            RelocKind::StringLiteral { text } => {
                let (str_ptr, len) = emit_byte_constant(module, &builder, &ctx, text.as_bytes())?;
                let len_v = isize_ty.const_int(len as u64, false);
                let args: Vec<BasicMetadataValueEnum<'ctx>> =
                    vec![slot_ptr.into(), str_ptr.into(), len_v.into()];
                builder
                    .build_call(set_strlit, &args, "")
                    .map_err(|e| AotError::Llvm(format!("call set_strlit: {e}")))?;
            }
            RelocKind::SymbolLiteral { name } => {
                let (str_ptr, len) = emit_byte_constant(module, &builder, &ctx, name.as_bytes())?;
                let len_v = isize_ty.const_int(len as u64, false);
                let args: Vec<BasicMetadataValueEnum<'ctx>> =
                    vec![slot_ptr.into(), str_ptr.into(), len_v.into()];
                builder
                    .build_call(set_symlit, &args, "")
                    .map_err(|e| AotError::Llvm(format!("call set_symlit: {e}")))?;
            }
            RelocKind::CacheSlot { site_id } => {
                let (kp_ptr, kp_len) =
                    emit_byte_constant(module, &builder, &ctx, manifest.key_prefix.as_bytes())?;
                let kp_len_v = isize_ty.const_int(kp_len as u64, false);
                let site_id_v = i64_ty.const_int(*site_id, false);
                let args: Vec<BasicMetadataValueEnum<'ctx>> = vec![
                    slot_ptr.into(),
                    kp_ptr.into(),
                    kp_len_v.into(),
                    site_id_v.into(),
                ];
                builder
                    .build_call(set_cache_slot, &args, "")
                    .map_err(|e| AotError::Llvm(format!("call set_cache_slot: {e}")))?;
            }
            RelocKind::Generic { name } => {
                let (str_ptr, len) = emit_byte_constant(module, &builder, &ctx, name.as_bytes())?;
                let len_v = isize_ty.const_int(len as u64, false);
                let args: Vec<BasicMetadataValueEnum<'ctx>> =
                    vec![slot_ptr.into(), str_ptr.into(), len_v.into()];
                builder
                    .build_call(set_generic, &args, "")
                    .map_err(|e| AotError::Llvm(format!("call set_generic: {e}")))?;
            }
            RelocKind::StubEntry { .. } => {
                // Sprint 39a does not support `define c-function`. The
                // driver already errors out earlier if the lowered module
                // has stub entries — reaching this arm indicates a
                // sema/driver bug.
                return Err(AotError::Conflict(format!(
                    "AOT cannot resolve StubEntry relocation `{}` — Win32 imports \
                     are Sprint 39b's job. The driver should have rejected the input.",
                    entry.symbol
                )));
            }
        }
    }
    builder
        .build_return(None)
        .map_err(|e| AotError::Llvm(format!("resolver build_return: {e}")))?;
    Ok(resolver_fn)
}

/// Emit a private `[N x i8]` constant containing `bytes` and return a
/// pointer to its first element. Used by `emit_resolve_relocs_function`
/// to pass string literals + symbol names + the key prefix as
/// `(ptr, len)` pairs to the runtime helpers.
///
/// Each call creates a fresh global. Doing dedup here would shave a
/// few bytes from the EXE but complicates the code; the linker's COMDAT
/// dedup handles identical constants on its own.
fn emit_byte_constant<'ctx>(
    module: &Module<'ctx>,
    builder: &inkwell::builder::Builder<'ctx>,
    ctx: &inkwell::context::ContextRef<'ctx>,
    bytes: &[u8],
) -> Result<(inkwell::values::PointerValue<'ctx>, usize), AotError> {
    let i8_ty = ctx.i8_type();
    let arr_ty = i8_ty.array_type(bytes.len() as u32);
    let g = module.add_global(arr_ty, Some(AddressSpace::default()), "__nod_aot_str");
    g.set_linkage(Linkage::Private);
    g.set_constant(true);
    let const_arr = i8_ty.const_array(
        &bytes
            .iter()
            .map(|b| i8_ty.const_int(*b as u64, false))
            .collect::<Vec<_>>(),
    );
    g.set_initializer(&const_arr);
    let zero = ctx.i32_type().const_zero();
    let ptr = unsafe {
        builder
            .build_gep(arr_ty, g.as_pointer_value(), &[zero, zero], "")
            .map_err(|e| AotError::Llvm(format!("build_gep byte constant: {e}")))?
    };
    Ok((ptr, bytes.len()))
}

/// Sprint 39a — write `module` to disk as a Windows COFF / ELF `.obj`
/// file at `path`. Caller is `nod-driver`; the produced `.obj` is fed
/// to `link.exe` alongside `nod_runtime.lib` to produce a Dylan EXE.
///
/// # Choices
///
/// - **Triple**: [`TargetMachine::get_default_triple`] — matches the
///   host. Sprint 39a doesn't cross-compile; a future sprint can
///   parameterise this.
/// - **CPU + features**: host CPU via [`TargetMachine::get_host_cpu_name`]
///   and `get_host_cpu_features`. The user's EXE runs on the same
///   machine that built it; using host features lets LLVM emit
///   AVX2/etc when present.
/// - **Optimisation level**: caller chooses. `nod-driver build` passes
///   `OptimizationLevel::Default` (LLVM's `-O2` equivalent) so the
///   shipped EXE is reasonably small + fast; `OptimizationLevel::None`
///   is exposed for debugging.
/// - **RelocMode**: `PIC`. Windows EXEs work fine with PIC (the loader
///   doesn't require it but accepts it), and PIC objects link cleanly
///   against `nod_runtime.lib` regardless of where its code lands in
///   the address space.
/// - **CodeModel**: `Default`. The Default model picks Small on
///   Windows x86_64, which is what we want for a non-huge EXE.
pub fn emit_object_file(
    module: &Module<'_>,
    path: &Path,
    opt_level: OptimizationLevel,
) -> Result<(), AotError> {
    // Initialise the X86 backend so `Target::from_triple` succeeds
    // even if no JIT has been spun up yet in this process. Cheap and
    // idempotent inside LLVM.
    Target::initialize_x86(&InitializationConfig::default());

    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| AotError::Llvm(e.to_string()))?;
    let cpu = TargetMachine::get_host_cpu_name();
    let features = TargetMachine::get_host_cpu_features();

    let machine = target
        .create_target_machine(
            &triple,
            cpu.to_str().unwrap_or("generic"),
            features.to_str().unwrap_or(""),
            opt_level,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| AotError::Llvm(format!("create_target_machine failed for {triple:?}")))?;

    // Ensure the module's data layout + triple match the target machine
    // — `inkwell` doesn't auto-populate these, and `link.exe` will
    // refuse mismatched object files. Setting them on the module is a
    // no-op if they're already set (codegen leaves them blank for JIT
    // use, but a fresh post-codegen module is OK to retag here).
    module.set_triple(&triple);
    module.set_data_layout(&machine.get_target_data().get_data_layout());

    machine
        .write_to_file(module, FileType::Object, path)
        .map_err(|e| AotError::Llvm(e.to_string()))?;
    Ok(())
}

/// Sprint 39a — convenience: the canonical AOT pipeline step that
/// follows `codegen_module_with_key`. Performs the entry-stub injection
/// plus writes the object file in one call. Most callers
/// (`nod-driver`'s `build` subcommand) want exactly this.
pub fn emit_aot_object(
    module: &Module<'_>,
    manifest: &ModuleManifest,
    path: &Path,
    opt_level: OptimizationLevel,
) -> Result<(), AotError> {
    emit_aot_entry_stubs(module, manifest)?;
    emit_object_file(module, path, opt_level)?;
    Ok(())
}

/// Synthetic helper used by tests + drivers to construct
/// `TargetTriple` from a string without dragging `inkwell::targets` into
/// callers. Only public so `nod-driver` can print the chosen triple
/// in `--verbose` output.
pub fn default_triple_string() -> String {
    TargetMachine::get_default_triple().as_str().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use inkwell::context::Context;
    use inkwell::values::AsValueRef;

    fn make_user_main_module<'ctx>(ctx: &'ctx Context) -> Module<'ctx> {
        let module = ctx.create_module("hello");
        let i64_ty = ctx.i64_type();
        let main_ty = i64_ty.fn_type(&[], false);
        let user_main = module.add_function("main", main_ty, Some(Linkage::External));
        let bb = ctx.append_basic_block(user_main, "entry");
        let builder = ctx.create_builder();
        builder.position_at_end(bb);
        let zero = i64_ty.const_zero();
        builder.build_return(Some(&zero)).unwrap();
        module
    }

    #[test]
    fn entry_stub_renames_main() {
        let ctx = Context::create();
        let module = make_user_main_module(&ctx);
        let manifest = ModuleManifest::default();
        emit_aot_entry_stubs(&module, &manifest).expect("entry stub emission");
        // Original `main` should be renamed and a fresh `i32 @main`
        // should now exist as a separate function.
        let user = module.get_function(NOD_USER_MAIN_SYMBOL).unwrap();
        assert_eq!(user.get_name().to_str().unwrap(), NOD_USER_MAIN_SYMBOL);
        let new_main = module.get_function("main").unwrap();
        // Distinct function values.
        assert_ne!(user.as_global_value().as_value_ref(),
                   new_main.as_global_value().as_value_ref());
        // New main returns i32.
        let ret = new_main.get_type().get_return_type();
        assert!(matches!(ret.map(|t| t.is_int_type()), Some(true)));
    }

    #[test]
    fn entry_stub_errors_without_main() {
        let ctx = Context::create();
        let module = ctx.create_module("no_main");
        let manifest = ModuleManifest::default();
        let err = emit_aot_entry_stubs(&module, &manifest);
        assert!(matches!(err, Err(AotError::MissingMain)));
    }

    #[test]
    fn object_file_is_written_and_non_empty() {
        // Run inside the test's tempdir (per-process). Smoke check
        // only — checking COFF magic happens in the higher-level
        // `aot_object_emission` integration test.
        let dir = std::env::temp_dir().join(format!(
            "nod-aot-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hello.obj");
        let ctx = Context::create();
        let module = make_user_main_module(&ctx);
        let manifest = ModuleManifest::default();
        emit_aot_entry_stubs(&module, &manifest).unwrap();
        emit_object_file(&module, &path, OptimizationLevel::None).unwrap();
        let bytes = std::fs::read(&path).expect("read .obj");
        assert!(bytes.len() > 16, "expected non-trivial .obj, got {} bytes", bytes.len());
        // Best-effort cleanup; if removal fails we leave a stray temp
        // file but the test passes.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
