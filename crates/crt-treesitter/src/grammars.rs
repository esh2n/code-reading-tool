//! The bundled grammars. Adding a language is one entry here: the grammar
//! crate, the tags query it ships, and optionally an owner query. No other
//! code changes.

use std::path::Path;

use tree_sitter::Language;

pub(crate) struct Grammar {
    /// Stable id, also the domain `LanguageId`.
    pub(crate) id: &'static str,
    /// File extensions without the dot.
    pub(crate) extensions: &'static [&'static str],
    language: fn() -> Language,
    /// Definitions and references, as shipped with the grammar.
    pub(crate) tags_query: &'static str,
    /// Which type a method belongs to, for languages that state it outside
    /// the type's body (Go receivers, Rust `impl` blocks). Each match binds
    /// `@method` to the method node and `@owner` to the type name.
    pub(crate) owner_query: Option<&'static str>,
}

impl Grammar {
    pub(crate) fn language(&self) -> Language {
        (self.language)()
    }
}

const GO_OWNER: &str = r"
(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [(type_identifier) @owner
             (pointer_type (type_identifier) @owner)]))) @method
";

const RUST_OWNER: &str = r"
(impl_item
  type: [(type_identifier) @owner
         (generic_type type: (type_identifier) @owner)]
  body: (declaration_list (function_item) @method))
";

const GRAMMARS: &[Grammar] = &[
    Grammar {
        id: "rust",
        extensions: &["rs"],
        language: || tree_sitter_rust::LANGUAGE.into(),
        tags_query: tree_sitter_rust::TAGS_QUERY,
        owner_query: Some(RUST_OWNER),
    },
    Grammar {
        id: "go",
        extensions: &["go"],
        language: || tree_sitter_go::LANGUAGE.into(),
        tags_query: tree_sitter_go::TAGS_QUERY,
        owner_query: Some(GO_OWNER),
    },
    Grammar {
        id: "python",
        extensions: &["py"],
        language: || tree_sitter_python::LANGUAGE.into(),
        tags_query: tree_sitter_python::TAGS_QUERY,
        owner_query: None,
    },
    Grammar {
        id: "javascript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        language: || tree_sitter_javascript::LANGUAGE.into(),
        tags_query: tree_sitter_javascript::TAGS_QUERY,
        owner_query: None,
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
    fn every_bundled_query_compiles_against_its_grammar() {
        for g in all() {
            let lang = g.language();
            tree_sitter::Query::new(&lang, g.tags_query)
                .unwrap_or_else(|e| panic!("{}: tags query does not compile: {e}", g.id));
            if let Some(q) = g.owner_query {
                tree_sitter::Query::new(&lang, q)
                    .unwrap_or_else(|e| panic!("{}: owner query does not compile: {e}", g.id));
            }
        }
    }
}
