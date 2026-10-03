//! Use cases: read one function (cache first, then the explainer), and list
//! the readings a file already has in the cache.

use std::fmt;
use std::path::Path;

use crt_domain::{Function, LanguageId, Reading};

use crate::analyze_file::{AnalyzeError, FileAnalysis, analyze_file};
use crate::ports::{
    Caller, ExplainError, ExplainRequest, Explainer, ReadingKey, ReadingStore, StructureSource,
};

/// Which function to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FunctionSelector {
    /// The first function with this name.
    Name(String),
    /// The innermost function whose lines include this 1-based line.
    Line(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadOptions {
    /// Skip the cache lookup (the result is still stored).
    pub refresh: bool,
    /// How many same-file callers to send as context.
    pub max_callers: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            refresh: false,
            max_callers: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionReading {
    pub language: LanguageId,
    pub function: Function,
    pub reading: Reading,
    pub from_cache: bool,
    pub has_syntax_error: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum ReadError {
    Analyze(AnalyzeError),
    NoSuchFunction(FunctionSelector),
    Explain(ExplainError),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Analyze(_) => write!(f, "could not analyse the file"),
            Self::NoSuchFunction(FunctionSelector::Name(n)) => write!(f, "no function named {n}"),
            Self::NoSuchFunction(FunctionSelector::Line(l)) => write!(f, "no function at line {l}"),
            Self::Explain(_) => write!(f, "could not explain the function"),
        }
    }
}

impl std::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Analyze(e) => Some(e),
            Self::Explain(e) => Some(e),
            Self::NoSuchFunction(_) => None,
        }
    }
}

impl From<AnalyzeError> for ReadError {
    fn from(e: AnalyzeError) -> Self {
        Self::Analyze(e)
    }
}

/// The ports `read_function` works through.
pub struct Readers<'a> {
    pub structure: &'a dyn StructureSource,
    pub explainer: &'a dyn Explainer,
    pub store: &'a dyn ReadingStore,
}

/// Reads one function: from the cache when its source is unchanged,
/// otherwise from the explainer, checked against the function's facts and
/// stored for next time.
pub fn read_function(
    readers: &Readers<'_>,
    path: &Path,
    source: &[u8],
    selector: &FunctionSelector,
    options: ReadOptions,
) -> Result<FunctionReading, ReadError> {
    let analysis = analyze_file(readers.structure, path, source)?;
    let function = select(&analysis, selector)
        .ok_or_else(|| ReadError::NoSuchFunction(selector.clone()))?
        .clone();
    let mut warnings = Vec::new();

    let key = ReadingKey {
        language: analysis.language.clone(),
        function_hash: function.hash,
        author: readers.explainer.author(),
    };
    if !options.refresh {
        match readers.store.get(&key) {
            Ok(Some(reading)) => {
                return Ok(FunctionReading {
                    language: analysis.language,
                    function,
                    reading,
                    from_cache: true,
                    has_syntax_error: analysis.has_syntax_error,
                    warnings,
                });
            }
            Ok(None) => {}
            Err(e) => warnings.push(e.to_string()),
        }
    }

    let request = ExplainRequest {
        language: analysis.language.clone(),
        numbered_source: numbered(source, &function),
        callers: callers_of(&analysis, &function, source, options.max_callers),
        function: function.clone(),
    };
    let explained = readers
        .explainer
        .explain(&request)
        .map_err(ReadError::Explain)?;
    warnings.extend(explained.warnings);
    let reading = Reading::check(&function, explained.author.clone(), explained.draft);

    // Stored under the author that actually wrote it, so a fallback's
    // reading is never served later as the primary model's.
    let stored_key = ReadingKey {
        author: explained.author,
        ..key
    };
    if let Err(e) = readers.store.put(&stored_key, &reading) {
        warnings.push(e.to_string());
    }
    Ok(FunctionReading {
        language: analysis.language,
        function,
        reading,
        from_cache: false,
        has_syntax_error: analysis.has_syntax_error,
        warnings,
    })
}

/// Every function in the file with its cached reading, if any. Never calls
/// the explainer, so it answers in milliseconds when a file is opened.
pub fn cached_readings(
    structure: &dyn StructureSource,
    explainer: &dyn Explainer,
    store: &dyn ReadingStore,
    path: &Path,
    source: &[u8],
) -> Result<(FileAnalysis, Vec<Option<Reading>>), AnalyzeError> {
    let analysis = analyze_file(structure, path, source)?;
    let author = explainer.author();
    let readings = analysis
        .functions
        .iter()
        .map(|f| {
            let key = ReadingKey {
                language: analysis.language.clone(),
                function_hash: f.hash,
                author: author.clone(),
            };
            store.get(&key).ok().flatten()
        })
        .collect();
    Ok((analysis, readings))
}

fn select<'a>(analysis: &'a FileAnalysis, selector: &FunctionSelector) -> Option<&'a Function> {
    match selector {
        FunctionSelector::Name(name) => analysis.functions.iter().find(|f| &f.name == name),
        FunctionSelector::Line(line) => analysis
            .functions
            .iter()
            .filter(|f| f.span.start_line <= *line && *line <= f.span.end_line)
            .min_by_key(|f| f.span.len()),
    }
}

/// `function`'s source with each line prefixed by its 1-based number.
fn numbered(source: &[u8], function: &Function) -> String {
    let text = String::from_utf8_lossy(&source[function.span.start_byte..function.span.end_byte]);
    // The span starts at the definition, which may be mid-line; numbering
    // starts at that line either way.
    text.lines()
        .enumerate()
        .map(|(i, l)| format!("{:>5} | {l}", function.span.start_line + i))
        .collect::<Vec<_>>()
        .join("\n")
}

fn callers_of(
    analysis: &FileAnalysis,
    function: &Function,
    source: &[u8],
    max: usize,
) -> Vec<Caller> {
    analysis
        .functions
        .iter()
        .filter(|f| f.span != function.span)
        .filter(|f| {
            f.lines
                .iter()
                .any(|l| l.calls.iter().any(|c| c.name == function.name))
        })
        .take(max)
        .map(|f| Caller {
            name: f.name.clone(),
            enclosing: f.enclosing.clone(),
            start_line: f.span.start_line,
            numbered_source: numbered(source, f),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{Explained, StoreError, Structure, StructureError};
    use crt_domain::{Author, Basis, Draft, FactRef, Note, Span, Symbol, SymbolKind};
    use std::cell::RefCell;
    use std::collections::HashMap;

    // Source: line 1 `fn g() { f() }`, line 2 `fn f() {`, line 3 ` x()`, line 4 `}`.
    const SRC: &[u8] = b"fn g() { f() }\nfn f() {\n x()\n}\n";

    struct Fake;
    impl StructureSource for Fake {
        fn language_for(&self, _: &Path) -> Option<LanguageId> {
            Some(LanguageId::new("x"))
        }
        fn structure(&self, _: &LanguageId, _: &[u8]) -> Result<Structure, StructureError> {
            let s = |name: &str, kind, def, b: (usize, usize), l: (usize, usize)| Symbol {
                name: String::from(name),
                kind,
                is_definition: def,
                span: Span {
                    start_byte: b.0,
                    end_byte: b.1,
                    start_line: l.0,
                    end_line: l.1,
                },
                name_line: l.0,
                owner: None,
                docs: None,
            };
            Ok(Structure {
                symbols: vec![
                    s("g", SymbolKind::Function, true, (0, 14), (1, 1)),
                    s("f", SymbolKind::Call, false, (9, 12), (1, 1)),
                    s("f", SymbolKind::Function, true, (15, 30), (2, 4)),
                    s("x", SymbolKind::Call, false, (25, 28), (3, 3)),
                ],
                has_syntax_error: false,
            })
        }
    }

    struct Scripted {
        calls: RefCell<Vec<ExplainRequest>>,
        actual_model: &'static str,
    }
    impl Explainer for Scripted {
        fn author(&self) -> Author {
            Author {
                model: "primary".into(),
                prompt: "v1".into(),
            }
        }
        fn explain(&self, r: &ExplainRequest) -> Result<Explained, ExplainError> {
            self.calls.borrow_mut().push(r.clone());
            Ok(Explained {
                draft: Draft {
                    notes: vec![
                        Note {
                            line: 3,
                            text: "calls x".into(),
                            detail: None,
                            assumptions: vec![],
                            basis: Basis::Fact(FactRef::Call { name: "x".into() }),
                        },
                        Note {
                            line: 2,
                            text: "claims a call that is not there".into(),
                            detail: None,
                            assumptions: vec![],
                            basis: Basis::Fact(FactRef::Call { name: "y".into() }),
                        },
                    ],
                    scenarios: vec![],
                },
                author: Author {
                    model: self.actual_model.into(),
                    prompt: "v1".into(),
                },
                warnings: vec![],
            })
        }
    }

    #[derive(Default)]
    struct Memory(RefCell<HashMap<ReadingKey, Reading>>);
    impl ReadingStore for Memory {
        fn get(&self, k: &ReadingKey) -> Result<Option<Reading>, StoreError> {
            Ok(self.0.borrow().get(k).cloned())
        }
        fn put(&self, k: &ReadingKey, r: &Reading) -> Result<(), StoreError> {
            self.0.borrow_mut().insert(k.clone(), r.clone());
            Ok(())
        }
    }

    fn scripted(model: &'static str) -> Scripted {
        Scripted {
            calls: RefCell::new(vec![]),
            actual_model: model,
        }
    }

    #[test]
    fn first_read_explains_checks_and_stores_second_read_hits_the_cache() {
        let explainer = scripted("primary");
        let store = Memory::default();
        let readers = Readers {
            structure: &Fake,
            explainer: &explainer,
            store: &store,
        };
        let sel = FunctionSelector::Name("f".into());

        let first = read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &sel,
            ReadOptions::default(),
        )
        .unwrap();
        assert!(!first.from_cache);
        assert_eq!(first.reading.check.demoted, 1);
        assert!(matches!(first.reading.notes[1].basis, Basis::Fact(_)));

        let req = &explainer.calls.borrow()[0];
        assert!(req.numbered_source.starts_with("    2 | fn f() {"));
        assert_eq!(req.callers.len(), 1);
        assert_eq!(req.callers[0].name, "g");

        let second = read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &sel,
            ReadOptions::default(),
        )
        .unwrap();
        assert!(second.from_cache);
        assert_eq!(explainer.calls.borrow().len(), 1);
        assert_eq!(second.reading, first.reading);
    }

    #[test]
    fn a_fallback_reading_is_not_served_as_the_primary_one() {
        let explainer = scripted("fallback");
        let store = Memory::default();
        let readers = Readers {
            structure: &Fake,
            explainer: &explainer,
            store: &store,
        };
        let sel = FunctionSelector::Line(3);
        read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &sel,
            ReadOptions::default(),
        )
        .unwrap();
        let again = read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &sel,
            ReadOptions::default(),
        )
        .unwrap();
        assert!(!again.from_cache);
        assert_eq!(explainer.calls.borrow().len(), 2);
    }

    #[test]
    fn cached_readings_never_call_the_explainer() {
        let explainer = scripted("primary");
        let store = Memory::default();
        let readers = Readers {
            structure: &Fake,
            explainer: &explainer,
            store: &store,
        };
        read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &FunctionSelector::Name("f".into()),
            ReadOptions::default(),
        )
        .unwrap();
        let (analysis, readings) =
            cached_readings(&Fake, &explainer, &store, Path::new("a.x"), SRC).unwrap();
        assert_eq!(analysis.functions.len(), 2);
        assert!(readings[0].is_none());
        assert!(readings[1].is_some());
        assert_eq!(explainer.calls.borrow().len(), 1);
    }

    #[test]
    fn an_unknown_function_is_a_clear_error() {
        let explainer = scripted("primary");
        let store = Memory::default();
        let readers = Readers {
            structure: &Fake,
            explainer: &explainer,
            store: &store,
        };
        let err = read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &FunctionSelector::Line(99),
            ReadOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "no function at line 99");
    }
}
