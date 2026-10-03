//! One static HTML page for a file: source with per-line notes, facts and
//! scenarios. Every string from the model or the source is escaped by the
//! template engine (`.html` templates auto-escape).

use minijinja::{Environment, context};
use serde::Serialize;

use crt_wire::{BasisDto, FileAnalysisDto, FunctionDto, ReadingDto, ScenarioDto};

const PAGE: &str = include_str!("../templates/page.html");

/// What the page shows.
pub struct Page<'a> {
    /// Shown as the heading, e.g. the file's path.
    pub title: &'a str,
    pub source: &'a str,
    pub analysis: &'a FileAnalysisDto,
    /// One per function in `analysis.functions`.
    pub readings: &'a [Option<ReadingDto>],
}

#[derive(Serialize)]
struct NoteView {
    style: &'static str,
    text: String,
    detail: Option<String>,
    assumptions: Vec<String>,
}

#[derive(Serialize)]
struct Row {
    line: usize,
    text: String,
    notes: Vec<NoteView>,
    calls: Vec<String>,
}

#[derive(Serialize)]
struct FunctionView<'a> {
    anchor: String,
    kind: &'a str,
    name: &'a str,
    enclosing: Option<&'a str>,
    start_line: usize,
    end_line: usize,
    read: bool,
    checked: Option<String>,
    rows: Vec<Row>,
    scenarios: &'a [ScenarioDto],
}

/// Renders the page.
pub fn render(page: &Page<'_>) -> Result<String, minijinja::Error> {
    let lines: Vec<&str> = page.source.lines().collect();
    let functions: Vec<FunctionView<'_>> = page
        .analysis
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| view(f, page.readings.get(i).and_then(Option::as_ref), &lines))
        .collect();
    let mut models: Vec<&str> = page
        .readings
        .iter()
        .flatten()
        .map(|r| r.model.as_str())
        .collect();
    models.sort_unstable();
    models.dedup();

    let mut env = Environment::new();
    env.add_template("page.html", PAGE)?;
    env.get_template("page.html")?.render(context! {
        title => page.title,
        language => page.analysis.language,
        has_syntax_error => page.analysis.has_syntax_error,
        models => models,
        functions => functions,
    })
}

fn view<'a>(
    f: &'a FunctionDto,
    reading: Option<&'a ReadingDto>,
    lines: &[&str],
) -> FunctionView<'a> {
    let rows = (f.start_line..=f.end_line)
        .map(|line| {
            let notes = reading
                .map(|r| {
                    r.notes
                        .iter()
                        .filter(|n| n.line == line)
                        .map(|n| NoteView {
                            style: match n.basis {
                                BasisDto::Fact { .. } => "fact",
                                BasisDto::Inference => "guess",
                            },
                            text: n.text.clone(),
                            detail: n.detail.clone(),
                            assumptions: n.assumptions.clone(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let calls = if notes.is_empty() {
                f.lines
                    .iter()
                    .find(|l| l.line == line)
                    .map(|l| l.calls.iter().map(|c| c.name.clone()).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            };
            Row {
                line,
                text: lines
                    .get(line.saturating_sub(1))
                    .copied()
                    .unwrap_or("")
                    .to_string(),
                notes,
                calls,
            }
        })
        .collect();
    let checked = reading.and_then(|r| {
        let mut parts = vec![];
        if r.demoted > 0 {
            parts.push(format!("{} claimed fact(s) demoted to guesses", r.demoted));
        }
        if r.dropped > 0 {
            parts.push(format!(
                "{} note(s) outside the function dropped",
                r.dropped
            ));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    });
    FunctionView {
        anchor: format!("L{}", f.start_line),
        kind: match f.kind {
            crt_wire::FunctionKindDto::Method => "method",
            crt_wire::FunctionKindDto::Function => "function",
        },
        name: &f.name,
        enclosing: f.enclosing.as_deref(),
        start_line: f.start_line,
        end_line: f.end_line,
        read: reading.is_some(),
        checked,
        rows,
        scenarios: reading.map(|r| r.scenarios.as_slice()).unwrap_or(&[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn analysis() -> FileAnalysisDto {
        serde_json::from_value(json!({
            "wire_version": 1, "language": "go", "has_syntax_error": false,
            "functions": [
                { "name": "Inc", "kind": "method", "enclosing": "C", "start_line": 2, "end_line": 4,
                  "hash": "h", "docs": null,
                  "lines": [ { "line": 3, "calls": [ { "name": "add", "defined_in_file": true } ] } ] },
                { "name": "add", "kind": "function", "enclosing": null, "start_line": 6, "end_line": 6,
                  "hash": "h2", "docs": null, "lines": [] }
            ]
        }))
        .unwrap()
    }

    fn reading() -> ReadingDto {
        serde_json::from_value(json!({
            "function_hash": "h", "model": "m", "prompt": "v1-english",
            "notes": [
                { "line": 3, "text": "calls add <script>alert(1)</script>", "detail": "d", "assumptions": ["a & b"],
                  "basis": { "kind": "fact", "call": "add" } },
                { "line": 4, "text": "returns", "detail": null, "assumptions": [], "basis": { "kind": "inference" } }
            ],
            "scenarios": [ { "kind": "concurrent", "title": "two at once", "input": "x",
                             "steps": [ { "line": 3, "what": "both read" } ], "outcome": "lost", "assumptions": [] } ],
            "demoted": 1, "dropped": 0
        }))
        .unwrap()
    }

    #[test]
    fn renders_notes_scenarios_and_escapes_model_text() {
        let source = "package p\nfunc (c *C) Inc() {\n\tc.n = add(c.n, 1)\n}\n\nfunc add(a, b int) int { return a + b }\n";
        let html = render(&Page {
            title: "p.go",
            source,
            analysis: &analysis(),
            readings: &[Some(reading()), None],
        })
        .unwrap();
        assert!(
            !html.contains("<script>alert(1)</script>"),
            "model text must be escaped"
        );
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains(r#"<span class="n fact">"#));
        assert!(html.contains(r#"<span class="n guess">returns</span>"#));
        assert!(html.contains("method C.Inc"));
        assert!(html.contains("two at once"));
        assert!(html.contains("1 claimed fact(s) demoted to guesses"));
        assert!(html.contains("a &amp; b"));
        assert!(html.contains("Run <code>crt read --line 6</code>"));
        assert!(html.contains("c.n = add(c.n, 1)"));
    }
}
