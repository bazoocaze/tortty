//! Grid: row and text helpers.

use unicode_width::UnicodeWidthChar;

use crate::cell::Cell;

/// How many of a cell's tail the renderer paints as combining marks: the kitty
/// placeholder needs three (row, column and the top byte of the image id) and
/// more than that is noise.
pub const MAX_MARKS: usize = 3;

/// How many codepoints a cell's tail keeps (`B66`): a family of four is seven
/// (`👨‍👩‍👧‍👦`), a flag with a tag is seven, a kiss is seven. Beyond that the
/// cluster is not a cluster anymore and the rest is dropped.
pub const MAX_TAIL: usize = 16;

/// Zero-width characters that are part of a sequence and not decoration, so a
/// repeat is meaningful: a joiner, a variation selector, a keycap and a tag.
#[must_use]
pub fn is_structural(ch: char) -> bool {
    matches!(
        ch,
        '\u{200C}' | '\u{200D}' | '\u{FE0E}' | '\u{FE0F}' | '\u{20E3}'
    ) || ('\u{E0020}'..='\u{E007F}').contains(&ch)
}

/// A plain combining mark: zero width and not part of a sequence. This is what
/// the renderer paints over the cell it belongs to.
#[must_use]
pub fn is_mark(ch: char) -> bool {
    ch.width() == Some(0) && !is_structural(ch)
}

/// Is this codepoint an emoji base or modifier? `B66` uses it to decide that a
/// cluster has emoji presentation (and is therefore two columns wide) — the
/// range, not a table, because a table of emoji is a file that ages.
#[must_use]
pub fn is_emoji(ch: char) -> bool {
    let c = u32::from(ch);
    (0x1F000..=0x1FAFF).contains(&c) || (0x2600..=0x27BF).contains(&c)
}

/// A regional indicator (`U+1F1E6..=U+1F1FF`): two of them spell a flag.
#[must_use]
pub fn is_regional(ch: char) -> bool {
    (0x1F1E6..=0x1F1FF).contains(&u32::from(ch))
}

/// A skin-tone modifier (`U+1F3FB..=U+1F3FF`): it tints the base it follows.
#[must_use]
pub fn is_modifier(ch: char) -> bool {
    (0x1F3FB..=0x1F3FF).contains(&u32::from(ch))
}

/// Does this cell's tail make the cluster emoji presentation? Then the cell is
/// two columns wide, whatever the base alone measured (`❤️` is 2, `❤︎` is 1).
#[must_use]
pub fn emoji_presentation(base: char, tail: &[char]) -> bool {
    if tail.iter().any(|c| *c == '\u{FE0F}' || *c == '\u{20E3}') {
        return true;
    }
    if tail.iter().any(|c| is_modifier(*c)) {
        return true;
    }
    if is_regional(base) && tail.iter().any(|c| is_regional(*c)) {
        return true;
    }
    let joined = tail.contains(&'\u{200D}');
    joined && (is_emoji(base) || tail.iter().any(|c| is_emoji(*c)))
}

/// Grid row. `wrapped` is the soft wrap (autowrap): the row continues on the one
/// below. The parser uses it for (future) reflow and to concatenate text.
#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
    /// What belongs to the cell before it, by column: combining marks (`M-U`),
    /// and since `B66` the rest of a grapheme cluster — the joiners, the joined
    /// members, a variation selector, a skin tone, the second half of a flag.
    /// Kept beside the cells (and not in `Cell`) because they are rare and a
    /// cell is 20 bytes times the whole scrollback; empty for almost every row.
    /// Sorted by column, at most [`MAX_TAIL`] per cell.
    pub tail: Vec<(u16, Vec<char>)>,
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
            tail: Vec::new(),
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
            tail: Vec::new(),
            chrome,
        }
    }

    /// Puts `ch` on the cell at `col` as a combining mark. A mark belongs to the
    /// character it follows, so it does not take a cell of its own and does not
    /// move the cursor (`M-U`).
    pub fn add_mark(&mut self, col: usize, ch: char) {
        self.add_tail(col, ch);
    }

    /// Appends `ch` to the tail of the cell at `col`: the codepoints of a grapheme
    /// cluster that live beside the base (`B66`). A repeated plain mark is noise
    /// and is dropped; a repeated joiner or selector is the sequence and is kept.
    pub fn add_tail(&mut self, col: usize, ch: char) {
        let Ok(col) = u16::try_from(col) else {
            return;
        };
        match self.tail.binary_search_by_key(&col, |(c, _)| *c) {
            Ok(i) => {
                let tail = &mut self.tail[i].1;
                let repeat = ch.width() == Some(0) && !is_structural(ch) && tail.contains(&ch);
                if tail.len() < MAX_TAIL && !repeat {
                    tail.push(ch);
                }
            },
            Err(i) => self.tail.insert(i, (col, vec![ch])),
        }
    }

    /// The tail of the cell at `col` — combining marks and the rest of the
    /// cluster — in the order it arrived.
    #[must_use]
    pub fn tail_at(&self, col: usize) -> &[char] {
        let Ok(col) = u16::try_from(col) else {
            return &[];
        };
        match self.tail.binary_search_by_key(&col, |(c, _)| *c) {
            Ok(i) => &self.tail[i].1,
            Err(_) => &[],
        }
    }

    /// The tail of the cell at `col` as it was in `M-U`: only the zero-width
    /// characters that are decoration, which is what the renderer paints and
    /// what the kitty placeholder reads.
    #[must_use]
    pub fn marks_at(&self, col: usize) -> &[char] {
        self.tail_at(col)
    }

    /// A cell being written replaces what was there, tail included.
    pub fn clear_marks(&mut self, col: usize) {
        let Ok(col) = u16::try_from(col) else {
            return;
        };
        if let Ok(i) = self.tail.binary_search_by_key(&col, |(c, _)| *c) {
            self.tail.remove(i);
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
        let mut s = String::new();
        for (col, cell) in self.cells.iter().enumerate() {
            if cell.is_cont() {
                continue;
            }
            s.push(cell.ch);
            s.extend(self.tail_at(col));
        }
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
        let mut s = String::new();
        for col in a..b {
            if self.cells[col].is_cont() {
                continue;
            }
            s.push(self.cells[col].ch);
            s.extend(self.tail_at(col));
        }
        s
    }

    /// Resize the row (grows with a blank cell, cuts the excess).
    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::BLANK);
    }
}
