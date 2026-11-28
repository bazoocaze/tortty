//! Mouse: window event -> bytes of the protocol the app expects.
//!
//! Pure (integers and bytes only): `Term` is the one that
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

/// The wheel is press only: xterm has no release for it, and a TUI that counts
/// every `<64;x;yM` scrolls twice as far when it also gets the `<64;x;ym`.
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

/// Which dialect the app asked for (`?1006` SGR, `?1005` UTF-8, `?1016`
/// pixels).
#[derive(Copy, Clone, Default)]
pub struct Proto {
    pub sgr: bool,
    pub utf8: bool,
    /// `?1016`: the SGR shape, with the coordinates in pixels. The caller fills
    /// `col`/`row` with pixels instead of cells.
    pub pixels: bool,
}

/// A mouse event already translated into the protocol vocabulary.
#[derive(Copy, Clone)]
pub struct Ev {
    pub button: u8,
    /// 0-based; becomes 1-based in the byte. Pixels when `Proto::pixels`.
    pub col: usize,
    /// 0-based; becomes 1-based in the byte. Pixels when `Proto::pixels`.
    pub row: usize,
    pub pressed: bool,
    pub motion: bool,
    /// Bits `MOD_*`.
    pub mods: u8,
}

/// Encodes an event. Returns empty when the event must not go out (a wheel
/// release, which no protocol has).
pub fn encode(ev: Ev, proto: Proto) -> Vec<u8> {
    let Ev {
        button,
        col,
        row,
        pressed,
        motion,
        mods,
    } = ev;
    let Proto { sgr, utf8, pixels } = proto;
    // `?1016` is the SGR form with pixel coordinates: there is no legacy pixel
    // protocol to fall back to, so it decides the shape on its own
    let sgr = sgr || pixels;
    let mods = mods & (MOD_SHIFT | MOD_ALT | MOD_CTRL);
    if !pressed && is_wheel(button) {
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

/// Lines of scroll one wheel event is worth, before the whole-line rounding
/// the caller accumulates (`B86`): the detent is whole and pays the D4's 3
/// lines (its press/release twin, ~1 ms later and same direction, is dropped);
/// a touchpad travel pays its own factor per unit (`TOUCHPAD_LINES`). Because
/// the accumulation happens in lines and not in notches, a small movement
/// still reaches one line: the fraction the finger asked for is not rounded
/// up to a multi-line step.
// an exact compare on purpose: winit turns the wheel buttons into exactly
// ±1.0, and a touchpad never lands there
#[allow(clippy::float_cmp)]
pub fn scroll_lines(pair: &mut Option<(std::time::Instant, i8)>, travel: f32) -> f32 {
    if travel.abs() == 1.0 {
        let dir: i8 = if travel > 0.0 { 1 } else { -1 };
        let now = std::time::Instant::now();
        let repeated = pair.is_some_and(|(when, previous)| {
            previous == dir && now.duration_since(when).as_millis() < 10
        });
        *pair = Some((now, dir));
        if repeated {
            return 0.0;
        }
        // the wheel notch: the D4 factor — the touchpad's own factor is below
        return travel * 3.0;
    }
    travel * TOUCHPAD_LINES
}

/// Lines of scroll one unit of touchpad travel is worth. Measured choice of
/// the owner (2026-11-27): the family band was 150-250 lines per swipe with
/// 3 and 4 landed inside it and felt right — the detent keeps the D4's 3.
const TOUCHPAD_LINES: f32 = 4.0;

/// Notches the finger asked for, from its travel.
///
/// Every surface where we own the wheel needs a *count*, and one event is not
/// one notch: a touchpad delivers fractions of a detent at up to ~37 events/s
/// (measured on 2026-11-04), so the fraction accumulates here and one detent of
/// travel buys one notch — for the child that reports mouse (`B40`) and for the
/// tab bar that cycles a tab per notch. A detent — exactly ±1.0 — never
/// comes through here: it is already a whole notch, and the caller sends it
/// without touching the remainder. A direction change drops the remainder,
/// which has the wrong sign and would otherwise eat the first event of the new
/// direction (kitty does the same with its V120 carry).
pub fn notches(frac: &mut f32, lines: f32) -> i32 {
    if lines == 0.0 || !lines.is_finite() {
        return 0;
    }
    if *frac * lines < 0.0 {
        *frac = 0.0;
    }
    *frac += lines;
    let n = frac.trunc();
    if n == 0.0 {
        return 0;
    }
    *frac -= n;
    // bounded by construction: the remainder is always below one detent
    n as i32
}

/// Where a drag on the tab bar takes the tab, and where the press point goes
/// with it: `(destination, travel consumed)`.
///
/// A tab only changes place after half of its own label (otherwise it wobbles),
/// and the press point travels with it. That second half is the difference
/// between reordering and a runaway: with the press point fixed, the same travel
/// is measured again by every following motion event, so a hand that barely
/// moved walks the tab to the end of the bar (2026-11-07). What the anchor carries is
/// the distance the tab **actually** moved, never what the finger asked — at the
/// end of the bar the two differ, and a clamped move must not leave a debt.
pub fn drag_crossed(i: usize, len: usize, step: f64, dx: f64) -> (usize, f64) {
    if i >= len || !step.is_finite() || step <= 0.0 || !dx.is_finite() {
        return (i, 0.0);
    }
    let n = (dx / step).round();
    if n == 0.0 {
        return (i, 0.0);
    }
    // the same cast the drag did before: the travel is bounded by the bar
    let n = n as i64;
    let dest = if n > 0 {
        (i + n as usize).min(len - 1)
    } else {
        i.saturating_sub(n.unsigned_abs() as usize)
    };
    (dest, (dest as f64 - i as f64) * step)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tortty_core::term::Term;

    #[test]
    fn a_drag_moves_one_tab_per_label_and_the_anchor_goes_with_it() {
        // a 1 px shake does not reorder anything
        assert_eq!(drag_crossed(1, 3, 80.0, 1.0), (1, 0.0));
        assert_eq!(drag_crossed(1, 3, 80.0, -1.0), (1, 0.0));
        // half of the label is the threshold, and the anchor moves one label:
        // the next event measures what is left, not the same travel again
        assert_eq!(drag_crossed(1, 3, 80.0, 40.0), (2, 80.0));
        assert_eq!(drag_crossed(2, 3, 80.0, 42.0 - 80.0), (2, 0.0));
        assert_eq!(drag_crossed(2, 3, 80.0, -45.0), (1, -80.0));
        // two labels in one motion move two, and the anchor keeps only the rest
        assert_eq!(drag_crossed(0, 4, 80.0, 170.0), (2, 160.0));
        // at the end of the bar the tab stops and leaves no debt behind
        assert_eq!(drag_crossed(1, 2, 80.0, 1000.0), (1, 0.0));
        assert_eq!(drag_crossed(0, 1, 80.0, 1000.0), (0, 0.0));
        // a label without width (no font yet) cannot be crossed
        assert_eq!(drag_crossed(1, 3, 0.0, 100.0), (1, 0.0));
    }

    #[test]
    fn the_detent_pays_three_the_touchpad_its_own_factor_and_a_small_move_reaches_one() {
        // the detent's press/release twin is dropped (X11 delivers both, ~1 ms
        // apart): one notch, not two — so a wheel click pays 3, not 6
        let mut pair = None;
        assert_eq!(scroll_lines(&mut pair, 1.0), 3.0);
        assert_eq!(scroll_lines(&mut pair, 1.0), 0.0, "the twin is dropped");
        assert_eq!(
            scroll_lines(&mut pair, -1.0),
            -3.0,
            "the other direction passes"
        );
        assert_eq!(scroll_lines(&mut pair, 0.0), 0.0);
        // a touchpad fraction (B86): a share of the unit, worth its share of
        // the experiment factor — accumulated by the caller until it completes
        // a whole line
        assert_eq!(scroll_lines(&mut pair, 0.2), 0.2 * TOUCHPAD_LINES);
        assert_eq!(scroll_lines(&mut pair, -0.1), -0.1 * TOUCHPAD_LINES);
        // three small moves accumulate: nothing rounds them up to a 3-line
        // step (the granularity regression the owner caught)
        let mut frac = 0.0;
        for _ in 0..3 {
            frac += scroll_lines(&mut pair, 0.25);
        }
        assert_eq!(
            frac,
            0.75 * TOUCHPAD_LINES,
            "small movements accumulate, line by line"
        );
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
            pixels: false,
        }
    }

    #[test]
    fn pixel_mode_keeps_the_sgr_shape_and_the_raw_numbers() {
        // ?1016 alone, without ?1006: the shape is still SGR and the numbers
        // are pixels, not cells
        let px = Proto {
            sgr: false,
            utf8: false,
            pixels: true,
        };
        assert_eq!(encode(ev(LEFT, 123, 456), px), b"\x1b[<0;124;457M");
        // and the pixel coordinate is not pushed into the legacy byte path
        assert_eq!(encode(ev(LEFT, 3000, 2000), px), b"\x1b[<0;3001;2001M");
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
    fn a_wheel_release_goes_out_in_no_protocol() {
        let mut released = ev(WHEEL_UP, 1, 1);
        released.pressed = false;
        // a TUI that counts every mouse report scrolled twice as far while the
        // SGR release came out too (B40)
        assert_eq!(encode(released, old()).len(), 0);
        assert_eq!(encode(released, sgr()).len(), 0);
        assert_eq!(
            encode(ev(WHEEL_UP, 0, 0), old()),
            vec![0x1b, b'[', b'M', 32 + WHEEL_UP, 33, 33]
        );
    }

    #[test]
    fn a_touchpad_buys_one_notch_per_detent_of_travel() {
        let mut frac = 0.0;
        // the measured gesture: fractions of a detent, one event each, and none
        // of them is worth a notch on its own
        assert_eq!(notches(&mut frac, 0.4), 0);
        assert_eq!(notches(&mut frac, 0.3), 0);
        assert_eq!(notches(&mut frac, 0.4), 1, "1.1 detents is one notch");
        assert!((frac - 0.1).abs() < 0.001, "and a tenth is carried: {frac}");
        // a fast flick carries two or three detents in a single event
        assert_eq!(notches(&mut frac, 2.06), 2);
        assert_eq!(notches(&mut frac, 3.5), 3);
    }

    #[test]
    fn a_direction_change_drops_the_carried_fraction() {
        let mut frac = 0.0;
        assert_eq!(notches(&mut frac, 0.9), 0);
        // backwards: the 0.9 in the other direction would eat the first event
        assert_eq!(notches(&mut frac, -0.2), 0);
        assert_eq!(notches(&mut frac, -0.8), -1, "the whole detent counts");
        assert!((frac + 0.0).abs() < 0.001, "nothing left over: {frac}");
    }

    #[test]
    fn no_delta_and_no_nonsense_buy_nothing() {
        let mut frac = 0.0;
        assert_eq!(notches(&mut frac, 0.0), 0);
        assert_eq!(notches(&mut frac, f32::NAN), 0);
        assert_eq!(notches(&mut frac, f32::INFINITY), 0);
        assert_eq!(frac, 0.0);
    }

    #[test]
    fn utf8_encodes_coord_above_222() {
        let v = encode(
            ev(LEFT, 300, 0),
            Proto {
                sgr: false,
                utf8: true,
                pixels: false,
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
            pixels: t.mouse_px,
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
        // in SGR the wheel goes out as a press and never as a release (B40)
        assert_eq!(encode(ev(WHEEL_UP, 9, 4), sgr()), b"\x1b[<64;10;5M");
        let mut low = ev(WHEEL_DOWN, 9, 4);
        low.pressed = false;
        assert_eq!(encode(low, sgr()), Vec::<u8>::new());
        // ctrl+shift+wheel: 64 | 16 | 4 = 84
        let mut mods = ev(WHEEL_UP, 0, 0);
        mods.mods = MOD_SHIFT | MOD_CTRL;
        assert_eq!(encode(mods, sgr()), b"\x1b[<84;1;1M");
        // in the old protocol the modifier enters the button byte, and the
        // wheel release does not exist in any protocol: the app only sees the
        // press
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
