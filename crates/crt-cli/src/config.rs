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
# output_language = "Japanese"

# The API key: choose one of these, or neither for an endpoint without keys.
# The key itself never goes in this file.
#
# From an environment variable (only programs started from a shell that
# sets it will see it):
# api_key_env = "YOUR_API_KEY_VARIABLE"
#
# From a command that prints the key on its first line. Run once, the first
# time a model is called; editors started from the Dock or a launcher get the
# key this way too. For example:
# api_key_command = ["security", "find-generic-password", "-s", "your-service", "-w"]  # macOS Keychain
# api_key_command = ["secret-tool", "lookup", "service", "your-service"]              # Linux Secret Service
# api_key_command = ["op", "read", "op://Vault/Item/credential"]                      # 1Password
# api_key_command = ["bw", "get", "password", "your-item"]                            # Bitwarden
# api_key_command = ["pass", "show", "your-item"]                                     # pass

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

/// Writes the commented example to `path` unless a file is already there.
/// Returns whether it wrote one.
pub fn write_example_if_missing(path: &Path) -> Result<bool> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut f) => {
            std::io::Write::write_all(&mut f, EXAMPLE.as_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e).with_context(|| format!("creating {}", path.display())),
    }
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
        let uncommented =
            EXAMPLE.replace("# api_key_command = [\"op\"", "api_key_command = [\"op\"");
        let file: ConfigFile = toml::from_str(&uncommented).unwrap();
        assert_eq!(file.llm.unwrap().api_key_command.unwrap()[0], "op");
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
        assert!(err.contains("api_key_command"));
    }

    #[test]
    fn the_example_is_written_once_and_never_over_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        assert!(write_example_if_missing(&path).unwrap());
        std::fs::write(&path, "mine").unwrap();
        assert!(!write_example_if_missing(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
    }
}
