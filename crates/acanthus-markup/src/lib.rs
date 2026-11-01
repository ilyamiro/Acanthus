//! The ACN markup language: lexer, expressions and parser with `file:line:col` diagnostics.
//! The syntax is specified in `docs/acn-spec.md`.
#![deny(missing_docs)]

mod diag;
mod expr;
mod lex;

pub use diag::{Diagnostic, Span};
pub use expr::{Expr, ExprKind, Functions, Op, Scope, Value};
pub use lex::{Token, TokenKind, lex};
