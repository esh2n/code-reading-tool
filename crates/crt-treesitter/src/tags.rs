//! Runs a grammar's tags query and translates the result into domain symbols.

use crt_app::StructureError;
use crt_domain::{Span, Symbol, SymbolKind};
use tree_sitter_tags::{TagsConfiguration, TagsContext};

use crate::grammars::Grammar;
use crate::lines::LineIndex;

pub(crate) fn extract(grammar: &Grammar, source: &[u8]) -> Result<Vec<Symbol>, StructureError> {
    let config = TagsConfiguration::new(grammar.language(), grammar.tags_query, "")
        .map_err(|e| StructureError::Parse(format!("{}: tags query: {e}", grammar.id)))?;
    let mut context = TagsContext::new();
    let (tags, _has_syntax_error) = context
        .generate_tags(&config, source, None)
        .map_err(|e| StructureError::Parse(e.to_string()))?;
    let lines = LineIndex::new(source);

    let mut symbols = Vec::new();
    for tag in tags {
        let tag = tag.map_err(|e| StructureError::Parse(e.to_string()))?;
        symbols.push(Symbol {
            name: String::from_utf8_lossy(&source[tag.name_range.clone()]).into_owned(),
            kind: kind_from(config.syntax_type_name(tag.syntax_type_id)),
            is_definition: tag.is_definition,
            span: Span {
                start_byte: tag.range.start,
                end_byte: tag.range.end,
                start_line: lines.line_of(tag.range.start),
                end_line: lines.end_line_of(tag.range.start, tag.range.end),
            },
            docs: tag.docs,
        });
    }
    Ok(symbols)
}

/// Tags queries name kinds slightly differently per grammar (Go says
/// `type` where others say `class`); this is the one place that knows.
fn kind_from(syntax_type: &str) -> SymbolKind {
    match syntax_type {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "class" | "type" => SymbolKind::Type,
        "interface" => SymbolKind::Interface,
        "module" => SymbolKind::Module,
        "call" => SymbolKind::Call,
        other => SymbolKind::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammars;

    fn extract_for(id: &str, src: &str) -> Vec<Symbol> {
        extract(grammars::by_id(id).unwrap(), src.as_bytes()).unwrap()
    }

    #[test]
    fn go_definitions_span_their_whole_body() {
        let src = "package p\n\ntype C struct{}\n\nfunc (c *C) Copy() *C {\n\treturn helper(c)\n}\n\nfunc helper(c *C) *C { return c }\n";
        let symbols = extract_for("go", src);
        let defs: Vec<_> = symbols
            .iter()
            .filter(|s| s.is_definition)
            .map(|s| {
                (
                    s.kind.clone(),
                    s.name.as_str(),
                    s.span.start_line,
                    s.span.end_line,
                )
            })
            .collect();
        assert_eq!(
            defs,
            vec![
                (SymbolKind::Type, "C", 3, 3),
                (SymbolKind::Method, "Copy", 5, 7),
                (SymbolKind::Function, "helper", 9, 9),
            ]
        );
        let calls: Vec<_> = symbols
            .iter()
            .filter(|s| s.is_call())
            .map(|s| (s.name.as_str(), s.span.start_line))
            .collect();
        assert_eq!(calls, vec![("helper", 6)]);
    }

    #[test]
    fn python_method_sits_inside_its_class() {
        let symbols = extract_for(
            "python",
            "class A:\n    def run(self):\n        return go()\n",
        );
        let class = symbols.iter().find(|s| s.name == "A").unwrap();
        let method = symbols.iter().find(|s| s.name == "run").unwrap();
        assert_eq!(class.kind, SymbolKind::Type);
        assert!(class.span.contains(&method.span));
        assert_eq!((method.span.start_line, method.span.end_line), (2, 3));
    }

    #[test]
    fn rust_free_function_and_impl_method() {
        let src = "struct S;\nimpl S {\n    fn m(&self) { f(); }\n}\nfn f() {}\n";
        let symbols = extract_for("rust", src);
        let kinds: Vec<_> = symbols
            .iter()
            .filter(|s| s.is_definition)
            .map(|s| (s.kind.clone(), s.name.as_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (SymbolKind::Type, "S"),
                (SymbolKind::Method, "m"),
                (SymbolKind::Function, "f")
            ]
        );
    }
}
