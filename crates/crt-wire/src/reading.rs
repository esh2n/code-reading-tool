//! JSON shapes for readings. Used for CLI output, LSP payloads, HTML input
//! and the on-disk cache, so they convert both ways.

use serde::{Deserialize, Serialize};

use crt_app::FunctionReading;
use crt_domain::{
    Author, Basis, CheckReport, ContentHash, FactRef, Note, Reading, Scenario, ScenarioKind, Step,
};

use crate::{FunctionDto, WIRE_VERSION};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionReadingDto {
    pub wire_version: u32,
    pub language: String,
    pub has_syntax_error: bool,
    pub from_cache: bool,
    pub warnings: Vec<String>,
    pub function: FunctionDto,
    pub reading: ReadingDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadingDto {
    pub function_hash: String,
    pub model: String,
    pub prompt: String,
    pub notes: Vec<NoteDto>,
    pub scenarios: Vec<ScenarioDto>,
    /// Notes demoted from fact to guess by the checker.
    pub demoted: usize,
    /// Notes or steps removed because they were outside the function.
    pub dropped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteDto {
    pub line: usize,
    pub text: String,
    pub detail: Option<String>,
    pub assumptions: Vec<String>,
    pub basis: BasisDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BasisDto {
    /// Read off the syntax tree.
    Fact { call: String },
    /// The model's reasoning.
    Inference,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScenarioKindDto {
    Normal,
    Boundary,
    Concurrent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScenarioDto {
    pub kind: ScenarioKindDto,
    pub title: String,
    pub input: String,
    pub steps: Vec<StepDto>,
    pub outcome: String,
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepDto {
    pub line: usize,
    pub what: String,
}

impl From<&FunctionReading> for FunctionReadingDto {
    fn from(r: &FunctionReading) -> Self {
        Self {
            wire_version: WIRE_VERSION,
            language: r.language.to_string(),
            has_syntax_error: r.has_syntax_error,
            from_cache: r.from_cache,
            warnings: r.warnings.clone(),
            function: FunctionDto::from(&r.function),
            reading: ReadingDto::from(&r.reading),
        }
    }
}

impl From<&Reading> for ReadingDto {
    fn from(r: &Reading) -> Self {
        Self {
            function_hash: r.function_hash.to_string(),
            model: r.author.model.clone(),
            prompt: r.author.prompt.clone(),
            notes: r.notes.iter().map(NoteDto::from).collect(),
            scenarios: r.scenarios.iter().map(ScenarioDto::from).collect(),
            demoted: r.check.demoted,
            dropped: r.check.dropped,
        }
    }
}

impl From<&Note> for NoteDto {
    fn from(n: &Note) -> Self {
        Self {
            line: n.line,
            text: n.text.clone(),
            detail: n.detail.clone(),
            assumptions: n.assumptions.clone(),
            basis: match &n.basis {
                Basis::Fact(FactRef::Call { name }) => BasisDto::Fact { call: name.clone() },
                _ => BasisDto::Inference,
            },
        }
    }
}

impl From<&Scenario> for ScenarioDto {
    fn from(s: &Scenario) -> Self {
        Self {
            kind: match s.kind {
                ScenarioKind::Normal => ScenarioKindDto::Normal,
                ScenarioKind::Boundary => ScenarioKindDto::Boundary,
                ScenarioKind::Concurrent => ScenarioKindDto::Concurrent,
            },
            title: s.title.clone(),
            input: s.input.clone(),
            steps: s
                .steps
                .iter()
                .map(|st| StepDto {
                    line: st.line,
                    what: st.what.clone(),
                })
                .collect(),
            outcome: s.outcome.clone(),
            assumptions: s.assumptions.clone(),
        }
    }
}

/// A stored reading that cannot be turned back into a domain value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidReading(pub String);

impl std::fmt::Display for InvalidReading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid stored reading: {}", self.0)
    }
}

impl std::error::Error for InvalidReading {}

impl TryFrom<ReadingDto> for Reading {
    type Error = InvalidReading;

    fn try_from(d: ReadingDto) -> Result<Self, Self::Error> {
        let function_hash = ContentHash::parse_hex(&d.function_hash)
            .ok_or_else(|| InvalidReading(format!("bad hash {}", d.function_hash)))?;
        Ok(Reading {
            function_hash,
            author: Author {
                model: d.model,
                prompt: d.prompt,
            },
            notes: d.notes.into_iter().map(Note::from).collect(),
            scenarios: d.scenarios.into_iter().map(Scenario::from).collect(),
            check: CheckReport {
                demoted: d.demoted,
                dropped: d.dropped,
            },
        })
    }
}

impl From<NoteDto> for Note {
    fn from(d: NoteDto) -> Self {
        Self {
            line: d.line,
            text: d.text,
            detail: d.detail,
            assumptions: d.assumptions,
            basis: match d.basis {
                BasisDto::Fact { call } => Basis::Fact(FactRef::Call { name: call }),
                BasisDto::Inference => Basis::Inference,
            },
        }
    }
}

impl From<ScenarioDto> for Scenario {
    fn from(d: ScenarioDto) -> Self {
        Self {
            kind: match d.kind {
                ScenarioKindDto::Normal => ScenarioKind::Normal,
                ScenarioKindDto::Boundary => ScenarioKind::Boundary,
                ScenarioKindDto::Concurrent => ScenarioKind::Concurrent,
            },
            title: d.title,
            input: d.input,
            steps: d
                .steps
                .into_iter()
                .map(|s| Step {
                    line: s.line,
                    what: s.what,
                })
                .collect(),
            outcome: d.outcome,
            assumptions: d.assumptions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_round_trips_through_json() {
        let reading = Reading {
            function_hash: ContentHash::of(b"f"),
            author: Author {
                model: "m".into(),
                prompt: "v2".into(),
            },
            notes: vec![
                Note {
                    line: 3,
                    text: "calls x".into(),
                    detail: Some("longer".into()),
                    assumptions: vec!["x is pure".into()],
                    basis: Basis::Fact(FactRef::Call { name: "x".into() }),
                },
                Note {
                    line: 4,
                    text: "guess".into(),
                    detail: None,
                    assumptions: vec![],
                    basis: Basis::Inference,
                },
            ],
            scenarios: vec![Scenario {
                kind: ScenarioKind::Concurrent,
                title: "two at once".into(),
                input: "two requests".into(),
                steps: vec![Step {
                    line: 3,
                    what: "both read".into(),
                }],
                outcome: "lost update".into(),
                assumptions: vec![],
            }],
            check: CheckReport {
                demoted: 1,
                dropped: 2,
            },
        };
        let json = serde_json::to_string(&ReadingDto::from(&reading)).unwrap();
        assert!(json.contains("\"kind\":\"fact\""));
        assert!(json.contains("\"kind\":\"concurrent\""));
        let back: ReadingDto = serde_json::from_str(&json).unwrap();
        assert_eq!(Reading::try_from(back).unwrap(), reading);
    }

    #[test]
    fn a_corrupt_hash_is_rejected_not_trusted() {
        let mut dto = ReadingDto::from(&Reading {
            function_hash: ContentHash::of(b"f"),
            author: Author {
                model: "m".into(),
                prompt: "v1".into(),
            },
            notes: vec![],
            scenarios: vec![],
            check: CheckReport::default(),
        });
        dto.function_hash = "zz".into();
        assert!(Reading::try_from(dto).is_err());
    }
}
