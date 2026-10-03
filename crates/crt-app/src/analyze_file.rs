//! Use case: list the functions of one file with their structural facts.

use std::fmt;
use std::path::{Path, PathBuf};

use crt_domain::{Function, LanguageId};

use crate::ports::{StructureError, StructureSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAnalysis {
    pub language: LanguageId,
    pub functions: Vec<Function>,
}

#[derive(Debug)]
pub enum AnalyzeError {
    /// No structure source claims this file.
    UnsupportedFile(PathBuf),
    Structure(StructureError),
}

impl fmt::Display for AnalyzeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFile(p) => write!(f, "no bundled language for {}", p.display()),
            Self::Structure(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for AnalyzeError {}

impl From<StructureError> for AnalyzeError {
    fn from(e: StructureError) -> Self {
        Self::Structure(e)
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
    let symbols = structure.symbols(&language, source)?;
    Ok(FileAnalysis {
        functions: Function::derive_all(source, &symbols),
        language,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crt_domain::{Span, Symbol, SymbolKind};

    struct Fake;

    impl StructureSource for Fake {
        fn language_for(&self, path: &Path) -> Option<LanguageId> {
            (path.extension()? == "x").then(|| LanguageId::new("x"))
        }

        fn symbols(&self, _: &LanguageId, _: &[u8]) -> Result<Vec<Symbol>, StructureError> {
            Ok(vec![Symbol {
                name: "f".into(),
                kind: SymbolKind::Function,
                is_definition: true,
                span: Span {
                    start_byte: 0,
                    end_byte: 4,
                    start_line: 1,
                    end_line: 1,
                },
                docs: None,
            }])
        }
    }

    #[test]
    fn derives_functions_for_a_known_file() {
        let out = analyze_file(&Fake, Path::new("a.x"), b"f(){}").unwrap();
        assert_eq!(out.language, LanguageId::new("x"));
        assert_eq!(out.functions[0].name, "f");
    }

    #[test]
    fn rejects_a_file_no_source_claims() {
        let err = analyze_file(&Fake, Path::new("a.txt"), b"").unwrap_err();
        assert!(matches!(err, AnalyzeError::UnsupportedFile(_)));
    }
}
