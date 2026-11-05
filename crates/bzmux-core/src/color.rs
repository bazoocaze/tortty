//! Cor de célula e paleta xterm-256.
//!
//! O Python guardava três coisas no mesmo `int` (índice 0-255, sentinelas
//! 256/257 e RGB com o bit 24 marcado). Aqui o tipo diz o que é, sem
//! decodificação no caminho quente: `Default` é a sentinela, `Idx` é a paleta e
//! `Rgb` é truecolor direto na célula (sem tabela de cores alocadas, como no P4).

use std::fmt;

/// Cor de uma célula. `Default` vale como fg default no slot de fg e bg default
/// no slot de bg — é a mesma convenção do bzmux Python (`DEF_FG`/`DEF_BG`).
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug, Hash)]
pub enum Color {
    #[default]
    Default,
    Idx(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    /// Truecolor a partir dos três canais (clampados, como o SGR faz).
    #[inline]
    pub const fn rgb(r: u32, g: u32, b: u32) -> Color {
        const fn c(v: u32) -> u8 {
            if v > 255 {
                255
            } else {
                v as u8
            }
        }
        Color::Rgb(c(r), c(g), c(b))
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::Default => f.write_str("default"),
            Color::Idx(i) => write!(f, "{i}"),
            Color::Rgb(r, g, b) => write!(f, "#{r:02x}{g:02x}{b:02x}"),
        }
    }
}

/// Os 16 primeiros tons (a paleta ANSI clássica do bzmux Python).
pub const PALETTE: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// Paleta xterm-256 completa (16 ANSI + cubo 6×6×6 + 24 tons de cinza).
pub fn palette256() -> Vec<(u8, u8, u8)> {
    // cubo 6x6x6: componentes 0, 95, 135, 175, 215, 255
    const STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let mut pal: Vec<(u8, u8, u8)> = PALETTE.to_vec();
    for &r in &STEPS {
        for &g in &STEPS {
            for &b in &STEPS {
                pal.push((r, g, b));
            }
        }
    }
    for i in 0..24u8 {
        let v = 8 + i * 10;
        pal.push((v, v, v));
    }
    debug_assert_eq!(pal.len(), 256);
    pal
}

/// RGB final de uma cor de célula: precisa dos defaults do tema.
#[inline]
pub fn resolve(c: Color, pal: &[(u8, u8, u8)], default: (u8, u8, u8)) -> (u8, u8, u8) {
    match c {
        Color::Default => default,
        Color::Idx(i) => pal.get(i as usize).copied().unwrap_or(default),
        Color::Rgb(r, g, b) => (r, g, b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_has_256_e_bate_in_extremos() {
        let p = palette256();
        assert_eq!(p.len(), 256);
        assert_eq!(p[16], (0, 0, 0));
        assert_eq!(p[21], (0, 0, 255));
        assert_eq!(p[196], (255, 0, 0));
        assert_eq!(p[231], (255, 255, 255));
        assert_eq!(p[232], (8, 8, 8));
        assert_eq!(p[255], (238, 238, 238));
    }

    #[test]
    fn rgb_clampa() {
        assert_eq!(Color::rgb(300, 1, 2), Color::Rgb(255, 1, 2));
    }
}
