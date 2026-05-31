//! Sprint 51e — `DylanAst` (wire tree) → `nod_reader::ast::Module`.
//!
//! This is the payoff of the AST wire format: turn the Dylan-side
//! parser's output into the *canonical* Rust AST, so the Dylan parser
//! can **replace** `parse_module` for the files it fully understands.
//! Everything it can't yet reconstruct returns [`Unsupported`], and the
//! `--parse-with-dylan` driver path falls back to the Rust parser for
//! that whole file. The bar is **byte-identical** `format_ast_module`
//! output vs the Rust parser — so "translated" genuinely means "the two
//! parsers agree on the AST," not merely "didn't crash."
//!
//! ## What v1 translates
//!
//! - The module header (`Module: foo`) — re-scanned host-side with
//!   [`nod_reader::scan_preamble`], because the Dylan parser treats the
//!   header as ordinary body forms (a `SymbolLit`/`VariableRef` pair).
//!   Those leading forms are skipped by source offset.
//! - `define function` / `define method` with required params, a return
//!   spec, and a body of expression statements.
//! - Expressions: identifiers, string literals, integer/float/boolean
//!   literals, and calls.
//!
//! Anything else — modifiers on a definition, `#rest`/`#key` params,
//! `let`/`if`/`while`/… statement bodies, binary operators, classes,
//! generics, macros — is [`Unsupported`] and triggers fallback. Each
//! increment grows this set; the translation-coverage harness measures
//! how many corpus files take the Dylan path.
//!
//! Spans don't matter to the comparison: `format_ast_module` prints no
//! spans, only names / structure / values / operators / modifiers. We
//! still thread real spans through (recovered from the wire) so the
//! resulting `Module` is usable downstream, not just dump-equal.

use crate::dylan_parse_wire::{DylanAst, Kind};
use nod_reader::ast::{
    Binder, Expr, Item, Module, Param, ReturnRest, ReturnSig, ReturnValue, SlotAllocation, SlotDef,
    Statement,
};
use nod_reader::span::{FileId, Span};

/// A construct the v1 translator doesn't reconstruct yet. Carries a
/// short reason for the `--parse-with-dylan` fallback log.
#[derive(Debug, Clone)]
pub struct Unsupported(pub String);

fn unsupported<T>(msg: impl Into<String>) -> Result<T, Unsupported> {
    Err(Unsupported(msg.into()))
}

fn span_of(node: &DylanAst) -> Span {
    Span::new(FileId(0), node.span_lo, node.span_hi)
}

/// `&src[lo..hi]`, bounds-checked. Returns `Unsupported` on a bad span
/// rather than panicking — a malformed wire record shouldn't crash the
/// driver, just decline the Dylan path.
fn slice<'a>(src: &'a str, node: &DylanAst) -> Result<&'a str, Unsupported> {
    let lo = node.span_lo as usize;
    let hi = node.span_hi as usize;
    src.get(lo..hi)
        .ok_or_else(|| Unsupported(format!("span {lo}..{hi} out of bounds / not a char boundary")))
}

/// Translate the whole wire tree into a [`Module`]. `src` is the exact
/// source the Dylan parser was handed (the host re-reads it for every
/// leaf payload). Returns `Unsupported` if any item isn't reconstructible.
pub fn to_ast_module(tree: &DylanAst, src: &str) -> Result<Module, Unsupported> {
    if tree.kind != Kind::Body {
        return unsupported(format!("top node is {:?}, expected Body", tree.kind));
    }

    // The Dylan parser doesn't model the `Key: value` header — it lexes
    // those lines as ordinary forms. Re-scan the header host-side and
    // skip every top-level form that starts inside the preamble.
    let preamble = nod_reader::scan_preamble(src);
    let header = preamble
        .as_ref()
        .map(|p| p.entries.clone())
        .unwrap_or_default();
    let body_start = preamble.as_ref().map(|p| p.end).unwrap_or(0);

    let mut items = Vec::new();
    for child in &tree.children {
        // Skip the header forms the Dylan parser lexed as ordinary
        // constituents (`Module: foo` → a SymbolLit/VariableRef pair):
        // they are spanned and lie entirely within the preamble. An
        // UNSPANNED node (span_hi == 0) is NEVER a header form — it's an
        // `Error` or some unspanned construct — and must not be silently
        // dropped, or we'd emit a too-empty Module instead of an honest
        // fallback. (This bit us on stdlib-min/ide_win_calls, whose
        // `define macro`/`define c-function` forms emit as `Error 0..0`.)
        if child.span_hi != 0 && child.span_hi <= body_start {
            continue;
        }
        if child.kind == Kind::Error {
            return unsupported("Dylan parser emitted an Error node");
        }
        items.push(translate_item(child, src)?);
    }

    Ok(Module {
        span: span_of(tree),
        header,
        items,
    })
}

fn translate_item(node: &DylanAst, src: &str) -> Result<Item, Unsupported> {
    match node.kind {
        Kind::DefineFunction | Kind::DefineMethod => translate_def(node, src),
        Kind::DefineClass => translate_class(node, src),
        other => unsupported(format!("top-level {other:?}")),
    }
}

/// `define class NAME (supers) slot… end` → `Item::DefineClass`. Wire
/// children: `DefName` (class name), then super exprs and `SlotSpec`s
/// (dispatched by kind).
fn translate_class(node: &DylanAst, src: &str) -> Result<Item, Unsupported> {
    if has_modifiers(src, node.span_lo as usize) {
        return unsupported("class has modifiers (not on the wire yet)");
    }
    let mut name: Option<String> = None;
    let mut supers: Vec<Expr> = Vec::new();
    let mut slots: Vec<SlotDef> = Vec::new();
    for child in &node.children {
        match child.kind {
            Kind::DefName => name = Some(slice(src, child)?.to_string()),
            Kind::SlotSpec => slots.push(translate_slot(child, src)?),
            _ => supers.push(translate_expr(child, src)?),
        }
    }
    let name = name.ok_or_else(|| Unsupported("class has no DefName".into()))?;
    Ok(Item::DefineClass {
        span: span_of(node),
        modifiers: Vec::new(),
        name,
        supers,
        slots,
    })
}

/// One `SlotSpec` → `SlotDef`. Children are kind-tagged: `DefName`
/// (name), `SlotAlloc` (allocation adjective), `SlotInitKw`
/// (init-keyword, host strips the trailing `:`), `SlotRequired`
/// (required-init-keyword marker), `SlotType`/`SlotInit` (wrapped exprs).
fn translate_slot(node: &DylanAst, src: &str) -> Result<SlotDef, Unsupported> {
    let mut name: Option<String> = None;
    let mut allocation = SlotAllocation::Instance;
    let mut init_keyword: Option<String> = None;
    let mut required_init_keyword = false;
    let mut type_: Option<Expr> = None;
    let mut init_value: Option<Expr> = None;
    for child in &node.children {
        match child.kind {
            Kind::DefName => name = Some(slice(src, child)?.to_string()),
            Kind::SlotAlloc => {
                allocation = match slice(src, child)? {
                    "class" => SlotAllocation::Class,
                    "each-subclass" => SlotAllocation::EachSubclass,
                    "virtual" => SlotAllocation::Virtual,
                    "constant" => SlotAllocation::Constant,
                    other => return unsupported(format!("slot allocation {other:?}")),
                };
            }
            Kind::SlotInitKw => {
                init_keyword = Some(slice(src, child)?.trim_end_matches(':').to_string());
            }
            Kind::SlotRequired => required_init_keyword = true,
            Kind::SlotType => {
                let t = child
                    .children
                    .first()
                    .ok_or_else(|| Unsupported("SlotType has no child".into()))?;
                type_ = Some(translate_expr(t, src)?);
            }
            Kind::SlotInit => {
                let v = child
                    .children
                    .first()
                    .ok_or_else(|| Unsupported("SlotInit has no child".into()))?;
                init_value = Some(translate_expr(v, src)?);
            }
            other => return unsupported(format!("slot child {other:?}")),
        }
    }
    let name = name.ok_or_else(|| Unsupported("slot has no name".into()))?;
    Ok(SlotDef {
        span: span_of(node),
        name,
        type_,
        init_value,
        init_keyword,
        required_init_keyword,
        setter: None,
        allocation,
    })
}

/// Shared translation for `DefineFunction` / `DefineMethod`, whose wire
/// children are (in any order, dispatched by kind): `DefName`,
/// `ParamList`, optional `ReturnSpec`, `Body`.
fn translate_def(node: &DylanAst, src: &str) -> Result<Item, Unsupported> {
    // The wire doesn't carry definition modifiers yet. They sit between
    // `define` and the body-word; if the token immediately preceding the
    // body-word isn't `define`, there's a modifier we can't reconstruct.
    if has_modifiers(src, node.span_lo as usize) {
        return unsupported("definition has modifiers (not on the wire yet)");
    }

    let mut name: Option<String> = None;
    let mut params: Vec<Param> = Vec::new();
    let mut return_: Option<ReturnSig> = None;
    let mut body: Option<Vec<Statement>> = None;

    for child in &node.children {
        match child.kind {
            Kind::DefName => name = Some(slice(src, child)?.to_string()),
            Kind::ParamList => params = translate_param_list(child, src)?,
            Kind::ReturnSpec => return_ = Some(translate_return_spec(child, src)?),
            Kind::Body => body = Some(translate_body(child, src)?),
            other => return unsupported(format!("unexpected definition child {other:?}")),
        }
    }

    let name = name.ok_or_else(|| Unsupported("definition has no DefName".into()))?;
    let body = body.ok_or_else(|| Unsupported("definition has no Body".into()))?;
    let span = span_of(node);

    Ok(match node.kind {
        Kind::DefineFunction => Item::DefineFunction {
            span,
            modifiers: Vec::new(),
            name,
            params,
            return_,
            body,
        },
        Kind::DefineMethod => Item::DefineMethod {
            span,
            modifiers: Vec::new(),
            name,
            params,
            return_,
            body,
        },
        _ => unreachable!("translate_def only called for function/method"),
    })
}

/// Is there a modifier word between `define` and the body-word at
/// `body_word_lo`? The token directly before the body-word is `define`
/// when there are none. Scans back over whitespace, then over the
/// preceding identifier run.
fn has_modifiers(src: &str, body_word_lo: usize) -> bool {
    let bytes = src.as_bytes();
    let mut i = body_word_lo;
    // Back over whitespace.
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    // Back over the identifier run (Dylan names: alnum plus -, _, !, ?,
    // *, $, <, >).
    let end = i;
    while i > 0 && is_name_byte(bytes[i - 1]) {
        i -= 1;
    }
    let word = &src[i..end];
    word != "define"
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'!' | b'?' | b'*' | b'$' | b'<' | b'>')
}

fn translate_param_list(node: &DylanAst, src: &str) -> Result<Vec<Param>, Unsupported> {
    let mut params = Vec::new();
    for child in &node.children {
        match child.kind {
            Kind::Param => {
                let span = span_of(child);
                let name = slice(src, child)?.to_string();
                let type_ = match child.children.first() {
                    Some(t) => Some(translate_expr(t, src)?),
                    None => None,
                };
                params.push(Param { span, name, type_ });
            }
            Kind::VarMarker => {
                return unsupported("param list has #rest/#key/#all-keys/#next");
            }
            other => return unsupported(format!("unexpected param-list child {other:?}")),
        }
    }
    Ok(params)
}

fn translate_return_spec(node: &DylanAst, src: &str) -> Result<ReturnSig, Unsupported> {
    let mut values = Vec::new();
    // v1 always declines `#rest` returns (the VarMarker arm below bails),
    // so the reconstructed rest is always absent.
    let rest: Option<ReturnRest> = None;
    for child in &node.children {
        match child.kind {
            Kind::ReturnValue => {
                let span = span_of(child);
                // A type child present → `name :: type` (name = span).
                // No child → a bare type like `<integer>` → the Dylan
                // parser stored the type AS the token, so name = None
                // and type = Ident(span). See DYLAN_AST_WIRE.md row 30.
                match child.children.first() {
                    Some(t) => values.push(ReturnValue {
                        span,
                        name: Some(slice(src, child)?.to_string()),
                        type_: Some(translate_expr(t, src)?),
                    }),
                    None => {
                        let ident = Expr::Ident(span, slice(src, child)?.to_string());
                        values.push(ReturnValue {
                            span,
                            name: None,
                            type_: Some(ident),
                        });
                    }
                }
            }
            Kind::VarMarker => return unsupported("return spec has #rest"),
            other => return unsupported(format!("unexpected return-spec child {other:?}")),
        }
    }
    Ok(ReturnSig {
        span: span_of(node),
        values,
        rest,
    })
}

/// A function/method body Body → a `Vec<Statement>`. A `LocalDecl`
/// constituent is a `Statement::Let`; everything else is a translatable
/// expression wrapped in `Statement::Expr` (an `if` at statement
/// position becomes `Statement::Expr(Expr::If)`, matching the Rust
/// parser).
fn translate_body(node: &DylanAst, src: &str) -> Result<Vec<Statement>, Unsupported> {
    translate_stmts(&node.children, src)
}

/// A sequence of body constituents → `Vec<Statement>`. `LocalDecl` →
/// `Statement::Let`; a `Statement` node → the matching statement form
/// (`while`/`until` → loops, `if` → `Stmt(Expr::If)`); everything else
/// is a `Statement::Expr`.
fn translate_stmts(children: &[DylanAst], src: &str) -> Result<Vec<Statement>, Unsupported> {
    let mut stmts = Vec::new();
    for child in children {
        match child.kind {
            Kind::LocalDecl => stmts.push(translate_local_decl(child, src)?),
            Kind::Statement => stmts.push(translate_statement(child, src)?),
            _ => stmts.push(Statement::Expr(translate_expr(child, src)?)),
        }
    }
    Ok(stmts)
}

/// `let <binder> = <init>` → `Statement::Let`. The Dylan parser models
/// the whole `binder = init` as a single `=`-`BinaryOp` inside the
/// LocalDecl's body. v1 handles a single, untyped binder
/// (`let x = e`); a typed binder (`let x :: T = e`), a multi-binder
/// (`let (a, b) = e`), or a missing init falls back.
fn translate_local_decl(node: &DylanAst, src: &str) -> Result<Statement, Unsupported> {
    let body = node
        .children
        .first()
        .ok_or_else(|| Unsupported("let: no body".into()))?;
    if body.kind != Kind::Body {
        return unsupported("let: child is not a Body");
    }
    if body.children.len() != 1 {
        return unsupported(format!("let: body has {} forms", body.children.len()));
    }
    let binop = &body.children[0];
    if binop.kind != Kind::BinaryOp || binop.children.len() != 2 {
        return unsupported("let: body is not a `binder = init` binding");
    }
    let lhs = &binop.children[0];
    let rhs = &binop.children[1];
    // Confirm the join operator is `=` (the let binder), not something else.
    let lhs_ext = subtree_extent(lhs)
        .ok_or_else(|| Unsupported("let: binder has no span".into()))?;
    let rhs_ext = subtree_extent(rhs)
        .ok_or_else(|| Unsupported("let: init has no span".into()))?;
    let gap = src
        .get(lhs_ext.1 as usize..rhs_ext.0 as usize)
        .ok_or_else(|| Unsupported("let: binder gap out of bounds".into()))?;
    let op_str = operator_in_gap(gap);
    if op_str != "=" {
        return unsupported(format!("let binder operator {op_str:?}"));
    }
    if lhs.kind != Kind::VariableRef {
        return unsupported("let: non-simple binder (typed or destructuring)");
    }
    let name = slice(src, lhs)?.to_string();
    let value = translate_expr(rhs, src)?;
    Ok(Statement::Let {
        span: span_of(node),
        binders: vec![Binder {
            span: span_of(lhs),
            name,
            type_: None,
        }],
        rest: None,
        value,
    })
}

fn translate_expr(node: &DylanAst, src: &str) -> Result<Expr, Unsupported> {
    let span = span_of(node);
    match node.kind {
        Kind::VariableRef => Ok(Expr::Ident(span, slice(src, node)?.to_string())),
        // ast::Expr::String stores the RAW quoted source slice (the Rust
        // parser does NOT decode escapes here) — so the verbatim span
        // text is exactly right.
        Kind::StringLit => Ok(Expr::String(span, slice(src, node)?.to_string())),
        Kind::IntegerLit => {
            let text = slice(src, node)?;
            let v = parse_integer(text)
                .ok_or_else(|| Unsupported(format!("integer literal {text:?}")))?;
            Ok(Expr::Integer(span, v))
        }
        Kind::FloatLit => {
            let text = slice(src, node)?;
            let v: f64 = text
                .parse()
                .map_err(|_| Unsupported(format!("float literal {text:?}")))?;
            Ok(Expr::Float(span, v))
        }
        Kind::BoolLit => {
            let text = slice(src, node)?;
            match text {
                "#t" => Ok(Expr::Bool(span, true)),
                "#f" => Ok(Expr::Bool(span, false)),
                other => unsupported(format!("boolean literal {other:?}")),
            }
        }
        Kind::Call => {
            let mut it = node.children.iter();
            let callee_node = it
                .next()
                .ok_or_else(|| Unsupported("Call with no callee".into()))?;
            // The Dylan parser has no body-macro knowledge: it parses
            // `when (cond) body end` as a plain call `when(cond)` with a
            // dangling body, whereas the Rust parser (seeded with the
            // stdlib macro names) folds the whole form into one
            // `Expr::MacroCall`. The two ASTs genuinely disagree, so we
            // can't authoritatively translate a call to a known macro —
            // fall back to the Rust parser for the whole file. (Until the
            // Dylan parser itself learns macro-call parsing + seeding.)
            if callee_node.kind == Kind::VariableRef && is_body_macro(slice(src, callee_node)?) {
                return unsupported(format!(
                    "call to body-macro {:?} (Dylan parser lacks macro seeding)",
                    slice(src, callee_node)?
                ));
            }
            let callee = Box::new(translate_expr(callee_node, src)?);
            let mut args = Vec::new();
            for a in it {
                args.push(translate_expr(a, src)?);
            }
            Ok(Expr::Call { span, callee, args })
        }
        Kind::BinaryOp => {
            if node.children.len() != 2 {
                return unsupported(format!("BinaryOp arity {}", node.children.len()));
            }
            let lhs = &node.children[0];
            let rhs = &node.children[1];
            // PRECEDENCE FORK: the Rust parser climbs C-style precedence
            // (`*` binds tighter than `+`), while the Dylan-in-Dylan
            // parser is flat left-associative (the DRM rule: all infix
            // operators share one precedence). For a chain like
            // `a * b + c * d` the two build DIFFERENT trees. We can't
            // reconcile that here, so any nested binary operator falls
            // back to the Rust parser. A single binop (operands that
            // aren't themselves binops) is unambiguous and safe.
            // (Reconciling the two precedence models is its own task —
            // see docs/journal.)
            if lhs.kind == Kind::BinaryOp || rhs.kind == Kind::BinaryOp {
                return unsupported("nested binary op (Rust precedence vs Dylan flat-assoc)");
            }
            // The operator token isn't a node — it lives in the source
            // gap between the operands. A node's own span may not cover
            // its children (a `Call`'s span is just its paren), so we
            // bound the gap by the TRUE subtree extents.
            let lhs_ext = subtree_extent(lhs)
                .ok_or_else(|| Unsupported("BinaryOp lhs has no span".into()))?;
            let rhs_ext = subtree_extent(rhs)
                .ok_or_else(|| Unsupported("BinaryOp rhs has no span".into()))?;
            let gap = src
                .get(lhs_ext.1 as usize..rhs_ext.0 as usize)
                .ok_or_else(|| Unsupported("BinaryOp operator gap out of bounds".into()))?;
            let op_str = operator_in_gap(gap);
            let op = parse_binop(&op_str)
                .ok_or_else(|| Unsupported(format!("binary operator {op_str:?}")))?;
            let lhs = Box::new(translate_expr(lhs, src)?);
            let rhs = Box::new(translate_expr(rhs, src)?);
            Ok(Expr::BinOp { span, op, lhs, rhs })
        }
        // A statement at expression position — Dylan's `if`/`while`/… are
        // value-producing. v1 reconstructs `if` (→ Expr::If with
        // Begin-wrapped arms); other statement keywords fall back.
        Kind::Statement => translate_statement_as_expr(node, src),
        // `key: value` keyword argument → the Rust parser's synthetic
        // `%kw-arg(Symbol("key:"), value)` call. The `key:` symbol keeps
        // its trailing colon (matches `(Symbol "x:")`).
        Kind::KwArg => {
            let key = slice(src, node)?.to_string();
            let value_node = node
                .children
                .first()
                .ok_or_else(|| Unsupported("KwArg has no value".into()))?;
            let value = translate_expr(value_node, src)?;
            Ok(Expr::Call {
                span,
                callee: Box::new(Expr::Ident(span, "%kw-arg".to_string())),
                args: vec![Expr::Symbol(span, key), value],
            })
        }
        other => unsupported(format!("expression {other:?}")),
    }
}

/// The true byte extent of a subtree: min `span_lo` / max `span_hi`
/// over the node and all descendants that carry a real span (`hi >
/// lo`). Unspanned nodes (`0..0`, e.g. a backfill-less `Call` whose own
/// record is just the paren) contribute only through their children.
fn subtree_extent(node: &DylanAst) -> Option<(u32, u32)> {
    let mut acc: Option<(u32, u32)> = if node.span_hi > node.span_lo {
        Some((node.span_lo, node.span_hi))
    } else {
        None
    };
    for c in &node.children {
        if let Some((clo, chi)) = subtree_extent(c) {
            acc = Some(match acc {
                Some((lo, hi)) => (lo.min(clo), hi.max(chi)),
                None => (clo, chi),
            });
        }
    }
    acc
}

/// The stdlib body-shaped macro names the Rust `dump-ast` path seeds
/// the parser with. A call to one of these is a `MacroCall` to the Rust
/// parser but a plain function call to the (macro-unaware) Dylan parser
/// — so the translator declines it. Keep in sync with the seed list in
/// `main.rs::run_dump_ast`.
fn is_body_macro(name: &str) -> bool {
    matches!(
        name,
        "case" | "cond" | "for-each" | "iterate" | "select" | "unless" | "when" | "while"
    )
}

/// Extract the operator token from the source gap between two operands.
/// The gap can carry a closing `)` from the left operand's call/parens
/// and/or an opening `(` from the right operand's — e.g. `f(x) + y`
/// yields the gap `") + "`. Strip ALL parens and whitespace; what
/// remains is the operator (`+`, `<=`, `:=`, `mod`, …). Operators never
/// contain parens or whitespace, so this is lossless.
fn operator_in_gap(gap: &str) -> String {
    gap.chars()
        .filter(|c| !c.is_whitespace() && *c != '(' && *c != ')')
        .collect()
}

/// Map a Dylan infix-operator token to `ast::BinOp`. The operator is
/// matched exactly, so `=`/`==`/`:=` disambiguate cleanly.
fn parse_binop(op: &str) -> Option<nod_reader::ast::BinOp> {
    use nod_reader::ast::BinOp;
    Some(match op {
        "+" => BinOp::Add,
        "-" => BinOp::Sub,
        "*" => BinOp::Mul,
        "/" => BinOp::Div,
        "mod" => BinOp::Mod,
        "rem" => BinOp::Rem,
        "^" => BinOp::Pow,
        "=" => BinOp::Eq,
        "==" => BinOp::EqEq,
        "~=" => BinOp::Ne,
        "~==" => BinOp::NeEq,
        "<" => BinOp::Lt,
        ">" => BinOp::Gt,
        "<=" => BinOp::Le,
        ">=" => BinOp::Ge,
        "&" => BinOp::And,
        "|" => BinOp::Or,
        ":=" => BinOp::Assign,
        _ => return None,
    })
}

/// A `Statement` wire node at EXPRESSION position. `if` → `Expr::If`;
/// `while`/`until` → `Expr::Stmt(Statement::While|Until)` (the Rust
/// parser wraps a statement form in `Expr::Stmt` when it appears where
/// a value is expected). Other keywords fall back.
fn translate_statement_as_expr(node: &DylanAst, src: &str) -> Result<Expr, Unsupported> {
    match slice(src, node)? {
        "if" => build_if(node, src),
        "while" | "until" => Ok(Expr::Stmt(Box::new(translate_statement(node, src)?))),
        other => unsupported(format!("statement {other:?}")),
    }
}

/// A `Statement` wire node at STATEMENT position → `ast::Statement`.
/// `if` → `Statement::Expr(Expr::If)`; `while`/`until` → the loop forms.
fn translate_statement(node: &DylanAst, src: &str) -> Result<Statement, Unsupported> {
    match slice(src, node)? {
        "if" => Ok(Statement::Expr(build_if(node, src)?)),
        "while" => build_loop(node, src, /* is_while */ true),
        "until" => build_loop(node, src, /* is_while */ false),
        other => unsupported(format!("statement {other:?}")),
    }
}

/// `if` → `Expr::If`. Wire shape: child[0] = leading `Body` holding
/// `[cond, then-forms…]`, then zero-or-more `StatementClause` children
/// (`else`/`elseif`). v1 handles a bare `if` and a single `else`;
/// `elseif` (nested-If desugaring) falls back.
fn build_if(node: &DylanAst, src: &str) -> Result<Expr, Unsupported> {
    let mut children = node.children.iter();
    let head = children
        .next()
        .ok_or_else(|| Unsupported("if: no head body".into()))?;
    if head.kind != Kind::Body {
        return unsupported("if: head is not a Body");
    }
    if head.children.is_empty() {
        return unsupported("if: empty head body (no condition)");
    }
    let cond = Box::new(translate_expr(&head.children[0], src)?);
    let then_body = head.children[1..]
        .iter()
        .map(|c| translate_expr(c, src))
        .collect::<Result<Vec<_>, _>>()?;
    let then_ = Box::new(Expr::Begin {
        span: span_of(head),
        body: then_body,
    });

    let mut else_: Option<Box<Expr>> = None;
    for clause in children {
        if clause.kind != Kind::StatementClause {
            return unsupported(format!("if: unexpected child {:?}", clause.kind));
        }
        let ckw = slice(src, clause)?;
        if ckw != "else" {
            // `elseif`/`finally`/… — the nested-If desugaring is a later
            // increment; fall back for now.
            return unsupported(format!("if clause {ckw:?}"));
        }
        if else_.is_some() {
            return unsupported("if: multiple else clauses");
        }
        let cbody = clause
            .children
            .first()
            .ok_or_else(|| Unsupported("else: no body".into()))?;
        if cbody.kind != Kind::Body {
            return unsupported("else: clause child is not a Body");
        }
        let else_body = cbody
            .children
            .iter()
            .map(|c| translate_expr(c, src))
            .collect::<Result<Vec<_>, _>>()?;
        else_ = Some(Box::new(Expr::Begin {
            span: span_of(cbody),
            body: else_body,
        }));
    }

    Ok(Expr::If {
        span: span_of(node),
        cond,
        then_,
        else_,
    })
}

/// `while`/`until` → `Statement::While`/`Until`. Wire shape: child[0] =
/// leading `Body` holding `[cond, body-forms…]`. The body forms are
/// translated as statements (so a nested `let`/loop is handled too).
fn build_loop(node: &DylanAst, src: &str, is_while: bool) -> Result<Statement, Unsupported> {
    let head = node
        .children
        .first()
        .ok_or_else(|| Unsupported("loop: no head body".into()))?;
    if head.kind != Kind::Body {
        return unsupported("loop: head is not a Body");
    }
    if head.children.is_empty() {
        return unsupported("loop: empty head body (no condition)");
    }
    if node.children.len() != 1 {
        // A loop has no trailing clauses; extra children mean something
        // we don't model (e.g. a `for`/`finally` shape).
        return unsupported("loop: unexpected trailing clause");
    }
    let cond = translate_expr(&head.children[0], src)?;
    let body = translate_stmts(&head.children[1..], src)?;
    let span = span_of(node);
    Ok(if is_while {
        Statement::While { span, cond, body }
    } else {
        Statement::Until { span, cond, body }
    })
}

/// Parse a Dylan integer literal text into `i128`. Handles decimal and
/// the `#x`/`#o`/`#b`/`#d` radix prefixes. Returns `None` on anything
/// else so the caller can fall back.
fn parse_integer(text: &str) -> Option<i128> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix("#x").or_else(|| t.strip_prefix("#X")) {
        return i128::from_str_radix(&hex.replace('_', ""), 16).ok();
    }
    if let Some(oct) = t.strip_prefix("#o").or_else(|| t.strip_prefix("#O")) {
        return i128::from_str_radix(&oct.replace('_', ""), 8).ok();
    }
    if let Some(bin) = t.strip_prefix("#b").or_else(|| t.strip_prefix("#B")) {
        return i128::from_str_radix(&bin.replace('_', ""), 2).ok();
    }
    if let Some(dec) = t.strip_prefix("#d").or_else(|| t.strip_prefix("#D")) {
        return dec.replace('_', "").parse().ok();
    }
    t.replace('_', "").parse().ok()
}
