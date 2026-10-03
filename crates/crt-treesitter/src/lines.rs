//! Byte offset to 1-based line number, for turning tag byte ranges into
//! the domain's line spans. Only `\n` ends a line, matching tree-sitter's
//! row counting; a `\r` stays on its line.

pub(crate) struct LineIndex {
    /// Byte offset at which each line starts; `starts[0] == 0`.
    starts: Vec<usize>,
}

impl LineIndex {
    pub(crate) fn new(source: &[u8]) -> Self {
        let mut starts = vec![0];
        starts.extend(
            source
                .iter()
                .enumerate()
                .filter(|(_, b)| **b == b'\n')
                .map(|(i, _)| i + 1),
        );
        Self { starts }
    }

    /// The 1-based line containing byte `offset`. An offset equal to the
    /// source length after a trailing newline names the empty last line.
    pub(crate) fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&s| s <= offset)
    }

    /// The 1-based line containing the last byte of the half-open range
    /// `start..end`; equals the start line when the range is empty.
    pub(crate) fn end_line_of(&self, start: usize, end: usize) -> usize {
        self.line_of(end.saturating_sub(1).max(start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_offsets_to_lines() {
        let idx = LineIndex::new(b"ab\ncd\n\nef");
        assert_eq!(idx.line_of(0), 1);
        assert_eq!(idx.line_of(2), 1);
        assert_eq!(idx.line_of(3), 2);
        assert_eq!(idx.line_of(6), 3);
        assert_eq!(idx.line_of(7), 4);
        assert_eq!(idx.end_line_of(0, 5), 2);
        assert_eq!(idx.end_line_of(0, 6), 2);
        assert_eq!(idx.end_line_of(3, 3), 2);
    }

    #[test]
    fn empty_source_crlf_trailing_newline_and_multibyte() {
        assert_eq!(LineIndex::new(b"").line_of(0), 1);

        let crlf = LineIndex::new(b"a\r\nb\r\n");
        assert_eq!(crlf.line_of(1), 1);
        assert_eq!(crlf.line_of(3), 2);
        assert_eq!(crlf.end_line_of(0, 3), 1);

        let trailing = LineIndex::new(b"a\n");
        assert_eq!(trailing.end_line_of(0, 2), 1);
        assert_eq!(trailing.line_of(2), 2);

        let utf8 = "日本\nx".as_bytes();
        let idx = LineIndex::new(utf8);
        assert_eq!(idx.line_of(5), 1);
        assert_eq!(idx.line_of(7), 2);
        assert_eq!(idx.end_line_of(0, utf8.len()), 2);
    }
}
