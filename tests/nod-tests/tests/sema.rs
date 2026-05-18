//! Sprint 06 — AST → DFM lowering + verifier + format_dfm.

use std::path::{Path, PathBuf};

use nod_dfm::{
    Block, BlockId, Computation, ConstValue, Function, FunctionId, PrimOp, Temporary, TempId,
    Terminator, TypeEstimate, VerifyError, format_dfm, format_dfm_module, verify,
};
use nod_reader::{
    FileId, Module, SourceMap, Span, lex, parse_module, scan_preamble,
};
use nod_sema::lower_module;

fn parse_src(src: &str) -> Module {
    let mut sm = SourceMap::new();
    let id = sm.add("<t>", src.to_string()).unwrap();
    let toks = lex(src, id);
    let pre = scan_preamble(src);
    parse_module(src, &toks, pre.as_ref())
        .unwrap_or_else(|d| panic!("parse_module diagnostics: {d:?}\n--- src ---\n{src}"))
}

fn lower_src(src: &str) -> Vec<Function> {
    let m = parse_src(src);
    lower_module(&m).unwrap_or_else(|e| panic!("lower errors: {e:?}\n--- src ---\n{src}"))
}

fn fake_span() -> Span {
    Span { file_id: FileId(0), lo: 0, hi: 0 }
}

// ─── 1. Const lowering ───────────────────────────────────────────────────

#[test]
fn const_int_lowers() {
    let fns = lower_src("define constant x = 42;");
    assert_eq!(fns.len(), 1);
    let f = &fns[0];
    assert_eq!(f.name, "x");
    assert_eq!(f.return_type, TypeEstimate::Integer);
    assert_eq!(f.blocks.len(), 1);
    let entry = &f.blocks[0];
    assert_eq!(entry.computations.len(), 1);
    match &entry.computations[0] {
        Computation::Const { value: ConstValue::Integer(42), .. } => {}
        c => panic!("expected Const Integer(42), got {c:?}"),
    }
    match &entry.terminator {
        Terminator::Return { value: Some(_) } => {}
        t => panic!("expected Return Some, got {t:?}"),
    }
    verify(f).expect("verify");
}

// ─── 2. Integer arithmetic ───────────────────────────────────────────────

#[test]
fn integer_add_lowers() {
    let src = "define function add (x :: <integer>, y :: <integer>) => (<integer>) x + y end;";
    let fns = lower_src(src);
    assert_eq!(fns.len(), 1);
    let f = &fns[0];
    assert_eq!(f.name, "add");
    assert_eq!(f.return_type, TypeEstimate::Integer);
    assert_eq!(f.params.len(), 2);
    let entry = &f.blocks[0];
    assert_eq!(entry.computations.len(), 1);
    match &entry.computations[0] {
        Computation::PrimOp { op: PrimOp::AddInt, args, .. } => {
            assert_eq!(args.len(), 2);
            assert_eq!(args[0], f.params[0]);
            assert_eq!(args[1], f.params[1]);
        }
        c => panic!("expected AddInt, got {c:?}"),
    }
    verify(f).expect("verify");
}

// ─── 3. Float arithmetic ─────────────────────────────────────────────────

#[test]
fn float_arith_lowers() {
    let src = "\
define function fadd (x :: <double-float>, y :: <double-float>) => (<double-float>)
  x + y
end;
define function fsub (x :: <double-float>, y :: <double-float>) => (<double-float>)
  x - y
end;
";
    let fns = lower_src(src);
    assert_eq!(fns.len(), 2);
    for (expected_op, f) in [PrimOp::AddFloat, PrimOp::SubFloat].iter().zip(&fns) {
        assert_eq!(f.return_type, TypeEstimate::DoubleFloat);
        match &f.blocks[0].computations[0] {
            Computation::PrimOp { op, .. } if op == expected_op => {}
            c => panic!("expected {expected_op:?}, got {c:?}"),
        }
        verify(f).expect("verify");
    }
}

// ─── 4. Comparison ───────────────────────────────────────────────────────

#[test]
fn comparison_lowers_to_lt_int() {
    let src = "define function lt (x :: <integer>, y :: <integer>) => (<boolean>) x < y end;";
    let fns = lower_src(src);
    let f = &fns[0];
    assert_eq!(f.return_type, TypeEstimate::Boolean);
    match &f.blocks[0].computations[0] {
        Computation::PrimOp { op: PrimOp::LtInt, .. } => {}
        c => panic!("expected LtInt, got {c:?}"),
    }
    verify(f).expect("verify");
}

// ─── 5. If lowering ──────────────────────────────────────────────────────

#[test]
fn if_lowers_to_three_blocks_plus_join() {
    let src = "\
define function abs (x :: <integer>) => (<integer>)
  if (x < 0) -x else x end
end function abs;
";
    let fns = lower_src(src);
    let f = &fns[0];
    // entry + then + else + join = 4 blocks.
    assert_eq!(f.blocks.len(), 4, "expected 4 blocks, dump:\n{}", format_dfm(f));
    let labels: Vec<&str> = f.blocks.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels[0], "entry");
    assert!(labels[1].starts_with("then"));
    assert!(labels[2].starts_with("else"));
    assert!(labels[3].starts_with("join"));
    // Entry's terminator is an If.
    assert!(matches!(f.blocks[0].terminator, Terminator::If { .. }));
    // Both then and else jump to the join block, supplying one arg each.
    let join = &f.blocks[3];
    assert_eq!(join.params.len(), 1, "join block should carry one parameter");
    for arm in &f.blocks[1..3] {
        match &arm.terminator {
            Terminator::Jump { target, args } => {
                assert_eq!(*target, join.id);
                assert_eq!(args.len(), 1);
            }
            t => panic!("expected Jump from {} to join, got {t:?}", arm.label),
        }
    }
    // Return is in the join block.
    assert!(matches!(f.blocks[3].terminator, Terminator::Return { value: Some(_) }));
    verify(f).expect("verify");
}

// ─── 6. Direct call ──────────────────────────────────────────────────────

#[test]
fn direct_call_to_known_top_level() {
    let src = "\
define function sq (x :: <integer>) => (<integer>) x * x end;
define function double (x :: <integer>) => (<integer>) sq(x) + sq(x) end;
";
    let fns = lower_src(src);
    assert_eq!(fns.len(), 2);
    let double = fns.iter().find(|f| f.name == "double").unwrap();
    let calls: Vec<&Computation> = double.blocks[0]
        .computations
        .iter()
        .filter(|c| matches!(c, Computation::DirectCall { callee, .. } if callee == "sq"))
        .collect();
    assert_eq!(calls.len(), 2, "expected two DirectCall sq, dump:\n{}", format_dfm(double));
    verify(double).expect("verify");
}

// ─── 7. Let binding ──────────────────────────────────────────────────────

#[test]
fn let_binding_resolves_through_env() {
    let src = "\
define function f (x :: <integer>) => (<integer>)
  let y = x * 2;
  y + 1
end function f;
";
    let fns = lower_src(src);
    let f = &fns[0];
    // Computations: const 2, mul (= y), const 1, add. 4 in entry.
    let entry = &f.blocks[0];
    assert_eq!(entry.computations.len(), 4, "dump:\n{}", format_dfm(f));
    let mul = entry
        .computations
        .iter()
        .find_map(|c| match c {
            Computation::PrimOp { op: PrimOp::MulInt, args, dst } => {
                assert_eq!(args[0], f.params[0]);
                Some(*dst)
            }
            _ => None,
        })
        .expect("expected a MulInt for the let-value");
    let last = entry
        .computations
        .last()
        .expect("non-empty entry computations");
    match last {
        Computation::PrimOp { op: PrimOp::AddInt, args, .. } => {
            assert!(args.contains(&mul), "expected add to consume the let-bound temp");
        }
        c => panic!("expected AddInt as the final entry stmt, got {c:?}"),
    }
    verify(f).expect("verify");
}

// ─── 8. Sprint 12: define class lowers to slot accessors ─────────────────

#[test]
fn define_class_emits_slot_accessors() {
    let src =
        "define class <sema-pt> (<object>) slot foo :: <integer>, init-keyword: foo:; end class;";
    let fns = lower_src(src);
    // Sprint 12 emits at least a getter (and a setter, since the
    // slot is mutable by default). The user supplied no constants
    // or functions, so the entire fns list is accessors.
    assert!(
        fns.iter().any(|f| f.name == "<sema-pt>-getter-foo"),
        "expected <sema-pt>-getter-foo, dump: {:?}",
        fns.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    assert!(
        fns.iter().any(|f| f.name == "<sema-pt>-setter-foo"),
        "expected <sema-pt>-setter-foo"
    );
    // The getter is a single LoadSlot.
    let getter = fns.iter().find(|f| f.name == "<sema-pt>-getter-foo").unwrap();
    let entry = &getter.blocks[0];
    assert_eq!(entry.computations.len(), 1, "dump:\n{}", format_dfm(getter));
    match &entry.computations[0] {
        Computation::LoadSlot { offset, .. } => assert_eq!(*offset, 8),
        c => panic!("expected LoadSlot, got {c:?}"),
    }
    verify(getter).expect("verify getter");
}

// ─── 9. Verifier round-trip on every lowered function ────────────────────

#[test]
fn verifier_round_trip_kernel_arith() {
    let path = fixtures_dir().join("kernel-arith.dylan");
    let src = std::fs::read_to_string(&path).expect("read fixture");
    let fns = lower_src(&src);
    assert!(!fns.is_empty());
    for f in &fns {
        verify(f).unwrap_or_else(|e| panic!("verify failed for {}: {e:?}", f.name));
    }
}

// ─── 10. End-to-end via format_dfm — snapshot fixture ────────────────────

#[test]
fn kernel_arith_fixture_snapshot() {
    let path = fixtures_dir().join("kernel-arith.dylan");
    let src = std::fs::read_to_string(&path).expect("read fixture");
    let fns = lower_src(&src);
    let dump = format_dfm_module(&fns);
    assert_eq!(dump, EXPECTED_KERNEL_ARITH_DUMP, "DFM dump drift:\n{dump}");
}

const EXPECTED_KERNEL_ARITH_DUMP: &str = r#"fn *answer* () -> <integer>:
  entry:
    t0: <integer> = Const Integer(42)
    Return t0

fn sq (t0: <integer>) -> <integer>:
  entry:
    t1: <integer> = PrimOp MulInt t0 t0
    Return t1

fn abs (t0: <integer>) -> <integer>:
  entry:
    t1: <integer> = Const Integer(0)
    t2: <boolean> = PrimOp LtInt t0 t1
    If t2 then1 else2
  then1:
    t3: <integer> = PrimOp NegInt t0
    Jump join3(t3)
  else2:
    Jump join3(t0)
  join3(t4: <integer>):
    Return t4

fn hypot-sq (t0: <integer>, t1: <integer>) -> <integer>:
  entry:
    t2: <integer> = DirectCall sq(t0)
    t3: <integer> = DirectCall sq(t1)
    t4: <integer> = PrimOp AddInt t2 t3
    Return t4
"#;

// ─── 11. Negative verifier — four malformed Functions ────────────────────

fn empty_block(id: BlockId, label: &str, term: Terminator) -> Block {
    Block {
        id,
        label: label.to_string(),
        params: Vec::new(),
        computations: Vec::new(),
        terminator: term,
    }
}

fn t(i: u32, ty: TypeEstimate) -> Temporary {
    Temporary { id: TempId(i), type_estimate: ty }
}

#[test]
fn verify_use_before_def() {
    let f = Function {
        id: FunctionId(0),
        name: "bad".into(),
        params: Vec::new(),
        entry: BlockId(0),
        blocks: vec![Block {
            id: BlockId(0),
            label: "entry".into(),
            params: Vec::new(),
            computations: vec![Computation::PrimOp {
                dst: TempId(0),
                op: PrimOp::AddInt,
                args: vec![TempId(99)], // never defined
            }],
            terminator: Terminator::Return { value: Some(TempId(0)) },
        }],
        temps: vec![t(0, TypeEstimate::Integer)],
        return_type: TypeEstimate::Integer,
        span: fake_span(),
    };
    let errs = verify(&f).expect_err("expected use-before-def");
    assert!(
        errs.iter().any(|e| matches!(e, VerifyError::UseBeforeDef { temp, .. } if *temp == TempId(99))),
        "errors: {errs:?}"
    );
}

#[test]
fn verify_dangling_block_ref() {
    let f = Function {
        id: FunctionId(0),
        name: "bad".into(),
        params: Vec::new(),
        entry: BlockId(0),
        blocks: vec![empty_block(
            BlockId(0),
            "entry",
            Terminator::Jump {
                target: BlockId(42),
                args: Vec::new(),
            },
        )],
        temps: Vec::new(),
        return_type: TypeEstimate::Unit,
        span: fake_span(),
    };
    let errs = verify(&f).expect_err("expected dangling block");
    assert!(
        errs.iter().any(|e| matches!(e, VerifyError::DanglingBlockRef { to, .. } if *to == BlockId(42))),
        "errors: {errs:?}"
    );
}

#[test]
fn verify_double_define() {
    let f = Function {
        id: FunctionId(0),
        name: "bad".into(),
        params: Vec::new(),
        entry: BlockId(0),
        blocks: vec![Block {
            id: BlockId(0),
            label: "entry".into(),
            params: Vec::new(),
            computations: vec![
                Computation::Const { dst: TempId(0), value: ConstValue::Integer(1) },
                Computation::Const { dst: TempId(0), value: ConstValue::Integer(2) },
            ],
            terminator: Terminator::Return { value: Some(TempId(0)) },
        }],
        temps: vec![t(0, TypeEstimate::Integer)],
        return_type: TypeEstimate::Integer,
        span: fake_span(),
    };
    let errs = verify(&f).expect_err("expected double-define");
    assert!(
        errs.iter().any(|e| matches!(e, VerifyError::DoubleDefine { temp } if *temp == TempId(0))),
        "errors: {errs:?}"
    );
}

#[test]
fn verify_missing_entry() {
    // Entry-id points at a block that doesn't exist. Blocks list still has
    // a block, with id BlockId(7) — but `f.entry` is BlockId(0), absent.
    let f = Function {
        id: FunctionId(0),
        name: "bad".into(),
        params: Vec::new(),
        entry: BlockId(0),
        blocks: vec![empty_block(
            BlockId(7),
            "loose",
            Terminator::Return { value: None },
        )],
        temps: Vec::new(),
        return_type: TypeEstimate::Unit,
        span: fake_span(),
    };
    let errs = verify(&f).expect_err("expected missing-entry");
    assert!(
        errs.iter().any(|e| matches!(e, VerifyError::MissingEntry { .. })),
        "errors: {errs:?}"
    );
}

// ─── Fixture-locator helper ──────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}
