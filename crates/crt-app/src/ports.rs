//! Capabilities the use cases need from the outside, in the domain's words.

use std::fmt;
use std::path::Path;

use crt_domain::{
    Author, ContentHash, Draft, Function, LanguageId, Note, Reading, Scenario, Symbol,
};

/// What a structure source found in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    /// Every symbol, in file order.
    pub symbols: Vec<Symbol>,
    /// True when the parser hit syntax errors; symbols may then be missing
    /// or have wrong spans, and readers must be told.
    pub has_syntax_error: bool,
}

/// Reads the structure (definitions and references) of source text.
pub trait StructureSource {
    /// The language this source would use for `path`, if it knows one.
    fn language_for(&self, path: &Path) -> Option<LanguageId>;

    /// The structure of `source` in `language`.
    fn structure(&self, language: &LanguageId, source: &[u8]) -> Result<Structure, StructureError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StructureError {
    UnknownLanguage(LanguageId),
    Parse(String),
}

impl fmt::Display for StructureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLanguage(id) => write!(f, "no structure source for language {id}"),
            Self::Parse(msg) => write!(f, "could not read structure: {msg}"),
        }
    }
}

impl std::error::Error for StructureError {}

/// A function whose body calls the function being read, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub name: String,
    pub enclosing: Option<String>,
    pub start_line: usize,
    /// The caller's source with line numbers.
    pub numbered_source: String,
}

/// Everything the explainer is given about one function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainRequest {
    pub language: LanguageId,
    pub function: Function,
    /// The function's source with line numbers, as the notes must refer to.
    pub numbered_source: String,
    /// Callers in the same file, found by name; may include wrong matches.
    pub callers: Vec<Caller>,
}

/// What the explainer wrote, and who actually wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explained {
    pub draft: Draft,
    /// The author that produced this draft. Differs from
    /// [`Explainer::author`] when a fallback was used.
    pub author: Author,
    /// Things the reader should know (a fallback was used, a retry was
    /// needed).
    pub warnings: Vec<String>,
}

/// Scenarios the explainer wrote, and who actually wrote them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainedScenarios {
    pub scenarios: Vec<Scenario>,
    pub author: Author,
    pub warnings: Vec<String>,
}

/// Writes per-line notes and, separately and on request, behaviour
/// scenarios for a function. Two calls rather than one because the notes
/// are wanted as soon as a file opens and the scenarios rarely are; one
/// answer holding both takes several times longer to arrive.
pub trait Explainer {
    /// Every author this explainer can write as, primary first, then the
    /// fallbacks in order. Used to look up the cache before calling
    /// anything.
    fn authors(&self) -> Vec<Author>;

    /// Writes the notes. `on_progress` is called with every note received
    /// so far, unchecked, each time one more arrives. It may start again
    /// from fewer notes (a retry or a fallback starts over); each call
    /// replaces what the previous one said.
    fn explain(
        &self,
        request: &ExplainRequest,
        on_progress: &mut dyn FnMut(&[Note]),
    ) -> Result<Explained, ExplainError>;

    /// Writes the scenarios.
    fn scenarios(&self, request: &ExplainRequest) -> Result<ExplainedScenarios, ExplainError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExplainError {
    /// No endpoint (primary or fallback) could be reached.
    Unreachable(String),
    /// The endpoint does not accept a JSON Schema for structured output.
    StructuredOutputUnsupported(String),
    /// The endpoint kept returning output that does not fit the schema.
    Malformed(String),
    /// The endpoint refused the request (auth, bad request).
    Rejected(String),
    /// The configuration cannot be used as written.
    Config(String),
}

impl fmt::Display for ExplainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(m) => write!(f, "no LLM endpoint could be reached: {m}"),
            Self::StructuredOutputUnsupported(m) => {
                write!(
                    f,
                    "the LLM endpoint does not support structured output: {m}"
                )
            }
            Self::Malformed(m) => write!(f, "the LLM kept answering in the wrong shape: {m}"),
            Self::Rejected(m) => write!(f, "the LLM endpoint refused the request: {m}"),
            Self::Config(m) => write!(f, "the LLM configuration cannot be used: {m}"),
        }
    }
}

impl std::error::Error for ExplainError {}

/// Identifies one stored reading.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReadingKey {
    pub language: LanguageId,
    pub function_hash: ContentHash,
    pub author: Author,
}

/// Keeps readings between runs. Best-effort: a failing store never fails
/// a read. Readings are handed over with lines relative to the function's
/// first line (0 = first line; see `Reading::relative_to`), so a stored
/// reading stays right when its function moves within the file.
pub trait ReadingStore {
    fn get(&self, key: &ReadingKey) -> Result<Option<Reading>, StoreError>;
    fn put(&self, key: &ReadingKey, reading: &Reading) -> Result<(), StoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "reading store: {}", self.0)
    }
}

impl std::error::Error for StoreError {}
