//! N2: configuração e linha de comando (equivalente ao `checks.py:125-287`).
//!
//! Um check monta o cenário, exercita e compara com o esperado. Nada aqui
//! precisa de janela, GPU ou pty.

use crate::checks::Check;

/// Checks desta área, na ordem do relatório.
#[must_use]
pub fn all() -> Vec<Check> {
    Vec::new()
}
