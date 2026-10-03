//! Entities and rules of the code-reading tool.
//!
//! This crate knows nothing about tree-sitter, LLMs, JSON or editors. It
//! defines what a symbol, a function and a structural fact are, and the
//! rules that derive functions from symbols. Its only dependency is a pure
//! digest; see docs/architecture.md.

pub mod function;
pub mod hash;
pub mod language;
pub mod symbol;

pub use function::{Call, Function, LineFacts, SpanOutOfRange};
pub use hash::ContentHash;
pub use language::LanguageId;
pub use symbol::{Span, Symbol, SymbolKind};
