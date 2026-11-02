//! Mouse: evento da janela -> bytes do protocolo que o app espera.
//!
//! Puro (só inteiros e bytes), como o `bzmux/mouse.py`: quem sabe se o app
//! pediu mouse é o `Term` (`?1000`/`?1002`/`?1003`/`?1006`/`?1005`); aqui mora
//! só o formato.
//!
//! Protocolo xterm:
//!
//! ```text
//! 1000  press/release de botão
//! 1002  + movimento com botão apertado
//! 1003  + movimento sem botão
//! 1005  coordenada como UTF-8 (sai do limite de 223 do byte único)
//! 1006  SGR: `CSI < b ; x ; y M` (press) / `m` (release), coordenada decimal
//! normal (sem 1006/1005): `CSI M` + 3 bytes (b+32, x+32, y+32); release é
//!       sempre "botão 3" e coordenada acima de 222 não cabe no byte.
//! ```

/// Botão esquerdo.
pub const LEFT: u8 = 0;
/// Botão do meio.
pub const MIDDLE: u8 = 1;
/// Botão direito.
pub const RIGHT: u8 = 2;
/// "Release" no protocolo antigo (e o código do movimento sem botão).
pub const RELEASE: u8 = 3;
/// Roda para cima.
pub const WHEEL_UP: u8 = 64;
/// Roda para baixo.
pub const WHEEL_DOWN: u8 = 65;
/// Roda para a esquerda.
pub const WHEEL_LEFT: u8 = 66;
/// Roda para a direita.
pub const WHEEL_RIGHT: u8 = 67;
/// Botão "voltar".
pub const BACK: u8 = 128;
/// Botão "avançar".
pub const FORWARD: u8 = 129;

/// Shift no código do botão.
pub const MOD_SHIFT: u8 = 4;
/// Alt no código do botão.
pub const MOD_ALT: u8 = 8;
/// Ctrl no código do botão.
pub const MOD_CTRL: u8 = 16;
/// Marca de movimento no código do botão.
pub const MOD_MOTION: u8 = 32;

/// A roda é só press no protocolo antigo (não existe release lá).
#[inline]
pub fn is_wheel(button: u8) -> bool {
    (WHEEL_UP..=WHEEL_RIGHT).contains(&button)
}

/// Botão do protocolo a partir do botão do winit.
pub fn from_winit(button: winit::event::MouseButton) -> Option<u8> {
    use winit::event::MouseButton::{Back, Forward, Left, Middle, Other, Right};
    match button {
        Left => Some(LEFT),
        Middle => Some(MIDDLE),
        Right => Some(RIGHT),
        Back => Some(BACK),
        Forward => Some(FORWARD),
        Other(n) if n >= 4 => Some(BACK),
        Other(_) => None,
    }
}

/// Qual dialeto o app pediu (`?1006` SGR, `?1005` UTF-8).
#[derive(Copy, Clone, Default)]
pub struct Proto {
    pub sgr: bool,
    pub utf8: bool,
}

/// Um evento de mouse já traduzido para o vocabulário do protocolo.
#[derive(Copy, Clone)]
pub struct Ev {
    pub button: u8,
    /// 0-based; vira 1-based no byte.
    pub col: usize,
    /// 0-based; vira 1-based no byte.
    pub row: usize,
    pub pressed: bool,
    pub motion: bool,
    /// Bits `MOD_*`.
    pub mods: u8,
}

/// Codifica um evento. Devolve vazio quando o evento não deve ir (release de
/// roda no protocolo antigo, que não existe lá).
pub fn encode(ev: Ev, proto: Proto) -> Vec<u8> {
    let Ev {
        button,
        col,
        row,
        pressed,
        motion,
        mods,
    } = ev;
    let Proto { sgr, utf8 } = proto;
    let mods = mods & (MOD_SHIFT | MOD_ALT | MOD_CTRL);
    if !pressed && !sgr && is_wheel(button) {
        return Vec::new();
    }
    let mut b = button | mods;
    if motion {
        b |= MOD_MOTION;
    }
    if sgr {
        return format!(
            "\x1b[<{b};{};{}{}",
            col + 1,
            row + 1,
            if pressed { 'M' } else { 'm' }
        )
        .into_bytes();
    }
    // protocolo antigo: release é sempre 3 (o app deduz o botão do press)
    if !pressed {
        b = RELEASE | mods;
    }
    let mut out = Vec::with_capacity(6 + 3);
    out.extend_from_slice(b"\x1b[M");
    // no byte único a coordenada satura em 222; em UTF-8 (?1005) cabe até 2014
    let max = if utf8 { 2014 } else { 222 };
    let coord = |v: usize, out: &mut Vec<u8>| {
        let v = (v.min(max) + 33) as u32;
        if utf8 {
            let ch = char::from_u32(v).unwrap_or('\u{7f}');
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        } else {
            out.push(u8::try_from(v & 0xff).unwrap_or(b'?'));
        }
    };
    out.push(32u8.wrapping_add(b));
    coord(col, &mut out);
    coord(row, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bzmux_core::term::Term;

    fn ev(button: u8, col: usize, row: usize) -> Ev {
        Ev {
            button,
            col,
            row,
            pressed: true,
            motion: false,
            mods: 0,
        }
    }

    fn sgr() -> Proto {
        Proto {
            sgr: true,
            utf8: false,
        }
    }

    fn old() -> Proto {
        Proto::default()
    }

    #[test]
    fn sgr_press_e_release() {
        assert_eq!(encode(ev(LEFT, 9, 4), sgr()), b"\x1b[<0;10;5M");
        let mut e = ev(RIGHT, 0, 0);
        e.pressed = false;
        assert_eq!(encode(e, sgr()), b"\x1b[<2;1;1m");
    }

    #[test]
    fn sgr_com_modificadores_e_movimento() {
        // shift+ctrl+alt = 4+16+8 = 28; movimento = 32
        let mut e = ev(MIDDLE, 0, 0);
        e.mods = MOD_SHIFT | MOD_CTRL | MOD_ALT;
        e.motion = true;
        assert_eq!(encode(e, sgr()), b"\x1b[<61;1;1M");
    }

    #[test]
    fn antigo_release_vira_tres_e_coordenada_satura() {
        // release do esquerdo no protocolo antigo: botão 3 (não 0)
        let mut e = ev(LEFT, 0, 0);
        e.pressed = false;
        assert_eq!(
            encode(e, old()),
            vec![0x1b, b'[', b'M', 32 + RELEASE, 33, 33]
        );
        // coluna 500 não cabe no byte: satura em 222 (+33 = 255)
        assert_eq!(
            encode(ev(LEFT, 500, 500), old()),
            vec![0x1b, b'[', b'M', 32, 255, 255]
        );
    }

    #[test]
    fn roda_no_protocolo_antigo_nao_manda_release() {
        let mut released = ev(WHEEL_UP, 1, 1);
        released.pressed = false;
        let vazio = encode(released, old());
        assert_eq!(vazio.len(), 0);
        assert_eq!(
            encode(ev(WHEEL_UP, 0, 0), old()),
            vec![0x1b, b'[', b'M', 32 + WHEEL_UP, 33, 33]
        );
        // no SGR manda os dois
        assert_ne!(encode(released, sgr()).len(), 0);
    }

    #[test]
    fn utf8_codifica_coordenada_acima_de_222() {
        let v = encode(
            ev(LEFT, 300, 0),
            Proto {
                sgr: false,
                utf8: true,
            },
        );
        // 300+33 = 333 = U+014D -> dois bytes em UTF-8 (y=1 vira 1 byte)
        assert_eq!(v.len(), 3 + 1 + 2 + 1);
        assert_eq!(&v[4..6], &[0xc5, 0x8d]);
        assert_eq!(v[6], 33);
    }

    // -- N3: o check `mouse_modes_and_sgr()` do `checks.py` ------------------

    /// O dialeto que o app monta a partir do que o filho pediu no `Term`.
    fn proto(t: &Term) -> Proto {
        Proto {
            sgr: t.mouse_sgr,
            utf8: t.mouse_utf8,
        }
    }

    #[test]
    fn modos_do_term_e_o_byte_de_cada_evento() {
        let mut t = Term::new(80, 24);
        // `?1002` reporta press/release e arrasto com botão; `?1006` pede SGR
        t.feed(b"\x1b[?1002h\x1b[?1006h");
        assert_eq!((t.mouse_mode, t.mouse_sgr, t.mouse_utf8), (2, true, false));
        // coordenada é 0-based no evento e 1-based no byte
        assert_eq!(encode(ev(LEFT, 0, 0), proto(&t)), b"\x1b[<0;1;1M");
        let mut release = ev(LEFT, 4, 2);
        release.pressed = false;
        assert_eq!(encode(release, proto(&t)), b"\x1b[<0;5;3m");
        // arrasto com o botão apertado (?1002): bit 32 no código do botão
        let mut drag = ev(LEFT, 4, 2);
        drag.motion = true;
        assert_eq!(encode(drag, proto(&t)), b"\x1b[<32;5;3M");
        // `?1003` reporta movimento sem botão: botão 3 + 32 = 35
        t.feed(b"\x1b[?1003h");
        assert_eq!(t.mouse_mode, 3);
        let mut hover = ev(RELEASE, 7, 1);
        hover.motion = true;
        assert_eq!(encode(hover, proto(&t)), b"\x1b[<35;8;2M");
        // o app desliga: mouse_mode 0 é o que faz o app voltar à seleção (o
        // encoder é puro — quem decide se manda é o `app.rs`)
        t.feed(b"\x1b[?1003l\x1b[?1002l\x1b[?1000l");
        assert_eq!(t.mouse_mode, 0);
        // `?1006l` volta ao X10: `CSI M` + 3 bytes, release é sempre 3
        t.feed(b"\x1b[?1006l");
        assert!(!t.mouse_sgr);
        assert_eq!(
            encode(ev(LEFT, 0, 0), proto(&t)),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
    }

    #[test]
    fn roda_e_modificadores_em_cada_protocolo() {
        // no SGR a roda tem release, e ele sai em minúsculo
        assert_eq!(encode(ev(WHEEL_UP, 9, 4), sgr()), b"\x1b[<64;10;5M");
        let mut baixo = ev(WHEEL_DOWN, 9, 4);
        baixo.pressed = false;
        assert_eq!(encode(baixo, sgr()), b"\x1b[<65;10;5m");
        // ctrl+shift+roda: 64 | 16 | 4 = 84
        let mut mods = ev(WHEEL_UP, 0, 0);
        mods.mods = MOD_SHIFT | MOD_CTRL;
        assert_eq!(encode(mods, sgr()), b"\x1b[<84;1;1M");
        // no protocolo antigo o modificador entra no byte do botão, e o
        // release da roda não existe: o app vê só o press
        assert_eq!(encode(mods, old()), vec![0x1b, b'[', b'M', 32 + 84, 33, 33]);
        let mut solta = ev(WHEEL_UP, 0, 0);
        solta.pressed = false;
        assert_eq!(encode(solta, old()), Vec::<u8>::new());
        // release do esquerdo com Alt no antigo: 3 | 8 = 11
        let mut alt = ev(LEFT, 0, 0);
        alt.mods = MOD_ALT;
        alt.pressed = false;
        assert_eq!(encode(alt, old()), vec![0x1b, b'[', b'M', 32 + 11, 33, 33]);
    }

    #[test]
    fn sgr_ganha_do_utf8_e_botoes_laterais() {
        let mut t = Term::new(80, 24);
        t.feed(b"\x1b[?1000h\x1b[?1005h\x1b[?1006h");
        assert_eq!((t.mouse_mode, t.mouse_utf8, t.mouse_sgr), (1, true, true));
        // com os dois ligados o SGR (decimal) tem precedência: 300 vira 301
        assert_eq!(encode(ev(LEFT, 300, 0), proto(&t)), b"\x1b[<0;301;1M");
        // botão "voltar"/"avançar" tem código próprio (128/129)
        assert_eq!(encode(ev(BACK, 2, 0), proto(&t)), b"\x1b[<128;3;1M");
        assert_eq!(encode(ev(FORWARD, 2, 0), proto(&t)), b"\x1b[<129;3;1M");
        // sem o SGR o X10 manda 32+128 = 160 no byte do botão
        t.feed(b"\x1b[?1006l");
        assert_eq!(
            encode(ev(BACK, 2, 0), proto(&t)),
            vec![0x1b, b'[', b'M', 160, 35, 33]
        );
    }
}
