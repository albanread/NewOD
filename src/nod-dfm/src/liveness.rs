//! Sprint 11b — across-call liveness for precise GC roots.
//!
//! The precise-roots story: at every potentially-allocating call site
//! (`Computation::DirectCall`, `Call`, `Dispatch`), the codegen layer
//! brackets the call with `nod_register_root` / `nod_unregister_root`
//! pairs around an alloca slot holding each live pointer-shaped temp.
//! The GC walks the registered slots, evacuates any reachable young
//! objects, and rewrites the slots; codegen reloads from the slot
//! after the call so downstream uses see the relocated address.
//!
//! This module computes the "which temps need protecting at which
//! call?" set. The algorithm is the simple per-block one described in
//! the Sprint 11b brief:
//!
//!   for each block:
//!     compute def_index(t) = position of t's defining Computation in
//!         the block's `computations` list (or `-1` if t is a function
//!         param or block param — defined before any computation).
//!     compute last_use_index(t) = the *maximum* index at which `t`
//!         appears as an operand in this block, OR `len` if `t`
//!         appears in the block's terminator, OR `len + 1` if `t` is
//!         live-out of this block via a successor's block-param
//!         (approximation: any temp passed in `Terminator::Jump.args`
//!         or referenced in `Terminator::Return.value` /
//!         `Terminator::If.cond`, plus temps defined in this block
//!         that are used in any other block).
//!
//!   for each call at index `c` in the block:
//!     live_across(c) = { t : def_index(t) < c ≤ last_use_index(t)
//!                          and t.type.needs_gc_protection() }
//!
//! Multi-block "is t defined in block A and used in block B" is the
//! conservative bit: we treat any temp defined in block A and
//! mentioned anywhere in another block as live to the end of A. This
//! over-protects when the actual control flow doesn't reach the
//! allocating block, but soundness wins.
//!
//! Function parameters are conceptually live from "before any
//! computation in the entry block" — `def_index` for them is `-1`. A
//! parameter that's a pointer-shaped type, used in any computation
//! after a call, must be protected.
//!
//! The output is written back into each call's `safepoint_roots`
//! field. The list is sorted for deterministic codegen output and
//! deterministic test snapshots.

use std::collections::{HashMap, HashSet};

use crate::ir::{Block, Computation, Function, TempId, Terminator};

/// Run the per-block live-across-call analysis and populate
/// `safepoint_roots` on every call-shaped Computation in `f`.
///
/// Idempotent: calling twice produces the same result. Tests rely on
/// this.
pub fn populate_safepoint_roots(f: &mut Function) {
    let temp_types: HashMap<TempId, crate::ir::TypeEstimate> =
        f.temps.iter().map(|t| (t.id, t.type_estimate)).collect();
    let param_set: HashSet<TempId> = f.params.iter().copied().collect();

    // 1. For each temp, compute "is it used outside its defining
    //    block?". A temp defined in block B and used in block B' != B
    //    is considered live-out of B for the entire tail of B (after
    //    its definition).
    let escapes_block = compute_escaping_temps(f);

    // 2. For each block, run the local analysis and rewrite calls.
    let blocks_len = f.blocks.len();
    for block_idx in 0..blocks_len {
        let computations_len = f.blocks[block_idx].computations.len();

        // Pass 1: collect def_index for each temp DEFINED in this block.
        // Block params get def_index = -1 ("defined before any computation").
        // Function params also get def_index = -1 in EVERY block, not only
        // the entry block. The DFM lowering sometimes references function-
        // param TempIds directly from non-entry blocks (e.g. join blocks
        // produced by `lower_if`) without threading them through block args
        // because the param was never reassigned in either arm. Restricting
        // function-param tracking to the entry block meant those params were
        // invisible to the safepoint-roots computation in the join block,
        // so GC evacuation would update the slot but the JIT's live register
        // copy went stale — leading to %byte-string-copy! receiving a
        // pointer to a reclaimed page.
        let mut def_index: HashMap<TempId, isize> = HashMap::new();
        for bp in &f.blocks[block_idx].params {
            def_index.insert(*bp, -1);
        }
        for p in &param_set {
            def_index.insert(*p, -1);
        }
        for (i, c) in f.blocks[block_idx].computations.iter().enumerate() {
            def_index.insert(c.dst(), i as isize);
        }

        // Pass 2: collect last_use_index for each temp used in this
        // block. `len()` means "used in the terminator"; `len() + 1`
        // means "used in a successor block" (i.e. live-out — approx by
        // `escapes_block`).
        let term_uses = terminator_uses(&f.blocks[block_idx].terminator);
        let mut last_use_index: HashMap<TempId, isize> = HashMap::new();
        for (i, c) in f.blocks[block_idx].computations.iter().enumerate() {
            for op in computation_operands(c) {
                let entry = last_use_index.entry(op).or_insert(-1);
                if (i as isize) > *entry {
                    *entry = i as isize;
                }
            }
        }
        let term_idx = computations_len as isize;
        for op in &term_uses {
            let entry = last_use_index.entry(*op).or_insert(-1);
            if term_idx > *entry {
                *entry = term_idx;
            }
        }
        // Live-out via escapes_block: any temp defined here that's
        // used in another block is considered live for the rest of
        // THIS block (last_use_index >= every call index in this
        // block).
        let block_live_out_idx = (computations_len + 1) as isize;
        for (t, &di) in &def_index {
            if di >= 0 && escapes_block.contains(t) {
                let entry = last_use_index.entry(*t).or_insert(-1);
                if block_live_out_idx > *entry {
                    *entry = block_live_out_idx;
                }
            }
        }
        // Function parameters live-out by escapes_block: a param used in
        // another block is live to the end of every block in which it
        // appears (it may need to stay alive across a call to reach the
        // successor block that consumes it). Apply to all blocks now that
        // function params are tracked in every block's def_index.
        for &p in &param_set {
            if escapes_block.contains(&p) {
                let entry = last_use_index.entry(p).or_insert(-1);
                if block_live_out_idx > *entry {
                    *entry = block_live_out_idx;
                }
            }
        }
        // Block params propagate the same way.
        for bp in &f.blocks[block_idx].params {
            if escapes_block.contains(bp) {
                let entry = last_use_index.entry(*bp).or_insert(-1);
                if block_live_out_idx > *entry {
                    *entry = block_live_out_idx;
                }
            }
        }

        // Pass 3: for each call at index c, compute live_across(c) and
        // write it into the call's `safepoint_roots`.
        for c_idx in 0..computations_len {
            if !f.blocks[block_idx].computations[c_idx].is_potentially_allocating_call() {
                continue;
            }
            let mut live: Vec<TempId> = Vec::new();
            // Walk every temp known to this block-context.
            for (&t, &di) in &def_index {
                if di >= c_idx as isize {
                    continue; // Defined at or after the call — not yet live.
                }
                let Some(&lu) = last_use_index.get(&t) else {
                    continue; // Never used in this block — dead.
                };
                if lu <= c_idx as isize {
                    continue; // Last use is at or before the call — already dead.
                }
                // Exclude the call's own dst from protection — its
                // value is produced BY the call, not flowing into it.
                if t == f.blocks[block_idx].computations[c_idx].dst() {
                    continue;
                }
                // Exclude operands of the call itself from protection
                // (their bit-patterns are arguments to the call, the
                // call sees them in registers; what we protect is
                // values that survive PAST the call, not "operand
                // copies"). However, if a temp is BOTH a call operand
                // AND used later in the block, the "later use" needs
                // the temp protected — the existing rule (last_use >
                // c) handles this correctly.
                let ty = match temp_types.get(&t) {
                    Some(t) => *t,
                    None => continue,
                };
                if !ty.needs_gc_protection() {
                    continue;
                }
                live.push(t);
            }
            live.sort_by_key(|t| t.0);
            live.dedup();
            if let Some(roots) = f.blocks[block_idx].computations[c_idx].safepoint_roots_mut() {
                *roots = live;
            }
        }
    }
}

/// Compute the set of temps defined in some block A and used in some
/// other block B != A (i.e. "escapes A"). The mapping is
/// (defining_block → set of escapees); we only need the union, since
/// the per-block driver checks "does THIS temp escape from THIS block".
fn compute_escaping_temps(f: &Function) -> HashSet<TempId> {
    // Where each temp is defined (block id).
    let mut def_block: HashMap<TempId, crate::ir::BlockId> = HashMap::new();
    for &p in &f.params {
        def_block.insert(p, f.entry);
    }
    for b in &f.blocks {
        for bp in &b.params {
            def_block.insert(*bp, b.id);
        }
        for c in &b.computations {
            def_block.insert(c.dst(), b.id);
        }
    }
    // Walk all uses; record any temp used in a block other than its
    // defining block.
    let mut escapes: HashSet<TempId> = HashSet::new();
    for b in &f.blocks {
        for c in &b.computations {
            for op in computation_operands(c) {
                if let Some(&db) = def_block.get(&op)
                    && db != b.id
                {
                    escapes.insert(op);
                }
            }
        }
        for op in terminator_uses(&b.terminator) {
            if let Some(&db) = def_block.get(&op)
                && db != b.id
            {
                escapes.insert(op);
            }
        }
    }
    escapes
}

fn computation_operands(c: &Computation) -> Vec<TempId> {
    match c {
        Computation::Const { .. } => Vec::new(),
        Computation::PrimOp { args, .. } => args.clone(),
        Computation::DirectCall { args, .. } => args.clone(),
        Computation::Call { callee, args, .. } => {
            let mut v = Vec::with_capacity(args.len() + 1);
            v.push(*callee);
            v.extend_from_slice(args);
            v
        }
        Computation::TypeCheck { value, .. } => vec![*value],
        Computation::WriteBarrier { slot, value, .. } => vec![*slot, *value],
        Computation::LoadSlot { instance, .. } => vec![*instance],
        Computation::StoreSlot { instance, value, .. } => vec![*instance, *value],
        Computation::Dispatch { args, .. } => args.clone(),
        Computation::SealedDirectCall { args, .. } => args.clone(),
    }
}

fn terminator_uses(t: &Terminator) -> Vec<TempId> {
    match t {
        Terminator::Return { value: Some(v) } => vec![*v],
        Terminator::Return { value: None } => Vec::new(),
        Terminator::If { cond, .. } => vec![*cond],
        Terminator::Jump { args, .. } => args.clone(),
    }
}

/// Validate that every call-shaped Computation's `safepoint_roots` is
/// a subset of the temps live across that call. Used by the Sprint
/// 11b verifier extension to catch programming errors in liveness
/// passes; if you populate `safepoint_roots` with a temp that isn't
/// actually live, the runtime registers a slot whose value the
/// post-call reload trashes the original temp with — silent
/// miscompilation.
pub fn verify_safepoint_roots(f: &Function) -> Result<(), Vec<SafepointError>> {
    let mut errs = Vec::new();
    let escapes_block = compute_escaping_temps(f);
    let temp_types: HashMap<TempId, crate::ir::TypeEstimate> =
        f.temps.iter().map(|t| (t.id, t.type_estimate)).collect();
    let param_set: HashSet<TempId> = f.params.iter().copied().collect();

    for block in &f.blocks {
        let block_live_data = block_live_intervals(f, block, &param_set, &escapes_block);
        for (c_idx, c) in block.computations.iter().enumerate() {
            let Some(roots) = c.safepoint_roots() else {
                continue;
            };
            for r in roots {
                let di = block_live_data.def_index.get(r).copied().unwrap_or(isize::MAX);
                let lu = block_live_data.last_use.get(r).copied().unwrap_or(-1);
                if !(di < c_idx as isize && lu > c_idx as isize) {
                    errs.push(SafepointError::TempNotLiveAcrossCall {
                        call_dst: c.dst(),
                        temp: *r,
                    });
                }
                let ty = temp_types.get(r).copied().unwrap_or(crate::ir::TypeEstimate::Top);
                if !ty.needs_gc_protection() {
                    errs.push(SafepointError::TempDoesNotNeedProtection {
                        call_dst: c.dst(),
                        temp: *r,
                    });
                }
            }
        }
    }
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

struct BlockLive {
    def_index: HashMap<TempId, isize>,
    last_use: HashMap<TempId, isize>,
}

fn block_live_intervals(
    f: &Function,
    block: &Block,
    param_set: &HashSet<TempId>,
    escapes_block: &HashSet<TempId>,
) -> BlockLive {
    let computations_len = block.computations.len();
    let mut def_index: HashMap<TempId, isize> = HashMap::new();
    for bp in &block.params {
        def_index.insert(*bp, -1);
    }
    if block.id == f.entry {
        for &p in param_set {
            def_index.insert(p, -1);
        }
    }
    for (i, c) in block.computations.iter().enumerate() {
        def_index.insert(c.dst(), i as isize);
    }
    let term_uses = terminator_uses(&block.terminator);
    let mut last_use: HashMap<TempId, isize> = HashMap::new();
    for (i, c) in block.computations.iter().enumerate() {
        for op in computation_operands(c) {
            let entry = last_use.entry(op).or_insert(-1);
            if (i as isize) > *entry {
                *entry = i as isize;
            }
        }
    }
    let term_idx = computations_len as isize;
    for op in &term_uses {
        let entry = last_use.entry(*op).or_insert(-1);
        if term_idx > *entry {
            *entry = term_idx;
        }
    }
    let block_live_out_idx = (computations_len + 1) as isize;
    for (t, &di) in &def_index {
        if di >= 0 && escapes_block.contains(t) {
            let entry = last_use.entry(*t).or_insert(-1);
            if block_live_out_idx > *entry {
                *entry = block_live_out_idx;
            }
        }
    }
    if block.id == f.entry {
        for &p in param_set {
            if escapes_block.contains(&p) {
                let entry = last_use.entry(p).or_insert(-1);
                if block_live_out_idx > *entry {
                    *entry = block_live_out_idx;
                }
            }
        }
    }
    for bp in &block.params {
        if escapes_block.contains(bp) {
            let entry = last_use.entry(*bp).or_insert(-1);
            if block_live_out_idx > *entry {
                *entry = block_live_out_idx;
            }
        }
    }
    BlockLive { def_index, last_use }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SafepointError {
    TempNotLiveAcrossCall { call_dst: TempId, temp: TempId },
    TempDoesNotNeedProtection { call_dst: TempId, temp: TempId },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{
        Block, BlockId, ConstValue, Function, FunctionId, Temporary, Terminator,
        TypeEstimate,
    };
    use nod_reader::{FileId, Span};

    fn fake_span() -> Span {
        Span::new(FileId(0), 0, 0)
    }

    fn mk_temp(id: u32, ty: TypeEstimate) -> Temporary {
        Temporary {
            id: TempId(id),
            type_estimate: ty,
        }
    }

    #[test]
    fn fixnum_args_do_not_register_as_roots() {
        // fn f() -> Integer:
        //   t0 = Const 1
        //   t1 = DirectCall foo(t0)     ; safepoint_roots empty (t0 is dead)
        //   Return t1
        let f = Function {
            id: FunctionId(0),
            name: "f".into(),
            params: vec![],
            entry: BlockId(0),
            blocks: vec![Block {
                id: BlockId(0),
                label: "entry".into(),
                params: vec![],
                computations: vec![
                    Computation::Const {
                        dst: TempId(0),
                        value: ConstValue::Integer(1),
                    },
                    Computation::DirectCall {
                        dst: TempId(1),
                        callee: "foo".into(),
                        args: vec![TempId(0)],
                        safepoint_roots: vec![],
                    },
                ],
                terminator: Terminator::Return { value: Some(TempId(1)) },
            }],
            temps: vec![
                mk_temp(0, TypeEstimate::Integer),
                mk_temp(1, TypeEstimate::Integer),
            ],
            return_type: TypeEstimate::Integer,
            span: fake_span(),
        };
        let mut f = f;
        populate_safepoint_roots(&mut f);
        let c = &f.blocks[0].computations[1];
        assert_eq!(c.safepoint_roots(), Some(&[][..]));
    }

    #[test]
    fn live_pointer_across_call_gets_protected() {
        // fn f() -> Top:
        //   t0 = Const string "hello"           ; pointer-shaped
        //   t1 = DirectCall foo()               ; t0 live across
        //   t2 = DirectCall bar(t0, t1)         ; uses t0 + t1
        //   Return t2
        let mut f = Function {
            id: FunctionId(0),
            name: "f".into(),
            params: vec![],
            entry: BlockId(0),
            blocks: vec![Block {
                id: BlockId(0),
                label: "entry".into(),
                params: vec![],
                computations: vec![
                    Computation::Const {
                        dst: TempId(0),
                        value: ConstValue::String("hello".into()),
                    },
                    Computation::DirectCall {
                        dst: TempId(1),
                        callee: "foo".into(),
                        args: vec![],
                        safepoint_roots: vec![],
                    },
                    Computation::DirectCall {
                        dst: TempId(2),
                        callee: "bar".into(),
                        args: vec![TempId(0), TempId(1)],
                        safepoint_roots: vec![],
                    },
                ],
                terminator: Terminator::Return { value: Some(TempId(2)) },
            }],
            temps: vec![
                mk_temp(0, TypeEstimate::String),
                mk_temp(1, TypeEstimate::Top),
                mk_temp(2, TypeEstimate::Top),
            ],
            return_type: TypeEstimate::Top,
            span: fake_span(),
        };
        populate_safepoint_roots(&mut f);
        // First call: t0 alive across.
        assert_eq!(f.blocks[0].computations[1].safepoint_roots(), Some(&[TempId(0)][..]));
        // Second call: t0 and t1 alive across (t1 used in Return).
        let second = f.blocks[0].computations[2].safepoint_roots().unwrap();
        // Args of the call don't need protection AT this call (they're
        // operands), but if also used later (Return reads t2 only, so
        // t0 and t1 dead AFTER bar), they aren't live AFTER the call.
        // The Return only references t2 — so t0 and t1 are dead at end
        // of block — the second call has no roots to protect.
        assert!(
            second.is_empty(),
            "bar's args are dead after the call; no roots: {second:?}"
        );
    }
}
