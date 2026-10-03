//! Capabilities the use cases need from the outside, in the domain's words.

use std::fmt;
use std::path::Path;

use crt_domain::{LanguageId, Symbol};

/// Reads the structure (definitions and references) of source text.
pub trait StructureSource {
    /// The language this source would use for `path`, if it knows one.
    fn language_for(&self, path: &Path) -> Option<LanguageId>;

    /// Every symbol in `source`, in file order.
    fn symbols(&self, language: &LanguageId, source: &[u8]) -> Result<Vec<Symbol>, StructureError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructureError {
    UnknownLanguage(LanguageId),
    Parse(String),
}

impl fmt::Display for StructureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLanguage(id) => write!(f, "no structure source for language {id}"),
            Self::Parse(msg) => write!(f, "could not read structure: {msg}"),
        }
    }
}

impl std::error::Error for StructureError {}
