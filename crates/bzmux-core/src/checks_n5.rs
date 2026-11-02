//! N5: histórico, âncora, dano e seleção (equivalente ao `checks.py:380-520`).
//!
//! Um check monta o cenário, exercita e compara com o esperado. Nada aqui
//! precisa de janela, GPU ou pty.

use crate::checks::Check;

/// Checks desta área, na ordem do relatório.
#[must_use]
pub fn all() -> Vec<Check> {
    Vec::new()
}
