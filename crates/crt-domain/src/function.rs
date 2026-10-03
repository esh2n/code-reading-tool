//! A function and the structural facts the grammar can state about it.
//! Every value here is a fact, not a guess: it was read off the syntax tree.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use crate::hash::ContentHash;
use crate::symbol::{Span, Symbol, SymbolKind};

/// A call found on one line of a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub name: String,
    /// True when a definition with this name exists in the same file.
    /// A name match, not a resolved call: it can point at the wrong thing,
    /// and it is true for a recursive call to the function itself.
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
    /// The type a method belongs to: what the source states (receiver,
    /// `impl` block) or, failing that, the innermost type-like definition
    /// that contains it.
    pub enclosing: Option<String>,
    pub span: Span,
    /// Identity of the function's source bytes; a cache key component.
    pub hash: ContentHash,
    pub docs: Option<String>,
    /// In line order.
    pub lines: Vec<LineFacts>,
}

/// A structure source handed over a symbol that does not fit the source
/// text. The port is a trust boundary, so this is an error, not a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanOutOfRange {
    pub symbol: String,
    pub span: Span,
    pub source_len: usize,
}

impl fmt::Display for SpanOutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "symbol {} has byte span {}..{} outside a source of {} bytes",
            self.symbol, self.span.start_byte, self.span.end_byte, self.source_len
        )
    }
}

impl std::error::Error for SpanOutOfRange {}

impl Function {
    /// Derives every function in `symbols`, in file order. `symbols` may be
    /// in any order.
    pub fn derive_all(source: &[u8], symbols: &[Symbol]) -> Result<Vec<Function>, SpanOutOfRange> {
        let defined: HashSet<&str> = symbols
            .iter()
            .filter(|s| s.is_definition)
            .map(|s| s.name.as_str())
            .collect();
        let mut functions: Vec<&Symbol> = symbols.iter().filter(|s| s.is_function_like()).collect();
        functions.sort_by_key(|s| (s.span.start_byte, s.span.end_byte));
        functions
            .into_iter()
            .map(|f| Self::derive(source, symbols, &defined, f))
            .collect()
    }

    fn derive(
        source: &[u8],
        symbols: &[Symbol],
        defined: &HashSet<&str>,
        func: &Symbol,
    ) -> Result<Function, SpanOutOfRange> {
        let span = func.span;
        if !span.fits(source) {
            return Err(SpanOutOfRange {
                symbol: func.name.clone(),
                span,
                source_len: source.len(),
            });
        }
        Ok(Function {
            name: func.name.clone(),
            kind: func.kind.clone(),
            enclosing: func
                .owner
                .clone()
                .or_else(|| enclosing_type(symbols, func).map(|s| s.name.clone())),
            span,
            hash: ContentHash::of(&source[span.start_byte..span.end_byte]),
            docs: func.docs.clone(),
            lines: calls_by_line(symbols, defined, func),
        })
    }
}

/// The smallest type-like definition that strictly contains `func`.
fn enclosing_type<'a>(symbols: &'a [Symbol], func: &Symbol) -> Option<&'a Symbol> {
    symbols
        .iter()
        .filter(|s| s.is_type_like() && s.span.contains(&func.span) && s.span != func.span)
        .min_by_key(|s| s.span.len())
}

/// Calls inside `func`, grouped by the line of the callee name, in line
/// order regardless of the order of `symbols`.
fn calls_by_line(symbols: &[Symbol], defined: &HashSet<&str>, func: &Symbol) -> Vec<LineFacts> {
    let mut by_line: BTreeMap<usize, Vec<Call>> = BTreeMap::new();
    for call in symbols
        .iter()
        .filter(|s| s.is_call() && func.span.contains(&s.span))
    {
        by_line.entry(call.name_line).or_default().push(Call {
            name: call.name.clone(),
            defined_in_file: defined.contains(call.name.as_str()),
        });
    }
    by_line
        .into_iter()
        .map(|(line, calls)| LineFacts { line, calls })
        .collect()
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
            name_line: lines.0,
            owner: None,
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
        let functions = Function::derive_all(source, &symbols).unwrap();
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
    fn a_stated_owner_wins_over_containment() {
        let source = b"type T struct{}; func (t T) m() { x() }";
        let mut m = sym("m", SymbolKind::Method, true, (17, 39), (1, 1));
        m.owner = Some("T".into());
        let symbols = vec![sym("T", SymbolKind::Type, true, (0, 15), (1, 1)), m];
        let f = &Function::derive_all(source, &symbols).unwrap()[0];
        assert_eq!(f.enclosing.as_deref(), Some("T"));
    }

    #[test]
    fn modules_do_not_count_as_enclosing_types() {
        let source = b"mod m { fn f() {} }";
        let symbols = vec![
            sym("m", SymbolKind::Module, true, (0, 19), (1, 1)),
            sym("f", SymbolKind::Function, true, (8, 17), (1, 1)),
        ];
        assert!(
            Function::derive_all(source, &symbols).unwrap()[0]
                .enclosing
                .is_none()
        );
    }

    #[test]
    fn calls_are_ordered_by_line_even_when_symbols_are_not() {
        let source = b"fn f() {\n b();\n a();\n}";
        let symbols = vec![
            sym("a", SymbolKind::Call, false, (16, 19), (3, 3)),
            sym("f", SymbolKind::Function, true, (0, 22), (1, 4)),
            sym("b", SymbolKind::Call, false, (10, 13), (2, 2)),
        ];
        let f = &Function::derive_all(source, &symbols).unwrap()[0];
        let lines: Vec<_> = f
            .lines
            .iter()
            .map(|l| (l.line, l.calls[0].name.as_str()))
            .collect();
        assert_eq!(lines, vec![(2, "b"), (3, "a")]);
    }

    #[test]
    fn calls_outside_the_function_are_not_counted() {
        let source = b"fn f() {} g();";
        let symbols = vec![
            sym("f", SymbolKind::Function, true, (0, 9), (1, 1)),
            sym("g", SymbolKind::Call, false, (10, 13), (1, 1)),
        ];
        assert!(
            Function::derive_all(source, &symbols).unwrap()[0]
                .lines
                .is_empty()
        );
    }

    #[test]
    fn a_span_past_the_end_of_the_source_is_an_error_not_a_panic() {
        let symbols = vec![sym("f", SymbolKind::Function, true, (0, 99), (1, 1))];
        let err = Function::derive_all(b"fn f() {}", &symbols).unwrap_err();
        assert_eq!(err.symbol, "f");
        assert_eq!(err.source_len, 9);
    }
}
