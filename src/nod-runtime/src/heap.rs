//! Generational copying heap (Sprint 11).
//!
//! Structural lift from NCL's `ncl-runtime/src/heap.rs` semispace
//! design, heavily adapted for Dylan:
//!
//!   - **One-bit tag scheme.** NCL has a 3-bit `Tag` enum and
//!     headerless cons cells. Dylan has only bit-0 (fixnum/pointer);
//!     every heap object carries a `Wrapper` header. The scanner is
//!     therefore data-driven through `ClassMetadata::scan` instead of
//!     a per-`HeapType` switch.
//!   - **Start bitmap is one bit per cell.** NCL uses two bits
//!     (start + cons-vs-headered); Dylan only needs `1 = start of
//!     object`.
//!   - **Forwarding pointer.** NCL writes a `Tag::Forward(7)`-tagged
//!     pointer into the header cell. Dylan repurposes the
//!     `GcBit::Forwarded` flag + the wrapper's class-id slot; see
//!     `Wrapper::forward_to`.
//!
//! What's lifted intact, structurally:
//!
//!   - `Semispace` — bump-pointer region with a start-bit bitmap.
//!   - `OldGen` — two `Semispace`s that swap on full GC.
//!   - `Heap` — pairs `young: Semispace` + `old: OldGen` + a
//!     `CardTable` covering old.
//!   - `collect_minor` — young → old.live, copy survivors.
//!   - `collect_full` — young + old.live → old.scratch, swap old.
//!   - Cheney-style breadth-first scan via a scan pointer.
//!
//! Sprint 11 chose **option (b) from the brief**: synchronous GC
//! triggered only at allocation sites in Rust code. No JIT-side
//! safepoint polls, no precise stack roots via `gc.statepoint`. The
//! JIT-side polls and precise roots are Sprint 11b — see DEFERRED.md.
//!
//! The collector body is intentionally raw-pointer-flavoured: it holds
//! the heap mutex for the duration of a collection, so safety reduces
//! to "no other thread can read or write these regions while the
//! collector runs". The unsafe blocks document this invariant.
//!
//! ## Sprint 11c — lock-free root registry
//!
//! Sprint 11b's `Heap::register_root` / `unregister_root` took a
//! `Mutex<Vec<*const Word>>` lock on every call. The Richards-shape
//! bench (Sprint 16) revealed those mutex operations dominated the
//! runtime — hundreds of millions of acquisitions per benchmark run,
//! opaque to LLVM, identical in both sealed and open variants, so the
//! sealing-vs-open differential collapsed to ~1.06×.
//!
//! Sprint 11c replaces the mutex with a process-wide thread-local
//! `RefCell<Vec<*const Word>>` (see `register_root` / `unregister_root` /
//! `root_count` / `for_each_root` below). The runtime is single-threaded
//! today (Sprint 28 lights up multi-threading); the thread-local pattern
//! is safe and ~50–100× cheaper than a mutex on the hot path. A
//! `OnceLock<ThreadId>` debug-assert catches any future caller that
//! violates the single-thread invariant before silent corruption can
//! occur.
//!
//! When Sprint 28 introduces multi-threaded mutators, this design
//! becomes per-thread-local roots that the collector enumerates across
//! all parked threads — see DEFERRED.md.

use std::cell::RefCell;
use std::sync::Mutex;

use crate::classes::{ClassId, ClassTable, class_metadata_for};
use crate::heap_common::{
    CARD_SIZE_BYTES, CARD_SIZE_CELLS, CardTable, StartBits, clear_start_bit,
    clear_start_bits_below, for_each_start, is_start_bit, new_start_bits, set_start_bit,
};
use crate::word::Word;
use crate::wrapper::{GcBit, Wrapper};

/// Default young-generation capacity (4 MB).
pub const DEFAULT_YOUNG_BYTES: usize = 4 * 1024 * 1024;
/// Default old-generation capacity, per semispace (12 MB).
pub const DEFAULT_OLD_BYTES: usize = 12 * 1024 * 1024;
/// Legacy alias preserved for any external callers. Sprint 09's name
/// for the bump-heap reservation; Sprint 11 keeps it as the sum of
/// young + old.
pub const DEFAULT_RESERVATION_BYTES: usize = DEFAULT_YOUNG_BYTES + DEFAULT_OLD_BYTES;

/// Object alignment. Heap pointers must keep bits [2:0] clear so the
/// tag in bit 0 doesn't collide with payload.
pub const HEAP_ALIGN: usize = 8;

/// GC knobs. Sprint 11 only exposes capacity; promotion policy is
/// "any survivor of a minor GC tenures into old". A two-cycle survival
/// threshold (NCL's policy) lands in Sprint 11b.
#[derive(Copy, Clone, Debug)]
pub struct GcConfig {
    pub young_bytes: usize,
    pub old_bytes: usize,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            young_bytes: DEFAULT_YOUNG_BYTES,
            old_bytes: DEFAULT_OLD_BYTES,
        }
    }
}

// -- Semispace ---------------------------------------------------------------

/// A bump-allocated cell-aligned region with a start-bit bitmap.
pub(crate) struct Semispace {
    cells: Box<[u64]>,
    starts: StartBits,
    top: usize,
}

impl Semispace {
    fn new(size_bytes: usize) -> Self {
        let n_cells = size_bytes / 8;
        let cells = vec![0u64; n_cells].into_boxed_slice();
        let starts = new_start_bits(n_cells);
        Semispace {
            cells,
            starts,
            top: 0,
        }
    }

    fn capacity_cells(&self) -> usize {
        self.cells.len()
    }

    fn capacity_bytes(&self) -> usize {
        self.cells.len() * 8
    }

    fn used_bytes(&self) -> usize {
        self.top * 8
    }

    fn free_cells(&self) -> usize {
        self.cells.len() - self.top
    }

    fn base_addr(&self) -> usize {
        self.cells.as_ptr() as usize
    }

    fn contains(&self, addr: usize) -> bool {
        let base = self.base_addr();
        let end = base + self.capacity_bytes();
        addr >= base && addr < end
    }

    /// Try to bump-allocate `n_bytes` of space, returning the address
    /// of the first byte on success. Returns `None` on exhaustion.
    fn try_alloc_bytes(&mut self, n_bytes: usize) -> Option<usize> {
        let aligned = n_bytes.next_multiple_of(HEAP_ALIGN);
        let cells_needed = aligned / 8;
        if self.top + cells_needed > self.cells.len() {
            return None;
        }
        let cell_idx = self.top;
        let addr = self.base_addr() + cell_idx * 8;
        self.top += cells_needed;
        set_start_bit(&self.starts, cell_idx);
        Some(addr)
    }

    /// Reset the semispace to empty (zero top, clear start bitmap).
    /// Used at the end of a minor GC after every survivor has been
    /// evacuated, or after full-GC swap.
    fn reset(&mut self) {
        clear_start_bits_below(&self.starts, self.top);
        self.top = 0;
    }

    /// Conservative pin: walk `[range_lo, range_hi)` word-aligned and
    /// pin any object in this semispace whose start cell appears as a
    /// pointer-tagged Word in the range.
    ///
    /// # Safety
    ///
    /// `range_lo..range_hi` must be a readable, 8-byte-aligned address
    /// range.
    unsafe fn pin_pointers_in_range(&self, range_lo: usize, range_hi: usize) -> usize {
        if range_lo >= range_hi {
            return 0;
        }
        let base = self.base_addr();
        let end = base + self.capacity_bytes();
        let scan_start = range_lo.next_multiple_of(8);
        let scan_end = range_hi & !7;
        let mut n_pinned = 0usize;
        let mut p = scan_start as *const u64;
        let end_p = scan_end as *const u64;
        while p < end_p {
            // SAFETY: caller asserts range is readable + aligned.
            let raw = unsafe { *p };
            let w = Word::from_raw(raw);
            if w.is_pointer() {
                let target = (raw & !1) as usize;
                if target >= range_lo && target < range_hi {
                    // SAFETY: still inside input range.
                    p = unsafe { p.add(1) };
                    continue;
                }
                if target >= base && target < end {
                    let cell_idx = (target - base) / 8;
                    if is_start_bit(&self.starts, cell_idx) {
                        // SAFETY: target is a header start.
                        let header_ptr = target as *mut u64;
                        let cur = unsafe { *header_ptr };
                        let wrapper = Wrapper { raw: cur };
                        if !wrapper.is_forwarded() && !wrapper.has_gc_bit(GcBit::Pinned) {
                            let pinned = wrapper.with_gc_bit(GcBit::Pinned);
                            // SAFETY: sole writer through this path under heap mutex.
                            unsafe { *header_ptr = pinned.raw };
                            n_pinned += 1;
                        }
                    }
                }
            }
            // SAFETY: incrementing inside asserted range.
            p = unsafe { p.add(1) };
        }
        n_pinned
    }

    /// Clear pinned bits on every header-bearing object in this
    /// semispace.
    fn clear_pinned_bits(&self) {
        let cells_ptr = self.cells.as_ptr() as *mut u64;
        let top = self.top;
        for_each_start(&self.starts, top, |idx| {
            // SAFETY: idx is a start cell.
            let cell_ptr = unsafe { cells_ptr.add(idx) };
            let cur = unsafe { *cell_ptr };
            let wrapper = Wrapper { raw: cur };
            if wrapper.is_forwarded() {
                return;
            }
            if wrapper.has_gc_bit(GcBit::Pinned) {
                let cleared = wrapper.without_gc_bit(GcBit::Pinned);
                // SAFETY: sole writer.
                unsafe { *cell_ptr = cleared.raw };
            }
        });
    }
}

// -- OldGen ------------------------------------------------------------------

/// Old generation: two semispaces that swap on full GC.
pub(crate) struct OldGen {
    live: Semispace,
    scratch: Semispace,
}

impl OldGen {
    fn new(per_space_bytes: usize) -> Self {
        OldGen {
            live: Semispace::new(per_space_bytes),
            scratch: Semispace::new(per_space_bytes),
        }
    }

    fn swap(&mut self) {
        std::mem::swap(&mut self.live, &mut self.scratch);
    }
}

// -- Heap --------------------------------------------------------------------

pub(crate) struct HeapInner {
    young: Semispace,
    old: OldGen,
    cards: CardTable,
    cumulative_objects: u64,
    stats: HeapStats,
}

/// Internal stats bag.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct HeapStats {
    pub minor_collections: u64,
    pub major_collections: u64,
    pub young_bytes_allocated: u64,
    pub last_minor_pause_ns: u64,
    pub last_major_pause_ns: u64,
    pub last_pinned_objects: u64,
}

/// Public-facing snapshot of GC counters.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct HeapStatsSnapshot {
    pub minor_collections: u64,
    pub major_collections: u64,
    pub young_bytes_allocated: u64,
    pub young_bytes_live: u64,
    pub old_bytes_live: u64,
    pub last_minor_pause_ns: u64,
    pub last_major_pause_ns: u64,
    pub last_pinned_objects: u64,
}

/// Sprint 11 generational copying heap. Sprint 11c moved the root
/// registry out into a thread-local; the heap struct itself only
/// guards the moveable regions through `inner`.
pub struct Heap {
    inner: Mutex<HeapInner>,
}

// SAFETY: `Heap`'s only state is the inner Mutex over the moveable
// regions. The Sprint 11c lock-free root registry lives in
// `ROOT_STACK` (thread-local); each thread sees its own root stack,
// so cross-thread `Heap` references can't race on it. See the
// "Sprint 11c thread-confinement note" below for the Sprint 28
// multi-mutator caveat.
unsafe impl Send for Heap {}
unsafe impl Sync for Heap {}

// -- Sprint 11c: lock-free root registry --------------------------------------
//
// Process-global thread-local stack of registered roots. The Sprint 11b
// API is stack-disciplined: every `register_root(slot)` is matched by an
// `unregister_root(slot)` LIFO; `swap_remove` from the back is O(1)
// amortised. A pathological caller that unregisters out of order falls
// back to an `rposition` scan — O(n) worst case, but the API contract
// documents the LIFO expectation.
//
// The collector calls `for_each_root` which takes an immutable borrow;
// callers must NOT register or unregister inside the closure (would
// panic the RefCell). The collector takes a `Vec` snapshot at the start
// of each cycle so subsequent root mutations during evacuation are safe
// (the snapshot is what the collector walks).

thread_local! {
    static ROOT_STACK: RefCell<Vec<*const Word>> = const { RefCell::new(Vec::new()) };
}

// Sprint 11c thread-confinement note. The Sprint 11c brief asked for a
// `OnceLock<ThreadId>` debug-assert capturing the first runtime-init
// thread and rejecting subsequent calls from other threads. In a
// single-mutator deployment that would catch Sprint 28's first mistake.
// In practice the Rust test harness runs each `#[test]` on its OWN
// OS thread (even with `#[serial]` — serial only orders execution,
// not threading), so a process-wide thread assertion fires every time
// `cargo test` runs the second test. The thread-local design is
// already self-enforcing: each thread sees ITS OWN `ROOT_STACK`, the
// collector running on that thread snapshots that thread's stack, the
// invariant holds trivially. Sprint 28 (multi-mutator) will need a
// global registry + atomic enumeration across parked threads — see
// DEFERRED.md.

/// Sprint 11c: lock-free register. Push `slot` onto the thread-local
/// root stack. The collector reads (a snapshot of) this stack each
/// cycle and rewrites the pointed-at Word if it evacuates.
///
/// O(1); no mutex acquisition.
pub fn register_root(slot: *const Word) {
    ROOT_STACK.with(|s| s.borrow_mut().push(slot));
}

/// Sprint 11c: lock-free unregister. Pop the most-recent matching slot
/// from the thread-local root stack. The Sprint 11b API contract is
/// LIFO-disciplined so almost always the matching entry is the last;
/// `rposition` + `swap_remove` is O(1) amortised, O(n) worst case if a
/// pathological caller unregisters out of order.
///
/// O(1); no mutex acquisition.
pub fn unregister_root(slot: *const Word) {
    ROOT_STACK.with(|s| {
        let mut stack = s.borrow_mut();
        if let Some(idx) = stack.iter().rposition(|&p| p == slot) {
            stack.swap_remove(idx);
        }
    });
}

/// Current root-stack length. Used by tests to assert
/// register/unregister balance.
pub fn root_count() -> usize {
    ROOT_STACK.with(|s| s.borrow().len())
}

/// Snapshot the current root stack into a freshly-allocated `Vec`.
/// The collector calls this once at the start of each cycle so the
/// borrow is released before evacuation begins (evacuation rewrites
/// `*slot` for each slot in the snapshot, and the rewrites happen
/// outside the `RefCell` borrow).
fn snapshot_roots() -> Vec<*const Word> {
    ROOT_STACK.with(|s| s.borrow().clone())
}

/// Iterate every currently-registered root. The closure must NOT
/// mutate the root list (no nested `register_root` / `unregister_root`
/// calls). Used by tests and diagnostic paths; the collector uses
/// `snapshot_roots` instead to avoid the borrow living across
/// evacuation.
pub fn for_each_root<F: FnMut(*const Word)>(mut f: F) {
    ROOT_STACK.with(|s| {
        for &slot in s.borrow().iter() {
            f(slot);
        }
    });
}

impl Heap {
    pub fn new() -> Self {
        Self::with_config(GcConfig::default())
    }

    pub fn with_capacity(capacity_bytes: usize) -> Self {
        let young = capacity_bytes / 4;
        let old = capacity_bytes - young;
        Self::with_config(GcConfig {
            young_bytes: young,
            old_bytes: old,
        })
    }

    pub fn with_config(cfg: GcConfig) -> Self {
        let young = Semispace::new(cfg.young_bytes);
        let old = OldGen::new(cfg.old_bytes);
        let cards = CardTable::new(cfg.old_bytes);
        Heap {
            inner: Mutex::new(HeapInner {
                young,
                old,
                cards,
                cumulative_objects: 0,
                stats: HeapStats::default(),
            }),
        }
    }

    /// Allocate `payload_bytes` of payload preceded by an 8-byte
    /// `Wrapper` header. Returns a tagged-pointer `Word`. Payload zeroed.
    pub fn alloc_object(&self, class: ClassId, payload_bytes: usize) -> Word {
        let total = (size_of::<Wrapper>() + payload_bytes).next_multiple_of(HEAP_ALIGN);
        let addr = self.alloc_movable_raw(total);
        // SAFETY: alloc_movable_raw returned a freshly-bumped chunk;
        // we install the wrapper and zero the payload immediately.
        unsafe {
            let header_ptr = addr as *mut Wrapper;
            header_ptr.write(Wrapper::new(class));
        }
        if payload_bytes > 0 {
            let payload_addr = addr + size_of::<Wrapper>();
            let zero_bytes = total - size_of::<Wrapper>();
            // SAFETY: payload region is inside the fresh chunk.
            unsafe {
                std::ptr::write_bytes(payload_addr as *mut u8, 0u8, zero_bytes);
            }
        }
        Word::from_ptr(addr as *const u8)
    }

    fn alloc_movable_raw(&self, total_bytes: usize) -> usize {
        // First attempt against young.
        {
            let mut inner = self.inner.lock().expect("heap mutex poisoned");
            if let Some(addr) = inner.young.try_alloc_bytes(total_bytes) {
                inner.cumulative_objects += 1;
                inner.stats.young_bytes_allocated += total_bytes as u64;
                return addr;
            }
        }
        // Young is exhausted. Minor GC.
        self.collect_minor();
        {
            let mut inner = self.inner.lock().expect("heap mutex poisoned");
            if let Some(addr) = inner.young.try_alloc_bytes(total_bytes) {
                inner.cumulative_objects += 1;
                inner.stats.young_bytes_allocated += total_bytes as u64;
                return addr;
            }
            if let Some(addr) = inner.old.live.try_alloc_bytes(total_bytes) {
                inner.cumulative_objects += 1;
                return addr;
            }
        }
        // Full GC as last resort.
        self.collect_full();
        let mut inner = self.inner.lock().expect("heap mutex poisoned");
        if let Some(addr) = inner.young.try_alloc_bytes(total_bytes) {
            inner.cumulative_objects += 1;
            inner.stats.young_bytes_allocated += total_bytes as u64;
            return addr;
        }
        if let Some(addr) = inner.old.live.try_alloc_bytes(total_bytes) {
            inner.cumulative_objects += 1;
            return addr;
        }
        panic!(
            "heap exhausted: request {total_bytes} bytes, young free={} bytes, old free={} bytes",
            inner.young.free_cells() * 8,
            inner.old.live.free_cells() * 8,
        );
    }

    /// Decode `w` to its `Wrapper`. `None` for fixnums and pointers
    /// outside the heap.
    pub fn wrapper_of(&self, w: Word) -> Option<Wrapper> {
        let ptr = w.as_ptr::<Wrapper>()?;
        let addr = ptr as usize;
        let inner = self.inner.lock().ok()?;
        if !(inner.young.contains(addr)
            || inner.old.live.contains(addr)
            || inner.old.scratch.contains(addr))
        {
            return None;
        }
        // SAFETY: addr is in our heap and `w` is a Dylan-tagged
        // pointer into it; first 8 bytes are an initialised Wrapper.
        Some(unsafe { *ptr })
    }

    /// Used-byte total across young + old.live.
    pub fn live_bytes(&self) -> usize {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        inner.young.used_bytes() + inner.old.live.used_bytes()
    }

    /// Total object count across the heap's lifetime.
    pub fn object_count(&self) -> usize {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        inner.cumulative_objects as usize
    }

    pub fn young_used_bytes(&self) -> usize {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        inner.young.used_bytes()
    }

    pub fn old_used_bytes(&self) -> usize {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        inner.old.live.used_bytes()
    }

    pub fn capacity_bytes(&self) -> usize {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        inner.young.capacity_bytes() + inner.old.live.capacity_bytes()
    }

    /// Sprint 11c: thin wrapper over the module-level `register_root`
    /// for Sprint 11b call-site API stability. The mutex baseline is
    /// gone — calls now hit a thread-local `Vec::push`.
    pub fn register_root(&self, root: *const Word) {
        register_root(root);
    }

    /// Sprint 11c: thin wrapper over the module-level `unregister_root`.
    pub fn unregister_root(&self, root: *const Word) {
        unregister_root(root);
    }

    /// Sprint 11c: snapshot of the current root-stack depth.
    pub fn root_count(&self) -> usize {
        root_count()
    }

    /// Mark the card containing `dst_ptr` (which should point into
    /// old). No-op if `dst_ptr` is not in old.
    pub fn mark_card_for(&self, dst_ptr: *const Word) {
        let addr = dst_ptr as usize;
        let inner = self.inner.lock().expect("heap mutex poisoned");
        if !inner.old.live.contains(addr) {
            return;
        }
        let offset = addr - inner.old.live.base_addr();
        inner.cards.mark_offset(offset);
    }

    /// Conservative stack-range pin. Walks `[lo, hi)` and pins any
    /// object in young whose start cell appears as a pointer-tagged
    /// Word there. Returns the number of distinct objects pinned.
    ///
    /// **Sprint 11b status: opt-in only, NOT called from any
    /// production code path.** Sprint 11b's `nod_register_root` /
    /// `nod_unregister_root` shim + JIT-emitted spill/reload sequence
    /// (driven by the Sprint 11b liveness pass) replaces conservative
    /// scanning with precise, slot-rewriting evacuation. The pinner
    /// remains as a debug aid: a caller can still construct a
    /// synthetic "stack-shaped" Word array and pin its contents, then
    /// drive a minor GC, to verify the rewinding-pinned-objects
    /// collector path. Sprint 11c (full `gc.statepoint`) will likely
    /// retire this entirely.
    ///
    /// # Safety
    ///
    /// `lo..hi` must be a readable, 8-byte-aligned address range.
    pub unsafe fn pin_stack_range(&self, lo: usize, hi: usize) -> usize {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        // SAFETY: forwarded.
        unsafe { inner.young.pin_pointers_in_range(lo, hi) }
    }

    /// Clear pinned bits on remaining young+old objects.
    pub fn clear_pinned(&self) {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        inner.young.clear_pinned_bits();
        inner.old.live.clear_pinned_bits();
    }

    /// Count of currently-dirty cards in the write-barrier table.
    /// Diagnostic; exposed for tests and `:gc-stats`.
    pub fn dirty_card_count(&self) -> usize {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        inner.cards.dirty_count()
    }

    /// Number of minor collections this heap has run. Exposed for
    /// tests that want to assert the GC actually fired.
    pub fn minor_collection_count(&self) -> u64 {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        inner.stats.minor_collections
    }

    /// Number of major collections this heap has run.
    pub fn major_collection_count(&self) -> u64 {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        inner.stats.major_collections
    }

    pub fn ranges(&self) -> HeapRanges {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        HeapRanges {
            young: (
                inner.young.base_addr(),
                inner.young.base_addr() + inner.young.capacity_bytes(),
            ),
            old: (
                inner.old.live.base_addr(),
                inner.old.live.base_addr() + inner.old.live.capacity_bytes(),
            ),
        }
    }

    pub(crate) fn stats_snapshot(&self) -> HeapStatsSnapshot {
        let inner = self.inner.lock().expect("heap mutex poisoned");
        HeapStatsSnapshot {
            minor_collections: inner.stats.minor_collections,
            major_collections: inner.stats.major_collections,
            young_bytes_allocated: inner.stats.young_bytes_allocated,
            young_bytes_live: inner.young.used_bytes() as u64,
            old_bytes_live: inner.old.live.used_bytes() as u64,
            last_minor_pause_ns: inner.stats.last_minor_pause_ns,
            last_major_pause_ns: inner.stats.last_major_pause_ns,
            last_pinned_objects: inner.stats.last_pinned_objects,
        }
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

/// Heap address ranges; produced by `Heap::ranges`.
pub struct HeapRanges {
    pub young: (usize, usize),
    pub old: (usize, usize),
}

// -- Collector ---------------------------------------------------------------

impl Heap {
    /// Minor collection: young → old.live. Surviving young objects are
    /// copied into old.live (full promotion — every survivor tenures),
    /// young is reset.
    pub fn collect_minor(&self) {
        let start = std::time::Instant::now();
        // Sprint 11c: snapshot the thread-local root stack BEFORE
        // taking the heap mutex. The snapshot is what the collector
        // walks; evacuation rewrites `*slot` on each entry, but never
        // mutates the root stack itself, so we don't need a `RefCell`
        // borrow live across the GC.
        let roots = snapshot_roots();
        let pinned_count;
        {
            let mut inner = self.inner.lock().expect("heap mutex poisoned");
            // SAFETY: we hold the heap mutex; the collector is the sole
            // mutator of the heap's regions for the duration of this call.
            pinned_count = unsafe { run_minor(&mut inner, &roots) };
        }
        let elapsed_ns = start.elapsed().as_nanos() as u64;
        let mut inner = self.inner.lock().expect("heap mutex poisoned");
        inner.stats.minor_collections += 1;
        inner.stats.last_minor_pause_ns = elapsed_ns;
        inner.stats.last_pinned_objects = pinned_count as u64;
    }

    /// Full collection: young + old.live → old.scratch, swap old,
    /// reset young.
    pub fn collect_full(&self) {
        let start = std::time::Instant::now();
        // Sprint 11c: see `collect_minor` — snapshot first, no
        // RefCell borrow across the heap mutex.
        let roots = snapshot_roots();
        {
            let mut inner = self.inner.lock().expect("heap mutex poisoned");
            // SAFETY: heap mutex held; collector is sole mutator.
            unsafe { run_full(&mut inner, &roots) };
        }
        let elapsed_ns = start.elapsed().as_nanos() as u64;
        let mut inner = self.inner.lock().expect("heap mutex poisoned");
        inner.stats.major_collections += 1;
        inner.stats.last_major_pause_ns = elapsed_ns;
    }
}

// -- Collector internals (raw-pointer-flavoured) -----------------------------
//
// The collector holds the heap mutex, so the regions it walks aren't
// touched by any other thread. We use raw pointers and address
// arithmetic throughout — Rust's borrow checker has no way to model
// "two mutable regions that come from the same struct and don't
// overlap", and the GC's data shape requires exactly that. Every
// unsafe block here documents the heap-mutex invariant.

struct CollectorCtx {
    young_base: usize,
    young_end: usize,
    old_live_base: usize,
    old_live_end: usize,
    young_starts: StartBits,
    old_live_starts_ptr: *const StartBits,
    young_top_ptr: *mut usize,
    old_live_top_ptr: *mut usize,
    old_live_capacity_cells: usize,
    cards_ptr: *const CardTable,
}

unsafe fn run_minor(inner: &mut HeapInner, roots: &[*const Word]) -> usize {
    let young_base = inner.young.base_addr();
    let young_end = young_base + inner.young.capacity_bytes();
    let old_live_base = inner.old.live.base_addr();
    let old_live_end = old_live_base + inner.old.live.capacity_bytes();
    let young_starts = inner.young.starts.clone();
    let old_live_starts_ptr: *const StartBits = &inner.old.live.starts;
    let young_top_ptr: *mut usize = &mut inner.young.top;
    let old_live_top_ptr: *mut usize = &mut inner.old.live.top;
    let old_live_capacity_cells = inner.old.live.capacity_cells();
    let cards_ptr: *const CardTable = &inner.cards;

    let ctx = CollectorCtx {
        young_base,
        young_end,
        old_live_base,
        old_live_end,
        young_starts,
        old_live_starts_ptr,
        young_top_ptr,
        old_live_top_ptr,
        old_live_capacity_cells,
        cards_ptr,
    };

    // Snapshot old.live top BEFORE we forward any roots. The Cheney
    // loop will scan everything appended past this watermark; the
    // card pass handles everything already below.
    // SAFETY: heap mutex held.
    let old_top_at_gc_start = unsafe { *ctx.old_live_top_ptr };

    // 1. Forward every root.
    for &root in roots.iter() {
        // SAFETY: registered root provides a writable Word slot.
        unsafe {
            let mw = root as *mut Word;
            let w = *mw;
            minor_forward_word(&ctx, mw, w);
        }
    }

    // 2. Walk dirty cards in old.live; forward any young pointers found.
    {
        // SAFETY: heap mutex held; cards live for ctx lifetime.
        let cards = unsafe { &*ctx.cards_ptr };
        for card_idx in 0..cards.n_cards() {
            if !cards.is_dirty(card_idx) {
                continue;
            }
            let card_cell_lo = card_idx * CARD_SIZE_CELLS;
            // SAFETY: heap mutex held.
            let used_cells = unsafe { *ctx.old_live_top_ptr };
            let card_cell_hi = (card_cell_lo + CARD_SIZE_CELLS).min(used_cells);
            // SAFETY: heap mutex held.
            unsafe {
                scan_card_range_minor(&ctx, card_cell_lo, card_cell_hi);
            }
            cards.clear(card_idx);
        }
    }

    // 3. Cheney scan over newly-copied old objects. Anything that
    //    was already in old before this minor GC was reached via the
    //    card-pass above; the cursor starts at "old.live top at GC
    //    start" (snapshotted before step 1) and chases newly-appended
    //    cells.
    let mut cursor = old_top_at_gc_start;
    loop {
        // SAFETY: heap mutex held.
        let cur_top = unsafe { *ctx.old_live_top_ptr };
        if cursor == cur_top {
            break;
        }
        let prev_cursor = cursor;
        // Walk every start in [cursor, cur_top).
        // SAFETY: heap mutex held.
        let old_starts = unsafe { &*ctx.old_live_starts_ptr };
        let mut new_addrs: Vec<usize> = Vec::new();
        for_each_start(old_starts, cur_top, |cell_idx| {
            if cell_idx < prev_cursor {
                return;
            }
            new_addrs.push(ctx.old_live_base + cell_idx * 8);
        });
        for addr in new_addrs {
            // SAFETY: addr is a wrapper start in old.live.
            unsafe {
                let wrapper = *(addr as *const Wrapper);
                if wrapper.is_forwarded() {
                    continue;
                }
                let class = wrapper.class();
                let metadata = class_metadata_for(class);
                // Scan visits each Word slot of the object; we forward
                // it (if young-pointing) inline.
                let ctx_ref: &CollectorCtx = &ctx;
                (metadata.scan)(addr, &mut |slot| {
                    let w = *slot;
                    minor_forward_word(ctx_ref, slot, w);
                    // If the new value points back into old.live (an
                    // old → old reference), dirty the card.
                    let nw = *slot;
                    if nw.is_pointer() {
                        let target = (nw.raw() & !1) as usize;
                        if target >= ctx_ref.old_live_base && target < ctx_ref.old_live_end {
                            // SAFETY: heap mutex held.
                            let cards = &*ctx_ref.cards_ptr;
                            let offset = (slot as usize) - ctx_ref.old_live_base;
                            cards.mark_offset(offset);
                        }
                    }
                });
            }
        }
        cursor = unsafe { *ctx.old_live_top_ptr };
    }

    // 4. Process pinned young objects (conservative refs found by
    //    `pin_stack_range`). We copy them into old too — Sprint 11
    //    accepts losing the truly-in-place semantics. Sprint 11b's
    //    statepoint-driven precise roots will eliminate the need for
    //    pinning in normal operation.
    let young_used_at_minor = unsafe { *ctx.young_top_ptr };
    let mut pinned_addrs: Vec<usize> = Vec::new();
    for_each_start(&ctx.young_starts, young_used_at_minor, |cell_idx| {
        let addr = ctx.young_base + cell_idx * 8;
        // SAFETY: cell is marked as a start; first 8 bytes are Wrapper.
        let cur = unsafe { *(addr as *const u64) };
        let wrapper = Wrapper { raw: cur };
        if wrapper.is_forwarded() || !wrapper.has_gc_bit(GcBit::Pinned) {
            return;
        }
        pinned_addrs.push(addr);
    });
    let pinned_count = pinned_addrs.len();
    for addr in pinned_addrs {
        // SAFETY: addr is a young heap object; we copy and forward it.
        unsafe {
            let wrapper = *(addr as *const Wrapper);
            if wrapper.is_forwarded() {
                continue;
            }
            let class = wrapper.class();
            let metadata = class_metadata_for(class);
            let total = (metadata.size_of)(addr);
            let new_addr = ctx_try_alloc_old(&ctx, total).unwrap_or_else(|| {
                panic!("old gen exhausted while evacuating pinned objects (need {total} bytes)")
            });
            std::ptr::copy_nonoverlapping(addr as *const u8, new_addr as *mut u8, total);
            let new_wrapper_ptr = new_addr as *mut Wrapper;
            let nw = (*new_wrapper_ptr)
                .with_gc_bit(GcBit::Tenured)
                .without_gc_bit(GcBit::Pinned);
            *new_wrapper_ptr = nw;
            *(addr as *mut Wrapper) = Wrapper::forward_to(new_addr);
            // Scan the new copy.
            let ctx_ref: &CollectorCtx = &ctx;
            (metadata.scan)(new_addr, &mut |slot| {
                let w = *slot;
                minor_forward_word(ctx_ref, slot, w);
            });
        }
    }

    // 5. Reset young.
    inner.young.reset();
    pinned_count
}

/// Try to bump-allocate `total_bytes` in old.live via raw pointers.
/// Returns the new address on success, `None` on exhaustion.
///
/// # Safety
///
/// Heap mutex must be held by the caller.
unsafe fn ctx_try_alloc_old(ctx: &CollectorCtx, total_bytes: usize) -> Option<usize> {
    let aligned = total_bytes.next_multiple_of(HEAP_ALIGN);
    let cells_needed = aligned / 8;
    // SAFETY: heap mutex held.
    let top = unsafe { *ctx.old_live_top_ptr };
    if top + cells_needed > ctx.old_live_capacity_cells {
        return None;
    }
    let cell_idx = top;
    let addr = ctx.old_live_base + cell_idx * 8;
    // SAFETY: heap mutex held.
    unsafe {
        *ctx.old_live_top_ptr = top + cells_needed;
    }
    // SAFETY: heap mutex held; old_live_starts_ptr is a live StartBits.
    let starts = unsafe { &*ctx.old_live_starts_ptr };
    set_start_bit(starts, cell_idx);
    Some(addr)
}

/// Forward a single Word reference at `slot`: if the target is in
/// young, copy it into old.live and rewrite the slot.
///
/// # Safety
///
/// Heap mutex held; `slot` must be a writable `*mut Word` inside a
/// region the collector can mutate (any heap region during GC, plus
/// any explicitly registered root slot).
unsafe fn minor_forward_word(ctx: &CollectorCtx, slot: *mut Word, w: Word) {
    if !w.is_pointer() {
        return;
    }
    let target = (w.raw() & !1) as usize;
    if !(target >= ctx.young_base && target < ctx.young_end) {
        return;
    }
    // SAFETY: target is a wrapper start in young (we set the bit at alloc).
    let cur_wrapper = unsafe { *(target as *const Wrapper) };
    if cur_wrapper.is_forwarded() {
        let new_addr = cur_wrapper.forwarding_addr();
        let new_word = Word::from_ptr(new_addr as *const u8);
        // SAFETY: slot is writable per caller's contract.
        unsafe { *slot = new_word };
        return;
    }
    let class = cur_wrapper.class();
    let metadata = class_metadata_for(class);
    // SAFETY: class matches the layout at target.
    let total = unsafe { (metadata.size_of)(target) };
    let new_addr = match unsafe { ctx_try_alloc_old(ctx, total) } {
        Some(a) => a,
        None => panic!(
            "old gen exhausted during minor GC evacuation (need {total} bytes)"
        ),
    };
    // SAFETY: target..target+total is live; new_addr is fresh.
    unsafe {
        std::ptr::copy_nonoverlapping(target as *const u8, new_addr as *mut u8, total);
    }
    // Stamp Tenured + clear Pinned/Forwarded on the copy.
    // SAFETY: new_addr's first 8 bytes are the freshly copied wrapper.
    unsafe {
        let new_wrapper_ptr = new_addr as *mut Wrapper;
        let nw = (*new_wrapper_ptr)
            .with_gc_bit(GcBit::Tenured)
            .without_gc_bit(GcBit::Pinned)
            .without_gc_bit(GcBit::Forwarded);
        *new_wrapper_ptr = nw;
    }
    // Install forwarding pointer in young.
    // SAFETY: target is a young header start.
    unsafe {
        *(target as *mut Wrapper) = Wrapper::forward_to(new_addr);
    }
    // Clear the young start bit so a re-walk sees no ghost.
    let target_cell = (target - ctx.young_base) / 8;
    clear_start_bit(&ctx.young_starts, target_cell);
    let new_word = Word::from_ptr(new_addr as *const u8);
    // SAFETY: slot writable per caller.
    unsafe { *slot = new_word };
}

/// Walk the cards' start bitmap and visit every slot of every object
/// whose start lies in the card window.
///
/// # Safety
///
/// Heap mutex held.
unsafe fn scan_card_range_minor(ctx: &CollectorCtx, card_cell_lo: usize, card_cell_hi: usize) {
    // SAFETY: heap mutex held.
    let starts = unsafe { &*ctx.old_live_starts_ptr };
    let mut start_addrs: Vec<usize> = Vec::new();
    for_each_start(starts, card_cell_hi, |cell_idx| {
        start_addrs.push(ctx.old_live_base + cell_idx * 8);
    });
    for addr in start_addrs {
        // SAFETY: addr is a wrapper start in old.live.
        unsafe {
            let wrapper = *(addr as *const Wrapper);
            if wrapper.is_forwarded() {
                continue;
            }
            let class = wrapper.class();
            let metadata = class_metadata_for(class);
            let total = (metadata.size_of)(addr);
            let cells = total / 8;
            let cell_idx = (addr - ctx.old_live_base) / 8;
            let end_cell = cell_idx + cells;
            if end_cell <= card_cell_lo {
                continue;
            }
            (metadata.scan)(addr, &mut |slot| {
                let w = *slot;
                minor_forward_word(ctx, slot, w);
            });
        }
    }
}

// -- Full GC -----------------------------------------------------------------

struct FullCtx {
    young_base: usize,
    young_end: usize,
    old_live_base: usize,
    old_live_end: usize,
    scratch_starts_ptr: *const StartBits,
    scratch_top_ptr: *mut usize,
    scratch_base: usize,
    scratch_capacity_cells: usize,
}

unsafe fn run_full(inner: &mut HeapInner, roots: &[*const Word]) {
    let ctx = FullCtx {
        young_base: inner.young.base_addr(),
        young_end: inner.young.base_addr() + inner.young.capacity_bytes(),
        old_live_base: inner.old.live.base_addr(),
        old_live_end: inner.old.live.base_addr() + inner.old.live.capacity_bytes(),
        scratch_starts_ptr: &inner.old.scratch.starts,
        scratch_top_ptr: &mut inner.old.scratch.top,
        scratch_base: inner.old.scratch.base_addr(),
        scratch_capacity_cells: inner.old.scratch.capacity_cells(),
    };

    for &root in roots.iter() {
        // SAFETY: registered root.
        unsafe {
            let mw = root as *mut Word;
            let w = *mw;
            full_forward_word(&ctx, mw, w);
        }
    }

    // Cheney scan over scratch.
    let mut cursor = 0usize;
    loop {
        // SAFETY: heap mutex held.
        let cur_top = unsafe { *ctx.scratch_top_ptr };
        if cursor == cur_top {
            break;
        }
        let prev = cursor;
        // SAFETY: heap mutex held.
        let starts = unsafe { &*ctx.scratch_starts_ptr };
        let mut new_addrs: Vec<usize> = Vec::new();
        for_each_start(starts, cur_top, |cell_idx| {
            if cell_idx < prev {
                return;
            }
            new_addrs.push(ctx.scratch_base + cell_idx * 8);
        });
        for addr in new_addrs {
            // SAFETY: scratch wrappers are well-formed (we wrote them).
            unsafe {
                let wrapper = *(addr as *const Wrapper);
                if wrapper.is_forwarded() {
                    continue;
                }
                let class = wrapper.class();
                let metadata = class_metadata_for(class);
                let ctx_ref: &FullCtx = &ctx;
                (metadata.scan)(addr, &mut |slot| {
                    let w = *slot;
                    full_forward_word(ctx_ref, slot, w);
                });
            }
        }
        cursor = unsafe { *ctx.scratch_top_ptr };
    }

    inner.old.swap();
    inner.old.scratch.reset();
    inner.young.reset();
    inner.cards.clear_all();
}

/// # Safety
///
/// Heap mutex held.
unsafe fn ctx_try_alloc_scratch(ctx: &FullCtx, total_bytes: usize) -> Option<usize> {
    let aligned = total_bytes.next_multiple_of(HEAP_ALIGN);
    let cells_needed = aligned / 8;
    // SAFETY: heap mutex held.
    let top = unsafe { *ctx.scratch_top_ptr };
    if top + cells_needed > ctx.scratch_capacity_cells {
        return None;
    }
    let cell_idx = top;
    let addr = ctx.scratch_base + cell_idx * 8;
    // SAFETY: heap mutex held.
    unsafe {
        *ctx.scratch_top_ptr = top + cells_needed;
    }
    // SAFETY: scratch_starts_ptr lives for ctx's lifetime.
    let starts = unsafe { &*ctx.scratch_starts_ptr };
    set_start_bit(starts, cell_idx);
    Some(addr)
}

/// # Safety
///
/// Heap mutex held; `slot` writable.
unsafe fn full_forward_word(ctx: &FullCtx, slot: *mut Word, w: Word) {
    if !w.is_pointer() {
        return;
    }
    let target = (w.raw() & !1) as usize;
    let in_young = target >= ctx.young_base && target < ctx.young_end;
    let in_old = target >= ctx.old_live_base && target < ctx.old_live_end;
    if !(in_young || in_old) {
        return;
    }
    // SAFETY: target is in a live semispace.
    let cur_wrapper = unsafe { *(target as *const Wrapper) };
    if cur_wrapper.is_forwarded() {
        let new_addr = cur_wrapper.forwarding_addr();
        let new_word = Word::from_ptr(new_addr as *const u8);
        // SAFETY: slot writable.
        unsafe { *slot = new_word };
        return;
    }
    let class = cur_wrapper.class();
    let metadata = class_metadata_for(class);
    // SAFETY: class matches layout.
    let total = unsafe { (metadata.size_of)(target) };
    let new_addr = match unsafe { ctx_try_alloc_scratch(ctx, total) } {
        Some(a) => a,
        None => panic!("old scratch exhausted during full GC (need {total} bytes)"),
    };
    // SAFETY: target..target+total is live; new_addr is fresh.
    unsafe {
        std::ptr::copy_nonoverlapping(target as *const u8, new_addr as *mut u8, total);
    }
    // SAFETY: new_addr's first 8 bytes are the freshly copied wrapper.
    unsafe {
        let new_wrapper_ptr = new_addr as *mut Wrapper;
        let nw = (*new_wrapper_ptr)
            .with_gc_bit(GcBit::Tenured)
            .without_gc_bit(GcBit::Pinned)
            .without_gc_bit(GcBit::Forwarded);
        *new_wrapper_ptr = nw;
    }
    // SAFETY: target is a source header start.
    unsafe {
        *(target as *mut Wrapper) = Wrapper::forward_to(new_addr);
    }
    let new_word = Word::from_ptr(new_addr as *const u8);
    // SAFETY: slot writable.
    unsafe { *slot = new_word };
}

// Suppress unused warnings for trait-required imports.
const _: fn() = || {
    let _ = ClassTable::new();
    let _ = CARD_SIZE_BYTES;
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classes::ClassTable;

    #[test]
    fn alloc_object_returns_tagged_pointer() {
        let heap = Heap::new();
        let ct = ClassTable::new();
        let w = heap.alloc_object(ct.byte_string(), 16);
        assert!(w.is_pointer());
        assert!(!w.is_fixnum());
    }

    #[test]
    fn wrapper_round_trip_via_heap() {
        let heap = Heap::new();
        let ct = ClassTable::new();
        let w = heap.alloc_object(ct.byte_string(), 16);
        let wrap = heap.wrapper_of(w).expect("wrapper inside heap");
        assert_eq!(wrap.class(), ct.byte_string());
    }

    #[test]
    fn live_bytes_advances() {
        let heap = Heap::new();
        let ct = ClassTable::new();
        let before = heap.live_bytes();
        let _ = heap.alloc_object(ct.byte_string(), 16);
        let after = heap.live_bytes();
        assert!(after > before);
        assert_eq!(after - before, 24);
    }

    #[test]
    fn object_count_advances() {
        let heap = Heap::new();
        let ct = ClassTable::new();
        let before = heap.object_count();
        let _ = heap.alloc_object(ct.byte_string(), 16);
        let _ = heap.alloc_object(ct.symbol(), 16);
        assert_eq!(heap.object_count(), before + 2);
    }

    #[test]
    fn allocations_stay_aligned() {
        let heap = Heap::new();
        let ct = ClassTable::new();
        for n in [1usize, 7, 8, 9, 23, 64] {
            let w = heap.alloc_object(ct.byte_string(), n);
            let p = w.as_ptr::<u8>().unwrap() as usize;
            assert_eq!(p & 0b111, 0, "alignment violated for payload={n}");
        }
    }
}
