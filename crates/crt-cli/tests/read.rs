//! `crt read` end to end against a scripted OpenAI-compatible server.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;

use assert_cmd::Command;
use serde_json::{Value, json};

/// Serves exactly `bodies.len()` requests, then stops listening.
fn serve(bodies: Vec<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    thread::spawn(move || {
        for body in bodies {
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
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    url
}

const SOURCE: &str = "package p\n\ntype C struct{}\n\nfunc (c *C) Copy() *C {\n\treturn helper(c)\n}\n\nfunc helper(c *C) *C { return c }\n";

fn answer() -> String {
    let content = json!({
        "notes": [
            { "line": 6, "text": "calls helper", "detail": "", "assumptions": [],
              "basis": { "kind": "fact", "call": "helper" } },
            { "line": 5, "text": "claims a call that is not on this line", "detail": "", "assumptions": [],
              "basis": { "kind": "fact", "call": "helper" } },
            { "line": 6, "text": "returns a copy", "detail": "", "assumptions": ["helper does not mutate c"],
              "basis": { "kind": "inference", "call": null } },
            { "line": 42, "text": "outside the function", "detail": "", "assumptions": [],
              "basis": { "kind": "inference", "call": null } }
        ],
        "scenarios": [
            { "kind": "boundary", "title": "nil receiver", "input": "c == nil",
              "steps": [{ "line": 6, "what": "helper gets nil" }], "outcome": "returns nil", "assumptions": [] }
        ]
    })
    .to_string();
    json!({ "choices": [{ "finish_reason": "stop", "message": { "role": "assistant", "content": content } }] })
        .to_string()
}

fn run(dir: &std::path::Path, args: &[&str]) -> Value {
    let out = Command::cargo_bin("crt")
        .unwrap()
        .args(args)
        .arg("--config")
        .arg(dir.join("config.toml"))
        .arg("--cache-dir")
        .arg(dir.join("cache"))
        .assert()
        .success();
    serde_json::from_slice(&out.get_output().stdout).unwrap()
}

#[test]
fn reads_checks_caches_and_lists() {
    let dir = tempfile::tempdir().unwrap();
    let url = serve(vec![answer()]);
    fs::write(
        dir.path().join("config.toml"),
        format!(
            "[llm]\nbase_url = \"{url}\"\nmodel = \"test-model\"\noutput_language = \"Japanese\"\n"
        ),
    )
    .unwrap();
    let file = dir.path().join("x.go");
    fs::write(&file, SOURCE).unwrap();
    let file = file.to_str().unwrap();

    let first = run(dir.path(), &["read", file, "--func", "Copy"]);
    assert_eq!(first["from_cache"], false);
    assert_eq!(first["function"]["enclosing"], "C");
    let r = &first["reading"];
    assert_eq!(r["model"], "test-model");
    assert_eq!(r["prompt"], "v1-japanese");
    assert_eq!(r["demoted"], 1, "the false fact on line 5 is demoted");
    assert_eq!(r["dropped"], 1, "the note on line 42 is dropped");
    let notes = r["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 3);
    assert_eq!(notes[0]["line"], 5);
    assert_eq!(notes[0]["basis"]["kind"], "inference");
    assert_eq!(
        notes[1]["basis"],
        json!({ "kind": "fact", "call": "helper" })
    );
    assert_eq!(r["scenarios"][0]["kind"], "boundary");

    // The server has stopped; this can only succeed from the cache.
    let second = run(dir.path(), &["read", file, "--line", "6"]);
    assert_eq!(second["from_cache"], true);
    assert_eq!(second["reading"], first["reading"]);

    let listed = run(dir.path(), &["cached", file]);
    let names: Vec<_> = listed["analysis"]["functions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, vec!["Copy", "helper"]);
    assert!(listed["readings"][0].is_object());
    assert!(listed["readings"][1].is_null());

    // HTML from the cache only: the server is gone, so nothing is asked.
    let out = dir.path().join("p.html");
    Command::cargo_bin("crt")
        .unwrap()
        .args(["render", file, "--out"])
        .arg(&out)
        .arg("--config")
        .arg(dir.path().join("config.toml"))
        .arg("--cache-dir")
        .arg(dir.path().join("cache"))
        .assert()
        .success();
    let html = fs::read_to_string(out).unwrap();
    assert!(html.contains("method C.Copy"));
    assert!(html.contains(r#"<span class="n fact">calls helper</span>"#));
    assert!(html.contains("nil receiver"));
    assert!(html.contains("Run <code>crt read --line 9</code>"));
}

#[test]
fn a_missing_configuration_explains_what_to_write() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x.go");
    fs::write(&file, SOURCE).unwrap();
    Command::cargo_bin("crt")
        .unwrap()
        .args(["read", file.to_str().unwrap(), "--func", "Copy", "--config"])
        .arg(dir.path().join("missing.toml"))
        .assert()
        .failure()
        .stderr(predicates::str::contains("[llm]"));
}
