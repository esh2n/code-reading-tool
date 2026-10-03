//! The JSON shapes this tool exposes: CLI output, LSP custom-method payloads
//! and the HTML renderer's input all use these. They are versioned apart
//! from the domain types, which carry no serde at all.

use serde::{Deserialize, Serialize};

use crt_app::FileAnalysis;
use crt_domain::{Call, Function, LineFacts, SymbolKind};

/// Bumped whenever a shape below changes incompatibly.
pub const WIRE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileAnalysisDto {
    pub wire_version: u32,
    pub language: String,
    pub functions: Vec<FunctionDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionDto {
    pub name: String,
    /// `function` or `method`.
    pub kind: String,
    pub enclosing: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    /// Hex content hash of the function's source.
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
            functions: a.functions.iter().map(FunctionDto::from).collect(),
        }
    }
}

impl From<&Function> for FunctionDto {
    fn from(f: &Function) -> Self {
        Self {
            name: f.name.clone(),
            kind: kind_name(&f.kind).to_string(),
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

fn kind_name(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Type => "type",
        SymbolKind::Interface => "interface",
        SymbolKind::Module => "module",
        SymbolKind::Call => "call",
        SymbolKind::Other(_) => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crt_domain::{ContentHash, LanguageId, Span};

    #[test]
    fn serialises_a_function_with_its_facts() {
        let analysis = FileAnalysis {
            language: LanguageId::new("go"),
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
        };
        let json = serde_json::to_value(FileAnalysisDto::from(&analysis)).unwrap();
        assert_eq!(json["wire_version"], 1);
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
}
