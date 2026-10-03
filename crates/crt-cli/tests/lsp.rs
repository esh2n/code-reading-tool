//! `crt lsp` end to end: a minimal LSP client over the child's stdio, and a
//! scripted OpenAI-compatible server behind it.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

const SOURCE: &str = "package p\n\ntype C struct{ n int }\n\nfunc (c *C) Inc() {\n\tc.n = add(c.n, 1)\n}\n\nfunc add(a, b int) int { return a + b }\n";

fn answer_for(user: &str) -> String {
    let content = if user.contains("FUNCTION: Inc") {
        json!({
            "notes": [
                { "line": 6, "text": "calls add", "detail": "adds one", "assumptions": [],
                  "basis": { "kind": "fact", "call": "add" } }
            ],
            "scenarios": [
                { "kind": "concurrent", "title": "two Inc at once", "input": "two goroutines",
                  "steps": [ { "line": 6, "what": "both read c.n" }, { "line": 6, "what": "both write" } ],
                  "outcome": "one increment is lost", "assumptions": ["no lock around Inc"] }
            ]
        })
    } else {
        json!({ "notes": [ { "line": 9, "text": "returns the sum", "detail": "", "assumptions": [],
                  "basis": { "kind": "inference", "call": null } } ], "scenarios": [] })
    };
    json!({ "choices": [{ "finish_reason": "stop", "message": { "content": content.to_string() } }] }).to_string()
}

/// Answers every request by looking at which function it is about.
fn serve_llm() -> String {
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
            let body = answer_for(req["messages"][1]["content"].as_str().unwrap());
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    url
}

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    incoming: Receiver<Value>,
    next_id: i64,
}

impl Lsp {
    fn start(dir: &std::path::Path) -> Self {
        let mut child = Command::new(assert_cmd::cargo::cargo_bin("crt"))
            .arg("lsp")
            .arg("--config")
            .arg(dir.join("config.toml"))
            .arg("--cache-dir")
            .arg(dir.join("cache"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = channel();
        thread::spawn(move || {
            loop {
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if stdout.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut buf = vec![0; len];
                if stdout.read_exact(&mut buf).is_err() {
                    return;
                }
                if tx.send(serde_json::from_slice(&buf).unwrap()).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin,
            incoming: rx,
            next_id: 1,
        }
    }

    fn send(&mut self, msg: Value) {
        let text = msg.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{text}", text.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let reply = self.wait(|m| m["id"] == id && m.get("method").is_none());
        assert!(reply.get("error").is_none(), "{method} failed: {reply}");
        reply["result"].clone()
    }

    /// Waits for a message matching `pred`, answering server requests
    /// (progress creation) along the way.
    fn wait(&mut self, pred: impl Fn(&Value) -> bool) -> Value {
        loop {
            let msg = self
                .incoming
                .recv_timeout(Duration::from_secs(20))
                .expect("timed out waiting for the server");
            if msg.get("method").is_some() && msg.get("id").is_some() {
                let id = msg["id"].clone();
                self.send(json!({ "jsonrpc": "2.0", "id": id, "result": null }));
                continue;
            }
            if pred(&msg) {
                return msg;
            }
        }
    }
}

impl Drop for Lsp {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn file_readings(m: &Value) -> bool {
    m["method"] == "codeReading/fileReadings"
}

#[test]
fn facts_on_open_readings_on_view_hover_diagnostics_and_cache() {
    let dir = tempfile::tempdir().unwrap();
    let url = serve_llm();
    fs::write(
        dir.path().join("config.toml"),
        format!("[llm]\nbase_url = \"{url}\"\nmodel = \"m\"\n"),
    )
    .unwrap();
    let path = dir.path().join("p.go");
    fs::write(&path, SOURCE).unwrap();
    let uri = format!("file://{}", path.display());

    let mut lsp = Lsp::start(dir.path());
    let init = lsp.request(
        "initialize",
        json!({ "processId": null, "rootUri": null, "capabilities": { "window": { "workDoneProgress": true } },
                "initializationOptions": { "autoRead": true } }),
    );
    assert_eq!(init["capabilities"]["hoverProvider"], true);
    assert_eq!(
        init["capabilities"]["experimental"]["codeReading"]["version"],
        1
    );
    lsp.notify("initialized", json!({}));

    // Open: structural facts arrive at once, no readings yet.
    lsp.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "go", "version": 1, "text": SOURCE } }),
    );
    let opened = lsp.wait(file_readings)["params"].clone();
    let names: Vec<_> = opened["analysis"]["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].clone())
        .collect();
    assert_eq!(names, vec![json!("Inc"), json!("add")]);
    assert_eq!(opened["analysis"]["functions"][0]["enclosing"], "C");
    assert_eq!(opened["readings"], json!([null, null]));

    // Viewing the file reads the visible functions and pushes the results.
    lsp.notify(
        "codeReading/visibleRange",
        json!({ "uri": uri, "startLine": 0, "endLine": 20 }),
    );
    let done = lsp.wait(|m| {
        file_readings(m)
            && m["params"]["readings"]
                .as_array()
                .is_some_and(|r| r.iter().all(|x| !x.is_null()))
    });
    let inc = &done["params"]["readings"][0];
    assert_eq!(
        inc["notes"][0]["basis"],
        json!({ "kind": "fact", "call": "add" })
    );
    assert_eq!(inc["scenarios"][0]["kind"], "concurrent");

    // The concurrent scenario became a diagnostic linking its steps.
    let diag = lsp.wait(|m| {
        m["method"] == "textDocument/publishDiagnostics"
            && m["params"]["diagnostics"]
                .as_array()
                .is_some_and(|d| !d.is_empty())
    });
    let d = &diag["params"]["diagnostics"][0];
    assert_eq!(d["range"]["start"]["line"], 5);
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .starts_with("[guess] two Inc at once")
    );
    assert_eq!(
        d["relatedInformation"][0]["location"]["range"]["start"]["line"],
        5
    );

    // Hover on the call line shows the note, its label and the fact.
    let hover = lsp.request(
        "textDocument/hover",
        json!({ "textDocument": { "uri": uri }, "position": { "line": 5, "character": 2 } }),
    );
    let md = hover["contents"]["value"].as_str().unwrap();
    assert!(md.contains("**calls add** _(fact)_"), "{md}");
    assert!(md.contains("Calls: `add` (this file)"), "{md}");

    // An explicit read now comes from the cache.
    let read = lsp.request("codeReading/read", json!({ "uri": uri, "line": 4 }));
    assert_eq!(read["from_cache"], true);
    assert_eq!(read["function"]["name"], "Inc");

    // Editing the function invalidates only its reading.
    let edited = SOURCE.replace("c.n = add(c.n, 1)", "c.n = add(c.n, 2)");
    lsp.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": uri, "version": 2 }, "contentChanges": [ { "text": edited } ] }),
    );
    let changed = lsp.wait(|m| file_readings(m) && m["params"]["version"] == 2)["params"].clone();
    assert!(
        changed["readings"][0].is_null(),
        "the edited function has no reading"
    );
    assert!(
        changed["readings"][1].is_object(),
        "the untouched function keeps its reading"
    );

    lsp.request("shutdown", Value::Null);
    lsp.notify("exit", Value::Null);
}

#[test]
fn without_configuration_facts_still_arrive_and_reads_explain_why() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.go");
    fs::write(&path, SOURCE).unwrap();
    let uri = format!("file://{}", path.display());

    let mut lsp = Lsp::start(dir.path());
    lsp.request(
        "initialize",
        json!({ "processId": null, "rootUri": null, "capabilities": {} }),
    );
    lsp.notify("initialized", json!({}));
    lsp.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "go", "version": 1, "text": SOURCE } }),
    );
    let opened = lsp.wait(file_readings);
    assert_eq!(
        opened["params"]["analysis"]["functions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let id = lsp.next_id;
    lsp.next_id += 1;
    lsp.send(json!({ "jsonrpc": "2.0", "id": id, "method": "codeReading/read", "params": { "uri": uri, "line": 4 } }));
    let reply = lsp.wait(|m| m["id"] == id && m.get("method").is_none());
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("[llm]")
    );
}
