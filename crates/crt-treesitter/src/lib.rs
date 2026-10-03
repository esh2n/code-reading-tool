//! `StructureSource` backed by tree-sitter grammars bundled into the binary.
//! Translation only: tags-query output becomes domain symbols here, and
//! nothing tree-sitter-specific leaves this crate.

mod grammars;
mod lines;
mod tags;

use std::path::Path;

use crt_app::{StructureError, StructureSource};
use crt_domain::{LanguageId, Symbol};

pub use grammars::Grammar;

/// The bundled grammars as a structure source.
#[derive(Debug, Default, Clone, Copy)]
pub struct TreeSitterSource;

impl TreeSitterSource {
    /// Every bundled language, for listing.
    pub fn languages() -> impl Iterator<Item = &'static Grammar> {
        grammars::all().iter()
    }
}

impl StructureSource for TreeSitterSource {
    fn language_for(&self, path: &Path) -> Option<LanguageId> {
        grammars::for_path(path).map(|g| LanguageId::new(g.id))
    }

    fn symbols(&self, language: &LanguageId, source: &[u8]) -> Result<Vec<Symbol>, StructureError> {
        let grammar = grammars::by_id(language.as_str())
            .ok_or_else(|| StructureError::UnknownLanguage(language.clone()))?;
        tags::extract(grammar, source)
    }
}
