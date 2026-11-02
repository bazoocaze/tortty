//! Portão do desenho: no máximo um frame a cada ~16 ms, e nada enquanto o
//! `?2026` (synchronized output) estiver aberto — G16/B28/G15.
//!
//! Por que existe: `request_redraw` a cada leitura de pty dava um desenho por
//! pedaço de leitura sob despejo contínuo, e o custo do desenho passava a
//! competir com a leitura (a tela ainda congelava, porque o redraw tem
//! prioridade menor que o watch do pty no Python). É a mesma decisão do
//! `view.py:286-320`: coalescer e ritmar. Com `?2026` aberto o app ainda vai
//! redesenhar, então desenhar agora mostra o **meio** de cada repaint — segura
//! até o fecho, com teto de 200 ms para app que esquece de fechar.
//!
//! Fica num tipo puro e sem `winit` de propósito: o ritmo é testável sem
//! display, e o `app.rs` só pergunta "vencido?" e "quando?".

use std::time::{Duration, Instant};

/// Ritmo do frame coalescido: ~60 Hz.
pub const FRAME_MS: u64 = 16;
/// Teto da transação do `?2026`: app que abre e não fecha não congela a tela.
pub const SYNC_MS: u64 = 200;

#[derive(Debug, Default)]
pub struct Gate {
    /// Quando o próximo frame pode sair; `None` = nada pendente.
    frame_at: Option<Instant>,
    /// Fim da janela do `?2026` em curso (o teto de segurança).
    sync_ate: Option<Instant>,
    /// O `?2026` estava aberto na última leitura: é assim que se vê o fecho.
    sync_era: bool,
}

impl Gate {
    /// Chegou mais saída do pty. `sync` é o `?2026` da aba visível **agora**
    /// (por isso o chamador lê o parser depois de alimentá-lo, não antes).
    pub fn pedir(&mut self, agora: Instant, sync: bool) {
        if sync {
            // janela viva é mantida; teto estourado abre outra, senão o app
            // que nunca fecha o `?2026` desenharia a cada leitura
            let ate = match self.sync_ate {
                Some(a) if a > agora => a,
                _ => agora + Duration::from_millis(SYNC_MS),
            };
            self.sync_ate = Some(ate);
            self.sync_era = true;
            self.frame_at = Some(ate);
            return;
        }
        // fechou a transação: o estado final sai agora, sem esperar o ritmo
        let fechou = std::mem::take(&mut self.sync_era);
        self.sync_ate = None;
        if fechou {
            self.frame_at = Some(agora);
            return;
        }
        // coalescido: quem já tem hora marcada não remarca (é o ponto)
        self.frame_at
            .get_or_insert(agora + Duration::from_millis(FRAME_MS));
    }

    /// O frame venceu? Devolve `true` uma vez só e libera o portão.
    pub fn venceu(&mut self, agora: Instant) -> bool {
        if self.frame_at.is_some_and(|t| agora >= t) {
            self.frame_at = None;
            return true;
        }
        false
    }

    /// Quando acordar para desenhar (para o `ControlFlow::WaitUntil`).
    #[must_use]
    pub fn quando(&self) -> Option<Instant> {
        self.frame_at
    }
}

#[cfg(test)]
mod checks_frame {
    use super::{Gate, FRAME_MS, SYNC_MS};
    use std::time::{Duration, Instant};

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn leituras_seguidas_desenham_uma_vez() {
        let mut g = Gate::default();
        let t = t0();
        g.pedir(t, false);
        // outra leitura 5 ms depois não remarca nada: um frame só
        g.pedir(t + Duration::from_millis(5), false);
        let hora = g.quando().expect("tem frame marcado");
        assert_eq!(hora, t + Duration::from_millis(FRAME_MS));
        assert!(!g.venceu(t + Duration::from_millis(FRAME_MS - 1)));
        assert!(g.venceu(t + Duration::from_millis(FRAME_MS)));
        // e o portão liberou: não vence duas vezes com a mesma hora
        assert!(!g.venceu(t + Duration::from_millis(FRAME_MS)));
        // saída nova depois do frame marca outro
        g.pedir(t + Duration::from_millis(FRAME_MS + 1), false);
        assert!(g.quando().is_some());
    }

    #[test]
    fn sync_aberto_segura_ate_o_fecho() {
        let mut g = Gate::default();
        let t = t0();
        g.pedir(t, true);
        // nada sai no ritmo normal: a transação está aberta
        assert!(!g.venceu(t + Duration::from_millis(FRAME_MS)));
        // um pedido a mais não estica a janela
        g.pedir(t + Duration::from_millis(100), true);
        assert_eq!(g.quando(), Some(t + Duration::from_millis(SYNC_MS)));
        // fechou: o estado final sai na hora
        g.pedir(t + Duration::from_millis(120), false);
        assert_eq!(g.quando(), Some(t + Duration::from_millis(120)));
        assert!(g.venceu(t + Duration::from_millis(120)));
    }

    #[test]
    fn sync_que_nunca_fecha_desenha_no_teto_e_nao_a_cada_leitura() {
        let mut g = Gate::default();
        let t = t0();
        g.pedir(t, true);
        assert!(g.venceu(t + Duration::from_millis(SYNC_MS)));
        // ainda aberto: nova janela (senão desenharia a cada leitura daqui
        // para a frente, que é o bug que o portão existe para evitar)
        let depois = t + Duration::from_millis(SYNC_MS + 1);
        g.pedir(depois, true);
        assert_eq!(g.quando(), Some(depois + Duration::from_millis(SYNC_MS)));
        assert!(!g.venceu(depois + Duration::from_millis(FRAME_MS)));
    }

    #[test]
    fn sem_pedido_nao_ha_frame() {
        let mut g = Gate::default();
        assert_eq!(g.quando(), None);
        assert!(!g.venceu(t0()));
    }
}
