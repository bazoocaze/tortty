//! The snapshot of a tab: what `restore_state = content` writes on the way out
//! and feeds back on the way in (`B45`).
//!
//! It is **ANSI**, not a private format, because the reader already exists: the
//! parser. Feeding the bytes of a snapshot into a fresh `Term` rebuilds the same
//! picture, and a snapshot taken in a 200-column window reopened in an 80-column
//! one reflows by the same code that reflows a live resize (soft-wrapped rows
//! come back without a hard newline, so the paragraph is still a paragraph).
//!
//! What goes in: text, SGR and `OSC 8` links. What stays out: everything that
//! *does* something — image, clipboard (`OSC 52`), notification, title. A
//! snapshot is a photograph of the screen, and the process that drew it is gone.

use std::fmt::Write as _;

use crate::cell::{
    Cell, BLINK, BOLD, FAINT, INV, ITALIC, OVERLINE, STRIKE, UL_CURLY, UL_DASHED, UL_DOTTED,
    UL_DOUBLE, UL_NONE, UL_SINGLE,
};
use crate::color::Color;
use crate::grid::Row;
use crate::select::Span;
use crate::term::Term;

/// The pen as SGR sees it: colors and attributes, no link.
#[derive(Copy, Clone, PartialEq, Eq, Default)]
struct Pen {
    fg: Color,
    bg: Color,
    ul_color: Color,
    flags: u8,
    ul: u8,
}

impl Pen {
    fn of(c: &Cell) -> Pen {
        Pen {
            fg: c.fg,
            bg: c.bg,
            ul_color: c.ul_color,
            flags: c.flags,
            ul: c.ul,
        }
    }

    /// The sequence for this pen, always starting from a reset: the state it
    /// replaces cannot be wrong if it is thrown away first, and a bad snapshot
    /// would leak one style into the whole screen otherwise.
    fn sgr(&self) -> String {
        if *self == Pen::default() {
            return "\x1b[0m".to_string();
        }
        let mut s = String::from("\x1b[0");
        for (bit, code) in [
            (BOLD, ";1"),
            (FAINT, ";2"),
            (ITALIC, ";3"),
            (BLINK, ";5"),
            (INV, ";7"),
            (STRIKE, ";9"),
            (OVERLINE, ";53"),
        ] {
            if self.flags & bit != 0 {
                s.push_str(code);
            }
        }
        match self.ul {
            UL_SINGLE => s.push_str(";4"),
            UL_DOUBLE => s.push_str(";4:2"),
            UL_CURLY => s.push_str(";4:3"),
            UL_DOTTED => s.push_str(";4:4"),
            UL_DASHED => s.push_str(";4:5"),
            _ => {},
        }
        color(&mut s, self.fg, 38, 39);
        color(&mut s, self.bg, 48, 49);
        if self.ul != UL_NONE {
            color(&mut s, self.ul_color, 58, 59);
        }
        s.push('m');
        s
    }
}

/// `SGR` for one color. `base` is the extended form (`38`, `48`, `58`) and
/// `plain` the code that means "the default one" (`39`, `49`, `59`).
///
/// The first 16 palette indices have short forms — `30..37`/`40..47` for the
/// normal eight, `90..97`/`100..107` for the bright ones — which is why the
/// arithmetic is written as `base - 8` and `base + 52` instead of a table.
fn color(out: &mut String, col: Color, base: u8, plain: u8) {
    match col {
        Color::Default => {
            let _ = write!(out, ";{plain}");
        },
        Color::Idx(i) if base != 58 && i < 8 => {
            let _ = write!(out, ";{}", base - 8 + i);
        },
        Color::Idx(i) if base != 58 && i < 16 => {
            let _ = write!(out, ";{}", base + 52 + i - 8);
        },
        Color::Idx(i) => {
            let _ = write!(out, ";{base};5;{i}");
        },
        Color::Rgb(red, green, blue) => {
            let _ = write!(out, ";{base};2;{red};{green};{blue}");
        },
    }
}

/// True for a cell that draws nothing and carries nothing: the trailing padding
/// of a row. A blank with a background color or an attribute is content.
fn padding(c: &Cell) -> bool {
    c.ch == ' ' && c.link == 0 && !c.has_style()
}

/// The snapshot of `t` as ANSI bytes.
pub fn snapshot(t: &Term) -> Vec<u8> {
    let mut out = picture(t, true, "\r\n");
    // On the alt screen the visible grid belongs to the app (vim repaints it),
    // so the snapshot carries the primary screen and the primary cursor is not
    // known while the app holds the screen: no `CUP` there.
    if !t.in_alt_screen() {
        let _ = write!(out, "\x1b[{};{}H", t.cy + 1, t.cx + 1);
    }
    out.into_bytes()
}

/// The tab as **text to read** (`B81`): the history and the screen joined by
/// `\n` — a file for the pager, not a picture for the parser. `colour` decides
/// whether `SGR` and links ride along (the `less -R` contract: we picked the
/// pager, so the dump keeps the output as it was); plain is what a pager we
/// did not pick receives — escapes in it are soup that breaks search.
/// No cursor addressing: the reader of this file is a human, not a `Term`.
pub fn dump(t: &Term, colour: bool) -> Vec<u8> {
    picture(t, colour, "\n").into_bytes()
}

/// One command's output as text to read (`B79`): the rows of `span`, the same
/// walk and the same colour contract as [`dump`].
pub fn dump_span(t: &Term, span: &Span, colour: bool) -> Vec<u8> {
    let (lo, hi) = (span.a.line.min(span.b.line), span.a.line.max(span.b.line));
    let rows = (lo..=hi).filter_map(|l| t.abs_row(l));
    walk(t, rows, colour, "\n").into_bytes()
}

/// The one walk both readers share: every row handed over, trimmed of padding,
/// clusters and links whole, pen changes emitted only when `colour`.
fn walk<'a>(t: &'a Term, rows: impl Iterator<Item = &'a Row>, colour: bool, sep: &str) -> String {
    let mut out = String::new();
    let mut pen = Pen::default();
    let mut link: u16 = 0;
    let mut prev_wrapped = false;
    let mut first = true;
    for row in rows {
        if !first && !prev_wrapped {
            out.push_str(sep);
        }
        first = false;
        prev_wrapped = row.wrapped;
        // a soft-wrapped row is full by definition: its trailing spaces are
        // content, and trimming them would move the wrap
        // a space that carries the rest of a cluster is content, not padding
        let end = if row.wrapped {
            row.cells.len()
        } else {
            row.cells
                .iter()
                .enumerate()
                .rposition(|(x, c)| !padding(c) || !row.tail_at(x).is_empty())
                .map_or(0, |i| i + 1)
        };
        for (x, c) in row.cells[..end].iter().enumerate() {
            if c.is_cont() {
                continue; // the wide char before it takes the two cells
            }
            if colour && c.link != link {
                if link != 0 {
                    out.push_str("\x1b]8;;\x1b\\");
                }
                link = 0;
                if let Some(uri) = c
                    .link
                    .checked_sub(1)
                    .and_then(|i| t.links.get(usize::from(i)))
                {
                    out.push_str("\x1b]8;;");
                    // the URI came from the child: a control character in it
                    // would end the sequence early and inject another one
                    out.extend(uri.chars().filter(|ch| !ch.is_control()));
                    out.push_str("\x1b\\");
                    link = c.link;
                }
            }
            let p = Pen::of(c);
            if colour && p != pen {
                out.push_str(&p.sgr());
                pen = p;
            }
            out.push(c.ch);
            // the codepoints of the cluster live beside the row, not in the cell
            // (`B66`): without them the restored screen draws the base alone —
            // the family back as one face, the flag back as one letter
            out.extend(row.tail_at(x).iter());
        }
    }
    if colour && link != 0 {
        out.push_str("\x1b]8;;\x1b\\");
    }
    if colour && pen != Pen::default() {
        out.push_str("\x1b[0m");
    }
    out
}

/// The whole tab — history and screen, chrome rows out — as the walk sees it.
fn picture(t: &Term, colour: bool, sep: &str) -> String {
    walk(
        t,
        t.scrollback
            .iter()
            .chain(t.primary().iter())
            .filter(|r| !r.chrome),
        colour,
        sep,
    )
}

#[cfg(test)]
mod checks_snapshot {
    use super::{dump, dump_span, snapshot};
    use crate::select::Span;
    use crate::term::Term;

    /// Feed the same bytes twice, into two different widths.
    fn round(bytes: &[u8], cols: usize, rows: usize) -> Term {
        let mut t = Term::new(cols, rows);
        t.feed(bytes);
        t
    }

    /// Every line of the history and of the screen, trailing spaces trimmed.
    fn all_text(t: &Term) -> Vec<String> {
        let mut v: Vec<String> = t.history_text(0, t.scrollback.len());
        v.extend(t.visible_rows(0).iter().map(|r| r.text()));
        v
    }

    #[test]
    fn a_snapshot_comes_back_as_the_same_screen() {
        let mut t = Term::new(20, 6);
        t.feed(b"\x1b[1;31mred bold\x1b[0m plain\r\n");
        t.feed(b"\x1b[4:3mcurly\x1b[0m \x1b[7minverse\x1b[0m\r\n");
        t.feed(b"w\xc3\xa9 and \xe6\x97\xa5\xe6\x9c\xac wide\r\n");
        for i in 0..3 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        let before = all_text(&t);
        let copy = round(&snapshot(&t), 20, 6);
        assert_eq!(all_text(&copy), before);
        assert_eq!(copy.scrollback.len(), t.scrollback.len());
    }

    #[test]
    fn the_snapshot_carries_the_cursor() {
        let mut t = Term::new(20, 6);
        t.feed(b"abc\r\ndefgh\r\nij\r\n");
        assert_eq!(
            (t.cx, t.cy),
            (0, 3),
            "the last LF leaves the cursor on row 3"
        );
        let copy = round(&snapshot(&t), 20, 6);
        assert_eq!((copy.cx, copy.cy), (t.cx, t.cy));
    }

    #[test]
    fn a_wrapped_row_comes_back_as_one_paragraph() {
        // 40 columns of text in a 20-column window: two soft-wrapped rows
        let mut t = Term::new(20, 6);
        t.feed(b"0123456789012345678901234567890123456789");
        assert!(t.grid[0].wrapped, "the row soft-wraps at 20 columns");
        let snap = snapshot(&t);
        let text = String::from_utf8_lossy(&snap).to_string();
        assert!(
            text.starts_with("0123456789012345678901234567890123456789"),
            "a soft wrap is not a hard newline: {text:?}"
        );
        // reopened wider, the paragraph is one line again
        let wide = round(&snap, 40, 6);
        assert_eq!(wide.row_text(0), "0123456789012345678901234567890123456789");
        // and in the original width it is the same two rows
        let same = round(&snap, 20, 6);
        assert_eq!(all_text(&same), all_text(&t));
    }

    /// The shapes `emoji-check.txt` is made of — the ones `B66` taught the grid
    /// to compose. A restore that dropped the tail brought back the picture from
    /// before it, which is what B67 was.
    #[test]
    fn a_cluster_survives_the_snapshot() {
        for (seq, wide) in [
            ("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", true), // family, joiner
            ("\u{1F1E7}\u{1F1F7}", true),                          // flag, two indicators
            ("\u{1F44D}\u{1F3FD}", true),                          // skin tone
            ("\u{2764}\u{FE0F}", true), // a selector turns the heart into a picture
            ("\u{2764}", false),        // and without it the heart stays a glyph
        ] {
            let mut t = Term::new(20, 4);
            t.feed(seq.as_bytes());
            let tail = t.primary()[0].tail_at(0).to_vec();
            let copy = round(&snapshot(&t), 20, 4);
            assert_eq!(copy.primary()[0].cells[0].ch, t.primary()[0].cells[0].ch);
            assert_eq!(copy.primary()[0].tail_at(0), tail.as_slice(), "{seq:?}");
            assert!(tail.is_empty() || wide, "{seq:?} composes a tail: {tail:?}");
            assert_eq!(
                copy.primary()[0].cells[1].is_cont(),
                wide,
                "{seq:?} keeps its width"
            );
            assert_eq!(
                t.primary()[0].cells[1].is_cont(),
                wide,
                "{seq:?} had that width before"
            );
        }
    }

    #[test]
    fn the_alt_screen_is_not_what_comes_back() {
        let mut t = Term::new(20, 6);
        t.feed(b"shell prompt\r\n");
        t.feed(b"\x1b[?1049h"); // an app takes the screen
        t.feed(b"vim owns this");
        assert!(t.in_alt_screen());
        let text = String::from_utf8_lossy(&snapshot(&t)).to_string();
        assert!(text.contains("shell prompt"));
        assert!(!text.contains("vim owns this"));
    }

    #[test]
    fn the_terminal_marker_is_not_content() {
        // the marker is fed as decoration (`B48`): it is on the screen and out of
        // the photograph
        let mut t = Term::new(30, 4);
        t.feed(b"before\r\n");
        t.feed_chrome(b"\x1b[4;1H\r\n-- restored --\r\n");
        t.feed(b"after\r\n");
        let file = String::from_utf8_lossy(&snapshot(&t)).to_string();
        assert!(file.contains("before"), "{file:?}");
        assert!(file.contains("after"), "{file:?}");
        assert!(!file.contains("restored"), "the marker left: {file:?}");
        // and it survives the scrollback and the wrap: the mark is on the row, not
        // on a position
        for i in 0..20 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        let file = String::from_utf8_lossy(&snapshot(&t)).to_string();
        assert!(!file.contains("restored"), "after scrolling: {file:?}");
        // a resize reflows and the mark goes with the paragraph
        t.resize_keep(24, 4, None);
        let file = String::from_utf8_lossy(&snapshot(&t)).to_string();
        assert!(!file.contains("restored"), "after reflow: {file:?}");
    }

    #[test]
    fn a_link_survives_and_its_control_characters_do_not() {
        let mut t = Term::new(20, 6);
        t.feed(b"\x1b]8;;https://example.com/x\x1b\\link\x1b]8;;\x1b\\\r\n");
        let copy = round(&snapshot(&t), 20, 6);
        assert_eq!(
            copy.links.first().map(String::as_str),
            Some("https://example.com/x")
        );
        // an ESC inside the URI must not become an escape in the file
        let mut bad = Term::new(20, 2);
        bad.links.push("a\x1b[31mb".to_string());
        // a cell carrying link 1 (the parser is the only writer of `link` in
        // practice; here the field is set directly to keep it a unit test)
        bad.grid[0].cells[0].link = 1;
        bad.grid[0].cells[0].ch = 'X';
        let file = String::from_utf8_lossy(&snapshot(&bad)).to_string();
        assert!(file.contains('X'));
        assert!(
            file.contains("a[31mb"),
            "the ESC is dropped, the text around it is not: {file:?}"
        );
    }

    #[test]
    fn the_coloured_dump_carries_sgr_and_no_cursor_addressing() {
        let mut t = Term::new(20, 6);
        t.feed(b"\x1b[1;31mred\x1b[0m\r\nrest");
        let d = String::from_utf8_lossy(&dump(&t, true)).to_string();
        assert!(
            d.contains("1;31red") || (d.contains("1;31") && d.contains("red")),
            "the pen of the output rides along: {d:?}"
        );
        assert!(d.contains('\n'));
        assert!(!d.contains('H'), "no `CUP` for a human reader: {d:?}");
    }

    #[test]
    fn the_plain_dump_carries_no_escape_at_all() {
        let mut t = Term::new(20, 6);
        t.feed(b"\x1b[1;31mred\x1b[0m plain\r\n\x1b]8;;https://x\x1b\\link\x1b]8;;\x1b\\");
        let d = String::from_utf8_lossy(&dump(&t, false)).to_string();
        assert!(!d.contains('\x1b'), "escapes in a plain pager are soup");
        assert!(d.contains("red plain"));
        assert!(d.contains("link"));
    }

    #[test]
    fn the_dump_carries_the_history_and_the_screen() {
        let mut t = Term::new(20, 3);
        for i in 0..6 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        let d = String::from_utf8_lossy(&dump(&t, true)).to_string();
        assert!(d.contains("line 0"), "the whole cap, not only the screen");
        assert!(d.contains("line 5"));
    }

    #[test]
    fn the_span_dump_takes_only_the_range() {
        let mut t = Term::new(20, 6);
        t.feed(b"header\r\n\x1b[1;31merr one\x1b[0m\r\nerr two\r\ntail");
        let span = Span::new(t.anchor_at(0, 1, 0), t.anchor_at(0, 2, 6));
        let d = String::from_utf8_lossy(&dump_span(&t, &span, true)).to_string();
        assert!(!d.contains("header"), "before the span stays out: {d:?}");
        assert!(!d.contains("tail"), "after the span stays out: {d:?}");
        assert!(d.contains("err one") && d.contains("err two"));
        assert!(
            d.contains("1;31"),
            "the colour rides when the reader speaks SGR"
        );
        let p = String::from_utf8_lossy(&dump_span(&t, &span, false)).to_string();
        assert!(!p.contains('\x1b'));
    }
}
