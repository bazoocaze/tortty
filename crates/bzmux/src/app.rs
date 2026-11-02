//! O app: janela, pty, teclado, desenho e o socket de introspecção.

use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

use bzmux_core::Term;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::cli::Options;
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
            let _ = writeln!(f, "{}", msg);
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

    fn drain(&mut self) {
        let mut bytes: Vec<u8> = Vec::new();
        if let Some(rx) = &self.rx {
            loop {
                match rx.try_recv() {
                    Ok(v) if v.is_empty() => {
                        self.eof = true;
                        break;
                    }
                    Ok(v) => bytes.extend_from_slice(&v),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.eof = true;
                        break;
                    }
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
        match renderer.render(&self.term, self.view_off) {
            crate::render::FrameOutcome::Reconfigure => {
                let (w, h) = (renderer.config.width, renderer.config.height);
                renderer.resize(w, h);
            }
            crate::render::FrameOutcome::Fatal => self.exiting = true,
            _ => {}
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
                    "cell": [cw, ch],
                    "font": match &self.renderer { Some(r) => r.fonts.family.clone(), None => String::new() },
                    "font_px": match &self.renderer { Some(r) => r.fonts.px, None => 0.0 },
                    "tabs": 1,
                    "workspaces": 1,
                    "errors": [],
                }))
            }
            "text" => {
                let where_ = req.get("where").and_then(|v| v.as_str()).unwrap_or("view");
                let start = req.get("start").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let count = req
                    .get("count")
                    .and_then(|v| v.as_u64())
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
            }
            "write" => match req.get("data").and_then(|v| v.as_str()) {
                Some(data) => {
                    if let Some(pty) = self.pty.as_mut() {
                        pty.write(data.as_bytes());
                    }
                    debugsock::ok(serde_json::json!({}))
                }
                None => debugsock::err("write precisa de data (string)"),
            },
            "resize" => {
                let cols = req.get("cols").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let rows = req.get("rows").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
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
            }
            "keys" => match req.get("data").and_then(|v| v.as_str()) {
                Some(spec) => {
                    let bytes = encode_spec(spec);
                    if let Some(pty) = self.pty.as_mut() {
                        pty.write(&bytes);
                    }
                    debugsock::ok(serde_json::json!({ "keys": [bytes.len()] }))
                }
                None => debugsock::err("keys precisa de data"),
            },
            "prof" => {
                if let Some(on) = req.get("on").and_then(|v| v.as_bool()) {
                    if on {
                        prof::reset();
                    }
                    prof::set_on(on);
                }
                debugsock::ok(prof::fields())
            }
            "log" => {
                let value = req.get("value").and_then(|v| v.as_str()).unwrap_or("off");
                set_log(value);
                debugsock::ok(serde_json::json!({ "path": value }))
            }
            "quit" => {
                self.exiting = true;
                debugsock::ok(serde_json::json!({}))
            }
            _ => debugsock::err(format!("cmd desconhecido: {:?}", cmd)),
        }
    }

    fn spawn_shell(&mut self, cols: usize, rows: usize, wake: Arc<dyn Fn() + Send + Sync>) {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        match Pty::spawn(self.opts.cmd.clone(), cols as u16, rows as u16, tx, wake) {
            Ok(p) => {
                self.pty = Some(p);
                self.rx = Some(rx);
            }
            Err(e) => {
                eprintln!("bzmux: {}", e);
                self.exiting = true;
            }
        }
    }
}

/// `ctrl+shift+n` -> bytes (versão mínima do `parse_key` do Python: o caminho
/// completo de teclado é o do winit, este verbo serve para teste/automação).
fn encode_spec(spec: &str) -> Vec<u8> {
    let parts: Vec<&str> = spec.split('+').collect();
    let key = parts.last().copied().unwrap_or("");
    let mods: Vec<String> = parts[..parts.len().saturating_sub(1)]
        .iter()
        .map(|s| s.to_lowercase())
        .collect();
    let ctrl = mods.iter().any(|m| m == "ctrl");
    let alt = mods.iter().any(|m| m == "alt");
    let shift = mods.iter().any(|m| m == "shift");
    let mut out = Vec::new();
    let named = match key.to_lowercase().as_str() {
        "enter" => Some("\r"),
        "esc" | "escape" => Some("\x1b"),
        "tab" => Some(if shift { "\x1b[Z" } else { "\t" }),
        "bs" | "backspace" => Some("\x7f"),
        "space" => Some(" "),
        "up" => Some("\x1b[A"),
        "down" => Some("\x1b[B"),
        "right" => Some("\x1b[C"),
        "left" => Some("\x1b[D"),
        "home" => Some("\x1b[H"),
        "end" => Some("\x1b[F"),
        "pgup" => Some("\x1b[5~"),
        "pgdn" => Some("\x1b[6~"),
        "del" => Some("\x1b[3~"),
        "ins" => Some("\x1b[2~"),
        _ => None,
    };
    if let Some(seq) = named {
        if alt && !seq.starts_with("\x1b[") {
            out.push(0x1b);
        }
        out.extend_from_slice(seq.as_bytes());
        return out;
    }
    let ch = key.chars().next().unwrap_or(' ');
    if ctrl {
        if alt {
            out.push(0x1b);
        }
        let low = ch.to_ascii_lowercase();
        out.push(match low {
            'a'..='z' => (low as u8) & 0x1f,
            ' ' | '2' => 0,
            '-' | '_' => 0x1f,
            _ => ch as u8,
        });
        return out;
    }
    if alt {
        out.push(0x1b);
    }
    let mut buf = [0u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    out
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let scale = event_loop
            .primary_monitor()
            .map(|m| m.scale_factor())
            .unwrap_or(1.0);
        let fonts = match Fonts::new(&self.opts.font_family, self.font_px(scale)) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("bzmux: {}", e);
                event_loop.exit();
                return;
            }
        };
        let w = (self.opts.cols as f32 * fonts.cw).round() as u32;
        let h = (self.opts.rows as f32 * fonts.ch).round() as u32;
        let attrs = Window::default_attributes()
            .with_title("bzmux")
            .with_inner_size(PhysicalSize::new(w.max(1), h.max(1)));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("bzmux: janela: {}", e);
                event_loop.exit();
                return;
            }
        };
        let renderer = match Renderer::new(window.clone(), fonts, self.opts.vsync) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("bzmux: GPU: {}", e);
                event_loop.exit();
                return;
            }
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
                }
                Err(e) => eprintln!("bzmux: debug socket: {}", e),
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
            self.renderer.as_ref().map(|r| r.fonts.cw).unwrap_or(0.0),
            self.renderer.as_ref().map(|r| r.fonts.ch).unwrap_or(0.0),
            self.renderer.as_ref().map(|r| r.fonts.family.clone()).unwrap_or_default(),
            self.renderer.as_ref().map(|r| r.fonts.px).unwrap_or(0.0),
        ));
        window.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Wake => {
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            UserEvent::Debug(line, reply) => {
                let out = self.debug_dispatch(&line);
                let _ = reply.send(out);
                // `resize`/`quit` precisam de um ciclo de evento para valer
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.apply_size(size.width, size.height);
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                if self.exiting {
                    event_loop.exit();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    if self.view_off > 0 {
                        self.view_off = 0; // qualquer tecla volta para o vivo
                    }
                    let keysym_scroll = self.view_off;
                    let _ = keysym_scroll;
                    let Some(bytes) = keys::encode(&event, self.mods, self.term.app_cursor)
                    else {
                        return;
                    };
                    if self.view_off > 0 {
                        self.view_off = 0;
                    }
                    if let Some(pty) = self.pty.as_mut() {
                        pty.write(&bytes);
                    }
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 24.0,
                };
                // roda para cima (delta positivo) entra no histórico
                let sb = self.term.scrollback.len();
                if lines > 0.0 {
                    self.view_off = (self.view_off + lines.ceil() as usize).min(sb);
                } else {
                    self.view_off = self.view_off.saturating_sub((-lines).ceil() as usize);
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::Focused(focused) if self.term.focus_report => {
                let seq = if focused { "\x1b[I" } else { "\x1b[O" };
                if let Some(pty) = self.pty.as_mut() {
                    pty.write(seq.as_bytes());
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.exiting {
            event_loop.exit();
            return;
        }
        // filho saiu: fecha a janela (fase 1 = uma sessão por janela)
        let exited = self.pty.as_mut().map(|p| p.child_exited()).unwrap_or(false);
        if exited {
            self.drain();
            self.redraw();
            event_loop.exit();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if prof::enabled() || self.opts.prof {
            let summary = prof::summary();
            eprintln!("prof: {}", summary);
            logln(&format!("prof: {}", summary));
        }
        if let Some(pty) = self.pty.as_mut() {
            pty.kill();
        }
        self.debug = None;
        let _ = BG;
    }
}
