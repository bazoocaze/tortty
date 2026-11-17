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

use crate::platform;

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

    /// PRIMARY text (the X11 selection). Where the system has no second
    /// selection, this is CLIPBOARD: the seam answers, not a `#[cfg]` here.
    pub fn primary_text(&mut self) -> Option<String> {
        if !platform::has_primary() {
            return self.text();
        }
        platform::primary_get(self.board()?)
    }

    pub fn set(&mut self, text: &str) {
        if let Some(b) = self.board() {
            let _ = b.set_text(text.to_string());
        }
    }

    /// Puts it in PRIMARY (X11) — the "paste with the middle button" — or in
    /// CLIPBOARD where there is no second selection.
    pub fn set_primary(&mut self, text: &str) {
        if !platform::has_primary() {
            self.set(text);
            return;
        }
        if let Some(b) = self.board() {
            platform::primary_set(b, text);
        }
    }
}
