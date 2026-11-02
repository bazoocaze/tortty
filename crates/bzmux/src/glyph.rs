//! Fonte e rasterização de glifo (`fontdb` para achar, `swash` para desenhar).
//!
//! Sem freetype, sem fontconfig, sem Pango: é o que faz o binário ser
//! autocontido nos três sistemas. A célula em pixels sai das métricas da fonte
//! no tamanho pedido — o equivalente à métrica do Pango que o Python mede.

use std::sync::Arc;

use swash::scale::{Render, ScaleContext, Source};
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
    pub fallback: Option<Face>,
    /// Tamanho em pixels (já convertido de pontos).
    pub px: f32,
    /// Avanço da célula (largura).
    pub cw: f32,
    /// Altura da linha.
    pub ch: f32,
    pub ascent: f32,
    pub family: String,
}

/// Índices de face usados pelo cache de glifo.
/// Com "monospace" (ou vazio) a escolha é nossa: a família genérica do fontdb
/// cai em FreeMono nesta máquina. Com nome explícito, o nome manda.
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
pub const FACE_FALLBACK: usize = 3;
pub const FACE_BOLD_ITALIC: usize = 4;

impl Fonts {
    pub fn new(family: &str, px: f32) -> Result<Fonts, String> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();

        // Nome genérico (`Monospace`, que é o padrão do arquivo) tem que cair
        // na lista de preferência: o fontdb **não** tem os aliases do
        // fontconfig, então casar "Monospace" pelo nome dá FreeMono — a fonte
        // feia que o Python nunca usava (o Pango consultava o fontconfig).
        let generico = matches!(
            family.to_ascii_lowercase().as_str(),
            "" | "monospace" | "mono" | "fixed" | "sans-serif" | "sans" | "serif" | "system-ui"
        );
        let explicit = !generico;
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
            .ok_or_else(|| format!("nenhuma fonte monoespaçada encontrada ({family})"))?;
        let family: &str = if explicit {
            family
        } else {
            real_family.as_str()
        };

        let styled = query(&db, family, 700, fontdb::Style::Normal).map(|(f, _)| f);
        let italic = query(&db, family, 400, fontdb::Style::Italic).map(|(f, _)| f);
        let bold_italic = query(&db, family, 700, fontdb::Style::Italic).map(|(f, _)| f);
        let fallback = query(&db, "sans-serif", 400, fontdb::Style::Normal).map(|(f, _)| f);

        let font = primary
            .font()
            .ok_or_else(|| "fonte primária ilegível".to_string())?;
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
            fallback,
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
            FACE_FALLBACK => self.fallback.as_ref().unwrap_or(&self.primary),
            _ => &self.primary,
        }
    }

    /// Face + glyph id para um caractere. `glyph == 0` = .notdef.
    pub fn pick(&self, ch: char, bold: bool, italic: bool) -> (usize, u16) {
        // Ordem: a face que casa os dois atributos, depois cada um sozinho.
        // Antes o negrito vinha primeiro e "negrito itálico" saía em pé.
        let mut candidates = [FACE_PRIMARY; 5];
        let mut n = 0usize;
        if bold && italic && self.bold_italic.is_some() {
            candidates[n] = FACE_BOLD_ITALIC;
            n += 1;
        }
        if italic && self.italic.is_some() {
            candidates[n] = FACE_ITALIC;
            n += 1;
        }
        if bold && self.styled.is_some() {
            candidates[n] = FACE_STYLED;
            n += 1;
        }
        candidates[n] = FACE_PRIMARY;
        n += 1;
        if self.fallback.is_some() {
            candidates[n] = FACE_FALLBACK;
            n += 1;
        }
        for &idx in &candidates[..n] {
            let face = self.face(idx);
            if let Some(font) = face.font() {
                let g = font.charmap().map(ch);
                if g != 0 {
                    return (idx, g);
                }
            }
        }
        (FACE_PRIMARY, 0)
    }
}

fn query(
    db: &fontdb::Database,
    family: &str,
    weight: u16,
    style: fontdb::Style,
) -> Option<(Face, String)> {
    // nome explícito primeiro; depois a família genérica equivalente
    if let Some(r) = query_one(db, &[fontdb::Family::Name(family)], weight, style) {
        return Some(r);
    }
    let generic = match family.to_ascii_lowercase().as_str() {
        "sans-serif" | "sans" | "system-ui" => fontdb::Family::SansSerif,
        "serif" => fontdb::Family::Serif,
        "cursive" => fontdb::Family::Cursive,
        "fantasy" => fontdb::Family::Fantasy,
        _ => fontdb::Family::Monospace,
    };
    query_one(db, &[generic], weight, style)
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

/// Rasteriza um glifo (cobertura alpha) no tamanho da célula.
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
