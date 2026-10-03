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
    /// A command that prints the API key on its first line (a password
    /// manager, the OS keychain). Run once, on first use. Use either this
    /// or `api_key_env`.
    pub api_key_command: Option<Vec<String>>,
}

/// The `[llm]` section of the configuration file.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LlmConfig {
    pub base_url: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub api_key_command: Option<Vec<String>>,
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
    /// Extra fields merged into every request body, for endpoint-specific
    /// options (for example turning a model's thinking mode off). They
    /// never replace `model`, `messages` or `response_format`. Part of the
    /// cache key: different options make a different reading.
    #[serde(default)]
    pub extra_body: serde_json::Map<String, serde_json::Value>,
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
            api_key_command: self.api_key_command.clone(),
        };
        std::iter::once(primary)
            .chain(self.fallback.iter().cloned())
            .collect()
    }
}
