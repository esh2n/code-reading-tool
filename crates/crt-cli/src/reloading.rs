//! The explainer the language server uses: built from the configuration
//! file, and built again when the file changes, so an edit takes effect on
//! the next read without restarting the editor.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crt_app::{ExplainError, ExplainRequest, Explained, ExplainedScenarios, Explainer};
use crt_domain::{Author, Note};
use crt_llm::OpenAiCompatible;

use crate::config;

/// What identifies one version of the file: its modification time and
/// size. `None` when the file does not exist.
type Stamp = Option<(SystemTime, u64)>;

/// The explainer built from one version of the file, or why it could not be.
type Built = Result<Arc<OpenAiCompatible>, String>;

pub struct Reloading {
    path: PathBuf,
    loaded: Mutex<Option<(Stamp, Built)>>,
}

impl Reloading {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            loaded: Mutex::new(None),
        }
    }

    fn stamp(&self) -> Stamp {
        let meta = std::fs::metadata(&self.path).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    }

    /// The explainer for the file as it is now, or why there is none.
    pub fn current(&self) -> Built {
        let stamp = self.stamp();
        let mut loaded = self
            .loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((seen, result)) = loaded.as_ref()
            && *seen == stamp
        {
            return result.clone();
        }
        let result = config::load_llm(&self.path)
            .and_then(|llm| OpenAiCompatible::new(llm).map_err(anyhow::Error::from))
            .map(Arc::new)
            .map_err(|e| format!("{e:#}"));
        *loaded = Some((stamp, result.clone()));
        result
    }
}

impl Explainer for Reloading {
    fn authors(&self) -> Vec<Author> {
        self.current().map(|e| e.authors()).unwrap_or_default()
    }

    fn explain(
        &self,
        request: &ExplainRequest,
        on_progress: &mut dyn FnMut(&[Note]),
    ) -> Result<Explained, ExplainError> {
        self.current()
            .map_err(ExplainError::Config)?
            .explain(request, on_progress)
    }

    fn scenarios(&self, request: &ExplainRequest) -> Result<ExplainedScenarios, ExplainError> {
        self.current()
            .map_err(ExplainError::Config)?
            .scenarios(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_file_is_picked_up_without_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let r = Reloading::new(path.clone());
        let missing = r.current().err().unwrap();
        assert!(missing.contains("[llm]"), "{missing}");

        std::fs::write(
            &path,
            "[llm]\nbase_url = 'http://localhost:1/v1'\nmodel = 'a'\n",
        )
        .unwrap();
        assert_eq!(r.authors()[0].model, "a");

        // A longer file, so the change is seen even within the same
        // modification-time tick.
        std::fs::write(
            &path,
            "[llm]\nbase_url = 'http://localhost:1/v1'\nmodel = 'model-b'\n",
        )
        .unwrap();
        assert_eq!(r.authors()[0].model, "model-b");

        std::fs::write(&path, "[llm]\nbase_url = 'x'\n").unwrap();
        assert!(r.authors().is_empty());
        assert!(r.current().is_err());
    }
}
