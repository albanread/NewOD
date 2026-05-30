//! Sprint 51b — in-process JIT-strapped Dylan-side lexer.
//!
//! `--lex-with-dylan` materialises `dylan-lexer.dylan` +
//! `dylan-lex-shim.dylan` into a temp directory, runs the full
//! parse → expand → lower pipeline, JITs the result into an isolated
//! MCJIT engine via [`nod_sema::jit_lowered_module`], looks up
//! `dylan-lex-collect`, and installs a `nod_reader::lex` override
//! whose body wraps the raw fn pointer behind a Word-marshalling
//! shim.
//!
//! The bridge is the contract from `docs/DYLAN_TOKEN_WIRE.md` plus
//! one ABI agreement:
//!
//!   * `dylan-lex-collect(source: <byte-string>) => <stretchy-vector>`
//!     compiles to a Word-in, Word-out C-callable:
//!     `extern "C" fn(u64) -> u64`.
//!   * The returned `<stretchy-vector>` holds `3N` boxed fixnums —
//!     `(kind, lo, hi)` triples ending at the EOF token.
//!
//! ## Isolation
//!
//! Each successful `init` leaks ONE `Context + Jit` pair. The Dylan
//! runtime's global registries (classes, dispatch caches, stub table)
//! are shared with any other JIT engine the host process runs — that's
//! by design: the lex-shim's classes (`<token>`, `<span>`, …) need to
//! be visible if/when downstream code interrogates them. But the
//! engine's compiled code is segregated from any user-code JIT engine
//! the host might spin up later for `eval`-style commands.
//!
//! ## Calling overhead
//!
//! Per lex call: one O(n) byte-by-byte allocation of a `<byte-string>`
//! (n FFI calls), one Dylan call, one O(3T) stretchy-vector readback
//! (T = token count). On corpus fixtures this should be ~100× faster
//! than spawning a subprocess and ~comparable to or faster than the
//! Rust path itself (the Dylan lex implementation is in scope of the
//! same LLVM `-O0` codegen the Rust lex would JIT through).
//!
//! ## Failure mode
//!
//! If `init` fails (compile error, missing entry, …) the override
//! never gets installed and `nod_reader::lex` keeps using
//! `lex_rust`. The init error is surfaced as a stderr message;
//! callers shouldn't fall over.

use std::path::PathBuf;
use std::sync::OnceLock;

use nod_reader::{FileId, Span, Token, TokenKind};
use nod_runtime::Word;

// Shim sources — same `include_str!` pattern the existing
// `dump-dylan-tokens` infrastructure uses.

const DYLAN_LEXER_SOURCE: &str =
    include_str!("../../../tests/nod-tests/fixtures/dylan-lexer.dylan");
const DYLAN_LEX_SHIM_SOURCE: &str =
    include_str!("../../../tests/nod-tests/fixtures/dylan-lex-shim.dylan");

/// Signature of `dylan-lex-collect`. Word in, Word out.
type CollectFn = unsafe extern "C" fn(u64) -> u64;

/// Process-wide handle to the side-loaded engine + entry pointer.
/// Held inside a `OnceLock` keyed by the static `JIT_HANDLE` — first
/// `init` wins, subsequent calls observe the loaded state.
struct DylanLexJit {
    collect: CollectFn,
    // The `JittedModule` keeps the underlying `Jit` alive (leaked) so
    // `collect` stays valid for the process lifetime. We hold the
    // handle even though we never look up another symbol; dropping
    // it would be undefined-behaviour territory because the fn
    // pointer aliases JIT-owned memory.
    _handle: nod_sema::JittedModule,
}

// SAFETY: the leaked MCJIT engine is read-only after init; `CollectFn`
// is a raw fn pointer (`Send + Sync` by definition). The Dylan side
// is single-threaded by virtue of the runtime's heap not being
// thread-safe yet, but the LEX override has to be `Send + Sync`-able
// for `OnceLock`. Callers must not actually parallel-call `lex`.
unsafe impl Send for DylanLexJit {}
unsafe impl Sync for DylanLexJit {}

static JIT_HANDLE: OnceLock<DylanLexJit> = OnceLock::new();

/// Materialise the shim sources into a deterministic cache dir,
/// lower + JIT them, look up `dylan-lex-collect`, and stash the
/// result in [`JIT_HANDLE`]. Idempotent — repeat calls observe the
/// already-loaded handle.
///
/// Returns `Ok(())` on success or first-cache-hit; `Err(message)`
/// on any failure (compile error, missing entry function, etc.).
pub fn init() -> Result<(), String> {
    if JIT_HANDLE.get().is_some() {
        return Ok(());
    }
    let handle = build()?;
    let _ = JIT_HANDLE.set(handle);
    Ok(())
}

fn cache_dir() -> Result<PathBuf, String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    DYLAN_LEXER_SOURCE.hash(&mut h);
    DYLAN_LEX_SHIM_SOURCE.hash(&mut h);
    env!("CARGO_PKG_VERSION").hash(&mut h);
    let digest = h.finish();
    let dir = std::env::temp_dir().join(format!("nod-dylan-lex-jit-{digest:016x}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create cache dir {}: {e}", dir.display()))?;
    Ok(dir)
}

fn build() -> Result<DylanLexJit, String> {
    let dir = cache_dir()?;
    let lexer_path = dir.join("dylan-lexer.dylan");
    let shim_path = dir.join("dylan-lex-shim.dylan");
    std::fs::write(&lexer_path, DYLAN_LEXER_SOURCE)
        .map_err(|e| format!("write {}: {e}", lexer_path.display()))?;
    std::fs::write(&shim_path, DYLAN_LEX_SHIM_SOURCE)
        .map_err(|e| format!("write {}: {e}", shim_path.display()))?;

    // Reuse the AOT multi-file front-end: parse each file, concat AST,
    // lower once, merge stdlib. Same `LoweredModule` shape the
    // AOT pipeline would consume.
    // Use the AOT-style multi-file front-end: stdlib MUST be merged into
    // the lowered module so codegen can resolve calls to user-stdlib
    // functions (e.g. `make-string-stream`, called from
    // `print-token-to-string` in the lexer). The JIT-friendly sibling
    // (`compile_files_for_jit`) leaves those out and codegen aborts with
    // `unknown callee`. The double-registration risk vs. the existing
    // pre-loaded stdlib is OK in practice: the host process either
    // (a) only runs this lexer JIT and so the registry mutations are
    // its only consumer, or (b) was running another JIT engine, in
    // which case the stdlib addresses in the global registry get
    // overwritten with our new ones and any subsequent user-code lookup
    // routes to our copy. The user-code JIT path itself rebinds before
    // calling, so the swap is invisible at user-call time. A later
    // sprint can layer in per-engine registries if we ever need real
    // isolation.
    let paths: [&std::path::Path; 2] = [&lexer_path, &shim_path];
    let lm = nod_sema::compile_files_for_aot(&paths)
        .map_err(|e| format!("compile_files_for_aot: {e}"))?;

    let handle = nod_sema::jit_lowered_module(&lm, "dylan_lex_jit")
        .map_err(|e| format!("jit_lowered_module: {e}"))?;


    // Resolve the entry point. The Dylan name `dylan-lex-collect`
    // round-trips into the LLVM symbol of the same name (front-end
    // doesn't sanitise the dash).
    // SAFETY: `JittedModule::get_function_ptr` returns a raw pointer
    // into the leaked MCJIT engine; we immediately transmute to the
    // calling-convention-correct fn type.
    let ptr = unsafe { handle.get_function_ptr("dylan-lex-collect") }.ok_or_else(|| {
        "dylan_lex_jit: dylan-lex-collect not found in JIT module (sema must have lowered it \
         as a top-level function)"
            .to_string()
    })?;
    let collect: CollectFn = unsafe { std::mem::transmute::<*const (), CollectFn>(ptr) };

    Ok(DylanLexJit { collect, _handle: handle })
}

/// `nod_reader::LexFn`-compatible entry point. Falls back to
/// `nod_reader::lex_rust` if the JIT never got initialised — should
/// never happen if the driver registers this only after a successful
/// `init`, but defensive.
pub fn lex(src: &str, file_id: FileId) -> Vec<Token> {
    let Some(jit) = JIT_HANDLE.get() else {
        return nod_reader::lex_rust(src, file_id);
    };

    // Step 1 — build a Dylan `<byte-string>` from `src.as_bytes()`.
    let bytes = src.as_bytes();
    let len_word = Word::from_fixnum(bytes.len() as i64).expect("source under fixnum max");
    // SAFETY: `nod_byte_string_allocate` is the runtime's vetted
    // constructor; it allocates a `<byte-string>` of size `len` and
    // returns the tagged Word pointer.
    let bs_raw = unsafe { nod_runtime::nod_byte_string_allocate(len_word.raw()) };
    for (i, &b) in bytes.iter().enumerate() {
        let byte_word = Word::from_fixnum(b as i64).expect("byte fits");
        let i_word = Word::from_fixnum(i as i64).expect("offset fits");
        // SAFETY: `bs_raw` is the byte-string we just allocated, still
        // live and not yet handed to the JIT'd lex.
        unsafe {
            nod_runtime::nod_byte_string_element_setter(byte_word.raw(), bs_raw, i_word.raw());
        }
    }

    // Step 2 — call `dylan-lex-collect(bs)`. Word in, Word out.
    // SAFETY: `bs_raw` is a valid `<byte-string>` pointer; the JIT'd
    // function is the entry installed by `init`. Single-threaded —
    // no concurrent GC can move `bs_raw` during the call (Dylan's
    // runtime is STW and the only mutator is the JIT we're calling
    // into, which threads its own safepoints).
    let sv_raw = unsafe { (jit.collect)(bs_raw) };

    // Step 3 — walk the returned stretchy-vector as (kind, lo, hi)
    // triples.
    let size_word_raw = unsafe { nod_runtime::nod_stretchy_vector_size(sv_raw) };
    let size = Word::from_raw(size_word_raw)
        .as_fixnum()
        .expect("size is fixnum") as usize;
    debug_assert!(
        size % 3 == 0,
        "dylan-lex-collect returned {size} ints — not a multiple of 3 (kind, lo, hi)"
    );

    let mut tokens = Vec::with_capacity(size / 3);
    let mut i = 0;
    while i + 2 < size {
        let kind_raw = unsafe {
            nod_runtime::nod_stretchy_vector_element(
                sv_raw,
                Word::from_fixnum(i as i64).unwrap().raw(),
            )
        };
        let lo_raw = unsafe {
            nod_runtime::nod_stretchy_vector_element(
                sv_raw,
                Word::from_fixnum((i + 1) as i64).unwrap().raw(),
            )
        };
        let hi_raw = unsafe {
            nod_runtime::nod_stretchy_vector_element(
                sv_raw,
                Word::from_fixnum((i + 2) as i64).unwrap().raw(),
            )
        };

        let kind_ord = Word::from_raw(kind_raw)
            .as_fixnum()
            .expect("kind is fixnum");
        let lo = Word::from_raw(lo_raw).as_fixnum().expect("lo is fixnum") as u32;
        let hi = Word::from_raw(hi_raw).as_fixnum().expect("hi is fixnum") as u32;
        let kind = token_kind_from_ordinal(kind_ord);
        tokens.push(Token { kind, span: Span::new(file_id, lo, hi) });
        i += 3;
    }

    tokens
}

/// Map the wire-format kind ordinal (`docs/DYLAN_TOKEN_WIRE.md` §3)
/// back to a [`TokenKind`]. The discriminants ARE the `#[repr(u8)]`
/// ordinals of `TokenKind`, but we go through an explicit match so a
/// future enum reshuffle fails loudly here rather than producing
/// silently-corrupt tokens via `transmute`.
fn token_kind_from_ordinal(n: i64) -> TokenKind {
    match n {
        0 => TokenKind::Ident,
        1 => TokenKind::KwDefine,
        2 => TokenKind::KwEnd,
        3 => TokenKind::KwOtherwise,
        4 => TokenKind::EscapedIdent,
        5 => TokenKind::HashTrue,
        6 => TokenKind::HashFalse,
        7 => TokenKind::HashLParen,
        8 => TokenKind::HashLBracket,
        9 => TokenKind::HashLBrace,
        10 => TokenKind::HashHash,
        11 => TokenKind::HashRest,
        12 => TokenKind::HashKey,
        13 => TokenKind::HashAllKeys,
        14 => TokenKind::HashNext,
        15 => TokenKind::HashIncludeMarker,
        16 => TokenKind::Symbol,
        17 => TokenKind::HashKeyword,
        18 => TokenKind::IntegerBin,
        19 => TokenKind::IntegerOct,
        20 => TokenKind::IntegerHex,
        21 => TokenKind::KeywordColon,
        22 => TokenKind::Integer,
        23 => TokenKind::Float,
        24 => TokenKind::Ratio,
        25 => TokenKind::String,
        26 => TokenKind::StringMulti,
        27 => TokenKind::StringRaw,
        28 => TokenKind::Char,
        29 => TokenKind::LParen,
        30 => TokenKind::RParen,
        31 => TokenKind::LBracket,
        32 => TokenKind::RBracket,
        33 => TokenKind::LBrace,
        34 => TokenKind::RBrace,
        35 => TokenKind::Comma,
        36 => TokenKind::Semicolon,
        37 => TokenKind::Dot,
        38 => TokenKind::Ellipsis,
        39 => TokenKind::Colon,
        40 => TokenKind::ColonColon,
        41 => TokenKind::ColonEqual,
        42 => TokenKind::Equal,
        43 => TokenKind::EqualEqual,
        44 => TokenKind::Arrow,
        45 => TokenKind::Tilde,
        46 => TokenKind::TildeEqual,
        47 => TokenKind::TildeEqualEqual,
        48 => TokenKind::Plus,
        49 => TokenKind::Minus,
        50 => TokenKind::Star,
        51 => TokenKind::Slash,
        52 => TokenKind::Caret,
        53 => TokenKind::Amp,
        54 => TokenKind::Bar,
        55 => TokenKind::Less,
        56 => TokenKind::Greater,
        57 => TokenKind::LessEqual,
        58 => TokenKind::GreaterEqual,
        59 => TokenKind::Query,
        60 => TokenKind::QueryQuery,
        61 => TokenKind::QueryEqual,
        62 => TokenKind::QueryAt,
        63 => TokenKind::Eof,
        64 => TokenKind::Invalid,
        other => panic!(
            "dylan_lex_jit: unrecognised token kind {other} from wire format \
             (extend docs/DYLAN_TOKEN_WIRE.md §3 and this match together)"
        ),
    }
}
