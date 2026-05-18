//! `nod-reader` — Dylan lexer, AST, and parser.
//!
//! Sprint 02: lexer + source map.
//! Sprint 03: fragments + expression parser.
//! Sprint 04: top-level forms + statement parser + pretty-printer.
//!
//! See `specs/01-lexer.md` for the lexer contract; `SPRINTS.md` §117–178
//! for the parser sketch.

pub mod ast;
pub mod format;
pub mod format_dylan;
pub mod fragments;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod token;

pub use ast::{
    BinOp, Binder, CaseArm, ExceptionClause, Expr, ForClause, ImportSet, ImportSpec, Item,
    LibraryUseClause, LocalMethodDecl, Modifier, Module, ModuleUseClause, Param, ReturnRest,
    ReturnSig, ReturnValue, SlotAllocation, SlotDef, Statement, UnOp, format_ast,
    format_ast_module,
};
pub use format::format_tokens;
pub use format_dylan::format_dylan;
pub use fragments::{Fragment, FragmentError, GroupKind, build_fragments};
pub use lexer::{Preamble, lex, scan_preamble};
pub use parser::{Diagnostic, parse_expr, parse_module, parse_top_level_exprs};
pub use span::{FileId, SourceMap, SourceMapError, Span};
pub use token::{Token, TokenKind};
