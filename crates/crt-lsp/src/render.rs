//! Translation from readings to LSP values: hover text and diagnostics.
//! LSP lines are 0-based; domain lines are 1-based.

use crt_domain::{Basis, Function, Reading, ScenarioKind};
use tower_lsp_server::ls_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, Position, Range, Uri,
};

/// The whole of a 1-based line.
pub(crate) fn line_range(line: usize) -> Range {
    let l = u32::try_from(line.saturating_sub(1)).unwrap_or(u32::MAX);
    Range {
        start: Position {
            line: l,
            character: 0,
        },
        end: Position {
            line: l,
            character: u32::MAX,
        },
    }
}

/// Markdown for a hover on 1-based `line` of `function`.
pub(crate) fn hover(function: &Function, reading: Option<&Reading>, line: usize) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(r) = reading {
        for n in r.notes.iter().filter(|n| n.line == line) {
            let label = match n.basis {
                Basis::Fact(_) => "fact",
                _ => "guess",
            };
            let mut s = format!("**{}** _({label})_", md(&n.text));
            if let Some(d) = &n.detail {
                s.push_str(&format!("\n\n{}", md(d)));
            }
            if !n.assumptions.is_empty() {
                s.push_str("\n\nAssumes:");
                for a in &n.assumptions {
                    s.push_str(&format!("\n- {}", md(a)));
                }
            }
            parts.push(s);
        }
        let scenarios = r.scenarios.as_deref().unwrap_or_default();
        if line == function.span.start_line && !scenarios.is_empty() {
            let mut s = String::from("**Scenarios** _(guesses)_");
            for sc in scenarios {
                s.push_str(&format!(
                    "\n- {}: {} → {}",
                    kind_name(sc.kind),
                    md(&sc.title),
                    md(&sc.outcome)
                ));
            }
            parts.push(s);
        }
    }
    if let Some(facts) = function.lines.iter().find(|l| l.line == line) {
        let calls: Vec<String> = facts
            .calls
            .iter()
            .map(|c| {
                let where_ = if c.defined_in_file {
                    "this file"
                } else {
                    "elsewhere"
                };
                format!("`{}` ({where_})", c.name)
            })
            .collect();
        parts.push(format!("Calls: {}", calls.join(", ")));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n---\n\n"))
    }
}

/// One diagnostic per concurrent scenario: at its first step, with the other
/// steps as related locations so the reader can jump between them.
pub(crate) fn diagnostics(uri: &Uri, reading: &Reading) -> Vec<Diagnostic> {
    reading
        .scenarios
        .iter()
        .flatten()
        .filter(|s| s.kind == ScenarioKind::Concurrent)
        .filter_map(|s| {
            let (first, rest) = s.steps.split_first()?;
            let related = rest
                .iter()
                .map(|step| DiagnosticRelatedInformation {
                    location: Location {
                        uri: uri.clone(),
                        range: line_range(step.line),
                    },
                    message: step.what.clone(),
                })
                .collect::<Vec<_>>();
            Some(Diagnostic {
                range: line_range(first.line),
                severity: Some(DiagnosticSeverity::INFORMATION),
                source: Some("crt".into()),
                message: format!("[guess] {}: {} ({})", s.title, s.outcome, first.what),
                related_information: (!related.is_empty()).then_some(related),
                ..Diagnostic::default()
            })
        })
        .collect()
}

/// Escapes text written by the model so it shows as text: no links, images,
/// HTML or emphasis it did not ask for in plain words.
fn md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '`'
                | '*'
                | '_'
                | '{'
                | '}'
                | '['
                | ']'
                | '('
                | ')'
                | '#'
                | '+'
                | '-'
                | '.'
                | '!'
                | '|'
                | '<'
                | '>'
                | '~'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn kind_name(k: ScenarioKind) -> &'static str {
    match k {
        ScenarioKind::Normal => "normal",
        ScenarioKind::Boundary => "boundary",
        ScenarioKind::Concurrent => "concurrent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crt_domain::{
        Author, Call, CheckReport, ContentHash, FactRef, LineFacts, Note, Scenario, Span, Step,
        SymbolKind,
    };

    fn function() -> Function {
        Function {
            name: "f".into(),
            kind: SymbolKind::Function,
            enclosing: None,
            span: Span {
                start_byte: 0,
                end_byte: 1,
                start_line: 2,
                end_line: 6,
            },
            hash: ContentHash::of(b"f"),
            docs: None,
            lines: vec![LineFacts {
                line: 3,
                calls: vec![Call {
                    name: "g".into(),
                    defined_in_file: false,
                }],
            }],
        }
    }

    fn reading() -> Reading {
        Reading {
            function_hash: ContentHash::of(b"f"),
            author: Author {
                model: "m".into(),
                prompt: "v1".into(),
            },
            notes: vec![
                Note {
                    line: 3,
                    text: "calls g".into(),
                    detail: Some("more".into()),
                    assumptions: vec!["g is cheap".into()],
                    basis: Basis::Fact(FactRef::Call { name: "g".into() }),
                },
                Note {
                    line: 3,
                    text: "then loops".into(),
                    detail: None,
                    assumptions: vec![],
                    basis: Basis::Inference,
                },
            ],
            scenarios: Some(vec![Scenario {
                kind: ScenarioKind::Concurrent,
                title: "two writers".into(),
                input: "two calls".into(),
                steps: vec![
                    Step {
                        line: 3,
                        what: "A reads".into(),
                    },
                    Step {
                        line: 5,
                        what: "B writes".into(),
                    },
                ],
                outcome: "lost update".into(),
                assumptions: vec![],
            }]),
            check: CheckReport::default(),
        }
    }

    #[test]
    fn hover_shows_notes_labels_assumptions_and_facts() {
        let md = hover(&function(), Some(&reading()), 3).unwrap();
        assert!(md.contains("**calls g** _(fact)_"));
        assert!(md.contains("**then loops** _(guess)_"));
        assert!(md.contains("- g is cheap"));
        assert!(md.contains("Calls: `g` (elsewhere)"));
        assert!(
            hover(&function(), Some(&reading()), 2)
                .unwrap()
                .contains("concurrent: two writers → lost update")
        );
        assert!(hover(&function(), None, 4).is_none());
    }

    #[test]
    fn model_text_cannot_inject_links_images_or_html() {
        let mut r = reading();
        r.notes[0].text = "see ![x](https://evil/?d=1) <img src=x>".into();
        let md = hover(&function(), Some(&r), 3).unwrap();
        assert!(!md.contains("![x](https"), "{md}");
        assert!(md.contains("\\!\\[x\\]\\(https"), "{md}");
        assert!(md.contains("\\<img src=x\\>"), "{md}");
    }

    #[test]
    fn a_concurrent_scenario_becomes_a_diagnostic_linking_its_steps() {
        let uri: Uri = "file:///tmp/a.go".parse().unwrap();
        let d = diagnostics(&uri, &reading());
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].range.start.line, 2);
        assert!(d[0].message.starts_with("[guess] two writers"));
        let related = d[0].related_information.as_ref().unwrap();
        assert_eq!(related[0].location.range.start.line, 4);
    }
}
