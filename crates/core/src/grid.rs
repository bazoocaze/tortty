//! Grid: row and text helpers.

use crate::cell::Cell;

/// How many combining marks one cell keeps: the kitty placeholder needs three
/// (row, column and the top byte of the image id) and more than that is noise.
pub const MAX_MARKS: usize = 3;

/// Grid row. `wrapped` is the soft wrap (autowrap): the row continues on the one
/// below. The parser uses it for (future) reflow and to concatenate text.
#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
    /// Combining marks, by column (`M-U`): a mark has no cell of its own, it
    /// belongs to the character it follows. Kept beside the cells (and not in
    /// `Cell`) because they are rare and a cell is 12 bytes times the whole
    /// scrollback; empty for almost every row. Sorted by column, at most
    /// [`MAX_MARKS`] per cell.
    pub marks: Vec<(u16, Vec<char>)>,
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
            marks: Vec::new(),
            chrome: false,
        }
    }

    /// A row built from cells the terminal is writing: no marks (a reflow drops
    /// them — a mark is not addressed, it moves with the character it follows).
    #[must_use]
    pub fn filled(cells: Vec<Cell>, wrapped: bool, chrome: bool) -> Row {
        Row {
            cells,
            wrapped,
            marks: Vec::new(),
            chrome,
        }
    }

    /// Puts `ch` on the cell at `col` as a combining mark. A mark belongs to the
    /// character it follows, so it does not take a cell of its own and does not
    /// move the cursor (`M-U`).
    pub fn add_mark(&mut self, col: usize, ch: char) {
        let Ok(col) = u16::try_from(col) else {
            return;
        };
        match self.marks.binary_search_by_key(&col, |(c, _)| *c) {
            Ok(i) => {
                let marks = &mut self.marks[i].1;
                if marks.len() < MAX_MARKS && !marks.contains(&ch) {
                    marks.push(ch);
                }
            },
            Err(i) => self.marks.insert(i, (col, vec![ch])),
        }
    }

    /// The marks of the cell at `col`, in the order they arrived.
    #[must_use]
    pub fn marks_at(&self, col: usize) -> &[char] {
        let Ok(col) = u16::try_from(col) else {
            return &[];
        };
        match self.marks.binary_search_by_key(&col, |(c, _)| *c) {
            Ok(i) => &self.marks[i].1,
            Err(_) => &[],
        }
    }

    /// A cell being written replaces what was there, marks included.
    pub fn clear_marks(&mut self, col: usize) {
        let Ok(col) = u16::try_from(col) else {
            return;
        };
        if let Ok(i) = self.marks.binary_search_by_key(&col, |(c, _)| *c) {
            self.marks.remove(i);
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
