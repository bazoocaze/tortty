//! Font and glyph rasterization (`fontdb` to find, `swash` to draw).
//!
//! No freetype, no fontconfig, no Pango: that is what keeps the binary
//! self-contained on the three systems. The cell in pixels comes from the font
//! metrics at the requested size.

use std::sync::Arc;

use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::Format;
use swash::FontRef;

pub struct Face {
    pub data: Arc<Vec<u8>>,
    pub index: u32,
}

impl Face {
    pub fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(&self.data, self.index as usize)
    }
}

pub struct Fonts {
    pub primary: Face,
    pub styled: Option<Face>,
    pub italic: Option<Face>,
    pub bold_italic: Option<Face>,
    /// Fallback fonts, in the order they are tried when the primary does not
    /// have the character: symbols, emoji, CJK. Without the chain, emoji and CJK
    /// came out blank.
    pub fallbacks: Vec<Face>,
    /// Size in pixels (already converted from points).
    pub px: f32,
    /// Cell advance (width).
    pub cw: f32,
    /// Line height.
    pub ch: f32,
    pub ascent: f32,
    pub family: String,
}

/// Face indices used by the glyph cache.
/// With "monospace" (or empty) the choice is ours: the fontdb generic family
/// lands on FreeMono on this machine. With an explicit name, the name rules.
const PREFER: [&str; 6] = [
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "JetBrains Mono",
    "Hack",
    "Menlo",
];

pub const FACE_PRIMARY: usize = 0;
pub const FACE_STYLED: usize = 1;
pub const FACE_ITALIC: usize = 2;
/// From here on the indices are `FACE_FALLBACK + i` in the fallback chain.
pub const FACE_FALLBACK: usize = 3;
pub const FACE_BOLD_ITALIC: usize = 4;

impl Fonts {
    pub fn new(family: &str, px: f32) -> Result<Fonts, String> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();

        // A generic name (`Monospace`, which is the file default) has to land
        // on the preference list: fontdb does **not** have the fontconfig
        // aliases, so matching "Monospace" by name gives FreeMono — the ugly
        // font nobody meant to pick.
        let generic = matches!(
            family.to_ascii_lowercase().as_str(),
            "" | "monospace" | "mono" | "fixed" | "sans-serif" | "sans" | "serif" | "system-ui"
        );
        let explicit = !generic;
        let mut chosen: Option<(Face, String)> = None;
        if explicit {
            chosen = query(&db, family, 400, fontdb::Style::Normal);
        }
        if chosen.is_none() {
            for name in PREFER {
                if let Some(r) = query(&db, name, 400, fontdb::Style::Normal) {
                    chosen = Some(r);
                    break;
                }
            }
        }
        let (primary, real_family) = chosen
            .or_else(|| query(&db, "monospace", 400, fontdb::Style::Normal))
            .ok_or_else(|| format!("no monospaced font found ({family})"))?;
        let family: &str = if explicit {
            family
        } else {
            real_family.as_str()
        };

        let styled = query(&db, family, 700, fontdb::Style::Normal).map(|(f, _)| f);
        let italic = query(&db, family, 400, fontdb::Style::Italic).map(|(f, _)| f);
        let bold_italic = query(&db, family, 700, fontdb::Style::Italic).map(|(f, _)| f);
        let fallbacks = fallback_chain(&db, &primary, real_family.as_str());

        let font = primary
            .font()
            .ok_or_else(|| "primary font unreadable".to_string())?;
        let metrics = font.metrics(&[]).scale(px);
        let gm = font.glyph_metrics(&[]).scale(px);
        let charmap = font.charmap();

        let mut cw = gm.advance_width(charmap.map('M'));
        if cw <= 1.0 {
            cw = (px * 0.6).max(1.0);
        }
        let ch = (metrics.ascent + metrics.descent + metrics.leading).max(px);
        let ascent = metrics.ascent;

        Ok(Fonts {
            primary,
            styled,
            italic,
            bold_italic,
            fallbacks,
            px,
            cw,
            ch,
            ascent,
            family: real_family,
        })
    }

    pub fn face(&self, idx: usize) -> &Face {
        match idx {
            FACE_STYLED => self.styled.as_ref().unwrap_or(&self.primary),
            FACE_ITALIC => self.italic.as_ref().unwrap_or(&self.primary),
            FACE_BOLD_ITALIC => self
                .bold_italic
                .as_ref()
                .or(self.italic.as_ref())
                .unwrap_or(&self.primary),
            _ if idx >= FACE_FALLBACK => self
                .fallbacks
                .get(idx - FACE_FALLBACK)
                .unwrap_or(&self.primary),
            _ => &self.primary,
        }
    }

    /// Faces to try, in order: the one matching both attributes, each one
    /// alone, the primary, and finally the fallback chain.
    /// Bold used to come first and "bold italic" came out upright.
    fn candidates(&self, bold: bool, italic: bool) -> Vec<usize> {
        let mut out = Vec::with_capacity(5 + self.fallbacks.len());
        if bold && italic && self.bold_italic.is_some() {
            out.push(FACE_BOLD_ITALIC);
        }
        if italic && self.italic.is_some() {
            out.push(FACE_ITALIC);
        }
        if bold && self.styled.is_some() {
            out.push(FACE_STYLED);
        }
        out.push(FACE_PRIMARY);
        out.extend((0..self.fallbacks.len()).map(|i| FACE_FALLBACK + i));
        out
    }

    /// First face in the chain whose cmap has the character: that is what the
    /// tests want to know ("where did this glyph come from"). Test-only —
    /// drawing uses `glyph_bitmap`, which decides by the bitmap, not the cmap.
    #[cfg(test)]
    pub fn origin(&self, ch: char) -> (usize, u16) {
        let idx = self.face_of(ch, false, false).unwrap_or(FACE_PRIMARY);
        let gid = self.face(idx).font().map_or(0, |ft| ft.charmap().map(ch));
        (idx, gid)
    }

    /// Glyph bitmap, with the information of whether it is colored.
    ///
    /// Two passes: first the color bitmap (emoji) over the whole chain, then
    /// the mask. A single pass is not enough because having the character in
    /// the cmap does not mean the face can draw it — DejaVu Sans maps U+1F600
    /// and produces no bitmap at all, and the emoji came out blank.
    pub fn glyph_bitmap(
        &self,
        ctx: &mut ScaleContext,
        ch: char,
        bold: bool,
        italic: bool,
    ) -> Option<(GlyphBitmap, bool, usize, u16)> {
        let faces = self.candidates(bold, italic);
        let gids: Vec<(usize, u16)> = faces
            .iter()
            .filter_map(|&idx| {
                let g = self.face(idx).font().map(|f| f.charmap().map(ch))?;
                (g != 0).then_some((idx, g))
            })
            .collect();
        // 1) does the cell font (and the style variants) draw it? then it is
        //    theirs. The color pass used to sweep the whole chain before any
        //    mask, and Noto Color Emoji — which maps ASCII digits and has a
        //    19x18 bitmap — stole the numbers: the tab label came out with a
        //    two-cell rectangle instead of the digit (B4, 2026-11-04).
        for &(idx, gid) in gids
            .iter()
            .filter(|(idx, _)| !Fonts::is_from_fallback(*idx))
        {
            if let Some(b) = rasterize_color(ctx, self.face(idx), self.px, gid) {
                return Some((b, true, idx, gid));
            }
            if let Some(b) = rasterize(ctx, self.face(idx), self.px, gid) {
                return Some((b, false, idx, gid));
            }
        }
        // 2) outside it, color first in the chain: having the character in the
        //    cmap does not mean the face can draw it (DejaVu Sans maps U+1F600
        //    and produces no bitmap), so emoji needs the color pass before any
        //    fallback mask.
        let fallback = || gids.iter().filter(|(idx, _)| Fonts::is_from_fallback(*idx));
        for &(idx, gid) in fallback() {
            if let Some(b) = rasterize_color(ctx, self.face(idx), self.px, gid) {
                return Some((b, true, idx, gid));
            }
        }
        fallback().find_map(|&(idx, gid)| {
            rasterize(ctx, self.face(idx), self.px, gid).map(|b| (b, false, idx, gid))
        })
    }

    /// Is the face from the fallback chain? `FACE_BOLD_ITALIC` sits in the
    /// middle of the indices but is not a fallback.
    fn is_from_fallback(idx: usize) -> bool {
        idx >= FACE_FALLBACK && idx != FACE_BOLD_ITALIC
    }

    /// Bitmap of an **already resolved** glyph (face + gid from shaping): color
    /// first, mask after. It does not go through the chain — the face is chosen.
    pub fn bitmap_for(
        &self,
        ctx: &mut ScaleContext,
        face: usize,
        gid: u16,
    ) -> Option<(GlyphBitmap, bool)> {
        let f = self.face(face);
        if let Some(b) = rasterize_color(ctx, f, self.px, gid) {
            return Some((b, true));
        }
        rasterize(ctx, f, self.px, gid).map(|b| (b, false))
    }

    /// First face in the chain whose cmap has the character — it is what groups
    /// cells into shaping runs. It says who **maps** the character, not who can
    /// draw it: drawing checks and, with no bitmap, falls back to the whole
    /// chain.
    pub fn face_of(&self, ch: char, bold: bool, italic: bool) -> Option<usize> {
        for idx in self.candidates(bold, italic) {
            if let Some(g) = self.face(idx).font().map(|f| f.charmap().map(ch)) {
                if g != 0 {
                    return Some(idx);
                }
            }
        }
        None
    }
}

/// Fallback fonts, in priority order. Only the ones that exist on the machine
/// go in; a repeated family does not go in twice (each face is a font copy).
fn fallback_chain(db: &fontdb::Database, primary: &Face, family: &str) -> Vec<Face> {
    const QUERIES: &[&str] = &[
        "sans-serif",  // symbols, arrows, boxes
        "DejaVu Sans", // braille, geometric shapes
        "Noto Sans Symbols 2",
        "Symbola",
        "Noto Color Emoji", // colored emoji (CBDT/sbix)
        "Noto Sans CJK SC", // CJK
        "Noto Sans CJK JP",
        "Noto Serif CJK SC",
        "Source Han Sans SC",
    ];
    let mut out: Vec<Face> = Vec::new();
    // the primary itself may cover what the others do not: do not repeat it
    let mut seen: Vec<(u32, String)> = vec![(primary.index, family.to_string())];
    for name in QUERIES {
        let Some((face, real)) = query(db, name, 400, fontdb::Style::Normal) else {
            continue;
        };
        if seen.iter().any(|(i, f)| *i == face.index && *f == real) {
            continue;
        }
        seen.push((face.index, real));
        out.push(face);
    }
    out
}

fn query(
    db: &fontdb::Database,
    family: &str,
    weight: u16,
    style: fontdb::Style,
) -> Option<(Face, String)> {
    // A requested name that is absent returns `None`: falling back to the
    // generic family here turned "Noto Sans Symbols 2" (not installed) into
    // FreeMono, which maps emoji and draws a gray slash in place of the
    // colored emoji.
    let generic = match family.to_ascii_lowercase().as_str() {
        "sans-serif" | "sans" | "system-ui" => Some(fontdb::Family::SansSerif),
        "serif" => Some(fontdb::Family::Serif),
        "monospace" | "mono" | "fixed" => Some(fontdb::Family::Monospace),
        "cursive" => Some(fontdb::Family::Cursive),
        "fantasy" => Some(fontdb::Family::Fantasy),
        _ => None,
    };
    let target = match generic {
        Some(g) => vec![g],
        None => vec![fontdb::Family::Name(family)],
    };
    query_one(db, &target, weight, style)
}

fn query_one(
    db: &fontdb::Database,
    families: &[fontdb::Family<'_>],
    weight: u16,
    style: fontdb::Style,
) -> Option<(Face, String)> {
    let q = fontdb::Query {
        families,
        weight: fontdb::Weight(weight),
        stretch: fontdb::Stretch::Normal,
        style,
    };
    let id = db.query(&q)?;
    let name = db
        .face(id)
        .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
        .unwrap_or_default();
    let mut data: Option<Vec<u8>> = None;
    let mut index = 0u32;
    db.with_face_data(id, |d, i| {
        data = Some(d.to_vec());
        index = i;
    })?;
    Some((
        Face {
            data: Arc::new(data?),
            index,
        },
        name,
    ))
}

pub struct GlyphBitmap {
    pub w: u32,
    pub h: u32,
    pub left: i32,
    pub top: i32,
    pub data: Vec<u8>,
}

/// Rasterizes a glyph (alpha coverage) at the cell size.
/// Colored glyph (emoji CBDT/sbix): RGBA straight from the font. `None` when
/// the font only has an outline, and then the caller uses `rasterize` (mask).
pub fn rasterize_color(
    ctx: &mut ScaleContext,
    face: &Face,
    px: f32,
    glyph: u16,
) -> Option<GlyphBitmap> {
    let font = face.font()?;
    let mut scaler = ctx.builder(font).size(px).hint(true).build();
    // no `format`: a color bitmap already comes as RGBA; `Format` is mask-only
    let render = Render::new(&[
        Source::ColorBitmap(StrikeWith::BestFit),
        Source::ColorOutline(0),
    ]);
    let image = render.render(&mut scaler, glyph)?;
    if image.placement.width == 0 || image.placement.height == 0 {
        return None;
    }
    Some(GlyphBitmap {
        w: image.placement.width,
        h: image.placement.height,
        left: image.placement.left,
        top: image.placement.top,
        data: image.data.clone(),
    })
}

pub fn rasterize(ctx: &mut ScaleContext, face: &Face, px: f32, glyph: u16) -> Option<GlyphBitmap> {
    let font = face.font()?;
    let mut scaler = ctx.builder(font).size(px).hint(true).build();
    let mut render = Render::new(&[Source::Outline]);
    render.format(Format::Alpha);
    let image = render.render(&mut scaler, glyph)?;
    if image.placement.width == 0 || image.placement.height == 0 {
        return None;
    }
    Some(GlyphBitmap {
        w: image.placement.width,
        h: image.placement.height,
        left: image.placement.left,
        top: image.placement.top,
        data: image.data.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::new("DejaVu Sans Mono", 17.0).expect("font")
    }

    /// Box drawing (DEC/Unicode) has to come from the primary: coming from a
    /// proportional fallback it does not close the frame (found 2026-11-02).
    #[test]
    fn box_comes_from_primary() {
        let f = fonts();
        for ch in ['┌', '─', '┐', '│', '└', '┘'] {
            let (face, gid) = f.origin(ch);
            assert_ne!(gid, 0, "{ch} has no glyph");
            assert_eq!(face, FACE_PRIMARY, "{ch} came from face {face}");
        }
    }

    /// A digit is the primary's, as a mask: Noto Color Emoji maps '1' and has
    /// a 19x18 color bitmap, and it beat the primary mask — the tab number
    /// came out as a wide rectangle instead of the digit (B4).
    #[test]
    fn digit_comes_from_primary_as_mask() {
        let f = fonts();
        let mut ctx = ScaleContext::new();
        for ch in ['0', '1', '2', '4', '9'] {
            let (b, cor, face, _) = f
                .glyph_bitmap(&mut ctx, ch, false, false)
                .unwrap_or_else(|| panic!("{ch} has no bitmap"));
            assert!(!cor, "{ch} came out colored");
            assert_eq!(face, FACE_PRIMARY, "{ch} came from face {face}");
            assert!(b.w < 16, "{ch} too wide: {}x{}", b.w, b.h);
        }
    }

    /// Emoji has to come out **colored**, even when an earlier face in the
    /// chain maps the character: DejaVu Sans maps U+1F600 and draws nothing,
    /// and the emoji stayed blank on screen.
    #[test]
    fn emoji_comes_out_colored() {
        let f = fonts();
        let mut ctx = ScaleContext::new();
        for ch in ['\u{1F600}', '\u{1F525}', '\u{1F680}', '\u{1F44D}'] {
            let (b, cor, _, _) = f
                .glyph_bitmap(&mut ctx, ch, false, false)
                .unwrap_or_else(|| panic!("{ch} has no bitmap"));
            assert!(cor, "{ch} came out as a mask, it should be color");
            assert!(b.w > 1 && b.h > 1, "{ch} empty bitmap");
        }
    }

    /// A missing fallback font cannot become another font: that is how
    /// "Noto Sans Symbols 2" turned into FreeMono and drew emoji as a slash.
    #[test]
    fn font_missing_not_becomes_other() {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        assert!(query(
            &db,
            "Tortty Font That Does Not Exist",
            400,
            fontdb::Style::Normal
        )
        .is_none());
        // the generic one still works
        assert!(query(&db, "monospace", 400, fontdb::Style::Normal).is_some());
    }

    /// CJK and emoji are not in the primary: the fallback chain has to find
    /// them.
    #[test]
    fn cjk_and_emoji_land_on_chain() {
        let f = fonts();
        for ch in ['日', '😀'] {
            let (face, gid) = f.origin(ch);
            assert_ne!(gid, 0, "{ch} found no face");
            assert!(face >= FACE_FALLBACK, "{ch} should come from fallback");
        }
    }
}

#[cfg(test)]
mod metrics_probe {
    use super::*;

    /// Only runs with `--nocapture`: shows the box glyph metric against the
    /// cell, which is what decides whether the frame closes.
    #[test]
    fn print_box_metric() {
        let f = Fonts::new("DejaVu Sans Mono", 17.0).expect("font");
        let mut ctx = ScaleContext::new();
        println!(
            "cell: cw={} ch={} ascent={} px={}",
            f.cw, f.ch, f.ascent, f.px
        );
        for ch in ['─', '│', '┌', '┼', 'M', '日'] {
            let (face, gid) = f.origin(ch);
            let b = rasterize(&mut ctx, f.face(face), f.px, gid);
            match b {
                Some(b) => println!(
                    "{ch} face={face} gid={gid} w={} h={} left={} top={}",
                    b.w, b.h, b.left, b.top
                ),
                None => println!("{ch} face={face} gid={gid} no bitmap"),
            }
        }
    }
}
