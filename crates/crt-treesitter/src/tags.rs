//! Runs a grammar's tags query (and owner query) and translates the result
//! into domain symbols, sorted into file order.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crt_app::{Structure, StructureError};
use crt_domain::{Span, Symbol, SymbolKind};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};
use tree_sitter_tags::{TagsConfiguration, TagsContext};

use crate::grammars::Grammar;
use crate::lines::LineIndex;

pub(crate) fn extract(
    grammar: &'static Grammar,
    source: &[u8],
) -> Result<Structure, StructureError> {
    let config = tags_config(grammar)?;
    let mut context = TagsContext::new();
    let (tags, has_syntax_error) = context
        .generate_tags(&config, source, None)
        .map_err(|e| StructureError::Parse(e.to_string()))?;
    let lines = LineIndex::new(source);

    let mut symbols = Vec::new();
    for tag in tags {
        let tag = tag.map_err(|e| StructureError::Parse(e.to_string()))?;
        symbols.push(Symbol {
            // Name ranges fall on node boundaries, so this is lossless for
            // valid UTF-8 input; invalid bytes become U+FFFD in the name
            // while the hash stays over the raw bytes.
            name: String::from_utf8_lossy(&source[tag.name_range.clone()]).into_owned(),
            kind: kind_from(config.syntax_type_name(tag.syntax_type_id)),
            is_definition: tag.is_definition,
            span: Span {
                start_byte: tag.range.start,
                end_byte: tag.range.end,
                start_line: lines.line_of(tag.range.start),
                end_line: lines.end_line_of(tag.range.start, tag.range.end),
            },
            name_line: lines.line_of(tag.name_range.start),
            owner: None,
            docs: tag.docs,
        });
    }
    // tree-sitter-tags orders by name position, which is not a documented
    // contract; the port promises file order, so enforce it here.
    symbols.sort_by_key(|s| (s.span.start_byte, s.span.end_byte));
    if let Some(query) = grammar.owner_query {
        assign_owners(grammar, query, source, &mut symbols)?;
    }
    Ok(Structure {
        symbols,
        has_syntax_error,
    })
}

/// Compiling the tags query is the expensive part; do it once per grammar.
fn tags_config(grammar: &'static Grammar) -> Result<Arc<TagsConfiguration>, StructureError> {
    static CONFIGS: OnceLock<Mutex<HashMap<&'static str, Arc<TagsConfiguration>>>> =
        OnceLock::new();
    let configs = CONFIGS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut configs = configs
        .lock()
        .map_err(|_| StructureError::Parse("config cache poisoned".into()))?;
    if let Some(c) = configs.get(grammar.id) {
        return Ok(Arc::clone(c));
    }
    let config = TagsConfiguration::new(grammar.language(), grammar.tags_query, "")
        .map_err(|e| StructureError::Parse(format!("{}: tags query: {e}", grammar.id)))?;
    let config = Arc::new(config);
    configs.insert(grammar.id, Arc::clone(&config));
    Ok(config)
}

/// Runs the owner query and sets `owner` on the method symbols it names.
fn assign_owners(
    grammar: &Grammar,
    query_src: &str,
    source: &[u8],
    symbols: &mut [Symbol],
) -> Result<(), StructureError> {
    let language = grammar.language();
    let query = Query::new(&language, query_src)
        .map_err(|e| StructureError::Parse(format!("{}: owner query: {e}", grammar.id)))?;
    let method_idx = capture_index(&query, "method", grammar.id)?;
    let owner_idx = capture_index(&query, "owner", grammar.id)?;

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|e| StructureError::Parse(e.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| StructureError::Parse("parser was cancelled".into()))?;

    let mut owners: HashMap<usize, String> = HashMap::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        let method = m.nodes_for_capture_index(method_idx).next();
        let owner = m.nodes_for_capture_index(owner_idx).next();
        if let (Some(method), Some(owner)) = (method, owner) {
            let name = String::from_utf8_lossy(&source[owner.byte_range()]).into_owned();
            owners.insert(method.start_byte(), name);
        }
    }
    for s in symbols.iter_mut().filter(|s| s.kind == SymbolKind::Method) {
        if let Some(owner) = owners.get(&s.span.start_byte) {
            s.owner = Some(owner.clone());
        }
    }
    Ok(())
}

fn capture_index(query: &Query, name: &str, grammar_id: &str) -> Result<u32, StructureError> {
    query
        .capture_index_for_name(name)
        .ok_or_else(|| StructureError::Parse(format!("{grammar_id}: owner query lacks @{name}")))
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

    fn extract_for(id: &str, src: &str) -> Structure {
        extract(grammars::by_id(id).unwrap(), src.as_bytes()).unwrap()
    }

    fn find<'a>(s: &'a Structure, name: &str) -> &'a Symbol {
        s.symbols
            .iter()
            .find(|x| x.name == name && x.is_definition)
            .unwrap()
    }

    #[test]
    fn go_definitions_span_their_whole_body_and_methods_know_their_receiver() {
        let src = "package p\n\ntype C struct{}\n\nfunc (c *C) Copy() *C {\n\treturn helper(c)\n}\n\nfunc helper(c *C) *C { return c }\n";
        let s = extract_for("go", src);
        assert!(!s.has_syntax_error);
        let defs: Vec<_> = s
            .symbols
            .iter()
            .filter(|x| x.is_definition)
            .map(|x| {
                (
                    x.kind.clone(),
                    x.name.as_str(),
                    x.span.start_line,
                    x.span.end_line,
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
        assert_eq!(find(&s, "Copy").owner.as_deref(), Some("C"));
        assert_eq!(find(&s, "helper").owner, None);
        let calls: Vec<_> = s
            .symbols
            .iter()
            .filter(|x| x.is_call())
            .map(|x| (x.name.as_str(), x.name_line))
            .collect();
        assert_eq!(calls, vec![("helper", 6)]);
    }

    #[test]
    fn rust_impl_methods_know_their_type_and_generic_impls_too() {
        let src = "struct S;\nimpl S {\n    fn m(&self) { f(); }\n}\nimpl<T> Wrap<T> {\n    fn w(&self) {}\n}\nfn f() {}\n";
        let s = extract_for("rust", src);
        assert_eq!(find(&s, "m").owner.as_deref(), Some("S"));
        assert_eq!(find(&s, "w").owner.as_deref(), Some("Wrap"));
        assert_eq!(find(&s, "f").owner, None);
        assert_eq!(find(&s, "m").kind, SymbolKind::Method);
    }

    #[test]
    fn python_method_sits_inside_its_class() {
        let s = extract_for(
            "python",
            "class A:\n    def run(self):\n        return go()\n",
        );
        let class = find(&s, "A");
        let method = find(&s, "run");
        assert_eq!(class.kind, SymbolKind::Type);
        assert!(class.span.contains(&method.span));
        assert_eq!((method.span.start_line, method.span.end_line), (2, 3));
    }

    #[test]
    fn javascript_class_method_and_multiline_call_chain() {
        let src =
            "class A {\n  run() {\n    return this\n      .first()\n      .second();\n  }\n}\n";
        let s = extract_for("javascript", src);
        assert!(find(&s, "A").span.contains(&find(&s, "run").span));
        let calls: Vec<_> = s
            .symbols
            .iter()
            .filter(|x| x.is_call())
            .map(|x| (x.name.as_str(), x.name_line))
            .collect();
        assert_eq!(calls, vec![("first", 4), ("second", 5)]);
    }

    #[test]
    fn symbols_come_out_in_file_order() {
        let s = extract_for("go", "package p\nfunc a() { z() }\nfunc b() { y() }\n");
        let starts: Vec<_> = s.symbols.iter().map(|x| x.span.start_byte).collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted);
    }

    #[test]
    fn syntax_errors_are_reported_not_hidden() {
        let s = extract_for("rust", "fn f( {\n");
        assert!(s.has_syntax_error);
    }
}
