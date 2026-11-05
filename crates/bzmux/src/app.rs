//! O app: janela, pty, teclado, desenho e o socket de introspecção.

use std::io::Write as _;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bzmux_core::config::Bell;
use bzmux_core::{Anchor, Span, Term};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
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

/// Argumento para o shell: `'…'` com `'` virando `'\''` (o mesmo que o
/// `shlex.quote` do Python faz para o caso comum).
fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+:%=@,".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Super puro (sem Ctrl nem Alt): o alias de workspace do Python.
fn super_only(mods: ModifiersState) -> bool {
    mods.super_key() && !mods.control_key() && !mods.alt_key()
}

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
fn descendant(pid: u32) -> u32 {
    let mut cur = pid;
    for _ in 0..8 {
        let path = format!("/proc/{cur}/task/{cur}/children");
        let Ok(txt) = std::fs::read_to_string(path) else {
            break;
        };
        let Some(child) = txt.split_whitespace().next().and_then(|s| s.parse().ok()) else {
            break;
        };
        cur = child;
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
    text: String,
    desde: std::time::Instant,
    /// Quantas mensagens iguais chegaram seguidas (o toast soma, não empilha).
    vezes: u32,
}

/// Quanto tempo o toast fica na tela.
const TOAST_DUR: Duration = Duration::from_millis(3500);

/// Texto do aviso de configuração (K13). Cabe em ~60 colunas de propósito: o
/// toast é cortado na largura da grade, e "linha e chave" é o que se precisa
/// ver, não a contagem escrita por extenso.
fn notice_config(errs: &[bzmux_core::config::Error]) -> String {
    let Some(first) = errs.first() else {
        return String::new();
    };
    if errs.len() > 1 {
        format!("config mantida — {first} (+{})", errs.len() - 1)
    } else {
        format!("config mantida — {first}")
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
    /// Identidade da aba, monotônica e única na execução. O dano por linha
    /// (A17/G17) guarda instâncias por linha do `Renderer`, então trocar de aba
    /// com a mesma geometria não pode reaproveitar linha nenhuma — e índice não
    /// serve: fechar e abrir aba volta o índice, a identidade não.
    pub id: u64,
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
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Tab {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
        let name = bzmux_core::term::sanitize_title(&cwd.file_name()?.to_string_lossy());
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    /// cwd do que está rodando **na frente** do pty: o shell pode ter lançado
    /// `vim`/`mc`, e é o cwd deles que a próxima aba deve herdar (é o que o
    /// `fork` do Python fazia ao olhar o último descendente).
    fn cwd(&self) -> Option<std::path::PathBuf> {
        let pid = self.pty.as_ref()?.child_pid()?;
        let target = descendant(pid);
        std::fs::read_link(format!("/proc/{target}/cwd")).ok()
    }
}

/// Nome em edição no cartão (I8).
struct Rename {
    /// Índice lógico do workspace.
    ws: usize,
    text: String,
    /// O nome antigo está inteiro no campo e ainda não foi tocado: a primeira
    /// tecla o substitui, como o "selecionado" do `Gtk.Entry` do Python.
    replace: bool,
}

/// Um conjunto de abas com nome próprio (I5).
///
/// O workspace da frente mora em [`App::tabs`]/[`App::active`]; os outros
/// ficam inteiros aqui (pty vivo, grade própria), e trocar de workspace é
/// `mem::swap` do conteúdo — nada de copiar termo nem matar processo.
struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    name: String,
}

impl Workspace {
    /// Embrulha a sessão corrente para estacionar.
    fn take(app: &mut App) -> Workspace {
        Workspace {
            tabs: std::mem::take(&mut app.tabs),
            active: app.active,
            name: std::mem::take(&mut app.ws_name),
        }
    }

    /// Devolve o conteúdo para o lugar da sessão corrente.
    fn into_current(self, app: &mut App) {
        app.tabs = self.tabs;
        app.active = self.active.min(app.tabs.len().saturating_sub(1));
        app.ws_name = self.name;
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
    /// Nome do workspace da frente (`workspace 1`, renomeável — I8).
    ws_name: String,
    /// Os workspaces que não estão na frente. O da frente não mora aqui: ele
    /// **é** `tabs`/`active`. Invariante: `parked.len() + 1 == ws_count()`,
    /// com os índices lógicos em ordem (o da frente cai no meio).
    parked: Vec<Workspace>,
    /// Índice lógico do workspace da frente em `0..ws_count()`.
    ws_idx: usize,
    /// Aba apertada na barra, para arrastar e reordenar (I10): índice e o x
    /// do press, para só começar o arrasto depois de andar de verdade.
    bar_press: Option<(usize, f64)>,
    /// Última tecla comida por um atalho local: a soltura dela não vai ao
    /// filho (o protocolo do kitty relata soltura, e um atalho comido não pode
    /// virar `CSI ...:3u`).
    consumido: Option<winit::keyboard::PhysicalKey>,
    /// Renomeando um workspace no cartão (I8). Enquanto é `Some`, o teclado é
    /// do nome — o filho não vê as teclas.
    rename: Option<Rename>,
    /// Instante do último clique no cartão, para o duplo clique (I8).
    card_click: Option<(usize, std::time::Instant)>,
    /// Último giro tratado na barra (instante e sentido). O X11 manda
    /// **dois** eventos por entalhe (press e release), e sem isto um entalhe
    /// troca de aba e o segundo desfaz.
    bar_wheel: Option<(std::time::Instant, i8)>,
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
    blink_phase: bool,
    /// Há célula com SGR 5 na tela. Recalculado a cada `feed`: perguntar isso
    /// ao grid em cada tique custaria uma varredura por evento.
    blink_text: bool,
    /// Portão do desenho (G16/B28/G15): ritmo de ~16 ms e `?2026` segurado.
    gate: crate::frame::Gate,
    /// Aviso na tela (erro de config etc.). Um por vez, some sozinho.
    toast: Option<Toast>,
    /// Erros do arquivo no arranque, que só podem virar toast quando a janela
    /// existir (no terminal, `stderr` some; numa sessão gráfica, idem).
    startup_errs: Vec<bzmux_core::config::Error>,
    /// Erros da config que estão valendo agora (o `errors` do `state`): o
    /// arquivo recusado mantém a config anterior, mas o erro é fato.
    cfg_errors: Vec<String>,
    debug: Option<DebugServer>,
    /// Grid pedido pelo verbo `resize`; o WM pode arredondar em pixels, então
    /// `apply_size` corrige em passos até bater (como um terminal de verdade).
    want: Option<(usize, usize)>,
    adjust_left: u8,
    eof: bool,
    /// Último título posto na janela (evita `set_title` por frame).
    last_title: String,
    exiting: bool,
    /// `--quit-after`: instante em que fecha sozinho (automação).
    deadline: Option<std::time::Instant>,
    /// Um dump já foi escrito nesta execução (SIGUSR1 pode vir várias vezes).
    dumpou: bool,
    /// Tamanho normal da janela (J5: maximizado **não** sobrescreve este).
    geom_normal: (u32, u32),
    /// Última posição externa vista (moldura, como o `get_position()` do Python).
    geom_pos: Option<(i32, i32)>,
    /// O estado lembrado só vale se ninguém pediu grade na linha de comando.
    remember_geom: bool,
}

/// O WM aqui é X11? É ele que aceita posição de janela; no Wayland ela não
/// existe (J6). O `winit` não expõe qual backend escolheu, então a leitura é do
/// ambiente — a mesma escolha dele: Wayland quando `WAYLAND_DISPLAY` existe.
fn backend_x11() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_none_or(|v| v.is_empty())
        && std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
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
        let deadline = opts
            .quit_after
            .map(|seg| std::time::Instant::now() + std::time::Duration::from_secs(seg));
        App {
            opts,
            over,
            proxy,
            window: None,
            renderer: None,
            tabs: vec![tab],
            active: 0,
            ws_name: "workspace 1".to_string(),
            parked: Vec::new(),
            ws_idx: 0,
            bar_press: None,
            consumido: None,
            rename: None,
            card_click: None,
            bar_wheel: None,
            mods: ModifiersState::empty(),
            cursor_px: (0.0, 0.0),
            held: None,
            clip: Clip::new(),
            wake: None,
            spawned: false,
            cfg_mtime,
            cfg_checked: std::time::Instant::now(),
            blink_epoch: std::time::Instant::now(),
            blink_phase: true,
            blink_text: false,
            gate: crate::frame::Gate::default(),
            toast: None,
            startup_errs: Vec::new(),
            cfg_errors: Vec::new(),
            debug: None,
            want: None,
            adjust_left: 0,
            eof: false,
            last_title: String::new(),
            exiting: false,
            deadline,
            dumpou: false,
            geom_normal: crate::state::PADRAO,
            geom_pos: None,
            remember_geom: true,
        }
    }

    /// Texto de todas as abas de todos os workspaces (o `dump` do Python):
    /// scrollback (as últimas 40) e o viewport de cada sessão, com cabeçalho
    /// quando há mais de uma.
    fn dump_text(&self) -> String {
        let mut partes: Vec<String> = Vec::new();
        let mut multi_ws = self.ws_count() > 1;
        multi_ws = multi_ws || self.parked.len() > 1;
        // a ordem lógica: os estacionados nas suas posições e o corrente no
        // lugar dele (o `parked` guarda os outros inteiros, na ordem)
        let mut i_ws = 0usize;
        let mut empurra = |tabs: &[Tab], active: usize, name: &str, partes: &mut Vec<String>| {
            if multi_ws {
                partes.push(format!("=== workspace {}: {name} ===", i_ws + 1));
            }
            let multi_tab = tabs.len() > 1;
            for (i, t) in tabs.iter().enumerate() {
                if multi_tab {
                    partes.push(format!("=== aba {}: {} ===", i + 1, t.title()));
                }
                partes.extend(dump_tab(t, i == active));
            }
            i_ws += 1;
        };
        // o corrente entra na posição dele: antes dele vêm os estacionados de
        // índice menor
        let mut feitos = 0usize;
        for slot in 0..self.parked.len() {
            if feitos == self.ws_idx {
                empurra(&self.tabs, self.active, &self.ws_name, &mut partes);
                feitos += 1;
            }
            let w = &self.parked[slot];
            empurra(&w.tabs, w.active, &w.name, &mut partes);
            feitos += 1;
        }
        if feitos == self.ws_idx {
            empurra(&self.tabs, self.active, &self.ws_name, &mut partes);
        }
        let mut s = partes.join("\n");
        s.push('\n');
        s
    }

    /// Escreve o dump onde o `--dump-to` mandou (senão /tmp/bzmux-dump.txt).
    fn dump(&mut self) {
        let path = self
            .opts
            .dump_to
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/bzmux-dump.txt"));
        match std::fs::write(&path, self.dump_text()) {
            Ok(()) => {
                self.dumpou = true;
                logln(&format!("dump: {}", path.display()));
            },
            Err(e) => eprintln!("dump falhou: {}: {e}", path.display()),
        }
    }

    /// XID da janela no X11 (0 no Wayland): quem vai capturar pixel pergunta
    /// ao app, não ao X (é o que o `benchrun.py` do Python faz).
    fn xid(&self) -> u64 {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Some(w) = self.window.as_ref() else {
            return 0;
        };
        match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xlib(x)) => x.window,
            Ok(RawWindowHandle::Xcb(x)) => u64::from(x.window.get()),
            _ => 0,
        }
    }

    /// Escrita no pty com contador (`prof`): bytes e repetições por `EAGAIN`.
    fn pty_write(&mut self, bytes: &[u8]) {
        let Some(pty) = self.tabs[self.active].pty.as_mut() else {
            return;
        };
        prof::add_write(bytes.len());
        if !pty.write(bytes) {
            prof::add_eagain();
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

    /// Os cartões da sidebar (I6), em ordem lógica: os estacionados antes do
    /// da frente e os depois, na posição correta.
    fn wscards(&self) -> Vec<crate::render::WsCard> {
        let total = self.ws_count();
        let mut cards = Vec::with_capacity(total);
        for i in 0..total {
            let em_edicao = self
                .rename
                .as_ref()
                .filter(|r| r.ws == i)
                .map(|r| format!("{}|", r.text));
            if i == self.ws_idx {
                cards.push(crate::render::WsCard {
                    name: em_edicao.unwrap_or_else(|| self.ws_name.clone()),
                    titulo: self.tabs[self.active].title(),
                    n_tabs: self.tabs.len(),
                    ativa: true,
                });
            } else {
                let slot = if i < self.ws_idx { i } else { i - 1 };
                let w = &self.parked[slot];
                cards.push(crate::render::WsCard {
                    name: em_edicao.unwrap_or_else(|| w.name.clone()),
                    titulo: w.tabs[w.active].title(),
                    n_tabs: w.tabs.len(),
                    ativa: false,
                });
            }
        }
        cards
    }

    /// Largura da sidebar em px (0 com um workspace só — I7).
    fn side_w(&self, cw: f32) -> f32 {
        if self.ws_count() > 1 {
            cw * 18.0
        } else {
            0.0
        }
    }

    /// O que a barra mostra: número, título e alerta de BEL de cada aba.
    fn tabinfo(&self) -> Vec<crate::render::TabInfo> {
        let now = std::time::Instant::now();
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| crate::render::TabInfo {
                titulo: t.title(),
                ativa: i == self.active,
                alert: t
                    .bell_at
                    .is_some_and(|when| now.duration_since(when) < FLASH_DUR),
            })
            .collect()
    }

    fn grid_from_px(&self, w: u32, h: u32) -> (usize, usize) {
        match &self.renderer {
            Some(r) => {
                // margem, barra de abas e sidebar não são grade
                let bar = self.bar_h(r.fonts.ch);
                let side = self.side_w(r.fonts.cw);
                let (iw, ih) = (
                    (w as f32 - 2.0 * r.pad - side).max(r.fonts.cw),
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
        // descontar a origem: com margem, barra de abas e sidebar, o (0,0)
        // da grade não é o (0,0) da janela
        let bar = f64::from(self.bar_h(ch));
        let side = f64::from(self.side_w(cw));
        let col = ((px - f64::from(pad) - side) / f64::from(cw)).max(0.0) as usize;
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
        // a barra começa onde a grade começa (à direita da sidebar): a linha 1
        // é dividida entre o cartão do workspace e a barra de abas
        let mut x = f64::from(pad) + f64::from(self.side_w(cw));
        for (i, t) in info.iter().enumerate() {
            let rotulo = format!(" {} {} ", i + 1, t.titulo);
            let width = rotulo.chars().count() as f64 * f64::from(cw);
            if px >= x && px < x + width {
                return Some(i);
            }
            x += width;
        }
        None
    }

    /// Workspace sob o cursor, se ele estiver na sidebar (I9).
    fn card_at(&self, px: f64, py: f64) -> Option<usize> {
        let (cw, ch, pad) = self
            .renderer
            .as_ref()
            .map_or((10.0, 19.0, 0.0), |r| (r.fonts.cw, r.fonts.ch, r.pad));
        let side = f64::from(self.side_w(cw));
        if side <= 0.0 || px >= f64::from(pad) + side || px < f64::from(pad) {
            return None;
        }
        // a sidebar começa na linha 1, ao lado da barra de abas (não abaixo)
        let top = f64::from(pad);
        if py < top {
            return None;
        }
        let card_h = f64::from(ch) * 2.4;
        let i = ((py - top) / card_h) as usize;
        if i < self.ws_count() {
            Some(i)
        } else {
            None
        }
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
        self.pty_write(&bytes);
    }

    /// Atalhos Ctrl+Shift e Super de workspace/edidor: extraído de `on_key`,
    /// que só manda o que sobrar para o encoder.
    #[allow(clippy::too_many_lines)] // um braço por atalho
    fn shortcut_ws(&mut self, k: &KeyIn<'_>, named: Option<NamedKey>, ch: Option<char>) -> bool {
        let ctrl = k.mods.control_key();
        let shift = k.mods.shift_key();
        if ctrl && shift {
            match ch.map(|c| c.to_ascii_lowercase()) {
                Some('t') => {
                    self.new_tab();
                    return true;
                },
                Some('w') => {
                    self.close_tab();
                    return true;
                },
                Some('n') => {
                    self.new_workspace();
                    return true;
                },
                Some('q') => {
                    let i = self.ws_idx;
                    self.close_workspace(i);
                    return true;
                },
                _ => {},
            }
            // Ctrl+Shift+F2: o arquivo de configuração no editor, em aba nova
            // (P33 do Python). Fica **antes** do encoding: o vim nunca vê a
            // tecla, senão ele a interpreta como atalho próprio.
            if named == Some(NamedKey::F2) {
                self.open_config();
                return true;
            }
            // Ctrl+Shift+1..9 escolhe o workspace (0 = o décimo)
            if let Some(n) = k.digit {
                let i = if n == 0 { 9 } else { n - 1 };
                self.select_workspace(i);
                return true;
            }
        }
        // Super+N/Q e Super+1..9: alias de workspace (o WM pode engolir, mas
        // quando chega é o mesmo efeito — é o que o Python fazia)
        if super_only(self.mods) {
            match ch.map(|c| c.to_ascii_lowercase()) {
                Some('n') => {
                    self.new_workspace();
                    return true;
                },
                Some('q') => {
                    let i = self.ws_idx;
                    self.close_workspace(i);
                    return true;
                },
                _ => {},
            }
            if let Some(n) = k.digit {
                let i = if n == 0 { 9 } else { n - 1 };
                self.select_workspace(i);
                return true;
            }
        }
        false
    }

    /// Tecla: atalhos da janela primeiro (o filho nunca vê Ctrl+Shift+C/V),
    /// depois o encoder. Qualquer tecla de verdade volta ao vivo.
    fn on_key(&mut self, event: &winit::event::KeyEvent) {
        let press = event.state == ElementState::Pressed;
        if press {
            let mut k = KeyIn::from_event(event);
            k.mods = self.mods;
            if self.press_locais(&k) {
                // a tecla foi do app: a soltura dela também é (senão o release
                // de Ctrl+Shift+1 virava `CSI 49;1:3u` no filho)
                self.consumido = Some(event.physical_key);
                return;
            }
            self.consumido = None;
        } else if self.consumido == Some(event.physical_key) {
            // o X11 solta Shift/Ctrl **antes** da tecla: o marcador não pode
            // ser consumido por elas (era o vazamento do `CSI 49;1:3u`)
            self.consumido = None;
            return;
        } else if self.tabs[self.active].term.kbd_flags & keys::F_EVENT_TYPES == 0 {
            return;
        }
        let Some(bytes) = keys::encode(
            event,
            self.mods,
            self.tabs[self.active].term.app_cursor,
            self.tabs[self.active].term.kbd_flags,
        ) else {
            return;
        };
        self.pty_write(&bytes);
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Atalhos que comem a tecla antes do encoder. `true` = o filho não vê
    /// nem o aperto nem a soltura.
    fn press_locais(&mut self, k: &KeyIn<'_>) -> bool {
        self.blink_reset();
        // renomeando no cartão: o teclado é do nome, não do terminal (I8)
        if self.rename_key(k) {
            return true;
        }
        let ctrl = k.mods.control_key();
        let shift = k.mods.shift_key();
        let named = k.named;
        let ch = k.character.and_then(|s| s.chars().next());

        if self.shortcut_ws(k, named, ch) {
            return true;
        }
        // Alt+1..9 escolhe a aba; Alt+0 é a décima (I15). O dígito não vaza
        // para o filho — é o que o Python fazia.
        if alt_digit_only(k.mods) {
            if let Some(n) = k.digit {
                self.select_tab(if n == 0 { 9 } else { n - 1 });
                return true;
            }
        }
        // Ctrl+Shift+C/V são do terminal, não do app: sem isso o Ctrl+Shift+C
        // vira o mesmo ^C de Ctrl+C e mata o processo (bug do P-bzmux)
        if ctrl && shift {
            match ch.map(|c| c.to_ascii_lowercase()) {
                Some('c') => {
                    self.copy();
                    return true;
                },
                Some('v') => {
                    self.paste();
                    return true;
                },
                _ => {},
            }
        }
        // Shift+PgUp/PgDn/Home/End olham o histórico sem falar com o app
        if shift && !ctrl {
            match named {
                Some(NamedKey::PageUp) => {
                    self.scroll_view(self.tabs[self.active].term.rows as i64 - 1);
                    return true;
                },
                Some(NamedKey::PageDown) => {
                    self.scroll_view(-(self.tabs[self.active].term.rows as i64 - 1));
                    return true;
                },
                Some(NamedKey::Home) if !self.tabs[self.active].term.scrollback.is_empty() => {
                    self.scroll_view(self.tabs[self.active].term.scrollback.len() as i64);
                    return true;
                },
                Some(NamedKey::End) => {
                    self.scroll_view(-(self.tabs[self.active].term.scrollback.len() as i64));
                    return true;
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
        false
    }

    /// Movimento: só interessa ao app quando ele pediu `?1002`/`?1003`.
    fn on_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_px = (x, y);
        // arrasto sobre a barra reordena aba (I10); sobre a grade é seleção
        if self.bar_press.is_some() {
            self.bar_drag();
            return;
        }
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
                // clique em cartão de workspace também não é do app
                if let Some(i) = self.card_at(self.cursor_px.0, self.cursor_px.1) {
                    self.select_workspace(i);
                    self.held = None;
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                    return;
                }
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
        // roda na barra de abas circula entre elas (I11); na grade é rolagem
        if self.tab_at(self.cursor_px.0, self.cursor_px.1).is_some() {
            let dir: f32 = match delta {
                MouseScrollDelta::LineDelta(_, y) => y,
                MouseScrollDelta::PixelDelta(p) => (p.y as f32).signum() * 3.0,
            };
            // press e release do mesmo entalhe chegam quase juntos (1 ms) e
            // têm o mesmo sentido; duas voltas de verdade ficam a >10 ms
            let now = std::time::Instant::now();
            let direction: i8 = if dir > 0.0 { 1 } else { -1 };
            let repetido = self.bar_wheel.is_some_and(|(when, previous)| {
                previous == direction && now.duration_since(when).as_millis() < 10
            });
            if repetido {
                return;
            }
            self.bar_wheel = Some((now, direction));
            let n = self.tabs.len();
            let at = if dir > 0.0 {
                (self.active + n - 1) % n
            } else {
                (self.active + 1) % n
            };
            self.select_tab(at);
            return;
        }
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
                // clique no cartão da sidebar troca de workspace (I9) e o
                // duplo clique edita o nome no lugar (I8)
                if let Some(i) = self.card_at(self.cursor_px.0, self.cursor_px.1) {
                    self.bar_press = None;
                    let double = self
                        .card_click
                        .is_some_and(|(j, when)| j == i && when.elapsed().as_millis() < 400);
                    self.card_click = Some((i, std::time::Instant::now()));
                    if double {
                        self.card_click = None;
                        self.start_rename(i);
                    } else {
                        self.select_workspace(i);
                    }
                    return;
                }
                // clique na barra de abas troca de aba (não seleciona texto);
                // 'apertou' fica guardado para o arrasto (I10)
                if let Some(i) = self.tab_at(self.cursor_px.0, self.cursor_px.1) {
                    self.bar_press = Some((i, self.cursor_px.0));
                    self.select_tab(i);
                    self.tabs[self.active].dragging = false;
                    self.tabs[self.active].press = None;
                    self.redraw();
                    return;
                }
                self.bar_press = None;
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
                // soltar na barra encerra o arrasto de reordenação (I10)
                self.bar_press = None;
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
                // botão do meio no CARTÃO fecha o workspace (I9)
                if let Some(i) = self.card_at(self.cursor_px.0, self.cursor_px.1) {
                    self.close_workspace(i);
                    return;
                }
                // botão do meio NA ABA fecha ela (I9); na grade cola o PRIMARY
                if let Some(i) = self.tab_at(self.cursor_px.0, self.cursor_px.1) {
                    self.bar_press = None;
                    self.close_tab_at(i);
                    return;
                }
                // colar fala com o app, então a visão vai para o vivo
                self.tabs[self.active].view_off = 0;
                if let Some(txt) = self.clip.primary_text() {
                    self.write_paste(&txt);
                }
            },
            _ => {},
        }
    }

    /// Arrasto de reordenação na barra (I10): andou uma aba de largura com o
    /// botão apertado sobre a barra, a aba troca de lugar. Só anda de verdade
    /// depois de passar da metade do próprio rótulo (senão balança).
    fn bar_drag(&mut self) {
        let Some((i, x0)) = self.bar_press else {
            return;
        };
        let Some((cw, _, pad)) = self
            .renderer
            .as_ref()
            .map(|r| (r.fonts.cw, r.fonts.ch, r.pad))
        else {
            return;
        };
        let info = self.tabinfo();
        // onde o press começou e onde estamos: x pixel da aba i no arrasto
        let mut x = f64::from(pad);
        let mut today = None;
        for (k, t) in info.iter().enumerate() {
            let width = format!(" {} {} ", k + 1, t.titulo).chars().count() as f64 * f64::from(cw);
            if k == i {
                today = Some((x, width));
                break;
            }
            x += width;
        }
        let Some((_, width)) = today else {
            return;
        };
        let dx = self.cursor_px.0 - x0;
        let step = width.max(f64::from(cw));
        let n = ((dx / step).round() as i64).unsigned_abs() as usize;
        if n == 0 || dx.abs() < step / 2.0 {
            return;
        }
        let dest = if dx > 0.0 { i + n } else { i.saturating_sub(n) };
        self.move_tab(i, dest);
        self.bar_press = Some((dest, x0));
        self.redraw();
    }

    /// Reordena (I10): remove de `i` e insere em `d`; a ativa segue a aba.
    fn move_tab(&mut self, i: usize, d: usize) {
        let d = d.min(self.tabs.len() - 1);
        if i == d || i >= self.tabs.len() {
            return;
        }
        let t = self.tabs.remove(i);
        self.tabs.insert(d, t);
        if self.active == i {
            self.active = d;
        } else if i < self.active && d >= self.active {
            self.active -= 1;
        } else if i > self.active && d <= self.active {
            self.active += 1;
        }
        self.redraw();
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
        let new = Span::new(
            anchor,
            self.tabs[self.active]
                .term
                .anchor_at(self.tabs[self.active].view_off, row, col),
        );
        if self.tabs[self.active].sel != Some(new) {
            self.tabs[self.active].sel = Some(new);
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
        self.pty_write(&payload);
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
                let theme = self.tabs[i].term.theme.clone();
                if let Some(r) = self.renderer.as_mut() {
                    r.apply_theme(&theme);
                }
            }
            // I4: título por OSC 0/2 (o shell põe "~/projeto" e o `vim` o nome
            // do arquivo); o `bzmux` é o padrão e não conta como pedido
            let titulo = self.tabs[i].term.title.clone();
            if titulo != self.tabs[i].osc_title {
                let was = i == self.active;
                self.tabs[i].osc_title = if titulo == "bzmux" {
                    String::new()
                } else {
                    titulo
                };
                if was {
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
            prof::add_write(out.len());
            if let Some(pty) = self.tabs[i].pty.as_mut() {
                if !pty.write(&out) {
                    prof::add_eagain();
                }
            }
        }
    }

    /// Erros do arquivo lidos no arranque: viram toast no primeiro tique.
    pub fn startup_errors(&mut self, errs: Vec<bzmux_core::config::Error>) {
        self.cfg_errors = errs.iter().map(std::string::ToString::to_string).collect();
        self.startup_errs = errs;
    }

    /// Zera o pisca: logo depois de uma tecla o cursor tem que estar aceso.
    /// Sem isto o cursor pode "sumir" no exato instante em que a pessoa olha.
    fn blink_reset(&mut self) {
        self.blink_epoch = std::time::Instant::now();
        if !self.blink_phase {
            self.blink_phase = true;
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
        let phase = self.blink_phase;
        crate::render::Blink {
            phase,
            cursor: !self.cursor_pisca() || phase,
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
    fn pisca_active(&self) -> bool {
        (self.cursor_pisca() && self.tabs[self.active].term.cursor_visible)
            || self.has_text_piscando()
    }

    /// Há célula com SGR 5 na tela? Sem isto o loop não teria por que acordar
    /// no meio do período do pisca quando o cursor está fixo.
    fn has_text_piscando(&self) -> bool {
        self.blink_text
    }

    /// Mostra um aviso na tela. Mensagem igual seguida vira contagem ("×3"),
    /// não uma fila — quem salva o arquivo dez vezes não quer dez avisos.
    #[allow(clippy::needless_pass_by_value)] // quem chama monta o texto na hora
    fn toast(&mut self, text: String) {
        let vezes = match self.toast.take() {
            Some(t) if t.text == text => t.vezes + 1,
            _ => 1,
        };
        self.toast = Some(Toast {
            text: text.clone(),
            desde: std::time::Instant::now(),
            vezes,
        });
        logln(&format!("toast: {text}"));
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Texto do toast para o renderer (com a contagem quando houve repetição).
    fn toast_text(&self) -> Option<String> {
        let t = self.toast.as_ref()?;
        if t.desde.elapsed() > TOAST_DUR {
            return None;
        }
        Some(if t.vezes > 1 {
            format!("{} (×{})", t.text, t.vezes)
        } else {
            t.text.clone()
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
        const INTERVAL: Duration = Duration::from_millis(250);
        if self.cfg_checked.elapsed() < INTERVAL {
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
        let (new, errs) = crate::cli::Options::build(self.over.clone());
        self.cfg_errors = errs.iter().map(std::string::ToString::to_string).collect();
        if !errs.is_empty() {
            logln(&format!(
                "config: {} mantida; {} erro(s), o primeiro em {}",
                path.display(),
                errs.len(),
                errs[0]
            ));
            // K13: o arquivo é editado num editor, que não mostra o nosso log —
            // o aviso tem que aparecer na tela, com a primeira linha e a conta.
            self.toast(notice_config(&errs));
            return;
        }
        logln(&format!("config: {} recarregada", path.display()));
        // só avisa na tela se havia um erro aparecendo: recarregar deu certo e
        // quem salvou precisa saber que voltou a valer
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.text.starts_with("config"))
        {
            self.toast("config recarregada".to_string());
        }
        self.apply_options(new, &text);
    }

    /// Aplica configuração nova no que já está rodando.
    fn apply_options(&mut self, new: Options, text: &str) {
        let font_mudou = new.font_family != self.opts.font_family
            || (new.font_size_pt - self.opts.font_size_pt).abs() > f32::EPSILON
            || (new.dpi - self.opts.dpi).abs() > f32::EPSILON;
        let log_mudou = new.debug_log != self.opts.debug_log;
        self.tabs[self.active].term.scrollback_limit = new.scrollback;
        while self.tabs[self.active].term.scrollback.len()
            > self.tabs[self.active].term.scrollback_limit
        {
            self.tabs[self.active].term.scrollback.pop_front();
        }
        self.opts.cfg = new.cfg.clone();
        self.opts.scrollback = new.scrollback;
        self.tabs[self.active].term.theme = new.cfg.theme();
        if let Some(r) = self.renderer.as_mut() {
            r.apply_theme(&self.tabs[self.active].term.theme);
            r.apply_layout(&new.cfg);
        }
        if let Some(path) = new.debug_log.as_deref() {
            set_log(path);
        }
        let _ = text;
        if font_mudou {
            self.opts.font_family = new.font_family;
            self.opts.font_size_pt = new.font_size_pt;
            self.opts.dpi = new.dpi;
            self.rebuild_renderer();
        }
        if log_mudou && new.debug_log.is_none() {
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
        let (Some(window), Some(old)) = (self.window.clone(), self.renderer.take()) else {
            return;
        };
        let scale = window.scale_factor();
        let fonts = match Fonts::new(&self.opts.font_family, self.font_px(scale)) {
            Ok(f) => f,
            Err(e) => {
                logln(&format!("fonte: {e}"));
                self.renderer = Some(old);
                return;
            },
        };
        drop(old);
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
        let toast = self.toast_text();
        let tabs = if self.tabs.len() > 1 {
            Some(self.tabinfo())
        } else {
            None
        };
        let cards = self.wscards();
        let sidebar = if self.ws_count() > 1 {
            Some(cards)
        } else {
            None
        };
        let frame = crate::render::Frame {
            tab: self.tabs[self.active].id,
            term: &self.tabs[self.active].term,
            view_off: self.tabs[self.active].view_off,
            sel: self.tabs[self.active].sel,
            blink,
            toast: toast.as_deref(),
            tabs: tabs.as_deref(),
            sidebar: sidebar.as_deref(),
        };
        let desenhou = {
            let Some(renderer) = self.renderer.as_mut() else {
                return;
            };
            match renderer.render(&frame) {
                crate::render::FrameOutcome::Drawn => true,
                crate::render::FrameOutcome::Reconfigure => {
                    let (w, h) = (renderer.config.width, renderer.config.height);
                    renderer.resize(w, h);
                    false
                },
                crate::render::FrameOutcome::Fatal => {
                    self.exiting = true;
                    false
                },
                crate::render::FrameOutcome::Skipped => false,
            }
        };
        // dano consumido **só** quando o frame foi mesmo apresentado: um frame
        // pulado (oclusão/timeout) não pode apagar a marca, senão a linha suja
        // fica velha na tela para sempre (A17/G17)
        if desenhou {
            self.tabs[self.active].term.clear_damage();
        }
    }

    fn apply_size(&mut self, w: u32, h: u32) {
        // a barra entra na conta antes do grid: com N abas a grade é uma linha
        // mais baixa, e todas as abas acompanham (o pty de cada uma é resizado)
        let (ch, _cw, side) = self.renderer.as_ref().map_or((0.0, 0.0, 0.0), |r| {
            (r.fonts.ch, r.fonts.cw, self.side_w(r.fonts.cw))
        });
        let bar = ch * f32::from(u8::from(self.tabs.len() > 1));
        if let Some(r) = self.renderer.as_mut() {
            r.set_bar(bar);
            r.set_side(side);
            r.resize(w, h);
        }
        let (cols, rows) = self.grid_from_px(w, h);
        let cell = self.cell_px();
        // o parser precisa do mesmo tamanho de célula para responder
        // `CSI 14/16 t` e para o `c=`/`r=` default das imagens (H9). Era o
        // padrão 10x19 do core: a imagem saía 5x3 células em vez de 4x2 numa
        // célula de 15x29.
        let cell_u = (cell.0.max(1.0) as u32, cell.1.max(1.0) as u32);
        for t in &mut self.tabs {
            t.term.cell_size = cell_u;
            if cols != t.term.cols || rows != t.term.rows {
                t.term.resize(cols, rows);
                if let Some(pty) = t.pty.as_mut() {
                    pty.resize(cols as u16, rows as u16, cell);
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
                    // dano (A17/G17): quantas linhas/células foram remontadas
                    // desde o início. É a prova de que o desenho parcial
                    // funciona — sem depender de olhar a tela.
                    "draws": self.renderer.as_ref().map_or(0, |r| r.draw),
                    "last_frame": self.renderer.as_ref().map_or("", |r| r.last),
                    "drawn": self.renderer.as_ref().map_or(0, |r| r.drawn),
                    "skips": self.renderer.as_ref().map_or(0, |r| r.skips),
                    "last_damage": self.renderer.as_ref().map_or("", |r| r.last_damage.as_str()),
                    "frames_all": self.renderer.as_ref().map_or(0, |r| r.frames_all),
                    "rows_max": self.renderer.as_ref().map_or(0, |r| r.rows_max),
                    "all_reason": self.renderer.as_ref().map_or("", |r| r.all_reason),
                    "dirty_all": self.tabs[self.active].term.dirty_all,
                    "global_marks": self.tabs[self.active].term.global_marks,
                    "global_why": self.tabs[self.active].term.global_why,
                    "dirty_rows": self.tabs[self.active].term.dirty.iter().filter(|d| **d).count(),
                    "rows_built": self.renderer.as_ref().map_or(0, |r| r.linhas_remontadas),
                    "cells_built": self.renderer.as_ref().map_or(0, |r| r.celulas_remontadas),
                    // imagens do kitty: sem isto não há como conferir a
                    // colocação sem olhar a tela
                    "images": self.tabs[self.active].term.graphics.images().len(),
                    "placements": self.tabs[self.active]
                        .term
                        .graphics
                        .placements()
                        .iter()
                        .map(|p| {
                            serde_json::json!({
                                "id": p.image_id,
                                "col": p.col,
                                "row": p.line,
                                "cols": p.cols,
                                "rows": p.rows,
                                "dx": p.dx,
                                "dy": p.dy,
                            })
                        })
                        .collect::<Vec<_>>(),
                    "active": self.active,
                    "tab_titles": self.tabs.iter().map(Tab::title).collect::<Vec<_>>(),
                    "workspaces": self.ws_count(),
                    "ws": self.ws_idx,
                    "ws_names": self.wscards().into_iter().map(|c| c.name).collect::<Vec<_>>(),
                    "errors": self.cfg_errors,
                    "xid": self.xid(),
                    "config": self.opts.cfg_path.as_ref().map_or_else(String::new, |p| p.display().to_string()),
                    "dpi": self.opts.dpi,
                    "padding": self.opts.cfg.padding,
                    "advance": cw,
                    "pin": self.tabs[self.active].view_off,
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
                    self.pty_write(data.as_bytes());
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
                    other => return debugsock::err(format!("op de tab: {other:?}")),
                }
                debugsock::ok(serde_json::json!({
                    "tabs": self.tabs.len(),
                    "active": self.active,
                    "titles": self.tabs.iter().map(Tab::title).collect::<Vec<_>>(),
                }))
            },
            "keys" => match req.get("data").and_then(|v| v.as_str()) {
                Some(spec) => {
                    // mesmo caminho do teclado real: atalho local primeiro (que
                    // pode comer a tecla), depois o encoder
                    let Some(k) = key_from_spec(spec) else {
                        return debugsock::err(format!("tecla desconhecida: {spec}"));
                    };
                    if self.press_locais(&k) {
                        return debugsock::ok(serde_json::json!({
                            "keys": [0],
                            "hex": String::new(),
                            "handled": true,
                        }));
                    }
                    let bytes = crate::keys::encode_parts(
                        k.named,
                        k.character,
                        k.text,
                        k.mods,
                        self.tabs[self.active].term.app_cursor,
                        self.tabs[self.active].term.kbd_flags,
                    )
                    .unwrap_or_default();
                    self.pty_write(&bytes);
                    let hex = hex(&bytes);
                    debugsock::ok(serde_json::json!({
                        "keys": [bytes.len()],
                        "hex": hex,
                        "handled": false,
                    }))
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
            // Sonda dos OSC: os últimos payloads crus que o parser viu, da aba
            // ativa e da mais nova para a mais velha. É o que responde "por que
            // a barra de abas mostra lixo" sem recompilar nada.
            "osc" => {
                let tab = req
                    .get("tab")
                    .and_then(serde_json::Value::as_u64)
                    .map_or(self.active, |n| n as usize);
                let Some(t) = self.tabs.get(tab) else {
                    return debugsock::err(format!("aba {tab} não existe"));
                };
                let events: Vec<_> = t
                    .term
                    .osc_seen
                    .iter()
                    .rev()
                    .map(|o| {
                        serde_json::json!({
                            "hex": hex(&o.raw),
                            "text": String::from_utf8_lossy(&o.raw),
                            "truncated": o.truncated,
                            "title": o.title,
                        })
                    })
                    .collect();
                debugsock::ok(serde_json::json!({
                    "tab": tab,
                    "title": self.tabs[tab].term.title,
                    "events": events,
                }))
            },
            "log" => {
                // liga/desliga/troca o caminho em execução; `off` desliga
                let value = req.get("value").and_then(|v| v.as_str()).unwrap_or("off");
                set_log(value);
                self.opts.debug_log = if value.is_empty() || value == "off" {
                    None
                } else {
                    Some(value.to_string())
                };
                debugsock::ok(serde_json::json!({ "path": value }))
            },
            "quit" => {
                self.exiting = true;
                debugsock::ok(serde_json::json!({}))
            },
            _ => debugsock::err(format!("cmd desconhecido: {cmd:?}")),
        }
    }

    /// A primeira aba: o `--cmd` quando dado, senão o shell da config.
    fn spawn_shell(&mut self, cols: usize, rows: usize, wake: Arc<dyn Fn() + Send + Sync>) {
        let cmd = self.opts.cmd.clone();
        self.spawn_tab(self.active, cols, rows, wake, None, cmd.as_deref());
    }

    #[allow(clippy::too_many_arguments)]
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
            self.cell_px(),
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
        self.new_tab_cmd(None);
    }

    /// Aba nova, opcionalmente rodando `cmd` (o editor de config usa isto).
    fn new_tab_cmd(&mut self, cmd: Option<&str>) {
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
        self.spawn_tab(self.active, cols, rows, wake, cwd.as_deref(), cmd);
        self.sync_title();
        self.redraw();
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
            // última aba do workspace: o workspace vai junto (I20), e só o
            // último workspace fecha o app
            self.drop_empty_workspace();
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

    /// Tamanho da célula em px (largura, altura): vai para o `ws_xpixel` do pty
    /// e para o `CSI 14 t`, é de onde os apps tiram a resolução.
    fn cell_px(&self) -> (f32, f32) {
        self.renderer
            .as_ref()
            .map_or((0.0, 0.0), |r| (r.fonts.cw, r.fonts.ch))
    }

    /// Tamanho, posição e maximizado com que a janela nasce (J2–J6).
    ///
    /// Ordem: grade pedida na linha de comando (é medição) desliga o lembrar;
    /// sem ela decide o estado lembrado, se passar pelo `state::fit`; e o
    /// último recurso é o tamanho relativo à grade pedida.
    fn geometry_initial(
        &mut self,
        event_loop: &ActiveEventLoop,
        fonts: &Fonts,
    ) -> (u32, u32, Option<(i32, i32)>, bool) {
        let pad = 2.0 * self.opts.cfg.padding as f32;
        let padrao = (
            ((self.opts.cols as f32 * fonts.cw + pad).ceil() as u32).max(1),
            ((self.opts.rows as f32 * fonts.ch + pad).ceil() as u32).max(1),
        );
        self.geom_normal = padrao;
        self.remember_geom = !self.opts.grid_explicito;
        if !self.remember_geom {
            return (padrao.0, padrao.1, None, false);
        }
        let monitores: Vec<crate::state::Monitor> = event_loop
            .available_monitors()
            .map(|m| {
                let p = m.position();
                let s = m.size();
                (p.x, p.y, s.width, s.height)
            })
            .collect();
        let lembrada = crate::state::load();
        let maximizado = lembrada.is_some_and(|g| g.maximizado);
        // J2/J4: sem estado (ou com estado inválido: monitor desconectado,
        // lixo no arquivo) abre no padrão 980x640 do Python, não do tamanho da
        // grade — quem quer grade exata pede `--cols/--rows`, que desliga o
        // lembrar. Geometria lembrada inválida volta a este mesmo padrão.
        let geom = lembrada
            .and_then(|g| crate::state::fit(g.x, g.y, g.w, g.h, &monitores))
            .unwrap_or_default();
        self.geom_normal = (geom.w.max(1), geom.h.max(1));
        // J6: só o X11 aceita posição de janela; no Wayland ela não existe
        let pos = geom.x.zip(geom.y).filter(|_| backend_x11());
        (geom.w.max(1), geom.h.max(1), pos, maximizado)
    }

    /// Grava posição, tamanho normal e maximizado (J2–J6). Falhar aqui não pode
    /// atrapalhar o fechamento — o `state::save` já engole erro de I/O.
    fn save_geometry(&mut self) {
        if !self.remember_geom {
            return; // grade pedida na linha de comando: não lembra (é medição)
        }
        let maximizado = self.window.as_ref().is_some_and(|w| w.is_maximized());
        let (x, y) = match self.geom_pos.filter(|_| backend_x11()) {
            Some((x, y)) => (Some(x), Some(y)),
            None => (None, None), // Wayland: sem posição para lembrar (J6)
        };
        crate::state::save(&crate::state::Geometry {
            x,
            y,
            w: self.geom_normal.0,
            h: self.geom_normal.1,
            maximizado,
        });
    }

    /// Quantos workspaces existem (o da frente + os estacionados).
    fn ws_count(&self) -> usize {
        self.parked.len() + 1
    }

    /// Workspace novo, herdando o cwd da aba corrente (I5). O novo já nasce
    /// com uma aba, como no Python.
    fn new_workspace(&mut self) {
        let Some(wake) = self.wake.clone() else {
            return;
        };
        let (cols, rows) = (
            self.tabs[self.active].term.cols,
            self.tabs[self.active].term.rows,
        );
        let cwd = self.tabs[self.active].cwd();
        let cur = Workspace::take(self);
        self.parked.push(cur);
        self.ws_idx = self.parked.len(); // total = parked + 1, então é o último
        let mut tab = Tab::new(cols, rows);
        tab.term.scrollback_limit = self.opts.scrollback;
        self.tabs = vec![tab];
        self.active = 0;
        self.ws_name = format!("workspace {}", self.ws_idx + 1);
        self.after_ws_change();
        let (cols, rows) = (
            self.tabs[self.active].term.cols,
            self.tabs[self.active].term.rows,
        );
        self.spawn_tab(self.active, cols, rows, wake, cwd.as_deref(), None);
        self.sync_title();
        self.redraw();
    }

    /// Começa a renomear o workspace `i` no cartão (I8).
    fn start_rename(&mut self, i: usize) {
        if i >= self.ws_count() {
            return;
        }
        let current = if i == self.ws_idx {
            self.ws_name.clone()
        } else {
            let slot = if i < self.ws_idx { i } else { i - 1 };
            self.parked[slot].name.clone()
        };
        self.rename = Some(Rename {
            ws: i,
            text: current,
            replace: true,
        });
        self.redraw();
    }

    /// Aplica (ou descarta) o nome em edição. `keep=false` é o Esc.
    fn finish_rename(&mut self, keep: bool) {
        let Some(Rename { ws: i, text, .. }) = self.rename.take() else {
            return;
        };
        if !keep {
            self.redraw();
            return;
        }
        // o nome vai para o cartão e para o cabeçalho do `dump`: mesma regra do
        // título de aba (controle, zero-width, direção) — o campo é digitação
        // livre, não payload de terminal
        let new = bzmux_core::term::sanitize_title(&text).trim().to_string();
        if new.is_empty() {
            self.redraw();
            return;
        }
        if i == self.ws_idx {
            self.ws_name = new;
        } else {
            let slot = if i < self.ws_idx { i } else { i - 1 };
            self.parked[slot].name = new;
        }
        self.redraw();
    }

    /// Teclado durante o rename: digitar edita, Enter confirma, Esc cancela.
    /// Devolve `true` quando a tecla foi consumida (o filho não a vê).
    fn rename_key(&mut self, k: &KeyIn<'_>) -> bool {
        if self.rename.is_none() {
            return false;
        }
        match k.named {
            Some(NamedKey::Enter) => self.finish_rename(true),
            Some(NamedKey::Escape) => self.finish_rename(false),
            Some(NamedKey::Backspace) => {
                if let Some(r) = self.rename.as_mut() {
                    if r.replace {
                        r.text.clear();
                        r.replace = false;
                    } else {
                        r.text.pop();
                    }
                }
                self.redraw();
            },
            None => {
                let Some(s) = k.character else {
                    return true;
                };
                // tecla que só produz invisível (ou controle) não entra no
                // campo: o nome em edição é desenhado no cartão como está
                let clean = bzmux_core::term::sanitize_title(s);
                if clean.is_empty() {
                    return true;
                }
                if let Some(r) = self.rename.as_mut() {
                    if r.replace {
                        r.text.clear();
                        r.replace = false;
                    }
                    // só o que cabe no cartão; o resto seria invisível
                    if r.text.chars().count() + clean.chars().count() <= 40 {
                        r.text.push_str(&clean);
                    }
                }
                self.redraw();
            },
            _ => {},
        }
        true
    }

    /// Troca o workspace da frente sem matar nada: só troca quem ocupa o
    /// lugar. A ordem lógica é remontada de propósito — trocar `parked[slot]`
    /// no lugar embaralhava os cartões quando o corrente ia para uma posição
    /// mais alta (o buraco dele ficava onde não devia).
    fn select_workspace(&mut self, i: usize) {
        let total = self.ws_count();
        if i >= total || i == self.ws_idx {
            return;
        }
        let old = self.ws_idx;
        let cur = Workspace::take(self);
        // `all[j]` = workspace lógico j; o do meio é o que está na frente
        let mut parked = std::mem::take(&mut self.parked).into_iter();
        let mut all: Vec<Option<Workspace>> = (0..total)
            .map(|j| if j == old { None } else { parked.next() })
            .collect();
        all[old] = Some(cur);
        let new = all[i].take().expect("workspace do índice lógico");
        self.ws_idx = i;
        self.parked = all
            .into_iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, w)| w.expect("só o slot alvo fica vazio"))
            .collect();
        new.into_current(self);
        self.after_ws_change();
    }

    /// Fecha um workspace com todas as abas dele.
    fn close_workspace(&mut self, i: usize) {
        if i >= self.ws_count() {
            return;
        }
        if i == self.ws_idx {
            for t in &mut self.tabs {
                if let Some(p) = t.pty.as_mut() {
                    p.kill();
                }
            }
            self.tabs.clear();
            self.drop_empty_workspace();
            return;
        }
        let slot = if i < self.ws_idx { i } else { i - 1 };
        let mut w = self.parked.remove(slot);
        for t in &mut w.tabs {
            if let Some(p) = t.pty.as_mut() {
                p.kill();
            }
        }
        if i < self.ws_idx {
            self.ws_idx -= 1;
        }
        self.after_ws_change();
    }

    /// O workspace da frente ficou sem abas: promove o primeiro estacionado
    /// (ou fecha o app, se não sobrou nenhum) — I20.
    fn drop_empty_workspace(&mut self) {
        if self.parked.is_empty() {
            // último workspace: o app fecha no próximo `RedrawRequested`
            self.exiting = true;
            self.redraw();
            return;
        }
        let new = self.parked.remove(0);
        new.into_current(self);
        self.ws_idx = 0;
        self.after_ws_change();
    }

    /// Depois de mexer em workspaces: barra pode aparecer/sumir (a altura
    /// depende do número de abas *deste* workspace), então a grade é refeita.
    fn after_ws_change(&mut self) {
        if let Some(w) = self.window.as_ref() {
            let s = w.inner_size();
            self.apply_size(s.width, s.height);
        }
        self.sync_title();
        self.redraw();
    }

    /// Ctrl+Shift+F2: abre o arquivo de configuração no editor, em aba nova
    /// (P33). `VISUAL`/`EDITOR` vencem o `vim`; o diretório é criado porque o
    /// editor só cria o arquivo ao salvar.
    fn open_config(&mut self) {
        let Some(path) = self.opts.cfg_path.clone() else {
            self.toast("config: sem caminho (sem HOME)".to_string());
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vim".to_string());
        let cmd = format!("{} {}", editor, shell_quote(&path.to_string_lossy()));
        self.new_tab_cmd(Some(&cmd));
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
        // `BZMUX_NO_TITLE=1`: sonda de depuração, não configuração. Existe por
        // causa de um tropeço do winit no X11: qualquer erro X pendente na
        // conexão vira `panic` na próxima chamada de propriedade, e o WSI do
        // Mesa pode deixar um lá (ver DEBUG.md). Sem tocar no título, dá para
        // usar o terminal e investigar.
        if std::env::var_os("BZMUX_NO_TITLE").is_some() {
            return;
        }
        if let Some(w) = self.window.as_ref() {
            w.set_title(&format!("{titulo} — bzmux"));
        }
    }
}

/// Uma tecla em pedaços, sem depender do `KeyEvent`: o teclado real e o verbo
/// `keys` do socket passam pelo **mesmo** caminho de atalho e encoder.
struct KeyIn<'a> {
    named: Option<NamedKey>,
    character: Option<&'a str>,
    text: Option<&'a str>,
    /// Dígito da tecla física (0..=9); no socket vem do próprio `spec`.
    digit: Option<usize>,
    mods: ModifiersState,
}

impl<'a> KeyIn<'a> {
    fn from_event(ev: &'a winit::event::KeyEvent) -> KeyIn<'a> {
        let (named, character) = match &ev.logical_key {
            Key::Named(n) => (Some(*n), None),
            Key::Character(s) => (None, Some(s.as_str())),
            _ => (None, None),
        };
        KeyIn {
            named,
            character,
            text: ev.text.as_deref(),
            digit: digit_of(ev),
            mods: ModifiersState::empty(), // quem chama põe o `self.mods`
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
/// `spec` do verbo `keys` -> `KeyIn`: modificadores, nome de tecla e dígito.
fn key_from_spec(spec: &str) -> Option<KeyIn<'_>> {
    let parts: Vec<&str> = spec.split('+').filter(|p| !p.is_empty()).collect();
    let (key, mods_spec) = parts.split_last()?;
    let key: &str = key;
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
    let digit =
        if key.chars().count() == 1 && key.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            key.chars()
                .next()
                .and_then(|c| c.to_digit(10))
                .map(|d| d as usize)
        } else {
            None
        };
    let character = if named.is_none() && key.chars().count() == 1 {
        Some(key)
    } else {
        None
    };
    Some(KeyIn {
        named,
        character,
        text: character,
        digit,
        mods,
    })
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
        // sinais: `SIGUSR1` (dump) e `SIGTERM` (sair limpo), como no Python
        instala_sinais();
        let (w, h, pos, maximizado) = self.geometry_initial(event_loop, &fonts);
        let mut attrs = Window::default_attributes()
            .with_title("bzmux")
            .with_inner_size(PhysicalSize::new(w, h))
            .with_maximized(maximizado);
        if let Some((x, y)) = pos {
            attrs = attrs.with_position(PhysicalPosition::new(x, y));
        }
        // K19: `WM_CLASS` explícito, não herdado do nome do executável — é o
        // que o `StartupWMClass` do `.desktop` procura para agrupar a janela
        // no dock. Binário renomeado (`bzmux-rs`) não pode mudar o WM_CLASS.
        #[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
        let attrs = {
            use winit::platform::x11::WindowAttributesExtX11 as _;
            attrs.with_name("bzmux", "bzmux")
        };
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("bzmux: janela: {e}");
                event_loop.exit();
                return;
            },
        };
        self.window = Some(window.clone());
        // o título sai **antes** de tocar na GPU: criar a swapchain deixa
        // erro X pendente na conexão (o WSI do Mesa cria e destrói pixmaps) e
        // o winit trata qualquer erro X como fatal na próxima chamada de
        // propriedade — o `set_title` seguinte morria com `BadDrawable` que não
        // era dele. Ver DEBUG.md, "erro X pendente".
        self.sync_title();
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
                Err(e) => {
                    // quem abriu a superfície de debug tem de saber que ela não
                    // está de pé (o Python mostra no toast)
                    eprintln!("bzmux: debug socket: {e}");
                    self.toast(format!("debug socket: {e}"));
                },
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
                // alimentar **antes** de perguntar pelo `?2026`: o estado que
                // interessa é o dos bytes que acabaram de chegar, não o da
                // leitura anterior (um desenho por leitura era o G16)
                self.drain();
                let sync = self
                    .tabs
                    .get(self.active)
                    .is_some_and(|t| t.term.sync_output);
                self.gate.request(std::time::Instant::now(), sync);
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
            WindowEvent::Moved(pos) => {
                // posição externa (moldura): é a que `with_position` consome,
                // e a que o `get_position()` do Python salvava — salvar a do
                // cliente descia a janela a barra de título a cada execução
                self.geom_pos = Some((pos.x, pos.y));
            },
            WindowEvent::Resized(size) => {
                // J5: maximizado é estado do WM, não tamanho; se salvasse aqui,
                // o tamanho de antes de maximizar se perderia
                if !self.window.as_ref().is_some_and(|w| w.is_maximized()) {
                    self.geom_normal = (size.width, size.height);
                }
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
                // o release só tem valor no protocolo do kitty (flag 2); o
                // caminho legado não relata soltura, e o `encode` devolve nada
                self.on_key(&event);
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
                self.pty_write(seq.as_bytes());
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
            self.toast(notice_config(&errs));
        }

        let now = std::time::Instant::now();
        // o reload precisa de um tique mesmo sem evento nenhum
        let mut when = now + Duration::from_millis(250);
        // frame coalescido: venceu → um pedido de desenho; senão acorda nele
        if self.gate.venceu(now) {
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
        if let Some(t) = self.gate.when() {
            when = when.min(t);
        }

        // sinais: o handler só levanta a bandeira, quem age é aqui
        let (dump, finish) = sinais();
        if dump {
            self.dump();
        }
        if finish {
            self.dump();
            self.exiting = true;
            event_loop.exit();
            return;
        }
        // `--quit-after`: fecha sozinho (automação) e deixa o dump
        if self.deadline.is_some_and(|p| now >= p) {
            self.deadline = None;
            self.dump();
            self.exiting = true;
            event_loop.exit();
            return;
        }

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
        if self.pisca_active() {
            let e = self.blink_epoch.elapsed().as_millis() as u64;
            let next = (e / BLINK_MS + 1) * BLINK_MS;
            when = when.min(self.blink_epoch + Duration::from_millis(next));
            let phase = (e / BLINK_MS).is_multiple_of(2);
            if phase != self.blink_phase {
                self.blink_phase = phase;
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(when));
        // flash do BEL (I13): acorda quando o primeiro rótulo apaga e
        // redesenha uma vez para tirar o destaque
        let now = std::time::Instant::now();
        let mut apagou = false;
        for t in &mut self.tabs {
            if t.bell_at
                .is_some_and(|b| now.duration_since(b) >= FLASH_DUR)
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
        let next_flash = self
            .tabs
            .iter()
            .filter_map(|t| t.bell_at)
            .map(|b| b + FLASH_DUR)
            .min();
        if let Some(q) = next_flash {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                q.min(std::time::Instant::now() + Duration::from_millis(250)),
            ));
        }

        // aba cujo shell morreu fecha sozinha; a última fecha o workspace, e só
        // o último workspace fecha o app (I20). Aqui estava o `M-W`: o atalho
        // `tabs.len() == 1 → exit` pulava o `drop_empty_workspace`, então um
        // `Ctrl+D`/`exit` na aba visível matava o app inteiro mesmo com outro
        // workspace estacionado (o Python fecha só a aba). A morte do filho tem
        // de entrar no **mesmo** caminho do `close_tab_at`.
        if let Some(morta) = (0..self.tabs.len()).find(|&i| {
            self.tabs[i]
                .pty
                .as_mut()
                .is_some_and(super::pty::Pty::child_exited)
        }) {
            self.drain();
            self.close_tab_at(morta);
            if self.exiting {
                event_loop.exit();
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.save_geometry();
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

/// Linhas de uma aba para o dump: scrollback recente e o viewport vivo.
fn dump_tab(t: &Tab, ativa: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let hist = t.term.scrollback.len();
    if hist > 0 {
        let ini = hist.saturating_sub(40);
        out.push(format!("=== scrollback ({hist}) ==="));
        for row in t.term.scrollback.iter().skip(ini) {
            out.push(row.text().trim_end().to_string());
        }
    }
    // o off vai no cabeçalho: as linhas abaixo são a grade viva, não a visão
    // rolada — sem isso o dump não conta se a visão estava no histórico
    let off = if t.view_off > 0 {
        format!(" (histórico +{})", t.view_off)
    } else {
        String::new()
    };
    let mark = if ativa { "" } else { " [fundo]" };
    out.push(format!(
        "=== viewport {}x{}{off}{mark} ===",
        t.term.cols, t.term.rows
    ));
    for y in 0..t.term.rows {
        out.push(t.term.row_text(y).trim_end().to_string());
    }
    out
}

// ------------------------------------------------------------ sinais

/// `SIGUSR1` pediu um dump; `SIGTERM` pediu sair limpo (dump + fechar).
static SIGNAL_DUMP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SIGNAL_TERM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_usr1(_: libc::c_int) {
    SIGNAL_DUMP.store(true, std::sync::atomic::Ordering::Relaxed);
}

extern "C" fn on_term(_: libc::c_int) {
    SIGNAL_TERM.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Instala `SIGUSR1`/`SIGTERM`. O handler só levanta um `AtomicBool`: `store`
/// de `AtomicBool` é livre de lock e seguro em handler, e quem age é o laço de
/// eventos (que acorda a cada 250 ms de qualquer forma).
///
/// `unsafe`: `libc::signal` troca o handler do processo — não há API segura
/// para isso, como no `SO_PEERCRED` do `debugsock`.
#[allow(unsafe_code)]
pub fn instala_sinais() {
    unsafe {
        libc::signal(libc::SIGUSR1, on_usr1 as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
    }
}

/// Leitura dos sinais (uma vez cada): (dump, sair).
fn sinais() -> (bool, bool) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        SIGNAL_DUMP.swap(false, Relaxed),
        SIGNAL_TERM.swap(false, Relaxed),
    )
}
