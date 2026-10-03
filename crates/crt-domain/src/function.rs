//! A function and the structural facts the grammar can state about it.
//! Every value here is a fact, not a guess: it was read off the syntax tree.

use crate::hash::ContentHash;
use crate::symbol::{Span, Symbol, SymbolKind};

/// A call found on one line of a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub name: String,
    /// True when a definition with this name exists in the same file.
    /// A name match, not a resolved call: it can point at the wrong thing.
    pub defined_in_file: bool,
}

/// Facts for one source line. Lines without facts are not listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineFacts {
    pub line: usize,
    pub calls: Vec<Call>,
}

/// A function or method with its structural facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub kind: SymbolKind,
    /// The innermost type-like definition that contains a method.
    pub enclosing: Option<String>,
    pub span: Span,
    /// Identity of the function's source bytes; the cache key component.
    pub hash: ContentHash,
    pub docs: Option<String>,
    pub lines: Vec<LineFacts>,
}

impl Function {
    /// Derives every function in `symbols`, in file order.
    pub fn derive_all(source: &[u8], symbols: &[Symbol]) -> Vec<Function> {
        symbols
            .iter()
            .filter(|s| s.is_function_like())
            .map(|f| Self::derive(source, symbols, f))
            .collect()
    }

    fn derive(source: &[u8], symbols: &[Symbol], func: &Symbol) -> Function {
        let span = func.span;
        Function {
            name: func.name.clone(),
            kind: func.kind.clone(),
            enclosing: enclosing_type(symbols, func).map(|s| s.name.clone()),
            span,
            hash: ContentHash::of(&source[span.start_byte..span.end_byte]),
            docs: func.docs.clone(),
            lines: calls_by_line(symbols, func),
        }
    }
}

/// The smallest type-like definition that strictly contains `func`.
fn enclosing_type<'a>(symbols: &'a [Symbol], func: &Symbol) -> Option<&'a Symbol> {
    symbols
        .iter()
        .filter(|s| s.is_type_like() && s.span.contains(&func.span) && s.span != func.span)
        .min_by_key(|s| s.span.len())
}

fn calls_by_line(symbols: &[Symbol], func: &Symbol) -> Vec<LineFacts> {
    let mut lines: Vec<LineFacts> = Vec::new();
    for call in symbols
        .iter()
        .filter(|s| s.is_call() && func.span.contains(&s.span))
    {
        let fact = Call {
            name: call.name.clone(),
            defined_in_file: symbols
                .iter()
                .any(|s| s.is_definition && s.name == call.name),
        };
        let line = call.span.start_line;
        match lines.last_mut() {
            Some(last) if last.line == line => last.calls.push(fact),
            _ => lines.push(LineFacts {
                line,
                calls: vec![fact],
            }),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(
        name: &str,
        kind: SymbolKind,
        def: bool,
        bytes: (usize, usize),
        lines: (usize, usize),
    ) -> Symbol {
        Symbol {
            name: name.into(),
            kind,
            is_definition: def,
            span: Span {
                start_byte: bytes.0,
                end_byte: bytes.1,
                start_line: lines.0,
                end_line: lines.1,
            },
            docs: None,
        }
    }

    #[test]
    fn method_takes_the_innermost_enclosing_type_and_groups_calls_by_line() {
        let source = b"class Outer { class Inner { fn m() { a(); b(); c(); } } } fn a() {}";
        let symbols = vec![
            sym("Outer", SymbolKind::Type, true, (0, 57), (1, 1)),
            sym("Inner", SymbolKind::Type, true, (14, 55), (1, 1)),
            sym("m", SymbolKind::Method, true, (28, 53), (1, 2)),
            sym("a", SymbolKind::Call, false, (37, 40), (1, 1)),
            sym("b", SymbolKind::Call, false, (42, 45), (1, 1)),
            sym("c", SymbolKind::Call, false, (47, 50), (2, 2)),
            sym("a", SymbolKind::Function, true, (59, 67), (3, 3)),
        ];
        let functions = Function::derive_all(source, &symbols);
        assert_eq!(functions.len(), 2);
        let m = &functions[0];
        assert_eq!(m.enclosing.as_deref(), Some("Inner"));
        assert_eq!(m.lines.len(), 2);
        assert_eq!(m.lines[0].line, 1);
        assert_eq!(
            m.lines[0].calls,
            vec![
                Call {
                    name: "a".into(),
                    defined_in_file: true
                },
                Call {
                    name: "b".into(),
                    defined_in_file: false
                }
            ]
        );
        assert_eq!(m.lines[1].calls[0].name, "c");
        assert!(functions[1].enclosing.is_none());
        assert_eq!(m.hash, ContentHash::of(&source[28..53]));
    }

    #[test]
    fn calls_outside_the_function_are_not_counted() {
        let source = b"fn f() {} g();";
        let symbols = vec![
            sym("f", SymbolKind::Function, true, (0, 9), (1, 1)),
            sym("g", SymbolKind::Call, false, (10, 13), (1, 1)),
        ];
        let f = &Function::derive_all(source, &symbols)[0];
        assert!(f.lines.is_empty());
    }
}
