//! Fonte e rasterização de glifo (`fontdb` para achar, `swash` para desenhar).
//!
//! Sem freetype, sem fontconfig, sem Pango: é o que faz o binário ser
//! autocontido nos três sistemas. A célula em pixels sai das métricas da fonte
//! no tamanho pedido — o equivalente à métrica do Pango que o Python mede.

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
    /// Fontes de socorro, na ordem em que são tentadas quando a primária não
    /// tem o caractere: símbolos, emoji, CJK. O Python sempre teve isso pelo
    /// fontconfig/Pango; sem a cadeia, emoji e CJK ficavam em branco.
    pub fallbacks: Vec<Face>,
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
/// A partir daqui os índices são `FACE_FALLBACK + i` na cadeia de fallback.
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
        let fallbacks = fallback_chain(&db, &primary, real_family.as_str());

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

    /// Faces a tentar, na ordem: a que casa os dois atributos, cada um sozinho,
    /// a primária e por fim a cadeia de socorro.
    /// Antes o negrito vinha primeiro e "negrito itálico" saía em pé.
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

    /// Primeira face da cadeia que tem o caractere no cmap: é o que os testes
    /// querem saber ("de onde veio este glifo"). Só existe em teste — o desenho
    /// usa `glyph_bitmap`, que decide pela bitmap, não pelo cmap.
    #[cfg(test)]
    pub fn origin(&self, ch: char) -> (usize, u16) {
        let idx = self.face_of(ch, false, false).unwrap_or(FACE_PRIMARY);
        let gid = self.face(idx).font().map_or(0, |ft| ft.charmap().map(ch));
        (idx, gid)
    }

    /// Bitmap do glifo, com a informação de se ele é colorido.
    ///
    /// Duas passadas: primeiro bitmap de cor (emoji) em toda a cadeia, depois
    /// máscara. Uma passada só não basta porque ter o caractere no cmap não
    /// garante que a face saiba desenhá-lo — DejaVu Sans mapeia U+1F600 e não
    /// produz bitmap nenhum, e o emoji ficava em branco.
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
        // 1) a fonte da célula (e as variantes de estilo) desenha? então é
        //    dela. A passada de cor varria a cadeia inteira antes de qualquer
        //    máscara, e a Noto Color Emoji — que mapeia dígito ASCII e tem
        //    bitmap de 19x18 — roubava os números: o rótulo da aba saía com um
        //    retângulo de duas células no lugar do dígito (B4, 2026-11-04).
        for &(idx, gid) in gids.iter().filter(|(idx, _)| !Fonts::e_fallback(*idx)) {
            if let Some(b) = rasterize_color(ctx, self.face(idx), self.px, gid) {
                return Some((b, true, idx, gid));
            }
            if let Some(b) = rasterize(ctx, self.face(idx), self.px, gid) {
                return Some((b, false, idx, gid));
            }
        }
        // 2) fora dela, cor primeiro na cadeia: ter o caractere no cmap não
        //    garante que a face saiba desenhá-lo (DejaVu Sans mapeia U+1F600 e
        //    não produz bitmap), então emoji precisa da passada de cor antes
        //    de qualquer máscara de fallback.
        let fallback = || gids.iter().filter(|(idx, _)| Fonts::e_fallback(*idx));
        for &(idx, gid) in fallback() {
            if let Some(b) = rasterize_color(ctx, self.face(idx), self.px, gid) {
                return Some((b, true, idx, gid));
            }
        }
        fallback().find_map(|&(idx, gid)| {
            rasterize(ctx, self.face(idx), self.px, gid).map(|b| (b, false, idx, gid))
        })
    }

    /// A face é da cadeia de socorro? `FACE_BOLD_ITALIC` mora no meio dos
    /// índices, mas não é fallback.
    fn e_fallback(idx: usize) -> bool {
        idx >= FACE_FALLBACK && idx != FACE_BOLD_ITALIC
    }

    /// Bitmap de um glifo **já resolvido** (face + gid do shaping): cor primeiro,
    /// máscara depois. Não passa pela cadeia — a face já foi escolhida.
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

    /// Primeira face da cadeia que tem o caractere no cmap — é o que agrupa as
    /// células em trechos de shaping. Diz quem **mapeia** o caractere, não quem
    /// sabe desenhá-lo: o desenho confere e, se não houver bitmap, cai na
    /// cadeia inteira.
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

/// Fontes de socorro, na ordem de prioridade. Só entra quem existe na máquina;
/// família repetida não entra duas vezes (cada face é uma cópia da fonte).
fn fallback_chain(db: &fontdb::Database, primary: &Face, family: &str) -> Vec<Face> {
    const QUERIES: &[&str] = &[
        "sans-serif",  // símbolos, setas, caixas
        "DejaVu Sans", // braille, formas geométricas
        "Noto Sans Symbols 2",
        "Symbola",
        "Noto Color Emoji", // emoji colorido (CBDT/sbix)
        "Noto Sans CJK SC", // CJK
        "Noto Sans CJK JP",
        "Noto Serif CJK SC",
        "Source Han Sans SC",
    ];
    let mut out: Vec<Face> = Vec::new();
    // a própria primária pode cobrir o que as outras não cobrem: não repetir
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
    // Nome pedido e ausente devolve `None`: cair na família genérica aqui fazia
    // "Noto Sans Symbols 2" (não instalada) virar FreeMono, que mapeia emoji e
    // desenha um risco cinza no lugar do emoji colorido.
    let generica = match family.to_ascii_lowercase().as_str() {
        "sans-serif" | "sans" | "system-ui" => Some(fontdb::Family::SansSerif),
        "serif" => Some(fontdb::Family::Serif),
        "monospace" | "mono" | "fixed" => Some(fontdb::Family::Monospace),
        "cursive" => Some(fontdb::Family::Cursive),
        "fantasy" => Some(fontdb::Family::Fantasy),
        _ => None,
    };
    let target = match generica {
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

/// Rasteriza um glifo (cobertura alpha) no tamanho da célula.
/// Glifo colorido (emoji CBDT/sbix): RGBA direto da fonte. `None` quando a
/// fonte só tem contorno, e aí o chamador usa `rasterize` (máscara).
pub fn rasterize_color(
    ctx: &mut ScaleContext,
    face: &Face,
    px: f32,
    glyph: u16,
) -> Option<GlyphBitmap> {
    let font = face.font()?;
    let mut scaler = ctx.builder(font).size(px).hint(true).build();
    // sem `format`: bitmap de cor já vem RGBA; `Format` só vale para máscara
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

    fn fontes() -> Fonts {
        Fonts::new("DejaVu Sans Mono", 17.0).expect("fonte")
    }

    /// Caixa (DEC/Unicode) tem de sair da primária: vindo de fallback
    /// proporcional ela não fecha a moldura (achado em 2026-11-02).
    #[test]
    fn box_comes_da_primaria() {
        let f = fontes();
        for ch in ['┌', '─', '┐', '│', '└', '┘'] {
            let (face, gid) = f.origin(ch);
            assert_ne!(gid, 0, "{ch} sem glifo");
            assert_eq!(face, FACE_PRIMARY, "{ch} veio da face {face}");
        }
    }

    /// Dígito é da primária, em máscara: a Noto Color Emoji mapeia '1' e tem
    /// bitmap colorido de 19x18, e ganhava da máscara da primária — o número
    /// da aba saía como um retângulo largo no lugar do dígito (B4).
    #[test]
    fn digito_comes_da_primaria_em_mascara() {
        let f = fontes();
        let mut ctx = ScaleContext::new();
        for ch in ['0', '1', '2', '4', '9'] {
            let (b, cor, face, _) = f
                .glyph_bitmap(&mut ctx, ch, false, false)
                .unwrap_or_else(|| panic!("{ch} sem bitmap"));
            assert!(!cor, "{ch} saiu colorido");
            assert_eq!(face, FACE_PRIMARY, "{ch} veio da face {face}");
            assert!(b.w < 16, "{ch} largo demais: {}x{}", b.w, b.h);
        }
    }

    /// Emoji tem de sair **colorido**, mesmo quando uma face anterior da cadeia
    /// mapeia o caractere: DejaVu Sans mapeia U+1F600 e não desenha nada, e o
    /// emoji ficava em branco na tela.
    #[test]
    fn emoji_exits_colorido() {
        let f = fontes();
        let mut ctx = ScaleContext::new();
        for ch in ['\u{1F600}', '\u{1F525}', '\u{1F680}', '\u{1F44D}'] {
            let (b, cor, _, _) = f
                .glyph_bitmap(&mut ctx, ch, false, false)
                .unwrap_or_else(|| panic!("{ch} sem bitmap"));
            assert!(cor, "{ch} saiu em máscara, devia ser cor");
            assert!(b.w > 1 && b.h > 1, "{ch} bitmap vazio");
        }
    }

    /// Fonte de socorro que não existe não pode virar outra fonte: era assim
    /// que "Noto Sans Symbols 2" virava FreeMono e desenhava emoji como risco.
    #[test]
    fn font_missing_not_becomes_other() {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        assert!(query(
            &db,
            "Bzmux Fonte Que Nao Existe",
            400,
            fontdb::Style::Normal
        )
        .is_none());
        // genérica continua funcionando
        assert!(query(&db, "monospace", 400, fontdb::Style::Normal).is_some());
    }

    /// CJK e emoji não estão na primária: a cadeia de socorro tem de achar.
    #[test]
    fn cjk_e_emoji_caem_na_chain() {
        let f = fontes();
        for ch in ['日', '😀'] {
            let (face, gid) = f.origin(ch);
            assert_ne!(gid, 0, "{ch} não achou face");
            assert!(face >= FACE_FALLBACK, "{ch} devia vir de fallback");
        }
    }
}

#[cfg(test)]
mod metrics_probe {
    use super::*;

    /// Só roda com `--nocapture`: mostra a métrica do glifo de caixa contra a
    /// célula, que é o que decide se a moldura fecha.
    #[test]
    fn imprime_metrica_da_box() {
        let f = Fonts::new("DejaVu Sans Mono", 17.0).expect("fonte");
        let mut ctx = ScaleContext::new();
        println!(
            "celula: cw={} ch={} ascent={} px={}",
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
                None => println!("{ch} face={face} gid={gid} sem bitmap"),
            }
        }
    }
}
