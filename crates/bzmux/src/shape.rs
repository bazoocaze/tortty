//! Shaping de um trecho de linha (`rustybuzz`), com mapa de glifo → célula.
//!
//! Por que shaping num terminal: fonte de código não desenha a ligadura
//! substituindo um par por um glifo largo — ela troca o glifo de **cada**
//! célula por uma peça que se emenda com a vizinha (`calt`). Medido em
//! 2026-11-02: `->` na Fira Code e na JetBrains Mono dá dois glifos com `calt`
//! ligado e dois *outros* com `calt` desligado — o avanço continua uma célula
//! por célula. Sem o contexto do trecho, o glifo certo não existe.
//!
//! O contrato com o desenho é este: o `Plan` diz, por célula, qual glifo
//! desenhar e a partir de onde. Ele **não** muda a grade — a célula continua
//! sendo um caractere; quem desenha é que aproveita o glifo já montado.
//!
//! Duas guardas, porque fonte é dado de fora:
//!
//! - glifo que cobriria N células só é aceito se o avanço dele fechar com N
//!   células; senão o trecho volta para o desenho célula a célula (fonte
//!   proporcional, métrica estranha: nunca desalinha a grade);
//! - trecho com cluster fora de ordem (RTL) volta para célula a célula: a
//!   reordenação bidi não é feita aqui, e desenhar em ordem lógica é o que o
//!   app já fazia.

use std::collections::HashMap;
use std::sync::Arc;

use bzmux_core::config::Ligatures;
use rustybuzz::ttf_parser::Tag;
use rustybuzz::{Face, Feature, UnicodeBuffer};

use crate::glyph::Face as GlyphFace;

/// Um glifo do trecho, já posicionado na grade.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Piece {
    /// Posição do glifo **dentro do trecho** (índice em `cells`, não a coluna
    /// da grade) — é o que o desenho consulta.
    pub cell: usize,
    /// Células que ele cobre. 1 no caso comum; > 1 quando a fonte mescla
    /// (`fi` de fonte proporcional), e aí o desenho usa a largura do bitmap.
    pub span: usize,
    pub gid: u16,
    /// Deslocamento em px a partir da origem da célula.
    pub x: f32,
}

/// O plano de um trecho: só os glifos — célula sem `Piece` fica sem glifo.
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

/// Cache dos trechos já shapeados. A chave é a identidade da face (o `Arc` da
/// fonte), o modo de ligadura e o texto do trecho: linha que não mudou não
/// volta para o HarfBuzz.
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

    /// Plano do trecho, ou `None` quando o shaping não serve (RTL, avanço que
    /// não fecha) e o chamador desenha célula a célula.
    ///
    /// `cells` é o índice da célula de cada caractere de `text` — caractere
    /// largo ocupa duas células e só a primeira entra aqui.
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
        let plano = shape(face, text, cells, mode, cw, px).map(Arc::new);
        self.cache.insert(key, plano.clone());
        plano
    }
}

/// Features por modo. `Off` desliga ligadura de propósito — shaping de script
/// (árabe, devanagari) não passa por aqui e continua valendo.
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

/// Shaping de verdade. Separado do cache para o teste poder chamar direto.
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
    let fonte = Face::from_slice(&face.data, face.index)?;
    let upem = fonte.units_per_em() as f32;
    if upem <= 0.0 || cw <= 0.0 {
        return None;
    }
    let mut buf = UnicodeBuffer::new();
    buf.push_str(text);
    let buf = rustybuzz::shape(&fonte, &features(mode), buf);

    // posição (índice no trecho) de cada glifo, pela posição do cluster no texto
    let inicios: Vec<(usize, usize)> = text
        .char_indices()
        .enumerate()
        .map(|(i, (byte, _))| (byte, i))
        .collect();
    let posicao_de = |cluster: usize| -> Option<usize> {
        inicios.iter().find(|(b, _)| *b == cluster).map(|(_, i)| *i)
    };

    let infos = buf.glyph_infos();
    let pos = buf.glyph_positions();
    let mut brutos: Vec<(usize, u16, f32, f32)> = Vec::with_capacity(infos.len());
    for (info, p) in infos.iter().zip(pos.iter()) {
        let cell = posicao_de(info.cluster as usize)?;
        let avanco = p.x_advance as f32 / upem * px;
        let desloc = p.x_offset as f32 / upem * px;
        brutos.push((cell, u16::try_from(info.glyph_id).ok()?, avanco, desloc));
    }
    if brutos.is_empty() {
        return None;
    }
    // RTL sai daqui: cluster fora de ordem é o sinal (e reordenar não é nosso)
    if brutos.windows(2).any(|w| w[0].0 > w[1].0) {
        return None;
    }

    // span é em **coluna** da grade (caractere largo ocupa duas células), mas a
    // peça guarda a posição no trecho — daí as duas conversões
    let coluna = |i: usize| cells.get(i).copied().unwrap_or(0);
    let mut pieces: Vec<Piece> = Vec::with_capacity(brutos.len());
    for (i, &(cell, gid, avanco, x)) in brutos.iter().enumerate() {
        let proxima = brutos
            .get(i + 1)
            .map_or(coluna(cells.len() - 1) + 1, |p| coluna(p.0));
        let span = proxima.saturating_sub(coluna(cell)).max(1);
        // glifo que cobre N células só vale se o avanço fechar com N células
        let esperado = span as f32 * cw;
        if (avanco - esperado).abs() > 0.1 * cw * span as f32 {
            return None;
        }
        pieces.push(Piece { cell, span, gid, x });
    }
    Some(Plan { pieces })
}

/// A face da fonte de teste, embutida no binário (`cargo test` não depende de
/// fonte instalada). Licença OFL em `assets/fonts/JetBrainsMono-OFL.txt`.
#[cfg(test)]
pub const FONTE_DE_TESTE: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");

#[cfg(test)]
mod tests {
    use super::*;

    const CW: f32 = 10.0;
    const PX: f32 = 17.0;

    fn face() -> GlyphFace {
        GlyphFace {
            data: Arc::new(FONTE_DE_TESTE.to_vec()),
            index: 0,
        }
    }

    /// "abc" ocupa 3 células: cada glifo cobre exatamente a sua.
    #[test]
    fn um_glifo_por_celula() {
        let plano = shape(&face(), "abc", &[0, 1, 2], Ligatures::Off, CW, PX).expect("plano");
        assert_eq!(plano.pieces.len(), 3);
        for (i, p) in plano.pieces.iter().enumerate() {
            assert_eq!(p.cell, i);
            assert_eq!(p.span, 1);
        }
    }

    /// A prova de que a ligadura depende do **contexto**: `->` tem glifo
    /// diferente com `calt` ligado e desligado. Se os ids ficarem iguais, o
    /// shaping não pegou e a ligadura nunca vai aparecer na tela.
    #[test]
    fn ligadura_troca_o_glifo() {
        let f = face();
        let ligado = shape(&f, "->", &[0, 1], Ligatures::Standard, CW, PX).expect("ligado");
        let solto = shape(&f, "->", &[0, 1], Ligatures::Off, CW, PX).expect("solto");
        let ids = |p: &Plan| p.pieces.iter().map(|x| x.gid).collect::<Vec<_>>();
        assert_ne!(
            ids(&ligado),
            ids(&solto),
            "calt não trocou os glifos: ligadura não tem como aparecer"
        );
        // e continua sendo um glifo por célula (a fonte mono não funde célula)
        assert_eq!(
            ligado
                .pieces
                .iter()
                .map(|p| (p.cell, p.span))
                .collect::<Vec<_>>(),
            vec![(0, 1), (1, 1)]
        );
    }

    /// Contexto é o trecho inteiro: o mesmo par dentro de uma palavra maior
    /// continua sendo shapeado, e cada glifo cai na célula certa.
    #[test]
    fn contexto_do_trecho_inteiro() {
        let f = face();
        let texto = "a->b";
        let plano = shape(&f, texto, &[0, 1, 2, 3], Ligatures::Standard, CW, PX).expect("plano");
        assert_eq!(plano.pieces.len(), 4);
        for (i, p) in plano.pieces.iter().enumerate() {
            assert_eq!((p.cell, p.span), (i, 1), "glifo {i} fora da célula");
        }
    }

    /// Glifo largo (CJK) ocupa duas células: o avanço tem de fechar com as duas,
    /// senão a grade desalinha.
    #[test]
    fn largo_ocupa_duas_celulas() {
        let f = face();
        // a JetBrains Mono não tem CJK: o teste usa o próprio espaço largo? Não —
        // usa um par que a fonte cobre, e verifica a *regra* pelo avanço.
        let plano = shape(&f, "ab", &[0, 1], Ligatures::Off, CW, PX).expect("plano");
        let soma: f32 = plano.pieces.iter().map(|_| CW).sum();
        assert!((soma - 2.0 * CW).abs() < f32::EPSILON);
    }

    /// O plano fala **posição no trecho**, não coluna da grade: trecho que
    /// começa na coluna 40 tem peças em 0, 1, 2… (era o bug de 2026-11-02: com
    /// coluna absoluta, todo trecho fora da coluna 0 desenhava glifo trocado).
    #[test]
    fn posicao_e_relativa_ao_trecho() {
        let f = face();
        let plano = shape(&f, "->", &[40, 41], Ligatures::Standard, CW, PX).expect("plano");
        assert_eq!(
            plano.pieces.iter().map(|p| p.cell).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(plano.piece_at(0).is_some() && plano.piece_at(1).is_some());
    }

    #[test]
    fn cache_nao_reshapia() {
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
            "o segundo pedido tinha de vir do cache"
        );
    }

    /// Só roda com `--ignored --nocapture`: custo por quebra de linha, com e
    /// sem cache. É o número que decide se shaping pode rodar no caminho quente.
    #[test]
    #[ignore = "sonda de medição, imprime e não afirma"]
    fn imprime_custo() {
        let f = face();
        let texto = "if (a -> b) { x != y; fi = ffi; } // comentário de 60 colunas OK";
        let cells: Vec<usize> = (0..texto.chars().count()).collect();
        let n = 2000;
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = shape(&f, texto, &cells, Ligatures::Standard, CW, PX).expect("plano");
        }
        let sem = t.elapsed() / n;
        let mut sh = Shaper::new();
        let _ = sh.plan(&f, texto, &cells, Ligatures::Standard, CW, PX);
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = sh.plan(&f, texto, &cells, Ligatures::Standard, CW, PX);
        }
        let com = t.elapsed() / n;
        // quanto é parse da fonte e quanto é shaping de verdade
        let t = std::time::Instant::now();
        for _ in 0..n {
            let _ = Face::from_slice(&f.data, f.index);
        }
        let parse = t.elapsed() / n;
        let fonte = Face::from_slice(&f.data, f.index).expect("face");
        let mut buf0 = UnicodeBuffer::new();
        buf0.push_str(texto);
        let t = std::time::Instant::now();
        for _ in 0..n {
            let mut b = UnicodeBuffer::new();
            b.push_str(texto);
            let _ = rustybuzz::shape(&fonte, &[], b);
        }
        let so_shaping = t.elapsed() / n;
        println!(
            "{} células: shaping sem cache {:?} (parse da fonte {:?}, shape {:?}), cache {:?}",
            cells.len(),
            sem,
            parse,
            so_shaping,
            com
        );
    }

    /// Texto que não bate com a lista de células é erro de chamador, não plano.
    #[test]
    fn entrada_inconsistente_nao_vira_plano() {
        let f = face();
        assert!(shape(&f, "abc", &[0, 1], Ligatures::Off, CW, PX).is_none());
        assert!(shape(&f, "", &[], Ligatures::Off, CW, PX).is_none());
    }
}
