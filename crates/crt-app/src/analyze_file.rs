//! Use case: list the functions of one file with their structural facts.

use std::fmt;
use std::path::{Path, PathBuf};

use crt_domain::{Function, LanguageId, SpanOutOfRange};

use crate::ports::{StructureError, StructureSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAnalysis {
    pub language: LanguageId,
    /// In file order.
    pub functions: Vec<Function>,
    /// True when the file did not parse cleanly; facts may be incomplete.
    pub has_syntax_error: bool,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum AnalyzeError {
    /// No structure source claims this file.
    UnsupportedFile(PathBuf),
    Structure(StructureError),
    /// The structure source returned a symbol outside the source text.
    BadSymbol(SpanOutOfRange),
}

impl fmt::Display for AnalyzeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFile(p) => write!(f, "no structure source for {}", p.display()),
            Self::Structure(_) => write!(f, "could not read the file's structure"),
            Self::BadSymbol(_) => write!(f, "the structure source returned an invalid symbol"),
        }
    }
}

impl std::error::Error for AnalyzeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnsupportedFile(_) => None,
            Self::Structure(e) => Some(e),
            Self::BadSymbol(e) => Some(e),
        }
    }
}

impl From<StructureError> for AnalyzeError {
    fn from(e: StructureError) -> Self {
        Self::Structure(e)
    }
}

impl From<SpanOutOfRange> for AnalyzeError {
    fn from(e: SpanOutOfRange) -> Self {
        Self::BadSymbol(e)
    }
}

/// Analyses `source`, taking the language from `path`.
pub fn analyze_file(
    structure: &dyn StructureSource,
    path: &Path,
    source: &[u8],
) -> Result<FileAnalysis, AnalyzeError> {
    let language = structure
        .language_for(path)
        .ok_or_else(|| AnalyzeError::UnsupportedFile(path.to_path_buf()))?;
    let found = structure.structure(&language, source)?;
    Ok(FileAnalysis {
        functions: Function::derive_all(source, &found.symbols)?,
        has_syntax_error: found.has_syntax_error,
        language,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Structure;
    use crt_domain::{Span, Symbol, SymbolKind};

    struct Fake {
        end_byte: usize,
    }

    impl StructureSource for Fake {
        fn language_for(&self, path: &Path) -> Option<LanguageId> {
            (path.extension()? == "x").then(|| LanguageId::new("x"))
        }

        fn structure(&self, _: &LanguageId, _: &[u8]) -> Result<Structure, StructureError> {
            Ok(Structure {
                symbols: vec![Symbol {
                    name: "f".into(),
                    kind: SymbolKind::Function,
                    is_definition: true,
                    span: Span {
                        start_byte: 0,
                        end_byte: self.end_byte,
                        start_line: 1,
                        end_line: 1,
                    },
                    name_line: 1,
                    owner: None,
                    docs: None,
                }],
                has_syntax_error: true,
            })
        }
    }

    #[test]
    fn derives_functions_and_carries_the_syntax_error_flag() {
        let out = analyze_file(&Fake { end_byte: 4 }, Path::new("a.x"), b"f(){}").unwrap();
        assert_eq!(out.language, LanguageId::new("x"));
        assert_eq!(out.functions[0].name, "f");
        assert!(out.has_syntax_error);
    }

    #[test]
    fn rejects_a_file_no_source_claims() {
        let err = analyze_file(&Fake { end_byte: 4 }, Path::new("a.txt"), b"").unwrap_err();
        assert!(matches!(err, AnalyzeError::UnsupportedFile(_)));
    }

    #[test]
    fn a_bad_span_from_the_source_is_reported_with_its_cause() {
        let err = analyze_file(&Fake { end_byte: 99 }, Path::new("a.x"), b"f(){}").unwrap_err();
        assert!(matches!(err, AnalyzeError::BadSymbol(_)));
        assert!(std::error::Error::source(&err).is_some());
    }
}
