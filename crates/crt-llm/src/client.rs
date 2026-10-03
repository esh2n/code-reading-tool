//! The HTTP side: one request per attempt, retries on a malformed answer,
//! falls back to the next endpoint on transport or server errors.

use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::{Value, json};

use crt_app::{ExplainError, ExplainRequest, Explained, Explainer};
use crt_domain::{Author, Draft};

use crate::config::{EndpointConfig, LlmConfig};
use crate::prompt;

/// How many times one endpoint is asked again when its answer does not
/// fit the schema (after the first attempt).
const MALFORMED_RETRIES: usize = 2;

pub struct OpenAiCompatible {
    config: LlmConfig,
    http: Client,
}

impl OpenAiCompatible {
    pub fn new(config: LlmConfig) -> Result<Self, ExplainError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| ExplainError::Unreachable(e.to_string()))?;
        Ok(Self { config, http })
    }

    fn author_for(&self, endpoint: &EndpointConfig) -> Author {
        Author {
            model: endpoint.model.clone(),
            prompt: prompt::identity(&self.config.output_language),
        }
    }
}

/// How one attempt against one endpoint ended.
enum Attempt {
    Ok(Draft),
    /// The answer did not fit the schema; worth asking the same endpoint again.
    Malformed(String),
    /// Transport failure or server error; worth trying the next endpoint.
    Down(String),
    /// A final answer for this request: no retry, no fallback.
    Fatal(ExplainError),
}

impl Explainer for OpenAiCompatible {
    fn author(&self) -> Author {
        self.author_for(&self.config.endpoints()[0])
    }

    fn explain(&self, request: &ExplainRequest) -> Result<Explained, ExplainError> {
        let body_for = |model: &str| {
            json!({
                "model": model,
                "messages": [
                    { "role": "system", "content": prompt::system(&self.config.output_language) },
                    { "role": "user", "content": prompt::user(request) }
                ],
                "response_format": {
                    "type": "json_schema",
                    "json_schema": { "name": "function_reading", "strict": true, "schema": prompt::schema() }
                }
            })
        };

        let mut warnings = Vec::new();
        let mut down = Vec::new();
        for (i, endpoint) in self.config.endpoints().iter().enumerate() {
            let body = body_for(&endpoint.model);
            let mut last_malformed = String::new();
            for attempt in 0..=MALFORMED_RETRIES {
                match self.attempt(endpoint, &body) {
                    Attempt::Ok(draft) => {
                        if attempt > 0 {
                            warnings.push(format!(
                                "{} answered in the wrong shape {attempt} time(s) before a valid answer",
                                endpoint.model
                            ));
                        }
                        if i > 0 {
                            warnings.push(format!(
                                "primary endpoint failed ({}); this reading was written by fallback model {}",
                                down.join("; "),
                                endpoint.model
                            ));
                        }
                        return Ok(Explained {
                            draft,
                            author: self.author_for(endpoint),
                            warnings,
                        });
                    }
                    Attempt::Malformed(m) => last_malformed = m,
                    Attempt::Down(m) => {
                        down.push(format!("{}: {m}", endpoint.base_url));
                        last_malformed.clear();
                        break;
                    }
                    Attempt::Fatal(e) => return Err(e),
                }
            }
            if !last_malformed.is_empty() {
                return Err(ExplainError::Malformed(format!(
                    "{}: {last_malformed}",
                    endpoint.model
                )));
            }
        }
        Err(ExplainError::Unreachable(down.join("; ")))
    }
}

impl OpenAiCompatible {
    fn attempt(&self, endpoint: &EndpointConfig, body: &Value) -> Attempt {
        let url = format!(
            "{}/chat/completions",
            endpoint.base_url.trim_end_matches('/')
        );
        let mut req = self.http.post(&url).json(body);
        if let Some(var) = &endpoint.api_key_env {
            match std::env::var(var) {
                Ok(key) if !key.is_empty() => req = req.bearer_auth(key),
                _ => {
                    return Attempt::Fatal(ExplainError::Rejected(format!(
                        "environment variable {var} (api_key_env for {}) is not set",
                        endpoint.model
                    )));
                }
            }
        }
        let response = match req.send() {
            Ok(r) => r,
            Err(e) => return Attempt::Down(e.to_string()),
        };
        let status = response.status();
        let text = match response.text() {
            Ok(t) => t,
            Err(e) => return Attempt::Down(e.to_string()),
        };
        if status.is_server_error() {
            return Attempt::Down(format!("{status}: {}", snippet(&text)));
        }
        if status == StatusCode::BAD_REQUEST && mentions_structured_output(&text) {
            return Attempt::Fatal(ExplainError::StructuredOutputUnsupported(format!(
                "{} at {}: {}",
                endpoint.model,
                endpoint.base_url,
                snippet(&text)
            )));
        }
        if !status.is_success() {
            return Attempt::Fatal(ExplainError::Rejected(format!(
                "{status}: {}",
                snippet(&text)
            )));
        }
        match content_of(&text) {
            Ok(content) => match prompt::parse(&content) {
                Ok(draft) => Attempt::Ok(draft),
                Err(e) => Attempt::Malformed(e),
            },
            Err(e) => Attempt::Malformed(e),
        }
    }
}

/// The assistant message content from a chat completion response.
fn content_of(text: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("response is not JSON: {e}"))?;
    let choice = &v["choices"][0];
    if choice["finish_reason"] == "length" {
        return Err("the answer was cut off at the output limit".into());
    }
    let message = &choice["message"];
    if let Some(refusal) = message["refusal"].as_str().filter(|r| !r.is_empty()) {
        return Err(format!("the model refused: {refusal}"));
    }
    message["content"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "response has no message content".into())
}

fn mentions_structured_output(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("response_format") || lower.contains("json_schema")
}

fn snippet(text: &str) -> String {
    text.chars().take(300).collect()
}
