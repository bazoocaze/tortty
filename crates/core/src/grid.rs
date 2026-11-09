//! Grid: row and text helpers.

use crate::cell::Cell;

/// Grid row. `wrapped` is the soft wrap (autowrap): the row continues on the one
/// below. The parser uses it for (future) reflow and to concatenate text.
#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
    /// Decoration, not content: a row the terminal itself wrote to say something
    /// about the session (the line that separates a restored picture from the run
    /// that follows it, `B48`). It is drawn like any other and **never travels in
    /// the snapshot** — otherwise every restart would save the marker of the
    /// previous one and a tab reopened five times would hold five markers and no
    /// content. It survives the scrollback and the wrap, which a position would
    /// not; a reflow carries it by paragraph.
    pub chrome: bool,
}

impl Row {
    pub fn blank(cols: usize) -> Row {
        Row {
            cells: vec![Cell::BLANK; cols],
            wrapped: false,
            chrome: false,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Row text without the trailing spaces (for the socket `text`).
    pub fn text(&self) -> String {
        let mut s: String = self
            .cells
            .iter()
            .filter(|c| !c.is_cont())
            .map(|c| c.ch)
            .collect();
        while s.ends_with(' ') {
            s.pop();
        }
        s
    }

    /// Text of columns `a..b` (end exclusive), without cont and without trailing
    /// space. It is the base of the copy: cut by column, not by character.
    #[must_use]
    pub fn text_cols(&self, a: usize, b: usize) -> String {
        let mut s = self.text_cols_full(a, b);
        while s.ends_with(' ') {
            s.pop();
        }
        s
    }

    /// The same, keeping the trailing spaces: a `wrapped` row is full (the line
    /// goes on in the row below), so there they are content and not padding.
    #[must_use]
    pub fn text_cols_full(&self, a: usize, b: usize) -> String {
        let b = b.min(self.cells.len());
        let a = a.min(b);
        self.cells[a..b]
            .iter()
            .filter(|c| !c.is_cont())
            .map(|c| c.ch)
            .collect()
    }

    /// Resize the row (grows with a blank cell, cuts the excess).
    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::BLANK);
    }
}
