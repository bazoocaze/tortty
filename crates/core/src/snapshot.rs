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
    let mut out = String::new();
    let mut pen = Pen::default();
    let mut link: u16 = 0;
    let mut prev_wrapped = false;
    let mut first = true;
    // the history and then the screen: the app's own reading order, and the
    // same one `visible_rows` uses
    // decoration is not content (`B48`): the marker the terminal writes to
    // separate a restored picture from the new run stays out of the picture
    for row in t
        .scrollback
        .iter()
        .chain(t.primary().iter())
        .filter(|r| !r.chrome)
    {
        if !first && !prev_wrapped {
            out.push_str("\r\n");
        }
        first = false;
        prev_wrapped = row.wrapped;
        // a soft-wrapped row is full by definition: its trailing spaces are
        // content, and trimming them would move the wrap
        let end = if row.wrapped {
            row.cells.len()
        } else {
            row.cells
                .iter()
                .rposition(|c| !padding(c))
                .map_or(0, |i| i + 1)
        };
        for c in &row.cells[..end] {
            if c.is_cont() {
                continue; // the wide char before it takes the two cells
            }
            if c.link != link {
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
            if p != pen {
                out.push_str(&p.sgr());
                pen = p;
            }
            out.push(c.ch);
        }
    }
    if link != 0 {
        out.push_str("\x1b]8;;\x1b\\");
    }
    if pen != Pen::default() {
        out.push_str("\x1b[0m");
    }
    // On the alt screen the visible grid belongs to the app (vim repaints it),
    // so the snapshot carries the primary screen and the primary cursor is not
    // known while the app holds the screen: no `CUP` there.
    if !t.in_alt_screen() {
        let _ = write!(out, "\x1b[{};{}H", t.cy + 1, t.cx + 1);
    }
    out.into_bytes()
}

#[cfg(test)]
mod checks_snapshot {
    use super::snapshot;
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
}
