use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;

fn crt() -> Command {
    Command::cargo_bin("crt").expect("crt binary")
}

#[test]
fn languages_lists_the_bundled_set() {
    crt()
        .arg("languages")
        .assert()
        .success()
        .stdout(predicate::str::contains("go\tgo"))
        .stdout(predicate::str::contains("rust\trs"));
}

#[test]
fn analyze_prints_wire_json_for_one_function() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x.go");
    fs::write(&file, "package p\n\ntype C struct{}\n\nfunc (c *C) Copy() *C {\n\treturn helper(c)\n}\n\nfunc helper(c *C) *C { return c }\n").unwrap();
    let out = crt()
        .arg("analyze")
        .arg(&file)
        .arg("--func")
        .arg("Copy")
        .assert()
        .success();
    let json: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    assert_eq!(json["wire_version"], 2);
    assert_eq!(json["language"], "go");
    assert_eq!(json["has_syntax_error"], false);
    assert_eq!(json["functions"].as_array().unwrap().len(), 1);
    let f = &json["functions"][0];
    assert_eq!(f["kind"], "method");
    assert_eq!(f["enclosing"], "C");
    assert_eq!(f["start_line"], 5);
    assert_eq!(f["end_line"], 7);
    assert_eq!(f["lines"][0]["line"], 6);
    assert_eq!(f["lines"][0]["calls"][0]["defined_in_file"], true);
}

#[test]
fn analyze_warns_on_syntax_errors_but_still_answers() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("broken.rs");
    fs::write(&file, "fn ok() {}\nfn broken( {\n").unwrap();
    crt()
        .arg("analyze")
        .arg(&file)
        .assert()
        .success()
        .stderr(predicate::str::contains("syntax errors"))
        .stdout(predicate::str::contains("\"has_syntax_error\": true"));
}

#[test]
fn analyze_fails_clearly_for_a_missing_function_and_an_unknown_extension() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x.py");
    fs::write(&file, "def a():\n    pass\n").unwrap();
    crt()
        .arg("analyze")
        .arg(&file)
        .arg("--func")
        .arg("nope")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no function named nope"));

    let other = dir.path().join("notes.txt");
    fs::write(&other, "hello").unwrap();
    crt()
        .arg("analyze")
        .arg(&other)
        .assert()
        .failure()
        .stderr(predicate::str::contains("no structure source for"));
}
