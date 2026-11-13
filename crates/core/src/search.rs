//! Scrollback search (`M-D`): find a text in what is above the screen and keep
//! the matches in order, so `Enter` can walk them.
//!
//! The scan is on demand — one pass over the scrollback and the grid when the
//! query changes — and it lives here because the lines do: the app does not know
//! what an absolute line is.

use crate::grid::Row;
use crate::term::Term;

/// One occurrence: an **absolute** line (the same space the prompt marks use),
/// the column and the length in cells.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Hit {
    pub line: i64,
    pub col: usize,
    pub len: usize,
}

/// Ceiling of matches kept: a one-letter query in a 100 000-line history would
/// otherwise hold every cell of it.
pub const HITS_MAX: usize = 5000;

/// The state of one search: what was typed and where it landed.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Search {
    pub query: String,
    pub hits: Vec<Hit>,
    /// Which hit is current (the one the view was sent to).
    pub at: usize,
}

impl Search {
    /// A search with a query already in it.
    #[must_use]
    pub fn new(query: &str) -> Search {
        Search {
            query: query.to_string(),
            hits: Vec::new(),
            at: 0,
        }
    }

    /// Recomputes the matches (`M-D`): after a keystroke and after the history
    /// grew, so the count on the bar does not lie.
    pub fn run(&mut self, term: &Term) {
        let at_line = self.hits.get(self.at).map(|h| h.line);
        self.hits = find(term, &self.query);
        // keep the current match under the cursor when it still exists: jumping
        // to the first one on every keystroke would be a different search
        self.at = at_line
            .and_then(|l| self.hits.iter().position(|h| h.line == l))
            .unwrap_or(0);
        self.at = self.at.min(self.hits.len().saturating_sub(1));
    }

    /// One character typed.
    pub fn push(&mut self, c: char, term: &Term) {
        self.query.push(c);
        self.run(term);
    }

    /// Backspace: `false` when there was nothing to erase (the caller closes the
    /// bar on an empty backspace, as a search field does).
    pub fn pop(&mut self, term: &Term) -> bool {
        if self.query.pop().is_none() {
            return false;
        }
        self.run(term);
        true
    }

    #[must_use]
    pub fn current(&self) -> Option<Hit> {
        self.hits.get(self.at).copied()
    }

    /// Next/previous match, wrapping: the view follows, so a search that reached
    /// the end does not stop talking to the user.
    pub fn step(&mut self, back: bool) -> Option<Hit> {
        if self.hits.is_empty() {
            return None;
        }
        self.at = if back {
            (self.at + self.hits.len() - 1) % self.hits.len()
        } else {
            (self.at + 1) % self.hits.len()
        };
        self.current()
    }

    /// `3/17` for the bar: where it is and how many there are.
    #[must_use]
    pub fn count(&self) -> String {
        if self.query.is_empty() {
            return String::new();
        }
        if self.hits.is_empty() {
            return "no match".to_string();
        }
        format!("{}/{}", self.at + 1, self.hits.len())
    }
}

/// Every occurrence of `query` in the history and the grid, top to bottom.
///
/// Case folding is ASCII only: a terminal's history is mostly ASCII, and a full
/// Unicode fold would change the number of cells a match covers.
#[must_use]
pub fn find(term: &Term, query: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let want: Vec<char> = query.chars().map(lower).collect();
    let sb = term.scrollback.len();
    let scrolled = term.scrolled as i64;
    let base = scrolled - sb as i64;
    // the history first (it is above the screen), then the grid
    let lines = term
        .scrollback
        .iter()
        .enumerate()
        .map(|(i, r)| (base + i as i64, r))
        .chain(
            term.grid
                .iter()
                .enumerate()
                .map(|(i, r)| (scrolled + i as i64, r)),
        );
    for (line, row) in lines {
        scan_row(row, &want, line, &mut out);
        if out.len() >= HITS_MAX {
            out.truncate(HITS_MAX);
            break;
        }
    }
    out
}

/// The cells of one row as characters **with their column**: a wide character
/// occupies two cells and the spacer between them is skipped, so a match after it
/// still knows where it is.
fn cells(row: &Row) -> Vec<(char, usize)> {
    let mut out = Vec::with_capacity(row.cells.len());
    for (i, c) in row.cells.iter().enumerate() {
        if c.is_cont() {
            continue;
        }
        out.push((c.ch, i));
    }
    out
}

fn scan_row(row: &Row, want: &[char], line: i64, out: &mut Vec<Hit>) {
    // trailing blanks are padding, not text: a query that ends in a space must
    // not match the empty rest of the line, and no match may start in padding
    let mut text = cells(row);
    while let Some(&(_, col)) = text.last() {
        if !row.cells[col].is_blank() {
            break;
        }
        text.pop();
    }
    let n = text.len();
    let m = want.len();
    if m == 0 || n < m {
        return;
    }
    let mut i = 0;
    while i + m <= n {
        let mut k = 0;
        while k < m && lower(text[i + k].0) == want[k] {
            k += 1;
        }
        if k == m {
            out.push(Hit {
                line,
                col: text[i].1,
                len: text[i + m - 1].1 + 1 - text[i].1,
            });
            // matches may touch (`aa` in `aaaa`): the next one starts one cell on
            i += 1;
        } else {
            i += 1;
        }
        if out.len() >= HITS_MAX {
            return;
        }
    }
}

/// ASCII case folding, and nothing else (`M-D`'s documented limit).
fn lower(c: char) -> char {
    c.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_finds_every_occurrence_in_the_grid_and_in_the_history() {
        let mut t = Term::new(20, 2);
        t.feed(b"alpha one\r\nbeta alpha\r\ngamma\r\nalpha two\r\n");
        // two lines of history + two of grid
        let hits = find(&t, "alpha");
        assert_eq!(hits.len(), 3);
        assert!(hits[0].line < hits[1].line && hits[1].line < hits[2].line);
        // the last one is the line on the screen, and its absolute line is the
        // one the prompt marks would give it (`scrolled` + row of the grid)
        let row = t
            .grid
            .iter()
            .position(|r| {
                r.cells
                    .iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .starts_with("alpha")
            })
            .expect("the last line is on the screen");
        assert_eq!(hits[2].line, t.scrolled as i64 + row as i64);
        assert_eq!(hits[0].col, 0);
        assert_eq!(hits[0].len, 5);
    }

    /// The query is matched byte for byte, spaces included: `error: ` has to
    /// find `error: ` and nothing else, and a bare space has to find the gaps.
    #[test]
    fn a_query_keeps_its_spaces() {
        let mut t = Term::new(20, 4);
        t.feed(b"error: one\r\n\r\nwarning: two\r\nfoo bar\r\n");
        assert_eq!(find(&t, "error: ").len(), 1);
        assert_eq!(find(&t, "foo bar").len(), 1);
        // a space is a character like any other, so it matches the blank line too
        assert_eq!(find(&t, " ").len(), 3);
        // and the length of the match is the length of the query
        assert_eq!(find(&t, "foo bar")[0].len, 7);
    }

    #[test]
    fn folding_is_ascii_only_and_the_column_is_the_match_start() {
        let mut t = Term::new(20, 3);
        t.feed(b"xx ALPHA\r\n");
        let hits = find(&t, "alpha");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].col, 3);
        // a query that is not in the text finds nothing
        assert_eq!(find(&t, "beta"), []);
        assert!(find(&t, "").is_empty(), "an empty query is not a match");
    }

    #[test]
    fn touching_matches_are_all_found() {
        let mut t = Term::new(20, 2);
        t.feed(b"aaaa\r\n");
        let hits = find(&t, "aa");
        assert_eq!(hits.len(), 3, "aaaa holds aa at 0, 1 and 2");
        assert_eq!(hits.iter().map(|h| h.col).collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn typing_runs_the_search_and_step_walks_it_with_a_wrap() {
        let mut t = Term::new(20, 2);
        t.feed(b"one two one two one\r\n");
        let mut s = Search::default();
        s.push('o', &t);
        s.push('n', &t);
        s.push('e', &t);
        assert_eq!(s.hits.len(), 3);
        assert_eq!(s.count(), "1/3");
        assert_eq!(s.current().unwrap().col, 0);
        assert_eq!(s.step(false).unwrap().col, 8);
        assert_eq!(s.step(false).unwrap().col, 16);
        assert_eq!(s.step(false).unwrap().col, 0, "wrapped");
        assert_eq!(s.count(), "1/3");
        assert_eq!(s.step(true).unwrap().col, 16, "back wraps the other way");
        // backspace that empties the query reports it
        assert!(s.pop(&t));
        assert!(s.pop(&t));
        assert!(s.pop(&t));
        assert!(!s.pop(&t));
        assert_eq!(s.count(), "", "an empty query has no count");
        assert_eq!(Search::new("").count(), "", "no query, no count");
        let mut none = Search::new("zzz");
        none.run(&t);
        assert_eq!(none.count(), "no match");
        assert!(none.step(false).is_none());
    }

    #[test]
    fn a_new_query_keeps_the_match_that_was_current_when_it_survives() {
        let mut t = Term::new(20, 2);
        t.feed(b"aaa bbb aaa\r\n");
        let mut s = Search::new("aaa");
        s.run(&t);
        s.step(false); // the second match
        assert_eq!(s.current().unwrap().col, 8);
        s.push('a', &t); // "aaaa" matches nothing anymore
        assert_eq!(s.hits, []);
        assert!(s.pop(&t)); // back to "aaa"
        assert_eq!(s.current().unwrap().col, 0, "the first one, not a crash");
    }
}
