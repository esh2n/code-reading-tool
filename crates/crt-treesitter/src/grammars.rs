//! The bundled grammars. Adding a language is one entry here: the grammar
//! crate and the tags query it ships. No other code changes.

use std::path::Path;

use tree_sitter::Language;

pub struct Grammar {
    /// Stable id, also the domain `LanguageId`.
    pub id: &'static str,
    /// File extensions without the dot.
    pub extensions: &'static [&'static str],
    language: fn() -> Language,
    pub(crate) tags_query: &'static str,
}

impl Grammar {
    pub(crate) fn language(&self) -> Language {
        (self.language)()
    }
}

const GRAMMARS: &[Grammar] = &[
    Grammar {
        id: "rust",
        extensions: &["rs"],
        language: || tree_sitter_rust::LANGUAGE.into(),
        tags_query: tree_sitter_rust::TAGS_QUERY,
    },
    Grammar {
        id: "go",
        extensions: &["go"],
        language: || tree_sitter_go::LANGUAGE.into(),
        tags_query: tree_sitter_go::TAGS_QUERY,
    },
    Grammar {
        id: "python",
        extensions: &["py"],
        language: || tree_sitter_python::LANGUAGE.into(),
        tags_query: tree_sitter_python::TAGS_QUERY,
    },
    Grammar {
        id: "javascript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        language: || tree_sitter_javascript::LANGUAGE.into(),
        tags_query: tree_sitter_javascript::TAGS_QUERY,
    },
];

pub(crate) fn all() -> &'static [Grammar] {
    GRAMMARS
}

pub(crate) fn by_id(id: &str) -> Option<&'static Grammar> {
    GRAMMARS.iter().find(|g| g.id == id)
}

pub(crate) fn for_path(path: &Path) -> Option<&'static Grammar> {
    let ext = path.extension()?.to_str()?;
    GRAMMARS.iter().find(|g| g.extensions.contains(&ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_grammar_by_extension() {
        assert_eq!(for_path(Path::new("a/b.go")).map(|g| g.id), Some("go"));
        assert_eq!(
            for_path(Path::new("x.jsx")).map(|g| g.id),
            Some("javascript")
        );
        assert!(for_path(Path::new("notes.txt")).is_none());
        assert!(for_path(Path::new("Makefile")).is_none());
    }

    #[test]
    fn every_bundled_tags_query_compiles_against_its_grammar() {
        for g in all() {
            tree_sitter::Query::new(&g.language(), g.tags_query)
                .unwrap_or_else(|e| panic!("{}: tags query does not compile: {e}", g.id));
        }
    }
}
