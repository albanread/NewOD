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
    Expr, Item, Module, Param, ReturnRest, ReturnSig, ReturnValue, Statement,
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
        other => unsupported(format!("top-level {other:?}")),
    }
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

/// A function/method body Body → a `Vec<Statement>`. Each constituent
/// must be a translatable expression (v1 doesn't do `let`/`if`/loops in
/// a body — those are `LocalDecl`/`Statement` wire kinds → Unsupported).
fn translate_body(node: &DylanAst, src: &str) -> Result<Vec<Statement>, Unsupported> {
    let mut stmts = Vec::new();
    for child in &node.children {
        let e = translate_expr(child, src)?;
        stmts.push(Statement::Expr(e));
    }
    Ok(stmts)
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
            let callee = it
                .next()
                .ok_or_else(|| Unsupported("Call with no callee".into()))?;
            let callee = Box::new(translate_expr(callee, src)?);
            let mut args = Vec::new();
            for a in it {
                args.push(translate_expr(a, src)?);
            }
            Ok(Expr::Call { span, callee, args })
        }
        other => unsupported(format!("expression {other:?}")),
    }
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
