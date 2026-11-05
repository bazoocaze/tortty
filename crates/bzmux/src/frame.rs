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
    sync_until: Option<Instant>,
    /// O `?2026` estava aberto na última leitura: é assim que se vê o fecho.
    sync_was: bool,
}

impl Gate {
    /// Chegou mais saída do pty. `sync` é o `?2026` da aba visível **agora**
    /// (por isso o chamador lê o parser depois de alimentá-lo, não antes).
    pub fn request(&mut self, now: Instant, sync: bool) {
        if sync {
            // janela viva é mantida; teto estourado abre outra, senão o app
            // que nunca fecha o `?2026` desenharia a cada leitura
            let until = match self.sync_until {
                Some(a) if a > now => a,
                _ => now + Duration::from_millis(SYNC_MS),
            };
            self.sync_until = Some(until);
            self.sync_was = true;
            self.frame_at = Some(until);
            return;
        }
        // fechou a transação: o estado final sai agora, sem esperar o ritmo
        let fechou = std::mem::take(&mut self.sync_was);
        self.sync_until = None;
        if fechou {
            self.frame_at = Some(now);
            return;
        }
        // coalescido: quem já tem hora marcada não remarca (é o ponto)
        self.frame_at
            .get_or_insert(now + Duration::from_millis(FRAME_MS));
    }

    /// O frame venceu? Devolve `true` uma vez só e libera o portão.
    pub fn venceu(&mut self, now: Instant) -> bool {
        if self.frame_at.is_some_and(|t| now >= t) {
            self.frame_at = None;
            return true;
        }
        false
    }

    /// Quando acordar para desenhar (para o `ControlFlow::WaitUntil`).
    #[must_use]
    pub fn when(&self) -> Option<Instant> {
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
    fn leituras_seguidas_desenham_a_time() {
        let mut g = Gate::default();
        let t = t0();
        g.request(t, false);
        // outra leitura 5 ms depois não remarca nada: um frame só
        g.request(t + Duration::from_millis(5), false);
        let time = g.when().expect("tem frame marcado");
        assert_eq!(time, t + Duration::from_millis(FRAME_MS));
        assert!(!g.venceu(t + Duration::from_millis(FRAME_MS - 1)));
        assert!(g.venceu(t + Duration::from_millis(FRAME_MS)));
        // e o portão liberou: não vence duas vezes com a mesma hora
        assert!(!g.venceu(t + Duration::from_millis(FRAME_MS)));
        // saída nova depois do frame marca outro
        g.request(t + Duration::from_millis(FRAME_MS + 1), false);
        assert!(g.when().is_some());
    }

    #[test]
    fn sync_open_segura_until_o_closure() {
        let mut g = Gate::default();
        let t = t0();
        g.request(t, true);
        // nada sai no ritmo normal: a transação está aberta
        assert!(!g.venceu(t + Duration::from_millis(FRAME_MS)));
        // um pedido a mais não estica a janela
        g.request(t + Duration::from_millis(100), true);
        assert_eq!(g.when(), Some(t + Duration::from_millis(SYNC_MS)));
        // fechou: o estado final sai na hora
        g.request(t + Duration::from_millis(120), false);
        assert_eq!(g.when(), Some(t + Duration::from_millis(120)));
        assert!(g.venceu(t + Duration::from_millis(120)));
    }

    #[test]
    fn sync_que_nunca_fecha_draws_no_cap_e_not_a_each_reading() {
        let mut g = Gate::default();
        let t = t0();
        g.request(t, true);
        assert!(g.venceu(t + Duration::from_millis(SYNC_MS)));
        // ainda aberto: nova janela (senão desenharia a cada leitura daqui
        // para a frente, que é o bug que o portão existe para evitar)
        let after = t + Duration::from_millis(SYNC_MS + 1);
        g.request(after, true);
        assert_eq!(g.when(), Some(after + Duration::from_millis(SYNC_MS)));
        assert!(!g.venceu(after + Duration::from_millis(FRAME_MS)));
    }

    #[test]
    fn without_request_not_ha_frame() {
        let mut g = Gate::default();
        assert_eq!(g.when(), None);
        assert!(!g.venceu(t0()));
    }
}
