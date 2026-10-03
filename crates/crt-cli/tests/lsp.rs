//! `crt lsp` end to end: a minimal LSP client over the child's stdio, and a
//! scripted OpenAI-compatible server behind it.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

mod common;
use common::{SOURCE, serve_llm, serve_llm_counting};

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
            if std::env::var_os("CRT_TEST_TRACE").is_some() {
                eprintln!("<- {msg}");
            }
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
        2
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

    // Viewing the file reads the visible functions. Notes are pushed while
    // they arrive, then the finished readings.
    lsp.notify(
        "codeReading/visibleRange",
        json!({ "uri": uri, "startLine": 0, "endLine": 20 }),
    );
    let partial = lsp.wait(|m| {
        file_readings(m)
            && m["params"]["partial"].as_array().is_some_and(|p| {
                p.iter()
                    .any(|x| x["notes"].as_array().is_some_and(|n| n.len() == 1))
            })
    })["params"]
        .clone();
    let first = partial["partial"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["notes"].as_array().is_some_and(|n| n.len() == 1))
        .unwrap();
    assert_eq!(
        first["functionHash"],
        opened["analysis"]["functions"][0]["hash"]
    );
    assert_eq!(first["notes"][0]["text"], "increments c.n");
    let done = lsp.wait(|m| {
        file_readings(m)
            && m["params"]["readings"]
                .as_array()
                .is_some_and(|r| r.iter().all(|x| !x.is_null()))
    });
    assert_eq!(done["params"]["partial"], json!([]));
    let inc = &done["params"]["readings"][0];
    assert_eq!(
        inc["notes"][1]["basis"],
        json!({ "kind": "fact", "call": "add" })
    );
    assert!(inc["scenarios"].is_null(), "scenarios wait to be asked for");

    // Asking for scenarios writes them and keeps them with the notes; the
    // concurrent one becomes a diagnostic linking its steps. The server
    // publishes before it replies.
    let id = lsp.next_id;
    lsp.next_id += 1;
    lsp.send(
        json!({ "jsonrpc": "2.0", "id": id, "method": "codeReading/scenarios",
                     "params": { "uri": uri, "line": 4 } }),
    );
    let diag = lsp.wait(|m| {
        m["method"] == "textDocument/publishDiagnostics"
            && m["params"]["diagnostics"]
                .as_array()
                .is_some_and(|d| !d.is_empty())
    });
    let sc = lsp.wait(|m| m["id"] == id && m.get("method").is_none())["result"].clone();
    assert_eq!(sc["reading"]["scenarios"][0]["kind"], "concurrent");
    assert_eq!(sc["reading"]["notes"].as_array().unwrap().len(), 2);
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

#[test]
fn scenarios_asked_while_the_notes_are_being_written_wait_for_them() {
    let dir = tempfile::tempdir().unwrap();
    let (url, notes_requests) = serve_llm_counting();
    fs::write(
        dir.path().join("config.toml"),
        format!("[llm]\nbase_url = \"{url}\"\nmodel = \"m\"\n"),
    )
    .unwrap();
    let path = dir.path().join("p.go");
    fs::write(&path, SOURCE).unwrap();
    let uri = format!("file://{}", path.display());

    let mut lsp = Lsp::start(dir.path());
    lsp.request(
        "initialize",
        json!({ "processId": null, "rootUri": null, "capabilities": {},
                "initializationOptions": { "autoRead": true, "maxParallel": 2 } }),
    );
    lsp.notify("initialized", json!({}));
    lsp.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": uri, "languageId": "go", "version": 1, "text": SOURCE } }),
    );
    lsp.wait(file_readings);
    // The notes of Inc take over half a second; ask for its scenarios
    // while they are still being written.
    lsp.notify(
        "codeReading/visibleRange",
        json!({ "uri": uri, "startLine": 0, "endLine": 20 }),
    );
    lsp.wait(|m| {
        file_readings(m)
            && m["params"]["partial"]
                .as_array()
                .is_some_and(|p| !p.is_empty())
    });
    let sc = lsp.request("codeReading/scenarios", json!({ "uri": uri, "line": 4 }));
    assert_eq!(sc["reading"]["notes"].as_array().unwrap().len(), 2);
    assert!(sc["reading"]["scenarios"].is_array());
    // Inc and add: one notes request each (add's finished before the
    // reply), none repeated for the scenarios.
    assert_eq!(notes_requests.load(std::sync::atomic::Ordering::SeqCst), 2);
}
