//! Teclado: evento do winit -> bytes que o terminal manda para o filho.
//!
//! Mesma tabela do `bzmux/keys.py`: setas em modo aplicação (DECCKM `?1`),
//! modificadores no formato do xterm, Alt como prefixo ESC, Shift+Tab `\e[Z` e
//! Ctrl+letra em 0x01..0x1a.

use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// 1 + shift(1) + alt(2) + ctrl(4) — o parâmetro de modificador do xterm.
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

/// Aplica modificadores no formato do xterm (como o `_special` do Python).
fn with_mods(seq: &str, ctrl: bool, alt: bool, shift: bool) -> Vec<u8> {
    // Shift+Tab chega como `\t` com shift: a forma canônica é backtab
    if seq == "\t" && shift {
        // Shift+Tab é backtab; o shift já está na forma canônica
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

/// Traduz o evento. `None` = não manda nada (modificador, Super do WM, etc.).
pub fn encode(ev: &KeyEvent, mods: ModifiersState, app_cursor: bool) -> Option<Vec<u8>> {
    if ev.state != ElementState::Pressed {
        return None;
    }
    let ctrl = mods.control_key();
    let alt = mods.alt_key();
    let shift = mods.shift_key();
    // atalho do window manager: não é do terminal
    if mods.super_key() {
        return None;
    }

    if let Key::Named(n) = &ev.logical_key {
        let base = named_base(*n, app_cursor)?;
        return Some(with_mods(base, ctrl, alt, shift));
    }

    let Key::Character(s) = &ev.logical_key else {
        return None;
    };
    let first = s.chars().next()?;

    if ctrl {
        // Ctrl+letra e os pontuais do xterm (Ctrl+Space/NUL, Ctrl+-/US)
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

    // texto composto (dead keys já resolvidas pelo winit)
    let text = ev.text.as_deref().unwrap_or("");
    let body: &str = if text.is_empty() { s.as_str() } else { text };
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
