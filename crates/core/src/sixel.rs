//! Sixel (`B3`): the bitmap protocol `chafa -f sixel`, `img2sixel` and `lsix`
//! speak. It arrives inside a DCS — `ESC P <p1>;<p2>;<p3> q <body> ST` — and the
//! body is a run of six-pixel-tall bands: every character paints the six pixels
//! above it, one bit each, in the colour of the register `#` selected.
//!
//! The decoder is pure arithmetic — a palette and a bitmap, no zlib, no clock —
//! so it lives here and the app calls it, the same way it calls the PNG one it
//! owns. `decode` never trusts the stream for its own bounds: the raster is the
//! one `size` measured, and a `!` run past its edge stops at the edge.

/// The three parameters a sixel DCS carries before its `q`.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Params {
    /// `P1`: the pixel aspect hint (2:1, 5:1, 3:1). Parsed and kept, not applied:
    /// a sixel pixel is drawn square, which is what a producer that does not set
    /// it expects, and stretching is the one thing that would make a picture look
    /// wrong on a display nobody remembers.
    pub aspect: u16,
    /// `P2`: `0` the terminal's background behind the picture, `1` transparent,
    /// `2` leave what is there (we do the same as `0`, since nothing reads back a
    /// framebuffer).
    pub background: u8,
    /// `P3`: the horizontal grid, for a producer that draws on paper. Unused.
    pub grid: u16,
}

/// One picture, ready to become a texture.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Splits a DCS body into its parameters and its sixel data. `None` when the DCS
/// is not a sixel at all — `DECRQSS` is `DCS $ q …`, whose `q` does not follow a
/// run of digits, so it stays out.
#[must_use]
pub fn split(dcs: &[u8]) -> Option<(Params, &[u8])> {
    let end = dcs
        .iter()
        .position(|&b| !(b.is_ascii_digit() || b == b';'))?;
    if dcs.get(end) != Some(&b'q') {
        return None;
    }
    let mut p = [0u16; 3];
    for (slot, part) in p.iter_mut().zip(split_params(&dcs[..end])) {
        *slot = part;
    }
    #[allow(clippy::cast_possible_truncation)]
    let params = Params {
        aspect: p[0],
        background: p[1].min(255) as u8,
        grid: p[2],
    };
    Some((params, &dcs[end + 1..]))
}

/// The params are at most three numbers, so a `[u16; 3]` is the whole story and
/// nothing needs to allocate to read them.
fn split_params(text: &[u8]) -> impl Iterator<Item = u16> + '_ {
    text.split(|&b| b == b';').map(|part| {
        let mut n = 0u16;
        for &b in part {
            n = n.saturating_mul(10).saturating_add(u16::from(b - b'0'));
        }
        n
    })
}

/// The pixel size the stream describes: as wide as its widest band, six pixels
/// tall per band.
#[must_use]
pub fn size(body: &[u8]) -> Option<(u32, u32)> {
    let (mut x, mut width) = (0u32, 0u32);
    let (mut band, mut bands) = (0u32, 0u32);
    let mut i = 0;
    while i < body.len() {
        match token(body, &mut i) {
            Token::Data(bits) => step(&mut x, &mut width, &mut bands, band, 1, bits),
            Token::Repeat(n, bits) => {
                #[allow(clippy::cast_possible_truncation)]
                step(&mut x, &mut width, &mut bands, band, n as u32, bits);
            },
            Token::Carriage => x = 0,
            Token::Band => {
                band = band.saturating_add(1);
                x = 0;
            },
            Token::Colour(..) | Token::Ignore => {},
        }
    }
    let height = bands.checked_mul(6)?;
    (width != 0 && height != 0).then_some((width, height))
}

/// Width and the last band painted; split out so `size` stays a walk and not a
/// nest of four counters in every arm.
fn step(x: &mut u32, width: &mut u32, bands: &mut u32, band: u32, n: u32, bits: u8) {
    if bits == 0 {
        *x = x.saturating_add(n);
        return;
    }
    *x = x.saturating_add(n);
    *width = (*width).max(*x);
    *bands = (*bands).max(band.saturating_add(1));
}

/// Bytes → RGBA8. `under` is the colour an unset pixel gets — the terminal
/// background for `P2=0` — and `None` leaves it transparent (`P2=1`).
///
/// `raster` is the size the caller already measured (the core did, to place the
/// picture): the stream is walked once here, not twice, and a band is clamped to
/// that raster, so a claimed run of a million pixels cannot invent them. A
/// `raster` that does not match what the stream says is the caller's mistake and
/// nothing paints.
#[must_use]
pub fn decode(
    body: &[u8],
    under: Option<[u8; 3]>,
    raster: (u32, u32),
    max_px: u32,
) -> Option<Raster> {
    let (width, height) = raster;
    let px = width.checked_mul(height)?;
    if width == 0 || height == 0 || px > max_px {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let mut rgba = vec![0u8; px as usize * 4];
    if let Some([r, g, b]) = under {
        for cell in rgba.as_chunks_mut::<4>().0 {
            cell.copy_from_slice(&[r, g, b, 255]);
        }
    }
    let canvas: &mut [[u8; 4]] = rgba.as_chunks_mut::<4>().0;
    let stride = width as usize;
    let mut registers = [[0u8, 0, 0, 255]; 256];
    let mut cur = 0usize;
    let (mut x, mut band) = (0u32, 0u32);
    let mut i = 0;
    while i < body.len() {
        match token(body, &mut i) {
            Token::Colour(n, def) => {
                cur = usize::from(n);
                if let Some(c) = def {
                    registers[cur] = c;
                }
            },
            Token::Repeat(n, bits) => {
                paint(canvas, stride, height, (x, band), n, bits, registers[cur]);
                #[allow(clippy::cast_possible_truncation)]
                {
                    x = x.saturating_add(n as u32);
                }
            },
            Token::Data(bits) => {
                paint(canvas, stride, height, (x, band), 1, bits, registers[cur]);
                x = x.saturating_add(1);
            },
            Token::Carriage => x = 0,
            Token::Band => {
                band = band.saturating_add(1);
                x = 0;
            },
            Token::Ignore => {},
        }
    }
    Some(Raster {
        width,
        height,
        rgba,
    })
}

/// Paints `n` columns of six bits at `(x, band)`, stopping at the raster's edge.
/// Paints `n` columns of six bits at `(x, band)`, stopping at the raster's edge.
fn paint(
    canvas: &mut [[u8; 4]],
    stride: usize,
    height: u32,
    at: (u32, u32),
    n: usize,
    bits: u8,
    colour: [u8; 4],
) {
    if bits == 0 {
        return;
    }
    let (start, band) = at;
    #[allow(clippy::cast_possible_truncation)]
    let n = n as u32;
    let end = start.saturating_add(n).min(stride as u32);
    for x in start..end {
        #[allow(clippy::cast_possible_truncation)]
        let x = x as usize;
        for row in 0..6u32 {
            if bits & (1 << row) == 0 {
                continue;
            }
            let y = band.saturating_mul(6).saturating_add(row);
            if y >= height {
                break;
            }
            #[allow(clippy::cast_possible_truncation)]
            {
                canvas[y as usize * stride + x] = colour;
            }
        }
    }
}

/// One thing in the stream, with the index already past it.
enum Token {
    /// `#<n>` or `#<n>;2;<h>;<l>;<s>` / `#<n>;1;<h>;<l>;<s>`: select a register,
    /// and define it when the parameters are there. The colour is resolved here so
    /// the two walks agree on what a register holds.
    Colour(u8, Option<[u8; 4]>),
    /// `!<n><char>`: a run.
    Repeat(usize, u8),
    /// `$`: back to the left edge of the same band, painting over it.
    Carriage,
    /// `-`: the next band.
    Band,
    /// A data character: six pixels, one per bit.
    Data(u8),
    Ignore,
}

fn token(body: &[u8], i: &mut usize) -> Token {
    let start = *i;
    let Some(&b) = body.get(start) else {
        return Token::Ignore;
    };
    *i += 1;
    match b {
        b'#' => {
            let n = number(body, i);
            let mut def = None;
            if body.get(*i) == Some(&b';') {
                let space = number_u8(body, i, b';');
                let mut p = [0u16; 3];
                for slot in &mut p {
                    *slot = number_u8(body, i, b';');
                }
                def = match space {
                    2 => Some(rgb(p)),
                    1 => Some(hls(p)),
                    _ => None, // `#n;3` and beyond are not ours
                };
            }
            #[allow(clippy::cast_possible_truncation)]
            Token::Colour((n & 0xff) as u8, def)
        },
        b'!' => {
            let n = number(body, i);
            let Some(&c) = body.get(*i) else {
                return Token::Ignore;
            };
            *i += 1;
            Token::Repeat(n, c.wrapping_sub(0x3f))
        },
        b'$' => Token::Carriage,
        b'-' => Token::Band,
        0x3f..=0x7e => Token::Data(b - 0x3f),
        _ => Token::Ignore,
    }
}

/// A decimal number; absent digits read as zero, which is what every default in
/// the protocol is.
fn number(body: &[u8], i: &mut usize) -> usize {
    let mut n = 0usize;
    while let Some(&b) = body.get(*i) {
        if !b.is_ascii_digit() {
            break;
        }
        n = n.saturating_mul(10).saturating_add(usize::from(b - b'0'));
        *i += 1;
    }
    n
}

/// A number that follows a separator, consuming the separator first (and letting
/// a missing one read as zero, so `#1;2;50` and `#1;2;50;;` are both read).
fn number_u8(body: &[u8], i: &mut usize, sep: u8) -> u16 {
    if body.get(*i) == Some(&sep) {
        *i += 1;
    }
    u16::try_from(number(body, i)).unwrap_or(u16::MAX)
}

/// `#n;2;h;l;s`: the three components are percentages.
fn rgb(p: [u16; 3]) -> [u8; 4] {
    let one = |v: u16| -> u8 {
        let v = v.min(100);
        #[allow(clippy::cast_possible_truncation)]
        let out = (v * 255 + 50) / 100;
        out as u8
    };
    [one(p[0]), one(p[1]), one(p[2]), 255]
}

/// `#n;1;h;l;s`: hue in degrees, lightness and saturation in percent.
fn hls(p: [u16; 3]) -> [u8; 4] {
    let hue = f64::from(p[0].min(360)) / 360.0;
    let light = f64::from(p[1].min(100)) / 100.0;
    let sat = f64::from(p[2].min(100)) / 100.0;
    let q = if light < 0.5 {
        light * (1.0 + sat)
    } else {
        light + sat - light * sat
    };
    let p2 = 2.0 * light - q;
    let channel = |t: f64| -> u8 {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p2 + (q - p2) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p2 + (q - p2) * (2.0 / 3.0 - t) * 6.0
        } else {
            p2
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let out = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        out
    };
    [
        channel(hue + 1.0 / 3.0),
        channel(hue),
        channel(hue - 1.0 / 3.0),
        255,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `!` runs a character, `-` moves a band, `$` goes back to the left edge.
    #[test]
    fn a_run_carriage_and_band_give_the_raster() {
        // two bands; the second paints three full columns and one half column
        let body = b"#1;2;100;0;0!4~-!3~!1@";
        let (w, h) = size(body).expect("a raster");
        assert_eq!(
            (w, h),
            (4, 12),
            "as wide as the widest band, 6 rows per band"
        );
        let r = decode(body, Some([0, 0, 0]), (4, 12), 1000).expect("decodes");
        assert_eq!((r.width, r.height), (4, 12));
        let at = |x: u32, y: u32| -> [u8; 4] {
            #[allow(clippy::cast_possible_truncation)]
            let i = (y as usize * 4 + x as usize) * 4;
            [r.rgba[i], r.rgba[i + 1], r.rgba[i + 2], r.rgba[i + 3]]
        };
        assert_eq!(at(0, 0), [255, 0, 0, 255], "the first band is full");
        assert_eq!(at(3, 5), [255, 0, 0, 255]);
        assert_eq!(
            at(1, 6),
            [255, 0, 0, 255],
            "the second band starts at row 6"
        );
        assert_eq!(at(2, 6), [255, 0, 0, 255]);
        assert_eq!(at(3, 6), [255, 0, 0, 255], "`@` is only the top pixel");
        assert_eq!(at(3, 7), [0, 0, 0, 255], "and leaves the rest alone");
    }

    /// `$` paints the same band twice instead of starting a new one.
    #[test]
    fn a_carriage_return_overlays_the_band() {
        let body = b"#1;2;100;0;0~$#2;2;0;0;100A";
        let r = decode(body, Some([0, 0, 0]), (1, 6), 1000).expect("decodes");
        assert_eq!(r.height, 6, "`$` is not a new band");
        assert_eq!(&r.rgba[0..4], &[255, 0, 0, 255], "row 0 from `~`");
        assert_eq!(&r.rgba[4..8], &[0, 0, 255, 255], "row 1 from `A`, in blue");
    }

    /// `P2=1` leaves the unset pixels transparent, `P2=0` fills them.
    #[test]
    fn the_background_parameter_decides_what_an_unset_pixel_is() {
        let body = b"#1;2;100;0;0@";
        let opaque = decode(body, Some([9, 8, 7]), (1, 6), 1000).expect("decodes");
        assert_eq!(
            &opaque.rgba[4..8],
            &[9, 8, 7, 255],
            "row 1 is the background"
        );
        let clear = decode(body, None, (1, 6), 1000).expect("decodes");
        assert_eq!(&clear.rgba[4..8], &[0, 0, 0, 0], "row 1 is transparent");
        assert_eq!(&clear.rgba[0..4], &[255, 0, 0, 255], "row 0 is the ink");
    }

    /// A parameter list that is not digits-then-`q` is not a sixel, so `DECRQSS`
    /// (`DCS $ q …`) keeps its own meaning.
    #[test]
    fn only_a_digit_run_before_q_is_a_sixel() {
        assert!(split(b"q#1;2;100;0;0~").is_some());
        assert!(split(b"0;1;0q~").is_some());
        assert!(split(b"$q\"p").is_none(), "DECRQSS is not a picture");
        assert!(split(b"+q").is_none());
        let (p, body) = split(b"0;1;0q~").expect("params");
        assert_eq!((p.aspect, p.background, p.grid), (0, 1, 0));
        assert_eq!(body, b"~");
    }

    /// The raster is the one `size` measured: a run that claims a million columns
    /// is clamped, and the pixel ceiling refuses instead of allocating.
    #[test]
    fn a_run_cannot_go_past_the_edge_nor_the_ceiling() {
        let body = b"#1;2;100;0;0!999999~";
        let (w, h) = size(body).expect("a raster");
        assert_eq!((w, h), (999_999, 6), "the size is what the stream says");
        assert!(
            decode(body, Some([0, 0, 0]), (999_999, 6), 1_000).is_none(),
            "refused"
        );
        let small = b"#1;2;100;0;0!4~";
        let r = decode(small, Some([0, 0, 0]), (4, 6), 100).expect("decodes");
        assert_eq!(r.rgba.len(), 4 * 6 * 4, "four columns of six rows");
    }

    /// Both colour spaces the protocol defines: `;2` percentages and `;1` HLS.
    #[test]
    fn colours_arrive_as_percentages_or_as_hls() {
        let rgb = rgb([100, 0, 50]);
        assert_eq!(rgb, [255, 0, 128, 255]);
        let red = hls([0, 50, 100]);
        assert_eq!(red, [255, 0, 0, 255], "hue 0, full saturation");
        let grey = hls([0, 50, 0]);
        assert_eq!(grey, [128, 128, 128, 255], "no saturation is grey");
    }
}
