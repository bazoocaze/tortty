//! N5: history, anchor, damage and selection (equivalent to `checks.py:409-640`).
//!
//! A check builds the scenario, exercises it and compares against the expected
//! value. Nothing here needs a window, GPU or pty.
//!
//! A case already covered by another check here (`paste_markers`,
//! `selection_follows_history`) does not show up again: the goal is to cover what
//! is missing, not to duplicate a name or an assertion.

use crate::checks::Check;
use crate::color::Color;
use crate::config::Theme;
use crate::grid::Row;
use crate::term::Term;

/// Checks of this area, in report order.
#[must_use]
pub fn all() -> Vec<Check> {
    vec![
        Check {
            name: "history",
            run: history,
        },
        Check {
            name: "scroll_anchor",
            run: scroll_anchor,
        },
        Check {
            name: "redraw_keeps_history_position",
            run: redraw_keeps_history_position,
        },
        Check {
            name: "draw_damage",
            run: draw_damage,
        },
        Check {
            name: "selection",
            run: selection,
        },
        Check {
            name: "truecolor",
            run: truecolor,
        },
        Check {
            name: "style_contrast",
            run: style_contrast,
        },
        Check {
            name: "graphics_kitty",
            run: graphics_kitty,
        },
    ]
}

// by value on purpose: the caller builds the expected value on the spot (`linhas(&["a"])`)
#[allow(clippy::needless_pass_by_value)]
fn expected<T: PartialEq + std::fmt::Debug>(
    what: &str,
    obtained: T,
    expected: T,
) -> Result<(), String> {
    if obtained == expected {
        Ok(())
    } else {
        Err(format!("{what}: expected {expected:?}, got {obtained:?}"))
    }
}

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_string()).collect()
}

/// Text of the `rows` lines of the view with `off` history lines.
fn text_of_view(t: &Term, off: usize) -> Vec<String> {
    t.visible_rows(off).iter().map(|r| r.text()).collect()
}

/// `off` that keeps the absolute line `linha` at the top of the view.
///
/// The `anchored_view_off` arithmetic: `scrolled` is an absolute
/// index and never goes back — not even when the history hits the ceiling and
/// drops an old line — so the absolute arithmetic does not drift line by line the
/// way the size difference did. The clamp is on the history length: if the line
/// died (CSI 3J) there is nothing to hold and the view returns to the live end.
fn off_of_anchor(t: &Term, line: i64) -> usize {
    (t.scrolled as i64 - line).clamp(0, t.scrollback.len() as i64) as usize
}

/// Damage: `None` = the whole grid, `Some(lines)` = only them.
///
/// The core keeps damage as `dirty`/`dirty_all`; consuming it is the draw's job,
/// so the read that clears it lives here (the app does the same in `app.rs`).
fn take_damage(t: &mut Term) -> Option<Vec<usize>> {
    if t.dirty_all {
        t.clear_damage();
        return None;
    }
    let lines: Vec<usize> = t
        .dirty
        .iter()
        .enumerate()
        .filter(|(_, d)| **d)
        .map(|(y, _)| y)
        .collect();
    t.clear_damage();
    Some(lines)
}

fn history() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    t.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\n");
    // the screen is a window: `off`=0 is the live end, `off`>0 pulls history up
    expected("live", text_of_view(&t, 0), lines(&["d", "e", ""]))?;
    expected("one line up", text_of_view(&t, 1), lines(&["c", "d", "e"]))?;
    expected("three lines off the top", t.scrolled, 3)?;
    // the window never exceeds `rows` lines (regression: `off` > rows returned
    // `off` lines and drawing got slower the further up one went)
    for off in [0, 1, t.rows, t.rows + 1, t.scrollback.len()] {
        expected(
            &format!("window with off={off}"),
            t.visible_rows(off).len(),
            t.rows,
        )?;
    }
    // bottom of the history: asking too much clamps on the oldest, not the live
    expected(
        "bottom of the history",
        text_of_view(&t, 99),
        lines(&["a", "b", "c"]),
    )?;
    expected("history has 3", t.scrollback.len(), 3)?;
    Ok(())
}

fn scroll_anchor() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    for i in 0..10 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    // the user went up two lines and stopped reading: the anchor is the top line
    let anchor = t.scrolled as i64 - 2;
    let top = t.visible_rows(2)[0].text();
    expected("top line read", top.clone(), "6".to_string())?;
    let antes = t.mark();
    for i in 10..15 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    // the output does not yank the view off the line being read
    expected(
        "the view follows the output",
        t.visible_rows(off_of_anchor(&t, anchor))[0].text(),
        top.clone(),
    )?;
    // and the app's incremental arithmetic (D10) reaches the same offset
    expected(
        "view_after_output",
        t.view_after_output(2, antes),
        off_of_anchor(&t, anchor),
    )?;
    // at the history ceiling `len()` stops growing: the size-difference
    // arithmetic would let the text drift; the absolute one does not
    for i in 15..1100 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    expected("history at the ceiling", t.scrollback.len(), 1000)?;
    let anchor = t.scrolled as i64 - 3;
    let top = t.visible_rows(3)[0].text();
    let cap = t.mark();
    for i in 1100..1110 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    let off = off_of_anchor(&t, anchor);
    expected("absolute offset", off, 13)?;
    // the app's incremental arithmetic must give the same **at the ceiling**: this
    // is where it subtracted the dropped lines and the view drifted — the reader
    // lost one line of content per new line, the opposite of D10
    expected(
        "view_after_output at the ceiling",
        t.view_after_output(3, cap),
        off,
    )?;
    expected(
        "same line at the top",
        t.visible_rows(off)[0].text(),
        top.clone(),
    )?;
    // the old arithmetic (fixed offset at 3) would show another line: 10 of drift
    if t.visible_rows(3)[0].text() == top {
        return Err(
            "fixed offset at 3 returned the same line: the test does not tell them apart"
                .to_string(),
        );
    }
    // clearing the history (CSI 3J) destroys the anchor: nothing to read, live
    t.feed(b"\x1b[3J");
    expected("ED3 returns to live", off_of_anchor(&t, anchor), 0)?;
    Ok(())
}

fn redraw_keeps_history_position() -> Result<(), String> {
    let mut t = Term::new(12, 4);
    t.scrollback_limit = 200;
    for i in 1..=40 {
        t.feed(format!("l{i:02}\r\n").as_bytes());
    }
    let off = 12usize; // the user scrolled 12 lines back
    let anchor = t.scrolled as i64 - off as i64;
    let before = t.visible_rows(off)[0].text();
    expected("line read", before.clone(), "l26".to_string())?;
    let gen = t.hist_gen;
    // exactly the start of pi's fullRender(clear)
    t.feed(b"\x1b[?2026h\x1b[2J\x1b[H\x1b[3J");
    if !t.sync_output {
        return Err("?2026 did not enter synchronized output mode".to_string());
    }
    expected("history generation", t.hist_gen, gen + 1)?;
    expected("3J empties the history", t.scrollback.len(), 0)?;
    // the absolute anchor died: without the pin, the clamp would return the live
    // end and, with the buffer rewritten, the view would slide to the top (the B20
    // bug)
    expected("absolute anchor dies", off_of_anchor(&t, anchor), 0)?;
    for i in 1..=40 {
        t.feed(format!("l{i:02}\r\n").as_bytes());
    }
    t.feed(b"\x1b[?2026l");
    if t.sync_output {
        return Err("?2026l did not turn synchronized output off".to_string());
    }
    if t.scrollback.len() < off {
        return Err(format!(
            "the app did not rewrite enough buffer: history with {}",
            t.scrollback.len()
        ));
    }
    // the distance from the live end survives, even with the absolute anchor dead
    let pin = off.min(t.scrollback.len());
    expected("distance preserved", pin, off)?;
    expected(
        "the line read comes back",
        t.visible_rows(pin)[0].text(),
        before.clone(),
    )?;
    // the absolute anchor instead gives the top of the new history — that was the bug
    expected(
        "absolute anchor at the top",
        off_of_anchor(&t, anchor),
        t.scrollback.len(),
    )?;
    if t.visible_rows(t.scrollback.len())[0].text() == before {
        return Err("the absolute anchor returned the line read".to_string());
    }
    Ok(())
}

fn draw_damage() -> Result<(), String> {
    let mut t = Term::new(10, 5);
    // the first draw is always the whole grid
    if take_damage(&mut t).is_some() {
        return Err("the first draw should be the whole grid".to_string());
    }
    expected("damage consumed", take_damage(&mut t), Some(Vec::new()))?;
    // one character on one line: only that line
    t.feed(b"\x1b[3;1Hx");
    expected("1 dirty cell 1 line", take_damage(&mut t), Some(vec![2]))?;
    expected("damage consumed", take_damage(&mut t), Some(Vec::new()))?;
    // running text on two lines
    t.feed(b"\x1b[1;1Habcdefghij\x1b[2;1Hyz");
    expected("two lines", take_damage(&mut t), Some(vec![0, 1]))?;
    // EL and ECH dirty only the current line
    t.feed(b"\x1b[2;3H\x1b[K");
    expected("EL", take_damage(&mut t), Some(vec![1]))?;
    t.feed(b"\x1b[2;1H\x1b[2X");
    expected("ECH", take_damage(&mut t), Some(vec![1]))?;
    // ED2 changes the whole screen
    t.feed(b"\x1b[2J");
    if take_damage(&mut t).is_some() {
        return Err("ED2 should dirty the whole screen".to_string());
    }
    // full-screen scroll does not dirty the grid: it reports the shift, and the
    // line coming in blank is the only one to remount (drawing slides the rest)
    t.feed(b"\x1b[5;1H\n");
    if t.scroll_shift != 1 {
        return Err(format!(
            "scroll had to report shift 1, got {}",
            t.scroll_shift
        ));
    }
    if t.dirty_all {
        return Err("full-screen scroll cannot dirty the whole grid".to_string());
    }
    if !t.dirty.get(t.rows - 1).copied().unwrap_or(false) {
        return Err("the line that came in blank had to be dirty".to_string());
    }
    // region scroll has no full-screen shift: global damage
    t.clear_damage();
    t.feed(b"\x1b[2;3r"); // region 2..3
    t.feed(b"\x1b[3;1H\n");
    if !t.dirty_all {
        return Err("region scroll had to dirty the whole grid".to_string());
    }
    t.feed(b"\x1b[r");
    t.feed(b"\x1b[2J");
    t.resize(8, 4);
    if take_damage(&mut t).is_some() {
        return Err("reflow moves every line: damage had to be the grid".to_string());
    }
    t.feed(b"\x1bc");
    if take_damage(&mut t).is_some() {
        return Err("RIS restarts the grid: damage had to be the grid".to_string());
    }
    Ok(())
}

fn selection() -> Result<(), String> {
    let mut t = Term::new(20, 5);
    t.feed(b"abc\r\ndef\r\nghi");
    let rows: Vec<&Row> = t.grid.iter().collect();
    expected(
        "multi-line",
        t.selection_text((0, 1), (1, 1), &rows),
        "bc\nde".to_string(),
    )?;
    expected(
        "single line",
        t.selection_text((0, 0), (0, 2), &rows),
        "abc".to_string(),
    )?;
    // dragging bottom-up is normal: the order is normalized
    expected(
        "inverted order",
        t.selection_text((1, 1), (0, 1), &rows),
        "bc\nde".to_string(),
    )?;
    // inclusive ends and the final column comes in whole
    expected(
        "final column",
        t.selection_text((0, 2), (1, 0), &rows),
        "c\nd".to_string(),
    )?;
    // trailing space on each line is out (it is what every terminal copies)
    expected(
        "no trailing space",
        t.selection_text((0, 0), (0, 9), &rows),
        "abc".to_string(),
    )?;
    expected(
        "one cell",
        t.selection_text((2, 1), (2, 1), &rows),
        "h".to_string(),
    )?;
    Ok(())
}

fn truecolor() -> Result<(), String> {
    let mut t = Term::new(20, 5);
    t.feed(b"\x1b[38;2;18;52;86mA\x1b[48;2;250;128;114mB");
    expected(
        "fg RGB on the cell",
        t.grid[0].cells[0].fg,
        Color::Rgb(18, 52, 86),
    )?;
    expected(
        "bg RGB on the cell",
        t.grid[0].cells[1].bg,
        Color::Rgb(250, 128, 114),
    )?;
    // the ':' form (subparameters) has to give the same RGB
    t.feed(b"\x1b[38:2::7:8:9mC");
    expected("':' form", t.grid[0].cells[2].fg, Color::Rgb(7, 8, 9))?;
    // 256 is an index, not RGB: drawing resolves it through the theme palette
    let mut t = Term::new(20, 5);
    t.feed(b"\x1b[38;5;196mA\x1b[0mB");
    expected("index 196", t.grid[0].cells[0].fg, Color::Idx(196))?;
    expected("reset restores fg", t.grid[0].cells[1].fg, Color::Default)?;
    expected("reset restores bg", t.grid[0].cells[1].bg, Color::Default)?;
    // the color lives in the cell: reflow neither decodes nor loses the RGB
    let mut t = Term::new(4, 2);
    t.feed(b"\x1b[38;2;1;2;3mXYZW");
    t.resize(8, 2);
    expected(
        "RGB survives the resize",
        t.grid[0].cells[0].fg,
        Color::Rgb(1, 2, 3),
    )?;
    Ok(())
}

fn style_contrast() -> Result<(), String> {
    // WCAG 2.1 contrast: relative luminance
    // and ratio (L1+0.05)/(L2+0.05). The GUI chrome (cards, tabs, toast) does not
    // exist in the core — what exists here is the terminal theme.
    fn channel(v: u8) -> f64 {
        let v = f64::from(v) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }
    fn contrast(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> f64 {
        let (a, b) = (luminance(fg), luminance(bg));
        let (light, dark) = if a > b { (a, b) } else { (b, a) };
        (light + 0.05) / (dark + 0.05)
    }
    let theme = Theme::default();
    let bg = theme.rgb(theme.bg, (0, 0, 0));
    // normal text asks for 4.5:1; the cursor is a UI component (WCAG 1.4.11: 3:1)
    let pairs = [
        ("fg over the background", theme.fg, 4.5),
        ("cursor over the background", theme.cursor, 3.0),
    ];
    for (what, cor, minimo) in pairs {
        let ratio = contrast(theme.rgb(cor, bg), bg);
        if ratio < minimo {
            return Err(format!(
                "{what}: contrast {ratio:.2} below the minimum {minimo:.1}"
            ));
        }
    }
    Ok(())
}

/// kitty images end to end through the parser (H1–H13): APC, transmission,
/// placement, reply, scrolling along with the text and `ED 3`.
fn graphics_kitty() -> Result<(), String> {
    use base64::Engine as _;
    let mut t = Term::new(20, 4);
    t.cell_size = (10, 19);
    let rgba = base64::engine::general_purpose::STANDARD.encode([9, 8, 7, 255].repeat(4));
    t.feed(format!("\x1b_Ga=T,f=32,s=2,v=2,i=3;{rgba}\x1b\\").as_bytes());
    expected("image stored", t.graphics.images().len(), 1)?;
    expected(
        "APC reply",
        t.take_child_out(),
        b"\x1b_Gi=3;OK\x1b\\".to_vec(),
    )?;
    let p = t.graphics.placements()[0];
    expected(
        "placement",
        (p.image_id, p.col, p.line, p.cols, p.rows),
        (3, 0, 0, 1, 1),
    )?;
    // H10: without `C=1` the cursor moves down the image lines
    expected("cursor moved down", t.cy, 1)?;
    // H12: scrolling takes the placement into the history
    for _ in 0..6 {
        t.feed(b"\r\n");
    }
    let p = t.graphics.placements()[0];
    if p.line >= 0 {
        return Err(format!("image should be in the history: line {}", p.line));
    }
    // H13: `ED 3` clears image and history
    t.feed(b"\x1b[3J");
    expected("ED 3 clears", t.graphics.images().len(), 0)?;
    expected(
        "ED 3 clears the placement",
        t.graphics.placements().len(),
        0,
    )
}
