//! Display-less smoke checks (`tortty --selftest`).
//!
//! They run in the binary: one function per area, all using only the core (no
//! window, GPU or pty). The goal is to have **one command** that answers "is the emulator still
//! whole?" on any machine, including on CI and over ssh.
//!
//! Rule: every check builds the scenario, exercises it and **compares against the
//! expected value** — never "it did not panic". On failure it returns the reason
//! as text.

use crate::cell::{BOLD, ITALIC, UL_CURLY};
use crate::color::Color;
use crate::config::Config;
use crate::select::Span;
use crate::term::Term;

/// One check: name (the same concept as `checks.py`) and the function.
pub struct Check {
    pub name: &'static str,
    pub run: fn() -> Result<(), String>,
}

/// All checks, in the order they appear in the report.
#[must_use]
pub fn all() -> Vec<Check> {
    let mut v = vec![
        Check {
            name: "parser_text_and_cursor",
            run: parser_text_and_cursor,
        },
        Check {
            name: "wrap_and_control",
            run: wrap_and_control,
        },
        Check {
            name: "sgr_and_erasing",
            run: sgr_and_erasing,
        },
        Check {
            name: "truecolor_and_palette",
            run: truecolor_and_palette,
        },
        Check {
            name: "charset_and_replies",
            run: charset_and_replies,
        },
        Check {
            name: "alt_screen_and_scroll",
            run: alt_screen_and_scroll,
        },
        Check {
            name: "history_and_scrollback",
            run: history_and_scrollback,
        },
        Check {
            name: "reflow",
            run: reflow,
        },
        Check {
            name: "selection_follows_history",
            run: selection_follows_history,
        },
        Check {
            name: "paste_markers",
            run: paste_markers,
        },
        Check {
            name: "csi_with_intermediate",
            run: csi_with_intermediate,
        },
        Check {
            name: "config_defaults",
            run: config_defaults,
        },
        Check {
            name: "theme_osc",
            run: theme_osc,
        },
        Check {
            name: "damage_tracking",
            run: damage_tracking,
        },
    ];
    // N2 (config/CLI) and N5 (history/scroll/selection) live in their own
    // modules only so this file does not become 3 thousand lines.
    v.extend(crate::checks_n2::all());
    v.extend(crate::checks_n5::all());
    v
}

/// Runs everything. Returns `(passed, failures)` with the text of each failure.
#[must_use]
pub fn run_all() -> (usize, Vec<String>) {
    let mut ok = 0;
    let mut failures = Vec::new();
    for c in all() {
        match (c.run)() {
            Ok(()) => ok += 1,
            Err(e) => failures.push(format!("{}: {e}", c.name)),
        }
    }
    (ok, failures)
}

/// `--selftest` also runs under `cargo test`: without this the checks only
/// existed in the binary, and a green `cargo test -p tortty-core` said nothing
/// about them.
#[cfg(test)]
#[test]
fn selftest_complete() {
    let (ok, failures) = run_all();
    assert!(
        failures.is_empty(),
        "{ok} ok, falhas:\n{}",
        failures.join("\n")
    );
}

fn term() -> Term {
    Term::new(20, 5)
}

// by value on purpose: the caller builds the expected value on the spot (`"abc".to_string()`)
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

fn parser_text_and_cursor() -> Result<(), String> {
    let mut t = term();
    t.feed(b"abc");
    expected("text", t.row_text(0), "abc".to_string())?;
    expected("cursor", (t.cy, t.cx), (0, 3))?;
    // CUP, CR, LF, BS and TAB
    t.feed(b"\x1b[3;5Hxy\r\nz\tw");
    expected("CUP+CRLF", t.row_text(3), "z       w".to_string())?;
    // the cursor is past the "w" (column 9); BS goes back to 8 and overwrites
    t.feed(b"\x08!");
    expected("backspace", t.row_text(3), "z       !".to_string())?;
    t.feed(b"\x1b[1;1H\x1b[2X");
    expected("ECH erases 2", t.row_text(0), "  c".to_string())?;
    Ok(())
}

fn wrap_and_control() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    t.feed(b"0123456789A");
    expected(
        "wrap",
        (t.row_text(0), t.row_text(1)),
        ("0123456789".to_string(), "A".to_string()),
    )?;
    if !t.grid[0].wrapped || t.grid[1].wrapped {
        return Err("wrong soft-wrap mark".to_string());
    }
    // autowrap off does not wrap: it overwrites the last column
    t.feed(b"\x1b[?7l");
    t.feed(b"\x1b[1;1H0123456789XY");
    expected("no autowrap", t.row_text(0), "012345678Y".to_string())?;
    t.feed(b"\x1b[?7h");
    Ok(())
}

fn sgr_and_erasing() -> Result<(), String> {
    let mut t = term();
    t.feed(b"\x1b[1;3;4:3;38;2;1;2;3mA");
    let c = t.grid[0].cells[0];
    if c.flags & BOLD == 0 || c.flags & ITALIC == 0 || c.ul != UL_CURLY {
        return Err("SGR attributes did not stick".to_string());
    }
    expected("truecolor", c.fg, Color::Rgb(1, 2, 3))?;
    t.feed(b"\x1b[0m\x1b[2J");
    expected("ED2 clears", t.row_text(0), String::new())?;
    expected("ED2 homes the cursor", (t.cy, t.cx), (0, 0))?;
    Ok(())
}

fn truecolor_and_palette() -> Result<(), String> {
    let (cfg, errs) = Config::parse("color1 #ff0000\nforeground #010203\n");
    if !errs.is_empty() {
        return Err(format!("config with errors: {errs:?}"));
    }
    let pal = cfg.palette256();
    expected("theme palette", pal[1], (0xff, 0, 0))?;
    expected("6x6x6 cube intact", pal[196], (255, 0, 0))?;
    let theme = cfg.theme();
    expected("theme fg", theme.rgb(theme.fg, (0, 0, 0)), (1, 2, 3))?;
    Ok(())
}

fn charset_and_replies() -> Result<(), String> {
    let mut t = term();
    t.feed(b"\x1b(0lqk\x1b(B");
    let s: String = t.grid[0].cells[0..3].iter().map(|c| c.ch).collect();
    expected(
        "DEC special graphics",
        s,
        "\u{250c}\u{2500}\u{2510}".to_string(),
    )?;
    t.feed(b"\x1b[c");
    let out = String::from_utf8_lossy(&t.take_child_out()).to_string();
    if !out.contains("\x1b[?") {
        return Err(format!("DA1 did not answer: {out:?}"));
    }
    t.feed(b"\x1b[6n");
    let out = String::from_utf8_lossy(&t.take_child_out()).to_string();
    if !out.contains('R') {
        return Err(format!("CPR did not answer: {out:?}"));
    }
    Ok(())
}

fn alt_screen_and_scroll() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    t.feed(b"um\ndois");
    t.feed(b"\x1b[?1049h");
    if !t.in_alt_screen() {
        return Err("did not enter the alternate screen".to_string());
    }
    t.feed(b"no alt\x1b[?1049l");
    expected("back from alt", t.row_text(0), "um".to_string())?;
    // scroll region (DECSTBM): only the middle scrolls
    t.feed(b"\x1b[2;3r\x1b[3;1H\n");
    expected("top untouched", t.row_text(0), "um".to_string())?;
    t.feed(b"\x1b[r");
    Ok(())
}

fn history_and_scrollback() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    for i in 0..8 {
        t.feed(format!("l{i}\r\n").as_bytes());
    }
    if t.scrollback.len() < 5 {
        return Err(format!("history too short: {}", t.scrollback.len()));
    }
    expected("last live line", t.row_text(2), String::new())?;
    // ED3 empties the history and bumps the generation (the anchor dies)
    let g = t.abs_gen;
    t.feed(b"\x1b[3J");
    expected("ED3 empties", t.scrollback.len(), 0)?;
    if t.abs_gen <= g {
        return Err("ED3 did not invalidate the anchors".to_string());
    }
    Ok(())
}

fn reflow() -> Result<(), String> {
    let mut t = Term::new(10, 4);
    t.feed(b"abcdefghijklmnopqrstuvwxy");
    t.resize(5, 4);
    let together: String = t
        .scrollback
        .iter()
        .chain(t.grid.iter())
        .map(crate::grid::Row::text)
        .collect::<String>();
    expected(
        "text preserved",
        together,
        "abcdefghijklmnopqrstuvwxy".to_string(),
    )?;
    t.resize(10, 4);
    expected("back to the size", t.row_text(0), "abcdefghij".to_string())?;
    Ok(())
}

fn selection_follows_history() -> Result<(), String> {
    let mut t = Term::new(10, 4);
    t.feed(b"linha-a\r\nlinha-b\r\n");
    let a = t.anchor_at(0, 0, 0);
    let b = t.anchor_at(0, 0, 6);
    let span = Span::new(a, b);
    // scroll the screen: the anchor keeps pointing at the same text
    for _ in 0..3 {
        t.feed(b"\r\n");
    }
    let found = t.span_text(&span).ok_or("anchor died for no reason")?;
    expected("selection survives scroll", found, "linha-a".to_string())?;
    Ok(())
}

fn paste_markers() -> Result<(), String> {
    let mut t = term();
    if t.bracketed_paste {
        return Err("bracketed paste on by default".to_string());
    }
    t.feed(b"\x1b[?2004h");
    if !t.bracketed_paste {
        return Err("?2004 did not turn it on".to_string());
    }
    let payload = String::from_utf8_lossy(&t.paste_payload("oi")).to_string();
    expected("payload", payload, "\x1b[200~oi\x1b[201~".to_string())?;
    t.feed(b"\x1b[?2004l");
    Ok(())
}

fn csi_with_intermediate() -> Result<(), String> {
    let mut t = term();
    // DECSCUSR with an intermediate (space) and CSI ! p (soft reset)
    // DECSCUSR: 4 = steady underline -> (shape 1 = underline, blink false)
    t.feed(b"\x1b[4 q");
    expected("cursor underline", t.app_cursor_style, Some((1, false)))?;
    t.feed(b"\x1b[1;1;1m\x1b[!p");
    expected("soft reset clears SGR", t.flags, 0)?;
    Ok(())
}

fn config_defaults() -> Result<(), String> {
    let (cfg, errs) = Config::parse("font_size 15\nfont_size 16\nbell visual\n");
    if !errs.is_empty() {
        return Err(format!("unexpected errors: {errs:?}"));
    }
    expected("the repeated one wins", cfg.font_size, 16.0)?;
    expected("bell", format!("{:?}", cfg.bell), "Visual".to_string())?;
    let (_, errs) = Config::parse("nao_existe 1\nfont_size abc\n");
    if errs.len() != 2 {
        return Err(format!("expected 2 errors, got {errs:?}"));
    }
    Ok(())
}

fn theme_osc() -> Result<(), String> {
    let mut t = term();
    let default = t.theme.bg;
    t.feed(b"\x1b]11;?\x1b\\");
    let out = String::from_utf8_lossy(&t.take_child_out()).to_string();
    if !out.contains("]11;rgb:") {
        return Err(format!("OSC 11 did not answer: {out:?}"));
    }
    t.feed(b"\x1b]11;#102030\x1b\\");
    expected("OSC 11 sets", t.theme.bg, Color::Rgb(0x10, 0x20, 0x30))?;
    if !t.theme_dirty {
        return Err("theme_dirty did not rise".to_string());
    }
    t.feed(b"\x1b]111\x1b\\");
    expected("OSC 111 restores", t.theme.bg, default)?;
    Ok(())
}

fn damage_tracking() -> Result<(), String> {
    let mut t = term();
    t.feed(b"a\r\nb");
    if !t.dirty[0] {
        return Err("line with text is not dirty".to_string());
    }
    // the "everything dirty" marker exists for resize/RIS
    t.dirty_all = false;
    t.resize(12, 5);
    if !t.dirty_all {
        return Err("resize did not mark the whole screen dirty".to_string());
    }
    Ok(())
}
