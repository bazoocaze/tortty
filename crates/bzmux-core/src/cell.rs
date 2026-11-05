//! Grid cell: character + pen (colors and attributes).
//!
//! The attributes are bits in `flags` and the underline has a field of its own
//! (`ul`) because it carries a *style* beyond presence: `4:3` is curly, `4:2` is
//! double. The underline color (`SGR 58`) is a slot separate from the text color.

use crate::color::Color;

/// Bold (SGR 1).
pub const BOLD: u8 = 1;
/// Inverse (SGR 7).
pub const INV: u8 = 2;
/// Italic (SGR 3).
pub const ITALIC: u8 = 4;
/// Faint (SGR 2).
pub const FAINT: u8 = 8;
/// Blink (SGR 5/6). The blinking belongs to the renderer; here it is the
/// attribute only.
pub const BLINK: u8 = 16;
/// Struck through (SGR 9).
pub const STRIKE: u8 = 32;
/// Overline (SGR 53).
pub const OVERLINE: u8 = 64;

/// No underline.
pub const UL_NONE: u8 = 0;
/// Single underline (SGR 4).
pub const UL_SINGLE: u8 = 1;
/// Double underline (SGR 4:2 and SGR 21).
pub const UL_DOUBLE: u8 = 2;
/// Curly underline (SGR 4:3).
pub const UL_CURLY: u8 = 3;
/// Dotted underline (SGR 4:4).
pub const UL_DOTTED: u8 = 4;
/// Dashed underline (SGR 4:5).
pub const UL_DASHED: u8 = 5;

/// Second half of a wide character: draws nothing, the previous cell takes twice
/// the width.
pub const WIDE_CONT: char = '\0';

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    /// `SGR 58`; `Default` = use the text color.
    pub ul_color: Color,
    pub flags: u8,
    /// Underline style, `UL_*`.
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

    /// Empty cell: nothing drawn beyond the default background.
    #[inline]
    pub fn is_blank(&self) -> bool {
        self.ch == ' ' && self.bg == Color::Default && self.flags == 0 && self.ul == UL_NONE
    }

    #[inline]
    pub fn is_cont(&self) -> bool {
        self.ch == WIDE_CONT
    }

    /// Did the pen change? Used by selection/copy to decide what counts as
    /// "space".
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
