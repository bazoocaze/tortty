//! Keyboard: winit event -> bytes the terminal sends to the child.
//!
//! The base table: arrows in application mode (DECCKM `?1`), modifiers in the
//! xterm format, Alt as an ESC prefix, Shift+Tab `\e[Z` and Ctrl+letter in
//! 0x01..0x1a.
//!
//! On top of that, the kitty keyboard protocol: when the child
//! turns flags on with `CSI > u`, keys go out as `CSI code ; mods u` instead of
//! the control byte. The flags live in `Term` (one per tab) and the app decides
//! — only the encoding lives here.

use std::fmt::Write as _;

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

/// 1 + shift(1) + alt(2) + ctrl(4) — the xterm modifier parameter.
fn mod_param(ctrl: bool, alt: bool, shift: bool) -> u8 {
    1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl)
}

fn named_base(n: NamedKey, app_cursor: bool) -> Option<&'static str> {
    use NamedKey::{
        ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Backspace, Delete, End, Enter, Escape, Home,
        Insert, PageDown, PageUp, Space, Tab, F1, F10, F11, F12, F2, F3, F4, F5, F6, F7, F8, F9,
    };
    let s = match n {
        Enter => "\r",
        Backspace => "\x7f",
        Tab => "\t",
        Escape => "\x1b",
        Space => " ",
        ArrowUp => {
            if app_cursor {
                "\x1bOA"
            } else {
                "\x1b[A"
            }
        },
        ArrowDown => {
            if app_cursor {
                "\x1bOB"
            } else {
                "\x1b[B"
            }
        },
        ArrowRight => {
            if app_cursor {
                "\x1bOC"
            } else {
                "\x1b[C"
            }
        },
        ArrowLeft => {
            if app_cursor {
                "\x1bOD"
            } else {
                "\x1b[D"
            }
        },
        Home => {
            if app_cursor {
                "\x1bOH"
            } else {
                "\x1b[H"
            }
        },
        End => {
            if app_cursor {
                "\x1bOF"
            } else {
                "\x1b[F"
            }
        },
        Insert => "\x1b[2~",
        Delete => "\x1b[3~",
        PageUp => "\x1b[5~",
        PageDown => "\x1b[6~",
        F1 => "\x1bOP",
        F2 => "\x1bOQ",
        F3 => "\x1bOR",
        F4 => "\x1bOS",
        F5 => "\x1b[15~",
        F6 => "\x1b[17~",
        F7 => "\x1b[18~",
        F8 => "\x1b[19~",
        F9 => "\x1b[20~",
        F10 => "\x1b[21~",
        F11 => "\x1b[23~",
        F12 => "\x1b[24~",
        _ => return None,
    };
    Some(s)
}

/// F13..F24 are, on a PC keyboard, F1..F12 with Shift held: xterm's terminfo
/// says so (`kf13=\E[1;2P`), so the base sequence is the F1 one and the Shift is
/// part of the key, not of the event.
fn shifted_fkey(n: NamedKey) -> Option<NamedKey> {
    use NamedKey::{
        F1, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F2, F20, F21, F22, F23, F24, F3, F4,
        F5, F6, F7, F8, F9,
    };
    Some(match n {
        F13 => F1,
        F14 => F2,
        F15 => F3,
        F16 => F4,
        F17 => F5,
        F18 => F6,
        F19 => F7,
        F20 => F8,
        F21 => F9,
        F22 => F10,
        F23 => F11,
        F24 => F12,
        _ => return None,
    })
}

/// The character a keypad key stands for in numeric mode.
///
/// winit leaves `KP_0`..`KP_9`, `KP_Decimal` and the operators out of its keysym
/// table (they are commented out in `keymap.rs`), so with NumLock on they arrive
/// as `Key::Unidentified` and the keypad was mute. The physical key is what says
/// which one it was. With NumLock off the keysym is a navigation one (`KP_Home`)
/// and winit **does** map it, so the logical key wins and the keypad keeps
/// acting as the arrow cluster.
fn numpad_char(p: PhysicalKey) -> Option<&'static str> {
    let PhysicalKey::Code(c) = p else {
        return None;
    };
    Some(match c {
        KeyCode::Numpad0 => "0",
        KeyCode::Numpad1 => "1",
        KeyCode::Numpad2 => "2",
        KeyCode::Numpad3 => "3",
        KeyCode::Numpad4 => "4",
        KeyCode::Numpad5 => "5",
        KeyCode::Numpad6 => "6",
        KeyCode::Numpad7 => "7",
        KeyCode::Numpad8 => "8",
        KeyCode::Numpad9 => "9",
        KeyCode::NumpadDecimal => ".",
        KeyCode::NumpadComma => ",",
        KeyCode::NumpadDivide => "/",
        KeyCode::NumpadMultiply => "*",
        KeyCode::NumpadSubtract => "-",
        KeyCode::NumpadAdd => "+",
        KeyCode::NumpadEqual => "=",
        _ => return None,
    })
}

/// The key the tables see: the logical one or, for the keypad with NumLock on,
/// the character its physical key stands for.
fn resolve_key(logical: &Key, physical: PhysicalKey) -> (Option<NamedKey>, Option<&str>) {
    match logical {
        Key::Named(n) => (Some(*n), None),
        Key::Character(s) => (None, Some(s.as_str())),
        _ => (None, numpad_char(physical)),
    }
}

/// Applies modifiers in the xterm format.
fn with_mods(seq: &str, ctrl: bool, alt: bool, shift: bool) -> Vec<u8> {
    // Shift+Tab arrives as `\t` with shift: the canonical form is backtab
    if seq == "\t" && shift {
        // Shift+Tab is backtab; the shift is already in the canonical form
        if ctrl || alt {
            return format!("\x1b[1;{}Z", mod_param(ctrl, alt, false)).into_bytes();
        }
        return b"\x1b[Z".to_vec();
    }
    if !(ctrl || alt || shift) {
        return seq.as_bytes().to_vec();
    }
    let m = mod_param(ctrl, alt, shift);
    if let Some(body) = seq.strip_prefix("\x1b[") {
        if let Some(num) = body.strip_suffix('~') {
            return format!("\x1b[{num};{m}~").into_bytes();
        }
        return format!("\x1b[1;{m}{body}").into_bytes();
    }
    if let Some(body) = seq.strip_prefix("\x1bO") {
        return format!("\x1b[1;{m}{body}").into_bytes();
    }
    if alt {
        let mut v = Vec::with_capacity(seq.len() + 1);
        v.push(0x1b);
        v.extend_from_slice(seq.as_bytes());
        return v;
    }
    seq.as_bytes().to_vec()
}

// ------------------------------------------------------ kitty protocol

/// Flags of `CSI > u` (kitty bits).
pub const F_DISAMBIGUATE: u32 = 1;
pub const F_EVENT_TYPES: u32 = 2;
/// 4: the alternate keys (`97:65:97`) come along.
pub const F_ALT_KEYS: u32 = 4;
pub const F_ALL_KEYS: u32 = 8;
pub const F_TEXT: u32 = 16;

const PRESS: u8 = 1;
const REPEAT: u8 = 2;
const RELEASE: u8 = 3;

/// A named key in the kitty shape: (CSI number, trailer).
///
/// The trailer is what changes between `CSI 27;5 u` (Escape with Ctrl) and
/// `CSI 1;5 A` (up arrow with Ctrl). The table of the protocol applies — F3 is
/// `1 R` (`SS3 R` with a modifier), not `13 ~`.
fn kitty_functional(n: NamedKey) -> Option<(u32, char)> {
    use NamedKey::{
        ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Backspace, Delete, End, Enter, Escape, Home,
        Insert, PageDown, PageUp, Space, Tab, F1, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19,
        F2, F20, F21, F22, F23, F24, F3, F4, F5, F6, F7, F8, F9,
    };
    Some(match n {
        Escape => (27, 'u'),
        Enter => (13, 'u'),
        Tab => (9, 'u'),
        Backspace => (127, 'u'),
        // Space is not "functional" in the legacy path: it only becomes `CSI u`
        // when the app asked for disambiguation (flag 1) or all keys (flag 8) —
        // see the guard in `kitty_encode`
        Space => (32, 'u'),
        Insert => (2, '~'),
        Delete => (3, '~'),
        ArrowLeft => (1, 'D'),
        ArrowRight => (1, 'C'),
        ArrowUp => (1, 'A'),
        ArrowDown => (1, 'B'),
        PageUp => (5, '~'),
        PageDown => (6, '~'),
        Home => (1, 'H'),
        End => (1, 'F'),
        F1 => (1, 'P'),
        F2 => (1, 'Q'),
        F3 => (1, 'R'),
        F4 => (1, 'S'),
        F5 => (15, '~'),
        F6 => (17, '~'),
        F7 => (18, '~'),
        F8 => (19, '~'),
        F9 => (20, '~'),
        F10 => (21, '~'),
        F11 => (23, '~'),
        F12 => (24, '~'),
        // F13..F24 have no legacy form: the kitty table gives them their own
        // numbers (GLFW's `GLFW_FKEY_F13`..), which is why they only travel as
        // `CSI <number> u`
        F13 => (57376, 'u'),
        F14 => (57377, 'u'),
        F15 => (57378, 'u'),
        F16 => (57379, 'u'),
        F17 => (57380, 'u'),
        F18 => (57381, 'u'),
        F19 => (57382, 'u'),
        F20 => (57383, 'u'),
        F21 => (57384, 'u'),
        F22 => (57385, 'u'),
        F23 => (57386, 'u'),
        F24 => (57387, 'u'),
        _ => return None,
    })
}

/// `CSI key ; mods[:event] [;text] trailer`, with no field the terminal does not
/// need to see.
fn kitty_csi(field: &str, mods: u8, trailer: char, event: Option<u8>, text: &str) -> Vec<u8> {
    let event = event.filter(|e| *e != PRESS);
    let mut out = String::from("\x1b[");
    if field != "1" || mods != 1 || event.is_some() || !text.is_empty() {
        out.push_str(field);
    }
    if mods != 1 || event.is_some() {
        out.push(';');
        if mods == 1 {
            out.push('1'); // without a modifier the field still exists
        } else {
            out.push_str(&mods.to_string());
        }
    }
    if let Some(e) = event {
        let _ = write!(out, ":{e}");
    }
    if !text.is_empty() {
        let cps: Vec<String> = text.chars().map(|c| (c as u32).to_string()).collect();
        out.push(';');
        out.push_str(&cps.join(":"));
    }
    out.push(trailer);
    out.into_bytes()
}

/// What the kitty protocol answered for this key.
enum Kitty {
    /// Goes out as `CSI ... u` (or a variant).
    Seq(Vec<u8>),
    /// Not covered: falls back to the legacy xterm path.
    Legacy,
}

#[allow(clippy::too_many_arguments)]
fn kitty_encode(
    named: Option<NamedKey>,
    character: Option<&str>,
    text: &str,
    mods: ModifiersState,
    flags: u32,
    event: u8,
) -> Kitty {
    let dis = flags & F_DISAMBIGUATE != 0;
    let ev_types = flags & F_EVENT_TYPES != 0;
    let all_keys = flags & F_ALL_KEYS != 0;
    let embed = flags & F_TEXT != 0;

    // without flag 2 there is no release: the legacy path only knows the press
    if event == RELEASE && !ev_types {
        return Kitty::Legacy;
    }
    let m = 1
        + u8::from(mods.shift_key())
        + 2 * u8::from(mods.alt_key())
        + 4 * u8::from(mods.control_key())
        + 8 * u8::from(mods.super_key());
    let text = if embed { text } else { "" };
    let ev = ev_types.then_some(event);

    if let Some(n) = named {
        let Some((key, trailer)) = kitty_functional(n) else {
            return Kitty::Legacy;
        };
        let special = matches!(
            n,
            NamedKey::Escape | NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace
        );
        // with only flag 2 (events), Ctrl+Space has to stay NUL: that is what
        // readline/vim expect, and swapping it for `CSI 32;5u` would break
        // whoever turns on flag 2 and nothing else
        if n == NamedKey::Space && !(dis || all_keys) {
            return Kitty::Legacy;
        }
        if !special && m == 1 && !all_keys {
            // arrow/Home/F1 without a modifier: the legacy path already sends
            // the right thing (and respects DECCKM)
            return Kitty::Legacy;
        }
        if special && m == 1 && !(dis || all_keys) {
            return Kitty::Legacy; // classic ESC, CR, TAB, DEL
        }
        return Kitty::Seq(kitty_csi(&key.to_string(), m, trailer, ev, text));
    }

    let Some(cp) = character.and_then(|s| s.chars().next()) else {
        return Kitty::Legacy; // key with no character (modifier, dead key)
    };
    let mut cp = cp as u32;
    // Ctrl+Shift+a reports the base key, not the uppercase character
    if mods.control_key() && (0x41..=0x5a).contains(&cp) {
        cp += 32;
    }
    if m == 2 && !all_keys {
        return Kitty::Legacy; // Shift+letter is still text
    }
    if m == 1 && !all_keys {
        return Kitty::Legacy; // plain text
    }
    if !(dis || all_keys) {
        return Kitty::Legacy; // modifier without disambiguate: classic xterm
    }
    // Flag 4 (`97:65:97`): the key as it is on the board, then the shifted one,
    // then the base-layout one — so Shift+a stops looking like a key named `A`.
    // Only the ASCII letters carry a known pair: the digit row would need the
    // layout (`!` is `1` on a US board and something else elsewhere), and a
    // wrong guess is worse than an absent field.
    let letters = (0x41..=0x5a).contains(&cp);
    let field = if flags & F_ALT_KEYS != 0 && mods.shift_key() && letters {
        let base = cp + 32;
        format!("{base}:{cp}:{base}")
    } else {
        cp.to_string()
    };
    Kitty::Seq(kitty_csi(&field, m, 'u', ev, text))
}

/// Translates the event. `None` = send nothing (modifier, WM Super, etc.).
pub fn encode(
    ev: &KeyEvent,
    mods: ModifiersState,
    app_cursor: bool,
    kbd_flags: u32,
) -> Option<Vec<u8>> {
    let event = if ev.state == ElementState::Released {
        RELEASE
    } else if ev.repeat {
        REPEAT
    } else {
        PRESS
    };
    // release only matters to the kitty protocol (the legacy path never reports
    // the key going up)
    if ev.state != ElementState::Pressed && kbd_flags & F_EVENT_TYPES == 0 {
        return None;
    }
    let (named, character) = resolve_key(&ev.logical_key, ev.physical_key);
    encode_event(
        named,
        character,
        ev.text.as_deref(),
        mods,
        app_cursor,
        kbd_flags,
        event,
    )
}

/// The body of `encode` without the `KeyEvent`: it is what the tests use for
/// release and repeat, which are a protocol decision and not a winit one.
pub fn encode_event(
    named: Option<NamedKey>,
    character: Option<&str>,
    text: Option<&str>,
    mods: ModifiersState,
    app_cursor: bool,
    kbd_flags: u32,
    event: u8,
) -> Option<Vec<u8>> {
    if kbd_flags != 0 {
        match kitty_encode(named, character, text.unwrap_or(""), mods, kbd_flags, event) {
            Kitty::Seq(v) => return Some(v),
            Kitty::Legacy => {},
        }
    }
    // release with no kitty shape: nothing goes out (E19 — otherwise `pi`,
    // which turns on flag 2, got the text twice: the legacy path does not know
    // the key went up and sent the text again)
    if event == RELEASE {
        return None;
    }
    encode_parts(named, character, text, mods, app_cursor, kbd_flags)
}

/// The translation itself, without depending on a `KeyEvent`: it is what the
/// `keys` verb of the debug socket uses, to exercise the **same** path as the
/// real keyboard (a parallel encoder in the socket diverged once: F5 became
/// 1 byte).
pub fn encode_parts(
    named: Option<NamedKey>,
    character: Option<&str>,
    text: Option<&str>,
    mods: ModifiersState,
    app_cursor: bool,
    kbd_flags: u32,
) -> Option<Vec<u8>> {
    if kbd_flags != 0 {
        match kitty_encode(named, character, text.unwrap_or(""), mods, kbd_flags, PRESS) {
            Kitty::Seq(v) => return Some(v),
            Kitty::Legacy => {},
        }
    }
    let ctrl = mods.control_key();
    let alt = mods.alt_key();
    let shift = mods.shift_key();
    // window manager shortcut: not the terminal's business
    if mods.super_key() {
        return None;
    }

    if let Some(n) = named {
        // Ctrl+Space is NUL (it is what readline/vim expect to "mark" a
        // selection), not the `CSI 1;5 ` the generic table would produce
        if ctrl && n == NamedKey::Space {
            return Some(if alt { vec![0x1b, 0x00] } else { vec![0x00] });
        }
        let (n, key_shift) = match shifted_fkey(n) {
            Some(base) => (base, true),
            None => (n, false),
        };
        let base = named_base(n, app_cursor)?;
        // a dead key resolved over a named key: `xkbcommon` composes into `text`
        // and keeps the logical key as the base, so `~`+space arrives as
        // `Named(Space)` with text `~` and the table would send the bare space
        // (B36). Control characters are not composition: `Backspace` brings
        // `\x08` and has to keep going out as `DEL`.
        if let Some(t) = text.filter(|t| *t != base && t.chars().any(|c| !c.is_control())) {
            let mut v = Vec::with_capacity(t.len() + 1);
            if alt {
                v.push(0x1b);
            }
            v.extend_from_slice(t.as_bytes());
            return Some(v);
        }
        return Some(with_mods(base, ctrl, alt, shift || key_shift));
    }

    let s = character?;
    let first = s.chars().next()?;

    if ctrl {
        // Ctrl+letter and the xterm one-offs (Ctrl+Space/NUL, Ctrl+-/US)
        let low = first.to_ascii_lowercase();
        let code = match low {
            'a'..='z' => (low as u8) & 0x1f,
            '@' | ' ' | '2' => 0x00,
            '[' | '3' => 0x1b,
            '\\' | '4' => 0x1c,
            ']' | '5' => 0x1d,
            '^' | '6' => 0x1e,
            '_' | '-' | '7' => 0x1f,
            _ => return None,
        };
        let mut v = Vec::with_capacity(2);
        if alt {
            v.push(0x1b);
        }
        v.push(code);
        return Some(v);
    }

    // composed text (dead keys already resolved by winit)
    let text = text.unwrap_or("");
    let body: &str = if text.is_empty() { s } else { text };
    if body.is_empty() {
        return None;
    }
    if alt {
        let mut v = Vec::with_capacity(body.len() + 1);
        v.push(0x1b);
        v.extend_from_slice(body.as_bytes());
        return Some(v);
    }
    Some(body.as_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::ModifiersState;

    const NONE: ModifiersState = ModifiersState::empty();

    fn enc(named: Option<NamedKey>, ch: Option<&str>, mods: ModifiersState, flags: u32) -> String {
        String::from_utf8_lossy(
            &encode_parts(named, ch, ch, mods, false, flags).unwrap_or_default(),
        )
        .to_string()
    }

    #[test]
    fn without_flags_the_path_is_the_legacy_one() {
        assert_eq!(enc(Some(NamedKey::Escape), None, NONE, 0), "\x1b");
        assert_eq!(enc(None, Some("a"), NONE, 0), "a");
        assert_eq!(enc(None, Some("a"), ModifiersState::CONTROL, 0), "\x01");
    }

    #[test]
    fn flag_1_disambiguates_escape_and_ctrl() {
        let f = F_DISAMBIGUATE;
        // ESC becomes CSI 27u (the "bare ESC" vim distinguishes from Alt+[)
        assert_eq!(enc(Some(NamedKey::Escape), None, NONE, f), "\x1b[27u");
        // Ctrl+a becomes CSI 97;5u instead of ^A
        assert_eq!(
            enc(None, Some("a"), ModifiersState::CONTROL, f),
            "\x1b[97;5u"
        );
        // Enter and Tab too
        assert_eq!(enc(Some(NamedKey::Enter), None, NONE, f), "\x1b[13u");
        assert_eq!(enc(Some(NamedKey::Tab), None, NONE, f), "\x1b[9u");
        // an arrow without a modifier stays legacy (and respects DECCKM)
        assert_eq!(enc(Some(NamedKey::ArrowUp), None, NONE, f), "\x1b[A");
        // with a modifier it goes out in the kitty shape
        assert_eq!(
            enc(Some(NamedKey::ArrowUp), None, ModifiersState::CONTROL, f),
            "\x1b[1;5A"
        );
    }

    #[test]
    fn flag_8_sends_all_as_csi_u() {
        let f = F_ALL_KEYS;
        assert_eq!(enc(None, Some("a"), NONE, f), "\x1b[97u");
        // Shift+letter stops being plain text. The code goes out as 65 ('A') and
        // not 97: winit hands over the character **after** shift, and the base
        // key would need a layout table to undo it. What must not come back is
        // the plain text.
        assert_eq!(enc(None, Some("A"), ModifiersState::SHIFT, f), "\x1b[65;2u");
        // Ctrl+Shift+a reports the base key, not the uppercase character
        assert_eq!(
            enc(
                None,
                Some("A"),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                f | F_DISAMBIGUATE
            ),
            "\x1b[97;6u"
        );
        // Shift+letter WITHOUT report-all is still text (winit already hands
        // over "A")
        assert_eq!(
            enc(None, Some("A"), ModifiersState::SHIFT, F_DISAMBIGUATE),
            "A"
        );
    }

    #[test]
    fn f3_uses_the_kitty_shape_not_the_xterm_typo() {
        // `CSI 1;2R` (kitty) and not `CSI 13;2~` (which is xterm's Shift+F3)
        assert_eq!(
            enc(
                Some(NamedKey::F3),
                None,
                ModifiersState::SHIFT,
                F_DISAMBIGUATE
            ),
            "\x1b[1;2R"
        );
    }

    #[test]
    fn flag_4_sends_the_alternate_keys_of_a_shifted_letter() {
        // the case that motivated flag 4: with all keys in CSI u form, Shift+a
        // came out as a key named `A` (65)
        assert_eq!(
            enc(None, Some("A"), ModifiersState::SHIFT, F_ALL_KEYS),
            "\x1b[65;2u"
        );
        // with flag 4 the key goes back to `a` and the pair rides along
        assert_eq!(
            enc(
                None,
                Some("A"),
                ModifiersState::SHIFT,
                F_ALL_KEYS | F_ALT_KEYS
            ),
            "\x1b[97:65:97;2u"
        );
        // a key without a known pair keeps a single field
        assert_eq!(
            enc(
                None,
                Some("!"),
                ModifiersState::SHIFT,
                F_ALL_KEYS | F_ALT_KEYS
            ),
            "\x1b[33;2u"
        );
        // Ctrl+Shift+a already reports the base key: flag 4 adds nothing
        assert_eq!(
            enc(
                None,
                Some("A"),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                F_ALL_KEYS | F_ALT_KEYS
            ),
            "\x1b[97;6u"
        );
        // and without flag 8 the key is still text: flag 4 alone changes nothing
        assert_eq!(
            enc(
                None,
                Some("A"),
                ModifiersState::SHIFT,
                F_DISAMBIGUATE | F_ALT_KEYS
            ),
            "A"
        );
    }

    #[test]
    fn flag_16_embeds_the_text() {
        let f = F_ALL_KEYS | F_TEXT;
        assert_eq!(
            encode_parts(None, Some("a"), Some("a"), NONE, false, f)
                .map(|v| String::from_utf8_lossy(&v).to_string())
                .unwrap(),
            "\x1b[97;97u"
        );
    }

    fn ev(named: Option<NamedKey>, ch: Option<&str>, flags: u32, event: u8) -> Option<String> {
        encode_event(named, ch, ch, NONE, false, flags, event)
            .map(|v| String::from_utf8_lossy(&v).to_string())
    }

    #[test]
    fn release_and_repeat_follow_flag_2() {
        // E19: release without flag 2 sends nothing (it was the `pi` bug: it got
        // the text again because the legacy path does not know the key went up)
        assert_eq!(ev(None, Some("a"), F_DISAMBIGUATE, RELEASE), None);
        assert_eq!(ev(None, Some("a"), 0, RELEASE), None);
        // with flag 2 and report-all, the release becomes `:3`
        assert_eq!(
            ev(None, Some("a"), F_ALL_KEYS | F_EVENT_TYPES, RELEASE).as_deref(),
            Some("\x1b[97;1:3u")
        );
        // press is the default (no suffix), repeat spells out `:2`
        assert_eq!(
            ev(None, Some("a"), F_ALL_KEYS | F_EVENT_TYPES, PRESS).as_deref(),
            Some("\x1b[97u")
        );
        assert_eq!(
            ev(None, Some("a"), F_ALL_KEYS | F_EVENT_TYPES, REPEAT).as_deref(),
            Some("\x1b[97;1:2u")
        );
    }

    #[test]
    fn dead_key_composed_over_space_keeps_the_dead_character() {
        // winit (xkbcommon) resolves the composition but keeps the logical key
        // as the base: `~`+space arrives as Named(Space) with text `~`
        let composed = |t: &str| {
            encode_parts(Some(NamedKey::Space), None, Some(t), NONE, false, 0)
                .map(|v| String::from_utf8_lossy(&v).to_string())
        };
        assert_eq!(composed("~").as_deref(), Some("~"));
        assert_eq!(composed("´").as_deref(), Some("´"));
        assert_eq!(composed("`").as_deref(), Some("`"));
        // Alt keeps its ESC prefix
        assert_eq!(
            encode_parts(
                Some(NamedKey::Space),
                None,
                Some("~"),
                ModifiersState::ALT,
                false,
                0
            )
            .as_deref(),
            Some(&b"\x1b~"[..])
        );
        // an uncomposed space is still one space
        assert_eq!(composed(" ").as_deref(), Some(" "));
        // and a control character in `text` is not a composition: Backspace
        // brings `\x08` and has to keep going out as DEL
        assert_eq!(
            encode_parts(
                Some(NamedKey::Backspace),
                None,
                Some("\x08"),
                NONE,
                false,
                0
            )
            .as_deref(),
            Some(&b"\x7f"[..])
        );
    }

    #[test]
    fn f13_to_f24_are_f1_to_f12_with_shift() {
        // xterm's terminfo: `kf13=\E[1;2P` .. `kf24=\E[24;2~`
        assert_eq!(enc(Some(NamedKey::F13), None, NONE, 0), "\x1b[1;2P");
        assert_eq!(enc(Some(NamedKey::F14), None, NONE, 0), "\x1b[1;2Q");
        assert_eq!(enc(Some(NamedKey::F17), None, NONE, 0), "\x1b[15;2~");
        assert_eq!(enc(Some(NamedKey::F24), None, NONE, 0), "\x1b[24;2~");
        // a modifier adds to the Shift the key already carries (1+shift+ctrl=6)
        assert_eq!(
            enc(Some(NamedKey::F13), None, ModifiersState::CONTROL, 0),
            "\x1b[1;6P"
        );
        // with the kitty flags they only travel as `CSI <number> u`: there is no
        // legacy sequence for them
        assert_eq!(
            enc(Some(NamedKey::F13), None, NONE, F_ALL_KEYS),
            "\x1b[57376u"
        );
        assert_eq!(
            enc(Some(NamedKey::F24), None, NONE, F_ALL_KEYS),
            "\x1b[57387u"
        );
        // disambiguate alone is not enough: without flag 8 the legacy path
        // already sends the right thing
        assert_eq!(
            enc(Some(NamedKey::F13), None, NONE, F_DISAMBIGUATE),
            "\x1b[1;2P"
        );
    }

    #[test]
    fn the_keypad_sends_its_character_even_with_num_lock_on() {
        // winit has no `NamedKey` for `KP_7` (`keymap.rs` comments them out):
        // with NumLock on the event arrives as `Unidentified` and the physical
        // key is the only thing left to read
        let kp = |c| resolve_key(&Key::Dead(None), PhysicalKey::Code(c));
        assert_eq!(kp(KeyCode::Numpad7), (None, Some("7")));
        assert_eq!(kp(KeyCode::Numpad0), (None, Some("0")));
        assert_eq!(kp(KeyCode::NumpadDecimal), (None, Some(".")));
        assert_eq!(kp(KeyCode::NumpadAdd), (None, Some("+")));
        assert_eq!(kp(KeyCode::NumpadDivide), (None, Some("/")));
        // with NumLock off the keysym is a navigation one and winit maps it: the
        // logical key wins and the keypad keeps being the arrow cluster
        assert_eq!(
            resolve_key(
                &Key::Named(NamedKey::ArrowUp),
                PhysicalKey::Code(KeyCode::Numpad8)
            ),
            (Some(NamedKey::ArrowUp), None)
        );
        // any other physical key stays nothing
        assert_eq!(
            resolve_key(&Key::Dead(None), PhysicalKey::Code(KeyCode::KeyA)),
            (None, None)
        );
    }
}

/// N3: the `kitty_keyboard()` check of `checks.py`, end to end — the parser in
/// `Term` turns the flags on (`CSI > u`, `CSI = u`, `CSI < u`, `CSI ? u`) and the
/// encoder in this file produces the bytes that go to the child.
#[cfg(test)]
mod checks_keyboard {
    use super::*;
    use tortty_core::term::Term;

    const NONE: ModifiersState = ModifiersState::empty();

    fn term() -> Term {
        Term::new(80, 24)
    }

    /// The encoder with the state the app really uses (`?1` and `kbd_flags` from
    /// `Term`), to compare the exact bytes sent to the child.
    fn enc(
        t: &Term,
        named: Option<NamedKey>,
        ch: Option<&str>,
        mods: ModifiersState,
    ) -> Option<String> {
        encode_parts(named, ch, ch, mods, t.app_cursor, t.kbd_flags)
            .map(|v| String::from_utf8_lossy(&v).to_string())
    }

    /// With text associated that differs from the character — the flag 16 case.
    fn enc_text(t: &Term, ch: &str, text: &str) -> Option<String> {
        encode_parts(None, Some(ch), Some(text), NONE, t.app_cursor, t.kbd_flags)
            .map(|v| String::from_utf8_lossy(&v).to_string())
    }

    /// Press/repeat/release, which reach winit as `KeyEvent` state.
    fn enc_ev(
        t: &Term,
        named: Option<NamedKey>,
        ch: Option<&str>,
        mods: ModifiersState,
        event: u8,
    ) -> Option<String> {
        encode_event(named, ch, ch, mods, t.app_cursor, t.kbd_flags, event)
            .map(|v| String::from_utf8_lossy(&v).to_string())
    }

    #[test]
    fn push_pop_set_and_query_of_keyboard_by_term() {
        let mut t = term();
        assert_eq!(t.kbd_flags, 0);
        // `CSI > 1 u`: push the current state (0) and start disambiguating
        t.feed(b"\x1b[>1u");
        assert_eq!(t.kbd_flags, F_DISAMBIGUATE);
        // `CSI ? u`: the terminal answers with the current state
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?1u".to_vec());
        // `CSI = 3 ; 1 u` (mode 1 = replace): flags 1|2
        t.feed(b"\x1b[=3;1u");
        assert_eq!(t.kbd_flags, F_DISAMBIGUATE | F_EVENT_TYPES);
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?3u".to_vec());
        // mode 2 adds, mode 3 removes
        t.feed(b"\x1b[=8;2u");
        assert_eq!(t.kbd_flags, 11);
        t.feed(b"\x1b[=8;3u");
        assert_eq!(t.kbd_flags, 3);
        // push of 8|16 and pop gives back what was stored
        t.feed(b"\x1b[>24u");
        assert_eq!(t.kbd_flags, F_ALL_KEYS | F_TEXT);
        t.feed(b"\x1b[<u");
        assert_eq!(t.kbd_flags, 3);
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?3u".to_vec());
        // `CSI < 2 u` empties the stack: kitty says the flags go back to 0
        t.feed(b"\x1b[<2u");
        assert_eq!(t.kbd_flags, 0);
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?0u".to_vec());
    }

    #[test]
    fn flag_1_of_term_disambiguates_escape_and_modifiers() {
        let mut t = term();
        t.feed(b"\x1b[>1u");
        // ESC, Enter, Tab and Backspace go out as `CSI code u`
        assert_eq!(
            enc(&t, Some(NamedKey::Escape), None, NONE).as_deref(),
            Some("\x1b[27u")
        );
        assert_eq!(
            enc(&t, Some(NamedKey::Enter), None, NONE).as_deref(),
            Some("\x1b[13u")
        );
        assert_eq!(
            enc(&t, Some(NamedKey::Tab), None, NONE).as_deref(),
            Some("\x1b[9u")
        );
        assert_eq!(
            enc(&t, Some(NamedKey::Backspace), None, NONE).as_deref(),
            Some("\x1b[127u")
        );
        // Ctrl+a stops being ^A; Alt+a stops being ESC + "a"
        assert_eq!(
            enc(&t, None, Some("a"), ModifiersState::CONTROL).as_deref(),
            Some("\x1b[97;5u")
        );
        assert_eq!(
            enc(&t, None, Some("a"), ModifiersState::ALT).as_deref(),
            Some("\x1b[97;3u")
        );
        assert_eq!(
            enc(
                &t,
                None,
                Some("a"),
                ModifiersState::CONTROL | ModifiersState::ALT
            )
            .as_deref(),
            Some("\x1b[97;7u")
        );
        // Shift+Tab is the kitty shape, not the legacy `\t`
        assert_eq!(
            enc(&t, Some(NamedKey::Tab), None, ModifiersState::SHIFT).as_deref(),
            Some("\x1b[9;2u")
        );
        // F1..F4 without a modifier stay `SS3 P`.., which already is the kitty
        // shape
        assert_eq!(
            enc(&t, Some(NamedKey::F3), None, NONE).as_deref(),
            Some("\x1bOR")
        );
        // and F3 with Shift is `CSI 1;2R`, not the `CSI 13~` of xterm
        assert_eq!(
            enc(&t, Some(NamedKey::F3), None, ModifiersState::SHIFT).as_deref(),
            Some("\x1b[1;2R")
        );
    }

    #[test]
    fn decckm_decides_the_legacy_not_the_kitty_shape() {
        let mut t = term();
        assert!(!t.app_cursor);
        assert_eq!(
            enc(&t, Some(NamedKey::ArrowUp), None, NONE).as_deref(),
            Some("\x1b[A")
        );
        // `CSI ? 1 h` turns DECCKM on: arrow and Home without a modifier become
        // SS3
        t.feed(b"\x1b[?1h");
        assert!(t.app_cursor);
        assert_eq!(
            enc(&t, Some(NamedKey::ArrowUp), None, NONE).as_deref(),
            Some("\x1bOA")
        );
        assert_eq!(
            enc(&t, Some(NamedKey::Home), None, NONE).as_deref(),
            Some("\x1bOH")
        );
        // with a modifier the kitty shape is the same in both modes
        t.feed(b"\x1b[>1u");
        assert_eq!(
            enc(&t, Some(NamedKey::ArrowUp), None, ModifiersState::CONTROL).as_deref(),
            Some("\x1b[1;5A")
        );
        t.feed(b"\x1b[?1l");
        assert!(!t.app_cursor);
        assert_eq!(
            enc(&t, Some(NamedKey::ArrowUp), None, ModifiersState::CONTROL).as_deref(),
            Some("\x1b[1;5A")
        );
        // flag 8 sends the arrow in the kitty shape even with DECCKM on
        t.feed(b"\x1b[?1h\x1b[=8;2u");
        assert!(t.app_cursor);
        assert_eq!(t.kbd_flags, F_DISAMBIGUATE | F_ALL_KEYS);
        assert_eq!(
            enc(&t, Some(NamedKey::ArrowUp), None, NONE).as_deref(),
            Some("\x1b[A")
        );
        assert_eq!(
            enc(&t, Some(NamedKey::Home), None, NONE).as_deref(),
            Some("\x1b[H")
        );
        assert_eq!(
            enc(&t, Some(NamedKey::ArrowLeft), None, NONE).as_deref(),
            Some("\x1b[D")
        );
    }

    #[test]
    fn flag_8_sends_all_including_super() {
        let mut t = term();
        t.feed(b"\x1b[>8u");
        assert_eq!(enc(&t, None, Some("a"), NONE).as_deref(), Some("\x1b[97u"));
        // Shift+letter goes out with the POST-shift code (LIMITATIONS §Keyboard):
        // winit hands over "A" and undoing the Shift would need a layout table
        assert_eq!(
            enc(&t, None, Some("A"), ModifiersState::SHIFT).as_deref(),
            Some("\x1b[65;2u")
        );
        // but Ctrl+Shift+a reports the base key (97), mods 1+shift+ctrl = 6
        assert_eq!(
            enc(
                &t,
                None,
                Some("A"),
                ModifiersState::CONTROL | ModifiersState::SHIFT
            )
            .as_deref(),
            Some("\x1b[97;6u")
        );
        // Super stops being a "WM shortcut": it enters the count (1+8 = 9)
        assert_eq!(
            enc(&t, None, Some("a"), ModifiersState::SUPER).as_deref(),
            Some("\x1b[97;9u")
        );
        // and with no flag at all Super still does not reach the child
        let without_flags = term();
        assert_eq!(
            enc(&without_flags, None, Some("a"), ModifiersState::SUPER),
            None
        );
    }

    #[test]
    fn flag_2_held_key_and_release() {
        let mut t = term();
        t.feed(b"\x1b[>10u"); // 2|8
        assert_eq!(
            enc_ev(&t, None, Some("a"), NONE, PRESS).as_deref(),
            Some("\x1b[97u")
        );
        // `:2` is the held key (auto-repeat), `:3` is the key going up
        assert_eq!(
            enc_ev(&t, None, Some("a"), NONE, REPEAT).as_deref(),
            Some("\x1b[97;1:2u")
        );
        assert_eq!(
            enc_ev(&t, None, Some("a"), NONE, RELEASE).as_deref(),
            Some("\x1b[97;1:3u")
        );
        // the arrow gets the event field too
        assert_eq!(
            enc_ev(&t, Some(NamedKey::ArrowUp), None, NONE, REPEAT).as_deref(),
            Some("\x1b[1;1:2A")
        );
        // only flag 2: text stays text and the release does not go out twice
        // (E19 — the legacy path does not know the key went up)
        let mut only_eventos = term();
        only_eventos.feed(b"\x1b[>2u");
        assert_eq!(
            enc_ev(&only_eventos, None, Some("a"), NONE, PRESS).as_deref(),
            Some("a")
        );
        assert_eq!(enc_ev(&only_eventos, None, Some("a"), NONE, RELEASE), None);
        // ESC without disambiguate has no release either
        assert_eq!(
            enc_ev(&only_eventos, Some(NamedKey::Escape), None, NONE, RELEASE),
            None
        );
        // with flag 1 along (1|2) ESC gains `:3` and the press loses the suffix
        only_eventos.feed(b"\x1b[=1;2u");
        assert_eq!(only_eventos.kbd_flags, F_DISAMBIGUATE | F_EVENT_TYPES);
        assert_eq!(
            enc_ev(&only_eventos, Some(NamedKey::Escape), None, NONE, RELEASE).as_deref(),
            Some("\x1b[27;1:3u")
        );
        assert_eq!(
            enc_ev(&only_eventos, Some(NamedKey::Escape), None, NONE, PRESS).as_deref(),
            Some("\x1b[27u")
        );
    }

    #[test]
    fn ctrl_space_depends_on_the_flag_not_the_shortcut() {
        // only flag 2 (events): Ctrl+Space stays NUL — readline/vim depend on it
        // and swapping it for `CSI 32;5u` would break whoever turns on flag 2
        let mut t = term();
        t.feed(b"\x1b[>2u");
        assert_eq!(
            enc(&t, Some(NamedKey::Space), None, ModifiersState::CONTROL).as_deref(),
            Some("\x00")
        );
        // with flag 1 kitty disambiguates: `CSI 32;5u`
        t.feed(b"\x1b[=1;2u");
        assert_eq!(
            enc(&t, Some(NamedKey::Space), None, ModifiersState::CONTROL).as_deref(),
            Some("\x1b[32;5u")
        );
        // with no modifier and no flag 8, space stays space
        assert_eq!(
            enc(&t, Some(NamedKey::Space), None, NONE).as_deref(),
            Some(" ")
        );
        // with flag 8, all keys: space becomes `CSI 32u`
        t.feed(b"\x1b[=24;1u");
        assert_eq!(t.kbd_flags, F_ALL_KEYS | F_TEXT);
        assert_eq!(enc_text(&t, " ", " ").as_deref(), Some("\x1b[32;32u"));
    }

    #[test]
    fn flag_16_embeds_the_text_of_the_key() {
        let mut t = term();
        t.feed(b"\x1b[>24u"); // 8|16
        assert_eq!(t.kbd_flags, F_ALL_KEYS | F_TEXT);
        assert_eq!(enc_text(&t, "a", "a").as_deref(), Some("\x1b[97;97u"));
        // 'é' = U+00E9 = 233: key and text are the same codepoint
        assert_eq!(enc_text(&t, "é", "é").as_deref(), Some("\x1b[233;233u"));
        // text with more than one codepoint goes out separated by ':'
        assert_eq!(enc_text(&t, "a", "aé").as_deref(), Some("\x1b[97;97:233u"));
        // without flag 16 the text does not travel along
        t.feed(b"\x1b[=16;3u");
        assert_eq!(t.kbd_flags, F_ALL_KEYS);
        assert_eq!(enc_text(&t, "a", "a").as_deref(), Some("\x1b[97u"));
        // and with no flags at all the text is the usual legacy path
        t.feed(b"\x1b[=8;3u");
        assert_eq!(t.kbd_flags, 0);
        assert_eq!(enc_text(&t, "a", "a").as_deref(), Some("a"));
    }
}
