//! Unicode placeholders for images (`M-L`): the private-use character
//! `U+10EEEE` plus combining diacritics say *which* image and *which cell* of it
//! a grid cell shows.
//!
//! The diacritic for a number is not arithmetic — it is a list, taken from the
//! protocol's `rowcolumn-diacritics.txt` (combining marks of class 230 without
//! decomposition, so no renderer fuses them into a precomposed character). The
//! order *is* the number.

/// The placeholder character: a cell holding it is a pixel of an image.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// Number → diacritic. `None` past the end of the list: a client that sends more
/// rows or columns than the protocol can address gets nothing, not a guess.
#[rustfmt::skip]
const DIACRITICS: [u32; 297] = [
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F,
    0x0346, 0x034A, 0x034B, 0x034C, 0x0350, 0x0351, 0x0352, 0x0357,
0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369,
0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F, 0x0483, 0x0484,
0x0485, 0x0486, 0x0487, 0x0592, 0x0593, 0x0594, 0x0595, 0x0597,
0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1,
0x05A8, 0x05A9, 0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611,
0x0612, 0x0613, 0x0614, 0x0615, 0x0616, 0x0617, 0x0657, 0x0658,
0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6, 0x06D7, 0x06D8,
0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2,
0x06E4, 0x06E7, 0x06E8, 0x06EB, 0x06EC, 0x0730, 0x0732, 0x0733,
0x0735, 0x0736, 0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743,
0x0745, 0x0747, 0x0749, 0x074A, 0x07EB, 0x07EC, 0x07ED, 0x07EE,
0x07EF, 0x07F0, 0x07F1, 0x07F3, 0x0816, 0x0817, 0x0818, 0x0819,
0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822,
0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A, 0x082B, 0x082C,
0x082D, 0x0951, 0x0953, 0x0954, 0x0F82, 0x0F83, 0x0F86, 0x0F87,
0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D,
0x1B6E, 0x1B6F, 0x1B70, 0x1B71, 0x1B72, 0x1B73, 0x1CD0, 0x1CD1,
0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1,
0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5, 0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9,
0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1,
0x20D4, 0x20D5, 0x20D6, 0x20D7, 0x20DB, 0x20DC, 0x20E1, 0x20E7,
0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0xA66F, 0xA67C, 0xA67D,
0xA6F0, 0xA6F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE,
0xAABF, 0xAAC1, 0x10A0F, 0x10A38, 0x1D242, 0x1D243, 0x1D244, 0x2DE0,
0x2DE1, 0x2DE2, 0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8,
0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC, 0x2DED, 0x2DEE, 0x2DEF, 0x2DF0,
0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8,
0x2DF9, 0x2DFA, 0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA8E0,
0xA8E1, 0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8,
0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED, 0xA8EE, 0xA8EF, 0xA8F0,
0xA8F1, 0xFE20, 0xFE21, 0xFE22, 0xFE23, 0xFE24, 0xFE25, 0xFE26,
0x1D185, 0x1D186, 0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC,
0x1D1AD,
];

/// The diacritic → number, or `None` when the mark is not one of them.
#[must_use]
pub fn number(ch: char) -> Option<u8> {
    let cp = u32::from(ch);
    DIACRITICS
        .binary_search(&cp)
        .ok()
        .or_else(|| DIACRITICS.iter().position(|d| *d == cp))
        .and_then(|i| u8::try_from(i).ok())
}

/// Which cell of which image a placeholder cell shows.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Cell {
    /// Row inside the image's placeholder grid (`r=` of the virtual placement).
    pub row: u16,
    /// Column inside it (`c=`).
    pub col: u16,
    /// The top byte of the image id, when the id does not fit the colour.
    pub msb: u8,
}

impl Cell {
    /// The image id this cell belongs to, given the low 24 bits the foreground
    /// colour carries.
    #[must_use]
    pub fn image(&self, low: u32) -> u32 {
        (low & 0x00FF_FFFF) | (u32::from(self.msb) << 24)
    }
}

/// Decodes one placeholder cell. `prev` is the cell to the left when it was also
/// a placeholder **with the same foreground and underline colours**, which is
/// what the protocol's inheritance rules are allowed to read.
///
/// The three rules, in the spec's order: no diacritics → inherit row, column
/// plus one and the top byte from the left; only the row → inherit the column
/// plus one (when the row matches); row and column → inherit only the top byte.
/// Everything not inherited is zero.
#[must_use]
pub fn decode(marks: &[char], prev: Option<Cell>) -> Cell {
    let nums: Vec<u8> = marks.iter().filter_map(|c| number(*c)).collect();
    let row = nums.first().copied();
    let col = nums.get(1).copied();
    let msb = nums.get(2).copied();
    match (row, col, msb) {
        (Some(row), Some(col), Some(msb)) => Cell {
            row: u16::from(row),
            col: u16::from(col),
            msb,
        },
        (Some(row), Some(col), None) => Cell {
            row: u16::from(row),
            col: u16::from(col),
            // the top byte comes from the left only when it sits right before
            msb: match prev {
                Some(p) if p.row == u16::from(row) && p.col + 1 == u16::from(col) => p.msb,
                _ => 0,
            },
        },
        (Some(row), None, _) => {
            let row = u16::from(row);
            Cell {
                row,
                col: match prev {
                    Some(p) if p.row == row => p.col.saturating_add(1),
                    _ => 0,
                },
                msb: match prev {
                    Some(p) if p.row == row => p.msb,
                    _ => 0,
                },
            }
        },
        (None, ..) => match prev {
            Some(p) => Cell {
                row: p.row,
                col: p.col.saturating_add(1),
                msb: p.msb,
            },
            None => Cell::default(),
        },
    }
}

/// The `uv` rectangle of cell (`row`, `col`) inside a `cols` × `rows` placeholder
/// grid: the picture is cut into that grid, one cell per cell of text.
#[must_use]
pub fn uv(cell: Cell, cols: u16, rows: u16) -> [f32; 4] {
    let cols = cols.max(1);
    let rows = rows.max(1);
    let part = |v: u16, of: u16| f32::from(v.min(of)) / f32::from(of);
    let (x0, y0) = (part(cell.col, cols), part(cell.row, rows));
    let (x1, y1) = (
        part(cell.col.saturating_add(1), cols),
        part(cell.row.saturating_add(1), rows),
    );
    [x0, y0, x1, y1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_numbers_are_the_diacritics_of_the_list() {
        // the spec's own examples: `U+305` is 0 and `U+30D` is 1
        assert_eq!(number('\u{305}'), Some(0));
        assert_eq!(number('\u{30D}'), Some(1));
        assert_eq!(number('\u{30E}'), Some(2));
        // a mark that is not in the list is not a number
        assert_eq!(
            number('\u{300}'),
            None,
            "a fusable accent is not in the list"
        );
    }

    #[test]
    fn a_full_cell_says_where_it_is() {
        let cell = decode(&['\u{30E}', '\u{30D}', '\u{305}'], None);
        assert_eq!(
            cell,
            Cell {
                row: 2,
                col: 1,
                msb: 0
            }
        );
        // the top byte of the image id rides in the third diacritic
        let high = decode(&['\u{305}', '\u{305}', '\u{30E}'], None);
        assert_eq!(high.msb, 2);
        assert_eq!(high.image(42), 42 + (2 << 24));
    }

    #[test]
    fn missing_diacritics_inherit_from_the_cell_to_the_left_only() {
        let left = Cell {
            row: 1,
            col: 4,
            msb: 0,
        };
        // no diacritics: row, column plus one and the top byte
        assert_eq!(
            decode(&[], Some(left)),
            Cell {
                row: 1,
                col: 5,
                msb: 0
            }
        );
        // only the row: the column plus one when the row matches
        assert_eq!(
            decode(&['\u{30D}'], Some(left)),
            Cell {
                row: 1,
                col: 5,
                msb: 0
            }
        );
        assert_eq!(
            decode(&['\u{305}'], Some(left)),
            Cell {
                row: 0,
                col: 0,
                msb: 0
            },
            "a different row inherits nothing"
        );
        // row and column: only the top byte
        let left = Cell {
            row: 0,
            col: 0,
            msb: 3,
        };
        assert_eq!(
            decode(&['\u{305}', '\u{30D}'], Some(left)),
            Cell {
                row: 0,
                col: 1,
                msb: 3
            }
        );
        assert_eq!(
            decode(&['\u{305}', '\u{30E}'], Some(left)),
            Cell {
                row: 0,
                col: 2,
                msb: 0
            },
            "the top byte only comes from the cell right before"
        );
    }

    /// The cell's slice of the picture: the grid of placeholders is the picture.
    #[test]
    fn the_uv_of_a_cell_is_its_slice_of_the_picture() {
        let cell = Cell {
            row: 1,
            col: 1,
            msb: 0,
        };
        assert_eq!(uv(cell, 2, 2), [0.5, 0.5, 1.0, 1.0]);
        assert_eq!(uv(Cell::default(), 2, 2), [0.0, 0.0, 0.5, 0.5]);
        // past the grid it clamps instead of leaving the picture
        let far = Cell {
            row: 9,
            col: 9,
            msb: 0,
        };
        assert_eq!(uv(far, 2, 2), [1.0, 1.0, 1.0, 1.0]);
    }
}
