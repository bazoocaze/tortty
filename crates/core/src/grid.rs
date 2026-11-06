//! Grid: row and text helpers.

use crate::cell::Cell;

/// Grid row. `wrapped` is the soft wrap (autowrap): the row continues on the one
/// below. The parser uses it for (future) reflow and to concatenate text.
#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
}

impl Row {
    pub fn blank(cols: usize) -> Row {
        Row {
            cells: vec![Cell::BLANK; cols],
            wrapped: false,
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
