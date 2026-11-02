//! Seleção: âncora absoluta e texto.
//!
//! A âncora é `(linha absoluta, coluna)` e **não** um par de coordenadas da
//! tela: assim a seleção sobrevive a rolar o histórico e a output chegando
//! (o texto anda, a seleção anda junto). É o mesmo modelo do `bzmux` Python
//! (`term.py:1050-1075`), com uma diferença: `scrolled` e `abs_gen` são
//! contadores aqui, não objetos.
//!
//! `abs_gen` é a geração do espaço absoluto: qualquer coisa que reescreve o
//! histórico por baixo (`CSI 3J`, RIS, reflow) incrementa e mata as âncoras
//! antigas — melhor perder a seleção que copiar o texto errado.

use crate::grid::Row;
use crate::term::Term;

/// Célula endereçada no espaço absoluto (histórico + grade viva).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Anchor {
    /// Linha absoluta: cresce com o scroll, não muda quando o histórico rola.
    pub line: i64,
    pub col: usize,
    /// Geração do espaço absoluto em que a âncora nasceu.
    pub gen: u64,
}

/// Seleção normalizada: `a <= b` na ordem de leitura.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Span {
    pub a: Anchor,
    pub b: Anchor,
}

impl Span {
    /// Ordena as pontas (arrastar de baixo para cima é normal).
    #[must_use]
    pub fn new(p: Anchor, q: Anchor) -> Span {
        if (p.line, p.col) <= (q.line, q.col) {
            Span { a: p, b: q }
        } else {
            Span { a: q, b: p }
        }
    }

    /// Interseção com a tela: `(r1, c1, r2, c2)` na visão com `view_off`.
    ///
    /// `None` = a seleção inteira está fora da tela (ou morreu). O que ficou
    /// para fora é preso na borda: o desenho pinta o pedaço visível e a cópia
    /// usa a âncora inteira.
    #[must_use]
    pub fn visible(&self, t: &Term, view_off: usize) -> Option<(usize, usize, usize, usize)> {
        if !t.anchor_alive(&self.a) || !t.anchor_alive(&self.b) {
            return None;
        }
        let rows = t.rows as i64;
        let r1 = t.anchor_row(&self.a, view_off);
        let r2 = t.anchor_row(&self.b, view_off);
        if r1 >= rows || r2 < 0 {
            return None;
        }
        let c1 = if r1 >= 0 { self.a.col } else { 0 };
        let c2 = if r2 < rows {
            self.b.col
        } else {
            t.cols.saturating_sub(1)
        };
        Some((r1.max(0) as usize, c1, (r2.min(rows - 1)) as usize, c2))
    }
}

impl Term {
    /// Âncora da célula `(row, col)` da visão com `off` linhas de histórico.
    #[must_use]
    pub fn anchor_at(&self, off: usize, row: usize, col: usize) -> Anchor {
        Anchor {
            line: self.scrolled as i64 - off as i64 + row as i64,
            col,
            gen: self.abs_gen,
        }
    }

    /// Primeira linha absoluta que ainda existe (o histórico tem teto).
    #[must_use]
    pub fn abs_first(&self) -> i64 {
        self.scrolled as i64 - self.scrollback.len() as i64
    }

    /// Fileira da visão onde a âncora cai (< 0 ou >= rows = fora da tela).
    #[must_use]
    pub fn anchor_row(&self, a: &Anchor, off: usize) -> i64 {
        a.line - self.scrolled as i64 + off as i64
    }

    /// A âncora ainda aponta para conteúdo que existe.
    #[must_use]
    pub fn anchor_alive(&self, a: &Anchor) -> bool {
        a.gen == self.abs_gen && self.abs_row(a.line).is_some()
    }

    /// A linha absoluta, se ela existe (histórico ou grade viva).
    #[must_use]
    pub fn abs_row(&self, line: i64) -> Option<&Row> {
        let lo = self.abs_first();
        if line < lo {
            return None;
        }
        if line < self.scrolled as i64 {
            self.scrollback.get((line - lo) as usize)
        } else {
            self.grid.get((line - self.scrolled as i64) as usize)
        }
    }

    /// Texto entre duas âncoras (pontas inclusivas), mesmo fora da tela.
    ///
    /// Espaço à direita de cada linha sai fora; âncora morta devolve `None`
    /// em vez de texto pela metade.
    #[must_use]
    pub fn span_text(&self, span: &Span) -> Option<String> {
        if !self.anchor_alive(&span.a) || !self.anchor_alive(&span.b) {
            return None;
        }
        let cols = self.cols;
        let mut out = Vec::with_capacity((span.b.line - span.a.line + 1) as usize);
        for line in span.a.line..=span.b.line {
            let row = self.abs_row(line)?;
            let from = if line == span.a.line { span.a.col } else { 0 }.min(cols);
            let to = if line == span.b.line {
                span.b.col
            } else {
                cols.saturating_sub(1)
            }
            .min(cols.saturating_sub(1));
            out.push(row.text_cols(from, to + 1));
        }
        Some(out.join("\n"))
    }

    /// Texto de um retângulo da visão `(r1,c1)..(r2,c2)`, pontas inclusivas.
    ///
    /// Usado para copiar a viewport quando não há seleção.
    #[must_use]
    pub fn selection_text(
        &self,
        start: (usize, usize),
        end: (usize, usize),
        rows: &[&Row],
    ) -> String {
        let ((r1, c1), (r2, c2)) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let last = rows.len().saturating_sub(1);
        let (r1, r2) = (r1.min(last), r2.min(last));
        let cols = self.cols;
        let mut out = Vec::with_capacity(r2 - r1 + 1);
        for (y, row) in rows.iter().enumerate().take(r2 + 1).skip(r1) {
            let from = if y == r1 { c1 } else { 0 };
            let to = if y == r2 { (c2 + 1).min(cols) } else { cols };
            out.push(row.text_cols(from, to));
        }
        out.join("\n")
    }

    /// A viewport inteira, sem linhas vazias no fim (Ctrl+Shift+C sem seleção).
    #[must_use]
    pub fn view_text(&self, off: usize) -> String {
        let mut lines: Vec<String> = self
            .visible_rows(off)
            .iter()
            .map(|r| r.text_cols(0, self.cols))
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        if lines.is_empty() {
            return String::new();
        }
        let mut s = lines.join("\n");
        s.push('\n');
        s
    }

    /// O que mandar para o filho quando o usuário cola `text`.
    ///
    /// Com `?2004` ligado o texto vai entre `ESC[200~` e `ESC[201~`: o app
    /// (readline, vim, python) sabe que é cola e não trata `\n` como Enter.
    /// A política mora aqui para ser testável sem display.
    #[must_use]
    pub fn paste_payload(&self, text: &str) -> Vec<u8> {
        if self.bracketed_paste {
            let mut v = Vec::with_capacity(text.len() + 12);
            v.extend_from_slice(b"\x1b[200~");
            v.extend_from_slice(text.as_bytes());
            v.extend_from_slice(b"\x1b[201~");
            v
        } else {
            text.as_bytes().to_vec()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term() -> Term {
        Term::new(10, 4)
    }

    #[test]
    fn ancora_sobrevive_ao_scroll() {
        let mut t = term();
        // três linhas: enche a tela, depois rola uma
        t.feed(b"AAA\r\nBBB\r\nCCC\r\nDDD");
        let a = t.anchor_at(0, 1, 0); // "BBB"
        t.feed(b"\r\nEEE");
        let b = t.anchor_at(0, 2, 1); // "CCC" col 1
        let txt = t.span_text(&Span::new(a, b)).unwrap();
        assert_eq!(txt, "BBB\nCCC\nDD", "o texto tem que andar junto");
    }

    #[test]
    fn ancora_morre_quando_o_historico_e_reescrito() {
        let mut t = term();
        t.feed(b"AAA\r\nBBB");
        let a = t.anchor_at(0, 0, 0);
        assert!(t.anchor_alive(&a));
        t.feed(b"\x1b[3J"); // ED3: limpa o histórico
        assert!(!t.anchor_alive(&a));
        assert_eq!(t.span_text(&Span::new(a, a)), None);
    }

    #[test]
    fn span_normaliza_e_corta_espaco_a_direita() {
        let mut t = term();
        t.feed(b"oi   \r\nmundo  ");
        let a = t.anchor_at(0, 1, 4); // "o" de "mundo" (ponta inclusiva)
        let b = t.anchor_at(0, 0, 0); // início de "oi"
        let txt = t.span_text(&Span::new(a, b)).unwrap();
        assert_eq!(txt, "oi\nmundo");
    }

    #[test]
    fn visible_prende_a_selecao_na_borda() {
        let mut t = Term::new(6, 2);
        t.feed(b"111111\r\n222222\r\n333333\r\n444444");
        // seleciona da linha 2 até a 3, mas a visão mostra 2 linhas
        let a = t.anchor_at(0, 0, 0);
        let b = t.anchor_at(0, 1, 5);
        let (r1, c1, r2, c2) = Span::new(a, b).visible(&t, 0).unwrap();
        assert_eq!((r1, c1, r2, c2), (0, 0, 1, 5));
        // rolar duas linhas para trás tira a seleção da tela
        assert_eq!(Span::new(a, b).visible(&t, 3), None);
    }

    #[test]
    fn view_text_corta_linhas_vazias_do_fim() {
        let mut t = term();
        t.feed(b"um\r\ndois");
        assert_eq!(t.view_text(0), "um\ndois\n");
        let vazio = super::tests::term(); // tela limpa -> nada
        assert_eq!(vazio.view_text(0), "");
    }

    #[test]
    fn paste_respeita_bracketed_paste() {
        let mut t = term();
        assert_eq!(t.paste_payload("a\nb"), b"a\nb");
        t.feed(b"\x1b[?2004h");
        assert_eq!(t.paste_payload("a\nb"), b"\x1b[200~a\nb\x1b[201~");
    }
}
