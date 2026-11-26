//! Remappable shortcuts (`M-H`, config v2): one table, `map`/`unmap` in the
//! config file.
//!
//! The machinery lives here; the **defaults** are `map` lines in `DEFAULT_CONF`,
//! like every other default (the palette, the font, the history cap). One table,
//! one place: an empty `Bindings` plus that text is the app's shortcut set, and a
//! test proves it without a window.
//!
//! A shortcut is a **chord** (one key plus modifiers) and an **action** (what the
//! app does). `map ctrl+shift+t new_tab` adds or replaces the chord;
//! `unmap ctrl+shift+w` frees it, and the child sees the key again — that is the
//! point: a key the app does not want must reach the program.

/// No modifiers.
pub const NONE: u8 = 0;
pub const CTRL: u8 = 1;
pub const ALT: u8 = 2;
pub const SHIFT: u8 = 4;
pub const SUPER: u8 = 8;

/// The key of a chord: a character (already lowercased) or a named key, spelled
/// as in the config file (`up`, `f5`, `pgup`).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Id {
    Char(char),
    Named(&'static str),
}

/// The named keys a chord may use. It is the same vocabulary `Chord::parse`
/// accepts, so what `KeyIn` produces always finds its way back into text.
pub const NAMED: &[&str] = &[
    "enter",
    "esc",
    "tab",
    "backspace",
    "space",
    "insert",
    "delete",
    "home",
    "end",
    "pgup",
    "pgdn",
    "up",
    "down",
    "left",
    "right",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
    "f13",
    "f14",
    "f15",
    "f16",
    "f17",
    "f18",
    "f19",
    "f20",
    "f21",
    "f22",
    "f23",
    "f24",
];

/// One key combination.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Chord {
    pub id: Id,
    pub mods: u8,
}

impl Chord {
    /// `ctrl+shift+t`, `alt+1`, `shift+pgup`, `f2`.
    ///
    /// The modifiers may come in any order and are joined with `+`; the key is
    /// the last part. An unknown modifier or key name returns `None` (the caller
    /// turns that into a per-line config error).
    #[must_use]
    pub fn parse(text: &str) -> Option<Chord> {
        let parts: Vec<&str> = text.split('+').filter(|p| !p.is_empty()).collect();
        let (key, mods) = parts.split_last()?;
        let mut bits = NONE;
        for m in mods {
            bits |= match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => CTRL,
                "alt" | "meta" => ALT,
                "shift" => SHIFT,
                "super" | "logo" => SUPER,
                _ => return None,
            };
        }
        let id = match key.to_ascii_lowercase().as_str() {
            name if NAMED.contains(&name) => {
                Id::Named(NAMED.iter().find(|n| **n == name).copied()?)
            },
            // a single character, lowercased: the config says `T` and the key is
            // the same key
            c if c.chars().count() == 1 => Id::Char(c.chars().next()?),
            _ => return None,
        };
        Some(Chord { id, mods: bits })
    }

    /// The chord as the config writes it (for the error message).
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (bit, name) in [
            (CTRL, "ctrl"),
            (ALT, "alt"),
            (SHIFT, "shift"),
            (SUPER, "super"),
        ] {
            if self.mods & bit != 0 {
                out.push_str(name);
                out.push('+');
            }
        }
        match self.id {
            Id::Char(c) => out.push(c),
            Id::Named(n) => out.push_str(n),
        }
        out
    }

    /// Exact match: the character (lowercased, so `T` and `t` are the same key)
    /// and the modifier bits. `shift+t` is not `t`, which is what keeps a
    /// `ctrl+shift+t` binding from eating `ctrl+t`.
    #[must_use]
    pub fn same(&self, other: Chord) -> bool {
        self.id == other.id && self.mods == other.mods
    }
}

/// What a chord does. The argument (a tab or workspace number) travels with the
/// action, which is what lets `alt+1` and `ctrl+shift+1` be the same action on
/// different domains.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Action {
    Copy,
    Paste,
    NewTab,
    CloseTab,
    NewWorkspace,
    CloseWorkspace,
    /// Rename the workspace in front. No default chord on purpose (`B70`): a
    /// factory key here is a key the child program may be owed (`F2` is `mc`'s
    /// user menu).
    WsRename,
    /// Walk the prompt marks (`M-R`).
    PrevPrompt,
    NextPrompt,
    OpenConfig,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    /// Open the search bar over the scrollback (`M-D`).
    Search,
    /// 1..=10, where 10 is the tenth (`alt+0`).
    SelectTab(u8),
    SelectWorkspace(u8),
    WsPrev,
    WsNext,
    WsMoveUp,
    WsMoveDown,
}

impl Action {
    /// Every action, with the name the config uses. Ordered as the docs show it.
    pub const ALL: &'static [(&'static str, Action)] = &[
        ("copy", Action::Copy),
        ("paste", Action::Paste),
        ("new_tab", Action::NewTab),
        ("close_tab", Action::CloseTab),
        ("new_workspace", Action::NewWorkspace),
        ("close_workspace", Action::CloseWorkspace),
        ("ws_rename", Action::WsRename),
        ("prev_prompt", Action::PrevPrompt),
        ("next_prompt", Action::NextPrompt),
        ("open_config", Action::OpenConfig),
        ("scroll_page_up", Action::ScrollPageUp),
        ("scroll_page_down", Action::ScrollPageDown),
        ("scroll_top", Action::ScrollTop),
        ("scroll_bottom", Action::ScrollBottom),
        ("search", Action::Search),
        ("select_tab", Action::SelectTab(0)),
        ("select_workspace", Action::SelectWorkspace(0)),
        ("ws_prev", Action::WsPrev),
        ("ws_next", Action::WsNext),
        ("ws_move_up", Action::WsMoveUp),
        ("ws_move_down", Action::WsMoveDown),
    ];

    /// `name` plus an optional `arg`, as the value of a `map` line.
    #[must_use]
    pub fn parse(text: &str) -> Option<Action> {
        let mut it = text.split_whitespace();
        let name = it.next()?;
        let arg = it.next();
        if it.next().is_some() {
            return None;
        }
        let base = Action::ALL
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, a)| *a)?;
        match base {
            Action::SelectTab(_) | Action::SelectWorkspace(_) => {
                // the number is part of the action: `map alt+3 select_tab 3`
                let n: u8 = arg?.parse().ok()?;
                if !(1..=10).contains(&n) {
                    return None;
                }
                Some(match base {
                    Action::SelectTab(_) => Action::SelectTab(n),
                    _ => Action::SelectWorkspace(n),
                })
            },
            // `select_tab` without a number is only a template in `ALL`
            _ if arg.is_some() => None,
            other => Some(other),
        }
    }

    /// The name (`select_tab` for the numbered ones: the number is the argument).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Action::SelectTab(_) => "select_tab",
            Action::SelectWorkspace(_) => "select_workspace",
            other => Action::ALL
                .iter()
                .find(|(_, a)| *a == other)
                .map_or("?", |(n, _)| n),
        }
    }
}

/// The table: chord → action.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Bindings {
    list: Vec<(Chord, Action)>,
}

impl Bindings {
    /// Adds or replaces one chord.
    fn set(&mut self, chord: Chord, action: Action) {
        match self.list.iter_mut().find(|(c, _)| c.same(chord)) {
            Some(slot) => slot.1 = action,
            None => self.list.push((chord, action)),
        }
    }

    /// Frees a chord: the app stops eating it.
    pub fn unset(&mut self, chord: Chord) {
        self.list.retain(|(c, _)| !c.same(chord));
    }

    #[must_use]
    pub fn get(&self, chord: Chord) -> Option<Action> {
        self.list
            .iter()
            .find(|(c, _)| c.same(chord))
            .map(|(_, a)| *a)
    }

    /// `chord action [arg]`, the value of a `map` line. `None` = the line is
    /// wrong, and the error says which part.
    pub fn map_text(&mut self, text: &str) -> Result<(), String> {
        let mut it = text.splitn(2, char::is_whitespace);
        let chord = it.next().unwrap_or("");
        let rest = it.next().unwrap_or("").trim();
        let chord = Chord::parse(chord).ok_or_else(|| format!("\"{chord}\" is not a chord"))?;
        if rest.is_empty() {
            return Err("no action after the chord".to_string());
        }
        let action = Action::parse(rest).ok_or_else(|| format!("\"{rest}\" is not an action"))?;
        self.set(chord, action);
        Ok(())
    }

    /// The value of an `unmap` line.
    pub fn unmap_text(&mut self, text: &str) -> Result<(), String> {
        let chord = text.trim();
        let c = Chord::parse(chord).ok_or_else(|| format!("\"{chord}\" is not a chord"))?;
        self.unset(c);
        Ok(())
    }

    /// Chords in the order they were added (for the docs test and probes).
    #[must_use]
    pub fn list(&self) -> &[(Chord, Action)] {
        &self.list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chords_of_the_documentation_parse() {
        for text in [
            "ctrl+shift+t",
            "alt+1",
            "shift+pgup",
            "f2",
            "super+shift+up",
        ] {
            assert!(Chord::parse(text).is_some(), "{text}");
        }
        for text in ["ctrl+shift", "hyper+t", ""] {
            assert!(Chord::parse(text).is_none(), "{text}");
        }
        // a modifier repeated is harmless: it is the same bit
        assert_eq!(
            Chord::parse("ctrl+ctrl+t").unwrap().mods,
            Chord::parse("ctrl+t").unwrap().mods
        );
        assert_eq!(Chord::parse("ctrl+shift+t").unwrap().text(), "ctrl+shift+t");
        assert_eq!(Chord::parse("T").unwrap().id, Id::Char('t'));
    }

    use crate::config::{Config, Os};

    fn chord(text: &str) -> Chord {
        Chord::parse(text).expect(text)
    }

    /// The defaults are `DEFAULT_CONF` plus the system's template, so the tests
    /// read them from there.
    fn base() -> Bindings {
        Config::base().bindings
    }

    fn base_of(os: Os) -> Bindings {
        Config::base_for(os).bindings
    }

    #[test]
    fn the_linux_table_keeps_every_shortcut_the_app_had() {
        let b = base_of(Os::Linux);
        let cases = [
            ("ctrl+shift+c", Action::Copy),
            ("ctrl+shift+v", Action::Paste),
            ("ctrl+shift+t", Action::NewTab),
            ("ctrl+shift+w", Action::CloseTab),
            ("ctrl+shift+n", Action::NewWorkspace),
            ("ctrl+shift+q", Action::CloseWorkspace),
            ("ctrl+shift+f2", Action::OpenConfig),
            ("ctrl+shift+up", Action::PrevPrompt),
            ("ctrl+shift+down", Action::NextPrompt),
            ("shift+pgup", Action::ScrollPageUp),
            ("shift+home", Action::ScrollTop),
            ("shift+end", Action::ScrollBottom),
            ("alt+2", Action::SelectTab(2)),
            ("alt+0", Action::SelectTab(10)),
            ("ctrl+shift+3", Action::SelectWorkspace(3)),
            ("super+9", Action::SelectWorkspace(9)),
            ("super+0", Action::SelectWorkspace(10)),
            ("super+up", Action::WsPrev),
            ("super+shift+down", Action::WsMoveDown),
        ];
        for (text, want) in cases {
            let chord = Chord::parse(text).expect(text);
            assert_eq!(b.get(chord), Some(want), "{text}");
        }
        // a key nobody bound reaches the program
        assert_eq!(b.get(Chord::parse("a").unwrap()), None);
        assert_eq!(b.get(Chord::parse("ctrl+l").unwrap()), None);
    }

    /// A template has to answer for every action: an `Action` that only lands on
    /// one system is a shortcut missing on the other two. The one exception is
    /// deliberate (`B70`): the rename has no factory chord, because a chord here
    /// is a chord the child program may be owed — and the same test swears no
    /// default ever binds it.
    #[test]
    fn every_system_has_a_chord_for_every_action() {
        for os in Os::ALL {
            let bound: Vec<Action> = base_of(os).list().iter().map(|(_, a)| *a).collect();
            for (name, action) in Action::ALL {
                if matches!(action, Action::WsRename) {
                    assert!(
                        !bound
                            .iter()
                            .any(|b| std::mem::discriminant(b) == std::mem::discriminant(action)),
                        "{}: {name} was meant to have no default chord",
                        os.name()
                    );
                    continue;
                }
                assert!(
                    bound
                        .iter()
                        .any(|b| std::mem::discriminant(b) == std::mem::discriminant(action)),
                    "{}: no chord for {name}",
                    os.name()
                );
            }
        }
    }

    /// On macOS `super` IS the Command key, so the tabs are where a mac user's
    /// hand already is — and Option+digit is left to the child, which composes
    /// `¡` with it.
    #[test]
    fn the_macos_template_puts_the_tabs_on_command() {
        let b = base_of(Os::Macos);
        assert_eq!(b.get(chord("super+1")), Some(Action::SelectTab(1)));
        assert_eq!(b.get(chord("super+t")), Some(Action::NewTab));
        assert_eq!(b.get(chord("super+v")), Some(Action::Paste));
        assert_eq!(
            b.get(chord("ctrl+shift+1")),
            Some(Action::SelectWorkspace(1))
        );
        assert_eq!(b.get(chord("super+alt+up")), Some(Action::WsPrev));
        assert_eq!(b.get(chord("alt+1")), None, "Option+1 belongs to the child");
    }

    /// Win+1 launches the taskbar and Win+arrow snaps, so Windows gets no `super`
    /// family at all: the tabs are on Alt and the walk is on Alt too.
    #[test]
    fn the_windows_template_has_no_super_key() {
        let b = base_of(Os::Windows);
        assert_eq!(b.get(chord("alt+1")), Some(Action::SelectTab(1)));
        assert_eq!(b.get(chord("ctrl+shift+t")), Some(Action::NewTab));
        assert_eq!(
            b.get(chord("ctrl+shift+0")),
            Some(Action::SelectWorkspace(10))
        );
        assert_eq!(b.get(chord("alt+up")), Some(Action::WsPrev));
        for text in ["super+1", "super+t", "super+up", "super+shift+up"] {
            assert_eq!(b.get(chord(text)), None, "{text}");
        }
    }

    #[test]
    fn map_replaces_a_chord_and_unmap_frees_it() {
        let mut b = base();
        b.map_text("ctrl+shift+t new_workspace").unwrap();
        let chord = Chord::parse("ctrl+shift+t").unwrap();
        assert_eq!(b.get(chord), Some(Action::NewWorkspace));
        // one entry per chord: replacing does not duplicate
        assert_eq!(
            b.list().iter().filter(|(c, _)| *c == chord).count(),
            1,
            "the chord is bound once"
        );
        b.unmap_text("ctrl+shift+t").unwrap();
        assert_eq!(b.get(chord), None);
        // and the freed key is not replaced by anything
        assert_eq!(
            b.get(Chord::parse("ctrl+shift+w").unwrap()),
            Some(Action::CloseTab)
        );
    }

    #[test]
    fn a_bad_map_line_says_what_is_wrong() {
        let mut b = base();
        assert!(b.map_text("ctrl+shift+y new_tab").is_ok());
        assert!(b.map_text("ctrl+shift+t").is_err(), "no action");
        assert!(
            b.map_text("ctrl+shift+t explode").is_err(),
            "unknown action"
        );
        assert!(b.map_text("hyper+t new_tab").is_err(), "unknown modifier");
        assert!(b.map_text("alt+5 select_tab 11").is_err(), "out of range");
        assert!(b.map_text("alt+5 select_tab").is_err(), "number required");
        assert!(b.map_text("alt+5 copy 3").is_err(), "no argument");
    }

    #[test]
    fn the_case_of_the_letter_does_not_matter_but_the_shift_bit_does() {
        let upper = Chord {
            id: Id::Char('t'),
            mods: SHIFT,
        };
        let plain = Chord {
            id: Id::Char('t'),
            mods: NONE,
        };
        assert!(!upper.same(plain), "shift is a modifier, not a letter");
        // `T` in the config is the same key as `t`
        assert_eq!(Chord::parse("T").unwrap().id, Chord::parse("t").unwrap().id);
        assert!(!Chord::parse("ctrl+t")
            .unwrap()
            .same(Chord::parse("ctrl+shift+t").unwrap()));
    }

    #[test]
    fn every_action_has_a_name_and_round_trips_through_text() {
        for (name, action) in Action::ALL {
            assert_eq!(action.name(), *name);
            let text = match action {
                Action::SelectTab(_) => format!("{name} 4"),
                Action::SelectWorkspace(_) => format!("{name} 10"),
                _ => (*name).to_string(),
            };
            assert!(Action::parse(&text).is_some(), "{text}");
        }
    }
}
