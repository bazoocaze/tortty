//! Mouse: window event -> bytes of the protocol the app expects.
//!
//! Pure (integers and bytes only), like `bzmux/mouse.py`: `Term` is the one that
//! knows whether the app asked for mouse (`?1000`/`?1002`/`?1003`/`?1006`/`?1005`);
//! only the format lives here.
//!
//! xterm protocol:
//!
//! ```text
//! 1000  button press/release
//! 1002  + motion with the button held down
//! 1003  + motion with no button
//! 1005  coordinate as UTF-8 (escapes the 223 limit of a single byte)
//! 1006  SGR: `CSI < b ; x ; y M` (press) / `m` (release), decimal coordinate
//! normal (without 1006/1005): `CSI M` + 3 bytes (b+32, x+32, y+32); release is
//!       always "button 3" and a coordinate above 222 does not fit the byte.
//! ```

/// Left button.
pub const LEFT: u8 = 0;
/// Middle button.
pub const MIDDLE: u8 = 1;
/// Right button.
pub const RIGHT: u8 = 2;
/// "Release" in the old protocol (and the code for motion with no button).
pub const RELEASE: u8 = 3;
/// Wheel up.
pub const WHEEL_UP: u8 = 64;
/// Wheel down.
pub const WHEEL_DOWN: u8 = 65;
/// Wheel left.
pub const WHEEL_LEFT: u8 = 66;
/// Wheel right.
pub const WHEEL_RIGHT: u8 = 67;
/// "Back" button.
pub const BACK: u8 = 128;
/// "Forward" button.
pub const FORWARD: u8 = 129;

/// Shift in the button code.
pub const MOD_SHIFT: u8 = 4;
/// Alt in the button code.
pub const MOD_ALT: u8 = 8;
/// Ctrl in the button code.
pub const MOD_CTRL: u8 = 16;
/// Motion mark in the button code.
pub const MOD_MOTION: u8 = 32;

/// The wheel is press only in the old protocol (there is no release there).
#[inline]
pub fn is_wheel(button: u8) -> bool {
    (WHEEL_UP..=WHEEL_RIGHT).contains(&button)
}

/// Protocol button from the winit button.
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

/// Which dialect the app asked for (`?1006` SGR, `?1005` UTF-8).
#[derive(Copy, Clone, Default)]
pub struct Proto {
    pub sgr: bool,
    pub utf8: bool,
}

/// A mouse event already translated into the protocol vocabulary.
#[derive(Copy, Clone)]
pub struct Ev {
    pub button: u8,
    /// 0-based; becomes 1-based in the byte.
    pub col: usize,
    /// 0-based; becomes 1-based in the byte.
    pub row: usize,
    pub pressed: bool,
    pub motion: bool,
    /// Bits `MOD_*`.
    pub mods: u8,
}

/// Encodes an event. Returns empty when the event must not go out (wheel
/// release in the old protocol, which does not exist there).
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
    // old protocol: release is always 3 (the app infers the button from press)
    if !pressed {
        b = RELEASE | mods;
    }
    let mut out = Vec::with_capacity(6 + 3);
    out.extend_from_slice(b"\x1b[M");
    // in the single byte the coordinate saturates at 222; in UTF-8 (?1005) it
    // fits up to 2014
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

/// Lines scrolled by one axis of a `MouseScrollDelta::LineDelta`.
///
/// A wheel notch arrives as exactly ±1.0 and is worth 3 lines (D4, the same as
/// the Python bzmux). A touchpad arrives as a *fraction* of a line — the X11
/// smooth valuators divided by the scroll increment — and there the fraction is
/// the distance the finger asked for, so it is kept as sent (the rest
/// accumulates in `scroll_frac`).
// an exact compare on purpose: winit turns the wheel buttons into exactly ±1.0,
// and a touchpad never lands there
#[allow(clippy::float_cmp)]
pub fn wheel_lines(v: f32) -> f32 {
    if v.abs() == 1.0 {
        v * 3.0
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tortty_core::term::Term;

    #[test]
    fn a_wheel_notch_is_three_lines_and_a_touchpad_fraction_is_kept() {
        // wheel notch: the buttons 4/5 of X11 arrive as exactly ±1.0
        assert_eq!(wheel_lines(1.0), 3.0);
        assert_eq!(wheel_lines(-1.0), -3.0);
        assert_eq!(wheel_lines(0.0), 0.0);
        // touchpad: the fraction is the distance, and the tab accumulates it
        assert_eq!(wheel_lines(0.4), 0.4);
        assert_eq!(wheel_lines(-0.7), -0.7);
        assert_eq!(wheel_lines(2.5), 2.5);
    }

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
    fn sgr_press_and_release() {
        assert_eq!(encode(ev(LEFT, 9, 4), sgr()), b"\x1b[<0;10;5M");
        let mut e = ev(RIGHT, 0, 0);
        e.pressed = false;
        assert_eq!(encode(e, sgr()), b"\x1b[<2;1;1m");
    }

    #[test]
    fn sgr_with_modifiers_and_movement() {
        // shift+ctrl+alt = 4+16+8 = 28; motion = 32
        let mut e = ev(MIDDLE, 0, 0);
        e.mods = MOD_SHIFT | MOD_CTRL | MOD_ALT;
        e.motion = true;
        assert_eq!(encode(e, sgr()), b"\x1b[<61;1;1M");
    }

    #[test]
    fn legacy_release_becomes_three_and_coord_saturates() {
        // left release in the old protocol: button 3 (not 0)
        let mut e = ev(LEFT, 0, 0);
        e.pressed = false;
        assert_eq!(
            encode(e, old()),
            vec![0x1b, b'[', b'M', 32 + RELEASE, 33, 33]
        );
        // column 500 does not fit the byte: it saturates at 222 (+33 = 255)
        assert_eq!(
            encode(ev(LEFT, 500, 500), old()),
            vec![0x1b, b'[', b'M', 32, 255, 255]
        );
    }

    #[test]
    fn wheel_in_legacy_protocol_sends_no_release() {
        let mut released = ev(WHEEL_UP, 1, 1);
        released.pressed = false;
        let empty = encode(released, old());
        assert_eq!(empty.len(), 0);
        assert_eq!(
            encode(ev(WHEEL_UP, 0, 0), old()),
            vec![0x1b, b'[', b'M', 32 + WHEEL_UP, 33, 33]
        );
        // in SGR both go out
        assert_ne!(encode(released, sgr()).len(), 0);
    }

    #[test]
    fn utf8_encodes_coord_above_222() {
        let v = encode(
            ev(LEFT, 300, 0),
            Proto {
                sgr: false,
                utf8: true,
            },
        );
        // 300+33 = 333 = U+014D -> two bytes in UTF-8 (y=1 becomes 1 byte)
        assert_eq!(v.len(), 3 + 1 + 2 + 1);
        assert_eq!(&v[4..6], &[0xc5, 0x8d]);
        assert_eq!(v[6], 33);
    }

    // -- N3: the `mouse_modes_and_sgr()` check of `checks.py` ----------------

    /// The dialect the app builds from what the child asked for in `Term`.
    fn proto(t: &Term) -> Proto {
        Proto {
            sgr: t.mouse_sgr,
            utf8: t.mouse_utf8,
        }
    }

    #[test]
    fn term_modes_and_the_byte_of_each_event() {
        let mut t = Term::new(80, 24);
        // `?1002` reports press/release and drag with the button; `?1006` asks
        // for SGR
        t.feed(b"\x1b[?1002h\x1b[?1006h");
        assert_eq!((t.mouse_mode, t.mouse_sgr, t.mouse_utf8), (2, true, false));
        // the coordinate is 0-based in the event and 1-based in the byte
        assert_eq!(encode(ev(LEFT, 0, 0), proto(&t)), b"\x1b[<0;1;1M");
        let mut release = ev(LEFT, 4, 2);
        release.pressed = false;
        assert_eq!(encode(release, proto(&t)), b"\x1b[<0;5;3m");
        // drag with the button held down (?1002): bit 32 in the button code
        let mut drag = ev(LEFT, 4, 2);
        drag.motion = true;
        assert_eq!(encode(drag, proto(&t)), b"\x1b[<32;5;3M");
        // `?1003` reports motion with no button: button 3 + 32 = 35
        t.feed(b"\x1b[?1003h");
        assert_eq!(t.mouse_mode, 3);
        let mut hover = ev(RELEASE, 7, 1);
        hover.motion = true;
        assert_eq!(encode(hover, proto(&t)), b"\x1b[<35;8;2M");
        // the app turns it off: mouse_mode 0 is what makes the app go back to
        // selection (the encoder is pure — `app.rs` decides whether to send)
        t.feed(b"\x1b[?1003l\x1b[?1002l\x1b[?1000l");
        assert_eq!(t.mouse_mode, 0);
        // `?1006l` goes back to X10: `CSI M` + 3 bytes, release is always 3
        t.feed(b"\x1b[?1006l");
        assert!(!t.mouse_sgr);
        assert_eq!(
            encode(ev(LEFT, 0, 0), proto(&t)),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
    }

    #[test]
    fn wheel_and_modifiers_in_each_protocol() {
        // in SGR the wheel has a release, and it goes out lowercase
        assert_eq!(encode(ev(WHEEL_UP, 9, 4), sgr()), b"\x1b[<64;10;5M");
        let mut low = ev(WHEEL_DOWN, 9, 4);
        low.pressed = false;
        assert_eq!(encode(low, sgr()), b"\x1b[<65;10;5m");
        // ctrl+shift+wheel: 64 | 16 | 4 = 84
        let mut mods = ev(WHEEL_UP, 0, 0);
        mods.mods = MOD_SHIFT | MOD_CTRL;
        assert_eq!(encode(mods, sgr()), b"\x1b[<84;1;1M");
        // in the old protocol the modifier enters the button byte, and the
        // wheel release does not exist: the app only sees the press
        assert_eq!(encode(mods, old()), vec![0x1b, b'[', b'M', 32 + 84, 33, 33]);
        let mut release_key = ev(WHEEL_UP, 0, 0);
        release_key.pressed = false;
        assert_eq!(encode(release_key, old()), Vec::<u8>::new());
        // left release with Alt in the old one: 3 | 8 = 11
        let mut alt = ev(LEFT, 0, 0);
        alt.mods = MOD_ALT;
        alt.pressed = false;
        assert_eq!(encode(alt, old()), vec![0x1b, b'[', b'M', 32 + 11, 33, 33]);
    }

    #[test]
    fn sgr_wins_over_utf8_and_side_buttons() {
        let mut t = Term::new(80, 24);
        t.feed(b"\x1b[?1000h\x1b[?1005h\x1b[?1006h");
        assert_eq!((t.mouse_mode, t.mouse_utf8, t.mouse_sgr), (1, true, true));
        // with both on, SGR (decimal) takes precedence: 300 becomes 301
        assert_eq!(encode(ev(LEFT, 300, 0), proto(&t)), b"\x1b[<0;301;1M");
        // the "back"/"forward" button has its own code (128/129)
        assert_eq!(encode(ev(BACK, 2, 0), proto(&t)), b"\x1b[<128;3;1M");
        assert_eq!(encode(ev(FORWARD, 2, 0), proto(&t)), b"\x1b[<129;3;1M");
        // without SGR, X10 sends 32+128 = 160 in the button byte
        t.feed(b"\x1b[?1006l");
        assert_eq!(
            encode(ev(BACK, 2, 0), proto(&t)),
            vec![0x1b, b'[', b'M', 160, 35, 33]
        );
    }
}
