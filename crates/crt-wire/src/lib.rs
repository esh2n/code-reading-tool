//! The JSON shapes this tool exposes: CLI output, LSP custom-method payloads
//! and the HTML renderer's input all use these. They are versioned apart
//! from the domain types, which carry no serde at all.

use serde::{Deserialize, Serialize};

use crt_app::FileAnalysis;
use crt_domain::{Call, Function, LineFacts, SymbolKind};

/// Bumped whenever a shape below changes incompatibly. Adding a field is
/// not a bump.
pub const WIRE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileAnalysisDto {
    pub wire_version: u32,
    pub language: String,
    /// True when the file did not parse cleanly; facts may be incomplete.
    pub has_syntax_error: bool,
    pub functions: Vec<FunctionDto>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FunctionKindDto {
    Function,
    Method,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionDto {
    pub name: String,
    pub kind: FunctionKindDto,
    pub enclosing: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    /// Hex SHA-256 of the function's source bytes.
    pub hash: String,
    pub docs: Option<String>,
    pub lines: Vec<LineDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineDto {
    pub line: usize,
    pub calls: Vec<CallDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallDto {
    pub name: String,
    pub defined_in_file: bool,
}

impl From<&FileAnalysis> for FileAnalysisDto {
    fn from(a: &FileAnalysis) -> Self {
        Self {
            wire_version: WIRE_VERSION,
            language: a.language.to_string(),
            has_syntax_error: a.has_syntax_error,
            functions: a.functions.iter().map(FunctionDto::from).collect(),
        }
    }
}

impl From<&Function> for FunctionDto {
    fn from(f: &Function) -> Self {
        Self {
            name: f.name.clone(),
            kind: match f.kind {
                SymbolKind::Method => FunctionKindDto::Method,
                _ => FunctionKindDto::Function,
            },
            enclosing: f.enclosing.clone(),
            start_line: f.span.start_line,
            end_line: f.span.end_line,
            hash: f.hash.to_string(),
            docs: f.docs.clone(),
            lines: f.lines.iter().map(LineDto::from).collect(),
        }
    }
}

impl From<&LineFacts> for LineDto {
    fn from(l: &LineFacts) -> Self {
        Self {
            line: l.line,
            calls: l.calls.iter().map(CallDto::from).collect(),
        }
    }
}

impl From<&Call> for CallDto {
    fn from(c: &Call) -> Self {
        Self {
            name: c.name.clone(),
            defined_in_file: c.defined_in_file,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crt_domain::{ContentHash, LanguageId, Span};

    fn sample() -> FileAnalysis {
        FileAnalysis {
            language: LanguageId::new("go"),
            has_syntax_error: false,
            functions: vec![Function {
                name: "Copy".into(),
                kind: SymbolKind::Method,
                enclosing: Some("C".into()),
                span: Span {
                    start_byte: 0,
                    end_byte: 3,
                    start_line: 5,
                    end_line: 7,
                },
                hash: ContentHash::of(b"abc"),
                docs: None,
                lines: vec![LineFacts {
                    line: 6,
                    calls: vec![Call {
                        name: "helper".into(),
                        defined_in_file: true,
                    }],
                }],
            }],
        }
    }

    #[test]
    fn serialises_a_function_with_its_facts() {
        let json = serde_json::to_value(FileAnalysisDto::from(&sample())).unwrap();
        assert_eq!(json["wire_version"], 1);
        assert_eq!(json["has_syntax_error"], false);
        assert_eq!(json["functions"][0]["kind"], "method");
        assert_eq!(json["functions"][0]["enclosing"], "C");
        assert_eq!(
            json["functions"][0]["lines"][0]["calls"][0]["name"],
            "helper"
        );
        assert_eq!(
            json["functions"][0]["hash"],
            ContentHash::of(b"abc").to_string()
        );
    }

    #[test]
    fn round_trips_through_json() {
        let dto = FileAnalysisDto::from(&sample());
        let text = serde_json::to_string(&dto).unwrap();
        let back: FileAnalysisDto = serde_json::from_str(&text).unwrap();
        assert_eq!(back, dto);
    }
}
