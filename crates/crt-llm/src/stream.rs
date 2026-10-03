//! Reading an answer while it arrives: the server-sent events of a
//! streamed chat completion, and the note objects that are already complete
//! inside a notes answer that is not.

use std::io::BufRead;

use serde_json::Value;

/// What one streamed answer amounted to.
pub(crate) struct Streamed {
    pub content: String,
    pub finish_reason: Option<String>,
    pub refusal: String,
}

/// Reads server-sent events and hands each piece of content to
/// `on_content` as it arrives. An event is its `data:` lines joined by
/// newlines, ended by a blank line; `data: [DONE]` or the end of the body
/// ends the stream. Events that are not JSON (keep-alive pings) are
/// skipped: if one carried content, the final parse of the whole answer
/// fails and the request is asked again.
pub(crate) fn read_events(
    body: impl BufRead,
    on_content: &mut dyn FnMut(&str),
) -> Result<Streamed, String> {
    let mut out = Streamed {
        content: String::new(),
        finish_reason: None,
        refusal: String::new(),
    };
    let mut data: Option<String> = None;
    for line in body.lines() {
        let line = line.map_err(|e| format!("the stream broke off: {e}"))?;
        if let Some(d) = line.strip_prefix("data:") {
            let d = d.strip_prefix(' ').unwrap_or(d);
            match &mut data {
                Some(buf) => {
                    buf.push('\n');
                    buf.push_str(d);
                }
                None => data = Some(d.to_string()),
            }
        } else if line.is_empty()
            && let Some(event) = data.take()
            && take_event(&event, &mut out, on_content)?
        {
            return Ok(out);
        }
        // Other lines (`event:`, `id:`, `: comment`) carry nothing we use.
    }
    if let Some(event) = data.take() {
        take_event(&event, &mut out, on_content)?;
    }
    Ok(out)
}

/// Applies one event. Returns true when it ends the stream.
fn take_event(
    event: &str,
    out: &mut Streamed,
    on_content: &mut dyn FnMut(&str),
) -> Result<bool, String> {
    if event.trim() == "[DONE]" {
        return Ok(true);
    }
    let Ok(chunk) = serde_json::from_str::<Value>(event) else {
        return Ok(false);
    };
    if let Some(err) = chunk.get("error") {
        return Err(format!("the server reported an error mid-stream: {err}"));
    }
    let choice = &chunk["choices"][0];
    if let Some(piece) = choice["delta"]["content"].as_str() {
        out.content.push_str(piece);
        on_content(piece);
    }
    if let Some(piece) = choice["delta"]["refusal"].as_str() {
        out.refusal.push_str(piece);
    }
    if let Some(reason) = choice["finish_reason"].as_str() {
        out.finish_reason = Some(reason.to_string());
    }
    Ok(false)
}

/// Finds the objects of the top-level array in `{"notes": [ {..}, {..} ]}`
/// as soon as each one is complete, without waiting for the rest.
///
/// It tracks nesting depth outside of strings (minding escapes), so braces
/// inside text do not count. It only cuts the text into objects; whether an
/// object is a valid note is for the parser to say, and the whole answer is
/// parsed again at the end.
#[derive(Default)]
pub(crate) struct ObjectScanner {
    text: String,
    /// Bytes of `text` already scanned.
    pos: usize,
    depth: usize,
    in_string: bool,
    escaped: bool,
    /// Where the object being read started, when one is open at depth 2.
    start: Option<usize>,
}

/// Depth of the array's items: inside the root object (1) and the array (2).
const ITEM_DEPTH: usize = 2;

impl ObjectScanner {
    /// Adds `piece` and returns the objects it completed, in order.
    pub(crate) fn push(&mut self, piece: &str) -> Vec<String> {
        self.text.push_str(piece);
        let mut done = Vec::new();
        let bytes = self.text.as_bytes();
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if self.in_string {
                if self.escaped {
                    self.escaped = false;
                } else if b == b'\\' {
                    self.escaped = true;
                } else if b == b'"' {
                    self.in_string = false;
                }
            } else {
                match b {
                    b'"' => self.in_string = true,
                    b'{' | b'[' => {
                        if b == b'{' && self.depth == ITEM_DEPTH {
                            self.start = Some(self.pos);
                        }
                        self.depth += 1;
                    }
                    b'}' | b']' => {
                        self.depth = self.depth.saturating_sub(1);
                        if b == b'}'
                            && self.depth == ITEM_DEPTH
                            && let Some(start) = self.start.take()
                        {
                            done.push(self.text[start..=self.pos].to_string());
                        }
                    }
                    _ => {}
                }
            }
            self.pos += 1;
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects_come_out_as_soon_as_they_close_whatever_the_chunking() {
        let answer = r#"{"notes":[{"line":1,"text":"a } { \" ] [ \\","basis":{"kind":"inference"}},{"line":2,"text":"日本語"}]}"#;
        for size in [1, 2, 3, 7, 1000] {
            let mut s = ObjectScanner::default();
            let mut got = Vec::new();
            let chars: Vec<char> = answer.chars().collect();
            for chunk in chars.chunks(size) {
                got.extend(s.push(&chunk.iter().collect::<String>()));
            }
            assert_eq!(got.len(), 2, "chunk size {size}");
            assert!(got[0].starts_with(r#"{"line":1"#) && got[0].ends_with("}}"));
            assert_eq!(got[1], r#"{"line":2,"text":"日本語"}"#);
        }
    }

    #[test]
    fn an_unfinished_object_is_not_returned() {
        let mut s = ObjectScanner::default();
        assert!(s.push(r#"{"notes":[{"line":1,"text":"hal"#).is_empty());
        assert_eq!(s.push(r#"f"}"#).len(), 1);
    }

    #[test]
    fn events_are_joined_and_done_ends_the_stream() {
        let body = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n\
: keep-alive\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"no\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"tes\\\":[]}\"},\"finish_reason\":\"stop\"}]}\n\n\
data: [DONE]\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\"ignored\"}}]}\n";
        let mut pieces = Vec::new();
        let s = read_events(body.as_bytes(), &mut |p| pieces.push(p.to_string())).unwrap();
        assert_eq!(s.content, r#"{"notes":[]}"#);
        assert_eq!(pieces.len(), 2);
        assert_eq!(s.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn pings_crlf_and_multi_line_data_are_handled() {
        let body = "data: ping\r\n\r\n\
data:{\"choices\":[{\"delta\":\r\n\
data: {\"content\":\"ab\"}}]}\r\n\r\n\
data: {\"choices\":[{\"delta\":{\"content\":\"c\"}}]}";
        let s = read_events(body.as_bytes(), &mut |_| {}).unwrap();
        assert_eq!(s.content, "abc", "the last event needs no blank line");
    }

    #[test]
    fn an_error_event_is_an_error() {
        let body = "data: {\"error\":{\"message\":\"boom\"}}\n\n";
        assert!(read_events(body.as_bytes(), &mut |_| {}).is_err());
    }
}
