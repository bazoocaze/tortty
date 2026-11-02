//! Grade: linha e utilitários de texto.

use crate::cell::Cell;

/// Linha da grade. `wrapped` é a quebra mole (autowrap): a linha continua na
/// de baixo. O parser usa isso para (futuro) reflow e para concatenar texto.
#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
}

impl Row {
    pub fn blank(cols: usize) -> Row {
        Row {
            cells: vec![Cell::BLANK; cols],
            wrapped: false,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Texto da linha sem os espaços à direita (para o `text` do socket).
    pub fn text(&self) -> String {
        let mut s: String = self
            .cells
            .iter()
            .filter(|c| !c.is_cont())
            .map(|c| c.ch)
            .collect();
        while s.ends_with(' ') {
            s.pop();
        }
        s
    }

    /// Texto das colunas `a..b` (fim exclusivo), sem cont e sem espaço à
    /// direita. É a base da cópia: cortar por coluna, não por caractere.
    #[must_use]
    pub fn text_cols(&self, a: usize, b: usize) -> String {
        let b = b.min(self.cells.len());
        let a = a.min(b);
        let mut s: String = self.cells[a..b]
            .iter()
            .filter(|c| !c.is_cont())
            .map(|c| c.ch)
            .collect();
        while s.ends_with(' ') {
            s.pop();
        }
        s
    }

    /// Redimensiona a linha (cresce com célula em branco, corta o excedente).
    pub fn resize(&mut self, cols: usize) {
        self.cells.resize(cols, Cell::BLANK);
    }
}
