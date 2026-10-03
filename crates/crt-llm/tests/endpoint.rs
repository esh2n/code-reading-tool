//! Drives the adapter against a scripted local HTTP server.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use crt_app::{ExplainError, ExplainRequest, Explainer};
use crt_domain::{
    Basis, Call, ContentHash, FactRef, Function, LanguageId, LineFacts, Span, SymbolKind,
};
use crt_llm::{EndpointConfig, LlmConfig, OpenAiCompatible};
use serde_json::{Value, json};

/// A server that answers each request with the next scripted (status, body)
/// and records the request bodies it saw.
struct Script {
    url: String,
    seen: Arc<Mutex<Vec<Value>>>,
}

fn serve(responses: Vec<(u16, String)>) -> Script {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_in = Arc::clone(&seen);
    thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut buf = vec![0; len];
            reader.read_exact(&mut buf).unwrap();
            seen_in
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&buf).unwrap());
            let content_type = if body.starts_with("data:") {
                "text/event-stream"
            } else {
                "application/json"
            };
            let reply = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    Script { url, seen }
}

fn completion(content: &str) -> String {
    json!({ "choices": [{ "finish_reason": "stop", "message": { "role": "assistant", "content": content } }] })
        .to_string()
}

/// The answer as server-sent events, cut into pieces of `size` characters.
fn events(content: &str, size: usize) -> String {
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::new();
    for piece in chars.chunks(size) {
        let piece: String = piece.iter().collect();
        let chunk =
            json!({ "choices": [{ "delta": { "content": piece }, "finish_reason": null }] });
        out.push_str(&format!("data: {chunk}\n\n"));
    }
    let end = json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] });
    out.push_str(&format!("data: {end}\n\ndata: [DONE]\n\n"));
    out
}

const GOOD: &str = r#"{"notes":[{"line":6,"text":"calls helper","detail":"","assumptions":[],"basis":{"kind":"fact","call":"helper"}}]}"#;

const TWO_NOTES: &str = r#"{"notes":[{"line":5,"text":"copies c","detail":"","assumptions":[],"basis":{"kind":"inference","call":null}},{"line":6,"text":"calls helper","detail":"","assumptions":[],"basis":{"kind":"fact","call":"helper"}}]}"#;

const SCENARIOS: &str = r#"{"scenarios":[{"kind":"normal","title":"ok","input":"c","steps":[{"line":6,"what":"returns"}],"outcome":"copy","assumptions":[]}]}"#;

fn config(url: &str, fallback: Option<&str>) -> LlmConfig {
    LlmConfig {
        base_url: url.into(),
        model: "primary-model".into(),
        api_key_env: None,
        fallback: fallback
            .map(|u| {
                vec![EndpointConfig {
                    base_url: u.into(),
                    model: "fallback-model".into(),
                    api_key_env: None,
                }]
            })
            .unwrap_or_default(),
        output_language: "Japanese".into(),
        timeout_secs: 10,
        extra_body: serde_json::Map::new(),
    }
}

fn request() -> ExplainRequest {
    ExplainRequest {
        language: LanguageId::new("go"),
        function: Function {
            name: "Copy".into(),
            kind: SymbolKind::Method,
            enclosing: Some("C".into()),
            span: Span {
                start_byte: 0,
                end_byte: 1,
                start_line: 5,
                end_line: 7,
            },
            hash: ContentHash::of(b"x"),
            docs: None,
            lines: vec![LineFacts {
                line: 6,
                calls: vec![Call {
                    name: "helper".into(),
                    defined_in_file: true,
                }],
            }],
        },
        numbered_source: "    5 | func (c *C) Copy() *C {\n    6 | \treturn helper(c)\n    7 | }"
            .into(),
        callers: vec![],
    }
}

#[test]
fn sends_a_strict_schema_and_parses_the_answer() {
    let s = serve(vec![(200, completion(GOOD))]);
    let llm = OpenAiCompatible::new(config(&s.url, None)).unwrap();
    assert_eq!(llm.authors()[0].prompt, "v3-japanese");
    let out = llm.explain(&request(), &mut |_| {}).unwrap();
    assert_eq!(out.author.model, "primary-model");
    assert!(out.warnings.is_empty());
    assert_eq!(
        out.draft.notes[0].basis,
        Basis::Fact(FactRef::Call {
            name: "helper".into()
        })
    );

    let body = &s.seen.lock().unwrap()[0];
    assert_eq!(body["model"], "primary-model");
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    assert_eq!(
        body["response_format"]["json_schema"]["name"],
        "function_notes"
    );
    assert_eq!(body["stream"], true);
    let user = body["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains("line 6: calls helper (defined in this file)"));
    assert!(
        body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Write in Japanese.")
    );
}

#[test]
fn retries_a_malformed_answer_on_the_same_endpoint() {
    let s = serve(vec![
        (200, completion("{not json")),
        (200, completion(GOOD)),
    ]);
    let out = OpenAiCompatible::new(config(&s.url, None))
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap();
    assert_eq!(out.author.model, "primary-model");
    assert_eq!(out.warnings.len(), 1);
    assert_eq!(s.seen.lock().unwrap().len(), 2);
}

#[test]
fn gives_up_after_three_malformed_answers() {
    let s = serve(vec![
        (200, completion("{}")),
        (200, completion("{}")),
        (200, completion("{}")),
    ]);
    let err = OpenAiCompatible::new(config(&s.url, None))
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap_err();
    assert!(matches!(err, ExplainError::Malformed(_)));
}

#[test]
fn falls_back_on_a_server_error_and_says_so() {
    let primary = serve(vec![(503, "{\"error\":\"overloaded\"}".into())]);
    let fallback = serve(vec![(200, completion(GOOD))]);
    let out = OpenAiCompatible::new(config(&primary.url, Some(&fallback.url)))
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap();
    assert_eq!(out.author.model, "fallback-model");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("fallback model fallback-model"))
    );
}

#[test]
fn falls_back_when_the_primary_cannot_be_reached() {
    let fallback = serve(vec![(200, completion(GOOD))]);
    let out = OpenAiCompatible::new(config("http://127.0.0.1:9/v1", Some(&fallback.url)))
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap();
    assert_eq!(out.author.model, "fallback-model");
}

#[test]
fn an_endpoint_without_structured_output_is_an_error_not_a_downgrade() {
    let s = serve(vec![(
        400,
        "{\"error\":{\"message\":\"response_format json_schema is not supported\"}}".into(),
    )]);
    let err = OpenAiCompatible::new(config(&s.url, None))
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap_err();
    assert!(matches!(err, ExplainError::StructuredOutputUnsupported(_)));
}

#[test]
fn a_missing_api_key_variable_is_reported_by_name() {
    let mut c = config("http://127.0.0.1:9/v1", None);
    c.api_key_env = Some("CRT_TEST_KEY_THAT_IS_NOT_SET".into());
    let err = OpenAiCompatible::new(c)
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap_err();
    assert!(err.to_string().contains("CRT_TEST_KEY_THAT_IS_NOT_SET"));
}

#[test]
fn a_cut_off_answer_counts_as_malformed() {
    let cut = json!({ "choices": [{ "finish_reason": "length", "message": { "content": "{\"notes\":[" } }] }).to_string();
    let s = serve(vec![(200, cut.clone()), (200, cut.clone()), (200, cut)]);
    let err = OpenAiCompatible::new(config(&s.url, None))
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap_err();
    assert!(err.to_string().contains("cut off"));
}

#[test]
fn a_rate_limited_primary_falls_back() {
    let primary = serve(vec![(429, "{\"error\":\"slow down\"}".into())]);
    let fallback = serve(vec![(200, completion(GOOD))]);
    let out = OpenAiCompatible::new(config(&primary.url, Some(&fallback.url)))
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap();
    assert_eq!(out.author.model, "fallback-model");
}

#[test]
fn a_missing_key_on_the_primary_moves_to_the_fallback() {
    let fallback = serve(vec![(200, completion(GOOD))]);
    let mut c = config("http://127.0.0.1:9/v1", Some(&fallback.url));
    c.api_key_env = Some("CRT_TEST_KEY_THAT_IS_NOT_SET".into());
    let out = OpenAiCompatible::new(c)
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap();
    assert_eq!(out.author.model, "fallback-model");
    assert!(
        out.warnings
            .iter()
            .any(|w| w.contains("CRT_TEST_KEY_THAT_IS_NOT_SET"))
    );
}

#[test]
fn keys_are_never_sent_over_plain_http_to_another_host() {
    let mut c = config("http://llm.example/v1", None);
    c.api_key_env = Some("ANY".into());
    let err = OpenAiCompatible::new(c).err().unwrap();
    assert!(matches!(err, ExplainError::Config(_)), "{err}");
    let mut local = config("http://localhost:4000/v1", None);
    local.api_key_env = Some("ANY".into());
    assert!(OpenAiCompatible::new(local).is_ok());
    assert!(
        OpenAiCompatible::new(config("http://llm.example/v1", None)).is_ok(),
        "keyless http is allowed"
    );
    assert!(OpenAiCompatible::new(config("ftp://x/v1", None)).is_err());
}

#[test]
fn authors_list_the_primary_then_the_fallbacks() {
    let llm = OpenAiCompatible::new(config(
        "http://127.0.0.1:9/v1",
        Some("http://127.0.0.1:8/v1"),
    ))
    .unwrap();
    let models: Vec<_> = llm.authors().into_iter().map(|a| a.model).collect();
    assert_eq!(models, vec!["primary-model", "fallback-model"]);
}

#[test]
fn an_unreachable_endpoint_reports_the_underlying_cause() {
    let err = OpenAiCompatible::new(config("http://127.0.0.1:9/v1", None))
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap_err();
    let msg = err.to_string().to_ascii_lowercase();
    assert!(msg.contains("connect") || msg.contains("refused"), "{msg}");
}

#[test]
fn extra_body_is_sent_but_never_overrides_the_core_fields() {
    let s = serve(vec![(200, completion(GOOD))]);
    let mut c = config(&s.url, None);
    c.extra_body.insert(
        "chat_template_kwargs".into(),
        json!({ "enable_thinking": false }),
    );
    c.extra_body.insert("model".into(), json!("sneaky"));
    c.extra_body
        .insert("response_format".into(), json!({ "type": "text" }));
    let llm = OpenAiCompatible::new(c).unwrap();
    assert!(llm.authors()[0].prompt.starts_with("v3-japanese-x"));
    llm.explain(&request(), &mut |_| {}).unwrap();
    let body = &s.seen.lock().unwrap()[0];
    assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
    assert_eq!(body["model"], "primary-model");
    assert_eq!(body["response_format"]["type"], "json_schema");
}

#[test]
fn streamed_notes_arrive_one_by_one() {
    let s = serve(vec![(200, events(TWO_NOTES, 5))]);
    let llm = OpenAiCompatible::new(config(&s.url, None)).unwrap();
    let mut seen: Vec<usize> = vec![];
    let out = llm
        .explain(&request(), &mut |n| seen.push(n.len()))
        .unwrap();
    assert_eq!(seen, vec![1, 2]);
    assert_eq!(out.draft.notes.len(), 2);
}

#[test]
fn a_retry_starts_the_progress_over() {
    let broken = r#"{"notes":[{"line":5,"text":"a","detail":"","assumptions":[],"basis":{"kind":"inference","call":null}},{"#;
    let s = serve(vec![(200, events(broken, 7)), (200, events(GOOD, 7))]);
    let llm = OpenAiCompatible::new(config(&s.url, None)).unwrap();
    let mut seen: Vec<usize> = vec![];
    let out = llm
        .explain(&request(), &mut |n| seen.push(n.len()))
        .unwrap();
    assert_eq!(seen, vec![1, 0, 1], "shown, cleared, shown again");
    assert_eq!(out.draft.notes[0].line, 6);
}

#[test]
fn a_cut_off_stream_counts_as_malformed() {
    let cut = format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({ "choices": [{ "delta": { "content": "{\"notes\":[" }, "finish_reason": "length" }] })
    );
    let s = serve(vec![(200, cut.clone()), (200, cut.clone()), (200, cut)]);
    let err = OpenAiCompatible::new(config(&s.url, None))
        .unwrap()
        .explain(&request(), &mut |_| {})
        .unwrap_err();
    assert!(err.to_string().contains("cut off"), "{err}");
}

#[test]
fn scenarios_are_a_separate_request_with_their_own_schema() {
    let s = serve(vec![(200, events(SCENARIOS, 11))]);
    let llm = OpenAiCompatible::new(config(&s.url, None)).unwrap();
    let out = llm.scenarios(&request()).unwrap();
    assert_eq!(out.scenarios[0].steps[0].line, 6);
    assert_eq!(out.author.model, "primary-model");
    let body = &s.seen.lock().unwrap()[0];
    assert_eq!(
        body["response_format"]["json_schema"]["name"],
        "function_scenarios"
    );
    assert_eq!(
        body["response_format"]["json_schema"]["schema"]["required"],
        json!(["scenarios"])
    );
}
