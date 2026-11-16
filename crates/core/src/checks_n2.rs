//! N2: configuration and command line (equivalent to `checks.py:125-287`).
//!
//! Each check sets up the scenario, exercises it and compares with the expected
//! value. Nothing here needs a window, GPU or pty.
//!
//! What has no equivalent is left out on purpose: `shell` is only resolved in
//! `pty::spawn` (it needs a pty and `$SHELL`), and there is no notion of a
//! *warning* (repeated key): here the result is only `Error`.

use crate::checks::Check;
use crate::config::{Bell, Config, CursorShape};
use crate::grid::Row;
use crate::term::Term;

/// Checks in this area, in report order.
#[must_use]
pub fn all() -> Vec<Check> {
    vec![
        Check {
            name: "config_file",
            run: config_file,
        },
        Check {
            name: "config_options",
            run: config_options,
        },
        Check {
            name: "scrollback_and_bell",
            run: scrollback_and_bell,
        },
    ]
}

/// Every key of `config::KNOWN` (which is private). The list exists so
/// `DEFAULT_CONF` does not silently lose a key: the defaults are the only
/// source of the values, so every key has to be written there.
const KEYS: &[&str] = &[
    "shell",
    "font_family",
    "font_size",
    "cursor_shape",
    "cursor_blink",
    "restore_state",
    "scrollback",
    "window_padding_width",
    "bell",
    "copy_on_select",
    "debug_log",
    "foreground",
    "background",
    "cursor_color",
    "color0",
    "color1",
    "color2",
    "color3",
    "color4",
    "color5",
    "color6",
    "color7",
    "color8",
    "color9",
    "color10",
    "color11",
    "color12",
    "color13",
    "color14",
    "color15",
];

// by value on purpose: the caller builds the expected value on the spot
// (`"abc".to_string()`)
#[allow(clippy::needless_pass_by_value)]
fn expected<T: PartialEq + std::fmt::Debug>(
    o_que: &str,
    obtained: T,
    expected: T,
) -> Result<(), String> {
    if obtained == expected {
        Ok(())
    } else {
        Err(format!("{o_que}: expected {expected:?}, got {obtained:?}"))
    }
}

/// `checks.py:config_file` — kitty format, merge with the built-in defaults and
/// validation with per-line errors.
fn config_file() -> Result<(), String> {
    // the defaults are embedded in the binary as a file (the same treatment as
    // kitty's `DEFAULT_CONF`): they parse clean and are the only source
    let (padroes, errs) = Config::parse("");
    if !errs.is_empty() {
        return Err(format!("DEFAULT_CONF has an error: {errs:?}"));
    }
    expected(
        "defaults = parse of DEFAULT_CONF",
        padroes.clone(),
        Config::base(),
    )?;
    // every key has to appear in one of the four files: the family of the font,
    // for one, only exists in the templates
    let lines: Vec<&str> = crate::config::defaults_text()
        .iter()
        .flat_map(|t| t.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    for k in KEYS {
        if !lines
            .iter()
            .any(|l| l.split_whitespace().next() == Some(*k))
        {
            return Err(format!("the defaults do not document the key '{k}'"));
        }
    }
    // empty = inherits the environment $SHELL (the pty resolves it; here it is
    // only `None`)
    expected("empty shell", padroes.shell, None)?;
    // whole-line and end-of-line comment, spaces at the edges, repeated key (the
    // last wins) and unknown key in the middle
    let (cfg, errs) =
        Config::parse("# header\n  shell   /bin/sh   # inline\n\nfoo bar\nshell /bin/bash\n");
    expected("last repeat wins", cfg.shell.as_deref(), Some("/bin/bash"))?;
    expected("one invalid line", errs.len(), 1)?;
    expected(
        "error on the 'foo' line",
        (errs[0].line, errs[0].key.as_str()),
        (4, "foo"),
    )?;
    // merge key by key: the user only overrides what they wrote
    let (cfg, errs) = Config::parse("shell /bin/sh\n");
    expected("user shell", cfg.shell.as_deref(), Some("/bin/sh"))?;
    expected(
        "font still the factory one",
        cfg.font_family.as_str(),
        "Monospace",
    )?;
    expected("no file, no error", errs.len(), 0)?;
    // unknown key is an error, but the rest of the file HOLDS (initial load)
    let (cfg, errs) = Config::parse("foo bar\nshell /bin/sh\n");
    expected("the rest applied", cfg.shell.as_deref(), Some("/bin/sh"))?;
    expected("unknown key is an error", errs.len(), 1)?;
    // invalid value keeps the default (the key does not apply) and the error
    // goes to the toast
    let (cfg, errs) = Config::parse("# nothing here\nfont_size 200\n");
    expected("invalid does not apply", cfg.font_size, padroes.font_size)?;
    expected("error on line 2", errs.first().map(|e| e.line), Some(2))?;
    if !errs.iter().any(|e| e.msg.contains("outside the range")) {
        return Err(format!("error without the range reason: {errs:?}"));
    }
    // reload: the caller only applies with `errs` empty, so any error keeps the
    // configuration that was already in effect
    let (good, _) = Config::parse("font_size 16\n");
    let (_, errs) = good.over("font_size 18\nbell beep\n");
    if errs.len() != 1 {
        return Err(format!("invalid reload had to give 1 error: {errs:?}"));
    }
    expected(
        "the base is not mutated by the reload",
        good.font_size,
        16.0,
    )?;
    Ok(())
}

/// `checks.py:config_options` — font, cursor, history, padding, bell,
/// `copy_on_select` and `debug_log`, key by key: default, valid value and error.
fn config_options() -> Result<(), String> {
    // with no user file the defaults hold
    let (cfg, errs) = Config::parse("");
    if !errs.is_empty() {
        return Err(format!("defaults with error: {errs:?}"));
    }
    expected("font_family", cfg.font_family.as_str(), "Monospace")?;
    expected("font_size", cfg.font_size, 12.0)?;
    expected("cursor_shape", cfg.cursor_shape, CursorShape::Block)?;
    expected("cursor_blink", cfg.cursor_blink, false)?;
    expected("scrollback", cfg.scrollback, 10_000)?;
    expected("window_padding_width", cfg.padding, 0)?;
    expected("bell", cfg.bell, Bell::None)?;
    expected("copy_on_select", cfg.copy_on_select, false)?;
    expected("debug_log", cfg.debug_log.as_deref(), None)?;
    // valid value of each key: family with a space and a fractional font size
    let (cfg, errs) = Config::parse(
        "font_family DejaVu Sans Mono\nfont_size 12.5\ncursor_shape beam\n\
         cursor_blink yes\nscrollback 200\nwindow_padding_width 4\n\
         bell visual\ncopy_on_select on\n",
    );
    if !errs.is_empty() {
        return Err(format!("valid values gave an error: {errs:?}"));
    }
    expected(
        "family with a space",
        cfg.font_family.as_str(),
        "DejaVu Sans Mono",
    )?;
    expected("fractional font size", cfg.font_size, 12.5)?;
    expected("cursor beam", cfg.cursor_shape, CursorShape::Beam)?;
    expected("blink yes", cfg.cursor_blink, true)?;
    expected("scrollback", cfg.scrollback, 200)?;
    expected("padding", cfg.padding, 4)?;
    expected("bell visual", cfg.bell, Bell::Visual)?;
    expected("copy_on_select on", cfg.copy_on_select, true)?;
    // debug_log: a free path and `off` turning it off
    let (cfg, errs) = Config::parse("debug_log /tmp/other.log\n");
    expected(
        "debug_log path",
        cfg.debug_log.as_deref(),
        Some("/tmp/other.log"),
    )?;
    expected("debug_log with no error", errs.len(), 0)?;
    let (cfg, errs) = Config::parse("debug_log off\n");
    expected("debug_log off", cfg.debug_log.as_deref(), None)?;
    expected("debug_log off with no error", errs.len(), 0)?;
    // the error is per line: the valid one applies, the invalid one keeps the
    // previous default
    for (garbage, reason) in [
        ("font_size abc", "is not a number"),
        ("font_size 0", "outside the range"),
        ("font_family", "empty family"),
        ("cursor_shape square", "is not block"),
        ("cursor_blink maybe", "is not on/off"),
        ("scrollback -1", "is not a number"),
        ("window_padding_width 8px", "is not a number"),
        ("bell beep", "is not none"),
        ("copy_on_select 2", "is not on/off"),
    ] {
        let (cfg, errs) = Config::parse(&format!("font_size 20\n{garbage}\n"));
        expected(
            "the valid line before the error applied",
            cfg.font_size,
            20.0,
        )?;
        match errs.as_slice() {
            [e] if e.line == 2 && e.msg.contains(reason) => {},
            other => {
                return Err(format!(
                    "'{garbage}': expected 1 error on line 2 with '{reason}', got {other:?}"
                ));
            },
        }
    }
    // the initial load shows the first error plus a count of the others; here the
    // list comes out in line order and the valid line in the middle applies
    let (cfg, errs) = Config::parse("bell beep\nfont_size 20\ncursor_shape square\n");
    expected("bell stayed at the default", cfg.bell, Bell::None)?;
    expected("the font_size in the middle applied", cfg.font_size, 20.0)?;
    expected(
        "one error per line",
        errs.iter().map(|e| e.line).collect::<Vec<_>>(),
        vec![1, 3],
    )?;
    if !errs[0].msg.contains("is not none") || !errs[1].msg.contains("is not block") {
        return Err(format!("wrong reasons: {errs:?}"));
    }
    Ok(())
}

/// `checks.py:scrollback_and_bell` — configurable history ceiling, RIS does not
/// own the configuration, BEL counted (not silenced) and DECSCUSR becoming an
/// app request.
fn scrollback_and_bell() -> Result<(), String> {
    let mut t = Term::new(12, 3);
    t.scrollback_limit = 2;
    for i in 0..20 {
        t.feed(format!("line {i}\r\n").as_bytes());
    }
    // trims from the oldest, not from the one arriving
    expected("history ceiling", t.scrollback.len(), 2)?;
    expected(
        "the last two that scrolled out",
        t.scrollback.iter().map(Row::text).collect::<Vec<_>>(),
        vec!["line 16".to_string(), "line 17".to_string()],
    )?;
    // the live screen is not affected by the ceiling
    expected("live row 18", t.row_text(0), "line 18".to_string())?;
    expected("live row 19", t.row_text(1), "line 19".to_string())?;
    // lowering the ceiling trims on the next scroll (the immediate trim belongs
    // to the app, which touches the tab's `scrollback_limit` and prunes the
    // `VecDeque` right away)
    t.scrollback_limit = 0;
    t.feed(b"x\r\n");
    expected("zero ceiling empties it", t.scrollback.len(), 0)?;
    t.scrollback_limit = 2;
    for i in 0..10 {
        t.feed(format!("line {i}\r\n").as_bytes());
    }
    expected("history filled up again", t.scrollback.len(), 2)?;
    // DECRQM has to see the mouse modes that exist: `?1005` was outside the
    // table and answered 0 with the mode on
    let mut q = Term::new(10, 3);
    q.feed(b"\x1b[?1005h");
    q.take_child_out();
    q.feed(b"\x1b[?1005$p");
    expected(
        "DECRQM 1005 on",
        q.take_child_out(),
        b"\x1b[?1005;1$y".to_vec(),
    )?;
    q.feed(b"\x1b[?1005l");
    q.take_child_out();
    q.feed(b"\x1b[?1005$p");
    expected(
        "DECRQM 1005 off",
        q.take_child_out(),
        b"\x1b[?1005;2$y".to_vec(),
    )?;
    // RIS (ESC c) clears the screen but does not own the configuration
    t.feed(b"\x1bc");
    expected("RIS keeps the ceiling", t.scrollback_limit, 2)?;
    expected("RIS clears the history", t.scrollback.len(), 0)?;
    // BEL is counted, not silenced: the view decides (visual/audible/none)
    expected("no BEL yet", t.bells, 0)?;
    t.feed(b"\x07");
    expected("BEL counted", t.bells, 1)?;
    t.feed(b"\x07\x07");
    expected("BEL accumulates", t.bells, 3)?;
    // `bell none` in the config does not stop the counting: the emulator only
    // counts
    let (cfg, errs) = Config::parse("bell none\n");
    expected("bell none", (cfg.bell, errs.len()), (Bell::None, 0))?;
    t.feed(b"\x07");
    expected("config does not silence the counter", t.bells, 4)?;
    // DECSCUSR (CSI 6 q) becomes an app request; 0 goes back to the config
    // (0 = block, 1 = underline, 2 = beam; the bool is the "blink")
    t.feed(b"\x1b[4 q");
    expected(
        "steady underline cursor",
        t.app_cursor_style,
        Some((1, false)),
    )?;
    t.feed(b"\x1b[5 q");
    expected("blinking beam cursor", t.app_cursor_style, Some((2, true)))?;
    t.feed(b"\x1b[0 q");
    expected("CSI 0 q goes back to the config", t.app_cursor_style, None)?;
    Ok(())
}
