//! Earlier readings of functions that have since been edited. A reading is
//! keyed by its function's content, so the first keystroke in a function
//! loses it; until the function is read again, the editor shows the last
//! reading it had, marked as old. Functions are matched across edits by
//! where they sit in the code (owner, name, and which one of that name).

use std::collections::HashMap;

use crt_domain::{Function, Reading};

/// A function's place in its document, stable while its body is edited.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Place {
    enclosing: Option<String>,
    name: String,
    /// 0 for the first function with this owner and name, 1 for the next.
    nth: usize,
}

/// The place of each function, in the same order.
pub(crate) fn places(functions: &[Function]) -> Vec<Place> {
    let mut seen: HashMap<(Option<String>, String), usize> = HashMap::new();
    functions
        .iter()
        .map(|f| {
            let n = seen
                .entry((f.enclosing.clone(), f.name.clone()))
                .or_insert(0);
            let place = Place {
                enclosing: f.enclosing.clone(),
                name: f.name.clone(),
                nth: *n,
            };
            *n += 1;
            place
        })
        .collect()
}

/// The last reading seen for each place of one document, with lines
/// relative to the function's first line.
#[derive(Debug, Default)]
pub(crate) struct Memory(HashMap<Place, Reading>);

impl Memory {
    /// Remembers the current readings, and returns, for each function that
    /// has none, the reading its place had before (placed at its current
    /// lines, notes past its end dropped).
    pub(crate) fn update(
        &mut self,
        functions: &[Function],
        readings: &[Option<Reading>],
    ) -> Vec<Option<Reading>> {
        places(functions)
            .into_iter()
            .zip(functions.iter().zip(readings))
            .map(|(place, (f, reading))| match reading {
                Some(r) => {
                    self.0.insert(place, r.relative_to(f.span.start_line));
                    None
                }
                None => self.0.get(&place).map(|old| {
                    let mut placed = old.placed_at(f.span.start_line);
                    placed.notes.retain(|n| n.line <= f.span.end_line);
                    // Scenarios of an older body are not shown as old notes.
                    placed.scenarios = None;
                    placed
                }),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crt_domain::{Author, Basis, CheckReport, ContentHash, Note, Span, SymbolKind};

    fn function(name: &str, start: usize, end: usize, body: &str) -> Function {
        Function {
            name: name.into(),
            kind: SymbolKind::Function,
            enclosing: None,
            span: Span {
                start_byte: 0,
                end_byte: 1,
                start_line: start,
                end_line: end,
            },
            hash: ContentHash::of(body.as_bytes()),
            docs: None,
            lines: vec![],
        }
    }

    fn reading(f: &Function, lines: &[usize]) -> Reading {
        Reading {
            function_hash: f.hash,
            author: Author {
                model: "m".into(),
                prompt: "p".into(),
            },
            notes: lines
                .iter()
                .map(|&line| Note {
                    line,
                    text: format!("note {line}"),
                    detail: None,
                    assumptions: vec![],
                    basis: Basis::Inference,
                })
                .collect(),
            scenarios: Some(vec![]),
            check: CheckReport::default(),
        }
    }

    #[test]
    fn an_edited_function_gets_its_last_reading_at_its_new_place() {
        let mut memory = Memory::default();
        let f = function("f", 10, 14, "v1");
        let g = function("g", 20, 22, "g");
        memory.update(
            &[f.clone(), g.clone()],
            &[Some(reading(&f, &[11, 14])), Some(reading(&g, &[21]))],
        );

        // f edited (new body, one line shorter) and moved down by 2 lines.
        let f2 = function("f", 12, 15, "v2");
        let stale = memory.update(&[f2, g.clone()], &[None, Some(reading(&g, &[21]))]);
        let old = stale[0].as_ref().unwrap();
        assert_eq!(
            old.notes.iter().map(|n| n.line).collect::<Vec<_>>(),
            vec![13],
            "moved with the function; the note past its new end is dropped"
        );
        assert_eq!(old.scenarios, None);
        assert!(
            stale[1].is_none(),
            "a function with a reading needs no old one"
        );
    }

    #[test]
    fn functions_of_the_same_name_keep_their_own_readings() {
        let mut memory = Memory::default();
        let a = function("new", 1, 3, "a");
        let b = function("new", 5, 7, "b");
        memory.update(
            &[a.clone(), b.clone()],
            &[Some(reading(&a, &[2])), Some(reading(&b, &[6]))],
        );
        let stale = memory.update(
            &[function("new", 1, 3, "a2"), function("new", 5, 7, "b2")],
            &[None, None],
        );
        assert_eq!(stale[0].as_ref().unwrap().notes[0].line, 2);
        assert_eq!(stale[1].as_ref().unwrap().notes[0].line, 6);
    }

    #[test]
    fn a_new_function_has_no_old_reading() {
        let mut memory = Memory::default();
        let stale = memory.update(&[function("h", 1, 2, "h")], &[None]);
        assert!(stale[0].is_none());
    }
}
