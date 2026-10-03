//! Capabilities the use cases need from the outside, in the domain's words.

use std::fmt;
use std::path::Path;

use crt_domain::{LanguageId, Symbol};

/// What a structure source found in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    /// Every symbol, in file order.
    pub symbols: Vec<Symbol>,
    /// True when the parser hit syntax errors; symbols may then be missing
    /// or have wrong spans, and readers must be told.
    pub has_syntax_error: bool,
}

/// Reads the structure (definitions and references) of source text.
pub trait StructureSource {
    /// The language this source would use for `path`, if it knows one.
    fn language_for(&self, path: &Path) -> Option<LanguageId>;

    /// The structure of `source` in `language`.
    fn structure(&self, language: &LanguageId, source: &[u8]) -> Result<Structure, StructureError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
