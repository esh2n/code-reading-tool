use std::fmt;

/// Identifies a programming language, e.g. `go` or `python`. The set of
/// known ids is decided by whichever structure source is plugged in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageId(String);

impl LanguageId {
    /// Wraps an id such as `"go"`.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LanguageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
