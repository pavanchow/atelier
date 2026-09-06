//! Byte offset source spans and line/column conversion.

/// A half open byte range `[start, end)` into a source file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }

    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, offset: u32) -> bool {
        offset >= self.start && offset < self.end
    }

    /// True when `offset` is inside the span or exactly at its edges. Queries use
    /// this so a click at the end of an identifier still selects it.
    pub fn touches(&self, offset: u32) -> bool {
        offset >= self.start && offset <= self.end
    }

    /// Shift both ends by a signed delta. Used when reusing unchanged AST after
    /// an edit inserts or removes text earlier in the file.
    pub fn shifted(self, delta: i64) -> Span {
        Span {
            start: (self.start as i64 + delta) as u32,
            end: (self.end as i64 + delta) as u32,
        }
    }
}

impl std::fmt::Debug for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// One based line and column, the shape editors present to a user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

/// Convert a byte offset into a one based line and column.
pub fn offset_to_linecol(text: &str, offset: u32) -> LineCol {
    let offset = offset.min(text.len() as u32) as usize;
    let mut line = 1u32;
    let mut col = 1u32;
    for &b in &text.as_bytes()[..offset] {
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    LineCol { line, col }
}

/// Convert a one based line and column into a byte offset, clamped to the text.
pub fn linecol_to_offset(text: &str, line: u32, col: u32) -> u32 {
    let mut cur_line = 1u32;
    let mut idx = 0usize;
    let bytes = text.as_bytes();
    while cur_line < line && idx < bytes.len() {
        if bytes[idx] == b'\n' {
            cur_line += 1;
        }
        idx += 1;
    }
    let mut cur_col = 1u32;
    while cur_col < col && idx < bytes.len() && bytes[idx] != b'\n' {
        idx += 1;
        cur_col += 1;
    }
    idx as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linecol_roundtrip() {
        let text = "let a = 1;\nlet b = 2;\n";
        let off = linecol_to_offset(text, 2, 5);
        assert_eq!(&text[(off as usize)..=(off as usize)], "b");
        let lc = offset_to_linecol(text, off);
        assert_eq!(lc, LineCol { line: 2, col: 5 });
    }

    #[test]
    fn span_touch_and_shift() {
        let s = Span::new(3, 7);
        assert!(s.contains(3));
        assert!(!s.contains(7));
        assert!(s.touches(7));
        assert_eq!(s.shifted(2), Span::new(5, 9));
    }
}
