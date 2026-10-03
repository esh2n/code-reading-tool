//! The prompt, the output schema, and the translation of the model's answer
//! into a domain draft. Change `VERSION` whenever any of these change: it is
//! part of the cache key.

use serde::Deserialize;
use serde_json::{Value, json};

use crt_app::ExplainRequest;
use crt_domain::{Basis, Draft, FactRef, Note, Scenario, ScenarioKind, Step, SymbolKind};

pub(crate) const VERSION: u32 = 1;

/// The cache-key form of the prompt: version plus output language.
pub(crate) fn identity(output_language: &str) -> String {
    let lang: String = output_language
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    format!("v{VERSION}-{lang}")
}

pub(crate) fn system(output_language: &str) -> String {
    format!(
        "You explain source code to a reader who is reading it in an editor. \
You never run the code; you reason about how it behaves.

Write in {output_language}.

Produce two things for the one function you are given:

1. notes: explanations attached to lines of that function.
   - Use only line numbers shown in the numbered source.
   - One note per line that does something worth knowing; skip braces and trivial lines.
   - text: one short sentence (at most 80 characters) saying what the line does.
   - detail: a longer explanation for a hover, or an empty string.
   - basis.kind = \"fact\" only when the note merely states that the line calls a function
     listed under STRUCTURAL FACTS for that same line; then basis.call is that function's name.
     Everything else is basis.kind = \"inference\" with basis.call = null.
   - assumptions: what the explanation takes for granted (inputs, caller behaviour,
     what an unseen function does). Empty when nothing is assumed.

2. scenarios: how the function behaves on concrete inputs.
   - At least one normal case.
   - Boundary cases that matter for this code: empty, zero, negative, maximum, null/None/nil,
     malformed input.
   - Concurrent cases when the code touches state that two callers could share at once:
     say which lines interleave and what goes wrong (lost update, data race, double submit).
     Omit concurrent scenarios when nothing is shared.
   - steps: the lines the input goes through, in order, each with what happens there.
   - outcome: the result or failure.
   - assumptions: what the scenario takes for granted.

Do not invent lines, functions or behaviour you cannot see; when you must guess, say so in
assumptions."
    )
}

pub(crate) fn user(request: &ExplainRequest) -> String {
    let f = &request.function;
    let kind = match f.kind {
        SymbolKind::Method => "method",
        _ => "function",
    };
    let mut out = format!(
        "LANGUAGE: {}\nFUNCTION: {} ({kind})",
        request.language, f.name
    );
    if let Some(owner) = &f.enclosing {
        out.push_str(&format!(" of {owner}"));
    }
    out.push_str("\n\nNUMBERED SOURCE:\n");
    out.push_str(&request.numbered_source);
    out.push_str("\n\nSTRUCTURAL FACTS (read off the syntax tree; calls are matched by name):\n");
    if f.lines.is_empty() {
        out.push_str("(none)\n");
    }
    for l in &f.lines {
        for c in &l.calls {
            let where_ = if c.defined_in_file {
                "defined in this file"
            } else {
                "defined elsewhere"
            };
            out.push_str(&format!("line {}: calls {} ({where_})\n", l.line, c.name));
        }
    }
    if !request.callers.is_empty() {
        out.push_str("\nCALLERS IN THIS FILE (found by name; may include wrong matches):\n");
        for c in &request.callers {
            let owner = c
                .enclosing
                .as_deref()
                .map(|o| format!(" of {o}"))
                .unwrap_or_default();
            out.push_str(&format!("--- {}{owner}\n{}\n", c.name, c.numbered_source));
        }
    }
    out
}

/// JSON Schema for the answer, in the strict subset (every property
/// required, no extra properties).
pub(crate) fn schema() -> Value {
    let strings = json!({ "type": "array", "items": { "type": "string" } });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["notes", "scenarios"],
        "properties": {
            "notes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["line", "text", "detail", "assumptions", "basis"],
                    "properties": {
                        "line": { "type": "integer" },
                        "text": { "type": "string" },
                        "detail": { "type": "string" },
                        "assumptions": strings,
                        "basis": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["kind", "call"],
                            "properties": {
                                "kind": { "type": "string", "enum": ["fact", "inference"] },
                                "call": { "type": ["string", "null"] }
                            }
                        }
                    }
                }
            },
            "scenarios": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["kind", "title", "input", "steps", "outcome", "assumptions"],
                    "properties": {
                        "kind": { "type": "string", "enum": ["normal", "boundary", "concurrent"] },
                        "title": { "type": "string" },
                        "input": { "type": "string" },
                        "steps": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["line", "what"],
                                "properties": {
                                    "line": { "type": "integer" },
                                    "what": { "type": "string" }
                                }
                            }
                        },
                        "outcome": { "type": "string" },
                        "assumptions": strings
                    }
                }
            }
        }
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    notes: Vec<AnswerNote>,
    scenarios: Vec<AnswerScenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerNote {
    line: usize,
    text: String,
    detail: String,
    assumptions: Vec<String>,
    basis: AnswerBasis,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerBasis {
    kind: AnswerBasisKind,
    call: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AnswerBasisKind {
    Fact,
    Inference,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerScenario {
    kind: AnswerScenarioKind,
    title: String,
    input: String,
    steps: Vec<AnswerStep>,
    outcome: String,
    assumptions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AnswerScenarioKind {
    Normal,
    Boundary,
    Concurrent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerStep {
    line: usize,
    what: String,
}

/// Some models HTML-escape inside JSON strings (`&amp;str` for `&str`).
/// The text is shown in editors and HTML that escape for themselves, so the
/// entities are turned back into characters here.
fn unescape(text: String) -> String {
    if !text.contains('&') {
        return text;
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

fn unescape_all(v: Vec<String>) -> Vec<String> {
    v.into_iter().map(unescape).collect()
}

/// Parses the model's JSON into a draft. A "fact" without a call name is
/// treated as an inference; the domain checker verifies the rest.
pub(crate) fn parse(content: &str) -> Result<Draft, String> {
    let answer: Answer = serde_json::from_str(content).map_err(|e| e.to_string())?;
    Ok(Draft {
        notes: answer
            .notes
            .into_iter()
            .map(|n| Note {
                line: n.line,
                text: unescape(n.text),
                detail: Some(unescape(n.detail)).filter(|d| !d.trim().is_empty()),
                assumptions: unescape_all(n.assumptions),
                basis: match (n.basis.kind, n.basis.call) {
                    (AnswerBasisKind::Fact, Some(name)) if !name.is_empty() => {
                        Basis::Fact(FactRef::Call { name })
                    }
                    _ => Basis::Inference,
                },
            })
            .collect(),
        scenarios: answer
            .scenarios
            .into_iter()
            .map(|s| Scenario {
                kind: match s.kind {
                    AnswerScenarioKind::Normal => ScenarioKind::Normal,
                    AnswerScenarioKind::Boundary => ScenarioKind::Boundary,
                    AnswerScenarioKind::Concurrent => ScenarioKind::Concurrent,
                },
                title: unescape(s.title),
                input: unescape(s.input),
                steps: s
                    .steps
                    .into_iter()
                    .map(|st| Step {
                        line: st.line,
                        what: unescape(st.what),
                    })
                    .collect(),
                outcome: unescape(s.outcome),
                assumptions: unescape_all(s.assumptions),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_folds_the_output_language_into_a_safe_token() {
        assert_eq!(identity("Japanese"), "v1-japanese");
        assert_eq!(identity("pt-BR"), "v1-ptbr");
    }

    #[test]
    fn parses_an_answer_and_treats_a_nameless_fact_as_inference() {
        let content = r#"{
          "notes": [
            {"line": 3, "text": "calls x", "detail": "", "assumptions": [], "basis": {"kind": "fact", "call": "x"}},
            {"line": 4, "text": "?", "detail": "long", "assumptions": ["a"], "basis": {"kind": "fact", "call": null}}
          ],
          "scenarios": [
            {"kind": "boundary", "title": "empty", "input": "\"\"", "steps": [{"line": 3, "what": "returns"}], "outcome": "ok", "assumptions": []}
          ]
        }"#;
        let draft = parse(content).unwrap();
        assert_eq!(
            draft.notes[0].basis,
            Basis::Fact(FactRef::Call { name: "x".into() })
        );
        assert_eq!(draft.notes[0].detail, None);
        assert_eq!(draft.notes[1].basis, Basis::Inference);
        assert_eq!(draft.notes[1].detail.as_deref(), Some("long"));
        assert_eq!(draft.scenarios[0].kind, ScenarioKind::Boundary);
    }

    #[test]
    fn html_entities_from_the_model_become_characters() {
        let content = r#"{"notes":[{"line":1,"text":"takes &amp;str","detail":"a &lt; b &amp;&amp; c","assumptions":["x &gt; 0"],"basis":{"kind":"inference","call":null}}],
          "scenarios":[{"kind":"normal","title":"&quot;a&quot;","input":"&#39;x&#39;","steps":[{"line":1,"what":"&amp;mut"}],"outcome":"ok","assumptions":[]}]}"#;
        let d = parse(content).unwrap();
        assert_eq!(d.notes[0].text, "takes &str");
        assert_eq!(d.notes[0].detail.as_deref(), Some("a < b && c"));
        assert_eq!(d.notes[0].assumptions[0], "x > 0");
        assert_eq!(d.scenarios[0].title, "\"a\"");
        assert_eq!(d.scenarios[0].input, "'x'");
        assert_eq!(d.scenarios[0].steps[0].what, "&mut");
    }

    #[test]
    fn rejects_an_answer_with_extra_or_missing_fields() {
        assert!(parse(r#"{"notes": []}"#).is_err());
        assert!(parse(r#"{"notes": [], "scenarios": [], "extra": 1}"#).is_err());
    }
}
