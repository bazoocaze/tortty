//! Dano: quais linhas da grade precisam ser remontadas neste frame (A17/G17).
//!
//! O equivalente no Python é o `queue_draw_area` do GTK (`view.py:330-364`):
//! quem escreve marca a linha, e o desenho só refaz o que sujou. Aqui a marca já
//! existe no núcleo (`Term::dirty` / `dirty_all`); o que faltava era o desenho
//! usá-la em vez de remontar a grade inteira a cada frame.
//!
//! A decisão mora aqui, num módulo puro, por um motivo: é onde é fácil errar.
//! Marcar "só a linha suja" é seguro apenas quando **nada global** mudou — se a
//! visão rolou, se a seleção apareceu, se o tema trocou ou se o número de linhas
//! mudou, todas as linhas são outras. Isso é a [`Assinatura`]: se ela mudou, o
//! frame é inteiro; se não mudou, vale o dano linha a linha. Guardar essa regra
//! num só lugar (em vez de espalhá-la pelo `app` e pelo `render`) é o que evita
//! o bug clássico de "aparece conteúdo velho até rolar a tela".

/// Tudo o que muda a tela inteira de uma vez.
///
/// Deliberadamente **não** entram aqui coisas que são desenhadas fora da grade
/// (cursor, barra de abas, sidebar, toast, badge de histórico): elas não moram
/// em [`crate::render::Renderer::por_linha`] e são remontadas todo frame de
/// qualquer jeito.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Assinatura {
    /// Qual aba/terminal está na tela: trocar de aba troca todo o conteúdo.
    pub aba: u64,
    pub cols: usize,
    /// Linhas visíveis (muda quando a barra de abas aparece, por exemplo).
    pub rows: usize,
    /// Quantas linhas o histórico subiu: muda o conteúdo de todas as linhas.
    pub view_off: usize,
    /// Seleção visível, já recortada na tela: `(linha1, col1, linha2, col2)`.
    pub sel: Option<(usize, usize, usize, usize)>,
    /// Fase do pisca **quando existe** célula com SGR 5 à vista. Sem isso o
    /// pisca do cursor obrigaria um rebuild completo duas vezes por segundo.
    pub blink: bool,
    /// Geração do tema: paleta, fg/bg/cursor e fonte mudam a cor de tudo.
    pub tema: u64,
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
}

/// O que remontar neste frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Dano {
    /// Tudo: mudou algo de tela inteira (ou o dano do núcleo é global).
    Tudo,
    /// Só estas linhas (as outras ficam como estão).
    Linhas(Vec<usize>),
}

/// Decide o dano do frame e já atualiza a assinatura guardada.
///
/// `sujas` é o dano do núcleo (uma posição por linha da grade). Linha que não
/// existe no vetor conta como suja: dano desconhecido nunca vira tela velha.
pub fn decide(
    nova: &Assinatura,
    anterior: &mut Assinatura,
    tudo: bool,
    sujas: &[bool],
    rows: usize,
) -> Dano {
    let mudou = *anterior != *nova;
    anterior.clone_from(nova);
    if tudo || mudou {
        return Dano::Tudo;
    }
    let mut linhas = Vec::new();
    for y in 0..rows {
        if sujas.get(y).copied().unwrap_or(true) {
            linhas.push(y);
        }
    }
    Dano::Linhas(linhas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assinatura() -> Assinatura {
        Assinatura {
            aba: 1,
            cols: 80,
            rows: 24,
            view_off: 0,
            sel: None,
            blink: false,
            tema: 1,
            fg: (0xd6, 0xd6, 0xd6),
            bg: (0, 0, 0),
        }
    }

    #[test]
    fn trocar_de_aba_remonta_tudo() {
        // duas abas com a mesma geometria: reaproveitar a linha da outra
        // mostraria o conteúdo errado
        let mut anterior = assinatura();
        let mut nova = assinatura();
        nova.aba = 2;
        assert_eq!(
            decide(&nova, &mut anterior, false, &[false; 24], 24),
            Dano::Tudo
        );
    }

    #[test]
    fn so_a_linha_suja_e_remontada() {
        let mut anterior = assinatura();
        let mut sujas = vec![false; 24];
        sujas[7] = true;
        let d = decide(&assinatura(), &mut anterior, false, &sujas, 24);
        assert_eq!(d, Dano::Linhas(vec![7]));
    }

    #[test]
    fn nada_sujo_nao_remonta_nada() {
        let mut anterior = assinatura();
        let d = decide(&assinatura(), &mut anterior, false, &[false; 24], 24);
        assert_eq!(d, Dano::Linhas(vec![]));
    }

    #[test]
    fn dano_global_remonta_tudo() {
        let mut anterior = assinatura();
        assert_eq!(
            decide(&assinatura(), &mut anterior, true, &[false; 24], 24),
            Dano::Tudo
        );
    }

    #[test]
    fn rolar_a_visao_remonta_tudo() {
        let mut anterior = assinatura();
        let mut nova = assinatura();
        nova.view_off = 3;
        assert_eq!(
            decide(&nova, &mut anterior, false, &[false; 24], 24),
            Dano::Tudo
        );
    }

    #[test]
    fn selecao_remonta_tudo() {
        let mut anterior = assinatura();
        let mut nova = assinatura();
        nova.sel = Some((3, 1, 3, 5));
        assert_eq!(
            decide(&nova, &mut anterior, false, &[false; 24], 24),
            Dano::Tudo
        );
    }

    #[test]
    fn trocar_o_tema_remonta_tudo() {
        let mut anterior = assinatura();
        let mut nova = assinatura();
        nova.tema = 2;
        assert_eq!(
            decide(&nova, &mut anterior, false, &[false; 24], 24),
            Dano::Tudo
        );
    }

    #[test]
    fn linha_numero_apareceu_remonta_tudo() {
        // 24 → 23 linhas (a barra de abas roubou uma): tudo é outra linha
        let mut anterior = assinatura();
        let mut nova = assinatura();
        nova.rows = 23;
        assert_eq!(
            decide(&nova, &mut anterior, false, &[false; 24], 23),
            Dano::Tudo
        );
    }

    #[test]
    fn pisca_com_celula_sgr5_remonta_tudo() {
        let mut anterior = assinatura();
        let mut nova = assinatura();
        nova.blink = true;
        assert_eq!(
            decide(&nova, &mut anterior, false, &[false; 24], 24),
            Dano::Tudo
        );
    }

    #[test]
    fn dano_desconhecido_conta_como_sujo() {
        // `sujas` menor que as linhas: o que falta é remontado, não ignorado
        let mut anterior = assinatura();
        let d = decide(&assinatura(), &mut anterior, false, &[false, true], 4);
        assert_eq!(d, Dano::Linhas(vec![1, 2, 3]));
    }

    #[test]
    fn a_assinatura_e_guardada_para_o_frame_seguinte() {
        let mut anterior = assinatura();
        let mut nova = assinatura();
        nova.view_off = 5;
        let _ = decide(&nova, &mut anterior, false, &[false; 24], 24);
        // o frame seguinte, igual, já não é "tudo"
        let d = decide(&nova, &mut anterior, false, &[false; 24], 24);
        assert_eq!(d, Dano::Linhas(vec![]));
    }
}
