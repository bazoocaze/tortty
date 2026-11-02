//! O app: janela, pty, teclado, desenho e o socket de introspecção.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

use bzmux_core::{Anchor, Span, Term};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::cli::Options;
use crate::clip::Clip;
use crate::debugsock::{self, DebugServer};
use crate::glyph::Fonts;
use crate::keys;
use crate::prof;
use crate::pty::Pty;
use crate::render::{Renderer, BG};

pub enum UserEvent {
    /// Acorda o loop (dado novo no canal do pty).
    Wake,
    /// Pedido do socket de debug: `(linha JSON, canal da resposta)`.
    Debug(String, Sender<String>),
}

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn set_log(path: &str) {
    let mut slot = LOG.lock().unwrap();
    if path.is_empty() || path == "off" {
        *slot = None;
        return;
    }
    *slot = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok();
}

pub fn logln(msg: &str) {
    if let Ok(mut slot) = LOG.lock() {
        if let Some(f) = slot.as_mut() {
            use std::io::Write as _;
            let _ = writeln!(f, "{msg}");
        }
    }
}

pub struct App {
    opts: Options,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    term: Term,
    pty: Option<Pty>,
    rx: Option<Receiver<Vec<u8>>>,
    mods: ModifiersState,
    view_off: usize,
    /// Última posição do mouse em pixels físicos (base do `cell_at`).
    cursor_px: (f64, f64),
    /// Botão que está apertado, para reportar release/movimento com ele.
    held: Option<u8>,
    /// Seleção de texto (âncoras absolutas: sobrevive a rolagem e output).
    sel: Option<Span>,
    /// Âncora e pixel do press: o arrasto só nasce depois de 3 px.
    press: Option<(Anchor, (f64, f64))>,
    dragging: bool,
    clip: Clip,
    /// Fração de linha acumulada da roda (trackpad manda fração).
    scroll_frac: f32,
    debug: Option<DebugServer>,
    /// Grid pedido pelo verbo `resize`; o WM pode arredondar em pixels, então
    /// `apply_size` corrige em passos até bater (como um terminal de verdade).
    want: Option<(usize, usize)>,
    adjust_left: u8,
    eof: bool,
    exiting: bool,
}

impl App {
    pub fn new(opts: Options, proxy: EventLoopProxy<UserEvent>) -> App {
        let mut term = Term::new(opts.cols, opts.rows);
        term.scrollback_limit = opts.scrollback;
        App {
            opts,
            proxy,
            window: None,
            renderer: None,
            term,
            pty: None,
            rx: None,
            mods: ModifiersState::empty(),
            view_off: 0,
            cursor_px: (0.0, 0.0),
            held: None,
            sel: None,
            press: None,
            dragging: false,
            clip: Clip::new(),
            scroll_frac: 0.0,
            debug: None,
            want: None,
            adjust_left: 0,
            eof: false,
            exiting: false,
        }
    }

    fn font_px(&self, scale: f64) -> f32 {
        self.opts.font_size_pt * self.opts.dpi / 72.0 * scale as f32
    }

    fn grid_from_px(&self, w: u32, h: u32) -> (usize, usize) {
        match &self.renderer {
            Some(r) => (
                ((w as f32 / r.fonts.cw + 1e-3).floor() as usize).max(1),
                ((h as f32 / r.fonts.ch + 1e-3).floor() as usize).max(1),
            ),
            None => (self.opts.cols, self.opts.rows),
        }
    }

    /// Pixel -> (col, row) 0-based, preso à grade (sem padding: G9 ainda falta).
    fn cell_at(&self, px: f64, py: f64) -> (usize, usize) {
        let (cw, ch) = self
            .renderer
            .as_ref()
            .map_or((10.0, 19.0), |r| (r.fonts.cw, r.fonts.ch));
        let col = (px / f64::from(cw)) as usize;
        let row = (py / f64::from(ch)) as usize;
        (
            col.min(self.term.cols.saturating_sub(1)),
            row.min(self.term.rows.saturating_sub(1)),
        )
    }

    fn mouse_mods(&self) -> u8 {
        let mut m = 0;
        if self.mods.shift_key() {
            m |= crate::mouse::MOD_SHIFT;
        }
        if self.mods.alt_key() {
            m |= crate::mouse::MOD_ALT;
        }
        if self.mods.control_key() {
            m |= crate::mouse::MOD_CTRL;
        }
        m
    }

    /// Manda um evento de mouse para o filho. Só é chamado quando o app pediu
    /// (`?1000/1002/1003`); o resto do mouse é nosso (seleção).
    fn report_mouse(&mut self, button: u8, pressed: bool, motion: bool) {
        let (col, row) = self.cell_at(self.cursor_px.0, self.cursor_px.1);
        let bytes = crate::mouse::encode(
            crate::mouse::Ev {
                button,
                col,
                row,
                pressed,
                motion,
                mods: self.mouse_mods(),
            },
            crate::mouse::Proto {
                sgr: self.term.mouse_sgr,
                utf8: self.term.mouse_utf8,
            },
        );
        if let Some(pty) = self.pty.as_mut() {
            pty.write(&bytes);
        }
    }

    /// Tecla: atalhos da janela primeiro (o filho nunca vê Ctrl+Shift+C/V),
    /// depois o encoder. Qualquer tecla de verdade volta ao vivo.
    fn on_key(&mut self, event: &winit::event::KeyEvent) {
        let ctrl = self.mods.control_key();
        let shift = self.mods.shift_key();
        let named = match &event.logical_key {
            Key::Named(n) => Some(*n),
            _ => None,
        };
        let ch = match &event.logical_key {
            Key::Character(s) => s.chars().next(),
            _ => None,
        };

        // Ctrl+Shift+C/V são do terminal, não do app: sem isso o Ctrl+Shift+C
        // vira o mesmo ^C de Ctrl+C e mata o processo (bug do P-bzmux)
        if ctrl && shift {
            match ch.map(|c| c.to_ascii_lowercase()) {
                Some('c') => {
                    self.copy();
                    return;
                },
                Some('v') => {
                    self.paste();
                    return;
                },
                _ => {},
            }
        }
        // Shift+PgUp/PgDn/Home/End olham o histórico sem falar com o app
        if shift && !ctrl {
            match named {
                Some(NamedKey::PageUp) => {
                    self.scroll_view(self.term.rows as i64 - 1);
                    return;
                },
                Some(NamedKey::PageDown) => {
                    self.scroll_view(-(self.term.rows as i64 - 1));
                    return;
                },
                Some(NamedKey::Home) if !self.term.scrollback.is_empty() => {
                    self.scroll_view(self.term.scrollback.len() as i64);
                    return;
                },
                Some(NamedKey::End) => {
                    self.scroll_view(-(self.term.scrollback.len() as i64));
                    return;
                },
                _ => {},
            }
        }
        // tecla de verdade volta ao vivo; só modificador não (senão o Shift
        // do Shift+PgUp já resetava a visão antes de rolar)
        if self.view_off > 0
            && !matches!(
                named,
                Some(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super)
            )
        {
            self.view_off = 0;
        }
        let Some(bytes) = keys::encode(event, self.mods, self.term.app_cursor) else {
            return;
        };
        if let Some(pty) = self.pty.as_mut() {
            pty.write(&bytes);
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Movimento: só interessa ao app quando ele pediu `?1002`/`?1003`.
    fn on_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_px = (x, y);
        if self.term.mouse_mode == 0 {
            self.drag_select(x, y);
            return;
        }
        match self.term.mouse_mode {
            // 1002: movimento só com botão apertado
            2 if self.held.is_some() => {
                if let Some(b) = self.held {
                    self.report_mouse(b, true, true);
                }
            },
            // 1003: movimento sem botão também (código 35 = 3 + 32)
            3 => self.report_mouse(crate::mouse::RELEASE, true, true),
            _ => {},
        }
    }

    /// Press/release de botão. Com `?1000/1002/1003` o evento vai para o
    /// filho; sem eles o mouse é nosso (seleção/cópia).
    fn on_mouse_input(&mut self, state: ElementState, button: winit::event::MouseButton) {
        let Some(btn) = crate::mouse::from_winit(button) else {
            return;
        };
        if self.term.mouse_mode == 0 {
            self.select_input(state, button);
            return;
        }
        match state {
            ElementState::Pressed => {
                // F19: o app desenha o vivo; clique no histórico só faz
                // sentido depois de voltar para ele
                self.view_off = 0;
                self.held = Some(btn);
                self.report_mouse(btn, true, false);
            },
            ElementState::Released => {
                let b = self.held.take().unwrap_or(btn);
                self.report_mouse(b, false, false);
            },
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Roda: do app quando ele pediu mouse e estamos no vivo; senão é rolagem
    /// do histórico (o wheel lendo histórico continua sendo nosso).
    fn on_wheel(&mut self, delta: MouseScrollDelta) {
        // um entalhe vale 3 linhas (o mesmo do bzmux Python); trackpad manda
        // pixel (ou fração) e o resto é acumulado em `scroll_frac`
        let (hx, lines) = match delta {
            MouseScrollDelta::LineDelta(x, y) => (x * 3.0, y * 3.0),
            MouseScrollDelta::PixelDelta(p) => {
                let ch = self.renderer.as_ref().map_or(19.0, |r| r.fonts.ch);
                (0.0, p.y as f32 / ch)
            },
        };
        if self.term.mouse_mode != 0 && self.view_off == 0 && (lines != 0.0 || hx != 0.0) {
            for (v, up, down) in [
                (lines, crate::mouse::WHEEL_UP, crate::mouse::WHEEL_DOWN),
                (hx, crate::mouse::WHEEL_RIGHT, crate::mouse::WHEEL_LEFT),
            ] {
                if v == 0.0 {
                    continue;
                }
                let btn = if v > 0.0 { up } else { down };
                self.report_mouse(btn, true, false);
                self.report_mouse(btn, false, false);
            }
            return;
        }
        self.scroll_frac += lines;
        let n = self.scroll_frac.trunc();
        self.scroll_frac -= n;
        if n != 0.0 {
            // roda para cima (delta positivo) entra no histórico
            self.scroll_view(n as i64);
        }
    }

    /// Mouse do terminal: press fixa a âncora, arrasto vira seleção, release
    /// copia. Um clique sozinho não pinta célula nem mexe no PRIMARY.
    fn select_input(&mut self, state: ElementState, button: winit::event::MouseButton) {
        use winit::event::MouseButton::{Left, Middle};
        match (state, button) {
            (ElementState::Pressed, Left) => {
                // clicar no histórico NÃO volta ao vivo: o arrasto é para
                // justamente copiar dali
                self.sel = None;
                let (col, row) = self.cell_at(self.cursor_px.0, self.cursor_px.1);
                self.press = Some((self.term.anchor_at(self.view_off, row, col), self.cursor_px));
                self.dragging = true;
                self.redraw();
            },
            (ElementState::Released, Left) => {
                self.dragging = false;
                self.press = None;
                if self.sel.is_some_and(|s| s.a != s.b) {
                    // PRIMARY é a convenção do X11; o CLIPBOARD também, para
                    // colar em app que só olha o CLIPBOARD (`copy_on_select`
                    // vira opção na F3)
                    if let Some(txt) = self.selection_text() {
                        self.clip.set_primary(&txt);
                        self.clip.set(&txt);
                    }
                } else {
                    self.sel = None;
                }
                self.redraw();
            },
            (ElementState::Pressed, Middle) => {
                // botão do meio cola o PRIMARY; colar fala com o app, então a
                // visão vai para o vivo
                self.view_off = 0;
                if let Some(txt) = self.clip.primary_text() {
                    self.write_paste(&txt);
                }
            },
            _ => {},
        }
    }

    /// Arrasto: a seleção só nasce depois de 3 px (tremer o mouse com o botão
    /// apertado não pode pintar uma célula em reverso).
    fn drag_select(&mut self, x: f64, y: f64) {
        let Some((anchor, (px, py))) = self.press else {
            return;
        };
        if !self.dragging || ((x - px).abs() <= 3.0 && (y - py).abs() <= 3.0) {
            return;
        }
        let (col, row) = self.cell_at(x, y);
        let novo = Span::new(anchor, self.term.anchor_at(self.view_off, row, col));
        if self.sel != Some(novo) {
            self.sel = Some(novo);
            self.redraw();
        }
    }

    fn selection_text(&self) -> Option<String> {
        self.sel.and_then(|s| self.term.span_text(&s))
    }

    /// Ctrl+Shift+C: a seleção ou, sem seleção, a viewport inteira.
    fn copy(&mut self) {
        let txt = self
            .selection_text()
            .unwrap_or_else(|| self.term.view_text(self.view_off));
        if txt.is_empty() {
            return;
        }
        self.clip.set(&txt);
        self.clip.set_primary(&txt);
    }

    /// Ctrl+Shift+V: cola do CLIPBOARD.
    fn paste(&mut self) {
        if let Some(txt) = self.clip.text() {
            self.view_off = 0;
            self.write_paste(&txt);
        }
    }

    /// Escreve o texto no filho respeitando o bracketed paste (`?2004`).
    fn write_paste(&mut self, txt: &str) {
        let payload = self.term.paste_payload(txt);
        if let Some(pty) = self.pty.as_mut() {
            pty.write(&payload);
        }
        self.redraw();
    }

    /// Rola a visão em `n` linhas (positivo = para o histórico).
    fn scroll_view(&mut self, n: i64) {
        let sb = self.term.scrollback.len() as i64;
        self.view_off = (self.view_off as i64 + n).clamp(0, sb) as usize;
        self.redraw();
    }

    fn drain(&mut self) {
        let mut bytes: Vec<u8> = Vec::new();
        if let Some(rx) = &self.rx {
            loop {
                match rx.try_recv() {
                    Ok(v) if v.is_empty() => {
                        self.eof = true;
                        break;
                    },
                    Ok(v) => bytes.extend_from_slice(&v),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.eof = true;
                        break;
                    },
                }
            }
        }
        if !bytes.is_empty() {
            let t0 = std::time::Instant::now();
            self.term.feed(&bytes);
            if prof::enabled() {
                prof::add_parse(t0.elapsed().as_nanos() as u64, bytes.len());
            }
        }
        let out = self.term.take_child_out();
        if !out.is_empty() {
            if let Some(pty) = self.pty.as_mut() {
                pty.write(&out);
            }
        }
    }

    fn redraw(&mut self) {
        self.drain();
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        match renderer.render(&self.term, self.view_off, self.sel) {
            crate::render::FrameOutcome::Reconfigure => {
                let (w, h) = (renderer.config.width, renderer.config.height);
                renderer.resize(w, h);
            },
            crate::render::FrameOutcome::Fatal => self.exiting = true,
            _ => {},
        }
    }

    fn apply_size(&mut self, w: u32, h: u32) {
        let (cols, rows) = self.grid_from_px(w, h);
        if let Some(r) = self.renderer.as_mut() {
            r.resize(w, h);
        }
        if cols != self.term.cols || rows != self.term.rows {
            self.term.resize(cols, rows);
            if let Some(pty) = self.pty.as_mut() {
                pty.resize(cols as u16, rows as u16);
            }
            self.view_off = 0;
        }
        // o alvo do socket manda: se o WM entregou menos células, pede a
        // diferença de volta (o `request_inner_size` é só uma sugestão)
        if let Some((wc, wr)) = self.want {
            if (cols, rows) == (wc, wr) {
                self.want = None;
            } else if self.adjust_left > 0 {
                self.adjust_left -= 1;
                if let (Some(win), Some(r)) = (self.window.as_ref(), self.renderer.as_ref()) {
                    let dw = (wc as f32 - cols as f32) * r.fonts.cw;
                    let dh = (wr as f32 - rows as f32) * r.fonts.ch;
                    let nw = (w as f32 + dw).round().max(1.0) as u32;
                    let nh = (h as f32 + dh).round().max(1.0) as u32;
                    let _ = win.request_inner_size(PhysicalSize::new(nw, nh));
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)] // um braço por verbo do protocolo
    fn debug_dispatch(&mut self, line: &str) -> String {
        let req: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return debugsock::err("json inválido"),
        };
        let cmd = req.get("cmd").and_then(|c| c.as_str()).unwrap_or("");
        match cmd {
            "state" => {
                let (cw, ch) = match &self.renderer {
                    Some(r) => (r.fonts.cw, r.fonts.ch),
                    None => (0.0, 0.0),
                };
                let at = self.cell_at(self.cursor_px.0, self.cursor_px.1);
                let sel_json = match self.sel {
                    Some(s) => serde_json::json!({
                        "a": [s.a.line, s.a.col],
                        "b": [s.b.line, s.b.col],
                        "scrolled": self.term.scrolled,
                    }),
                    None => serde_json::Value::Null,
                };
                debugsock::ok(serde_json::json!({
                    "pid": std::process::id(),
                    "cols": self.term.cols,
                    "rows": self.term.rows,
                    "cursor": [self.term.cy, self.term.cx],
                    "title": self.term.title,
                    "view_off": self.view_off,
                    "scrolled": self.term.scrolled,
                    "scrollback": self.term.scrollback.len(),
                    "alt": self.term.in_alt_screen(),
                    "sync": self.term.sync_output,
                    "mouse_mode": self.term.mouse_mode,
                    "mouse_sgr": self.term.mouse_sgr,
                    "mouse_utf8": self.term.mouse_utf8,
                    "cursor_px": [self.cursor_px.0, self.cursor_px.1],
                    "cell_at": [at.0, at.1],
                    "sel": sel_json,
                    "cell": [cw, ch],
                    "font": match &self.renderer { Some(r) => r.fonts.family.clone(), None => String::new() },
                    "font_px": match &self.renderer { Some(r) => r.fonts.px, None => 0.0 },
                    "tabs": 1,
                    "workspaces": 1,
                    "errors": [],
                }))
            },
            "text" => {
                let where_ = req.get("where").and_then(|v| v.as_str()).unwrap_or("view");
                let start = req
                    .get("start")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                let count = req
                    .get("count")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(40)
                    .clamp(1, 2000) as usize;
                let lines: Vec<String> = match where_ {
                    "history" => self.term.history_text(start, count),
                    "screen" => (start..(start + count).min(self.term.rows))
                        .map(|y| self.term.row_text(y))
                        .collect(),
                    _ => self
                        .term
                        .visible_rows(self.view_off)
                        .iter()
                        .skip(start)
                        .take(count)
                        .map(|r| r.text())
                        .collect(),
                };
                debugsock::ok(serde_json::json!({ "lines": lines }))
            },
            "write" => match req.get("data").and_then(|v| v.as_str()) {
                Some(data) => {
                    if let Some(pty) = self.pty.as_mut() {
                        pty.write(data.as_bytes());
                    }
                    debugsock::ok(serde_json::json!({}))
                },
                None => debugsock::err("write precisa de data (string)"),
            },
            "resize" => {
                let cols = req
                    .get("cols")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                let rows = req
                    .get("rows")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0) as usize;
                if cols == 0 || rows == 0 {
                    return debugsock::err("resize precisa de cols e rows > 0");
                }
                self.want = Some((cols, rows));
                self.adjust_left = 4;
                if let (Some(w), Some(r)) = (self.window.as_ref(), self.renderer.as_ref()) {
                    let pw = (cols as f32 * r.fonts.cw).round() as u32;
                    let ph = (rows as f32 * r.fonts.ch).round() as u32;
                    let _ = w.request_inner_size(PhysicalSize::new(pw.max(1), ph.max(1)));
                }
                debugsock::ok(serde_json::json!({ "requested": [cols, rows] }))
            },
            "keys" => match req.get("data").and_then(|v| v.as_str()) {
                Some(spec) => {
                    let bytes = encode_spec(spec, self.term.app_cursor);
                    if let Some(pty) = self.pty.as_mut() {
                        pty.write(&bytes);
                    }
                    debugsock::ok(serde_json::json!({ "keys": [bytes.len()] }))
                },
                None => debugsock::err("keys precisa de data"),
            },
            "prof" => {
                if let Some(on) = req.get("on").and_then(serde_json::Value::as_bool) {
                    if on {
                        prof::reset();
                    }
                    prof::set_on(on);
                }
                debugsock::ok(prof::fields())
            },
            "log" => {
                let value = req.get("value").and_then(|v| v.as_str()).unwrap_or("off");
                set_log(value);
                debugsock::ok(serde_json::json!({ "path": value }))
            },
            "quit" => {
                self.exiting = true;
                debugsock::ok(serde_json::json!({}))
            },
            _ => debugsock::err(format!("cmd desconhecido: {cmd:?}")),
        }
    }

    fn spawn_shell(&mut self, cols: usize, rows: usize, wake: Arc<dyn Fn() + Send + Sync>) {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        match Pty::spawn(self.opts.cmd.as_deref(), cols as u16, rows as u16, tx, wake) {
            Ok(p) => {
                self.pty = Some(p);
                self.rx = Some(rx);
            },
            Err(e) => {
                eprintln!("bzmux: {e}");
                self.exiting = true;
            },
        }
    }
}

/// `ctrl+shift+n` -> bytes (versão mínima do `parse_key` do Python: o caminho
/// completo de teclado é o do winit, este verbo serve para teste/automação).
/// `spec` do verbo `keys`: `ctrl+shift+t`, `alt+left`, `f5`, `a`.
///
/// Resolve modificadores e nome de tecla e entrega ao encoder real
/// (`keys::encode_parts`) — não há tabela paralela aqui. Tecla que o encoder
/// não conhece devolve vazio, e o socket responde `keys: [0]`.
fn encode_spec(spec: &str, app_cursor: bool) -> Vec<u8> {
    let parts: Vec<&str> = spec.split('+').filter(|p| !p.is_empty()).collect();
    let Some((key, mods_spec)) = parts.split_last() else {
        return Vec::new();
    };
    let mut mods = ModifiersState::empty();
    for m in mods_spec {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= ModifiersState::CONTROL,
            "alt" | "meta" => mods |= ModifiersState::ALT,
            "shift" => mods |= ModifiersState::SHIFT,
            "super" | "logo" => mods |= ModifiersState::SUPER,
            _ => {},
        }
    }
    // nome de tecla nomeada (setas, F1-F12, edição) ou texto de um caractere
    let named = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(NamedKey::Enter),
        "esc" | "escape" => Some(NamedKey::Escape),
        "tab" => Some(NamedKey::Tab),
        "bs" | "backspace" => Some(NamedKey::Backspace),
        "space" => Some(NamedKey::Space),
        "up" => Some(NamedKey::ArrowUp),
        "down" => Some(NamedKey::ArrowDown),
        "left" => Some(NamedKey::ArrowLeft),
        "right" => Some(NamedKey::ArrowRight),
        "home" => Some(NamedKey::Home),
        "end" => Some(NamedKey::End),
        "pgup" | "pageup" => Some(NamedKey::PageUp),
        "pgdn" | "pagedown" => Some(NamedKey::PageDown),
        "del" | "delete" => Some(NamedKey::Delete),
        "ins" | "insert" => Some(NamedKey::Insert),
        "f1" => Some(NamedKey::F1),
        "f2" => Some(NamedKey::F2),
        "f3" => Some(NamedKey::F3),
        "f4" => Some(NamedKey::F4),
        "f5" => Some(NamedKey::F5),
        "f6" => Some(NamedKey::F6),
        "f7" => Some(NamedKey::F7),
        "f8" => Some(NamedKey::F8),
        "f9" => Some(NamedKey::F9),
        "f10" => Some(NamedKey::F10),
        "f11" => Some(NamedKey::F11),
        "f12" => Some(NamedKey::F12),
        _ => None,
    };
    if let Some(n) = named {
        return crate::keys::encode_parts(Some(n), None, None, mods, app_cursor)
            .unwrap_or_default();
    }
    if key.chars().count() == 1 {
        let ch = key.to_string();
        return crate::keys::encode_parts(None, Some(&ch), Some(&ch), mods, app_cursor)
            .unwrap_or_default();
    }
    Vec::new()
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let scale = event_loop
            .primary_monitor()
            .map_or(1.0, |m| m.scale_factor());
        let fonts = match Fonts::new(&self.opts.font_family, self.font_px(scale)) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("bzmux: {e}");
                event_loop.exit();
                return;
            },
        };
        let w = (self.opts.cols as f32 * fonts.cw).round() as u32;
        let h = (self.opts.rows as f32 * fonts.ch).round() as u32;
        let attrs = Window::default_attributes()
            .with_title("bzmux")
            .with_inner_size(PhysicalSize::new(w.max(1), h.max(1)));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("bzmux: janela: {e}");
                event_loop.exit();
                return;
            },
        };
        let renderer = match Renderer::new(&window, fonts, self.opts.vsync) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("bzmux: GPU: {e}");
                event_loop.exit();
                return;
            },
        };
        // o grid real vem do tamanho que a janela conseguiu, não do pedido
        let size = window.inner_size();
        let (cols, rows) = (
            ((size.width as f32 / renderer.fonts.cw + 1e-3).floor() as usize).max(1),
            ((size.height as f32 / renderer.fonts.ch + 1e-3).floor() as usize).max(1),
        );
        self.renderer = Some(renderer);
        self.term.resize(cols, rows);
        self.window = Some(window.clone());

        let proxy = self.proxy.clone();
        let wake = Arc::new(move || {
            let _ = proxy.send_event(UserEvent::Wake);
        });
        self.spawn_shell(cols, rows, wake);

        if let Some(path) = self.opts.debug_socket.clone() {
            match DebugServer::start(&path, self.proxy.clone()) {
                Ok(s) => {
                    logln(&format!("debug socket: {}", s.path()));
                    self.debug = Some(s);
                },
                Err(e) => eprintln!("bzmux: debug socket: {e}"),
            }
        }
        if let Some(path) = self.opts.debug_log.clone() {
            set_log(&path);
        }
        logln(&format!(
            "bzmux {} — {}x{} células, célula {}x{}, fonte {} {}px",
            env!("CARGO_PKG_VERSION"),
            cols,
            rows,
            self.renderer.as_ref().map_or(0.0, |r| r.fonts.cw),
            self.renderer.as_ref().map_or(0.0, |r| r.fonts.ch),
            self.renderer
                .as_ref()
                .map(|r| r.fonts.family.clone())
                .unwrap_or_default(),
            self.renderer.as_ref().map_or(0.0, |r| r.fonts.px),
        ));
        window.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Wake => {
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            },
            UserEvent::Debug(line, reply) => {
                let out = self.debug_dispatch(&line);
                let _ = reply.send(out);
                // `resize`/`quit` precisam de um ciclo de evento para valer
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            },
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.apply_size(size.width, size.height);
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            },
            WindowEvent::RedrawRequested => {
                self.redraw();
                if self.exiting {
                    event_loop.exit();
                }
            },
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
            },
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    self.on_key(&event);
                }
            },
            WindowEvent::CursorMoved { position, .. } => {
                self.on_cursor_moved(position.x, position.y);
            },
            WindowEvent::MouseInput { state, button, .. } => {
                self.on_mouse_input(state, button);
            },
            WindowEvent::MouseWheel { delta, .. } => {
                self.on_wheel(delta);
            },
            WindowEvent::Focused(focused) if self.term.focus_report => {
                let seq = if focused { "\x1b[I" } else { "\x1b[O" };
                if let Some(pty) = self.pty.as_mut() {
                    pty.write(seq.as_bytes());
                }
            },
            _ => {},
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.exiting {
            event_loop.exit();
            return;
        }
        // filho saiu: fecha a janela (fase 1 = uma sessão por janela)
        let exited = self.pty.as_mut().is_some_and(super::pty::Pty::child_exited);
        if exited {
            self.drain();
            self.redraw();
            event_loop.exit();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if prof::enabled() || self.opts.prof {
            let summary = prof::summary();
            eprintln!("prof: {summary}");
            logln(&format!("prof: {summary}"));
        }
        if let Some(pty) = self.pty.as_mut() {
            pty.kill();
        }
        self.debug = None;
        let _ = BG;
    }
}
