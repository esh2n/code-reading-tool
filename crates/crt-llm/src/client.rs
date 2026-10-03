//! The HTTP side: one request per attempt, retries on a malformed answer,
//! falls back to the next endpoint when one is down, rate-limited or not
//! usable.

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
    output_language: String,
    extra_body: serde_json::Map<String, Value>,
    endpoints: Vec<EndpointConfig>,
    http: Client,
}

impl OpenAiCompatible {
    /// Checks the configuration and builds the client. An endpoint that
    /// would receive an API key must use https, except on this machine.
    pub fn new(config: LlmConfig) -> Result<Self, ExplainError> {
        let endpoints = config.endpoints();
        for e in &endpoints {
            check_endpoint(e)?;
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| ExplainError::Config(chain(&e)))?;
        Ok(Self {
            output_language: config.output_language,
            extra_body: config.extra_body,
            endpoints,
            http,
        })
    }

    fn author_for(&self, endpoint: &EndpointConfig) -> Author {
        Author {
            model: endpoint.model.clone(),
            prompt: prompt::identity(&self.output_language, &self.extra_body),
        }
    }
}

fn check_endpoint(e: &EndpointConfig) -> Result<(), ExplainError> {
    let url = reqwest::Url::parse(&e.base_url).map_err(|err| {
        ExplainError::Config(format!("base_url {:?} is not a URL: {err}", e.base_url))
    })?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
    );
    match url.scheme() {
        "https" => Ok(()),
        "http" if local || e.api_key_env.is_none() => Ok(()),
        "http" => Err(ExplainError::Config(format!(
            "{} sends an API key over plain http; use https (http is allowed only for localhost)",
            e.base_url
        ))),
        other => Err(ExplainError::Config(format!(
            "{}: unsupported scheme {other}",
            e.base_url
        ))),
    }
}

/// How one attempt against one endpoint ended.
enum Attempt {
    Ok(Draft),
    /// The answer did not fit the schema; worth asking the same endpoint again.
    Malformed(String),
    /// Not usable right now (unreachable, server error, rate limit, missing
    /// key); worth trying the next endpoint.
    Down(String),
    /// A final answer for this request: no retry, no fallback.
    Fatal(ExplainError),
}

impl Explainer for OpenAiCompatible {
    fn authors(&self) -> Vec<Author> {
        self.endpoints.iter().map(|e| self.author_for(e)).collect()
    }

    fn explain(&self, request: &ExplainRequest) -> Result<Explained, ExplainError> {
        let mut warnings = Vec::new();
        let mut down = Vec::new();
        for (i, endpoint) in self.endpoints.iter().enumerate() {
            let body = self.body(request, &endpoint.model);
            let mut malformed: Option<String> = None;
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
                    Attempt::Malformed(m) => malformed = Some(m),
                    Attempt::Down(m) => {
                        down.push(format!("{}: {m}", endpoint.base_url));
                        malformed = None;
                        break;
                    }
                    Attempt::Fatal(e) => return Err(e),
                }
            }
            // A model that keeps answering in the wrong shape is a problem
            // with that model, not an outage: report it rather than hide it
            // behind a fallback.
            if let Some(m) = malformed {
                return Err(ExplainError::Malformed(format!("{}: {m}", endpoint.model)));
            }
        }
        Err(ExplainError::Unreachable(down.join("; ")))
    }
}

impl OpenAiCompatible {
    fn body(&self, request: &ExplainRequest, model: &str) -> Value {
        let mut body = serde_json::Map::new();
        for (k, v) in &self.extra_body {
            body.insert(k.clone(), v.clone());
        }
        let core = json!({
            "model": model,
            "messages": [
                { "role": "system", "content": prompt::system(&self.output_language) },
                { "role": "user", "content": prompt::user(request) }
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "function_reading", "strict": true, "schema": prompt::schema() }
            }
        });
        // The core fields win over anything in extra_body.
        if let Value::Object(core) = core {
            body.extend(core);
        }
        Value::Object(body)
    }

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
                    return Attempt::Down(format!(
                        "environment variable {var} (api_key_env for {}) is not set",
                        endpoint.model
                    ));
                }
            }
        }
        let response = match req.send() {
            Ok(r) => r,
            Err(e) => return Attempt::Down(chain(&e)),
        };
        let status = response.status();
        let text = match response.text() {
            Ok(t) => t,
            Err(e) => return Attempt::Down(chain(&e)),
        };
        if status.is_server_error()
            || status == StatusCode::TOO_MANY_REQUESTS
            || status == StatusCode::REQUEST_TIMEOUT
        {
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
        match content_of(&text).and_then(|c| prompt::parse(&c)) {
            Ok(draft) => Attempt::Ok(draft),
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

/// The error and its causes on one line ("error sending request: ...:
/// connection refused").
fn chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        out.push_str(": ");
        out.push_str(&c.to_string());
        cur = c.source();
    }
    out
}
