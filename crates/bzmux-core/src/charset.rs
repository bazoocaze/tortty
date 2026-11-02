//! Charset DEC Special Graphics (o que desenha as molduras do `mc`).
//!
//! `ESC ( 0` designa o G0 como gráfico: as letras `l q k x m j` passam a ser
//! cantos e traços. Sem isso o `mc` desenha `lqqqqqk` em volta do painel — o
//! teste barato de "o terminal está são".

const fn table() -> [char; 96] {
    let mut t = ['\0'; 96];
    // 0x5f ..= 0x7e
    t[0x5f - 0x20] = '\u{00a0}'; // espaço duro no lugar do `_`
    t[0x60 - 0x20] = '◆';
    t[0x61 - 0x20] = '▒';
    t[0x62 - 0x20] = '␉';
    t[0x63 - 0x20] = '␌';
    t[0x64 - 0x20] = '␍';
    t[0x65 - 0x20] = '␊';
    t[0x66 - 0x20] = '°';
    t[0x67 - 0x20] = '±';
    t[0x68 - 0x20] = '␤';
    t[0x69 - 0x20] = '␋';
    t[0x6a - 0x20] = '┘';
    t[0x6b - 0x20] = '┐';
    t[0x6c - 0x20] = '┌';
    t[0x6d - 0x20] = '└';
    t[0x6e - 0x20] = '┼';
    t[0x6f - 0x20] = '⎺';
    t[0x70 - 0x20] = '⎻';
    t[0x71 - 0x20] = '─';
    t[0x72 - 0x20] = '⎼';
    t[0x73 - 0x20] = '⎽';
    t[0x74 - 0x20] = '├';
    t[0x75 - 0x20] = '┤';
    t[0x76 - 0x20] = '┴';
    t[0x77 - 0x20] = '┬';
    t[0x78 - 0x20] = '│';
    t[0x79 - 0x20] = '≤';
    t[0x7a - 0x20] = '≥';
    t[0x7b - 0x20] = 'π';
    t[0x7c - 0x20] = '≠';
    t[0x7d - 0x20] = '£';
    t[0x7e - 0x20] = '·';
    t
}

static DEC_TABLE: [char; 96] = table();

/// Traduz um caractere ASCII para o gráfico DEC, se a tabela cobrir.
#[inline]
pub fn dec_graphics(ch: char) -> char {
    let c = ch as u32;
    if (0x5f..0x7f).contains(&c) {
        let m = DEC_TABLE[(c - 0x20) as usize];
        if m != '\0' {
            return m;
        }
    }
    ch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn molduras_do_mc() {
        assert_eq!(dec_graphics('l'), '┌');
        assert_eq!(dec_graphics('q'), '─');
        assert_eq!(dec_graphics('k'), '┐');
        assert_eq!(dec_graphics('x'), '│');
        assert_eq!(dec_graphics('m'), '└');
        assert_eq!(dec_graphics('j'), '┘');
        // fora da faixa gráfica não muda nada
        assert_eq!(dec_graphics('A'), 'A');
    }
}
