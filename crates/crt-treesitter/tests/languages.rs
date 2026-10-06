//! For every bundled language: a small class with a method and a free
//! function, read through the whole path (adapter + domain rules). Each
//! case checks what the reader relies on: the function's full line range,
//! the type a method belongs to, and the calls on each line.

use std::path::Path;

use crt_app::analyze_file;
use crt_treesitter::TreeSitterSource;

/// (name, enclosing, start_line, end_line, [(line, [calls])])
type Expect<'a> = (
    &'a str,
    Option<&'a str>,
    usize,
    usize,
    Vec<(usize, Vec<&'a str>)>,
);

fn check(file: &str, source: &str, expected: &[Expect<'_>]) {
    let analysis = analyze_file(&TreeSitterSource, Path::new(file), source.as_bytes())
        .unwrap_or_else(|e| panic!("{file}: {e}: {:?}", std::error::Error::source(&e)));
    assert!(
        !analysis.has_syntax_error,
        "{file}: unexpected syntax error"
    );
    let got: Vec<Expect<'_>> = analysis
        .functions
        .iter()
        .map(|f| {
            (
                f.name.as_str(),
                f.enclosing.as_deref(),
                f.span.start_line,
                f.span.end_line,
                f.lines
                    .iter()
                    .map(|l| (l.line, l.calls.iter().map(|c| c.name.as_str()).collect()))
                    .collect(),
            )
        })
        .collect();
    assert_eq!(got, expected, "{file}");
}

#[test]
fn typescript() {
    let src = "class Repo {\n  save(x: Item): void {\n    validate(x);\n    this.store.put(x);\n  }\n}\n\nfunction validate(x: Item) {\n  return check(x);\n}\n\nconst helper = (n: number) => double(n);\n";
    check(
        "a.ts",
        src,
        &[
            (
                "save",
                Some("Repo"),
                2,
                5,
                vec![(3, vec!["validate"]), (4, vec!["put"])],
            ),
            ("validate", None, 8, 10, vec![(9, vec!["check"])]),
            ("helper", None, 12, 12, vec![(12, vec!["double"])]),
        ],
    );
}

#[test]
fn tsx() {
    let src = "function View(p: Props) {\n  const n = count(p);\n  return <div>{n}</div>;\n}\n";
    check(
        "a.tsx",
        src,
        &[("View", None, 1, 4, vec![(2, vec!["count"])])],
    );
}

#[test]
fn java() {
    let src = "class Repo {\n  void save(Item x) {\n    validate(x);\n    store.put(x);\n  }\n}\n";
    check(
        "A.java",
        src,
        &[(
            "save",
            Some("Repo"),
            2,
            5,
            vec![(3, vec!["validate"]), (4, vec!["put"])],
        )],
    );
}

#[test]
fn c() {
    let src = "struct counter { int n; };\n\nint add(int a, int b) {\n  return a + b;\n}\n\nvoid inc(struct counter *c) {\n  c->n = add(c->n, 1);\n  log_it(c);\n}\n\nchar *name(void) { return dup(\"x\"); }\n";
    check(
        "a.c",
        src,
        &[
            ("add", None, 3, 5, vec![]),
            (
                "inc",
                None,
                7,
                10,
                vec![(8, vec!["add"]), (9, vec!["log_it"])],
            ),
            ("name", None, 12, 12, vec![(12, vec!["dup"])]),
        ],
    );
}

#[test]
fn cpp() {
    let src = "class Counter {\n public:\n  void inc() {\n    n = add(n, 1);\n  }\n  int n;\n};\n\nvoid Counter::reset() {\n  n = 0;\n  log::write(n);\n}\n\nint add(int a, int b) { return a + b; }\n";
    check(
        "a.cpp",
        src,
        &[
            ("inc", Some("Counter"), 3, 5, vec![(4, vec!["add"])]),
            ("reset", Some("Counter"), 9, 12, vec![(11, vec!["write"])]),
            ("add", None, 14, 14, vec![]),
        ],
    );
}

#[test]
fn csharp() {
    let src = "class Repo {\n  void Save(Item x) {\n    Validate(x);\n    store.Put(x);\n  }\n}\n";
    check(
        "A.cs",
        src,
        &[(
            "Save",
            Some("Repo"),
            2,
            5,
            vec![(3, vec!["Validate"]), (4, vec!["Put"])],
        )],
    );
}

#[test]
fn ruby() {
    let src = "class Repo\n  def save(x)\n    validate(x)\n    store.put(x)\n  end\nend\n";
    // In Ruby `store` with no arguments is itself a method call.
    check(
        "a.rb",
        src,
        &[(
            "save",
            Some("Repo"),
            2,
            5,
            vec![(3, vec!["validate"]), (4, vec!["store", "put"])],
        )],
    );
}

#[test]
fn php() {
    let src = "<?php\nclass Repo {\n  function save($x) {\n    validate($x);\n    $this->store->put($x);\n  }\n}\n\nfunction validate($x) {\n  return check($x);\n}\n";
    check(
        "a.php",
        src,
        &[
            (
                "save",
                Some("Repo"),
                3,
                6,
                vec![(4, vec!["validate"]), (5, vec!["put"])],
            ),
            ("validate", None, 9, 11, vec![(10, vec!["check"])]),
        ],
    );
}

#[test]
fn bash() {
    // Both definition forms; `$runner` is an expansion, not a call by name.
    let src = "#!/usr/bin/env bash\nsave() {\n  validate \"$1\"\n  \"$runner\" put \"$1\"\n}\n\nfunction validate {\n  check \"$1\" || return 1\n}\n";
    check(
        "a.sh",
        src,
        &[
            ("save", None, 2, 5, vec![(3, vec!["validate"])]),
            ("validate", None, 7, 9, vec![(8, vec!["check"])]),
        ],
    );
}
