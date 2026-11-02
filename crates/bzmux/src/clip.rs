//! Área de transferência (só texto).
//!
//! `arboard` fala X11 (e Wayland, com a feature) falando o protocolo direto:
//! não linka `libX11`. Só texto, sem imagem — daí `default-features = false`
//! no `Cargo.toml` (imagem puxaria tiff/png/jpeg para dentro do binário).
//!
//! O `Clipboard` é criado na primeira vez que alguém usa, não no start: em
//! sessão sem display (ou com o servidor fora do ar) o app continua abrindo, e
//! copiar/colar só falha quando é chamado.

pub struct Clip {
    board: Option<arboard::Clipboard>,
}

impl Clip {
    pub fn new() -> Clip {
        Clip { board: None }
    }

    fn board(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.board.is_none() {
            self.board = arboard::Clipboard::new().ok();
        }
        self.board.as_mut()
    }

    /// Texto do CLIPBOARD.
    pub fn text(&mut self) -> Option<String> {
        self.board()?.get_text().ok()
    }

    /// Texto do PRIMARY (seleção do X11). Fora do Linux não existe.
    pub fn primary_text(&mut self) -> Option<String> {
        #[cfg(target_os = "linux")]
        {
            use arboard::{GetExtLinux, LinuxClipboardKind};
            self.board()?
                .get()
                .clipboard(LinuxClipboardKind::Primary)
                .text()
                .ok()
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.text()
        }
    }

    pub fn set(&mut self, text: &str) {
        if let Some(b) = self.board() {
            let _ = b.set_text(text.to_string());
        }
    }

    /// Põe no PRIMARY (X11) — o "colar com o botão do meio" do Linux.
    pub fn set_primary(&mut self, text: &str) {
        #[cfg(target_os = "linux")]
        {
            use arboard::{LinuxClipboardKind, SetExtLinux};
            if let Some(b) = self.board() {
                // `wait()` não: o app segue dono da seleção até a próxima,
                // que é o comportamento normal de terminal
                let _ = b
                    .set()
                    .clipboard(LinuxClipboardKind::Primary)
                    .text(text.to_string());
            }
        }
        #[cfg(not(target_os = "linux"))]
        self.set(text);
    }
}
