//! Runs the Neovim plugin headless against `crt lsp` and a scripted model
//! server. Skipped when `nvim` is not on PATH.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;

use serde_json::{Value, json};

const SOURCE: &str = "package p\n\ntype C struct{ n int }\n\nfunc (c *C) Inc() {\n\tc.n = add(c.n, 1)\n}\n\nfunc add(a, b int) int { return a + b }\n";

fn answer_for(user: &str) -> String {
    let content = if user.contains("FUNCTION: Inc") {
        json!({
            "notes": [ { "line": 6, "text": "calls add", "detail": "adds one", "assumptions": [],
                         "basis": { "kind": "fact", "call": "add" } } ],
            "scenarios": [ { "kind": "concurrent", "title": "two Inc at once", "input": "two goroutines",
                             "steps": [ { "line": 6, "what": "both read c.n" }, { "line": 7, "what": "both return" } ],
                             "outcome": "one increment is lost", "assumptions": ["no lock"] } ]
        })
    } else {
        json!({ "notes": [ { "line": 9, "text": "returns the sum", "detail": "", "assumptions": [],
                             "basis": { "kind": "inference", "call": null } } ], "scenarios": [] })
    };
    json!({ "choices": [{ "finish_reason": "stop", "message": { "content": content.to_string() } }] }).to_string()
}

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

#[test]
fn neovim_plugin_end_to_end() {
    if Command::new("nvim").arg("--version").output().is_err() {
        eprintln!("skipping: nvim not found");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let url = serve_llm();
    let config = dir.path().join("config.toml");
    fs::write(
        &config,
        format!("[llm]\nbase_url = \"{url}\"\nmodel = \"m\"\n"),
    )
    .unwrap();
    let file = dir.path().join("p.go");
    fs::write(&file, SOURCE).unwrap();
    let plugin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../editors/nvim");

    let out = Command::new("nvim")
        .args(["--headless", "--clean", "-l"])
        .arg(plugin.join("tests/e2e.lua"))
        .env("CRT_BIN", assert_cmd::cargo::cargo_bin("crt"))
        .env("CRT_CONFIG", &config)
        .env("CRT_CACHE", dir.path().join("cache"))
        .env("CRT_FILE", &file)
        .env("CRT_PLUGIN", &plugin)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("OK"),
        "nvim e2e failed\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// :CrInstall's logic: a published archive with a matching checksum
/// installs a binary that runs; a tampered one is refused.
#[test]
fn neovim_installer_checks_the_checksum() {
    use sha2::{Digest, Sha256};

    if Command::new("nvim").arg("--version").output().is_err() {
        eprintln!("skipping: nvim not found");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = "test-target";
    let asset = format!("crt-{target}.tar.gz");
    let staging = dir.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    fs::copy(assert_cmd::cargo::cargo_bin("crt"), staging.join("crt")).unwrap();

    for (version, tamper) in [("9.9.9", false), ("6.6.6", true)] {
        let rel = dir.path().join("release").join(format!("v{version}"));
        fs::create_dir_all(&rel).unwrap();
        let archive = rel.join(&asset);
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&staging)
            .arg("crt")
            .status()
            .unwrap();
        assert!(status.success());
        let mut digest: String = Sha256::digest(fs::read(&archive).unwrap())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if tamper {
            digest = "0".repeat(64);
        }
        fs::write(
            rel.join(format!("{asset}.sha256")),
            format!("{digest}  {asset}\n"),
        )
        .unwrap();
    }

    let plugin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../editors/nvim");
    let out = Command::new("nvim")
        .args(["--headless", "--clean", "-l"])
        .arg(plugin.join("tests/install.lua"))
        .env("CRT_PLUGIN", &plugin)
        .env(
            "CRT_RELEASE",
            format!("file://{}", dir.path().join("release").display()),
        )
        .env("CRT_TARGET", target)
        .env("XDG_DATA_HOME", dir.path().join("data"))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("OK"),
        "install test failed\nstdout: {stdout}\nstderr: {stderr}"
    );
}
