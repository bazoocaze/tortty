//! TIFF, the little that `chafa -f iterm` sends.
//!
//! The iTerm2 protocol carries "an image", not a PNG: `imgcat` sends a PNG, and
//! `chafa` writes an **uncompressed TIFF** — the container costs it a header and
//! then the pixels, without a zlib it would have to carry. A picture encoded
//! that way is refused by the PNG reader, so the third producer that speaks the
//! protocol had nothing to show (`1.0.85`).
//!
//! Only what that writer produces is read: strips, eight bits per sample, no
//! compression. Anything else answers `None` and the caller refuses with a
//! reason — a decoder that guesses at JPEG-in-TIFF would be a bigger surface
//! than the protocol deserves.

/// Above this a picture is refused: same ceiling as sixel, so one producer cannot
/// ask for a larger allocation than another.
pub const MAX_TIFF_PIXELS: u32 = 8_500_000;

struct Reader<'a> {
    data: &'a [u8],
    le: bool,
}

impl Reader<'_> {
    fn u16_at(&self, off: usize) -> Option<u16> {
        let b = self.data.get(off..off + 2)?;
        Some(if self.le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    fn u32_at(&self, off: usize) -> Option<u32> {
        let b = self.data.get(off..off + 4)?;
        Some(if self.le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    }
}

/// `true` when the bytes start with a TIFF header.
#[must_use]
pub fn is_tiff(data: &[u8]) -> bool {
    matches!(data.get(..4), Some(b"II*\0" | b"MM\0*"))
}

/// One IFD entry, with the values it points at (they may live outside the entry
/// when they do not fit four bytes).
fn entry(r: &Reader<'_>, ifd: usize, index: usize) -> Option<(u16, Vec<u32>)> {
    let at = ifd + 2 + index * 12;
    let tag = r.u16_at(at)?;
    let kind = r.u16_at(at + 2)?;
    let count = usize::try_from(r.u32_at(at + 4)?).ok()?;
    let width = match kind {
        3 => 2, // SHORT
        4 => 4, // LONG
        _ => return None,
    };
    let inline = width * count <= 4;
    let base = if inline {
        at + 8
    } else {
        usize::try_from(r.u32_at(at + 8)?).ok()?
    };
    let mut values = Vec::with_capacity(count.min(8));
    for i in 0..count {
        let off = base + i * width;
        values.push(if width == 2 {
            u32::from(r.u16_at(off)?)
        } else {
            r.u32_at(off)?
        });
    }
    Some((tag, values))
}

/// The size without decoding the pixels.
#[must_use]
pub fn size(data: &[u8]) -> Option<(u32, u32)> {
    let (_, width, height) = header(data)?;
    Some((width, height))
}

fn header(data: &[u8]) -> Option<(Reader<'_>, u32, u32)> {
    let le = match data.get(..4)? {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return None,
    };
    let r = Reader { data, le };
    let ifd = usize::try_from(r.u32_at(4)?).ok()?;
    let count = usize::from(r.u16_at(ifd)?);
    let (mut width, mut height) = (None, None);
    for i in 0..count {
        let Some((tag, values)) = entry(&r, ifd, i) else {
            continue;
        };
        let first = *values.first()?;
        match tag {
            256 => width = Some(first),
            257 => height = Some(first),
            _ => {},
        }
    }
    Some((r, width?, height?))
}

/// The picture as RGBA, `None` when it is not the kind this reader takes.
pub fn rgba(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let (r, width, height) = header(data)?;
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > u64::from(MAX_TIFF_PIXELS)
    {
        return None;
    }
    let ifd = usize::try_from(r.u32_at(4)?).ok()?;
    let count = usize::from(r.u16_at(ifd)?);
    let mut samples = 3u32;
    let mut bits = vec![8u32];
    let mut photo = 2u32; // RGB
    let mut planar = 1u32; // chunky
    let mut rows_per_strip = height;
    let mut strips: Vec<u32> = Vec::new();
    let mut byte_counts: Vec<u32> = Vec::new();
    for i in 0..count {
        let Some((tag, values)) = entry(&r, ifd, i) else {
            continue;
        };
        let first = || values.first().copied().unwrap_or(0);
        match tag {
            258 => bits = values,
            259 if first() != 1 => return None, // only "no compression"
            262 => photo = first(),
            273 => strips = values,
            277 => samples = first(),
            278 => rows_per_strip = first().max(1),
            279 => byte_counts = values,
            284 => planar = first(),
            _ => {},
        }
    }
    if planar != 1 || !(samples == 3 || samples == 4) || bits.iter().any(|&b| b != 8) {
        return None;
    }
    if photo != 2 && !(photo == 1 && samples == 3) {
        return None; // RGB, or the grey-as-three-samples some writers use
    }
    let samples = usize::try_from(samples).ok()?;
    let (w, h) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    let mut out = vec![0u8; w * h * 4];
    let mut row = 0usize;
    for (offset, len) in strips.iter().zip(byte_counts.iter()) {
        let offset = usize::try_from(*offset).ok()?;
        let len = usize::try_from(*len).ok()?;
        let strip = data.get(offset..offset.checked_add(len)?)?;
        let rows = usize::try_from(rows_per_strip).ok()?.min(h - row);
        let need = rows.checked_mul(w)?.checked_mul(samples)?;
        if strip.len() < need {
            return None; // the strip is shorter than what it claims to hold
        }
        for y in 0..rows {
            for x in 0..w {
                let from = (y * w + x) * samples;
                let to = ((row + y) * w + x) * 4;
                out[to] = strip[from];
                out[to + 1] = strip[from + 1];
                out[to + 2] = strip[from + 2];
                out[to + 3] = if samples == 4 { strip[from + 3] } else { 255 };
            }
        }
        row += rows;
        if row >= h {
            break;
        }
    }
    if row < h {
        return None; // the strips do not cover the picture
    }
    Some((width, height, out))
}

#[cfg(test)]
mod checks_tiff {
    use super::*;

    /// The shape `chafa` writes: header, the strips, then the IFD at the end
    /// (offset `0x1808` in a real capture). The reader follows the offsets, so
    /// where the IFD lives does not matter.
    fn chafa_like(w: u32, h: u32, px: &[u8], compression: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"II*\0");
        let ifd_at = 8 + px.len();
        out.extend_from_slice(&u32::try_from(ifd_at).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(px);
        let entries: Vec<(u16, u16, u32, u32)> = vec![
            (256, 4, 1, w),           // ImageWidth
            (257, 4, 1, h),           // ImageLength
            (258, 3, 3, 0),           // BitsPerSample -> at the tail
            (259, 3, 1, compression), // Compression
            (262, 3, 1, 2),           // Photometric = RGB
            (273, 4, 1, 8),           // StripOffsets -> the pixels
            (277, 3, 1, 3),           // SamplesPerPixel
            (278, 4, 1, h),           // RowsPerStrip
            (279, 4, 1, u32::try_from(px.len()).unwrap_or(0)),
        ];
        let bits_at = u32::try_from(ifd_at + 2 + entries.len() * 12 + 4).unwrap_or(0);
        out.extend_from_slice(&u16::try_from(entries.len()).unwrap_or(0).to_le_bytes());
        for (tag, kind, count, value) in entries {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            let value = if tag == 258 { bits_at } else { value };
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..3 {
            out.extend_from_slice(&8u16.to_le_bytes());
        }
        out
    }

    #[test]
    fn the_colours_and_the_row_order_come_out_as_they_went_in() {
        // 2x2: red, green / blue, white
        let px = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let t = chafa_like(2, 2, &px, 1);
        assert!(is_tiff(&t));
        assert_eq!(size(&t), Some((2, 2)));
        let (w, h, rgba) = rgba(&t).expect("decodes");
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            rgba,
            [
                255, 0, 0, 255, 0, 255, 0, 255, // the first row
                0, 0, 255, 255, 255, 255, 255, 255,
            ]
        );
    }

    #[test]
    fn a_compressed_tiff_is_not_ours() {
        let px = [0u8; 12];
        let t = chafa_like(2, 2, &px, 5); // LZW
        assert!(is_tiff(&t), "it is a TIFF");
        assert_eq!(size(&t), Some((2, 2)));
        assert!(rgba(&t).is_none(), "but not one this reader takes");
    }

    #[test]
    fn a_strip_that_is_shorter_than_it_claims_is_refused() {
        let mut t = chafa_like(2, 2, &[0u8; 12], 1);
        let ifd = 8 + 12;
        // StripByteCounts is the ninth entry: 2 + 8 * 12 + 8
        let at = ifd + 2 + 8 * 12 + 8;
        t[at..at + 4].copy_from_slice(&8u32.to_le_bytes());
        assert!(rgba(&t).is_none(), "the bytes are not there");
    }

    #[test]
    fn what_is_not_a_tiff_is_not_claimed() {
        assert!(!is_tiff(b"\x89PNG\r\n\x1a\n"));
        assert!(rgba(b"\x89PNG\r\n\x1a\n").is_none());
        assert!(rgba(b"II*\0").is_none());
    }
}
