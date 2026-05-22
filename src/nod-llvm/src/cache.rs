//! Sprint 37 — JIT object-code cache.
//!
//! ## Cache key
//!
//! The cache is keyed on the DFM IR text plus compiler / runtime /
//! LLVM / target / opt-level versioning. DFM IR is our own data
//! structure under our control; we audited it for determinism in
//! Phase A and confirmed identical Dylan source produces byte-identical
//! DFM IR (modulo a handful of fixable sources — process-global block
//! ids are now derived from `(parent_name, thunk_seq)` hashes; closure
//! captures already sort by name; anon-method counters reset per
//! lowering call). LLVM IR has known nondeterminism sources (cache
//! slot pointers, runtime fn addresses baked into i64 immediates), so
//! we hash the *upstream* DFM IR, not LLVM IR.
//!
//! ## Storage layout
//!
//! On-disk cache directory:
//!
//! ```text
//!   <cache_dir>/
//!     <hex_key>.bc      ← LLVM bitcode produced by codegen_module
//!     <hex_key>.json    ← sidecar metadata (created/accessed times, size, ABI versions)
//! ```
//!
//! The cache directory defaults to:
//!   - `$NOD_JIT_CACHE_DIR` if set
//!   - `$CARGO_TARGET_DIR/nod-jit-cache/` for dev builds (CARGO_TARGET_DIR set)
//!   - `target/nod-jit-cache/` if `Cargo.toml` is visible from cwd
//!   - `%LOCALAPPDATA%/NewOpenDylan/jit-cache/` on Windows otherwise
//!
//! ## Cache mechanism
//!
//! Note: LLVM-C does not expose `MCJIT::setObjectCache`; the
//! `LLVMObjectCache` C++ API is not bound through the C API in either
//! `llvm-sys` 221 or `inkwell` 0.9. As a result Sprint 37's "object
//! code on disk" intent lands in two complementary layers:
//!
//! 1. **In-process JIT-output cache** — a process-global
//!    `Mutex<HashMap<CacheKey, JitEntry>>` storing already-JIT'd
//!    function pointers and the binding metadata required to invoke
//!    them. On a cache hit the *entire* pipeline (parse, lower,
//!    codegen, MCJIT compile, registrations) is skipped. This delivers
//!    the headline ≥10× speedup for repeated `eval_expr_to_string`
//!    calls in the same process — which is exactly the IDE-shell hot
//!    re-eval path the sprint targets.
//!
//! 2. **On-disk bitcode + sidecar metadata** — every cold compile
//!    persists the post-codegen LLVM bitcode and a sidecar JSON to
//!    `<cache_dir>/<hex_key>.bc/.json`. Today this is observable
//!    infrastructure (LRU eviction, statistics, env-var override,
//!    corruption tolerance) and a forward investment in Sprint 38 AOT.
//!    Cross-process bitcode reuse is gated on Sprint 38: today's
//!    bitcode references baked-in runtime addresses that differ per
//!    process, so cross-process replay would link against stale
//!    pointers. Sprint 38 will fix-up these references at load time.

use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Sprint 37 — version stamp folded into every cache key. Bump when
/// any `extern "C-unwind"` runtime ABI signature changes, when the
/// DFM-IR text format used by [`nod_dfm::format_for_cache_key`]
/// changes, or when codegen changes the IR shape it emits for a given
/// DFM input. Drives both in-process and on-disk cache invalidation —
/// keys minted under an older value cannot match anything cached under
/// the new value.
pub const NOD_RUNTIME_ABI_VERSION: u32 = 1;

/// LLVM major version we link against. Sourced from
/// `llvm_sys::LLVM_VERSION_MAJOR` at compile time. Bump in lockstep
/// with the workspace's `llvm-sys` pin.
pub const LLVM_MAJOR: u32 = 22;

/// Target triple of the host. The cache key is per-triple so a Windows
/// vs. Linux build never share entries even if they share a dev disk.
pub fn target_triple() -> &'static str {
    // Sprint 37 is Windows-only (MessageBoxW / IDE-shell context);
    // hardcode the host triple. A future cross-platform sprint will
    // resolve this via `llvm_sys::target_machine::LLVMGetDefaultTargetTriple`.
    "x86_64-pc-windows-msvc"
}

/// MCJIT optimization level. Mirrors the value set in `jit.rs`.
pub const OPT_LEVEL: u8 = 2;

/// 256-bit cache key. Composed of four 64-bit SipHash 1-3 digests
/// computed with distinct domain-separation seeds. SipHash from
/// `std::collections::hash_map::DefaultHasher` is stable across
/// process runs (fixed seeds, see Rust stdlib docs), which is the
/// determinism property we need.
///
/// 256 bits is well above the collision-probability budget for a
/// 500MB / ~1000-entry cache: birthday-bound is ~2^128, far beyond
/// what disk capacity admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey(pub [u64; 4]);

impl CacheKey {
    /// Render as a 64-character lowercase hex string. Used for
    /// filenames and as the wire format in sidecar JSON.
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for word in self.0 {
            // Little-endian byte order so a `[u64; 4]` reads the same
            // as the byte stream a future SHA-256 swap would produce.
            for byte in word.to_le_bytes() {
                s.push_str(&format!("{byte:02x}"));
            }
        }
        s
    }
}

/// Compute the 256-bit cache key for a DFM module text and the
/// versioning inputs that make stale-but-textually-equal sources miss.
pub fn cache_key(
    dfm_text: &str,
    nod_version: &str,
    abi_version: u32,
    llvm_major: u32,
    target: &str,
    opt_level: u8,
) -> CacheKey {
    let mut words = [0u64; 4];
    for (i, word) in words.iter_mut().enumerate() {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // Domain-separation seed per word — the byte sequence is
        // arbitrary but unique per slot so the four hashes are
        // independent.
        (b"nod-jit-cache-key", i as u32).hash(&mut h);
        dfm_text.hash(&mut h);
        nod_version.hash(&mut h);
        abi_version.hash(&mut h);
        llvm_major.hash(&mut h);
        target.hash(&mut h);
        opt_level.hash(&mut h);
        *word = h.finish();
    }
    CacheKey(words)
}

/// Convenience constructor that fills in the workspace's standard
/// versioning constants ([`NOD_RUNTIME_ABI_VERSION`], [`LLVM_MAJOR`],
/// [`target_triple`], [`OPT_LEVEL`]) and the `CARGO_PKG_VERSION` of
/// the `nod-llvm` crate.
pub fn cache_key_for_dfm(dfm_text: &str) -> CacheKey {
    cache_key(
        dfm_text,
        env!("CARGO_PKG_VERSION"),
        NOD_RUNTIME_ABI_VERSION,
        LLVM_MAJOR,
        target_triple(),
        OPT_LEVEL,
    )
}

// ─── Sidecar JSON ──────────────────────────────────────────────────

/// Sidecar metadata stored next to each `.bc` file. Read on load to
/// (a) refresh access time for LRU eviction and (b) cross-check that
/// the file on disk wasn't compiled under an incompatible
/// nod/LLVM/ABI/target tuple (defense-in-depth against renaming a
/// cache dir between incompatible builds).
#[derive(Debug, Clone, PartialEq)]
pub struct SidecarMeta {
    pub key_hex: String,
    pub created_at_unix_ms: u64,
    pub accessed_at_unix_ms: u64,
    pub size_bytes: u64,
    pub nod_version: String,
    pub abi_version: u32,
    pub llvm_major: u32,
    pub target_triple: String,
    pub opt_level: u8,
}

impl SidecarMeta {
    fn to_json(&self) -> String {
        // Hand-rolled minimal JSON encoder — the workspace doesn't
        // currently pull `serde_json` into `nod-llvm`'s dep tree and
        // the schema is fixed. Strings are constrained to the safe
        // subset (hex digests + version strings + the literal target
        // triple), so we don't need general-purpose escaping.
        format!(
            "{{\
\"key\":\"{}\",\
\"created_at_unix_ms\":{},\
\"accessed_at_unix_ms\":{},\
\"size_bytes\":{},\
\"nod_version\":\"{}\",\
\"abi_version\":{},\
\"llvm_major\":{},\
\"target_triple\":\"{}\",\
\"opt_level\":{}\
}}",
            self.key_hex,
            self.created_at_unix_ms,
            self.accessed_at_unix_ms,
            self.size_bytes,
            self.nod_version,
            self.abi_version,
            self.llvm_major,
            self.target_triple,
            self.opt_level,
        )
    }

    fn parse(text: &str) -> Option<Self> {
        // Tiny key-extractor over the hand-rolled schema. Returns
        // `None` if any required field is missing or malformed —
        // callers treat that as "corrupted entry, ignore + recompile".
        fn find_str(s: &str, k: &str) -> Option<String> {
            let needle = format!("\"{k}\":\"");
            let i = s.find(&needle)?;
            let after = &s[i + needle.len()..];
            let j = after.find('"')?;
            Some(after[..j].to_string())
        }
        fn find_u64(s: &str, k: &str) -> Option<u64> {
            let needle = format!("\"{k}\":");
            let i = s.find(&needle)?;
            let after = &s[i + needle.len()..];
            let end = after.find([',', '}'])?;
            after[..end].trim().parse().ok()
        }
        Some(Self {
            key_hex: find_str(text, "key")?,
            created_at_unix_ms: find_u64(text, "created_at_unix_ms")?,
            accessed_at_unix_ms: find_u64(text, "accessed_at_unix_ms")?,
            size_bytes: find_u64(text, "size_bytes")?,
            nod_version: find_str(text, "nod_version")?,
            abi_version: find_u64(text, "abi_version")? as u32,
            llvm_major: find_u64(text, "llvm_major")? as u32,
            target_triple: find_str(text, "target_triple")?,
            opt_level: find_u64(text, "opt_level")? as u8,
        })
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ─── Disk cache directory + LRU ────────────────────────────────────

/// Resolve the on-disk cache directory at process start, honouring
/// `$NOD_JIT_CACHE_DIR` first.
pub fn default_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NOD_JIT_CACHE_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("nod-jit-cache");
    }
    // Walk up from cwd looking for a `target/` directory (dev builds
    // run from a workspace root or one of its subdirs); fall back to
    // `%LOCALAPPDATA%` for installed binaries.
    if let Ok(cwd) = std::env::current_dir() {
        let mut probe: &Path = &cwd;
        loop {
            let candidate = probe.join("target");
            if candidate.is_dir() {
                return candidate.join("nod-jit-cache");
            }
            match probe.parent() {
                Some(p) => probe = p,
                None => break,
            }
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA")
        && !local.is_empty()
    {
        return PathBuf::from(local).join("NewOpenDylan").join("jit-cache");
    }
    PathBuf::from("nod-jit-cache")
}

/// Default LRU cap. Override via `$NOD_JIT_CACHE_MAX_BYTES`.
pub const DEFAULT_MAX_BYTES: u64 = 500 * 1024 * 1024;

pub fn cache_max_bytes() -> u64 {
    std::env::var("NOD_JIT_CACHE_MAX_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_BYTES)
}

/// Write `bitcode` to `<dir>/<key>.bc` and a fresh sidecar JSON next
/// to it. Errors are logged to stderr and swallowed — the cache is an
/// optimization, not a correctness mechanism.
pub fn write_cache_entry(dir: &Path, key: CacheKey, bitcode: &[u8]) {
    if let Err(e) = fs::create_dir_all(dir) {
        eprintln!("nod-jit-cache: create_dir_all({dir:?}) failed: {e}");
        return;
    }
    let hex = key.to_hex();
    let bc_path = dir.join(format!("{hex}.bc"));
    let meta_path = dir.join(format!("{hex}.json"));
    if let Err(e) = fs::write(&bc_path, bitcode) {
        eprintln!("nod-jit-cache: write {bc_path:?} failed: {e}");
        return;
    }
    let now = now_unix_ms();
    let meta = SidecarMeta {
        key_hex: hex.clone(),
        created_at_unix_ms: now,
        accessed_at_unix_ms: now,
        size_bytes: bitcode.len() as u64,
        nod_version: env!("CARGO_PKG_VERSION").to_string(),
        abi_version: NOD_RUNTIME_ABI_VERSION,
        llvm_major: LLVM_MAJOR,
        target_triple: target_triple().to_string(),
        opt_level: OPT_LEVEL,
    };
    if let Err(e) = fs::write(&meta_path, meta.to_json()) {
        eprintln!("nod-jit-cache: write {meta_path:?} failed: {e}");
    }
}

/// Look up `<dir>/<key>.bc`. On success returns `(bitcode_bytes, meta)`
/// and bumps the sidecar's `accessed_at_unix_ms`. On any error
/// (missing file, malformed sidecar, ABI/triple/version mismatch) the
/// entry is treated as absent and the caller falls back to a fresh
/// compile.
pub fn read_cache_entry(dir: &Path, key: CacheKey) -> Option<(Vec<u8>, SidecarMeta)> {
    let hex = key.to_hex();
    let bc_path = dir.join(format!("{hex}.bc"));
    let meta_path = dir.join(format!("{hex}.json"));
    let bytes = fs::read(&bc_path).ok()?;
    let meta_text = fs::read_to_string(&meta_path).ok()?;
    let mut meta = SidecarMeta::parse(&meta_text)?;
    if meta.key_hex != hex
        || meta.abi_version != NOD_RUNTIME_ABI_VERSION
        || meta.llvm_major != LLVM_MAJOR
        || meta.target_triple != target_triple()
        || meta.opt_level != OPT_LEVEL
        || meta.size_bytes as usize != bytes.len()
    {
        return None;
    }
    meta.accessed_at_unix_ms = now_unix_ms();
    // Best-effort sidecar refresh so LRU eviction prioritises recently
    // touched entries — errors don't fail the read.
    let _ = fs::write(&meta_path, meta.to_json());
    Some((bytes, meta))
}

/// Walk `dir`, sort by `accessed_at_unix_ms` ascending, delete oldest
/// until total size ≤ `max_bytes`. Best-effort; errors per-entry are
/// logged and skipped. Returns the count of evicted entries.
pub fn evict_to(dir: &Path, max_bytes: u64) -> usize {
    let read = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut entries: Vec<(PathBuf, SidecarMeta)> = Vec::new();
    let mut total: u64 = 0;
    for de in read.flatten() {
        let path = de.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Some(meta) = SidecarMeta::parse(&text) else {
            continue;
        };
        total = total.saturating_add(meta.size_bytes);
        entries.push((path, meta));
    }
    if total <= max_bytes {
        return 0;
    }
    entries.sort_by_key(|(_, m)| m.accessed_at_unix_ms);
    let mut evicted = 0;
    for (json_path, meta) in entries {
        if total <= max_bytes {
            break;
        }
        let bc_path = json_path.with_extension("bc");
        let _ = fs::remove_file(&json_path);
        let _ = fs::remove_file(&bc_path);
        total = total.saturating_sub(meta.size_bytes);
        evicted += 1;
    }
    evicted
}

/// Test helper / `%jit-cache-clear()` primitive backend. Walks the
/// directory and deletes every `*.bc` / `*.json` pair.
pub fn clear_cache_dir(dir: &Path) {
    let read = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for de in read.flatten() {
        let path = de.path();
        match path.extension().and_then(|s| s.to_str()) {
            Some("bc") | Some("json") => {
                let _ = fs::remove_file(&path);
            }
            _ => {}
        }
    }
}

/// Compute the on-disk size of every entry in `dir`.
pub fn cache_size_on_disk(dir: &Path) -> u64 {
    let read = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut total = 0u64;
    for de in read.flatten() {
        let path = de.path();
        if path.extension().and_then(|s| s.to_str()) != Some("bc")
            && path.extension().and_then(|s| s.to_str()) != Some("json")
        {
            continue;
        }
        if let Ok(meta) = fs::metadata(&path) {
            total = total.saturating_add(meta.len());
        }
    }
    total
}

/// Number of `<key>.bc` files (one per cache entry).
pub fn cache_entry_count(dir: &Path) -> u32 {
    let read = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut n = 0u32;
    for de in read.flatten() {
        if de.path().extension().and_then(|s| s.to_str()) == Some("bc") {
            n += 1;
        }
    }
    n
}

// ─── Process-wide statistics ───────────────────────────────────────

#[derive(Debug, Default, Clone, Copy)]
pub struct JitCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub bytes_on_disk: u64,
    pub entries: u32,
}

#[derive(Default)]
struct StatsInner {
    hits: u64,
    misses: u64,
}

static STATS: LazyLock<Mutex<StatsInner>> = LazyLock::new(|| Mutex::new(StatsInner::default()));

pub fn record_hit() {
    if let Ok(mut g) = STATS.lock() {
        g.hits += 1;
    }
}

pub fn record_miss() {
    if let Ok(mut g) = STATS.lock() {
        g.misses += 1;
    }
}

pub fn reset_stats() {
    if let Ok(mut g) = STATS.lock() {
        *g = StatsInner::default();
    }
}

pub fn read_stats(dir: &Path) -> JitCacheStats {
    let (hits, misses) = STATS
        .lock()
        .map(|g| (g.hits, g.misses))
        .unwrap_or((0, 0));
    JitCacheStats {
        hits,
        misses,
        bytes_on_disk: cache_size_on_disk(dir),
        entries: cache_entry_count(dir),
    }
}

// ─── In-process JIT-output cache ───────────────────────────────────
//
// Stores opaque per-key payloads. The JIT installs callbacks that
// build the payload from a fresh compile and that "replay" it on a
// cache hit; this module just owns the table and serialises access.

pub type ReplayFn = Box<dyn Fn() -> JitReplayResult + Send + Sync>;

/// Replay outcome — a successful in-process hit returns a function
/// pointer (`<eval-entry>` JIT address) plus the formatted-result type
/// tag (looked up from the cached entry's recorded entry return
/// type). The caller's `call_and_format` consumes both.
#[derive(Debug, Clone)]
pub struct JitReplayResult {
    pub eval_entry_ptr: usize,
    /// Encoded `nod_dfm::TypeEstimate` discriminant — a u32 because
    /// `nod_dfm` doesn't have a stable repr we can rely on at the FFI
    /// boundary. The JIT-side encoder/decoder lives in `sema`.
    pub return_type_tag: u32,
    /// For `Class(_)` / `Singleton(_)` carry the inner value.
    pub return_type_payload: u64,
}

struct InProcessEntry {
    replay: ReplayFn,
}

static IN_PROC: LazyLock<Mutex<HashMap<CacheKey, InProcessEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn in_process_get(key: CacheKey) -> Option<JitReplayResult> {
    let g = IN_PROC.lock().ok()?;
    let entry = g.get(&key)?;
    let result = (entry.replay)();
    Some(result)
}

pub fn in_process_insert(key: CacheKey, replay: ReplayFn) {
    if let Ok(mut g) = IN_PROC.lock() {
        g.insert(key, InProcessEntry { replay });
    }
}

pub fn in_process_contains(key: CacheKey) -> bool {
    IN_PROC.lock().map(|g| g.contains_key(&key)).unwrap_or(false)
}

pub fn in_process_clear() {
    if let Ok(mut g) = IN_PROC.lock() {
        g.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_deterministic() {
        let k1 = cache_key("foo", "0.0.1", 1, 22, "x86_64-pc-windows-msvc", 2);
        let k2 = cache_key("foo", "0.0.1", 1, 22, "x86_64-pc-windows-msvc", 2);
        assert_eq!(k1, k2);
    }

    #[test]
    fn key_changes_with_source() {
        let k1 = cache_key("foo", "0.0.1", 1, 22, "x86_64-pc-windows-msvc", 2);
        let k2 = cache_key("foo2", "0.0.1", 1, 22, "x86_64-pc-windows-msvc", 2);
        assert_ne!(k1, k2);
    }

    #[test]
    fn key_changes_with_abi_version() {
        let k1 = cache_key("foo", "0.0.1", 1, 22, "x86_64-pc-windows-msvc", 2);
        let k2 = cache_key("foo", "0.0.1", 2, 22, "x86_64-pc-windows-msvc", 2);
        assert_ne!(k1, k2);
    }

    #[test]
    fn key_hex_round_trip_is_64_chars() {
        let k = cache_key("foo", "0.0.1", 1, 22, "x86_64-pc-windows-msvc", 2);
        let hex = k.to_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sidecar_round_trips() {
        let meta = SidecarMeta {
            key_hex: "abc123".repeat(10).chars().take(64).collect(),
            created_at_unix_ms: 1234567,
            accessed_at_unix_ms: 7654321,
            size_bytes: 4242,
            nod_version: "0.0.1".to_string(),
            abi_version: 1,
            llvm_major: 22,
            target_triple: "x86_64-pc-windows-msvc".to_string(),
            opt_level: 2,
        };
        let json = meta.to_json();
        let back = SidecarMeta::parse(&json).expect("round-trip");
        assert_eq!(meta, back);
    }
}
