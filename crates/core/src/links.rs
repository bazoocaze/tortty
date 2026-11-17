//! Plain-text links (`B69`): a URL the child printed instead of declaring.
//!
//! `OSC 8` is the only way a cell **states** a link, and most programs do not use
//! it — what `git log`, `cargo` and `grep` print is text. This module reads that
//! text: over the rows in sight, at the moment somebody asks (Ctrl held, a click,
//! the menu), which is why nothing here is stored per cell and `feed` does not
//! pay for the feature.
//!
//! The scanner is deliberately conservative. It offers exactly the schemes
//! [`opens`] accepts, so an underline never promises a click that would be
//! refused, and an unknown scheme (`javascript:`, `data:`) is not a link at all.

use crate::grid::Row;

/// The schemes a terminal may hand to the desktop, in the form they are compared.
///
/// This list is the single source: the scanner links only these, and the app
/// opens only these. The rest of the URI space (`javascript:`, `data:`, SSH
/// handlers an application registered) is not a terminal's to launch — the URI
/// comes from the child.
pub const SCHEMES: [&str; 5] = ["https://", "http://", "file://", "ftp://", "mailto:"];

/// Ceiling of links per scan. A grid of 50×200 cells can hold a few hundred at
/// most; the cap exists so a pathological line cannot make the scan unbounded.
pub const MAX_LINKS: usize = 512;

/// `true` when [`crate::links::SCHEMES`] covers `uri`. Schemes are
/// case-insensitive (`RFC 3986`), so `HTTPS://…` from a child opens as well.
#[must_use]
pub fn opens(uri: &str) -> bool {
    SCHEMES.iter().any(|s| {
        uri.get(..s.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(s))
    })
}

/// The cells one link covers **on one row**.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Run {
    pub row: usize,
    pub col: usize,
    /// In columns, not characters: the second half of a wide character belongs
    /// to the same link and the renderer tests cells, not characters.
    pub len: usize,
}

/// A URL found in the text, and every cell it is written on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Link {
    pub uri: String,
    /// In reading order, possibly over more than one row: a wrapped URL is one
    /// link, and its second half is a run of its own.
    pub runs: Vec<Run>,
}

impl Link {
    /// `true` when the cell `(row, col)` is part of this link — the test a click
    /// makes, and the one the renderer makes for the underline.
    #[must_use]
    pub fn covers(&self, row: usize, col: usize) -> bool {
        self.runs
            .iter()
            .any(|r| r.row == row && r.col <= col && col < r.col.saturating_add(r.len))
    }
}

/// One character of a logical line, and where it is on the grid.
struct At {
    ch: char,
    row: usize,
    col: usize,
    /// 2 for the first half of a wide character (its continuation cells are part
    /// of it), 1 otherwise.
    cols: usize,
}

/// Every plain-text link in `rows`, in reading order.
///
/// `rows` are consecutive rows on screen: index 0 is the top. A row whose
/// `wrapped` is set continues on the next one, and only then are the two halves
/// the same string.
#[must_use]
pub fn scan(rows: &[&Row]) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    let mut y = 0;
    while y < rows.len() && out.len() < MAX_LINKS {
        let (line, next) = logical_line(rows, y);
        scan_line(&line, &mut out);
        y = next;
    }
    out
}

/// The characters of one logical line (soft-wrapped rows glued together) and the
/// index of the first row that is **not** part of it.
fn logical_line(rows: &[&Row], start: usize) -> (Vec<At>, usize) {
    let mut line: Vec<At> = Vec::new();
    let mut y = start;
    loop {
        let row = rows[y];
        // the tail of the row is padding, not text: a URL does not end in the
        // blanks the grid wrote to fill it
        let mut last = 0;
        for (x, c) in row.cells.iter().enumerate() {
            if !c.is_blank() {
                last = x + 1;
            }
        }
        let mut x = 0;
        while x < last {
            let cell = row.cells[x];
            // the second half of a wide character carries no character of its own
            if cell.is_cont() {
                x += 1;
                continue;
            }
            let mut cols = 1;
            while row
                .cells
                .get(x + cols)
                .is_some_and(crate::cell::Cell::is_cont)
            {
                cols += 1;
            }
            line.push(At {
                ch: if cell.is_blank() { ' ' } else { cell.ch },
                row: y,
                col: x,
                cols,
            });
            x += cols;
        }
        y += 1;
        if !row.wrapped || y >= rows.len() {
            return (line, y);
        }
    }
}

/// Walks one logical line looking for the schemes, back to back.
fn scan_line(line: &[At], out: &mut Vec<Link>) {
    let mut i = 0;
    while i < line.len() && out.len() < MAX_LINKS {
        let Some(n) = scheme_at(line, i) else {
            i += 1;
            continue;
        };
        let mut end = i + n;
        while end < line.len() && url_char(line[end].ch) {
            end += 1;
        }
        let end = trim(line, i, end);
        if valid(line, i, end) {
            out.push(build(line, i, end));
        }
        // the candidate is consumed whole: a URL never holds another one inside
        i = end.max(i + n);
    }
}

/// Length of the scheme starting at `i`, or `None`. The character before it must
/// not be part of a word — `xhttp://` is not a link, `(http://` is.
fn scheme_at(line: &[At], i: usize) -> Option<usize> {
    if i > 0 && line[i - 1].ch.is_alphanumeric() {
        return None;
    }
    let at = &line[i..];
    SCHEMES
        .iter()
        .find(|s| {
            at.len() >= s.len()
                && at
                    .iter()
                    .zip(s.chars())
                    .all(|(a, c)| a.ch.eq_ignore_ascii_case(&c))
        })
        .map(|s| s.len())
}

/// What a URL may contain. Whitespace and the characters that quote or frame a
/// URL in prose end it; brackets and parentheses do not, they are decided by
/// [`trim`].
fn url_char(c: char) -> bool {
    !c.is_whitespace()
        && !c.is_control()
        && !matches!(c, '<' | '>' | '"' | '`' | '|' | '\\' | '^' | '{' | '}')
}

/// Takes off the punctuation that belongs to the sentence, not to the URL: a
/// trailing `.`, `,`, `;`, `:`, `!`, `?` or quote, and a closing bracket only
/// when it has no opening one inside the URL — `…/Foo_(bar)` keeps its `)`, and
/// `(see https://x.com)` does not.
fn trim(line: &[At], start: usize, mut end: usize) -> usize {
    while end > start {
        let c = line[end - 1].ch;
        let cut = match c {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
            ')' | ']' | '}' => unbalanced(line, start, end, c),
            _ => false,
        };
        if !cut {
            break;
        }
        end -= 1;
    }
    end
}

/// `true` when `line[start..end]` closes `close` more often than it opens it.
fn unbalanced(line: &[At], start: usize, end: usize, close: char) -> bool {
    let open = match close {
        ')' => '(',
        ']' => '[',
        _ => '{',
    };
    let mut depth = 0i32;
    for at in &line[start..end] {
        if at.ch == open {
            depth += 1;
        } else if at.ch == close {
            depth -= 1;
        }
    }
    depth < 0
}

/// Does the candidate carry an address? A scheme alone (`http://`, `ftp://`,
/// `mailto:`) is not a link, and neither is a host of a single unknown label:
/// an underline that opens nothing is worse than no underline.
fn valid(line: &[At], start: usize, end: usize) -> bool {
    let text: String = line[start..end].iter().map(|a| a.ch).collect();
    let scheme = SCHEMES.iter().find(|s| {
        text.get(..s.len())
            .is_some_and(|p| p.eq_ignore_ascii_case(s))
    });
    let Some(scheme) = scheme else {
        return false;
    };
    let rest = &text[scheme.len()..];
    if scheme.starts_with("mailto:") {
        let Some((local, domain)) = rest.split_once('@') else {
            return false;
        };
        let domain = domain.split(['?', '#']).next().unwrap_or("");
        return !local.is_empty() && !domain.is_empty();
    }
    // authority: up to the path, without the `user:pass@` and the `:port`
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let auth = auth.rsplit_once('@').map_or(auth, |(_, host)| host);
    // IPv6 literal: the brackets are the authority, and the colons inside are
    // not a port separator
    if auth.starts_with('[') {
        return auth.contains(']');
    }
    let host = auth.split(':').next().unwrap_or(auth);
    if host.is_empty() || host.starts_with('.') {
        return false;
    }
    // a dotted name or the one host every machine resolves without a domain;
    // `http://build` is a name only inside somebody's network and is left alone
    host.contains('.') || host.eq_ignore_ascii_case("localhost")
}

/// Turns the candidate into a link, merging consecutive characters into runs.
fn build(line: &[At], start: usize, end: usize) -> Link {
    let uri: String = line[start..end].iter().map(|a| a.ch).collect();
    let mut runs: Vec<Run> = Vec::new();
    for at in &line[start..end] {
        match runs.last_mut() {
            Some(last) if last.row == at.row && last.col + last.len == at.col => {
                last.len += at.cols;
            },
            _ => runs.push(Run {
                row: at.row,
                col: at.col,
                len: at.cols,
            }),
        }
    }
    Link { uri, runs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::Cell;

    /// A row of text: `wrapped` says whether it continues on the next one.
    fn row(text: &str, wrapped: bool) -> Row {
        let cells = text.chars().map(|ch| Cell { ch, ..Cell::BLANK }).collect();
        Row::filled(cells, wrapped, false)
    }

    /// `(uri, runs)` of one link, the shape the assertions below compare.
    type Found = Vec<(String, Vec<(usize, usize, usize)>)>;

    /// The links of a grid of rows, as `(uri, runs)` for short assertions.
    fn found(rows: &[Row]) -> Found {
        let refs: Vec<&Row> = rows.iter().collect();
        scan(&refs)
            .into_iter()
            .map(|l| {
                (
                    l.uri,
                    l.runs.into_iter().map(|r| (r.row, r.col, r.len)).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn a_url_printed_as_text_is_a_link() {
        let rows = [row("see https://example.com/x for more", false)];
        let got = found(&rows);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "https://example.com/x");
        assert_eq!(got[0].1, vec![(0, 4, 21)]);
    }

    #[test]
    fn the_punctuation_of_the_sentence_is_not_part_of_the_url() {
        let rows = [row(
            "go: https://example.com/a, then; \"https://b.org/q\". ok",
            false,
        )];
        let got = found(&rows);
        assert_eq!(
            got.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
            vec!["https://example.com/a", "https://b.org/q"]
        );
    }

    #[test]
    fn a_bracket_that_balances_stays_inside_the_url() {
        let rows = [row(
            "see (https://en.wikipedia.org/wiki/Foo_(bar)) now",
            false,
        )];
        let got = found(&rows);
        assert_eq!(
            got[0].0, "https://en.wikipedia.org/wiki/Foo_(bar)",
            "the outer parenthesis is the sentence's, the inner one the URL's"
        );
    }

    #[test]
    fn a_soft_wrap_joins_the_two_halves() {
        let rows = [
            row("https://example.com/a/very/long", true),
            row("/path/that/continues", false),
        ];
        let got = found(&rows);
        assert_eq!(got.len(), 1, "a wrapped URL is one link, not two");
        assert_eq!(
            got[0].0,
            "https://example.com/a/very/long/path/that/continues"
        );
        assert_eq!(got[0].1, vec![(0, 0, 31), (1, 0, 20)]);
    }

    #[test]
    fn a_hard_newline_does_not_join() {
        let rows = [
            row("https://example.com/a", false),
            row("/path/that/continues", false),
        ];
        let got = found(&rows);
        assert_eq!(
            got.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
            vec!["https://example.com/a"],
            "two strings printed on two lines are two strings"
        );
    }

    #[test]
    fn a_scheme_alone_is_not_a_link() {
        for text in [
            "http://",
            "https:// ",
            "ftp:/x",
            "mailto:",
            "mailto:nobody",
            "http://.com",
            "see http:// and nothing else",
        ] {
            assert!(
                found(&[row(text, false)]).is_empty(),
                "{text:?} must not become a link"
            );
        }
    }

    #[test]
    fn a_host_without_a_domain_needs_to_be_localhost() {
        let got = found(&[
            row("http://build:8080/x", false),
            row("http://localhost:8080/x", false),
            row("http://127.0.0.1:8080/", false),
            row("http://[::1]:8080/", false),
        ]);
        assert_eq!(
            got.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
            vec![
                "http://localhost:8080/x",
                "http://127.0.0.1:8080/",
                "http://[::1]:8080/"
            ]
        );
    }

    #[test]
    fn an_unknown_scheme_is_not_a_link() {
        let got = found(&[
            row("javascript:alert(1)", false),
            row("data:text/html,<b>", false),
            row("ssh://host/", false),
        ]);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn the_scheme_is_matched_without_case_and_the_text_is_kept_as_typed() {
        let got = found(&[row("HTTPS://Example.COM/Path", false)]);
        assert_eq!(got[0].0, "HTTPS://Example.COM/Path");
        assert!(opens("HTTPS://Example.COM/Path"));
    }

    #[test]
    fn a_word_that_ends_in_a_scheme_is_not_a_link() {
        let got = found(&[row("xhttps://example.com no", false)]);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn a_wide_character_counts_as_two_columns_of_the_link() {
        // `https://例え.jp`: the grid stores a wide glyph plus the continuation
        // cell, and the run counts columns, not characters
        let mut cells: Vec<Cell> = "https://"
            .chars()
            .map(|ch| Cell { ch, ..Cell::BLANK })
            .collect();
        for ch in ['例', 'え'] {
            cells.push(Cell { ch, ..Cell::BLANK });
            cells.push(Cell {
                ch: crate::cell::WIDE_CONT,
                ..Cell::BLANK
            });
        }
        cells.extend(".jp".chars().map(|ch| Cell { ch, ..Cell::BLANK }));
        let rows = [Row::filled(cells, false, false)];
        let got = found(&rows);
        assert_eq!(got[0].0, "https://例え.jp");
        // 8 + two wide glyphs (2 columns each) + 3
        assert_eq!(got[0].1, vec![(0, 0, 15)]);
    }

    #[test]
    fn the_scan_offers_only_what_the_opener_accepts() {
        for scheme in SCHEMES {
            let text = if scheme.starts_with("mailto:") {
                format!("{scheme}user@example.com")
            } else {
                format!("{scheme}example.com/x")
            };
            let got = found(&[row(&text, false)]);
            assert_eq!(got.len(), 1, "{scheme} must be a link");
            assert!(opens(&got[0].0), "{scheme} must be openable");
        }
        assert!(!opens("javascript:alert(1)"));
        assert!(!opens("data:text/plain,x"));
        assert!(!opens(""));
    }

    #[test]
    fn a_long_line_stops_at_the_cap() {
        let text = "https://a.com/x ".repeat(MAX_LINKS + 10);
        let got = found(&[row(&text, false)]);
        assert_eq!(got.len(), MAX_LINKS);
    }
}
