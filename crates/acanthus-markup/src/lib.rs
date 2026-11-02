//! The ACN markup language: lexer, expressions and parser with `file:line:col` diagnostics.
//! The syntax is specified in `crates/acanthus-cli/guides/acn-spec.md` (`acanthus docs acn-spec`).
#![deny(missing_docs)]

mod actions;
mod compile;
mod diag;
mod expr;
mod hot;
mod lex;
mod parse;

pub use actions::bind_actions;
pub use compile::{Built, build};
pub use diag::{Diagnostic, Span};
pub use expr::{Expr, ExprKind, Functions, Op, Scope, Value};
pub use hot::Hot;
pub use lex::{Token, TokenKind, lex};
pub use parse::{Component, Document, Member, VERSION, parse};
