//! O app: janela, pty, teclado, desenho e o socket de introspecção.

use std::io::Write as _;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bzmux_core::config::Bell;
use bzmux_core::{Anchor, Span, Term};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{UserAttentionType, Window, WindowId};

use crate::cli::Options;
use crate::clip::Clip;
use crate::debugsock::{self, DebugServer};
use crate::glyph::Fonts;
use crate::keys;
use crate::prof;
use crate::pty::Pty;
use crate::render::Renderer;

/// Alt puro (sem Ctrl nem Shift). `Alt+Shift+1` é do shell, não da barra.
fn alt_digit_only(mods: ModifiersState) -> bool {
    mods.alt_key() && !mods.control_key() && !mods.shift_key() && !mods.super_key()
}

/// Dígito da tecla **física** (0..=9), ignorando layout e Shift.
///
/// `Ctrl+Shift+1` chega como `!` e no ABNT2 o shiftado de cada dígito é outro
/// (`2` → `@`, `6` → `¨`); traduzir o código físico dá o dígito em qualquer
/// layout — mesma razão do `_digit_of` do Python (I16).
fn digit_of(event: &winit::event::KeyEvent) -> Option<usize> {
    let PhysicalKey::Code(code) = event.physical_key else {
        return None;
    };
    Some(match code {
        KeyCode::Digit0 => 0,
        KeyCode::Digit1 => 1,
        KeyCode::Digit2 => 2,
        KeyCode::Digit3 => 3,
        KeyCode::Digit4 => 4,
        KeyCode::Digit5 => 5,
        KeyCode::Digit6 => 6,
        KeyCode::Digit7 => 7,
        KeyCode::Digit8 => 8,
        KeyCode::Digit9 => 9,
        _ => return None,
    })
}

/// PID do descendente mais fundo de `pid` (o programa que está no primeiro
/// plano: o shell, ou o `vim`/`mc` que ele lançou). `/proc/<pid>/task/<pid>/
/// children` lista os filhos diretos; descer até o fim é o que dá o processo
/// que a pessoa está usando.
fn descendente(pid: u32) -> u32 {
    let mut cur = pid;
    for _ in 0..8 {
        let path = format!("/proc/{cur}/task/{cur}/children");
        let Ok(txt) = std::fs::read_to_string(path) else {
            break;
        };
        let Some(filho) = txt.split_whitespace().next().and_then(|s| s.parse().ok()) else {
            break;
        };
        cur = filho;
    }
    cur
}

pub enum UserEvent {
    /// Acorda o loop (dado novo no canal do pty).
    Wake,
    /// Pedido do socket de debug: `(linha JSON, canal da resposta)`.
    Debug(String, Sender<String>),
}

/// Aviso transitório do próprio terminal (I12): "config: 3 erros — ...".
struct Toast {
    texto: String,
    desde: std::time::Instant,
    /// Quantas mensagens iguais chegaram seguidas (o toast soma, não empilha).
    vezes: u32,
}

/// Quanto tempo o toast fica na tela.
const TOAST_DUR: Duration = Duration::from_millis(3500);

/// Texto do aviso de configuração (K13). Cabe em ~60 colunas de propósito: o
/// toast é cortado na largura da grade, e "linha e chave" é o que se precisa
/// ver, não a contagem escrita por extenso.
fn aviso_config(errs: &[bzmux_core::config::Error]) -> String {
    let Some(primeiro) = errs.first() else {
        return String::new();
    };
    if errs.len() > 1 {
        format!("config mantida — {primeiro} (+{})", errs.len() - 1)
    } else {
        format!("config mantida — {primeiro}")
    }
}

/// Período do pisca (cursor e SGR 5). Metade "aceso", metade "apagado".
const BLINK_MS: u64 = 500;

/// Quanto tempo o rótulo da aba fica destacado depois de um BEL (I13).
const FLASH_DUR: Duration = Duration::from_millis(800);

/// Bytes em hex (o `keys` do socket devolve isso para conferir o encoder).
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
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

/// Uma aba: um terminal e o pty que o alimenta, mais o estado que só faz
/// sentido enquanto ela está na frente (visão, seleção, arrasto do mouse).
///
/// A separação importa por causa de F4: com N abas vivas ao mesmo tempo, o pty
/// de cada uma continua lendo; só o que é "olhar" (rolagem, seleção) fica
/// parado na aba que não está ativa.
pub struct Tab {
    pub term: Term,
    pub pty: Option<Pty>,
    pub rx: Option<Receiver<Vec<u8>>>,
    /// Linhas roladas para o histórico (0 = acompanhando o output).
    pub view_off: usize,
    /// Seleção de texto, em âncoras absolutas.
    pub sel: Option<Span>,
    /// Âncora e pixel do press: o arrasto só nasce depois de 3 px.
    pub press: Option<(Anchor, (f64, f64))>,
    pub dragging: bool,
    /// Fração de linha acumulada da roda (trackpad manda fração).
    pub scroll_frac: f32,
    /// BELLs já tratados (o `Term` conta; o app decide o que tocar).
    pub bells_seen: u32,
    /// Título pedido por OSC 0/2 (vazio = derivar do cwd).
    pub osc_title: String,
    /// Último BEL, para o flash do rótulo (I13).
    pub bell_at: Option<std::time::Instant>,
}

impl Tab {
    fn new(cols: usize, rows: usize) -> Tab {
        Tab {
            term: Term::new(cols, rows),
            pty: None,
            rx: None,
            view_off: 0,
            sel: None,
            press: None,
            dragging: false,
            scroll_frac: 0.0,
            bells_seen: 0,
            osc_title: String::new(),
            bell_at: None,
        }
    }

    /// Nome da aba: OSC 0/2 vence, senão o cwd do shell, senão o padrão.
    fn title(&self) -> String {
        if !self.osc_title.is_empty() {
            return self.osc_title.clone();
        }
        match self.cwd_name() {
            Some(cwd) => cwd,
            None => "bzmux".to_string(),
        }
    }

    /// `basename` do cwd do processo filho (`/proc/<pid>/cwd`); sem isso, nada.
    fn cwd_name(&self) -> Option<String> {
        let cwd = self.cwd()?;
        let nome = cwd.file_name()?.to_string_lossy().to_string();
        if nome.is_empty() {
            None
        } else {
            Some(nome)
        }
    }

    /// cwd do que está rodando **na frente** do pty: o shell pode ter lançado
    /// `vim`/`mc`, e é o cwd deles que a próxima aba deve herdar (é o que o
    /// `fork` do Python fazia ao olhar o último descendente).
    fn cwd(&self) -> Option<std::path::PathBuf> {
        let pid = self.pty.as_ref()?.child_pid()?;
        let alvo = descendente(pid);
        std::fs::read_link(format!("/proc/{alvo}/cwd")).ok()
    }
}

#[allow(clippy::struct_excessive_bools)] // estado do app é booleano por natureza
pub struct App {
    opts: Options,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// Abas vivas (cada uma com o pty rodando), e qual está na frente.
    tabs: Vec<Tab>,
    active: usize,
    mods: ModifiersState,
    /// Última posição do mouse em pixels físicos (base do `cell_at`).
    cursor_px: (f64, f64),
    /// Botão que está apertado, para reportar release/movimento com ele.
    held: Option<u8>,
    clip: Clip,
    /// Acorda o loop quando chega dado do pty (o filho nasce depois da
    /// janela, quando o grid já é o definitivo).
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    spawned: bool,
    /// O que veio da linha de comando: o reload reconstrói as Options com ele
    /// para o CLI continuar vencendo o arquivo.
    over: crate::cli::Overrides,
    /// `mtime` da última leitura do arquivo de configuração (auto-reload).
    cfg_mtime: Option<std::time::SystemTime>,
    cfg_checked: std::time::Instant,
    /// Instante em que o pisca foi zerado (última tecla/colagem). A fase sai
    /// daí por aritmética: sem acumular estado, sem `sleep`.
    blink_epoch: std::time::Instant,
    /// Fase atual do pisca, para só redesenhar quando ela vira.
    blink_fase: bool,
    /// Há célula com SGR 5 na tela. Recalculado a cada `feed`: perguntar isso
    /// ao grid em cada tique custaria uma varredura por evento.
    blink_text: bool,
    /// Aviso na tela (erro de config etc.). Um por vez, some sozinho.
    toast: Option<Toast>,
    /// Erros do arquivo no arranque, que só podem virar toast quando a janela
    /// existir (no terminal, `stderr` some; numa sessão gráfica, idem).
    startup_errs: Vec<bzmux_core::config::Error>,
    debug: Option<DebugServer>,
    /// Grid pedido pelo verbo `resize`; o WM pode arredondar em pixels, então
    /// `apply_size` corrige em passos até bater (como um terminal de verdade).
    want: Option<(usize, usize)>,
    adjust_left: u8,
    eof: bool,
    /// Último título posto na janela (evita `set_title` por frame).
    last_title: String,
    exiting: bool,
}

impl App {
    pub fn new(
        opts: Options,
        over: crate::cli::Overrides,
        proxy: EventLoopProxy<UserEvent>,
    ) -> App {
        let mut tab = Tab::new(opts.cols, opts.rows);
        tab.term.scrollback_limit = opts.scrollback;
        tab.term.theme = opts.cfg.theme();
        // o `mtime` do arranque já vale: sem isso o primeiro tique reparseia o
        // mesmo arquivo e um erro de config sairia duas vezes em tela
        let cfg_mtime = opts
            .cfg_path
            .as_deref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        App {
            opts,
            over,
            proxy,
            window: None,
            renderer: None,
            tabs: vec![tab],
            active: 0,
            mods: ModifiersState::empty(),
            cursor_px: (0.0, 0.0),
            held: None,
            clip: Clip::new(),
            wake: None,
            spawned: false,
            cfg_mtime,
            cfg_checked: std::time::Instant::now(),
            blink_epoch: std::time::Instant::now(),
            blink_fase: true,
            blink_text: false,
            toast: None,
            startup_errs: Vec::new(),
            debug: None,
            want: None,
            adjust_left: 0,
            eof: false,
            last_title: String::new(),
            exiting: false,
        }
    }

    fn font_px(&self, scale: f64) -> f32 {
        self.opts.font_size_pt * self.opts.dpi / 72.0 * scale as f32
    }

    /// Altura da barra de abas em px (0 com uma aba só). A barra só aparece
    /// quando há escolha a fazer; com uma aba, ocupa espaço à toa.
    fn bar_h(&self, ch: f32) -> f32 {
        if self.tabs.len() > 1 {
            ch
        } else {
            0.0
        }
    }

    /// O que a barra mostra: número, título e alerta de BEL de cada aba.
    fn tabinfo(&self) -> Vec<crate::render::TabInfo> {
        let agora = std::time::Instant::now();
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| crate::render::TabInfo {
                titulo: t.title(),
                ativa: i == self.active,
                alerta: t
                    .bell_at
                    .is_some_and(|quando| agora.duration_since(quando) < FLASH_DUR),
            })
            .collect()
    }

    fn grid_from_px(&self, w: u32, h: u32) -> (usize, usize) {
        match &self.renderer {
            Some(r) => {
                // nem a margem (`window_padding_width`) nem a barra de abas
                // são grade
                let bar = self.bar_h(r.fonts.ch);
                let (iw, ih) = (
                    (w as f32 - 2.0 * r.pad).max(r.fonts.cw),
                    (h as f32 - 2.0 * r.pad - bar).max(r.fonts.ch),
                );
                (
                    ((iw / r.fonts.cw + 1e-3).floor() as usize).max(1),
                    ((ih / r.fonts.ch + 1e-3).floor() as usize).max(1),
                )
            },
            None => (self.opts.cols, self.opts.rows),
        }
    }

    /// Pixel -> (col, row) 0-based, preso à grade (sem padding: G9 ainda falta).
    fn cell_at(&self, px: f64, py: f64) -> (usize, usize) {
        let (cw, ch, pad) = self
            .renderer
            .as_ref()
            .map_or((10.0, 19.0, 0.0), |r| (r.fonts.cw, r.fonts.ch, r.pad));
        // descontar a origem: com margem (e barra de abas), o (0,0) da grade
        // não é o (0,0) da janela
        let bar = f64::from(self.bar_h(ch));
        let col = ((px - f64::from(pad)) / f64::from(cw)).max(0.0) as usize;
        let row = ((py - f64::from(pad) - bar) / f64::from(ch)).max(0.0) as usize;
        (
            col.min(self.tabs[self.active].term.cols.saturating_sub(1)),
            row.min(self.tabs[self.active].term.rows.saturating_sub(1)),
        )
    }

    /// Índice da aba sob o cursor, se ele estiver na faixa da barra.
    fn tab_at(&self, px: f64, py: f64) -> Option<usize> {
        let (cw, ch, pad) = self
            .renderer
            .as_ref()
            .map_or((10.0, 19.0, 0.0), |r| (r.fonts.cw, r.fonts.ch, r.pad));
        let bar = f64::from(self.bar_h(ch));
        if bar <= 0.0 || py < f64::from(pad) || py >= f64::from(pad) + bar {
            return None;
        }
        let info = self.tabinfo();
        let mut x = f64::from(pad);
        for (i, t) in info.iter().enumerate() {
            let rotulo = format!(" {} {} ", i + 1, t.titulo);
            let largura = rotulo.chars().count() as f64 * f64::from(cw);
            if px >= x && px < x + largura {
                return Some(i);
            }
            x += largura;
        }
        None
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
                sgr: self.tabs[self.active].term.mouse_sgr,
                utf8: self.tabs[self.active].term.mouse_utf8,
            },
        );
        if let Some(pty) = self.tabs[self.active].pty.as_mut() {
            pty.write(&bytes);
        }
    }

    /// Tecla: atalhos da janela primeiro (o filho nunca vê Ctrl+Shift+C/V),
    /// depois o encoder. Qualquer tecla de verdade volta ao vivo.
    fn on_key(&mut self, event: &winit::event::KeyEvent) {
        self.blink_reset();
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

        // Ctrl+Shift+<letra>: primeiro o que é do app (abas), depois o que é do
        // terminal (copiar/colar)
        if ctrl && shift {
            match ch.map(|c| c.to_ascii_lowercase()) {
                Some('t') => {
                    self.new_tab();
                    return;
                },
                Some('w') => {
                    self.close_tab();
                    return;
                },
                _ => {},
            }
        }
        // Alt+1..9 escolhe a aba; Alt+0 é a décima (I15). O dígito não vaza
        // para o filho — é o que o Python fazia.
        if alt_digit_only(self.mods) {
            if let Some(n) = digit_of(event) {
                self.select_tab(if n == 0 { 9 } else { n - 1 });
                return;
            }
        }
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
                    self.scroll_view(self.tabs[self.active].term.rows as i64 - 1);
                    return;
                },
                Some(NamedKey::PageDown) => {
                    self.scroll_view(-(self.tabs[self.active].term.rows as i64 - 1));
                    return;
                },
                Some(NamedKey::Home) if !self.tabs[self.active].term.scrollback.is_empty() => {
                    self.scroll_view(self.tabs[self.active].term.scrollback.len() as i64);
                    return;
                },
                Some(NamedKey::End) => {
                    self.scroll_view(-(self.tabs[self.active].term.scrollback.len() as i64));
                    return;
                },
                _ => {},
            }
        }
        // tecla de verdade volta ao vivo; só modificador não (senão o Shift
        // do Shift+PgUp já resetava a visão antes de rolar)
        if self.tabs[self.active].view_off > 0
            && !matches!(
                named,
                Some(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super)
            )
        {
            self.tabs[self.active].view_off = 0;
        }
        let Some(bytes) = keys::encode(event, self.mods, self.tabs[self.active].term.app_cursor)
        else {
            return;
        };
        if let Some(pty) = self.tabs[self.active].pty.as_mut() {
            pty.write(&bytes);
        }
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Movimento: só interessa ao app quando ele pediu `?1002`/`?1003`.
    fn on_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_px = (x, y);
        if self.tabs[self.active].term.mouse_mode == 0 {
            self.drag_select(x, y);
            return;
        }
        match self.tabs[self.active].term.mouse_mode {
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
        if state == ElementState::Pressed {
            self.blink_reset();
        }
        if self.tabs[self.active].term.mouse_mode == 0 {
            self.select_input(state, button);
            return;
        }
        match state {
            ElementState::Pressed => {
                // clique na barra não é do app (troca a aba, como na seleção)
                if let Some(i) = self.tab_at(self.cursor_px.0, self.cursor_px.1) {
                    self.select_tab(i);
                    self.held = None;
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                    return;
                }
                // F19: o app desenha o vivo; clique no histórico só faz
                // sentido depois de voltar para ele
                self.tabs[self.active].view_off = 0;
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
        if self.tabs[self.active].term.mouse_mode != 0
            && self.tabs[self.active].view_off == 0
            && (lines != 0.0 || hx != 0.0)
        {
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
        self.tabs[self.active].scroll_frac += lines;
        let n = self.tabs[self.active].scroll_frac.trunc();
        self.tabs[self.active].scroll_frac -= n;
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
                // clique na barra de abas troca de aba (não seleciona texto)
                if let Some(i) = self.tab_at(self.cursor_px.0, self.cursor_px.1) {
                    self.select_tab(i);
                    self.tabs[self.active].dragging = false;
                    self.tabs[self.active].press = None;
                    self.redraw();
                    return;
                }
                // clicar no histórico NÃO volta ao vivo: o arrasto é para
                // justamente copiar dali
                self.tabs[self.active].sel = None;
                let (col, row) = self.cell_at(self.cursor_px.0, self.cursor_px.1);
                self.tabs[self.active].press = Some((
                    self.tabs[self.active].term.anchor_at(
                        self.tabs[self.active].view_off,
                        row,
                        col,
                    ),
                    self.cursor_px,
                ));
                self.tabs[self.active].dragging = true;
                self.redraw();
            },
            (ElementState::Released, Left) => {
                self.tabs[self.active].dragging = false;
                self.tabs[self.active].press = None;
                if self.tabs[self.active].sel.is_some_and(|s| s.a != s.b) {
                    // PRIMARY é a convenção do X11 (botão do meio cola dali).
                    // O CLIPBOARD só entra se `copy_on_select` mandar: em app
                    // que só olha o CLIPBOARD é conveniência, mas pisar nele a
                    // cada arrasto atrapalha mais do que ajuda.
                    if let Some(txt) = self.selection_text() {
                        self.clip.set_primary(&txt);
                        if self.opts.cfg.copy_on_select {
                            self.clip.set(&txt);
                        }
                    }
                } else {
                    self.tabs[self.active].sel = None;
                }
                self.redraw();
            },
            (ElementState::Pressed, Middle) => {
                // botão do meio cola o PRIMARY; colar fala com o app, então a
                // visão vai para o vivo
                self.tabs[self.active].view_off = 0;
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
        let Some((anchor, (px, py))) = self.tabs[self.active].press else {
            return;
        };
        if !self.tabs[self.active].dragging || ((x - px).abs() <= 3.0 && (y - py).abs() <= 3.0) {
            return;
        }
        let (col, row) = self.cell_at(x, y);
        let novo = Span::new(
            anchor,
            self.tabs[self.active]
                .term
                .anchor_at(self.tabs[self.active].view_off, row, col),
        );
        if self.tabs[self.active].sel != Some(novo) {
            self.tabs[self.active].sel = Some(novo);
            self.redraw();
        }
    }

    fn selection_text(&self) -> Option<String> {
        self.tabs[self.active]
            .sel
            .and_then(|s| self.tabs[self.active].term.span_text(&s))
    }

    /// Ctrl+Shift+C: a seleção ou, sem seleção, a viewport inteira.
    fn copy(&mut self) {
        let txt = self.selection_text().unwrap_or_else(|| {
            self.tabs[self.active]
                .term
                .view_text(self.tabs[self.active].view_off)
        });
        if txt.is_empty() {
            return;
        }
        self.clip.set(&txt);
        self.clip.set_primary(&txt);
    }

    /// Ctrl+Shift+V: cola do CLIPBOARD.
    fn paste(&mut self) {
        if let Some(txt) = self.clip.text() {
            self.blink_reset();
            self.tabs[self.active].view_off = 0;
            self.write_paste(&txt);
        }
    }

    /// Escreve o texto no filho respeitando o bracketed paste (`?2004`).
    fn write_paste(&mut self, txt: &str) {
        let payload = self.tabs[self.active].term.paste_payload(txt);
        if let Some(pty) = self.tabs[self.active].pty.as_mut() {
            pty.write(&payload);
        }
        self.redraw();
    }

    /// Rola a visão em `n` linhas (positivo = para o histórico).
    fn scroll_view(&mut self, n: i64) {
        let sb = self.tabs[self.active].term.scrollback.len() as i64;
        self.tabs[self.active].view_off =
            (self.tabs[self.active].view_off as i64 + n).clamp(0, sb) as usize;
        self.redraw();
    }

    /// Lê o que o pty de **todas** as abas produziu.
    ///
    /// Aba em segundo plano também precisa ser drenada: o canal não é
    /// ilimitado, e um `yes` numa aba escondida encheria a memória. O que fica
    /// reservado à aba ativa é só o que é "olhar" (pisca e visão).
    fn drain(&mut self) {
        for i in 0..self.tabs.len() {
            self.drain_tab(i);
        }
    }

    fn drain_tab(&mut self, i: usize) {
        let mut bytes: Vec<u8> = Vec::new();
        if let Some(rx) = &self.tabs[i].rx {
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
            // D10: quem está lendo o histórico não pode ser arrastado pelo
            // output que chega — a visão anda junto com as linhas que saíram.
            let (sc, hist) = (
                self.tabs[i].term.scrolled,
                self.tabs[i].term.scrollback.len(),
            );
            let t0 = std::time::Instant::now();
            self.tabs[i].term.feed(&bytes);
            if prof::enabled() {
                prof::add_parse(t0.elapsed().as_nanos() as u64, bytes.len());
            }
            // `OSC 4/10/11/12` mexeu nas cores: o renderer tem que saber. Vale
            // para qualquer aba: o tema é do emulador, não da aba.
            if self.tabs[i].term.theme_dirty {
                self.tabs[i].term.theme_dirty = false;
                let tema = self.tabs[i].term.theme.clone();
                if let Some(r) = self.renderer.as_mut() {
                    r.apply_theme(&tema);
                }
            }
            // I4: título por OSC 0/2 (o shell põe "~/projeto" e o `vim` o nome
            // do arquivo); o `bzmux` é o padrão e não conta como pedido
            let titulo = self.tabs[i].term.title.clone();
            if titulo != self.tabs[i].osc_title {
                let era = i == self.active;
                self.tabs[i].osc_title = if titulo == "bzmux" {
                    String::new()
                } else {
                    titulo
                };
                if era {
                    self.sync_title();
                }
            }
            let bells = self.tabs[i].term.bells;
            while self.tabs[i].bells_seen != bells {
                self.tabs[i].bells_seen = self.tabs[i].bells_seen.wrapping_add(1);
                // I13: o rótulo da aba pisca por 800 ms, mesmo que ela não
                // esteja na frente (é o aviso de "aconteceu algo aqui")
                self.tabs[i].bell_at = Some(std::time::Instant::now());
                self.ring();
            }
            if i == self.active {
                self.blink_text = self.tabs[i].term.grid.iter().any(|r| {
                    r.cells
                        .iter()
                        .any(|c| c.flags & bzmux_core::cell::BLINK != 0)
                });
            }
            if self.tabs[i].view_off > 0 {
                let off = self.tabs[i].view_off;
                let off = self.tabs[i].term.view_after_output(off, sc, hist);
                // `CSI 3J` (ou um reflow) pode ter esvaziado o histórico: sem
                // isto a visão aponta para linhas que não existem mais e a tela
                // mostra lixo (ou o "pulo" que o F2 tinha que evitar)
                self.tabs[i].view_off = off.min(self.tabs[i].term.scrollback.len());
            }
        }
        let out = self.tabs[i].term.take_child_out();
        if !out.is_empty() {
            if let Some(pty) = self.tabs[i].pty.as_mut() {
                pty.write(&out);
            }
        }
    }

    /// Erros do arquivo lidos no arranque: viram toast no primeiro tique.
    pub fn startup_errors(&mut self, errs: Vec<bzmux_core::config::Error>) {
        self.startup_errs = errs;
    }

    /// Zera o pisca: logo depois de uma tecla o cursor tem que estar aceso.
    /// Sem isto o cursor pode "sumir" no exato instante em que a pessoa olha.
    fn blink_reset(&mut self) {
        self.blink_epoch = std::time::Instant::now();
        if !self.blink_fase {
            self.blink_fase = true;
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }

    /// Estado do pisca deste frame (G12).
    ///
    /// O cursor pisca se a config deixa **e** o programa não pediu cursor fixo
    /// por `CSI 6 q` (4/6 = fixo). O texto com SGR 5 pisca sempre — é o
    /// programa que pediu, não a config.
    fn blink(&self) -> crate::render::Blink {
        let fase = self.blink_fase;
        crate::render::Blink {
            fase,
            cursor: !self.cursor_pisca() || fase,
        }
    }

    /// O cursor deve piscar? A config manda, mas `CSI 6 q` com forma par (4/6)
    /// pede cursor fixo e vence enquanto o programa quiser.
    fn cursor_pisca(&self) -> bool {
        self.opts.cfg.cursor_blink
            && self.tabs[self.active]
                .term
                .app_cursor_style
                .is_none_or(|(_, pisca)| pisca)
    }

    /// Tem alguma coisa piscando na tela (cursor ou texto com SGR 5)? Só nesse
    /// caso o loop precisa acordar no meio do período.
    fn pisca_ativo(&self) -> bool {
        (self.cursor_pisca() && self.tabs[self.active].term.cursor_visible)
            || self.tem_texto_piscando()
    }

    /// Há célula com SGR 5 na tela? Sem isto o loop não teria por que acordar
    /// no meio do período do pisca quando o cursor está fixo.
    fn tem_texto_piscando(&self) -> bool {
        self.blink_text
    }

    /// Mostra um aviso na tela. Mensagem igual seguida vira contagem ("×3"),
    /// não uma fila — quem salva o arquivo dez vezes não quer dez avisos.
    #[allow(clippy::needless_pass_by_value)] // quem chama monta o texto na hora
    fn toast(&mut self, texto: String) {
        let vezes = match self.toast.take() {
            Some(t) if t.texto == texto => t.vezes + 1,
            _ => 1,
        };
        self.toast = Some(Toast {
            texto: texto.clone(),
            desde: std::time::Instant::now(),
            vezes,
        });
        logln(&format!("toast: {texto}"));
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Texto do toast para o renderer (com a contagem quando houve repetição).
    fn toast_texto(&self) -> Option<String> {
        let t = self.toast.as_ref()?;
        if t.desde.elapsed() > TOAST_DUR {
            return None;
        }
        Some(if t.vezes > 1 {
            format!("{} (×{})", t.texto, t.vezes)
        } else {
            t.texto.clone()
        })
    }

    /// BEL: o app pede (a contagem está no `Term`), a configuração decide o
    /// que acontece. `audible` escreve o BEL no nosso stdout (um terminal de
    /// verdade apita; numa sessão gráfica é o que sobra sem lib de áudio).
    fn ring(&mut self) {
        match self.opts.cfg.bell {
            Bell::None => {},
            Bell::Visual => {
                if let Some(w) = self.window.as_ref() {
                    w.request_user_attention(Some(UserAttentionType::Informational));
                }
            },
            Bell::Audible => {
                let mut out = std::io::stdout();
                let _ = out.write_all(b"\x07");
                let _ = out.flush();
            },
        }
    }

    /// Auto-reload do arquivo de configuração (K12): a cada 250 ms confere o
    /// `mtime`; mudou, reparseia. **Qualquer** erro mantém a configuração que
    /// já estava valendo — meia configuração é pior que nenhuma.
    fn check_config(&mut self) {
        const INTERVALO: Duration = Duration::from_millis(250);
        if self.cfg_checked.elapsed() < INTERVALO {
            return;
        }
        self.cfg_checked = std::time::Instant::now();
        let Some(path) = self.opts.cfg_path.clone() else {
            return;
        };
        let Ok(mtime) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
            return;
        };
        if self.cfg_mtime == Some(mtime) {
            return;
        }
        self.cfg_mtime = Some(mtime);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let (novo, errs) = crate::cli::Options::build(self.over.clone());
        if !errs.is_empty() {
            logln(&format!(
                "config: {} mantida; {} erro(s), o primeiro em {}",
                path.display(),
                errs.len(),
                errs[0]
            ));
            // K13: o arquivo é editado num editor, que não mostra o nosso log —
            // o aviso tem que aparecer na tela, com a primeira linha e a conta.
            self.toast(aviso_config(&errs));
            return;
        }
        logln(&format!("config: {} recarregada", path.display()));
        // só avisa na tela se havia um erro aparecendo: recarregar deu certo e
        // quem salvou precisa saber que voltou a valer
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.texto.starts_with("config"))
        {
            self.toast("config recarregada".to_string());
        }
        self.apply_options(novo, &text);
    }

    /// Aplica configuração nova no que já está rodando.
    fn apply_options(&mut self, novo: Options, text: &str) {
        let fonte_mudou = novo.font_family != self.opts.font_family
            || (novo.font_size_pt - self.opts.font_size_pt).abs() > f32::EPSILON
            || (novo.dpi - self.opts.dpi).abs() > f32::EPSILON;
        let log_mudou = novo.debug_log != self.opts.debug_log;
        self.tabs[self.active].term.scrollback_limit = novo.scrollback;
        while self.tabs[self.active].term.scrollback.len()
            > self.tabs[self.active].term.scrollback_limit
        {
            self.tabs[self.active].term.scrollback.pop_front();
        }
        self.opts.cfg = novo.cfg.clone();
        self.opts.scrollback = novo.scrollback;
        self.tabs[self.active].term.theme = novo.cfg.theme();
        if let Some(r) = self.renderer.as_mut() {
            r.apply_theme(&self.tabs[self.active].term.theme);
            r.apply_layout(&novo.cfg);
        }
        if let Some(path) = novo.debug_log.as_deref() {
            set_log(path);
        }
        let _ = text;
        if fonte_mudou {
            self.opts.font_family = novo.font_family;
            self.opts.font_size_pt = novo.font_size_pt;
            self.opts.dpi = novo.dpi;
            self.rebuild_renderer();
        }
        if log_mudou && novo.debug_log.is_none() {
            set_log("");
        }
        // a margem mudou: o grid é outro, então pede o tamanho de novo
        if let (Some(win), Some(r)) = (self.window.as_ref(), self.renderer.as_ref()) {
            let (cols, rows) = self.grid_from_px(r.config.width, r.config.height);
            if cols != self.tabs[self.active].term.cols || rows != self.tabs[self.active].term.rows
            {
                let size = win.inner_size();
                self.apply_size(size.width, size.height);
            }
        }
        self.redraw();
    }

    /// Fonte mudou: o atlas é outro, então refaz fonte + renderer (a janela e
    /// o pty continuam os mesmos).
    fn rebuild_renderer(&mut self) {
        let (Some(window), Some(velho)) = (self.window.clone(), self.renderer.take()) else {
            return;
        };
        let scale = window.scale_factor();
        let fonts = match Fonts::new(&self.opts.font_family, self.font_px(scale)) {
            Ok(f) => f,
            Err(e) => {
                logln(&format!("fonte: {e}"));
                self.renderer = Some(velho);
                return;
            },
        };
        drop(velho);
        match Renderer::new(&window, fonts, self.opts.vsync, &self.opts.cfg) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => logln(&format!("GPU ao trocar de fonte: {e}")),
        }
        let size = window.inner_size();
        self.apply_size(size.width, size.height);
    }

    fn redraw(&mut self) {
        self.drain();
        let blink = self.blink();
        let toast = self.toast_texto();
        let tabs = if self.tabs.len() > 1 {
            Some(self.tabinfo())
        } else {
            None
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        match renderer.render(
            &self.tabs[self.active].term,
            self.tabs[self.active].view_off,
            self.tabs[self.active].sel,
            blink,
            toast.as_deref(),
            tabs.as_deref(),
        ) {
            crate::render::FrameOutcome::Reconfigure => {
                let (w, h) = (renderer.config.width, renderer.config.height);
                renderer.resize(w, h);
            },
            crate::render::FrameOutcome::Fatal => self.exiting = true,
            _ => {},
        }
    }

    fn apply_size(&mut self, w: u32, h: u32) {
        // a barra entra na conta antes do grid: com N abas a grade é uma linha
        // mais baixa, e todas as abas acompanham (o pty de cada uma é resizado)
        if let Some(r) = self.renderer.as_mut() {
            let bar = r.fonts.ch * f32::from(u8::from(self.tabs.len() > 1));
            r.set_bar(bar);
            r.resize(w, h);
        }
        let (cols, rows) = self.grid_from_px(w, h);
        for t in &mut self.tabs {
            if cols != t.term.cols || rows != t.term.rows {
                t.term.resize(cols, rows);
                if let Some(pty) = t.pty.as_mut() {
                    pty.resize(cols as u16, rows as u16);
                }
                t.view_off = 0;
            }
        }
        if !self.spawned {
            if let Some(wake) = self.wake.clone() {
                self.spawned = true;
                self.spawn_shell(cols, rows, wake);
            }
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
                let sel_json = match self.tabs[self.active].sel {
                    Some(s) => serde_json::json!({
                        "a": [s.a.line, s.a.col],
                        "b": [s.b.line, s.b.col],
                        "scrolled": self.tabs[self.active].term.scrolled,
                    }),
                    None => serde_json::Value::Null,
                };
                debugsock::ok(serde_json::json!({
                    "pid": std::process::id(),
                    "cols": self.tabs[self.active].term.cols,
                    "rows": self.tabs[self.active].term.rows,
                    "cursor": [self.tabs[self.active].term.cy, self.tabs[self.active].term.cx],
                    "title": self.tabs[self.active].term.title,
                    "view_off": self.tabs[self.active].view_off,
                    "scrolled": self.tabs[self.active].term.scrolled,
                    "scrollback": self.tabs[self.active].term.scrollback.len(),
                    "alt": self.tabs[self.active].term.in_alt_screen(),
                    "sync": self.tabs[self.active].term.sync_output,
                    "mouse_mode": self.tabs[self.active].term.mouse_mode,
                    "mouse_sgr": self.tabs[self.active].term.mouse_sgr,
                    "mouse_utf8": self.tabs[self.active].term.mouse_utf8,
                    "cursor_px": [self.cursor_px.0, self.cursor_px.1],
                    "cell_at": [at.0, at.1],
                    "sel": sel_json,
                    "cell": [cw, ch],
                    "font": match &self.renderer { Some(r) => r.fonts.family.clone(), None => String::new() },
                    "font_px": match &self.renderer { Some(r) => r.fonts.px, None => 0.0 },
                    "tabs": self.tabs.len(),
                    "active": self.active,
                    "tab_titles": self.tabs.iter().map(Tab::title).collect::<Vec<_>>(),
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
                    "history" => self.tabs[self.active].term.history_text(start, count),
                    "screen" => (start..(start + count).min(self.tabs[self.active].term.rows))
                        .map(|y| self.tabs[self.active].term.row_text(y))
                        .collect(),
                    _ => self.tabs[self.active]
                        .term
                        .visible_rows(self.tabs[self.active].view_off)
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
                    if let Some(pty) = self.tabs[self.active].pty.as_mut() {
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
            // abas por socket: o que o teclado faz, testável sem XTest
            "tab" => {
                let op = req.get("op").and_then(|v| v.as_str()).unwrap_or("state");
                match op {
                    "new" => self.new_tab(),
                    "close" => self.close_tab(),
                    "select" => {
                        let n = req
                            .get("n")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0) as usize;
                        self.select_tab(n);
                    },
                    "state" => {},
                    outro => return debugsock::err(format!("op de tab: {outro:?}")),
                }
                debugsock::ok(serde_json::json!({
                    "tabs": self.tabs.len(),
                    "active": self.active,
                    "titles": self.tabs.iter().map(Tab::title).collect::<Vec<_>>(),
                }))
            },
            "keys" => match req.get("data").and_then(|v| v.as_str()) {
                Some(spec) => {
                    let bytes = encode_spec(spec, self.tabs[self.active].term.app_cursor);
                    if let Some(pty) = self.tabs[self.active].pty.as_mut() {
                        pty.write(&bytes);
                    }
                    let hex = hex(&bytes);
                    debugsock::ok(serde_json::json!({ "keys": [bytes.len()], "hex": hex }))
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
        self.spawn_in(self.active, cols, rows, wake, None);
    }

    /// Sobe o pty de uma aba (a primeira, ou uma nova herdando o cwd).
    fn spawn_in(
        &mut self,
        i: usize,
        cols: usize,
        rows: usize,
        wake: Arc<dyn Fn() + Send + Sync>,
        cwd: Option<&std::path::Path>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        match Pty::spawn(
            self.opts.cmd.as_deref(),
            self.opts.cfg.shell.as_deref(),
            cwd,
            cols as u16,
            rows as u16,
            tx,
            wake,
        ) {
            Ok(p) => {
                self.tabs[i].pty = Some(p);
                self.tabs[i].rx = Some(rx);
            },
            Err(e) => {
                eprintln!("bzmux: {e}");
                self.exiting = true;
            },
        }
    }

    /// Nova aba (I1): herda o cwd da aba corrente (I2) e vira a ativa.
    ///
    /// O `--cmd` vale só para a primeira: abrir outra aba com o comando de
    /// depuração repetiria o teste, não abriria um shell.
    fn new_tab(&mut self) {
        let Some(wake) = self.wake.clone() else {
            return;
        };
        let (cols, rows) = (
            self.tabs[self.active].term.cols,
            self.tabs[self.active].term.rows,
        );
        let cwd = self.tabs[self.active].cwd();
        let mut tab = Tab::new(cols, rows);
        tab.term.scrollback_limit = self.opts.scrollback;
        tab.term.theme = self.tabs[self.active].term.theme.clone();
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        // a barra pode ter aparecido agora: a grade de todas encolhe uma linha
        if let Some(w) = self.window.as_ref() {
            let s = w.inner_size();
            self.apply_size(s.width, s.height);
        }
        let (cols, rows) = (
            self.tabs[self.active].term.cols,
            self.tabs[self.active].term.rows,
        );
        // `--cmd` só vale para a primeira aba: repetir o comando de depuração
        // na segunda não abriria shell nenhum
        self.spawn_tab(self.active, cols, rows, wake, cwd.as_deref(), None);
        self.sync_title();
        self.redraw();
    }

    /// Sobe o pty de uma aba nova, com o `--cmd` só quando é permitido.
    fn spawn_tab(
        &mut self,
        i: usize,
        cols: usize,
        rows: usize,
        wake: Arc<dyn Fn() + Send + Sync>,
        cwd: Option<&std::path::Path>,
        cmd: Option<&str>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        match Pty::spawn(
            cmd,
            self.opts.cfg.shell.as_deref(),
            cwd,
            cols as u16,
            rows as u16,
            tx,
            wake,
        ) {
            Ok(p) => {
                self.tabs[i].pty = Some(p);
                self.tabs[i].rx = Some(rx);
            },
            Err(e) => {
                eprintln!("bzmux: {e}");
                self.exiting = true;
            },
        }
    }

    /// Fecha a aba ativa (I3).
    fn close_tab(&mut self) {
        self.close_tab_at(self.active);
    }

    /// Fecha uma aba (I3): mata o pty e escolhe a próxima. Fechar a última
    /// fecha o app (I20) — é o que o `Ctrl+Shift+W` no último terminal faz.
    fn close_tab_at(&mut self, i: usize) {
        if let Some(pty) = self.tabs[i].pty.as_mut() {
            pty.kill();
        }
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            self.exiting = true;
            return;
        }
        self.active = self.active.min(self.tabs.len() - 1);
        if let Some(w) = self.window.as_ref() {
            let s = w.inner_size();
            self.apply_size(s.width, s.height);
        }
        self.sync_title();
        self.redraw();
    }

    /// Troca de aba (Alt+N). Fora da faixa, não faz nada.
    fn select_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.active {
            return;
        }
        self.active = i;
        self.sync_title();
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Título da janela = título da aba ativa (I17), para o WM e a taskbar.
    fn sync_title(&mut self) {
        let titulo = self.tabs[self.active].title();
        if titulo == self.last_title {
            return;
        }
        self.last_title.clone_from(&titulo);
        if let Some(w) = self.window.as_ref() {
            w.set_title(&format!("{titulo} — bzmux"));
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
        // `ceil`: pedir `round` dá pixels a menos que o grid precisa e a
        // janela nasce com uma coluna/linha de menos (o WM não devolve)
        // a margem entra no pedido: a grade é `cols*cw` e o resto é padding
        let pad = 2.0 * self.opts.cfg.padding as f32;
        let w = (self.opts.cols as f32 * fonts.cw + pad).ceil() as u32;
        let h = (self.opts.rows as f32 * fonts.ch + pad).ceil() as u32;
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
        let renderer = match Renderer::new(&window, fonts, self.opts.vsync, &self.opts.cfg) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("bzmux: GPU: {e}");
                event_loop.exit();
                return;
            },
        };
        // o grid real vem do tamanho que a janela conseguiu, não do pedido
        let size = window.inner_size();
        self.renderer = Some(renderer);
        self.window = Some(window.clone());
        self.sync_title();
        let proxy = self.proxy.clone();
        self.wake = Some(Arc::new(move || {
            let _ = proxy.send_event(UserEvent::Wake);
        }));
        // aplica o tamanho agora e **só então** o filho nasce: com o filho
        // nascendo antes, os primeiros KB saem na largura do pedido e o
        // primeiro resize do WM embaralha o que já estava na tela
        self.apply_size(size.width, size.height);
        let (cols, rows) = (
            self.tabs[self.active].term.cols,
            self.tabs[self.active].term.rows,
        );

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
            WindowEvent::Focused(focused) if self.tabs[self.active].term.focus_report => {
                let seq = if focused { "\x1b[I" } else { "\x1b[O" };
                if let Some(pty) = self.tabs[self.active].pty.as_mut() {
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
        self.check_config();
        if !self.startup_errs.is_empty() {
            let errs = std::mem::take(&mut self.startup_errs);
            self.toast(aviso_config(&errs));
        }

        let agora = std::time::Instant::now();
        // o reload precisa de um tique mesmo sem evento nenhum
        let mut quando = agora + Duration::from_millis(250);

        // toast vencido sai da tela (ninguém mais o tiraria)
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.desde.elapsed() > TOAST_DUR)
        {
            self.toast = None;
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }

        // pisca: acorda na virada seguinte da fase; redesenha só quando ela
        // vira de verdade (um redesenho a cada 500 ms, não a cada tique)
        if self.pisca_ativo() {
            let e = self.blink_epoch.elapsed().as_millis() as u64;
            let proxima = (e / BLINK_MS + 1) * BLINK_MS;
            quando = quando.min(self.blink_epoch + Duration::from_millis(proxima));
            let fase = (e / BLINK_MS).is_multiple_of(2);
            if fase != self.blink_fase {
                self.blink_fase = fase;
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(quando));
        // flash do BEL (I13): acorda quando o primeiro rótulo apaga e
        // redesenha uma vez para tirar o destaque
        let agora = std::time::Instant::now();
        let mut apagou = false;
        for t in &mut self.tabs {
            if t.bell_at
                .is_some_and(|b| agora.duration_since(b) >= FLASH_DUR)
            {
                t.bell_at = None;
                apagou = true;
            }
        }
        if apagou {
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
        let proximo_flash = self
            .tabs
            .iter()
            .filter_map(|t| t.bell_at)
            .map(|b| b + FLASH_DUR)
            .min();
        if let Some(q) = proximo_flash {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                q.min(std::time::Instant::now() + Duration::from_millis(250)),
            ));
        }

        // aba cujo shell morreu fecha sozinha; a última fecha o app (I20)
        if let Some(morta) = (0..self.tabs.len()).find(|&i| {
            self.tabs[i]
                .pty
                .as_mut()
                .is_some_and(super::pty::Pty::child_exited)
        }) {
            self.drain();
            if self.tabs.len() == 1 {
                self.redraw();
                event_loop.exit();
            } else {
                self.close_tab_at(morta);
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if prof::enabled() || self.opts.prof {
            let summary = prof::summary();
            eprintln!("prof: {summary}");
            logln(&format!("prof: {summary}"));
        }
        if let Some(pty) = self.tabs[self.active].pty.as_mut() {
            pty.kill();
        }
        self.debug = None;
    }
}
