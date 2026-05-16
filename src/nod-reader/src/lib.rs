//! `nod-reader` — Dylan lexer and AST builder.
//!
//! Sprint 02: lexer + source map. Parser (AST) lands in Sprint 03/04.
//!
//! See `specs/01-lexer.md` for the contract this crate implements.

pub mod format;
pub mod lexer;
pub mod span;
pub mod token;

pub use format::format_tokens;
pub use lexer::{Preamble, lex, scan_preamble};
pub use span::{FileId, SourceMap, SourceMapError, Span};
pub use token::{Token, TokenKind};
