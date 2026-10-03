//! The HTTP side: one streamed request per attempt, retries on a malformed
//! answer, falls back to the next endpoint when one is down, rate-limited
//! or not usable.

use std::io::BufReader;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::{Value, json};

use crt_app::{ExplainError, ExplainRequest, Explained, ExplainedScenarios, Explainer};
use crt_domain::{Author, Note};

use crate::config::{EndpointConfig, LlmConfig};
use crate::key::KeySource;
use crate::prompt::{self, Task};
use crate::stream::{ObjectScanner, read_events};

/// How many times one endpoint is asked again when its answer does not
/// fit the schema (after the first attempt).
const MALFORMED_RETRIES: usize = 2;

pub struct OpenAiCompatible {
    output_language: String,
    extra_body: serde_json::Map<String, Value>,
    endpoints: Vec<EndpointConfig>,
    /// One per endpoint, same order.
    keys: Vec<KeySource>,
    http: Client,
}

impl OpenAiCompatible {
    /// Checks the configuration and builds the client. An endpoint that
    /// would receive an API key must use https, except on this machine.
    pub fn new(config: LlmConfig) -> Result<Self, ExplainError> {
        let endpoints = config.endpoints();
        let mut keys = Vec::with_capacity(endpoints.len());
        for e in &endpoints {
            check_endpoint(e)?;
            keys.push(KeySource::for_endpoint(e)?);
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| ExplainError::Config(chain(&e)))?;
        Ok(Self {
            output_language: config.output_language,
            extra_body: config.extra_body,
            endpoints,
            keys,
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
        "http" if local || (e.api_key_env.is_none() && e.api_key_command.is_none()) => Ok(()),
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
enum Attempt<T> {
    Ok(T),
    /// The answer did not fit the schema; worth asking the same endpoint again.
    Malformed(String),
    /// Not usable right now (unreachable, server error, rate limit, missing
    /// key); worth trying the next endpoint.
    Down(String),
    /// A final answer for this request: no retry, no fallback.
    Fatal(ExplainError),
}

/// A parsed answer and who wrote it.
struct Answered<T> {
    value: T,
    author: Author,
    warnings: Vec<String>,
}

impl Explainer for OpenAiCompatible {
    fn authors(&self) -> Vec<Author> {
        self.endpoints.iter().map(|e| self.author_for(e)).collect()
    }

    fn explain(
        &self,
        request: &ExplainRequest,
        on_progress: &mut dyn FnMut(&[Note]),
    ) -> Result<Explained, ExplainError> {
        let a = self.run(Task::Notes, request, on_progress, prompt::parse_notes)?;
        Ok(Explained {
            draft: a.value,
            author: a.author,
            warnings: a.warnings,
        })
    }

    fn scenarios(&self, request: &ExplainRequest) -> Result<ExplainedScenarios, ExplainError> {
        let a = self.run(
            Task::Scenarios,
            request,
            &mut |_| {},
            prompt::parse_scenarios,
        )?;
        Ok(ExplainedScenarios {
            scenarios: a.value,
            author: a.author,
            warnings: a.warnings,
        })
    }
}

impl OpenAiCompatible {
    /// Asks `task` of each endpoint in turn until one answers in shape.
    /// For notes, `on_progress` sees the notes complete so far while the
    /// answer streams in, and an empty list when an attempt starts over.
    fn run<T>(
        &self,
        task: Task,
        request: &ExplainRequest,
        on_progress: &mut dyn FnMut(&[Note]),
        parse: fn(&str) -> Result<T, String>,
    ) -> Result<Answered<T>, ExplainError> {
        let mut warnings = Vec::new();
        let mut down = Vec::new();
        let mut shown = false;
        for (i, endpoint) in self.endpoints.iter().enumerate() {
            let body = self.body(task, request, &endpoint.model);
            let mut malformed: Option<String> = None;
            for attempt in 0..=MALFORMED_RETRIES {
                if shown {
                    on_progress(&[]);
                    shown = false;
                }
                let mut scanner = ObjectScanner::default();
                let mut notes: Vec<Note> = Vec::new();
                let mut on_content = |piece: &str| {
                    if task != Task::Notes {
                        return;
                    }
                    let before = notes.len();
                    notes.extend(
                        scanner
                            .push(piece)
                            .iter()
                            .filter_map(|o| prompt::parse_note(o).ok()),
                    );
                    if notes.len() > before {
                        on_progress(&notes);
                        shown = true;
                    }
                };
                match self.attempt(endpoint, &self.keys[i], &body, &mut on_content, parse) {
                    Attempt::Ok(value) => {
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
                        return Ok(Answered {
                            value,
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

    fn body(&self, task: Task, request: &ExplainRequest, model: &str) -> Value {
        let mut body = serde_json::Map::new();
        for (k, v) in &self.extra_body {
            body.insert(k.clone(), v.clone());
        }
        let core = json!({
            "model": model,
            "messages": [
                { "role": "system", "content": prompt::system(task, &self.output_language) },
                { "role": "user", "content": prompt::user(task, request, &self.output_language) }
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": task.name(), "strict": true, "schema": prompt::schema(task) }
            },
            "stream": true
        });
        // The core fields win over anything in extra_body.
        if let Value::Object(core) = core {
            body.extend(core);
        }
        Value::Object(body)
    }

    /// One request. The answer's text is handed to `on_content` piece by
    /// piece as it streams in; a server that answers without streaming
    /// hands it over in one piece.
    fn attempt<T>(
        &self,
        endpoint: &EndpointConfig,
        key: &KeySource,
        body: &Value,
        on_content: &mut dyn FnMut(&str),
        parse: fn(&str) -> Result<T, String>,
    ) -> Attempt<T> {
        let url = format!(
            "{}/chat/completions",
            endpoint.base_url.trim_end_matches('/')
        );
        let mut req = self.http.post(&url).json(body);
        match key.key() {
            Ok(Some(k)) => req = req.bearer_auth(k),
            Ok(None) => {}
            Err(why) => return Attempt::Down(format!("{why} for {}", endpoint.model)),
        }
        let response = match req.send() {
            Ok(r) => r,
            Err(e) => return Attempt::Down(chain(&e)),
        };
        let status = response.status();
        let streamed = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if status.is_success() && streamed {
            return match read_events(BufReader::new(response), on_content) {
                Ok(s) if s.finish_reason.as_deref() == Some("length") => {
                    Attempt::Malformed("the answer was cut off at the output limit".into())
                }
                Ok(s) if !s.refusal.is_empty() => {
                    Attempt::Malformed(format!("the model refused: {}", s.refusal))
                }
                Ok(s) => parsed(&s.content, parse),
                Err(e) => Attempt::Down(e),
            };
        }
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
        match content_of(&text) {
            Ok(content) => {
                on_content(&content);
                parsed(&content, parse)
            }
            Err(e) => Attempt::Malformed(e),
        }
    }
}

fn parsed<T>(content: &str, parse: fn(&str) -> Result<T, String>) -> Attempt<T> {
    match parse(content) {
        Ok(value) => Attempt::Ok(value),
        Err(e) => Attempt::Malformed(e),
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
