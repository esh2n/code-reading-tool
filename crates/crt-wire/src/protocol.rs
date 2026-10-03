//! The custom LSP methods, shared by the server and anything that tests or
//! documents it. Line numbers here are LSP's: 0-based.
//!
//! Server → client
//! - `codeReading/fileReadings` (notification, [`FileReadingsParams`]):
//!   the functions of a document, their structural facts, and their cached
//!   readings, plus the notes received so far for readings still being
//!   written. Sent on open, on change, while notes arrive, and whenever a
//!   reading finishes.
//!
//! Client → server
//! - `codeReading/read` (request, [`ReadParams`] → [`FunctionReadingDto`]):
//!   read the notes of the function at a line now, asking the model if not
//!   cached.
//! - `codeReading/scenarios` (request, [`ReadParams`] → [`FunctionReadingDto`]):
//!   the same reading with its scenarios, asking the model for them if not
//!   cached. `refresh` rewrites the scenarios only.
//! - `codeReading/configPath` (request, no params → [`ConfigPathResult`]):
//!   where the configuration file is, creating it with a commented example
//!   when it does not exist, so editors can open it.
//! - `codeReading/visibleRange` (notification, [`VisibleRangeParams`]):
//!   which lines the user can see; the server reads uncached functions there
//!   when auto-read is on.
//!
//! [`FunctionReadingDto`]: crate::FunctionReadingDto

use serde::{Deserialize, Serialize};

use crate::{FileAnalysisDto, NoteDto, ReadingDto};

pub const FILE_READINGS: &str = "codeReading/fileReadings";
pub const READ: &str = "codeReading/read";
pub const SCENARIOS: &str = "codeReading/scenarios";
pub const CONFIG_PATH: &str = "codeReading/configPath";
pub const VISIBLE_RANGE: &str = "codeReading/visibleRange";

/// A document's functions with their cached readings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileReadingsParams {
    pub uri: String,
    /// The document version these were computed from.
    pub version: i32,
    pub analysis: FileAnalysisDto,
    /// One entry per function in `analysis.functions`; `null` when there is
    /// no reading yet.
    pub readings: Vec<Option<ReadingDto>>,
    /// Hashes of functions whose reading is being generated right now.
    pub pending: Vec<String>,
    /// For pending functions, the checked notes received so far. Replaced
    /// by the reading when it finishes.
    #[serde(default)]
    pub partial: Vec<PartialNotesDto>,
}

/// Notes of a reading still being written.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PartialNotesDto {
    pub function_hash: String,
    /// File lines, like a finished reading's.
    pub notes: Vec<NoteDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadParams {
    pub uri: String,
    /// 0-based line inside the function to read.
    pub line: u32,
    /// Ignore the cache and ask the model again.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPathResult {
    /// Absolute path of the configuration file.
    pub path: String,
    /// True when the file did not exist and was just written from the
    /// example.
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VisibleRangeParams {
    pub uri: String,
    /// 0-based, inclusive.
    pub start_line: u32,
    /// 0-based, inclusive.
    pub end_line: u32,
}

/// Options the client may pass as `initializationOptions`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct InitOptions {
    /// Read uncached functions in the visible range without being asked.
    pub auto_read: bool,
    /// How many functions may be read at once.
    pub max_parallel: usize,
}

impl Default for InitOptions {
    fn default() -> Self {
        Self {
            auto_read: true,
            max_parallel: 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_options_default_when_absent_or_partial() {
        let o: InitOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(o, InitOptions::default());
        let o: InitOptions = serde_json::from_str(r#"{"autoRead": false}"#).unwrap();
        assert!(!o.auto_read);
        assert_eq!(o.max_parallel, 2);
    }

    #[test]
    fn params_use_camel_case() {
        let p = VisibleRangeParams {
            uri: "file:///a".into(),
            start_line: 1,
            end_line: 2,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["startLine"], 1);
        assert_eq!(v["endLine"], 2);
    }
}
