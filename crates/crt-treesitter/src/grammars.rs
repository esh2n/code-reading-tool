//! The bundled grammars. Adding a language is one entry here: the grammar
//! crate, its tags query (upstream, ours from `queries/<id>/`, or both), and
//! optionally an owner query. No other code changes.

use std::path::Path;

use tree_sitter::Language;

pub(crate) struct Grammar {
    /// Stable id, also the domain `LanguageId`.
    pub(crate) id: &'static str,
    /// File extensions without the dot.
    pub(crate) extensions: &'static [&'static str],
    language: fn() -> Language,
    /// Pieces of the tags query (definitions and references), joined in
    /// order. Upstream queries are used as shipped where they fit; where
    /// they miss what a reader needs, ours replace or extend them.
    pub(crate) tags: &'static [&'static str],
    /// Which type a method belongs to, for languages that state it outside
    /// the type's body (Go receivers, Rust `impl`, C++ `Type::method`).
    /// Each match binds `@method` to the method node and `@owner` to the
    /// type name.
    pub(crate) owner_query: Option<&'static str>,
    /// Local-variable scopes, for grammars whose tags query needs them to
    /// tell a call from a variable (Ruby: `x` may be either).
    pub(crate) locals_query: &'static str,
}

impl Grammar {
    pub(crate) fn language(&self) -> Language {
        (self.language)()
    }

    pub(crate) fn tags_query(&self) -> String {
        self.tags.join("\n")
    }
}

const GRAMMARS: &[Grammar] = &[
    Grammar {
        id: "rust",
        extensions: &["rs"],
        language: || tree_sitter_rust::LANGUAGE.into(),
        tags: &[tree_sitter_rust::TAGS_QUERY],
        owner_query: Some(include_str!("../queries/rust/owner.scm")),
        locals_query: "",
    },
    Grammar {
        id: "go",
        extensions: &["go"],
        language: || tree_sitter_go::LANGUAGE.into(),
        tags: &[tree_sitter_go::TAGS_QUERY],
        owner_query: Some(include_str!("../queries/go/owner.scm")),
        locals_query: "",
    },
    Grammar {
        id: "python",
        extensions: &["py", "pyi"],
        language: || tree_sitter_python::LANGUAGE.into(),
        tags: &[tree_sitter_python::TAGS_QUERY],
        owner_query: None,
        locals_query: "",
    },
    Grammar {
        id: "javascript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        language: || tree_sitter_javascript::LANGUAGE.into(),
        tags: &[tree_sitter_javascript::TAGS_QUERY],
        owner_query: None,
        locals_query: "",
    },
    Grammar {
        id: "typescript",
        extensions: &["ts", "mts", "cts"],
        language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        tags: &[include_str!("../queries/typescript/tags.scm")],
        owner_query: None,
        locals_query: "",
    },
    Grammar {
        id: "tsx",
        extensions: &["tsx"],
        language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        tags: &[include_str!("../queries/typescript/tags.scm")],
        owner_query: None,
        locals_query: "",
    },
    Grammar {
        id: "java",
        extensions: &["java"],
        language: || tree_sitter_java::LANGUAGE.into(),
        tags: &[tree_sitter_java::TAGS_QUERY],
        owner_query: None,
        locals_query: "",
    },
    Grammar {
        id: "c",
        extensions: &["c", "h"],
        language: || tree_sitter_c::LANGUAGE.into(),
        tags: &[include_str!("../queries/c/tags.scm")],
        owner_query: None,
        locals_query: "",
    },
    Grammar {
        id: "cpp",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        language: || tree_sitter_cpp::LANGUAGE.into(),
        tags: &[include_str!("../queries/cpp/tags.scm")],
        owner_query: Some(include_str!("../queries/cpp/owner.scm")),
        locals_query: "",
    },
    Grammar {
        id: "csharp",
        extensions: &["cs"],
        language: || tree_sitter_c_sharp::LANGUAGE.into(),
        tags: &[include_str!("../queries/csharp/tags.scm")],
        owner_query: None,
        locals_query: "",
    },
    Grammar {
        id: "ruby",
        extensions: &["rb"],
        language: || tree_sitter_ruby::LANGUAGE.into(),
        tags: &[tree_sitter_ruby::TAGS_QUERY],
        owner_query: None,
        locals_query: tree_sitter_ruby::LOCALS_QUERY,
    },
    Grammar {
        id: "php",
        extensions: &["php"],
        language: || tree_sitter_php::LANGUAGE_PHP.into(),
        tags: &[
            tree_sitter_php::TAGS_QUERY,
            include_str!("../queries/php/calls.scm"),
        ],
        owner_query: None,
        locals_query: "",
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
        assert_eq!(for_path(Path::new("x.tsx")).map(|g| g.id), Some("tsx"));
        assert_eq!(for_path(Path::new("x.hpp")).map(|g| g.id), Some("cpp"));
        assert_eq!(for_path(Path::new("x.cs")).map(|g| g.id), Some("csharp"));
        assert!(for_path(Path::new("notes.txt")).is_none());
        assert!(for_path(Path::new("Makefile")).is_none());
    }

    #[test]
    fn ids_and_extensions_are_unique() {
        let mut ids: Vec<_> = all().iter().map(|g| g.id).collect();
        let mut exts: Vec<_> = all()
            .iter()
            .flat_map(|g| g.extensions.iter().copied())
            .collect();
        let (n_ids, n_exts) = (ids.len(), exts.len());
        ids.sort_unstable();
        ids.dedup();
        exts.sort_unstable();
        exts.dedup();
        assert_eq!((ids.len(), exts.len()), (n_ids, n_exts));
    }

    #[test]
    fn every_bundled_query_compiles_against_its_grammar() {
        for g in all() {
            let lang = g.language();
            // Building the tags configuration also checks the capture names
            // tree-sitter-tags accepts, which a plain Query does not.
            tree_sitter_tags::TagsConfiguration::new(lang.clone(), &g.tags_query(), g.locals_query)
                .unwrap_or_else(|e| panic!("{}: tags query is not usable: {e}", g.id));
            if let Some(q) = g.owner_query {
                tree_sitter::Query::new(&lang, q)
                    .unwrap_or_else(|e| panic!("{}: owner query does not compile: {e}", g.id));
            }
        }
    }
}
