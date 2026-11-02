//! Teclado: evento do winit -> bytes que o terminal manda para o filho.
//!
//! Mesma tabela do `bzmux/keys.py`: setas em modo aplicação (DECCKM `?1`),
//! modificadores no formato do xterm, Alt como prefixo ESC, Shift+Tab `\e[Z` e
//! Ctrl+letra em 0x01..0x1a.
//!
//! Por cima disso, o protocolo de teclado do kitty (`kitty_kbd.py`): quando o
//! filho liga flags por `CSI > u`, as teclas saem como `CSI codigo ; mods u` em
//! vez do byte de controle. As flags moram no `Term` (uma por aba) e quem
//! decide é o app — aqui só mora a codificação.

use std::fmt::Write as _;

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

// ------------------------------------------------- protocolo do kitty

/// Flags do `CSI > u` (bits do kitty; 4 = alternate keys, fora do escopo).
pub const F_DISAMBIGUATE: u32 = 1;
pub const F_EVENT_TYPES: u32 = 2;
pub const F_ALL_KEYS: u32 = 8;
pub const F_TEXT: u32 = 16;

const PRESS: u8 = 1;
const REPEAT: u8 = 2;
const RELEASE: u8 = 3;

/// Tecla nomeada na forma do kitty: (número do CSI, trailer).
///
/// O trailer é o que muda entre `CSI 27;5 u` (Escape com Ctrl) e
/// `CSI 1;5 A` (seta para cima com Ctrl). Vale a tabela do kitty
/// (`key_encoding.c`) — inclusive F3 = `1 R`, que no `kitty_kbd.py` do Python
/// saiu como `13 ~` (typo: F3 é `SS3 R`, não `CSI 13~`).
fn kitty_functional(n: NamedKey) -> Option<(u32, char)> {
    use NamedKey::{
        ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Backspace, Delete, End, Enter, Escape, Home,
        Insert, PageDown, PageUp, Tab, F1, F10, F11, F12, F2, F3, F4, F5, F6, F7, F8, F9,
    };
    Some(match n {
        Escape => (27, 'u'),
        Enter => (13, 'u'),
        Tab => (9, 'u'),
        Backspace => (127, 'u'),
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
        _ => return None,
    })
}

/// `CSI chave ; mods[:evento] [;texto] trailer`, sem campo que o terminal não
/// precisa ver (é o mesmo cuidado do `_csi` do Python).
fn kitty_csi(chave: u32, mods: u8, trailer: char, evento: Option<u8>, texto: &str) -> Vec<u8> {
    let evento = evento.filter(|e| *e != PRESS);
    let mut out = String::from("\x1b[");
    if chave != 1 || mods != 1 || evento.is_some() || !texto.is_empty() {
        out.push_str(&chave.to_string());
    }
    if mods != 1 || evento.is_some() {
        out.push(';');
        if mods == 1 {
            out.push('1'); // sem modificador o campo ainda existe
        } else {
            out.push_str(&mods.to_string());
        }
    }
    if let Some(e) = evento {
        let _ = write!(out, ":{e}");
    }
    if !texto.is_empty() {
        let cps: Vec<String> = texto.chars().map(|c| (c as u32).to_string()).collect();
        out.push(';');
        out.push_str(&cps.join(":"));
    }
    out.push(trailer);
    out.into_bytes()
}

/// O que o protocolo do kitty respondeu para esta tecla.
enum Kitty {
    /// Sai como `CSI ... u` (ou variante).
    Seq(Vec<u8>),
    /// Não cobre: cai no caminho legado do xterm.
    Legacy,
}

#[allow(clippy::too_many_arguments)]
fn kitty_encode(
    named: Option<NamedKey>,
    character: Option<&str>,
    text: &str,
    mods: ModifiersState,
    flags: u32,
    evento: u8,
) -> Kitty {
    let dis = flags & F_DISAMBIGUATE != 0;
    let ev_types = flags & F_EVENT_TYPES != 0;
    let all_keys = flags & F_ALL_KEYS != 0;
    let embed = flags & F_TEXT != 0;

    // sem a flag 2 não existe release: o legado só sabe "a tecla saiu"
    if evento == RELEASE && !ev_types {
        return Kitty::Legacy;
    }
    let m = 1
        + u8::from(mods.shift_key())
        + 2 * u8::from(mods.alt_key())
        + 4 * u8::from(mods.control_key())
        + 8 * u8::from(mods.super_key());
    let texto = if embed { text } else { "" };
    let ev = ev_types.then_some(evento);

    if let Some(n) = named {
        let Some((chave, trailer)) = kitty_functional(n) else {
            return Kitty::Legacy;
        };
        let especial = matches!(
            n,
            NamedKey::Escape | NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace
        );
        if !especial && m == 1 && !all_keys {
            // seta/Home/F1 sem modificador: o legado já manda o certo (e
            // respeita DECCKM)
            return Kitty::Legacy;
        }
        if especial && m == 1 && !(dis || all_keys) {
            return Kitty::Legacy; // ESC, CR, TAB, DEL clássicos
        }
        return Kitty::Seq(kitty_csi(chave, m, trailer, ev, texto));
    }

    let Some(cp) = character.and_then(|s| s.chars().next()) else {
        return Kitty::Legacy; // tecla sem caractere (modificador, tecla morta)
    };
    let mut cp = cp as u32;
    // Ctrl+Shift+a relata a tecla base, não o maiúsculo
    if mods.control_key() && (0x41..=0x5a).contains(&cp) {
        cp += 32;
    }
    if m == 2 && !all_keys {
        return Kitty::Legacy; // Shift+letra continua sendo texto
    }
    if m == 1 && !all_keys {
        return Kitty::Legacy; // texto puro
    }
    if !(dis || all_keys) {
        return Kitty::Legacy; // modificador sem disambiguate: xterm clássico
    }
    Kitty::Seq(kitty_csi(cp, m, 'u', ev, texto))
}

/// Traduz o evento. `None` = não manda nada (modificador, Super do WM, etc.).
pub fn encode(
    ev: &KeyEvent,
    mods: ModifiersState,
    app_cursor: bool,
    kbd_flags: u32,
) -> Option<Vec<u8>> {
    let evento = if ev.state == ElementState::Released {
        RELEASE
    } else if ev.repeat {
        REPEAT
    } else {
        PRESS
    };
    // release só interessa ao protocolo do kitty (o legado não relata soltura)
    if ev.state != ElementState::Pressed && kbd_flags & F_EVENT_TYPES == 0 {
        return None;
    }
    let (named, character) = match &ev.logical_key {
        Key::Named(n) => (Some(*n), None),
        Key::Character(s) => (None, Some(s.as_str())),
        _ => return None,
    };
    encode_event(
        named,
        character,
        ev.text.as_deref(),
        mods,
        app_cursor,
        kbd_flags,
        evento,
    )
}

/// O corpo do `encode` sem o `KeyEvent`: é o que os testes usam para o release
/// e o repeat, que são decisão do protocolo e não do winit.
pub fn encode_event(
    named: Option<NamedKey>,
    character: Option<&str>,
    text: Option<&str>,
    mods: ModifiersState,
    app_cursor: bool,
    kbd_flags: u32,
    evento: u8,
) -> Option<Vec<u8>> {
    if kbd_flags != 0 {
        match kitty_encode(
            named,
            character,
            text.unwrap_or(""),
            mods,
            kbd_flags,
            evento,
        ) {
            Kitty::Seq(v) => return Some(v),
            Kitty::Legacy => {},
        }
    }
    // release sem forma do kitty: não sai nada (E19 — senão o `pi`, que liga a
    // flag 2, recebia o texto duas vezes: o legado não sabe "a tecla saiu" e
    // mandava o texto de novo)
    if evento == RELEASE {
        return None;
    }
    encode_parts(named, character, text, mods, app_cursor, kbd_flags)
}

/// A tradução em si, sem depender de um `KeyEvent`: é o que o verbo `keys` do
/// debug socket usa, para testar o **mesmo** caminho do teclado real (um
/// encoder paralelo no socket já divergiu uma vez: F5 virava 1 byte).
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
    // atalho do window manager: não é do terminal
    if mods.super_key() {
        return None;
    }

    if let Some(n) = named {
        // Ctrl+Space é NUL (é o que o readline/vim esperam para "marcar"),
        // não o `CSI 1;5 ` que a tabela genérica produziria
        if ctrl && n == NamedKey::Space {
            return Some(if alt { vec![0x1b, 0x00] } else { vec![0x00] });
        }
        let base = named_base(n, app_cursor)?;
        return Some(with_mods(base, ctrl, alt, shift));
    }

    let s = character?;
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
    fn sem_flags_o_caminho_e_o_legado() {
        assert_eq!(enc(Some(NamedKey::Escape), None, NONE, 0), "\x1b");
        assert_eq!(enc(None, Some("a"), NONE, 0), "a");
        assert_eq!(enc(None, Some("a"), ModifiersState::CONTROL, 0), "\x01");
    }

    #[test]
    fn flag_1_desambigua_escape_e_ctrl() {
        let f = F_DISAMBIGUATE;
        // ESC vira CSI 27u (o "ESC sozinho" que o vim distingue de Alt+[)
        assert_eq!(enc(Some(NamedKey::Escape), None, NONE, f), "\x1b[27u");
        // Ctrl+a vira CSI 97;5u em vez de ^A
        assert_eq!(
            enc(None, Some("a"), ModifiersState::CONTROL, f),
            "\x1b[97;5u"
        );
        // Enter e Tab também
        assert_eq!(enc(Some(NamedKey::Enter), None, NONE, f), "\x1b[13u");
        assert_eq!(enc(Some(NamedKey::Tab), None, NONE, f), "\x1b[9u");
        // seta sem modificador continua no legado (e respeita DECCKM)
        assert_eq!(enc(Some(NamedKey::ArrowUp), None, NONE, f), "\x1b[A");
        // com modificador, sai na forma do kitty
        assert_eq!(
            enc(Some(NamedKey::ArrowUp), None, ModifiersState::CONTROL, f),
            "\x1b[1;5A"
        );
    }

    #[test]
    fn flag_8_manda_tudo_como_csi_u() {
        let f = F_ALL_KEYS;
        assert_eq!(enc(None, Some("a"), NONE, f), "\x1b[97u");
        // Shift+letra deixa de ser texto puro. O código sai como 65 ('A') e não
        // 97: o winit entrega o caractere **depois** do shift, e a tecla base
        // exigiria uma tabela de layout (o Python, via Gdk keyval, faz igual).
        // O que não pode voltar é o texto puro.
        assert_eq!(enc(None, Some("A"), ModifiersState::SHIFT, f), "\x1b[65;2u");
        // Ctrl+Shift+a relata a tecla base, não o maiúsculo
        assert_eq!(
            enc(
                None,
                Some("A"),
                ModifiersState::CONTROL | ModifiersState::SHIFT,
                f | F_DISAMBIGUATE
            ),
            "\x1b[97;6u"
        );
        // Shift+letra SEM report-all continua texto (o winit já entrega "A")
        assert_eq!(
            enc(None, Some("A"), ModifiersState::SHIFT, F_DISAMBIGUATE),
            "A"
        );
    }

    #[test]
    fn f3_usa_a_forma_do_kitty_e_nao_o_typo_do_python() {
        // `CSI 1;2R` (kitty) e não `CSI 13;2~` (o `kitty_kbd.py` tinha F3 como
        // 13~, que é Shift+F3 do xterm)
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
    fn flag_16_embute_o_texto() {
        let f = F_ALL_KEYS | F_TEXT;
        assert_eq!(
            encode_parts(None, Some("a"), Some("a"), NONE, false, f)
                .map(|v| String::from_utf8_lossy(&v).to_string())
                .unwrap(),
            "\x1b[97;97u"
        );
    }

    fn ev(named: Option<NamedKey>, ch: Option<&str>, flags: u32, evento: u8) -> Option<String> {
        encode_event(named, ch, ch, NONE, false, flags, evento)
            .map(|v| String::from_utf8_lossy(&v).to_string())
    }

    #[test]
    fn release_e_repeat_seguem_a_flag_2() {
        // E19: release sem a flag 2 não manda nada (era o bug do `pi`, que
        // recebia o texto de novo porque o legado não sabe "a tecla saiu")
        assert_eq!(ev(None, Some("a"), F_DISAMBIGUATE, RELEASE), None);
        assert_eq!(ev(None, Some("a"), 0, RELEASE), None);
        // com a flag 2 e report-all, o release vira `:3`
        assert_eq!(
            ev(None, Some("a"), F_ALL_KEYS | F_EVENT_TYPES, RELEASE).as_deref(),
            Some("\x1b[97;1:3u")
        );
        // press é o padrão (sem sufixo), repeat explicita `:2`
        assert_eq!(
            ev(None, Some("a"), F_ALL_KEYS | F_EVENT_TYPES, PRESS).as_deref(),
            Some("\x1b[97u")
        );
        assert_eq!(
            ev(None, Some("a"), F_ALL_KEYS | F_EVENT_TYPES, REPEAT).as_deref(),
            Some("\x1b[97;1:2u")
        );
    }
}

/// N3: o check `kitty_keyboard()` do `checks.py`, de ponta a ponta — o parser do
/// `Term` liga as flags (`CSI > u`, `CSI = u`, `CSI < u`, `CSI ? u`) e o encoder
/// deste arquivo produz os bytes que vão para o filho.
#[cfg(test)]
mod checks_teclado {
    use super::*;
    use bzmux_core::term::Term;

    const NONE: ModifiersState = ModifiersState::empty();

    fn term() -> Term {
        Term::new(80, 24)
    }

    /// O encoder com o estado que o app usa de verdade (`?1` e `kbd_flags` do
    /// `Term`), para comparar bytes como o check do Python faz.
    fn enc(
        t: &Term,
        named: Option<NamedKey>,
        ch: Option<&str>,
        mods: ModifiersState,
    ) -> Option<String> {
        encode_parts(named, ch, ch, mods, t.app_cursor, t.kbd_flags)
            .map(|v| String::from_utf8_lossy(&v).to_string())
    }

    /// Com texto associado diferente do caractere — é o caso da flag 16.
    fn enc_texto(t: &Term, ch: &str, texto: &str) -> Option<String> {
        encode_parts(None, Some(ch), Some(texto), NONE, t.app_cursor, t.kbd_flags)
            .map(|v| String::from_utf8_lossy(&v).to_string())
    }

    /// Press/repeat/release, que no winit chegam como estado do `KeyEvent`.
    fn enc_ev(
        t: &Term,
        named: Option<NamedKey>,
        ch: Option<&str>,
        mods: ModifiersState,
        evento: u8,
    ) -> Option<String> {
        encode_event(named, ch, ch, mods, t.app_cursor, t.kbd_flags, evento)
            .map(|v| String::from_utf8_lossy(&v).to_string())
    }

    #[test]
    fn push_pop_set_e_consulta_do_teclado_pelo_term() {
        let mut t = term();
        assert_eq!(t.kbd_flags, 0);
        // `CSI > 1 u`: empilha o estado atual (0) e passa a desambiguar
        t.feed(b"\x1b[>1u");
        assert_eq!(t.kbd_flags, F_DISAMBIGUATE);
        // `CSI ? u`: o terminal responde o estado atual
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?1u".to_vec());
        // `CSI = 3 ; 1 u` (modo 1 = substitui): flags 1|2
        t.feed(b"\x1b[=3;1u");
        assert_eq!(t.kbd_flags, F_DISAMBIGUATE | F_EVENT_TYPES);
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?3u".to_vec());
        // modo 2 acrescenta, modo 3 remove
        t.feed(b"\x1b[=8;2u");
        assert_eq!(t.kbd_flags, 11);
        t.feed(b"\x1b[=8;3u");
        assert_eq!(t.kbd_flags, 3);
        // push de 8|16 e pop devolve o que estava guardado
        t.feed(b"\x1b[>24u");
        assert_eq!(t.kbd_flags, F_ALL_KEYS | F_TEXT);
        t.feed(b"\x1b[<u");
        assert_eq!(t.kbd_flags, 3);
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?3u".to_vec());
        // `CSI < 2 u` esvazia a pilha: o kitty manda os flags voltarem a 0
        t.feed(b"\x1b[<2u");
        assert_eq!(t.kbd_flags, 0);
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?0u".to_vec());
    }

    #[test]
    fn flag_1_do_term_desambigua_escape_e_modificadores() {
        let mut t = term();
        t.feed(b"\x1b[>1u");
        // ESC, Enter, Tab e Backspace saem como `CSI codigo u`
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
        // Ctrl+a deixa de ser ^A; Alt+a deixa de ser ESC + "a"
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
        // Shift+Tab é a forma do kitty, não o `\t` do legado
        assert_eq!(
            enc(&t, Some(NamedKey::Tab), None, ModifiersState::SHIFT).as_deref(),
            Some("\x1b[9;2u")
        );
        // F1..F4 sem modificador continuam `SS3 P`.., que já é a forma do kitty
        assert_eq!(
            enc(&t, Some(NamedKey::F3), None, NONE).as_deref(),
            Some("\x1bOR")
        );
        // e F3 com Shift é `CSI 1;2R` (não o `CSI 13~` do `kitty_kbd.py`)
        assert_eq!(
            enc(&t, Some(NamedKey::F3), None, ModifiersState::SHIFT).as_deref(),
            Some("\x1b[1;2R")
        );
    }

    #[test]
    fn decckm_decide_o_legado_e_nao_a_forma_do_kitty() {
        let mut t = term();
        assert!(!t.app_cursor);
        assert_eq!(
            enc(&t, Some(NamedKey::ArrowUp), None, NONE).as_deref(),
            Some("\x1b[A")
        );
        // `CSI ? 1 h` liga DECCKM: seta e Home sem modificador viram SS3
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
        // com modificador a forma do kitty é a mesma nos dois modos
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
        // flag 8 manda a seta na forma do kitty mesmo com DECCKM ligado
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
    fn flag_8_manda_tudo_inclusive_super() {
        let mut t = term();
        t.feed(b"\x1b[>8u");
        assert_eq!(enc(&t, None, Some("a"), NONE).as_deref(), Some("\x1b[97u"));
        // Shift+letra sai com o código PÓS-shift (LIMITATIONS §Teclado): o winit
        // entrega "A" e desfazer o Shift exigiria tabela de layout
        assert_eq!(
            enc(&t, None, Some("A"), ModifiersState::SHIFT).as_deref(),
            Some("\x1b[65;2u")
        );
        // mas Ctrl+Shift+a relata a tecla base (97), mods 1+shift+ctrl = 6
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
        // Super deixa de ser "atalho do WM": entra na conta (1+8 = 9)
        assert_eq!(
            enc(&t, None, Some("a"), ModifiersState::SUPER).as_deref(),
            Some("\x1b[97;9u")
        );
        // e sem flag nenhuma o Super continua não indo para o filho
        let sem_flags = term();
        assert_eq!(
            enc(&sem_flags, None, Some("a"), ModifiersState::SUPER),
            None
        );
    }

    #[test]
    fn flag_2_tecla_segurada_e_soltura() {
        let mut t = term();
        t.feed(b"\x1b[>10u"); // 2|8
        assert_eq!(
            enc_ev(&t, None, Some("a"), NONE, PRESS).as_deref(),
            Some("\x1b[97u")
        );
        // `:2` é a tecla segurada (auto-repeat), `:3` é a soltura
        assert_eq!(
            enc_ev(&t, None, Some("a"), NONE, REPEAT).as_deref(),
            Some("\x1b[97;1:2u")
        );
        assert_eq!(
            enc_ev(&t, None, Some("a"), NONE, RELEASE).as_deref(),
            Some("\x1b[97;1:3u")
        );
        // seta também ganha o campo de evento
        assert_eq!(
            enc_ev(&t, Some(NamedKey::ArrowUp), None, NONE, REPEAT).as_deref(),
            Some("\x1b[1;1:2A")
        );
        // só a flag 2: texto continua texto e a soltura não sai duas vezes
        // (E19 — o legado não sabe "a tecla saiu")
        let mut so_eventos = term();
        so_eventos.feed(b"\x1b[>2u");
        assert_eq!(
            enc_ev(&so_eventos, None, Some("a"), NONE, PRESS).as_deref(),
            Some("a")
        );
        assert_eq!(enc_ev(&so_eventos, None, Some("a"), NONE, RELEASE), None);
        // ESC sem disambiguate também não tem release
        assert_eq!(
            enc_ev(&so_eventos, Some(NamedKey::Escape), None, NONE, RELEASE),
            None
        );
        // com a flag 1 junto (1|2) o ESC ganha `:3` e o press perde o sufixo
        so_eventos.feed(b"\x1b[=1;2u");
        assert_eq!(so_eventos.kbd_flags, F_DISAMBIGUATE | F_EVENT_TYPES);
        assert_eq!(
            enc_ev(&so_eventos, Some(NamedKey::Escape), None, NONE, RELEASE).as_deref(),
            Some("\x1b[27;1:3u")
        );
        assert_eq!(
            enc_ev(&so_eventos, Some(NamedKey::Escape), None, NONE, PRESS).as_deref(),
            Some("\x1b[27u")
        );
    }

    #[test]
    fn flag_16_embute_o_texto_da_tecla() {
        let mut t = term();
        t.feed(b"\x1b[>24u"); // 8|16
        assert_eq!(t.kbd_flags, F_ALL_KEYS | F_TEXT);
        assert_eq!(enc_texto(&t, "a", "a").as_deref(), Some("\x1b[97;97u"));
        // 'é' = U+00E9 = 233: a tecla e o texto são o mesmo codepoint
        assert_eq!(enc_texto(&t, "é", "é").as_deref(), Some("\x1b[233;233u"));
        // texto com mais de um codepoint sai separado por ':'
        assert_eq!(enc_texto(&t, "a", "aé").as_deref(), Some("\x1b[97;97:233u"));
        // sem a flag 16 o texto não viaja junto
        t.feed(b"\x1b[=16;3u");
        assert_eq!(t.kbd_flags, F_ALL_KEYS);
        assert_eq!(enc_texto(&t, "a", "a").as_deref(), Some("\x1b[97u"));
        // e com as flags nenhuma o texto é o caminho legado de sempre
        t.feed(b"\x1b[=8;3u");
        assert_eq!(t.kbd_flags, 0);
        assert_eq!(enc_texto(&t, "a", "a").as_deref(), Some("a"));
    }
}
