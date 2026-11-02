//! Célula da grade: caractere + caneta (cores e atributos).
//!
//! Os atributos são bits em `flags` e o sublinhado tem campo próprio (`ul`)
//! porque ele carrega *estilo* além de presença: `4:3` é curvo, `4:2` é duplo.
//! A cor do sublinhado (`SGR 58`) é um slot separado da cor do texto.

use crate::color::Color;

/// Negrito (SGR 1).
pub const BOLD: u8 = 1;
/// Inverso (SGR 7).
pub const INV: u8 = 2;
/// Itálico (SGR 3).
pub const ITALIC: u8 = 4;
/// Esmaecido (SGR 2).
pub const FAINT: u8 = 8;
/// Pisca (SGR 5/6). A piscada é do render; aqui é só o atributo.
pub const BLINK: u8 = 16;
/// Riscado (SGR 9).
pub const STRIKE: u8 = 32;
/// Overline (SGR 53).
pub const OVERLINE: u8 = 64;

/// Sem sublinhado.
pub const UL_NONE: u8 = 0;
/// Sublinhado simples (SGR 4).
pub const UL_SINGLE: u8 = 1;
/// Sublinhado duplo (SGR 4:2 e SGR 21).
pub const UL_DOUBLE: u8 = 2;
/// Sublinhado curvo (SGR 4:3).
pub const UL_CURLY: u8 = 3;
/// Sublinhado pontilhado (SGR 4:4).
pub const UL_DOTTED: u8 = 4;
/// Sublinhado tracejado (SGR 4:5).
pub const UL_DASHED: u8 = 5;

/// Segunda metade de um caractere largo: não desenha nada, a célula anterior
/// ocupa o dobro de largura.
pub const WIDE_CONT: char = '\0';

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    /// `SGR 58`; `Default` = usar a cor do texto.
    pub ul_color: Color,
    pub flags: u8,
    /// Estilo do sublinhado, `UL_*`.
    pub ul: u8,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: Color::Default,
        bg: Color::Default,
        ul_color: Color::Default,
        flags: 0,
        ul: UL_NONE,
    };

    /// Célula vazia: nada desenhado além do fundo default.
    #[inline]
    pub fn is_blank(&self) -> bool {
        self.ch == ' ' && self.bg == Color::Default && self.flags == 0 && self.ul == UL_NONE
    }

    #[inline]
    pub fn is_cont(&self) -> bool {
        self.ch == WIDE_CONT
    }

    /// A caneta mudou? Usado por seleção/cópia para decidir o que é "espaço".
    #[inline]
    pub fn has_style(&self) -> bool {
        self.flags != 0 || self.ul != UL_NONE || self.bg != Color::Default
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}
