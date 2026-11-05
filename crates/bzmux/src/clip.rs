//! Clipboard (text only).
//!
//! `arboard` speaks X11 (and Wayland, with the feature) by speaking the protocol
//! directly: it does not link `libX11`. Text only, no image — hence
//! `default-features = false` in `Cargo.toml` (an image would pull
//! tiff/png/jpeg into the binary).
//!
//! The `Clipboard` is created the first time someone uses it, not at start: in a
//! session without a display (or with the server down) the app still opens, and
//! copy/paste only fails when it is actually called.

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

    /// CLIPBOARD text.
    pub fn text(&mut self) -> Option<String> {
        self.board()?.get_text().ok()
    }

    /// PRIMARY text (the X11 selection). Outside Linux there is none.
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

    /// Puts it in PRIMARY (X11) — Linux' "paste with the middle button".
    pub fn set_primary(&mut self, text: &str) {
        #[cfg(target_os = "linux")]
        {
            use arboard::{LinuxClipboardKind, SetExtLinux};
            if let Some(b) = self.board() {
                // no `wait()`: the app stays the selection owner until the
                // next one, which is the normal terminal behaviour
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
