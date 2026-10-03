//! A reading of one function: per-line notes and behaviour scenarios, each
//! marked with what it rests on. The rule that matters most lives here: a
//! note may claim to rest on a structural fact only if that fact exists.

use crate::function::Function;
use crate::hash::ContentHash;

/// A structural fact a note can point at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactRef {
    /// "This line calls `name`."
    Call { name: String },
}

/// What a statement rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Basis {
    /// A fact read off the syntax tree. Shown as certain.
    Fact(FactRef),
    /// The model's reasoning about behaviour. Shown as a guess, with its
    /// assumptions next to it.
    Inference,
}

/// One explanation attached to one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub line: usize,
    /// One short sentence for the end of the line.
    pub text: String,
    /// The longer explanation for the hover.
    pub detail: Option<String>,
    /// What the explanation takes for granted ("the caller validated x").
    pub assumptions: Vec<String>,
    pub basis: Basis,
}

/// Which kind of input a scenario walks through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioKind {
    Normal,
    Boundary,
    Concurrent,
}

/// One step of a scenario: what happens on a line under that input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub line: usize,
    pub what: String,
}

/// "If this input arrives, the code goes like this." Always an inference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    pub kind: ScenarioKind,
    pub title: String,
    pub input: String,
    pub steps: Vec<Step>,
    pub outcome: String,
    pub assumptions: Vec<String>,
}

/// Notes as the explainer wrote them, not yet checked. Scenarios are
/// written later, on request, and checked with [`Reading::with_scenarios`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub notes: Vec<Note>,
}

/// Who wrote a reading. Part of the cache key: a different model or prompt
/// is a different reading.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Author {
    pub model: String,
    /// Identifies the prompt and its options (version, output language),
    /// e.g. `v1-ja`.
    pub prompt: String,
}

/// What the checker changed in a draft, so readers can be told.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    /// Notes that claimed a fact the function does not have; now guesses.
    pub demoted: usize,
    /// Notes or steps on lines outside the function; removed.
    pub dropped: usize,
}

/// A checked reading of one function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub function_hash: ContentHash,
    pub author: Author,
    /// In line order.
    pub notes: Vec<Note>,
    /// `None` until scenarios are asked for: they cost a second model call
    /// and most readers only need the notes.
    pub scenarios: Option<Vec<Scenario>>,
    pub check: CheckReport,
}

impl Reading {
    /// Checks `draft` against `function`'s facts:
    /// - a note outside the function's lines is dropped;
    /// - a note claiming a fact the function does not have on that line is
    ///   demoted to an inference (kept, never silently trusted).
    ///
    /// The reading has no scenarios yet.
    pub fn check(function: &Function, author: Author, draft: Draft) -> Reading {
        let (notes, check) = function.check_notes(draft.notes);
        Reading {
            function_hash: function.hash,
            author,
            notes,
            scenarios: None,
            check,
        }
    }

    /// This reading with `scenarios` checked against `function` and
    /// attached: steps outside the function's lines are dropped. Lines are
    /// file lines, as in [`Reading::check`].
    pub fn with_scenarios(&self, function: &Function, scenarios: Vec<Scenario>) -> Reading {
        let mut out = self.clone();
        let scenarios = scenarios
            .into_iter()
            .map(|mut s| {
                let before = s.steps.len();
                s.steps.retain(|step| function.contains_line(step.line));
                out.check.dropped += before - s.steps.len();
                s
            })
            .collect();
        out.scenarios = Some(scenarios);
        out
    }

    /// This reading with every line shifted by `delta`, which may be
    /// negative. Readings are kept relative to the function's first line
    /// (`relative_to(start)`, first line = 0) so that a function that moves
    /// in its file keeps its reading, and placed back with `placed_at(start)`.
    fn shifted(&self, delta: isize) -> Reading {
        let move_line = |l: usize| l.saturating_add_signed(delta);
        let mut out = self.clone();
        for n in &mut out.notes {
            n.line = move_line(n.line);
        }
        for s in out.scenarios.iter_mut().flatten() {
            for step in &mut s.steps {
                step.line = move_line(step.line);
            }
        }
        out
    }

    /// The reading with lines counted from the function's first line (0).
    /// This is the form a store keeps: it does not depend on where the
    /// function sits in its file.
    pub fn relative_to(&self, start_line: usize) -> Reading {
        self.shifted(-isize::try_from(start_line).unwrap_or(isize::MAX))
    }

    /// The reading with file line numbers, for a function starting at
    /// `start_line`. Inverse of [`Reading::relative_to`].
    pub fn placed_at(&self, start_line: usize) -> Reading {
        self.shifted(isize::try_from(start_line).unwrap_or(isize::MAX))
    }
}

impl Function {
    /// `notes` checked against this function's facts, in line order, as
    /// [`Reading::check`] does. Also used on notes still arriving.
    pub fn check_notes(&self, notes: Vec<Note>) -> (Vec<Note>, CheckReport) {
        let mut report = CheckReport::default();
        let mut kept: Vec<Note> = Vec::with_capacity(notes.len());
        for mut note in notes {
            if !self.contains_line(note.line) {
                report.dropped += 1;
                continue;
            }
            if let Basis::Fact(fact) = &note.basis
                && !self.has_fact(note.line, fact)
            {
                note.basis = Basis::Inference;
                report.demoted += 1;
            }
            kept.push(note);
        }
        kept.sort_by_key(|n| n.line);
        (kept, report)
    }

    /// True when `line` is one of this function's lines.
    pub fn contains_line(&self, line: usize) -> bool {
        self.span.start_line <= line && line <= self.span.end_line
    }

    /// True when the fact holds on `line` of this function.
    pub fn has_fact(&self, line: usize, fact: &FactRef) -> bool {
        match fact {
            FactRef::Call { name } => self
                .lines
                .iter()
                .any(|l| l.line == line && l.calls.iter().any(|c| &c.name == name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::{Call, LineFacts};
    use crate::symbol::{Span, SymbolKind};

    fn function() -> Function {
        Function {
            name: "f".into(),
            kind: SymbolKind::Function,
            enclosing: None,
            span: Span {
                start_byte: 0,
                end_byte: 10,
                start_line: 10,
                end_line: 14,
            },
            hash: ContentHash::of(b"f"),
            docs: None,
            lines: vec![LineFacts {
                line: 11,
                calls: vec![Call {
                    name: "validate".into(),
                    defined_in_file: true,
                }],
            }],
        }
    }

    fn note(line: usize, basis: Basis) -> Note {
        Note {
            line,
            text: "t".into(),
            detail: None,
            assumptions: vec![],
            basis,
        }
    }

    fn author() -> Author {
        Author {
            model: "m".into(),
            prompt: "v1".into(),
        }
    }

    #[test]
    fn a_true_fact_stays_a_fact_and_a_false_one_becomes_a_guess() {
        let draft = Draft {
            notes: vec![
                note(
                    12,
                    Basis::Fact(FactRef::Call {
                        name: "validate".into(),
                    }),
                ),
                note(
                    11,
                    Basis::Fact(FactRef::Call {
                        name: "validate".into(),
                    }),
                ),
                note(13, Basis::Inference),
            ],
        };
        let r = Reading::check(&function(), author(), draft);
        assert_eq!(
            r.notes.iter().map(|n| n.line).collect::<Vec<_>>(),
            vec![11, 12, 13]
        );
        assert_eq!(r.scenarios, None);
        assert!(matches!(r.notes[0].basis, Basis::Fact(_)));
        assert_eq!(r.notes[1].basis, Basis::Inference);
        assert_eq!(
            r.check,
            CheckReport {
                demoted: 1,
                dropped: 0
            }
        );
    }

    #[test]
    fn notes_and_steps_outside_the_function_are_dropped() {
        let draft = Draft {
            notes: vec![
                note(9, Basis::Inference),
                note(15, Basis::Inference),
                note(10, Basis::Inference),
            ],
        };
        let scenarios = vec![Scenario {
            kind: ScenarioKind::Boundary,
            title: "empty".into(),
            input: "x = \"\"".into(),
            steps: vec![
                Step {
                    line: 11,
                    what: "validate rejects".into(),
                },
                Step {
                    line: 40,
                    what: "elsewhere".into(),
                },
            ],
            outcome: "error".into(),
            assumptions: vec![],
        }];
        let r = Reading::check(&function(), author(), draft).with_scenarios(&function(), scenarios);
        assert_eq!(r.notes.len(), 1);
        assert_eq!(r.scenarios.as_ref().unwrap()[0].steps.len(), 1);
        assert_eq!(r.check.dropped, 3);
        assert_eq!(r.function_hash, function().hash);
    }

    #[test]
    fn a_reading_survives_its_function_moving_down_the_file() {
        let draft = Draft {
            notes: vec![note(11, Basis::Inference)],
        };
        let scenarios = vec![Scenario {
            kind: ScenarioKind::Normal,
            title: "t".into(),
            input: "i".into(),
            steps: vec![Step {
                line: 12,
                what: "w".into(),
            }],
            outcome: "o".into(),
            assumptions: vec![],
        }];
        let r = Reading::check(&function(), author(), draft).with_scenarios(&function(), scenarios);
        let stored = r.relative_to(10);
        assert_eq!(stored.notes[0].line, 1);
        assert_eq!(stored.scenarios.as_ref().unwrap()[0].steps[0].line, 2);
        let moved = stored.placed_at(25);
        assert_eq!(moved.notes[0].line, 26);
        assert_eq!(moved.scenarios.as_ref().unwrap()[0].steps[0].line, 27);
        assert_eq!(stored.placed_at(10), r);
    }
}
