//! Célula da grade.

use crate::color::Color;

/// Negrito (SGR 1).
pub const BOLD: u8 = 1;
/// Sublinhado (SGR 4).
pub const UL: u8 = 2;
/// Inverso (SGR 7).
pub const INV: u8 = 4;
/// Segunda metade de um caractere largo: não desenha nada, a célula anterior
/// ocupa o dobro de largura.
pub const WIDE_CONT: char = '\0';

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: u8,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: Color::Default,
        bg: Color::Default,
        flags: 0,
    };

    #[inline]
    pub fn is_blank(&self) -> bool {
        self.ch == ' ' && self.bg == Color::Default && self.flags == 0
    }

    #[inline]
    pub fn is_cont(&self) -> bool {
        self.ch == WIDE_CONT
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}
