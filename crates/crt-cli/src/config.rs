//! Locating and reading the user's configuration file and cache directory.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use crt_llm::LlmConfig;
use directories::ProjectDirs;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    llm: Option<LlmConfig>,
}

const EXAMPLE: &str = r#"[llm]
base_url = "https://your-endpoint.example/v1"   # an OpenAI-compatible endpoint
model = "your-model"
api_key_env = "YOUR_API_KEY_VARIABLE"           # omit for keyless endpoints
# output_language = "Japanese"

# [[llm.fallback]]
# base_url = "..."
# model = "..."
"#;

fn dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "crt")
}

/// The configuration file: `--config`, else the platform config directory.
pub fn config_path(flag: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = flag {
        return Ok(p.to_path_buf());
    }
    let dirs = dirs().context("cannot determine the configuration directory; pass --config")?;
    Ok(dirs.config_dir().join("config.toml"))
}

/// The cache directory for readings: `--cache-dir`, else the platform one.
pub fn cache_dir(flag: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = flag {
        return Ok(p.to_path_buf());
    }
    let dirs = dirs().context("cannot determine the cache directory; pass --cache-dir")?;
    Ok(dirs.cache_dir().join("readings"))
}

/// Reads the `[llm]` section, explaining how to write it when absent.
pub fn load_llm(path: &Path) -> Result<LlmConfig> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "no configuration at {}. Create it with:\n\n{EXAMPLE}",
                path.display()
            )
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let file: ConfigFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    match file.llm {
        Some(llm) => Ok(llm),
        None => bail!("{} has no [llm] section. Add:\n\n{EXAMPLE}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_parses() {
        let file: ConfigFile = toml::from_str(EXAMPLE).unwrap();
        let llm = file.llm.unwrap();
        assert_eq!(llm.model, "your-model");
        assert_eq!(llm.output_language, "English");
        assert!(llm.fallback.is_empty());
    }

    #[test]
    fn fallbacks_and_unknown_keys() {
        let ok = "[llm]\nbase_url='a'\nmodel='m'\n[[llm.fallback]]\nbase_url='b'\nmodel='n'\n";
        let llm = toml::from_str::<ConfigFile>(ok).unwrap().llm.unwrap();
        assert_eq!(llm.endpoints().len(), 2);
        assert!(
            toml::from_str::<ConfigFile>("[llm]\nbase_url='a'\nmodel='m'\napi_key='leak'\n")
                .is_err()
        );
    }

    #[test]
    fn a_missing_file_says_how_to_write_one() {
        let err = load_llm(Path::new("/nonexistent/crt.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("[llm]"));
        assert!(err.contains("api_key_env"));
    }
}
