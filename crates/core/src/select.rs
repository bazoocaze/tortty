//! Selection: absolute anchor and text.
//!
//! The anchor is `(absolute line, column)` and **not** a pair of screen
//! coordinates: that way the selection survives scrolling the history and output
//! arriving (the text moves, the selection moves with it). It is the same model
//! as Python `tortty` (`term.py:1050-1075`), with one difference: `scrolled` and
//! `abs_gen` are counters here, not objects.
//!
//! `abs_gen` is the generation of the absolute space: anything that rewrites the
//! history underneath (`CSI 3J`, RIS, reflow) increments it and kills the old
//! anchors — better to lose the selection than to copy the wrong text.

use crate::grid::Row;
use crate::term::Term;

/// Cell addressed in the absolute space (history + live grid).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Anchor {
    /// Absolute line: it grows with scrolling and does not move when the history
    /// scrolls.
    pub line: i64,
    pub col: usize,
    /// Generation of the absolute space the anchor was born in.
    pub gen: u64,
}

/// Normalized selection: `a <= b` in reading order.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Span {
    pub a: Anchor,
    pub b: Anchor,
}

impl Span {
    /// Sort the ends (dragging bottom-up is normal).
    #[must_use]
    pub fn new(p: Anchor, q: Anchor) -> Span {
        if (p.line, p.col) <= (q.line, q.col) {
            Span { a: p, b: q }
        } else {
            Span { a: q, b: p }
        }
    }

    /// Intersection with the screen: `(r1, c1, r2, c2)` in the view at `view_off`.
    ///
    /// `None` = the whole selection is off screen (or dead). Whatever fell outside
    /// is pinned to the edge: the drawing paints the visible piece and the copy
    /// uses the whole anchor.
    #[must_use]
    pub fn visible(&self, t: &Term, view_off: usize) -> Option<(usize, usize, usize, usize)> {
        if !t.anchor_alive(&self.a) || !t.anchor_alive(&self.b) {
            return None;
        }
        let rows = t.rows as i64;
        let r1 = t.anchor_row(&self.a, view_off);
        let r2 = t.anchor_row(&self.b, view_off);
        if r1 >= rows || r2 < 0 {
            return None;
        }
        let c1 = if r1 >= 0 { self.a.col } else { 0 };
        let c2 = if r2 < rows {
            self.b.col
        } else {
            t.cols.saturating_sub(1)
        };
        Some((r1.max(0) as usize, c1, (r2.min(rows - 1)) as usize, c2))
    }
}

impl Term {
    /// Anchor of cell `(row, col)` of the view with `off` lines of history.
    #[must_use]
    pub fn anchor_at(&self, off: usize, row: usize, col: usize) -> Anchor {
        Anchor {
            line: self.scrolled as i64 - off as i64 + row as i64,
            col,
            gen: self.abs_gen,
        }
    }

    /// First absolute line that still exists (the history has a ceiling).
    #[must_use]
    pub fn abs_first(&self) -> i64 {
        self.scrolled as i64 - self.scrollback.len() as i64
    }

    /// View row where the anchor falls (< 0 or >= rows = off screen).
    #[must_use]
    pub fn anchor_row(&self, a: &Anchor, off: usize) -> i64 {
        a.line - self.scrolled as i64 + off as i64
    }

    /// The anchor still points at content that exists.
    #[must_use]
    pub fn anchor_alive(&self, a: &Anchor) -> bool {
        a.gen == self.abs_gen && self.abs_row(a.line).is_some()
    }

    /// The absolute line, if it exists (history or live grid).
    #[must_use]
    pub fn abs_row(&self, line: i64) -> Option<&Row> {
        let lo = self.abs_first();
        if line < lo {
            return None;
        }
        if line < self.scrolled as i64 {
            self.scrollback.get((line - lo) as usize)
        } else {
            self.grid.get((line - self.scrolled as i64) as usize)
        }
    }

    /// Text between two anchors (ends inclusive), even off screen.
    ///
    /// Trailing space is dropped from each line; a dead anchor returns `None`
    /// instead of half the text.
    #[must_use]
    pub fn span_text(&self, span: &Span) -> Option<String> {
        if !self.anchor_alive(&span.a) || !self.anchor_alive(&span.b) {
            return None;
        }
        let cols = self.cols;
        let mut out = Vec::with_capacity((span.b.line - span.a.line + 1) as usize);
        for line in span.a.line..=span.b.line {
            let row = self.abs_row(line)?;
            let from = if line == span.a.line { span.a.col } else { 0 }.min(cols);
            let to = if line == span.b.line {
                span.b.col
            } else {
                cols.saturating_sub(1)
            }
            .min(cols.saturating_sub(1));
            out.push(row.text_cols(from, to + 1));
        }
        Some(out.join("\n"))
    }

    /// Text of a rectangle of the view `(r1,c1)..(r2,c2)`, ends inclusive.
    ///
    /// Used to copy the viewport when there is no selection.
    #[must_use]
    pub fn selection_text(
        &self,
        start: (usize, usize),
        end: (usize, usize),
        rows: &[&Row],
    ) -> String {
        let ((r1, c1), (r2, c2)) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let last = rows.len().saturating_sub(1);
        let (r1, r2) = (r1.min(last), r2.min(last));
        let cols = self.cols;
        let mut out = Vec::with_capacity(r2 - r1 + 1);
        for (y, row) in rows.iter().enumerate().take(r2 + 1).skip(r1) {
            let from = if y == r1 { c1 } else { 0 };
            let to = if y == r2 { (c2 + 1).min(cols) } else { cols };
            out.push(row.text_cols(from, to));
        }
        out.join("\n")
    }

    /// The whole viewport, without trailing blank lines (Ctrl+Shift+C with no
    /// selection).
    #[must_use]
    pub fn view_text(&self, off: usize) -> String {
        let mut lines: Vec<String> = self
            .visible_rows(off)
            .iter()
            .map(|r| r.text_cols(0, self.cols))
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        if lines.is_empty() {
            return String::new();
        }
        let mut s = lines.join("\n");
        s.push('\n');
        s
    }

    /// What to send to the child when the user pastes `text`.
    ///
    /// With `?2004` on, the text goes between `ESC[200~` and `ESC[201~`: the app
    /// (readline, vim, python) knows it is a paste and does not treat `\n` as
    /// Enter. The policy lives here so it is testable without a display.
    #[must_use]
    pub fn paste_payload(&self, text: &str) -> Vec<u8> {
        if self.bracketed_paste {
            let mut v = Vec::with_capacity(text.len() + 12);
            v.extend_from_slice(b"\x1b[200~");
            v.extend_from_slice(text.as_bytes());
            v.extend_from_slice(b"\x1b[201~");
            v
        } else {
            text.as_bytes().to_vec()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term() -> Term {
        Term::new(10, 4)
    }

    #[test]
    fn anchor_survives_scroll() {
        let mut t = term();
        // three lines: fills the screen, then scrolls one
        t.feed(b"AAA\r\nBBB\r\nCCC\r\nDDD");
        let a = t.anchor_at(0, 1, 0); // "BBB"
        t.feed(b"\r\nEEE");
        let b = t.anchor_at(0, 2, 1); // "CCC" col 1
        let txt = t.span_text(&Span::new(a, b)).unwrap();
        assert_eq!(txt, "BBB\nCCC\nDD", "the text has to move along with it");
    }

    #[test]
    fn anchor_dies_when_the_history_is_rewritten() {
        let mut t = term();
        t.feed(b"AAA\r\nBBB");
        let a = t.anchor_at(0, 0, 0);
        assert!(t.anchor_alive(&a));
        t.feed(b"\x1b[3J"); // ED3: clears the history
        assert!(!t.anchor_alive(&a));
        assert_eq!(t.span_text(&Span::new(a, a)), None);
    }

    #[test]
    fn span_normalizes_and_cuts_trailing_space() {
        let mut t = term();
        t.feed(b"hi   \r\nworld  ");
        let a = t.anchor_at(0, 1, 4); // last "d" of "world" (inclusive end)
        let b = t.anchor_at(0, 0, 0); // start of "hi"
        let txt = t.span_text(&Span::new(a, b)).unwrap();
        assert_eq!(txt, "hi\nworld");
    }

    #[test]
    fn visible_pins_the_selection_to_the_edge() {
        let mut t = Term::new(6, 2);
        t.feed(b"111111\r\n222222\r\n333333\r\n444444");
        // selects from line 2 to line 3, but the view shows 2 lines
        let a = t.anchor_at(0, 0, 0);
        let b = t.anchor_at(0, 1, 5);
        let (r1, c1, r2, c2) = Span::new(a, b).visible(&t, 0).unwrap();
        assert_eq!((r1, c1, r2, c2), (0, 0, 1, 5));
        // scrolling two lines back takes the selection off screen
        assert_eq!(Span::new(a, b).visible(&t, 3), None);
    }

    #[test]
    fn view_text_cuts_trailing_blank_lines() {
        let mut t = term();
        t.feed(b"one\r\ntwo");
        assert_eq!(t.view_text(0), "one\ntwo\n");
        let empty = super::tests::term(); // clean screen -> nothing
        assert_eq!(empty.view_text(0), "");
    }

    #[test]
    fn paste_respects_bracketed_paste() {
        let mut t = term();
        assert_eq!(t.paste_payload("a\nb"), b"a\nb");
        t.feed(b"\x1b[?2004h");
        assert_eq!(t.paste_payload("a\nb"), b"\x1b[200~a\nb\x1b[201~");
    }
}
