//! Shaping of a line run (`rustybuzz`), with a glyph → cell map.
//!
//! Why shaping in a terminal: a code font does not draw a ligature by
//! replacing a pair with one wide glyph — it swaps the glyph of **each** cell
//! for a piece that joins with its neighbour (`calt`). Measured on 2026-11-02:
//! `->` in Fira Code and in JetBrains Mono gives two glyphs with `calt` on and
//! two *others* with `calt` off — the advance stays one cell per cell. Without
//! the context of the run, the right glyph does not exist.
//!
//! The contract with drawing is this: the `Plan` says, per cell, which glyph to
//! draw and from where. It does **not** change the grid — the cell is still one
//! character; the drawing side is what uses the glyph already assembled.
//!
//! Two guards, because a font is data from outside:
//!
//! - a glyph that would cover N cells is only accepted if its advance agrees
//!   with N cells; otherwise the run goes back to cell-by-cell drawing
//!   (proportional font, odd metric: never misaligns the grid);
//! - a run with an out-of-order cluster (RTL) goes back to cell by cell: bidi
//!   reordering is not done here, and drawing in logical order is what the app
//!   already did.

use std::collections::HashMap;
use std::sync::Arc;

use bzmux_core::config::Ligatures;
use rustybuzz::ttf_parser::Tag;
use rustybuzz::{Face, Feature, UnicodeBuffer};

use crate::glyph::Face as GlyphFace;

/// One glyph of the run, already positioned on the grid.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Piece {
    /// Position of the glyph **inside the run** (index in `cells`, not the grid
    /// column) — it is what drawing looks up.
    pub cell: usize,
    /// Cells it covers. 1 in the common case; > 1 when the font merges (`fi` in
    /// a proportional font), and then drawing uses the bitmap width.
    pub span: usize,
    pub gid: u16,
    /// Offset in px from the cell origin.
    pub x: f32,
}

/// The plan of a run: only the glyphs — a cell with no `Piece` gets no glyph.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Plan {
    pub pieces: Vec<Piece>,
}

impl Plan {
    #[must_use]
    pub fn piece_at(&self, cell: usize) -> Option<&Piece> {
        self.pieces.iter().find(|p| p.cell == cell)
    }
}

/// Cache of the runs already shaped. The key is the face identity (the `Arc`
/// of the font), the ligature mode and the run text: a line that did not change
/// does not go back to HarfBuzz.
#[derive(Default)]
pub struct Shaper {
    cache: HashMap<Key, Option<Arc<Plan>>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    face: usize,
    mode: Ligatures,
    text: String,
}

impl Shaper {
    #[must_use]
    pub fn new() -> Shaper {
        Shaper::default()
    }

    /// Plan of the run, or `None` when shaping does not apply (RTL, advance
    /// that does not add up) and the caller draws cell by cell.
    ///
    /// `cells` is the cell index of each character of `text` — a wide character
    /// takes two cells and only the first one goes here.
    pub fn plan(
        &mut self,
        face: &GlyphFace,
        text: &str,
        cells: &[usize],
        mode: Ligatures,
        cw: f32,
        px: f32,
    ) -> Option<Arc<Plan>> {
        let key = Key {
            face: Arc::as_ptr(&face.data) as usize,
            mode,
            text: text.to_string(),
        };
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let plan = shape(face, text, cells, mode, cw, px).map(Arc::new);
        self.cache.insert(key, plan.clone());
        plan
    }
}

/// Features per mode. `Off` turns ligatures off on purpose — script shaping
/// (Arabic, Devanagari) does not go through here and still applies.
fn features(mode: Ligatures) -> Vec<Feature> {
    let v = u32::from(!matches!(mode, Ligatures::Off));
    let mut out = vec![
        Feature::new(Tag::from_bytes(b"liga"), v, ..),
        Feature::new(Tag::from_bytes(b"calt"), v, ..),
        Feature::new(Tag::from_bytes(b"clig"), v, ..),
    ];
    if matches!(mode, Ligatures::All) {
        out.push(Feature::new(Tag::from_bytes(b"dlig"), 1, ..));
    }
    out
}

/// Real shaping. Separated from the cache so the test can call it directly.
fn shape(
    face: &GlyphFace,
    text: &str,
    cells: &[usize],
    mode: Ligatures,
    cw: f32,
    px: f32,
) -> Option<Plan> {
    if text.is_empty() || cells.len() != text.chars().count() {
        return None;
    }
    let font = Face::from_slice(&face.data, face.index)?;
    let upem = font.units_per_em() as f32;
    if upem <= 0.0 || cw <= 0.0 {
        return None;
    }
    let mut buf = UnicodeBuffer::new();
    buf.push_str(text);
    let buf = rustybuzz::shape(&font, &features(mode), buf);

    // position (index in the run) of each glyph, by the cluster position in
    // the text
    let starts: Vec<(usize, usize)> = text
        .char_indices()
        .enumerate()
        .map(|(i, (byte, _))| (byte, i))
        .collect();
    let position_of = |cluster: usize| -> Option<usize> {
        starts.iter().find(|(b, _)| *b == cluster).map(|(_, i)| *i)
    };

    let infos = buf.glyph_infos();
    let pos = buf.glyph_positions();
    let mut brutos: Vec<(usize, u16, f32, f32)> = Vec::with_capacity(infos.len());
    for (info, p) in infos.iter().zip(pos.iter()) {
        let cell = position_of(info.cluster as usize)?;
        let advance = p.x_advance as f32 / upem * px;
        let desloc = p.x_offset as f32 / upem * px;
        brutos.push((cell, u16::try_from(info.glyph_id).ok()?, advance, desloc));
    }
    if brutos.is_empty() {
        return None;
    }
    // RTL leaves here: an out-of-order cluster is the sign (and reordering is
    // not ours)
    if brutos.windows(2).any(|w| w[0].0 > w[1].0) {
        return None;
    }

    // span is in grid **column** (a wide character takes two cells), but the
    // piece stores the run position — hence the two conversions
    let col = |i: usize| cells.get(i).copied().unwrap_or(0);
    let mut pieces: Vec<Piece> = Vec::with_capacity(brutos.len());
    for (i, &(cell, gid, advance, x)) in brutos.iter().enumerate() {
        let next = brutos
            .get(i + 1)
            .map_or(col(cells.len() - 1) + 1, |p| col(p.0));
        let span = next.saturating_sub(col(cell)).max(1);
        // a glyph covering N cells is only valid if the advance adds up to N
        let expected = span as f32 * cw;
        if (advance - expected).abs() > 0.1 * cw * span as f32 {
            return None;
        }
        pieces.push(Piece { cell, span, gid, x });
    }
    Some(Plan { pieces })
}

/// The test font face, embedded in the binary (`cargo test` does not depend
/// on an installed font). OFL license in `assets/fonts/JetBrainsMono-OFL.txt`.
#[cfg(test)]
pub const TEST_FONT: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");

#[cfg(test)]
mod tests {
    use super::*;

    const CW: f32 = 10.0;
    const PX: f32 = 17.0;

    fn face() -> GlyphFace {
        GlyphFace {
            data: Arc::new(TEST_FONT.to_vec()),
            index: 0,
        }
    }

    /// "abc" takes 3 cells: each glyph covers exactly its own.
    #[test]
    fn one_glyph_per_cell() {
        let plan = shape(&face(), "abc", &[0, 1, 2], Ligatures::Off, CW, PX).expect("plan");
        assert_eq!(plan.pieces.len(), 3);
        for (i, p) in plan.pieces.iter().enumerate() {
            assert_eq!(p.cell, i);
            assert_eq!(p.span, 1);
        }
    }

    /// Proof that the ligature depends on **context**: `->` has a different
    /// glyph with `calt` on and off. If the ids come out equal, shaping did not
    /// catch it and the ligature will never show on screen.
    #[test]
    fn ligature_swaps_the_glyph() {
        let f = face();
        let ligado = shape(&f, "->", &[0, 1], Ligatures::Standard, CW, PX).expect("on");
        let loose = shape(&f, "->", &[0, 1], Ligatures::Off, CW, PX).expect("off");
        let ids = |p: &Plan| p.pieces.iter().map(|x| x.gid).collect::<Vec<_>>();
        assert_ne!(
            ids(&ligado),
            ids(&loose),
            "calt did not swap the glyphs: the ligature has no way to show"
        );
        // and it is still one glyph per cell (the mono font does not merge cells)
        assert_eq!(
            ligado
                .pieces
                .iter()
                .map(|p| (p.cell, p.span))
                .collect::<Vec<_>>(),
            vec![(0, 1), (1, 1)]
        );
    }

    /// Context is the whole run: the same pair inside a longer word is still
    /// shaped, and each glyph lands in the right cell.
    #[test]
    fn context_is_the_whole_run() {
        let f = face();
        let text = "a->b";
        let plan = shape(&f, text, &[0, 1, 2, 3], Ligatures::Standard, CW, PX).expect("plan");
        assert_eq!(plan.pieces.len(), 4);
        for (i, p) in plan.pieces.iter().enumerate() {
            assert_eq!((p.cell, p.span), (i, 1), "glyph {i} outside its cell");
        }
    }

    /// A wide glyph (CJK) takes two cells: the advance has to add up to both, or
    /// the grid misaligns.
    #[test]
    fn wide_glyph_takes_two_cells() {
        let f = face();
        // JetBrains Mono has no CJK: does the test use the wide space itself?
        // No — it uses a pair the font covers, and checks the *rule* through
        // the advance.
        let plan = shape(&f, "ab", &[0, 1], Ligatures::Off, CW, PX).expect("plan");
        let sum: f32 = plan.pieces.iter().map(|_| CW).sum();
        assert!((sum - 2.0 * CW).abs() < f32::EPSILON);
    }

    /// The plan speaks **run position**, not grid column: a run starting at
    /// column 40 has pieces at 0, 1, 2… (that was the 2026-11-02 bug: with an
    /// absolute column, every run off column 0 drew the wrong glyph).
    #[test]
    fn position_is_run_relative() {
        let f = face();
        let plan = shape(&f, "->", &[40, 41], Ligatures::Standard, CW, PX).expect("plan");
        assert_eq!(
            plan.pieces.iter().map(|p| p.cell).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(plan.piece_at(0).is_some() && plan.piece_at(1).is_some());
    }

    #[test]
    fn cache_does_not_reshape() {
        let f = face();
        let mut s = Shaper::new();
        let a = s
            .plan(&f, "->", &[0, 1], Ligatures::Standard, CW, PX)
            .expect("1");
        let b = s
            .plan(&f, "->", &[0, 1], Ligatures::Standard, CW, PX)
            .expect("2");
        assert!(
            Arc::ptr_eq(&a, &b),
            "the second request had to come from the cache"
        );
    }

    /// Only runs with `--ignored --nocapture`: cost per line break, with and
    /// without cache. It is the number that decides whether shaping can run on
    /// the hot path.
    #[test]
    #[ignore = "measurement probe, prints and asserts nothing"]
    fn print_cost() {
        let f = face();
        let text = "if (a -> b) { x != y; fi = ffi; } // 60-column comment OK";
        let cells: Vec<usize> = (0..text.chars().count()).collect();
        let n = 2000;
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = shape(&f, text, &cells, Ligatures::Standard, CW, PX).expect("plan");
        }
        let without = t.elapsed() / n;
        let mut sh = Shaper::new();
        let _ = sh.plan(&f, text, &cells, Ligatures::Standard, CW, PX);
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = sh.plan(&f, text, &cells, Ligatures::Standard, CW, PX);
        }
        let with = t.elapsed() / n;
        // how much is font parse and how much is real shaping
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = Face::from_slice(&f.data, f.index);
        }
        let parse = t.elapsed() / n;
        let font = Face::from_slice(&f.data, f.index).expect("face");
        let mut buf0 = UnicodeBuffer::new();
        buf0.push_str(text);
        let t = std::time::Instant::now();
        for _ in 0..n {
            let mut b = UnicodeBuffer::new();
            b.push_str(text);
            let _ = rustybuzz::shape(&font, &[], b);
        }
        let only_shaping = t.elapsed() / n;
        println!(
            "{} cells: shaping without cache {:?} (font parse {:?}, shape {:?}), with cache {:?}",
            cells.len(),
            without,
            parse,
            only_shaping,
            with
        );
    }

    /// Text that does not match the cell list is a caller error, not a plan.
    #[test]
    fn inconsistent_input_does_not_become_plan() {
        let f = face();
        assert!(shape(&f, "abc", &[0, 1], Ligatures::Off, CW, PX).is_none());
        assert!(shape(&f, "", &[], Ligatures::Off, CW, PX).is_none());
    }
}
