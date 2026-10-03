//! `StructureSource` backed by tree-sitter grammars bundled into the binary.
//! Translation only: tags-query output becomes domain symbols here, and
//! nothing tree-sitter-specific leaves this crate.

mod grammars;
mod lines;
mod tags;

use std::path::Path;

use crt_app::{Structure, StructureError, StructureSource};
use crt_domain::LanguageId;

/// One bundled language, for listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledLanguage {
    pub id: LanguageId,
    pub extensions: &'static [&'static str],
}

/// The bundled grammars as a structure source.
#[derive(Debug, Default, Clone, Copy)]
pub struct TreeSitterSource;

impl TreeSitterSource {
    /// Every bundled language.
    pub fn languages() -> impl Iterator<Item = BundledLanguage> {
        grammars::all().iter().map(|g| BundledLanguage {
            id: LanguageId::new(g.id),
            extensions: g.extensions,
        })
    }
}

impl StructureSource for TreeSitterSource {
    fn language_for(&self, path: &Path) -> Option<LanguageId> {
        grammars::for_path(path).map(|g| LanguageId::new(g.id))
    }

    fn structure(&self, language: &LanguageId, source: &[u8]) -> Result<Structure, StructureError> {
        let grammar = grammars::by_id(language.as_str())
            .ok_or_else(|| StructureError::UnknownLanguage(language.clone()))?;
        tags::extract(grammar, source)
    }
}
