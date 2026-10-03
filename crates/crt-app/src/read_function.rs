//! Use cases: read one function's notes (cache first, then the explainer),
//! read its scenarios on request, and list the readings a file already has
//! in the cache.

use std::fmt;
use std::path::Path;

use crt_domain::{Author, Function, LanguageId, Note, Reading};

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

/// Reads one function's notes: from the cache when its source is
/// unchanged, otherwise from the explainer, checked against the function's
/// facts and stored for next time. `on_progress` receives the checked notes
/// received so far while the explainer is still writing; each call replaces
/// the previous one.
pub fn read_function(
    readers: &Readers<'_>,
    path: &Path,
    source: &[u8],
    selector: &FunctionSelector,
    options: ReadOptions,
    on_progress: &mut dyn FnMut(&[Note]),
) -> Result<FunctionReading, ReadError> {
    let (analysis, function) = load(readers, path, source, selector)?;
    let notes = notes_of(readers, &analysis, &function, source, options, on_progress)?;
    Ok(finish(
        analysis,
        function,
        notes.reading,
        notes.from_cache,
        notes.warnings,
    ))
}

/// Reads one function's scenarios, reading its notes first when there are
/// none yet. Cached scenarios are returned unless `options.refresh`, which
/// rewrites the scenarios only, never the notes.
pub fn read_scenarios(
    readers: &Readers<'_>,
    path: &Path,
    source: &[u8],
    selector: &FunctionSelector,
    options: ReadOptions,
) -> Result<FunctionReading, ReadError> {
    let (analysis, function) = load(readers, path, source, selector)?;
    let notes_options = ReadOptions {
        refresh: false,
        ..options
    };
    let Notes {
        reading,
        from_cache,
        mut warnings,
    } = notes_of(
        readers,
        &analysis,
        &function,
        source,
        notes_options,
        &mut |_| {},
    )?;
    if reading.scenarios.is_some() && !options.refresh {
        return Ok(finish(analysis, function, reading, from_cache, warnings));
    }

    let request = request_for(&analysis, &function, source, options);
    let explained = readers
        .explainer
        .scenarios(&request)
        .map_err(ReadError::Explain)?;
    warnings.extend(explained.warnings);
    if explained.author != reading.author {
        warnings.push(format!(
            "the scenarios were written by {}, the notes by {}",
            explained.author.model, reading.author.model
        ));
    }
    // Kept with the notes, under the notes' author: one reading per
    // function and author, whoever wrote its scenarios.
    let reading = reading.with_scenarios(&function, explained.scenarios);
    save(
        readers,
        &analysis.language,
        &function,
        &reading,
        &mut warnings,
    );
    Ok(finish(analysis, function, reading, false, warnings))
}

/// A function's notes and where they came from.
struct Notes {
    reading: Reading,
    from_cache: bool,
    warnings: Vec<String>,
}

/// The notes of `function`: from the cache unless `options.refresh`,
/// otherwise from the explainer, checked and stored.
fn notes_of(
    readers: &Readers<'_>,
    analysis: &FileAnalysis,
    function: &Function,
    source: &[u8],
    options: ReadOptions,
    on_progress: &mut dyn FnMut(&[Note]),
) -> Result<Notes, ReadError> {
    let mut warnings = Vec::new();
    if !options.refresh {
        let authors = readers.explainer.authors();
        let found = lookup(
            readers.store,
            &analysis.language,
            function,
            &authors,
            &mut warnings,
        );
        if let Some(reading) = found {
            return Ok(Notes {
                reading,
                from_cache: true,
                warnings,
            });
        }
    }

    let request = request_for(analysis, function, source, options);
    let mut checked_progress = |raw: &[Note]| {
        let (notes, _) = function.check_notes(raw.to_vec());
        on_progress(&notes);
    };
    let explained = readers
        .explainer
        .explain(&request, &mut checked_progress)
        .map_err(ReadError::Explain)?;
    warnings.extend(explained.warnings);
    // Stored under the author that actually wrote it, so a fallback's
    // reading is always labelled as the fallback's.
    let reading = Reading::check(function, explained.author, explained.draft);
    save(
        readers,
        &analysis.language,
        function,
        &reading,
        &mut warnings,
    );
    Ok(Notes {
        reading,
        from_cache: false,
        warnings,
    })
}

fn load(
    readers: &Readers<'_>,
    path: &Path,
    source: &[u8],
    selector: &FunctionSelector,
) -> Result<(FileAnalysis, Function), ReadError> {
    let analysis = analyze_file(readers.structure, path, source)?;
    let function = select(&analysis, selector)
        .ok_or_else(|| ReadError::NoSuchFunction(selector.clone()))?
        .clone();
    Ok((analysis, function))
}

fn request_for(
    analysis: &FileAnalysis,
    function: &Function,
    source: &[u8],
    options: ReadOptions,
) -> ExplainRequest {
    ExplainRequest {
        language: analysis.language.clone(),
        numbered_source: numbered(source, function),
        callers: callers_of(analysis, function, source, options.max_callers),
        function: function.clone(),
    }
}

/// Stores `reading` (file lines) in the relative form the store keeps. A
/// failing store becomes a warning, never an error.
fn save(
    readers: &Readers<'_>,
    language: &LanguageId,
    function: &Function,
    reading: &Reading,
    warnings: &mut Vec<String>,
) {
    let key = ReadingKey {
        language: language.clone(),
        function_hash: function.hash,
        author: reading.author.clone(),
    };
    if let Err(e) = readers
        .store
        .put(&key, &reading.relative_to(function.span.start_line))
    {
        warnings.push(e.to_string());
    }
}

fn finish(
    analysis: FileAnalysis,
    function: Function,
    reading: Reading,
    from_cache: bool,
    warnings: Vec<String>,
) -> FunctionReading {
    FunctionReading {
        language: analysis.language,
        function,
        reading,
        from_cache,
        has_syntax_error: analysis.has_syntax_error,
        warnings,
    }
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
    let authors = explainer.authors();
    let readings = analysis
        .functions
        .iter()
        .map(|f| lookup(store, &analysis.language, f, &authors, &mut Vec::new()))
        .collect();
    Ok((analysis, readings))
}

/// The stored reading for `function` by the first author that has one,
/// placed at the function's current lines. A reading by a fallback author
/// is returned with a warning saying so.
fn lookup(
    store: &dyn ReadingStore,
    language: &LanguageId,
    function: &Function,
    authors: &[Author],
    warnings: &mut Vec<String>,
) -> Option<Reading> {
    for (i, author) in authors.iter().enumerate() {
        let key = ReadingKey {
            language: language.clone(),
            function_hash: function.hash,
            author: author.clone(),
        };
        match store.get(&key) {
            Ok(Some(stored)) => {
                if i > 0 {
                    warnings.push(format!(
                        "cached reading of {} was written by fallback model {}",
                        function.name, author.model
                    ));
                }
                return Some(stored.placed_at(function.span.start_line));
            }
            Ok(None) => {}
            Err(e) => warnings.push(e.to_string()),
        }
    }
    None
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
    use crate::ports::{Explained, ExplainedScenarios, StoreError, Structure, StructureError};
    use crt_domain::{
        Author, Basis, Draft, FactRef, Note, Scenario, ScenarioKind, Span, Step, Symbol, SymbolKind,
    };
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

    /// `Fake`'s file with `n` empty lines inserted at the top.
    struct Shifted(usize);
    impl StructureSource for Shifted {
        fn language_for(&self, p: &Path) -> Option<LanguageId> {
            Fake.language_for(p)
        }
        fn structure(&self, l: &LanguageId, s: &[u8]) -> Result<Structure, StructureError> {
            let mut st = Fake.structure(l, s)?;
            for sym in &mut st.symbols {
                sym.span.start_byte += self.0;
                sym.span.end_byte += self.0;
                sym.span.start_line += self.0;
                sym.span.end_line += self.0;
                sym.name_line += self.0;
            }
            Ok(st)
        }
    }

    struct Scripted {
        calls: RefCell<Vec<ExplainRequest>>,
        scenario_calls: RefCell<Vec<ExplainRequest>>,
        actual_model: &'static str,
    }
    impl Explainer for Scripted {
        fn authors(&self) -> Vec<Author> {
            ["primary", "fallback"]
                .into_iter()
                .map(|m| Author {
                    model: m.into(),
                    prompt: "v1".into(),
                })
                .collect()
        }
        fn explain(
            &self,
            r: &ExplainRequest,
            on_progress: &mut dyn FnMut(&[Note]),
        ) -> Result<Explained, ExplainError> {
            self.calls.borrow_mut().push(r.clone());
            let notes = vec![
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
            ];
            on_progress(&notes[..1]);
            on_progress(&notes);
            Ok(Explained {
                draft: Draft { notes },
                author: self.author(),
                warnings: vec![],
            })
        }
        fn scenarios(&self, r: &ExplainRequest) -> Result<ExplainedScenarios, ExplainError> {
            self.scenario_calls.borrow_mut().push(r.clone());
            Ok(ExplainedScenarios {
                scenarios: vec![Scenario {
                    kind: ScenarioKind::Normal,
                    title: "t".into(),
                    input: "i".into(),
                    steps: vec![
                        Step {
                            line: 3,
                            what: "x runs".into(),
                        },
                        Step {
                            line: 9,
                            what: "outside".into(),
                        },
                    ],
                    outcome: "o".into(),
                    assumptions: vec![],
                }],
                author: self.author(),
                warnings: vec![],
            })
        }
    }

    impl Scripted {
        fn author(&self) -> Author {
            Author {
                model: self.actual_model.into(),
                prompt: "v1".into(),
            }
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
            scenario_calls: RefCell::new(vec![]),
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
            &mut |_| {},
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
            &mut |_| {},
        )
        .unwrap();
        assert!(second.from_cache);
        assert_eq!(explainer.calls.borrow().len(), 1);
        assert_eq!(second.reading, first.reading);
    }

    #[test]
    fn a_fallback_reading_is_found_again_and_labelled() {
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
            &mut |_| {},
        )
        .unwrap();
        let again = read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &sel,
            ReadOptions::default(),
            &mut |_| {},
        )
        .unwrap();
        assert!(again.from_cache);
        assert_eq!(again.reading.author.model, "fallback");
        assert!(
            again
                .warnings
                .iter()
                .any(|w| w.contains("fallback model fallback"))
        );
        assert_eq!(explainer.calls.borrow().len(), 1);
    }

    #[test]
    fn a_reading_follows_its_function_when_lines_are_added_above() {
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
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(first.reading.notes[1].line, 3);

        // Same function bytes, two lines lower.
        let moved = Shifted(2);
        let readers = Readers {
            structure: &moved,
            explainer: &explainer,
            store: &store,
        };
        let src = [b"\n\n".as_slice(), SRC].concat();
        let again = read_function(
            &readers,
            Path::new("a.x"),
            &src,
            &sel,
            ReadOptions::default(),
            &mut |_| {},
        )
        .unwrap();
        assert!(again.from_cache);
        assert_eq!(again.function.span.start_line, 4);
        assert_eq!(
            again.reading.notes[1].line, 5,
            "the note moved with its function"
        );
        assert_eq!(explainer.calls.borrow().len(), 1);
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
            &mut |_| {},
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
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(err.to_string(), "no function at line 99");
    }

    #[test]
    fn progress_carries_checked_notes_as_they_arrive() {
        let explainer = scripted("primary");
        let store = Memory::default();
        let readers = Readers {
            structure: &Fake,
            explainer: &explainer,
            store: &store,
        };
        let mut seen: Vec<Vec<Note>> = vec![];
        read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &FunctionSelector::Name("f".into()),
            ReadOptions::default(),
            &mut |n| seen.push(n.to_vec()),
        )
        .unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].len(), 1);
        assert_eq!(seen[1][0].line, 2, "sorted by line");
        assert_eq!(seen[1][0].basis, Basis::Inference, "already checked");
    }

    #[test]
    fn scenarios_are_written_on_request_once_and_kept_with_the_notes() {
        let explainer = scripted("primary");
        let store = Memory::default();
        let readers = Readers {
            structure: &Fake,
            explainer: &explainer,
            store: &store,
        };
        let sel = FunctionSelector::Name("f".into());
        let lines = read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &sel,
            ReadOptions::default(),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(lines.reading.scenarios, None);
        assert!(explainer.scenario_calls.borrow().is_empty());

        let first = read_scenarios(
            &readers,
            Path::new("a.x"),
            SRC,
            &sel,
            ReadOptions::default(),
        )
        .unwrap();
        let scenarios = first.reading.scenarios.clone().unwrap();
        assert_eq!(scenarios[0].steps.len(), 1, "the step outside was dropped");
        assert_eq!(first.reading.notes, lines.reading.notes);
        assert_eq!(
            explainer.calls.borrow().len(),
            1,
            "notes came from the cache"
        );

        let again = read_scenarios(
            &readers,
            Path::new("a.x"),
            SRC,
            &sel,
            ReadOptions::default(),
        )
        .unwrap();
        assert!(again.from_cache);
        assert_eq!(again.reading, first.reading);
        assert_eq!(explainer.scenario_calls.borrow().len(), 1);

        let notes_again = read_function(
            &readers,
            Path::new("a.x"),
            SRC,
            &sel,
            ReadOptions::default(),
            &mut |_| {},
        )
        .unwrap();
        assert!(notes_again.reading.scenarios.is_some());
    }

    #[test]
    fn scenarios_without_notes_read_the_notes_first() {
        let explainer = scripted("primary");
        let store = Memory::default();
        let readers = Readers {
            structure: &Fake,
            explainer: &explainer,
            store: &store,
        };
        let r = read_scenarios(
            &readers,
            Path::new("a.x"),
            SRC,
            &FunctionSelector::Line(3),
            ReadOptions::default(),
        )
        .unwrap();
        assert_eq!(r.reading.notes.len(), 2);
        assert!(r.reading.scenarios.is_some());
        assert_eq!(explainer.calls.borrow().len(), 1);
        assert_eq!(explainer.scenario_calls.borrow().len(), 1);
    }

    /// `Fake`, counting how often the file is analysed.
    #[derive(Default)]
    struct Counting(std::cell::Cell<usize>);
    impl StructureSource for Counting {
        fn language_for(&self, p: &Path) -> Option<LanguageId> {
            Fake.language_for(p)
        }
        fn structure(&self, l: &LanguageId, s: &[u8]) -> Result<Structure, StructureError> {
            self.0.set(self.0.get() + 1);
            Fake.structure(l, s)
        }
    }

    #[test]
    fn scenarios_analyse_the_file_once() {
        let explainer = scripted("primary");
        let store = Memory::default();
        let counting = Counting::default();
        let readers = Readers {
            structure: &counting,
            explainer: &explainer,
            store: &store,
        };
        read_scenarios(
            &readers,
            Path::new("a.x"),
            SRC,
            &FunctionSelector::Line(3),
            ReadOptions::default(),
        )
        .unwrap();
        assert_eq!(counting.0.get(), 1);
    }
}
