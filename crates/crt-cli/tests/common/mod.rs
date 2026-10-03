//! A scripted OpenAI-compatible server shared by the end-to-end tests. It
//! streams its answers, and pauses inside the notes of `Inc` so a client
//! sees the first note before the second.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

pub const SOURCE: &str = "package p\n\ntype C struct{ n int }\n\nfunc (c *C) Inc() {\n\tc.n = add(c.n, 1)\n}\n\nfunc add(a, b int) int { return a + b }\n";

/// The answer to one request, as the pieces of a streamed response. A
/// pause after a piece is marked with `None`.
fn answer_for(task: &str, user: &str) -> Vec<Option<String>> {
    let inc = user.contains("FUNCTION: Inc");
    if task == "function_scenarios" {
        let content = json!({ "scenarios": [
            { "kind": "concurrent", "title": "two Inc at once", "input": "two goroutines",
              "steps": [ { "line": 6, "what": "both read c.n" }, { "line": 6, "what": "both write" } ],
              "outcome": "one increment is lost", "assumptions": ["no lock around Inc"] }
        ] });
        return vec![Some(content.to_string())];
    }
    if inc {
        let first = json!({ "line": 5, "text": "increments c.n", "detail": "", "assumptions": [],
                            "basis": { "kind": "inference", "call": null } });
        let second = json!({ "line": 6, "text": "calls add", "detail": "adds one", "assumptions": [],
                             "basis": { "kind": "fact", "call": "add" } });
        // The second note comes later, so the first is seen on its own.
        return vec![
            Some(format!("{{\"notes\":[{first},")),
            None,
            Some(format!("{second}]}}")),
        ];
    }
    let content = json!({ "notes": [ { "line": 9, "text": "returns the sum", "detail": "", "assumptions": [],
              "basis": { "kind": "inference", "call": null } } ] });
    vec![Some(content.to_string())]
}

/// Answers every request by looking at which function and which part
/// (notes or scenarios) it is about. Returns the base URL.
pub fn serve_llm() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
            }
            let mut buf = vec![0; len];
            reader.read_exact(&mut buf).unwrap();
            let req: Value = serde_json::from_slice(&buf).unwrap();
            assert_eq!(req["stream"], true);
            let pieces = answer_for(
                req["response_format"]["json_schema"]["name"]
                    .as_str()
                    .unwrap(),
                req["messages"][1]["content"].as_str().unwrap(),
            );
            thread::spawn(move || {
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n")
                    .unwrap();
                for piece in pieces {
                    match piece {
                        Some(p) => {
                            let chunk = json!({ "choices": [{ "delta": { "content": p }, "finish_reason": null }] });
                            write!(stream, "data: {chunk}\n\n").unwrap();
                            stream.flush().unwrap();
                        }
                        None => thread::sleep(Duration::from_millis(600)),
                    }
                }
                let end = json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] });
                write!(stream, "data: {end}\n\ndata: [DONE]\n\n").unwrap();
            });
        }
    });
    url
}
