//! Configuration: kitty format (`key value`), built-in defaults and per-line
//! errors.
//!
//! The rule: **merge key by key**, with the defaults written as a file
//! (`DEFAULT_CONF`) and the user file on top.
//! The error is per line, not per file: whatever validated holds, the invalid
//! key keeps the previous default and the error comes out with the line number
//! (the app shows it in a toast). On reload the rule is stricter: **any** error
//! keeps the configuration that was already in effect — swapping everything for
//! half is worse than ignoring the edit.
//!
//! `#` comments when it sits at the start of the line or after a space, so
//! `foreground #e6e6e6` is still a colour and not a comment.

use crate::color::Color;

/// The defaults, written as the file the user would edit. It is the **common
/// base**: only the lines that mean the same thing on every system.
///
/// It lives in `defaults/tortty.conf`, not in a literal here, so that git sees
/// the configuration as configuration: no `\"` noise in a colour or a comment,
/// syntax highlighting, and a file the reader can point `--config` at. The keys
/// where the systems disagree live beside it, one template per system, and they
/// are parsed on top of this one (`Config::base_for`).
///
/// The split has a rule, and it is in `AGENTS.md`: a line is only in the base
/// when the same chord is correct, primary and free of a system collision on
/// all three. Nothing else may live here.
pub const DEFAULT_CONF: &str = include_str!("../defaults/tortty.conf");

/// The Linux template: copy/paste, tab and workspace open/close, the digits
/// under `alt`/`super` and the workspace walk, which are not the same chord
/// elsewhere.
const LINUX_CONF: &str = include_str!("../defaults/tortty-linux.conf");

/// The macOS template: `super` is the Command key there.
const MACOS_CONF: &str = include_str!("../defaults/tortty-macos.conf");

/// The Windows template: the Super key belongs to the shell there, so the family
/// does not exist.
const WINDOWS_CONF: &str = include_str!("../defaults/tortty-windows.conf");

/// The four files of the defaults, the base first — the order they are parsed in.
#[must_use]
pub fn defaults_text() -> [&'static str; 4] {
    [DEFAULT_CONF, LINUX_CONF, MACOS_CONF, WINDOWS_CONF]
}

/// Which system's template applies on top of the common base.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Os {
    Linux,
    Macos,
    Windows,
}

impl Os {
    /// The system we were built for. It is a compile-time choice: the binary
    /// carries one template and nothing is sniffed at run time.
    #[must_use]
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }

    /// The name the debug socket reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Windows => "windows",
        }
    }

    /// All three, for the tests that have to read the templates of systems this
    /// build cannot run on.
    pub const ALL: [Os; 3] = [Os::Linux, Os::Macos, Os::Windows];

    fn conf(self) -> &'static str {
        match self {
            Os::Linux => LINUX_CONF,
            Os::Macos => MACOS_CONF,
            Os::Windows => WINDOWS_CONF,
        }
    }
}

/// What comes back on the next run (`B45`).
///
/// The three degrees are a ladder: each one includes the previous. It says
/// nothing about the window — position, size and maximized state are remembered
/// whatever this is, because that is what anyone expects of an application with
/// a window.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum RestoreState {
    /// `none`: no layout is written at all — one tab on a clean screen.
    None,
    /// `layout` (the default): the workspaces and the tabs of the last run, in
    /// their order, the front where it was (`M-I`).
    #[default]
    Layout,
    /// `content`: the layout **and** what each tab had on screen and in the
    /// history. It is a photograph: the shell starts again in the same
    /// directory, the processes do not come back.
    Content,
}

/// How the app's BEL manifests itself.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Bell {
    #[default]
    None,
    Visual,
    Audible,
}

/// Typographic ligatures. Only takes effect in a font that has them (Fira Code,
/// JetBrains Mono…): the renderer does not fuse any cell, it asks HarfBuzz for
/// the glyph of each cell with the context of the run.
///
/// `Off` turns off **the ligatures**, not the shaping: Arabic and Devanagari
/// depend on shaping to be readable and keep being shaped.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum Ligatures {
    Off,
    #[default]
    Standard,
    /// `standard` + discretionary ones (`dlig`, `clig`).
    All,
}

/// Cursor shape when the app has not asked for another one via `CSI 6 q`.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Beam,
    Underline,
}

/// A line that did not pass: `line`, `key` and the reason.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Error {
    pub line: usize,
    pub key: String,
    pub msg: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.key.is_empty() {
            write!(f, "line {}: {}", self.line, self.msg)
        } else {
            write!(f, "line {}: '{}': {}", self.line, self.key, self.msg)
        }
    }
}

/// Theme colours as the terminal knows them: this is what the renderer uses and
/// what `OSC 4/10/11/12` queries and changes at run time.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Theme {
    pub fg: Color,
    pub bg: Color,
    pub cursor: Color,
    pub palette: [Color; 16],
}

impl Default for Theme {
    /// The same theme as the built-in defaults, without parsing text.
    fn default() -> Self {
        Theme {
            fg: Color::Rgb(0xe6, 0xe6, 0xe6),
            bg: Color::Rgb(0x16, 0x18, 0x1d),
            cursor: Color::Rgb(0xe6, 0xe6, 0xe6),
            palette: std::array::from_fn(|i| Color::Idx(i as u8)),
        }
    }
}

impl Theme {
    /// 256-colour palette with the first 16 coming from the theme.
    #[must_use]
    pub fn palette256(&self) -> Vec<(u8, u8, u8)> {
        let mut pal = crate::color::palette256();
        for (i, c) in self.palette.iter().enumerate() {
            if let Color::Rgb(r, g, b) = c {
                pal[i] = (*r, *g, *b);
            }
        }
        pal
    }

    /// Final RGB of a theme colour (the palette resolves the indices); `Default`
    /// falls back to the `default` the caller picks (background for space, text
    /// for glyph).
    #[must_use]
    pub fn rgb(&self, c: Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
        match c {
            Color::Default => default,
            other => crate::color::resolve(other, &self.palette256(), default),
        }
    }
}

/// The effective configuration.
#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    pub shell: Option<String>,
    pub font_family: String,
    pub font_size: f32,
    pub ligatures: Ligatures,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub restore_state: RestoreState,
    pub scrollback: usize,
    pub padding: u32,
    pub bell: Bell,
    pub copy_on_select: bool,
    pub bold_as_bright: bool,
    pub debug_log: Option<String>,
    /// The shortcut table (`M-H`): chords the app eats before the encoder.
    pub bindings: crate::keymap::Bindings,
    pub fg: Color,
    pub bg: Color,
    pub cursor: Color,
    pub palette: [Color; 16],
}

impl Default for Config {
    fn default() -> Self {
        Config::base()
    }
}

/// The known keys: a key outside this list is an error (with the rest of the
/// file still holding).
const KNOWN: &[&str] = &[
    "shell",
    "font_family",
    "font_size",
    "ligatures",
    "cursor_shape",
    "cursor_blink",
    "restore_state",
    "scrollback",
    "window_padding_width",
    "bell",
    "copy_on_select",
    "bold_as_bright",
    "debug_log",
    "map",
    "unmap",
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

const FONT_MIN: f32 = 4.0;
const FONT_MAX: f32 = 96.0;
const SCROLLBACK_MAX: usize = 100_000;
const PADDING_MAX: u32 = 200;

impl Config {
    /// The defaults of `os` (clean parse of `DEFAULT_CONF` and then its template;
    /// no user file).
    #[must_use]
    pub fn base_for(os: Os) -> Config {
        let (cfg, errs) = Config::parse_text(&Config::fallback(), DEFAULT_CONF);
        debug_assert!(errs.is_empty(), "DEFAULT_CONF has an error: {errs:?}");
        let (cfg, errs) = Config::parse_text(&cfg, os.conf());
        debug_assert!(errs.is_empty(), "{} has an error: {errs:?}", os.name());
        cfg
    }

    /// The defaults of the system this binary was built for.
    #[must_use]
    pub fn base() -> Config {
        Config::base_for(Os::current())
    }

    /// Minimum safety values, used only if `DEFAULT_CONF` is broken (it should
    /// not happen: the test guarantees it).
    fn fallback() -> Config {
        Config {
            shell: None,
            font_family: "monospace".to_string(),
            font_size: 12.0,
            ligatures: Ligatures::Standard,
            cursor_shape: CursorShape::Block,
            cursor_blink: false,
            restore_state: RestoreState::Layout,
            scrollback: 1000,
            padding: 0,
            bell: Bell::None,
            copy_on_select: false,
            bold_as_bright: false,
            debug_log: None,
            bindings: crate::keymap::Bindings::default(),
            fg: Color::Rgb(0xd6, 0xd6, 0xd6),
            bg: Color::Rgb(0, 0, 0),
            cursor: Color::Rgb(0xd6, 0xd6, 0xd6),
            palette: [
                Color::Idx(0),
                Color::Idx(1),
                Color::Idx(2),
                Color::Idx(3),
                Color::Idx(4),
                Color::Idx(5),
                Color::Idx(6),
                Color::Idx(7),
                Color::Idx(8),
                Color::Idx(9),
                Color::Idx(10),
                Color::Idx(11),
                Color::Idx(12),
                Color::Idx(13),
                Color::Idx(14),
                Color::Idx(15),
            ],
        }
    }

    /// Defaults + user text. Returns the errors (empty = everything applied).
    #[must_use]
    pub fn parse(text: &str) -> (Config, Vec<Error>) {
        Config::parse_text(&Config::base(), text)
    }

    /// Like `parse`, but starting from an arbitrary configuration.
    ///
    /// Not the shape of the reload: that one rebuilds from `Config::base()`
    /// (`cli::Options::build`), so a `map` line deleted from the file frees the
    /// chord at once (`B63`-era fix). Stacking on the configuration in force
    /// would keep the old chord alive until the next start.
    #[must_use]
    pub fn over(&self, text: &str) -> (Config, Vec<Error>) {
        Config::parse_text(self, text)
    }

    #[allow(clippy::too_many_lines)] // one arm per key, on purpose
    fn parse_text(base: &Config, text: &str) -> (Config, Vec<Error>) {
        let mut cfg = base.clone();
        let mut errors: Vec<Error> = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = n + 1;
            let without = raw.trim();
            // blank line or whole-line comment
            if without.is_empty() || without.starts_with('#') {
                continue;
            }
            let (key, rest) = match without.split_once(char::is_whitespace) {
                Some((k, v)) => (k, v),
                None => (without, ""),
            };
            let value = value_of(rest);
            let value = value.as_str();
            if !KNOWN.contains(&key) {
                errors.push(Error {
                    line,
                    key: key.to_string(),
                    msg: "unknown key".to_string(),
                });
                continue;
            }
            let bad = |msg: String| Error {
                line,
                key: key.to_string(),
                msg,
            };
            match key {
                "shell" => {
                    cfg.shell = if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    };
                },
                "font_family" => {
                    if value.is_empty() {
                        errors.push(bad("empty family".to_string()));
                    } else {
                        cfg.font_family = value.to_string();
                    }
                },
                "font_size" => match value.parse::<f32>() {
                    Ok(v) if (FONT_MIN..=FONT_MAX).contains(&v) => cfg.font_size = v,
                    Ok(v) => errors.push(bad(format!(
                        "{v} outside the range {FONT_MIN}..{FONT_MAX} points"
                    ))),
                    Err(_) => errors.push(bad(format!("\"{value}\" is not a number"))),
                },
                "ligatures" => match value {
                    "off" => cfg.ligatures = Ligatures::Off,
                    "standard" => cfg.ligatures = Ligatures::Standard,
                    "all" => cfg.ligatures = Ligatures::All,
                    other => {
                        errors.push(bad(format!("\"{other}\" is not off, standard or all")));
                    },
                },
                "cursor_shape" => match value {
                    "block" => cfg.cursor_shape = CursorShape::Block,
                    "beam" => cfg.cursor_shape = CursorShape::Beam,
                    "underline" => cfg.cursor_shape = CursorShape::Underline,
                    other => {
                        errors.push(bad(format!("\"{other}\" is not block, beam or underline")));
                    },
                },
                "cursor_blink" => match parse_bool(value) {
                    Some(b) => cfg.cursor_blink = b,
                    None => errors.push(bad(format!("\"{value}\" is not on/off"))),
                },
                "restore_state" => match value {
                    "none" => cfg.restore_state = RestoreState::None,
                    "layout" => cfg.restore_state = RestoreState::Layout,
                    "content" => cfg.restore_state = RestoreState::Content,
                    other => {
                        errors.push(bad(format!("\"{other}\" is not none, layout or content")));
                    },
                },
                "scrollback" => match value.parse::<usize>() {
                    Ok(v) if v <= SCROLLBACK_MAX => cfg.scrollback = v,
                    Ok(v) => errors.push(bad(format!("{v} above the ceiling {SCROLLBACK_MAX}"))),
                    Err(_) => errors.push(bad(format!("\"{value}\" is not a number"))),
                },
                "window_padding_width" => match value.parse::<u32>() {
                    Ok(v) if v <= PADDING_MAX => cfg.padding = v,
                    Ok(v) => errors.push(bad(format!("{v} above the ceiling {PADDING_MAX} px"))),
                    Err(_) => errors.push(bad(format!("\"{value}\" is not a number"))),
                },
                "bell" => match value {
                    "none" => cfg.bell = Bell::None,
                    "visual" => cfg.bell = Bell::Visual,
                    "audible" => cfg.bell = Bell::Audible,
                    other => {
                        errors.push(bad(format!("\"{other}\" is not none, visual or audible")));
                    },
                },
                "copy_on_select" => match parse_bool(value) {
                    Some(b) => cfg.copy_on_select = b,
                    None => errors.push(bad(format!("\"{value}\" is not on/off"))),
                },
                "bold_as_bright" => match parse_bool(value) {
                    Some(b) => cfg.bold_as_bright = b,
                    None => errors.push(bad(format!("\"{value}\" is not on/off"))),
                },
                "map" => {
                    if let Err(msg) = cfg.bindings.map_text(value) {
                        errors.push(bad(msg));
                    }
                },
                "unmap" => {
                    if let Err(msg) = cfg.bindings.unmap_text(value) {
                        errors.push(bad(msg));
                    }
                },
                "debug_log" => {
                    cfg.debug_log = if value.is_empty() || value == "off" {
                        None
                    } else {
                        Some(value.to_string())
                    };
                },
                "foreground" | "background" | "cursor_color" => match parse_color(value) {
                    Some(c) => match key {
                        "foreground" => cfg.fg = c,
                        "background" => cfg.bg = c,
                        _ => cfg.cursor = c,
                    },
                    None => errors.push(bad(format!("\"{value}\" is not a colour"))),
                },
                _ => {
                    // color0..color15
                    let idx: usize = key[5..].parse().unwrap_or(16);
                    match parse_color(value) {
                        Some(c) if idx < 16 => cfg.palette[idx] = c,
                        Some(_) => errors.push(bad("palette index outside 0..15".to_string())),
                        None => errors.push(bad(format!("\"{value}\" is not a colour"))),
                    }
                },
            }
        }
        (cfg, errors)
    }

    /// The theme (colours) of this configuration.
    #[must_use]
    pub fn theme(&self) -> Theme {
        Theme {
            fg: self.fg,
            bg: self.bg,
            cursor: self.cursor,
            palette: self.palette,
        }
    }

    /// 256-colour palette with the first 16 coming from the theme.
    #[must_use]
    pub fn palette256(&self) -> Vec<(u8, u8, u8)> {
        self.theme().palette256()
    }
}

/// Value of a line, with a trailing comment.
///
/// Rule: a `#` that **opens** the value is a colour (`foreground #e6e6e6`); a
/// `#` after a space, with the value already started, comments to the end
/// (`font_size 12  # note`). It is the only way to have a hex colour and a
/// comment in the same syntax.
fn value_of(rest: &str) -> String {
    let rest = rest.trim();
    if rest.starts_with('#') {
        return rest.split_whitespace().next().unwrap_or("").to_string();
    }
    let bytes = rest.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return rest[..i].trim().to_string();
        }
    }
    rest.to_string()
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" | "1" => Some(true),
        "off" | "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_color(v: &str) -> Option<Color> {
    let hex = v.strip_prefix('#')?;
    let n = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let d = |i: usize| n(&hex[i..=i]).map(|x| x * 17);
            Some(Color::Rgb(d(0)?, d(1)?, d(2)?))
        },
        6 => Some(Color::Rgb(n(&hex[0..2])?, n(&hex[2..4])?, n(&hex[4..6])?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_clean() {
        // the values are the base's and are the same everywhere; the font family
        // is the one line the template owns
        for (os, family) in [
            (Os::Linux, "Monospace"),
            (Os::Macos, "Menlo"),
            (Os::Windows, "Consolas"),
        ] {
            let cfg = Config::base_for(os);
            assert_eq!(cfg.font_family, family, "{}", os.name());
            assert!((cfg.font_size - 12.0).abs() < f32::EPSILON);
            assert_eq!(cfg.scrollback, 10_000);
            assert_eq!(cfg.restore_state, RestoreState::Layout);
            assert_eq!(cfg.bell, Bell::None);
            assert!(!cfg.copy_on_select);
            assert_eq!(cfg.debug_log, None);
            assert_eq!(cfg.shell, None);
            assert_eq!(cfg.palette.len(), 16);
            // the classic xterm palette: mc/htop rely on it (see CONFIG.md)
            assert!(matches!(cfg.palette[1], Color::Rgb(0xcd, 0x00, 0x00)));
            assert_eq!(cfg.bg, Color::Rgb(0, 0, 0));
        }
        let (_, errs) = Config::parse("");
        assert!(errs.is_empty(), "DEFAULT_CONF has an error: {errs:?}");
    }

    /// The `map` chords of a defaults text, in order, as written.
    fn chords(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| line.trim().strip_prefix("map "))
            .filter_map(|rest| rest.split_whitespace().next())
            .map(str::to_string)
            .collect()
    }

    fn assert_no_repeat(list: &[String], who: &str) {
        for (i, chord) in list.iter().enumerate() {
            assert!(!list[..i].contains(chord), "{who} binds '{chord}' twice");
        }
    }

    /// The rule of the four files: a template carries only what the base cannot,
    /// and no file binds a chord twice (a second `map` silently replaces the
    /// first, so the shortcut would vanish without a word).
    #[test]
    fn a_template_only_adds_what_the_base_cannot_carry() {
        let base = chords(DEFAULT_CONF);
        assert_no_repeat(&base, "the base");
        for os in Os::ALL {
            let template = chords(os.conf());
            assert_no_repeat(&template, os.name());
            for chord in &template {
                assert!(
                    !base.contains(chord),
                    "{} repeats the base's '{chord}'",
                    os.name()
                );
            }
            // a template that frees a base line would be a `unmap`, and the split
            // exists so that no template needs one
            assert!(
                !os.conf().lines().any(|l| l.trim().starts_with("unmap ")),
                "{} unmaps a line of the base",
                os.name()
            );
        }
    }

    /// `restore_state` is a ladder — none, layout, content — and `layout` is the
    /// default because it is what the app did before the key existed (`B45`).
    #[test]
    fn restore_state_has_three_degrees_and_layout_is_the_default() {
        assert_eq!(Config::parse("").0.restore_state, RestoreState::Layout);
        let (cfg, errs) = Config::parse("restore_state none");
        assert_eq!(errs.len(), 0, "{errs:?}");
        assert_eq!(cfg.restore_state, RestoreState::None);
        let (cfg, errs) = Config::parse("restore_state content");
        assert_eq!(errs.len(), 0, "{errs:?}");
        assert_eq!(cfg.restore_state, RestoreState::Content);
        // a wrong value costs the line, not the file: the default holds
        let (cfg, errs) = Config::parse("restore_state session\nfont_size 14");
        assert_eq!(cfg.restore_state, RestoreState::Layout);
        assert!((cfg.font_size - 14.0).abs() < f32::EPSILON);
        assert_eq!(errs.len(), 1);
        assert!(
            errs[0].msg.contains("none, layout or content"),
            "{}",
            errs[0]
        );
    }

    #[test]
    fn comment_does_not_swallow_colour() {
        let (cfg, errs) =
            Config::parse("# whole line\nforeground #112233   # note\nbackground #abc\n");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(cfg.fg, Color::Rgb(0x11, 0x22, 0x33));
        // #abc expands as #aabbcc
        assert_eq!(cfg.bg, Color::Rgb(0xaa, 0xbb, 0xcc));
    }

    /// All three `ligatures` values are valid and the default is `standard`
    /// (ligatures on, as in kitty); a wrong value does not bring the file down,
    /// only the line.
    #[test]
    fn ligatures_by_name() {
        assert_eq!(Config::parse("").0.ligatures, Ligatures::Standard);
        for (text, expected) in [
            ("off", Ligatures::Off),
            ("standard", Ligatures::Standard),
            ("all", Ligatures::All),
        ] {
            let (cfg, errs) = Config::parse(&format!("ligatures {text}\n"));
            assert!(errs.is_empty(), "{text}: {errs:?}");
            assert_eq!(cfg.ligatures, expected, "{text}");
        }
        let (cfg, errs) = Config::parse("ligatures maybe\nfont_size 14\n");
        assert_eq!(
            cfg.ligatures,
            Ligatures::Standard,
            "a bad line keeps the default"
        );
        assert!(
            (cfg.font_size - 14.0).abs() < f32::EPSILON,
            "the rest of the file holds"
        );
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].key, "ligatures");
    }

    #[test]
    fn repeated_key_last_wins() {
        let (cfg, _) = Config::parse("font_size 20\nfont_size 14\n");
        assert!((cfg.font_size - 14.0).abs() < f32::EPSILON);
    }

    #[test]
    fn unknown_key_is_error_and_rest_holds() {
        let (cfg, errs) = Config::parse("bogus 1\nfont_size 20\n");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].line, 1);
        assert_eq!(errs[0].key, "bogus");
        assert!((cfg.font_size - 20.0).abs() < f32::EPSILON, "the rest held");
    }

    #[test]
    fn invalid_value_keeps_default() {
        let (cfg, errs) = Config::parse("font_size abc\nscrollback -3\nbell beep\n");
        assert_eq!(errs.len(), 3);
        assert!((cfg.font_size - 12.0).abs() < f32::EPSILON);
        assert_eq!(cfg.scrollback, 10_000);
        assert_eq!(cfg.bell, Bell::None);
        assert!(errs[0].to_string().starts_with("line 1: 'font_size'"));
    }

    #[test]
    fn ranges_have_ceiling() {
        let (_, errs) =
            Config::parse("font_size 200\nscrollback 999999\nwindow_padding_width 900\n");
        assert_eq!(errs.len(), 3);
        let (cfg, _) = Config::parse("window_padding_width 6\n");
        assert_eq!(cfg.padding, 6);
    }

    #[test]
    fn empty_shell_is_none_and_debug_log_off_disables() {
        let (cfg, _) = Config::parse("shell del\n");
        assert_eq!(cfg.shell.as_deref(), Some("del"));
        let (cfg, _) = Config::parse("shell\ndebug_log off\n");
        assert_eq!(cfg.shell, None);
        assert_eq!(cfg.debug_log, None);
    }

    #[test]
    fn theme_palette_arrives_in_the_256() {
        let (cfg, errs) = Config::parse("color1 #ff0000\ncolor8 #00ff00\n");
        assert_eq!(errs, [], "unexpected errors");
        let p = cfg.palette256();
        assert_eq!(p[1], (0xff, 0, 0));
        assert_eq!(p[8], (0, 0xff, 0));
        // the rest of the 6x6x6 cube is still intact
        assert_eq!(p[196], (255, 0, 0));
        assert_eq!(p.len(), 256);
    }

    #[test]
    fn reload_with_error_swaps_nothing() {
        let (good, _) = Config::parse("font_size 16\n");
        let (attempt, errs) = good.over("font_size 18\nbell beep\n");
        assert_ne!(errs, [], "the error had to show up");
        // the caller only applies if `errs` is empty; here we prove the base is
        // not mutated and the good value survives
        assert!((good.font_size - 16.0).abs() < f32::EPSILON);
        assert!((attempt.font_size - 18.0).abs() < f32::EPSILON);
    }

    #[test]
    fn config_doc_mentions_all_keys() {
        // the doc cannot age on its own: every key of the defaults has to appear
        // in docs/CONFIG.md
        let doc =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CONFIG.md"))
                .expect("docs/CONFIG.md");
        for k in KNOWN {
            // the palette appears as a range in the doc (`color0` … `color15`)
            let interval =
                k.starts_with("color") && doc.contains("color0") && doc.contains("color15");
            assert!(
                doc.contains(k) || interval,
                "docs/CONFIG.md does not mention '{k}'"
            );
        }
    }
}
