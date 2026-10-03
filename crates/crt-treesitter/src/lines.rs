//! Byte offset to 1-based line number, for turning tag byte ranges into
//! the domain's line spans.

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

    /// The 1-based line containing byte `offset`.
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
}
