//! Where to send requests. No default endpoint or model: the user says.

use serde::Deserialize;

/// One OpenAI-compatible endpoint.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    /// Base URL ending before `/chat/completions`, e.g. `https://host/v1`.
    pub base_url: String,
    pub model: String,
    /// Name of the environment variable holding the API key. The key
    /// itself never appears in configuration. Omit for keyless endpoints.
    pub api_key_env: Option<String>,
}

/// The `[llm]` section of the configuration file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub api_key_env: Option<String>,
    /// Tried in order when the endpoint above cannot be reached or fails
    /// with a server error.
    #[serde(default)]
    pub fallback: Vec<EndpointConfig>,
    /// Language the explanations are written in, e.g. `English`, `Japanese`.
    #[serde(default = "default_output_language")]
    pub output_language: String,
    /// Seconds to wait for one response.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_output_language() -> String {
    "English".to_string()
}

fn default_timeout_secs() -> u64 {
    180
}

impl LlmConfig {
    /// The primary endpoint followed by the fallbacks.
    pub fn endpoints(&self) -> Vec<EndpointConfig> {
        let primary = EndpointConfig {
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            api_key_env: self.api_key_env.clone(),
        };
        std::iter::once(primary)
            .chain(self.fallback.iter().cloned())
            .collect()
    }
}
