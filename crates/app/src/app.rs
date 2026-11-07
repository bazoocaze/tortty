//! The app: window, pty, keyboard, drawing and the introspection socket.

use std::io::{Read as _, Write as _};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tortty_core::config::Bell;
use tortty_core::{Anchor, Span, Term};
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

/// Shell argument: `'…'` with `'` becoming `'\''` — enough for the common case.
fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+:%=@,".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Super alone (neither Ctrl nor Alt): the workspace alias.
fn super_only(mods: ModifiersState) -> bool {
    mods.super_key() && !mods.control_key() && !mods.alt_key()
}

/// Alt alone (neither Ctrl nor Shift). `Alt+Shift+1` belongs to the shell, not the bar.
fn alt_digit_only(mods: ModifiersState) -> bool {
    mods.alt_key() && !mods.control_key() && !mods.shift_key() && !mods.super_key()
}

/// Digit of the **physical** key (0..=9), ignoring layout and Shift.
///
/// `Ctrl+Shift+1` arrives as `!` and on ABNT2 each digit shifts to something
/// else (`2` → `@`, `6` → `¨`); translating the physical code gives the digit
/// in any layout (I16).
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

/// PID of the deepest descendant of `pid` (the program in the foreground: the
/// shell, or the `vim`/`mc` it launched). `/proc/<pid>/task/<pid>/children`
/// lists the direct children; walking down to the end is what yields the
/// process the person is actually using.
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
    /// Wakes the loop (new data on the pty channel).
    Wake,
    /// Request from the debug socket: `(JSON line, reply channel)`.
    Debug(String, Sender<String>),
}

/// Transient notice from the terminal itself (I12): "config: 3 errors — ...".
/// What a tab's `pump` asks the app to do: toast (foreground tab only),
/// window urgency and audible beeps. Executed by the caller so the tab logic
/// stays borrow-free.
#[derive(Default)]
struct PumpOut {
    toast: Option<String>,
    urgency: bool,
    beeps: u32,
}

struct Toast {
    text: String,
    since: std::time::Instant,
    /// How many equal messages arrived in a row (the toast adds up, it does not
    /// stack).
    times: u32,
}

/// How long the toast stays on screen. Every toast gets the same time and no
/// key takes it out: a notice that vanishes before it can be read is a notice
/// that was never delivered.
const TOAST_DUR: Duration = Duration::from_secs(8);

/// Text of the configuration notice (K13). Deliberately fits in ~60 columns:
/// the toast is cut at the width of the grid, and "line and key" is what one
/// needs to see, not the count spelled out.
fn notice_config(errs: &[tortty_core::config::Error]) -> String {
    let Some(first) = errs.first() else {
        return String::new();
    };
    if errs.len() > 1 {
        format!("config kept — {first} (+{})", errs.len() - 1)
    } else {
        format!("config kept — {first}")
    }
}

/// Blink period (cursor and SGR 5). Half "on", half "off".
const BLINK_MS: u64 = 500;

/// How long the tab label stays highlighted after a BEL (I13).
const FLASH_DUR: Duration = Duration::from_millis(800);
/// A notification stays immune to keys this long (`M-K`): whoever is typing
/// must not lose it. It is the toast's own time, because the toast outlives the
/// key that would dismiss it — what the floor now guards is the badge, which has
/// no other expiry.
const NOTIF_FLOOR: Duration = TOAST_DUR;

/// Bytes in hex (the socket's `keys` verb returns this to check the encoder).
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

/// When this process started: the `+ms` stamps of the debug log are relative to
/// it, and the log says the epoch once so they can be turned into wall clock.
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn log_stamp() -> String {
    let start = START.get_or_init(std::time::Instant::now);
    format!("+{:.1}ms", start.elapsed().as_secs_f64() * 1000.0)
}

/// One `?2026` transaction in the debug log: when it opened, how long it held
/// the screen, and what it cleared/reset while holding it (`SyncTrace`).
///
/// The duration is the app's alone — the core has no clock — and it measures
/// what the window cost *us*: the time between the read that opened it and the
/// one that closed it. A window that opens and closes in a single read is
/// therefore `0.0 ms` with `feeds 1`, which is the honest answer: the parser
/// never held anything.
fn log_sync(tab: &mut Tab, who: &str, view_before: usize, now: std::time::Instant) {
    let view_now = tab.view_off;
    if let Some(t) = tab.term.sync_open() {
        if tab.sync_at.is_none() {
            tab.sync_at = Some(now);
            logln(&format!(
                "{who} sync open {} — feeds {}, {} B, view {view_before}, hist {}{}",
                log_stamp(),
                t.feeds,
                t.bytes,
                tab.term.scrollback.len(),
                tally(t),
            ));
        }
    }
    for t in tab.term.take_sync_closed() {
        let held = tab
            .sync_at
            .take()
            .map_or(0.0, |at| now.duration_since(at).as_secs_f64() * 1000.0);
        logln(&format!(
            "{who} sync closed {} after {held:.1} ms — feeds {}, {} B, view {view_before}→{view_now}, \
             hist {}{}",
            log_stamp(),
            t.feeds,
            t.bytes,
            tab.term.scrollback.len(),
            tally(&t),
        ));
    }
}

/// The counters of one window, in a fixed order so two lines can be diffed.
fn tally(t: &tortty_core::term::SyncTrace) -> String {
    format!(
        " — ed0×{} ed1×{} ed2×{} ed3×{} el×{} home×{} stbm×{} alt×{} ris×{} decstr×{} hist+{}",
        t.ed[0], t.ed[1], t.ed[2], t.ed[3], t.el, t.home, t.stbm, t.alt, t.ris, t.decstr, t.hist
    )
}

/// One tab: a terminal and the pty feeding it, plus the state that only makes
/// sense while it is in front (view, selection, mouse drag).
///
/// The split matters because of F4: with N live tabs at once, each pty keeps
/// reading; only what is "looking" (scrolling, selection) stays frozen in the
/// tab that is not active.
pub struct Tab {
    /// Tab identity, monotonic and unique per run. The per-line damage
    /// (A17/G17) stores per-line instances of the `Renderer`, so switching tabs
    /// with the same geometry must not reuse any line — and an index will not
    /// do: closing and opening a tab brings the index back, the identity not.
    pub id: u64,
    pub term: Term,
    pub pty: Option<Pty>,
    pub rx: Option<Receiver<Vec<u8>>>,
    /// Lines scrolled into the history (0 = following the output).
    pub view_off: usize,
    /// Text selection, in absolute anchors.
    pub sel: Option<Span>,
    /// Anchor and pixel of the press: the drag only starts after 3 px.
    pub press: Option<(Anchor, (f64, f64))>,
    pub dragging: bool,
    /// Last press on the grid and how many clicks it made: 2 selects the word,
    /// 3 the line, 4 starts over.
    pub click: Option<(Anchor, u8, std::time::Instant)>,
    /// Accumulated row fraction from the wheel (a trackpad sends fractions).
    pub scroll_frac: f32,
    /// Travel still owed to a mouse-reporting child, in detents, per axis
    /// (vertical, horizontal): the fraction that did not complete a notch yet
    /// (`B40`). Separate from `scroll_frac` because that one is worth 3 lines a
    /// notch and belongs to our own history.
    pub child_frac: (f32, f32),
    /// When the `?2026` window currently open was first seen. This is the only
    /// clock the transaction gets: the core counts, the app times.
    sync_at: Option<std::time::Instant>,
    /// BELLs already handled (the `Term` counts them; the app decides what to play).
    pub bells_seen: u32,
    /// `OSC 9`/`777` already delivered (`M-K`): the toast fired and urgency
    /// went out at drain time. Delivery is not acknowledgement — the badge
    /// lives on `notifs_acked`.
    pub notifs_seen: u32,
    /// `OSC 9`/`777` acknowledged (`M-K`): pending while
    /// `notifs_acked != term.notifs`. Unlike the 800 ms bell flash, a badge
    /// stays until the tab is visited or a key lands after the floor.
    pub notifs_acked: u32,
    /// When the last notification arrived: keys before `NOTIF_FLOOR` must not
    /// eat it (whoever types must not lose it).
    pub notif_at: Option<std::time::Instant>,
    /// Title requested by OSC 0/2 (empty = derive from the cwd).
    pub osc_title: String,
    /// Last BEL, for the label flash (I13).
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
            click: None,
            scroll_frac: 0.0,
            child_frac: (0.0, 0.0),
            sync_at: None,
            bells_seen: 0,
            notifs_seen: 0,
            notifs_acked: 0,
            osc_title: String::new(),
            bell_at: None,
            notif_at: None,
        }
    }

    /// A notification is pending on this tab (`M-K`): delivered but neither
    /// visited nor acknowledged by key.
    fn notif_pending(&self) -> bool {
        self.notifs_acked != self.term.notifs
    }

    /// Marks every notification of this tab acknowledged (`M-K`).
    fn ack_tab(&mut self) {
        self.notifs_seen = self.term.notifs;
        self.notifs_acked = self.term.notifs;
        self.notif_at = None;
    }

    /// Advances the seen counters and reports what the app owes the user.
    /// One logic for the front tab and the parked ones: a background tab's
    /// bell behaves the same wherever the tab lives; only the toast stays
    /// with the foreground tab.
    fn pump(&mut self, bell: Bell, foreground: bool, out: &mut PumpOut) {
        let now = std::time::Instant::now();
        let bells = self.term.bells;
        while self.bells_seen != bells {
            self.bells_seen = self.bells_seen.wrapping_add(1);
            // I13: the tab label flashes for 800 ms, even if it is not in
            // front (it is the "something happened here" alert)
            self.bell_at = Some(now);
            match bell {
                Bell::None => {},
                Bell::Visual => out.urgency = true,
                Bell::Audible => out.beeps += 1,
            }
        }
        let notifs = self.term.notifs;
        while self.notifs_seen != notifs {
            self.notifs_seen = self.notifs_seen.wrapping_add(1);
            self.notif_at = Some(now);
            // a notification is an explicit "look here": urgency is not
            // governed by `bell`, which rules only BEL
            out.urgency = true;
            if foreground {
                out.toast = Some(self.term.notif_text.clone());
            }
        }
    }

    /// Tab name: OSC 0/2 wins, else the shell cwd, else the default.
    fn title(&self) -> String {
        if !self.osc_title.is_empty() {
            return self.osc_title.clone();
        }
        match self.cwd_name() {
            Some(cwd) => cwd,
            None => "tortty".to_string(),
        }
    }

    /// `basename` of the child process cwd (`/proc/<pid>/cwd`); nothing without it.
    fn cwd_name(&self) -> Option<String> {
        let cwd = self.cwd()?;
        let name = tortty_core::term::sanitize_title(&cwd.file_name()?.to_string_lossy());
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    /// cwd of what is running **in front** of the pty: the shell may have
    /// launched `vim`/`mc`, and their cwd is what the next tab should inherit
    /// (the last descendant of the shell is the one in front).
    fn cwd(&self) -> Option<std::path::PathBuf> {
        let from_proc = self
            .pty
            .as_ref()
            .and_then(Pty::child_pid)
            .and_then(|pid| std::fs::read_link(format!("/proc/{}/cwd", descendant(pid))).ok());
        prefer_cwd(from_proc, self.term.cwd.as_deref())
    }
}

/// Name being edited on the card (I8).
struct Rename {
    /// Logical index of the workspace.
    ws: usize,
    text: String,
    /// The old name is whole in the field and untouched so far: the first key
    /// replaces it, as if the text were selected.
    replace: bool,
}

/// A set of tabs with a name of its own (I5).
///
/// The workspace in front lives in [`App::tabs`]/[`App::active`]; the others
/// stay whole here (live pty, grid of their own), and switching workspace is a
/// `mem::swap` of the contents — no copying a term and no killing a process.
struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    name: String,
}

impl Workspace {
    /// Wraps the current session to park it.
    fn take(app: &mut App) -> Workspace {
        Workspace {
            tabs: std::mem::take(&mut app.tabs),
            active: app.active,
            name: std::mem::take(&mut app.ws_name),
        }
    }

    /// Hands the contents back into the slot of the current session.
    fn into_current(self, app: &mut App) {
        app.tabs = self.tabs;
        app.active = self.active.min(app.tabs.len().saturating_sub(1));
        app.ws_name = self.name;
    }
}

#[allow(clippy::struct_excessive_bools)] // app state is boolean by nature
pub struct App {
    opts: Options,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    /// Live tabs (each with its pty running), and which one is in front.
    tabs: Vec<Tab>,
    active: usize,
    /// Name of the workspace in front (`workspace 1`, renamable — I8).
    ws_name: String,
    /// The workspaces that are not in front. The one in front does not live
    /// here: it **is** `tabs`/`active`. Invariant: `parked.len() + 1 ==
    /// ws_count()`, with the logical indices in order (the one in front falls
    /// in the middle).
    parked: Vec<Workspace>,
    /// Logical index of the workspace in front, in `0..ws_count()`.
    ws_idx: usize,
    /// Tab pressed on the bar, to drag and reorder (I10): index and the x of
    /// the press, so the drag only starts after actually moving.
    bar_press: Option<(usize, f64)>,
    /// Dragging the scrollbar (`M-M`): the press started on the strip, so the
    /// motion belongs to the view and not to the selection.
    scroll_drag: bool,
    /// Last key eaten by a local shortcut: its release does not go to the
    /// child (the kitty protocol reports releases, and an eaten shortcut must
    /// not become `CSI ...:3u`).
    consumed: Option<winit::keyboard::PhysicalKey>,
    /// Renaming a workspace on the card (I8). While it is `Some`, the keyboard
    /// belongs to the name — the child does not see the keys.
    rename: Option<Rename>,
    /// Instant of the last card click, for the double click (I8).
    card_click: Option<(usize, std::time::Instant)>,
    /// Last wheel turn handled on the bar (instant and direction). X11 sends
    /// **two** events per notch (press and release), and without this one notch
    /// switches tab and the second undoes it.
    bar_wheel: Option<(std::time::Instant, i8)>,
    /// Travel still owed to the bar, in detents: the fraction of a notch a
    /// touchpad sent that has not become a tab change yet (2026-11-07).
    bar_frac: f32,
    /// Last detent handed to a mouse-reporting child (instant and direction), for
    /// the same reason as `bar_wheel`: the press and the release of one notch
    /// arrive as two `MouseWheel` events, and both would be reported (`B40`).
    wheel_pair: Option<(std::time::Instant, i8)>,
    mods: ModifiersState,
    /// Last mouse position in physical pixels (the base of `cell_at`).
    cursor_px: (f64, f64),
    /// Button currently held, to report release/motion with it.
    held: Option<u8>,
    clip: Clip,
    /// Wakes the loop when pty data arrives (the child is born after the
    /// window, when the grid is already the final one).
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    spawned: bool,
    /// What came from the command line: the reload rebuilds the Options with it
    /// so the CLI keeps beating the file.
    over: crate::cli::Overrides,
    /// `mtime` of the last read of the config file (auto-reload).
    cfg_mtime: Option<std::time::SystemTime>,
    cfg_checked: std::time::Instant,
    /// Instant the blink was reset (last key/paste). The phase comes out of it
    /// by arithmetic: no accumulated state, no `sleep`.
    blink_epoch: std::time::Instant,
    /// Current blink phase, so we only redraw when it flips.
    blink_phase: bool,
    /// There is a cell with SGR 5 on screen. Recomputed on every `feed`: asking
    /// the grid that on every tick would cost a scan per event.
    blink_text: bool,
    /// Draw gate (G16/B28/G15): ~16 ms pacing and `?2026` held back.
    gate: crate::frame::Gate,
    /// Notice on screen (config error etc.). One at a time, it goes away alone.
    toast: Option<Toast>,
    /// File errors at startup, which can only become a toast once the window
    /// exists (in the terminal `stderr` is gone; in a graphical session, same).
    startup_errs: Vec<tortty_core::config::Error>,
    /// Config errors in force right now (the `state` `errors` field): a refused
    /// file keeps the previous config, but the error is a fact.
    cfg_errors: Vec<String>,
    debug: Option<DebugServer>,
    /// Grid requested by the `resize` verb; the WM may round to pixels, so
    /// `apply_size` corrects in steps until it matches (like a real terminal).
    want: Option<(usize, usize)>,
    adjust_left: u8,
    eof: bool,
    /// Last title put on the window (avoids a `set_title` per frame).
    last_title: String,
    exiting: bool,
    /// `--quit-after`: instant it closes by itself (automation).
    deadline: Option<std::time::Instant>,
    /// A dump was already written in this run (SIGUSR1 may come several times).
    dumped: bool,
    /// Normal window size (J5: maximized does **not** overwrite this).
    geom_normal: (u32, u32),
    /// Last outer position seen (the frame, not the client area).
    geom_pos: Option<(i32, i32)>,
    /// The remembered state only counts if nobody asked for a grid on the
    /// command line.
    remember_geom: bool,
}

/// Is the WM here X11? It is the one that accepts a window position; on
/// Wayland that does not exist (J6). `winit` does not expose which backend it
/// picked, so the reading comes from the environment — the same choice it
/// makes: Wayland when `WAYLAND_DISPLAY` exists.
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
        // the startup `mtime` already counts: without it the first tick
        // reparses the same file and a config error would show twice on screen
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
            scroll_drag: false,
            consumed: None,
            rename: None,
            card_click: None,
            bar_wheel: None,
            bar_frac: 0.0,
            wheel_pair: None,
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
            dumped: false,
            geom_normal: crate::state::DEFAULT,
            geom_pos: None,
            remember_geom: true,
        }
    }

    /// Text of every tab of every workspace: scrollback (the last 40) and the
    /// viewport of each session, with a header when there is more than one.
    fn dump_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut multi_ws = self.ws_count() > 1;
        multi_ws = multi_ws || self.parked.len() > 1;
        // the logical order: the parked ones in their positions and the current
        // one in its own slot (`parked` keeps the others whole, in order)
        let mut i_ws = 0usize;
        let mut empurra = |tabs: &[Tab], active: usize, name: &str, parts: &mut Vec<String>| {
            if multi_ws {
                parts.push(format!("=== workspace {}: {name} ===", i_ws + 1));
            }
            let multi_tab = tabs.len() > 1;
            for (i, t) in tabs.iter().enumerate() {
                if multi_tab {
                    parts.push(format!("=== tab {}: {} ===", i + 1, t.title()));
                }
                parts.extend(dump_tab(t, i == active));
            }
            i_ws += 1;
        };
        // the current one goes in its position: before it come the parked ones
        // with a lower index
        let mut feitos = 0usize;
        for slot in 0..self.parked.len() {
            if feitos == self.ws_idx {
                empurra(&self.tabs, self.active, &self.ws_name, &mut parts);
                feitos += 1;
            }
            let w = &self.parked[slot];
            empurra(&w.tabs, w.active, &w.name, &mut parts);
            feitos += 1;
        }
        if feitos == self.ws_idx {
            empurra(&self.tabs, self.active, &self.ws_name, &mut parts);
        }
        let mut s = parts.join("\n");
        s.push('\n');
        s
    }

    /// Writes the dump where `--dump-to` said (else /tmp/tortty-dump.txt).
    fn dump(&mut self) {
        let path = self
            .opts
            .dump_to
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/tortty-dump.txt"));
        match std::fs::write(&path, self.dump_text()) {
            Ok(()) => {
                self.dumped = true;
                logln(&format!("dump: {}", path.display()));
            },
            Err(e) => eprintln!("dump failed: {}: {e}", path.display()),
        }
    }

    /// Window XID on X11 (0 on Wayland): whoever is going to capture pixels asks
    /// the app, not X, so the capture cannot pick another window.
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

    /// pty write with counters (`prof`): bytes and repeats per `EAGAIN`.
    /// Loads an image file for the kitty protocol (`B1`): `t=f` reads, `t=t`
    /// reads and deletes, `t=s` reads shared memory like a file. `~` expands
    /// to the home directory — the only expansion, done here so the core
    /// never resolves paths. Reads at most one byte over the cap: anything
    /// bigger answers `EINVAL:too big` from the core instead of growing here.
    fn read_image_file(path: &str, delete: bool) -> Option<Vec<u8>> {
        let path = match path.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                std::path::PathBuf::from(std::env::var_os("HOME")?)
                    .join(rest.trim_start_matches('/'))
            },
            _ => std::path::PathBuf::from(path),
        };
        let file = std::fs::File::open(&path).ok()?;
        if file.metadata().ok()?.is_dir() {
            return None;
        }
        let mut data = Vec::new();
        std::io::Read::take(file, u64::from(tortty_core::graphics::MAX_APC as u32) + 1)
            .read_to_end(&mut data)
            .ok()?;
        if delete {
            let _ = std::fs::remove_file(&path);
        }
        Some(data)
    }

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

    /// Tab bar height in px (0 with a single tab). The bar only shows up when
    /// there is a choice to make; with one tab it wastes space.
    fn bar_h(&self, ch: f32) -> f32 {
        if self.tabs.len() > 1 {
            ch
        } else {
            0.0
        }
    }

    /// The sidebar cards (I6), in logical order: the parked ones before the one
    /// in front and the ones after, in the right position.
    fn wscards(&self) -> Vec<crate::render::WsCard> {
        let total = self.ws_count();
        let mut cards = Vec::with_capacity(total);
        for i in 0..total {
            let editing = self
                .rename
                .as_ref()
                .filter(|r| r.ws == i)
                .map(|r| format!("{}|", r.text));
            if i == self.ws_idx {
                cards.push(crate::render::WsCard {
                    name: editing.unwrap_or_else(|| self.ws_name.clone()),
                    title: self.tabs[self.active].title(),
                    n_tabs: self.tabs.len(),
                    ativa: true,
                    alert: self.tabs.iter().any(Tab::notif_pending),
                });
            } else {
                let slot = if i < self.ws_idx { i } else { i - 1 };
                let w = &self.parked[slot];
                cards.push(crate::render::WsCard {
                    name: editing.unwrap_or_else(|| w.name.clone()),
                    title: w.tabs[w.active].title(),
                    n_tabs: w.tabs.len(),
                    ativa: false,
                    alert: w.tabs.iter().any(Tab::notif_pending),
                });
            }
        }
        cards
    }

    /// Sidebar width in px (0 with a single workspace — I7).
    fn side_w(&self, cw: f32) -> f32 {
        if self.ws_count() > 1 {
            cw * 18.0
        } else {
            0.0
        }
    }

    /// What the bar shows: number, title and BEL alert of each tab.
    fn tabinfo(&self) -> Vec<crate::render::TabInfo> {
        let now = std::time::Instant::now();
        self.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| crate::render::TabInfo {
                title: t.title(),
                ativa: i == self.active,
                alert: t
                    .bell_at
                    .is_some_and(|when| now.duration_since(when) < FLASH_DUR)
                    || t.notif_pending(),
            })
            .collect()
    }

    fn grid_from_px(&self, w: u32, h: u32) -> (usize, usize) {
        match &self.renderer {
            Some(r) => {
                // padding, tab bar and sidebar are not grid
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

    /// Cell size and padding, with the startup values when there is no
    /// renderer yet (an event can arrive before the first frame).
    fn cell_metrics(&self) -> (f32, f32, f32) {
        self.renderer
            .as_ref()
            .map_or((10.0, 19.0, 0.0), |r| (r.fonts.cw, r.fonts.ch, r.pad))
    }

    /// Pixel -> (col, row) 0-based, clamped to the grid (no padding: G9 still
    /// missing).
    fn cell_at(&self, px: f64, py: f64) -> (usize, usize) {
        let (cw, ch, pad) = self.cell_metrics();
        // subtract the origin: with padding, tab bar and sidebar, the grid's
        // (0,0) is not the window's (0,0)
        let bar = f64::from(self.bar_h(ch));
        let side = f64::from(self.side_w(cw));
        let col = ((px - f64::from(pad) - side) / f64::from(cw)).max(0.0) as usize;
        let row = ((py - f64::from(pad) - bar) / f64::from(ch)).max(0.0) as usize;
        (
            col.min(self.tabs[self.active].term.cols.saturating_sub(1)),
            row.min(self.tabs[self.active].term.rows.saturating_sub(1)),
        )
    }

    /// Pixel offset from cell (0,0) — the coordinate a child that asked for
    /// `?1016` expects. Same origin rule as `cell_at`, without the division.
    fn grid_px(&self, px: f64, py: f64) -> (usize, usize) {
        let (cw, ch, pad) = self.cell_metrics();
        let bar = f64::from(self.bar_h(ch));
        let side = f64::from(self.side_w(cw));
        let x = (px - f64::from(pad) - side).max(0.0);
        let y = (py - f64::from(pad) - bar).max(0.0);
        (x as usize, y as usize)
    }

    /// Index of the tab under the cursor, if it is in the bar strip.
    fn tab_at(&self, px: f64, py: f64) -> Option<usize> {
        let (cw, ch, pad) = self.cell_metrics();
        let bar = f64::from(self.bar_h(ch));
        if bar <= 0.0 || py < f64::from(pad) || py >= f64::from(pad) + bar {
            return None;
        }
        let info = self.tabinfo();
        // the bar starts where the grid starts (right of the sidebar): line 1
        // is split between the workspace card and the tab bar
        let mut x = f64::from(pad) + f64::from(self.side_w(cw));
        for (i, t) in info.iter().enumerate() {
            let label = format!(" {} {} ", i + 1, t.title);
            let width = label.chars().count() as f64 * f64::from(cw);
            if px >= x && px < x + width {
                return Some(i);
            }
            x += width;
        }
        None
    }

    /// Workspace under the cursor, if it is on the sidebar (I9).
    fn card_at(&self, px: f64, py: f64) -> Option<usize> {
        let (cw, ch, pad) = self.cell_metrics();
        let side = f64::from(self.side_w(cw));
        if side <= 0.0 || px >= f64::from(pad) + side || px < f64::from(pad) {
            return None;
        }
        // the sidebar starts on line 1, beside the tab bar (not below)
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

    /// `OSC 8` URI of the cell under the pointer, in the visible grid.
    fn link_at(&self, row: usize, col: usize) -> Option<String> {
        let tab = &self.tabs[self.active];
        let lines = tab.term.visible_rows(tab.view_off);
        let id = lines.get(row)?.cells.get(col)?.link;
        tab.term.link_uri(id).map(str::to_string)
    }

    /// Hands the URI to the desktop (`xdg-open`): we do not speak portal or
    /// D-Bus, and a browser that is already running does not need another.
    fn open_link(&mut self, uri: &str) {
        // only the schemes a terminal link plausibly carries: the URI comes from
        // the child, and `xdg-open` would happily execute a `.desktop` handler
        // for anything else
        const OK: [&str; 5] = ["http://", "https://", "file://", "ftp://", "mailto:"];
        if !OK.iter().any(|p| uri.starts_with(p)) {
            self.toast(format!("link refused: {uri}"));
            return;
        }
        match std::process::Command::new("xdg-open")
            .arg(uri)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => self.toast(format!("opening {uri}")),
            Err(e) => self.toast(format!("xdg-open failed: {e}")),
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

    /// Sends a mouse event to the child. Only called when the app asked for it
    /// (`?1000/1002/1003`); the rest of the mouse is ours (selection).
    fn report_mouse(&mut self, button: u8, pressed: bool, motion: bool) {
        let (pixels, sgr, utf8) = {
            let t = &self.tabs[self.active].term;
            (t.mouse_px, t.mouse_sgr, t.mouse_utf8)
        };
        let (px, py) = self.cursor_px;
        // `?1016` wants pixels from the same origin the cells are measured from
        let (col, row) = if pixels {
            self.grid_px(px, py)
        } else {
            self.cell_at(px, py)
        };
        let bytes = crate::mouse::encode(
            crate::mouse::Ev {
                button,
                col,
                row,
                pressed,
                motion,
                mods: self.mouse_mods(),
            },
            crate::mouse::Proto { sgr, utf8, pixels },
        );
        self.pty_write(&bytes);
    }

    /// Ctrl+Shift and Super shortcuts for workspace/editor: pulled out of
    /// `on_key`, which only sends whatever is left to the encoder.
    #[allow(clippy::too_many_lines)] // one arm per shortcut
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
            // Ctrl+Shift+F2: the config file in the editor, in a new tab. It
            // comes **before** encoding: vim never sees the key, otherwise it
            // interprets it as a shortcut of its own.
            if named == Some(NamedKey::F2) {
                self.open_config();
                return true;
            }
            // Ctrl+Shift+1..9 picks the workspace (0 = the tenth)
            if let Some(n) = k.digit {
                let i = if n == 0 { 9 } else { n - 1 };
                self.select_workspace(i);
                return true;
            }
        }
        // Super+N/Q and Super+1..9: workspace alias (the WM may swallow them,
        // but when they arrive they act)
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

    /// Key: window shortcuts first (the child never sees Ctrl+Shift+C/V), then
    /// the encoder. Any real key returns to live.
    fn on_key(&mut self, event: &winit::event::KeyEvent) {
        let press = event.state == ElementState::Pressed;
        if press {
            let mut k = KeyIn::from_event(event);
            k.mods = self.mods;
            if self.press_local_keys(&k) {
                // the key belonged to the app: so does its release (otherwise
                // the release of Ctrl+Shift+1 became `CSI 49;1:3u` in the child)
                self.consumed = Some(event.physical_key);
                return;
            }
            self.consumed = None;
        } else if self.consumed == Some(event.physical_key) {
            // X11 releases Shift/Ctrl **before** the key: the marker must not be
            // consumed by them (that was the `CSI 49;1:3u` leak)
            self.consumed = None;
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
        self.ack_notifs();
        self.pty_write(&bytes);
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Shortcuts that eat the key before the encoder. `true` = the child sees
    /// neither the press nor the release.
    fn press_local_keys(&mut self, k: &KeyIn<'_>) -> bool {
        self.blink_reset();
        // renaming on the card: the keyboard belongs to the name, not to the
        // terminal (I8)
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
        // Alt+1..9 picks the tab; Alt+0 is the tenth (I15). The digit does not
        // leak to the child.
        if alt_digit_only(k.mods) {
            if let Some(n) = k.digit {
                self.select_tab(if n == 0 { 9 } else { n - 1 });
                return true;
            }
        }
        // Ctrl+Shift+C/V belong to the terminal, not to the app: without this
        // Ctrl+Shift+C becomes the same ^C as Ctrl+C and kills the process
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
        // Shift+PgUp/PgDn/Home/End look at the history without talking to the
        // app
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
        // a real key returns to live; a bare modifier does not (otherwise the
        // Shift of Shift+PgUp already reset the view before scrolling)
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

    /// Motion: only matters to the app when it asked for `?1002`/`?1003`.
    fn on_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_px = (x, y);
        // the scrollbar owns the motion while it is being dragged
        if self.scroll_drag {
            self.scroll_to_y(y);
            return;
        }
        // a drag on the bar reorders tabs (I10); on the grid it is selection
        if self.bar_press.is_some() {
            self.bar_drag();
            return;
        }
        if self.tabs[self.active].term.mouse_mode == 0 {
            self.drag_select(x, y);
            return;
        }
        match self.tabs[self.active].term.mouse_mode {
            // 1002: motion only with a button held
            2 if self.held.is_some() => {
                if let Some(b) = self.held {
                    self.report_mouse(b, true, true);
                }
            },
            // 1003: motion without a button too (code 35 = 3 + 32)
            3 => self.report_mouse(crate::mouse::RELEASE, true, true),
            _ => {},
        }
    }

    /// Button press/release. With `?1000/1002/1003` the event goes to the
    /// child; without them the mouse is ours (selection/copy).
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
                // a click on a workspace card does not go to the child either
                if let Some(i) = self.card_at(self.cursor_px.0, self.cursor_px.1) {
                    self.select_workspace(i);
                    self.held = None;
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                    return;
                }
                // a click on the bar does not go to the child (it switches tab,
                // as in the selection)
                if let Some(i) = self.tab_at(self.cursor_px.0, self.cursor_px.1) {
                    self.select_tab(i);
                    self.held = None;
                    if let Some(w) = self.window.as_ref() {
                        w.request_redraw();
                    }
                    return;
                }
                // F19: the app draws live; a click on the history only makes
                // sense after going back to it
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

    /// Wheel: the child's when it asked for mouse and we are live; otherwise it
    /// is history rolling (the wheel reading history is still ours).
    fn on_wheel(&mut self, delta: MouseScrollDelta) {
        // the wheel on the tab bar cycles through them (I11); on the grid it
        // scrolls
        if self.tab_at(self.cursor_px.0, self.cursor_px.1).is_some() {
            let ch = self.renderer.as_ref().map_or(19.0, |r| r.fonts.ch);
            // the units are the raw ones, as in the child's branch below: a
            // detent is one notch, a pixel delta is cell heights of finger. One
            // event per notch — with `signum` even a fraction of a detent — made
            // a whisper of the touchpad a storm of tabs (2026-11-07).
            let lines = match delta {
                MouseScrollDelta::LineDelta(_, y) => y,
                MouseScrollDelta::PixelDelta(p) => p.y as f32 / ch,
            };
            let n = Self::wheel_notches(&mut self.bar_frac, &mut self.bar_wheel, lines);
            if n == 0 {
                return;
            }
            // up (positive) goes to the previous tab, one notch at a time
            let len = self.tabs.len() as i64;
            let at = (self.active as i64 - i64::from(n)).rem_euclid(len);
            self.select_tab(at as usize);
            return;
        }
        if self.tabs[self.active].term.mouse_mode != 0 && self.tabs[self.active].view_off == 0 {
            // The child decides how far one notch goes; what it needs is the
            // *count*, and one event is not one notch: a touchpad delivers
            // fractions of a detent at up to ~37 events/s (measured on
            // 2026-11-04). Reporting one notch per event made `opencode 2` fly
            // through dozens of screens on a whisper of the finger (B40).
            //
            // Here the units are the raw ones — detents, or cell heights for a
            // pixel delta — never `wheel_lines`: the 3-line factor is ours, and
            // it belongs to the history below.
            let ch = self.renderer.as_ref().map_or(19.0, |r| r.fonts.ch);
            let (vy, hx) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (y, x),
                MouseScrollDelta::PixelDelta(p) => (p.y as f32 / ch, p.x as f32 / ch),
            };
            for (v, up, down, vertical) in [
                (vy, crate::mouse::WHEEL_UP, crate::mouse::WHEEL_DOWN, true),
                (
                    hx,
                    crate::mouse::WHEEL_RIGHT,
                    crate::mouse::WHEEL_LEFT,
                    false,
                ),
            ] {
                if v == 0.0 {
                    continue;
                }
                let btn = if v > 0.0 { up } else { down };
                for _ in 0..self.child_wheel(v, vertical) {
                    // a wheel has no release: `mouse::encode` drops it, so this
                    // is the whole event (one press per notch)
                    self.report_mouse(btn, true, false);
                }
            }
            return;
        }
        // one notch is worth 3 lines; a trackpad
        // sends a fraction of a line per event and the rest accumulates in
        // `scroll_frac`
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => crate::mouse::wheel_lines(y),
            MouseScrollDelta::PixelDelta(p) => {
                let ch = self.renderer.as_ref().map_or(19.0, |r| r.fonts.ch);
                p.y as f32 / ch
            },
        };
        self.tabs[self.active].scroll_frac += lines;
        let n = self.tabs[self.active].scroll_frac.trunc();
        self.tabs[self.active].scroll_frac -= n;
        if n != 0.0 {
            // wheel up (positive delta) goes into the history
            self.scroll_view(n as i64);
        }
    }

    /// How many wheel notches this event is worth, in the event's own direction.
    ///
    /// A detent — exactly ±1.0, which is what the X11 wheel buttons become — is
    /// one notch, and X11 delivers it twice (press and release, same direction,
    /// ~1 ms apart), so the second half is dropped. A touchpad fraction is
    /// carried in `frac` until it completes a detent of travel: one event is not
    /// one notch (`B40` for the child, and the same rule on the bar).
    fn wheel_notches(
        frac: &mut f32,
        pair: &mut Option<(std::time::Instant, i8)>,
        lines: f32,
    ) -> i32 {
        // the same exact compare that `mouse::wheel_lines` documents: a detent
        // is exactly ±1.0 and a touchpad never lands there
        #[allow(clippy::float_cmp)]
        let detent = lines.abs() == 1.0;
        if !detent {
            return crate::mouse::notches(frac, lines);
        }
        let dir: i8 = if lines > 0.0 { 1 } else { -1 };
        let now = std::time::Instant::now();
        let repeated = pair.is_some_and(|(when, previous)| {
            previous == dir && now.duration_since(when).as_millis() < 10
        });
        *pair = Some((now, dir));
        if repeated {
            0
        } else {
            i32::from(dir)
        }
    }

    /// How many wheel notches this event is worth for a mouse-reporting child,
    /// in the event's own direction.
    ///
    /// A detent is one notch (the press/release pair deduplicated); a touchpad
    /// fraction is carried in `Tab::child_frac` until it completes a detent of
    /// travel (`B40`).
    fn child_wheel(&mut self, lines: f32, vertical: bool) -> i32 {
        let Tab { child_frac, .. } = &mut self.tabs[self.active];
        let frac = if vertical {
            &mut child_frac.0
        } else {
            &mut child_frac.1
        };
        Self::wheel_notches(frac, &mut self.wheel_pair, lines).abs()
    }

    /// Terminal mouse: press sets the anchor, drag becomes selection, release
    /// copies. A single click paints no cell and does not touch PRIMARY.
    fn select_input(&mut self, state: ElementState, button: winit::event::MouseButton) {
        use winit::event::MouseButton::{Left, Middle};
        match (state, button) {
            (ElementState::Pressed, Left) => {
                // a click on the sidebar card switches workspace (I9) and a
                // double click edits the name in place (I8)
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
                // a click on the tab bar switches tab (it does not select text);
                // the "press" is kept for the drag (I10)
                if let Some(i) = self.tab_at(self.cursor_px.0, self.cursor_px.1) {
                    self.bar_press = Some((i, self.cursor_px.0));
                    self.select_tab(i);
                    self.tabs[self.active].dragging = false;
                    self.tabs[self.active].press = None;
                    self.redraw();
                    return;
                }
                self.bar_press = None;
                // clicking the history does NOT go back to live: the drag is
                // exactly to copy from there
                let px = self.cursor_px;
                let (col, row) = self.cell_at(px.0, px.1);
                // a press on the scrollbar drags the view, not the text
                if let Some((x0, x1, y0, y1, _)) = self.scrollbar_rect() {
                    if px.0 >= x0 && px.0 < x1 && px.1 >= y0 && px.1 < y0 + y1 {
                        self.scroll_drag = true;
                        self.scroll_to_y(px.1);
                        return;
                    }
                }
                // Ctrl+click opens the `OSC 8` link under the pointer (and does
                // not select): it is the same modifier that underlines them
                if self.mods.control_key() {
                    if let Some(uri) = self.link_at(row, col) {
                        self.open_link(&uri);
                        return;
                    }
                }
                let t = &mut self.tabs[self.active];
                let at = t.term.anchor_at(t.view_off, row, col);
                // a second click on the same cell selects the word, a third the
                // line and a fourth starts over
                let count = match t.click {
                    Some((prev, n, when)) if prev == at && when.elapsed().as_millis() < 400 => {
                        n % 3 + 1
                    },
                    _ => 1,
                };
                t.click = Some((at, count, std::time::Instant::now()));
                let span = match count {
                    2 => t.term.word_span(at),
                    3 => t.term.line_span(at),
                    _ => None,
                };
                t.sel = span;
                // the drag extends from the start of the word/line
                t.press = Some((span.map_or(at, |s| s.a), px));
                t.dragging = true;
                self.redraw();
            },
            (ElementState::Released, Left) => {
                // releasing on the bar ends the reorder drag (I10)
                self.bar_press = None;
                self.scroll_drag = false;
                self.tabs[self.active].dragging = false;
                self.tabs[self.active].press = None;
                if self.tabs[self.active].sel.is_some_and(|s| s.a != s.b) {
                    // PRIMARY is the X11 convention (the middle button pastes
                    // from there). The CLIPBOARD only enters if
                    // `copy_on_select` says so: for an app that only looks at
                    // the CLIPBOARD it is convenience, but stepping on it on
                    // every drag gets in the way more than it helps.
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
                // middle button on the CARD closes the workspace (I9)
                if let Some(i) = self.card_at(self.cursor_px.0, self.cursor_px.1) {
                    self.close_workspace(i);
                    return;
                }
                // middle button ON THE TAB closes it (I9); on the grid it pastes
                // PRIMARY
                if let Some(i) = self.tab_at(self.cursor_px.0, self.cursor_px.1) {
                    self.bar_press = None;
                    self.close_tab_at(i);
                    return;
                }
                // pasting talks to the app, so the view goes back to live
                self.tabs[self.active].view_off = 0;
                if let Some(txt) = self.clip.primary_text() {
                    self.write_paste(&txt);
                }
            },
            _ => {},
        }
    }

    /// Reorder drag on the bar (I10): moved one tab width with the button held
    /// on the bar, the tab changes place. It only really moves after passing
    /// half of its own label (otherwise it wobbles).
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
        // where the press started and where we are: x pixel of tab i in the drag
        let mut x = f64::from(pad);
        let mut today = None;
        for (k, t) in info.iter().enumerate() {
            let width = format!(" {} {} ", k + 1, t.title).chars().count() as f64 * f64::from(cw);
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
        // the destination and the press point that goes with it: with the press
        // point fixed, every following motion event would spend the same travel
        // again and a hand that barely moved would walk the tab to the end of
        // the bar (2026-11-07)
        let (dest, consumed) = crate::mouse::drag_crossed(i, self.tabs.len(), step, dx);
        if dest == i {
            return;
        }
        self.move_tab(i, dest);
        self.bar_press = Some((dest, x0 + consumed));
        // motion path: a frame is asked for, not drawn (B4, the wheel's trap)
        self.ask_frame();
    }

    /// Reorders (I10): removes from `i` and inserts at `d`; the active one
    /// follows the tab.
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
    }

    /// Drag: the selection only starts after 3 px (shaking the mouse with the
    /// button held must not paint a cell in reverse).
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
            // motion path: a frame is asked for, not drawn (B4, the wheel's trap)
            self.ask_frame();
        }
    }

    fn selection_text(&self) -> Option<String> {
        self.tabs[self.active]
            .sel
            .and_then(|s| self.tabs[self.active].term.span_text(&s))
    }

    /// Ctrl+Shift+C: the selection or, without one, the whole viewport.
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

    /// Ctrl+Shift+V: pastes from the CLIPBOARD.
    fn paste(&mut self) {
        if let Some(txt) = self.clip.text() {
            self.blink_reset();
            self.tabs[self.active].view_off = 0;
            self.write_paste(&txt);
        }
    }

    /// Writes the text to the child honoring bracketed paste (`?2004`).
    fn write_paste(&mut self, txt: &str) {
        let payload = self.tabs[self.active].term.paste_payload(txt);
        self.pty_write(&payload);
        self.redraw();
    }

    /// Geometry of the scrollbar in window pixels: `(x0, x1, top, height,
    /// history)`. The strip is drawn on the grid's right edge, but the target
    /// runs all the way to the window's edge: `cols * cw` is rounded down, so
    /// the last column does not reach the border and a click there would fall
    /// through to the text (`cell_at` clamps) and start a selection.
    /// `None` when there is nothing to scroll — the same condition
    /// `Renderer::scrollbar` draws with.
    fn scrollbar_rect(&self) -> Option<(f64, f64, f64, f64, usize)> {
        let hist = self.tabs[self.active].term.scrollback.len();
        let rows = self.tabs[self.active].term.rows;
        if hist == 0 || rows == 0 {
            return None;
        }
        let (cw, ch, pad) = self.cell_metrics();
        let cols = self.tabs[self.active].term.cols;
        let grid_w = cols as f32 * cw;
        let left = f64::from(pad) + f64::from(self.side_w(cw)) + f64::from(grid_w);
        let top = f64::from(pad) + f64::from(self.bar_h(ch));
        let w = f64::from(self.scrollbar_w());
        let right = self
            .window
            .as_ref()
            .map_or(left, |w| f64::from(w.inner_size().width));
        // the target is the last `w` px of the window: with the grid rounded down
        // that is the strip plus the sliver to its right, and it stays inside the
        // window even when a window manager hands back a narrower one
        let x0 = (left - w).min(right - w).max(0.0);
        Some((x0, right, top, rows as f64 * f64::from(ch), hist))
    }

    /// Width of the scrollbar in px. It sits over the last column, so it is
    /// capped by the cell: a wider strip would hide the column it overlaps.
    fn scrollbar_w(&self) -> f32 {
        let (cw, _, _) = self.cell_metrics();
        f32::min(crate::render::SCROLLBAR_W, cw)
    }

    /// Puts the view where the pointer is on the bar: the thumb sits at the
    /// bottom when the view follows the output and at the top when it is at the
    /// oldest line.
    fn scroll_to_y(&mut self, py: f64) {
        let Some((_, _, top, h, hist)) = self.scrollbar_rect() else {
            return;
        };
        let rows = self.tabs[self.active].term.rows;
        let thumb = (h * rows as f64 / (rows + hist) as f64).max(12.0).min(h);
        let travel = h - thumb;
        if travel <= 0.0 {
            return;
        }
        let y = ((py - top) - thumb / 2.0).clamp(0.0, travel);
        let off = ((1.0 - y / travel) * hist as f64).round() as i64;
        let cur = self.tabs[self.active].view_off as i64;
        self.scroll_view(off - cur);
    }

    fn scroll_view(&mut self, n: i64) {
        let sb = self.tabs[self.active].term.scrollback.len() as i64;
        self.tabs[self.active].view_off =
            (self.tabs[self.active].view_off as i64 + n).clamp(0, sb) as usize;
        // A touchpad delivers up to ~125 wheel events per second, and drawing a
        // frame per event made the app consume slower than X delivers: the
        // queue kept draining after the finger left the pad, which reads as
        // inertia. The frame is requested, not drawn (one draw per frame).
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Reads what the pty of **every** tab produced.
    ///
    /// A background tab has to be drained too: the channel is not unbounded,
    /// and a `yes` in a hidden tab would fill memory. What is reserved to the
    /// active tab is only what is "looking" (blink and view).
    fn drain(&mut self) {
        for i in 0..self.tabs.len() {
            self.drain_tab(i);
        }
        self.drain_parked();
    }

    /// Feeds the parked workspaces (`M-K`): a notification in workspace 3 has
    /// to fire while workspace 1 is in front, and the pty produces whether
    /// anybody looks. Feed only — no title, blink or draw belongs to a parked
    /// tab. The view still follows the output so the switch back lands where
    /// it would have; toasts stay with the foreground tab, badges with whoever
    /// owns them. Mirrors the signal half of `drain_tab` through the same
    /// `pump` (hence no second formula for who owes what).
    fn drain_parked(&mut self) {
        for s in 0..self.parked.len() {
            for j in 0..self.parked[s].tabs.len() {
                let (bytes, eof) = Self::read_pty(self.parked[s].tabs[j].rx.as_ref());
                if eof {
                    self.eof = true;
                }
                if bytes.is_empty() {
                    continue;
                }
                let mark = self.parked[s].tabs[j].term.mark();
                let t0 = std::time::Instant::now();
                let view_before = self.parked[s].tabs[j].view_off;
                self.parked[s].tabs[j].term.feed(&bytes);
                if prof::enabled() {
                    prof::add_parse(t0.elapsed().as_nanos() as u64, bytes.len());
                }
                let who = format!("ws {s} tab {}", self.parked[s].tabs[j].id);
                log_sync(&mut self.parked[s].tabs[j], &who, view_before, t0);
                if self.parked[s].tabs[j].term.theme_dirty {
                    self.parked[s].tabs[j].term.theme_dirty = false;
                    let theme = self.parked[s].tabs[j].term.theme.clone();
                    if let Some(r) = self.renderer.as_mut() {
                        r.apply_theme(&theme);
                    }
                }
                if self.parked[s].tabs[j].view_off > 0 {
                    let off = self.parked[s].tabs[j].view_off;
                    // only real output moves the reader: a `?2026` repaint and an
                    // erased history are handled inside (D10)
                    self.parked[s].tabs[j].view_off =
                        self.parked[s].tabs[j].term.view_after_output(off, mark);
                }
                let mut pumped = PumpOut::default();
                let bell = self.opts.cfg.bell;
                self.parked[s].tabs[j].pump(bell, false, &mut pumped);
                for _ in 0..pumped.beeps {
                    Self::beep();
                }
                if pumped.urgency {
                    self.urgency();
                }
                // no toast: a parked tab must not write over the foreground
                self.parked[s].tabs[j]
                    .term
                    .serve_images(Self::read_image_file);
                let out = self.parked[s].tabs[j].term.take_child_out();
                if !out.is_empty() {
                    prof::add_write(out.len());
                    if let Some(pty) = self.parked[s].tabs[j].pty.as_mut() {
                        if !pty.write(&out) {
                            prof::add_eagain();
                        }
                    }
                }
            }
        }
    }

    /// Non-blocking read of a tab's pty channel. Shared by the front drain
    /// and the parked one (`M-K` feeds tabs nobody looks at).
    fn read_pty(rx: Option<&Receiver<Vec<u8>>>) -> (Vec<u8>, bool) {
        let mut bytes: Vec<u8> = Vec::new();
        let mut eof = false;
        if let Some(rx) = rx {
            loop {
                match rx.try_recv() {
                    Ok(v) if v.is_empty() => {
                        eof = true;
                        break;
                    },
                    Ok(v) => bytes.extend_from_slice(&v),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        eof = true;
                        break;
                    },
                }
            }
        }
        (bytes, eof)
    }

    fn drain_tab(&mut self, i: usize) {
        let (bytes, eof) = Self::read_pty(self.tabs[i].rx.as_ref());
        if eof {
            self.eof = true;
        }
        if !bytes.is_empty() {
            // D10: whoever is reading the history cannot be dragged by the
            // output that arrives — the view moves along with the lines that left.
            let mark = self.tabs[i].term.mark();
            let t0 = std::time::Instant::now();
            let view_before = self.tabs[i].view_off;
            self.tabs[i].term.feed(&bytes);
            if prof::enabled() {
                prof::add_parse(t0.elapsed().as_nanos() as u64, bytes.len());
            }
            let who = format!("tab {}", self.tabs[i].id);
            log_sync(&mut self.tabs[i], &who, view_before, t0);
            // `OSC 4/10/11/12` touched the colors: the renderer has to know. It
            // holds for any tab: the theme belongs to the emulator, not the tab.
            if self.tabs[i].term.theme_dirty {
                self.tabs[i].term.theme_dirty = false;
                let theme = self.tabs[i].term.theme.clone();
                if let Some(r) = self.renderer.as_mut() {
                    r.apply_theme(&theme);
                }
            }
            // I4: title via OSC 0/2 (the shell puts "~/project" and `vim` the
            // file name); `tortty` is the default and does not count as a request
            let title = self.tabs[i].term.title.clone();
            if title != self.tabs[i].osc_title {
                let was = i == self.active;
                self.tabs[i].osc_title = if title == "tortty" {
                    String::new()
                } else {
                    title
                };
                if was {
                    self.sync_title();
                }
            }
            let mut pumped = PumpOut::default();
            let bell = self.opts.cfg.bell;
            self.tabs[i].pump(bell, i == self.active, &mut pumped);
            for _ in 0..pumped.beeps {
                Self::beep();
            }
            if pumped.urgency {
                self.urgency();
            }
            if let Some(text) = pumped.toast {
                self.toast(text);
            }
            self.tabs[i].term.serve_images(Self::read_image_file);
            if i == self.active {
                self.blink_text = self.tabs[i].term.grid.iter().any(|r| {
                    r.cells
                        .iter()
                        .any(|c| c.flags & tortty_core::cell::BLINK != 0)
                });
            }
            if self.tabs[i].view_off > 0 {
                let off = self.tabs[i].view_off;
                // only real output moves the reader: a `?2026` repaint is one
                // frame of what is already on screen, and a `3J` takes the anchor
                // with it instead of throwing the view at the top (B41)
                self.tabs[i].view_off = self.tabs[i].term.view_after_output(off, mark);
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

    /// File errors read at startup: they become a toast on the first tick.
    pub fn startup_errors(&mut self, errs: Vec<tortty_core::config::Error>) {
        self.cfg_errors = errs.iter().map(std::string::ToString::to_string).collect();
        self.startup_errs = errs;
    }

    /// Resets the blink: right after a key the cursor must be lit. Without this
    /// the cursor may "vanish" at the exact instant the person looks at it.
    fn blink_reset(&mut self) {
        self.blink_epoch = std::time::Instant::now();
        if !self.blink_phase {
            self.blink_phase = true;
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
    }

    /// Blink state of this frame (G12).
    ///
    /// The cursor blinks if the config allows it **and** the program did not ask
    /// for a steady cursor via `CSI 6 q` (4/6 = steady). Text with SGR 5 always
    /// blinks — the program asked for it, not the config.
    fn blink(&self) -> crate::render::Blink {
        let phase = self.blink_phase;
        crate::render::Blink {
            phase,
            cursor: !self.cursor_blinks() || phase,
        }
    }

    /// Should the cursor blink? The config rules, but `CSI 6 q` with an even
    /// shape (4/6) asks for a steady cursor and wins for as long as the program
    /// wants it.
    fn cursor_blinks(&self) -> bool {
        self.opts.cfg.cursor_blink
            && self.tabs[self.active]
                .term
                .app_cursor_style
                .is_none_or(|(_, pisca)| pisca)
    }

    /// Is anything blinking on screen (cursor or text with SGR 5)? Only in that
    /// case does the loop need to wake up in the middle of the period.
    fn blink_active(&self) -> bool {
        (self.cursor_blinks() && self.tabs[self.active].term.cursor_visible)
            || self.has_text_piscando()
    }

    /// Is there a cell with SGR 5 on screen? Without this the loop would have no
    /// reason to wake up mid blink period when the cursor is steady.
    fn has_text_piscando(&self) -> bool {
        self.blink_text
    }

    /// Shows a notice on screen. An equal message in a row becomes a count
    /// ("×3"), not a queue — whoever saves the file ten times does not want ten
    /// notices.
    fn toast(&mut self, text: String) {
        let times = match self.toast.take() {
            Some(t) if t.text == text => t.times + 1,
            _ => 1,
        };
        logln(&format!("toast: {text}"));
        self.toast = Some(Toast {
            text,
            since: std::time::Instant::now(),
            times,
        });
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Toast text for the renderer (with the count when there was a repeat).
    fn toast_text(&self) -> Option<String> {
        let t = self.toast.as_ref()?;
        if t.since.elapsed() > TOAST_DUR {
            return None;
        }
        Some(if t.times > 1 {
            format!("{} (×{})", t.text, t.times)
        } else {
            t.text.clone()
        })
    }

    /// One audible BEL to our stdout (a real terminal beeps; in a graphical
    /// session that is what is left without an audio library).
    fn beep() {
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x07");
        let _ = out.flush();
    }

    /// Window urgency outside the bell config: a notification is an explicit
    /// "look here", while `bell` governs only BEL.
    fn urgency(&self) {
        if let Some(w) = self.window.as_ref() {
            w.request_user_attention(Some(UserAttentionType::Informational));
        }
    }

    /// A key delivered to the child acknowledges the notification badge
    /// (`M-K`): whoever types already saw it. Guarded by `NOTIF_FLOOR` — keys
    /// typed while the notification is fresh must not eat it. The toast is not
    /// in this list: it has its own time and leaves by the tick alone. Parked
    /// workspaces keep their card until visited: nobody looked there.
    fn ack_notifs(&mut self) {
        let now = std::time::Instant::now();
        for t in &mut self.tabs {
            if t.notif_pending()
                && t.notif_at
                    .is_some_and(|at| now.duration_since(at) >= NOTIF_FLOOR)
            {
                t.ack_tab();
            }
        }
    }

    /// Config file auto-reload (K12): every 250 ms it checks the `mtime`;
    /// changed, it reparses. **Any** error keeps the configuration already in
    /// force — half a configuration is worse than none.
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
                "config: {} kept; {} error(s), the first at {}",
                path.display(),
                errs.len(),
                errs[0]
            ));
            // K13: the file is edited in an editor, which does not show our log
            // — the notice has to appear on screen, with the first line and the
            // count.
            self.toast(notice_config(&errs));
            return;
        }
        logln(&format!("config: {} reloaded", path.display()));
        // only warn on screen if there was an error showing: the reload worked
        // and whoever saved needs to know it is in force again
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.text.starts_with("config"))
        {
            self.toast("config reloaded".to_string());
        }
        self.apply_options(new, &text);
    }

    /// Applies the new configuration to what is already running.
    fn apply_options(&mut self, new: Options, text: &str) {
        let font_changed = new.font_family != self.opts.font_family
            || (new.font_size_pt - self.opts.font_size_pt).abs() > f32::EPSILON
            || (new.dpi - self.opts.dpi).abs() > f32::EPSILON;
        let log_changed = new.debug_log != self.opts.debug_log;
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
        if font_changed {
            self.opts.font_family = new.font_family;
            self.opts.font_size_pt = new.font_size_pt;
            self.opts.dpi = new.dpi;
            self.rebuild_renderer();
        }
        if log_changed && new.debug_log.is_none() {
            set_log("");
        }
        // the padding changed: the grid is another one, so ask for the size again
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

    /// Font changed: the atlas is another one, so rebuild font + renderer (the
    /// window and the pty stay the same).
    fn rebuild_renderer(&mut self) {
        let (Some(window), Some(old)) = (self.window.clone(), self.renderer.take()) else {
            return;
        };
        let scale = window.scale_factor();
        let fonts = match Fonts::new(&self.opts.font_family, self.font_px(scale)) {
            Ok(f) => f,
            Err(e) => {
                logln(&format!("font: {e}"));
                self.renderer = Some(old);
                return;
            },
        };
        drop(old);
        match Renderer::new(&window, fonts, self.opts.vsync, &self.opts.cfg) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => logln(&format!("GPU while switching the font: {e}")),
        }
        let size = window.inner_size();
        self.apply_size(size.width, size.height);
    }

    /// Asks for a frame instead of drawing one.
    ///
    /// `redraw` presents synchronously, and a handler that calls it pays one
    /// present per event: on a motion path (`drag_select`, `bar_drag`) the
    /// events arrive faster than the present, the X queue grows and the drag
    /// keeps moving after the hand stops — the same trap the wheel had (B4).
    /// `request_redraw` is coalesced by `winit` into one `RedrawRequested` per
    /// event loop iteration, and the frame gate keeps the pacing.
    fn ask_frame(&self) {
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
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
            links: self.mods.control_key(),
        };
        let drew = {
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
        // damage consumed **only** when the frame was actually presented: a
        // skipped frame (occlusion/timeout) must not clear the mark, otherwise
        // the dirty line stays stale on screen forever (A17/G17)
        if drew {
            self.tabs[self.active].term.clear_damage();
        }
    }

    fn apply_size(&mut self, w: u32, h: u32) {
        // the bar enters the count before the grid: with N tabs the grid is one
        // row shorter, and every tab follows (each pty is resized)
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
        // the parser needs the same cell size to answer `CSI 14/16 t` and for
        // the default `c=`/`r=` of images (H9). It was the core's 10x19 default:
        // the image came out 5x3 cells instead of 4x2 in a 15x29 cell.
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
        // the socket target rules: if the WM delivered fewer cells, ask for the
        // difference back (`request_inner_size` is only a suggestion)
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

    #[allow(clippy::too_many_lines)] // one arm per protocol verb
    fn debug_dispatch(&mut self, line: &str) -> String {
        let req: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return debugsock::err("invalid json"),
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
                    "mouse_px": self.tabs[self.active].term.mouse_px,
                    "cwd": self.tabs[self.active].term.cwd,
                    "cursor_px": [self.cursor_px.0, self.cursor_px.1],
                    "cell_at": [at.0, at.1],
                    "sel": sel_json,
                    "cell": [cw, ch],
                    "font": match &self.renderer { Some(r) => r.fonts.family.clone(), None => String::new() },
                    "font_px": match &self.renderer { Some(r) => r.fonts.px, None => 0.0 },
                    "tabs": self.tabs.len(),
                    // damage (A17/G17): how many lines/cells were rebuilt since
                    // the start. It is the proof that partial drawing works —
                    // without depending on looking at the screen.
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
                    // kitty images: without this there is no way to check the
                    // placement without looking at the screen
                    "images": self.tabs[self.active].term.graphics.images().len(),
                    // notifications (M-K): what is still unacknowledged, for
                    // probes that cannot watch the toast
                    "notifs": self.tabs.iter().filter(|t| t.notif_pending()).count()
                        + self
                            .parked
                            .iter()
                            .flat_map(|w| w.tabs.iter())
                            .filter(|t| t.notif_pending())
                            .count(),
                    "notif": if self.tabs[self.active].notif_pending() {
                        self.tabs[self.active].term.notif_text.clone()
                    } else {
                        String::new()
                    },
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
                None => debugsock::err("write needs data (string)"),
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
                    return debugsock::err("resize needs cols and rows > 0");
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
            // tabs via socket: what the keyboard does, testable without XTest
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
                    other => return debugsock::err(format!("tab op: {other:?}")),
                }
                debugsock::ok(serde_json::json!({
                    "tabs": self.tabs.len(),
                    "active": self.active,
                    "titles": self.tabs.iter().map(Tab::title).collect::<Vec<_>>(),
                }))
            },
            "keys" => match req.get("data").and_then(|v| v.as_str()) {
                Some(spec) => {
                    // same path as the real keyboard: local shortcut first (which
                    // may eat the key), then the encoder
                    let Some(k) = key_from_spec(spec) else {
                        return debugsock::err(format!("unknown key: {spec}"));
                    };
                    if self.press_local_keys(&k) {
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
                None => debugsock::err("keys needs data"),
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
            // OSC probe: the last raw payloads the parser saw, from the active
            // tab, newest to oldest. It is what answers "why does the tab bar
            // show garbage" without recompiling anything.
            "osc" => {
                let tab = req
                    .get("tab")
                    .and_then(serde_json::Value::as_u64)
                    .map_or(self.active, |n| n as usize);
                let Some(t) = self.tabs.get(tab) else {
                    return debugsock::err(format!("tab {tab} does not exist"));
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
                // turns on/off/changes the path at runtime; `off` turns it off
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
            _ => debugsock::err(format!("unknown cmd: {cmd:?}")),
        }
    }

    /// The first tab: `--cmd` when given, else the shell from the config.
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
                eprintln!("tortty: {e}");
                self.exiting = true;
            },
        }
    }

    /// New tab (I1): inherits the cwd of the current tab (I2) and becomes the
    /// active one.
    ///
    /// `--cmd` only counts for the first: opening another tab with the debug
    /// command would repeat the test, not open a shell.
    fn new_tab(&mut self) {
        self.new_tab_cmd(None);
    }

    /// New tab, optionally running `cmd` (the config editor uses this).
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
        // the bar may have appeared now: the grid of every tab shrinks one row
        if let Some(w) = self.window.as_ref() {
            let s = w.inner_size();
            self.apply_size(s.width, s.height);
        }
        let (cols, rows) = (
            self.tabs[self.active].term.cols,
            self.tabs[self.active].term.rows,
        );
        // `--cmd` only counts for the first tab: repeating the debug command on
        // the second would open no shell at all
        self.spawn_tab(self.active, cols, rows, wake, cwd.as_deref(), cmd);
        self.sync_title();
        self.redraw();
    }

    /// Closes the active tab (I3).
    fn close_tab(&mut self) {
        self.close_tab_at(self.active);
    }

    /// Closes a tab (I3): kills the pty and picks the next one. Closing the last
    /// one closes the app (I20) — what `Ctrl+Shift+W` in the last terminal does.
    fn close_tab_at(&mut self, i: usize) {
        if let Some(pty) = self.tabs[i].pty.as_mut() {
            pty.kill();
        }
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            // last tab of the workspace: the workspace goes with it (I20), and
            // only the last workspace closes the app
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

    /// Cell size in px (width, height): it goes into the pty `ws_xpixel` and
    /// into `CSI 14 t`, it is where the apps get the resolution from.
    fn cell_px(&self) -> (f32, f32) {
        self.renderer
            .as_ref()
            .map_or((0.0, 0.0), |r| (r.fonts.cw, r.fonts.ch))
    }

    /// Size, position and maximized state the window is born with (J2–J6).
    ///
    /// Order: a grid requested on the command line (it is a measurement) turns
    /// the remembering off; without it the remembered state decides, if it
    /// passes `state::fit`; and the last resort is the size relative to the
    /// requested grid.
    fn geometry_initial(
        &mut self,
        event_loop: &ActiveEventLoop,
        fonts: &Fonts,
    ) -> (u32, u32, Option<(i32, i32)>, bool) {
        let pad = 2.0 * self.opts.cfg.padding as f32;
        let default = (
            ((self.opts.cols as f32 * fonts.cw + pad).ceil() as u32).max(1),
            ((self.opts.rows as f32 * fonts.ch + pad).ceil() as u32).max(1),
        );
        self.geom_normal = default;
        self.remember_geom = !self.opts.grid_explicito;
        if !self.remember_geom {
            return (default.0, default.1, None, false);
        }
        let monitors: Vec<crate::state::Monitor> = event_loop
            .available_monitors()
            .map(|m| {
                let p = m.position();
                let s = m.size();
                (p.x, p.y, s.width, s.height)
            })
            .collect();
        let remembered = crate::state::load();
        let maximized = remembered.is_some_and(|g| g.maximized);
        // J2/J4: with no state (or invalid state: monitor unplugged, garbage in
        // the file) it opens at the 980x640 default, not at the grid size —
        // whoever wants an exact grid asks for `--cols/--rows`, which turns the
        // remembering off. Invalid remembered geometry falls back to this same
        // default.
        let geom = remembered
            .and_then(|g| crate::state::fit(g.x, g.y, g.w, g.h, &monitors))
            .unwrap_or_default();
        self.geom_normal = (geom.w.max(1), geom.h.max(1));
        // J6: only X11 accepts a window position; on Wayland it does not exist
        let pos = geom.x.zip(geom.y).filter(|_| backend_x11());
        (geom.w.max(1), geom.h.max(1), pos, maximized)
    }

    /// Saves position, normal size and maximized state (J2–J6). Failing here
    /// cannot get in the way of closing — `state::save` already swallows I/O
    /// errors.
    fn save_geometry(&mut self) {
        if !self.remember_geom {
            return; // grid requested on the command line: no remembering (a measurement)
        }
        let maximized = self.window.as_ref().is_some_and(|w| w.is_maximized());
        let (x, y) = match self.geom_pos.filter(|_| backend_x11()) {
            Some((x, y)) => (Some(x), Some(y)),
            None => (None, None), // Wayland: no position to remember (J6)
        };
        crate::state::save(&crate::state::Geometry {
            x,
            y,
            w: self.geom_normal.0,
            h: self.geom_normal.1,
            maximized,
        });
    }

    /// How many workspaces exist (the one in front + the parked ones).
    fn ws_count(&self) -> usize {
        self.parked.len() + 1
    }

    /// New workspace, inheriting the cwd of the current tab (I5). The new one is
    /// born with a tab already.
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
        self.ws_idx = self.parked.len(); // total = parked + 1, so it is the last one
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

    /// Starts renaming workspace `i` on the card (I8).
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

    /// Applies (or discards) the name being edited. `keep=false` is Esc.
    fn finish_rename(&mut self, keep: bool) {
        let Some(Rename { ws: i, text, .. }) = self.rename.take() else {
            return;
        };
        if !keep {
            self.redraw();
            return;
        }
        // the name goes to the card and to the `dump` header: same rule as the
        // tab title (control, zero-width, direction) — the field is free typing,
        // not terminal payload
        let new = tortty_core::term::sanitize_title(&text).trim().to_string();
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

    /// Keyboard during the rename: typing edits, Enter confirms, Esc cancels.
    /// Returns `true` when the key was consumed (the child does not see it).
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
                // a key that only produces something invisible (or control)
                // does not enter the field: the name being edited is drawn on the
                // card as it is
                let clean = tortty_core::term::sanitize_title(s);
                if clean.is_empty() {
                    return true;
                }
                if let Some(r) = self.rename.as_mut() {
                    if r.replace {
                        r.text.clear();
                        r.replace = false;
                    }
                    // only what fits on the card; the rest would be invisible
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

    /// Switches the workspace in front without killing anything: only who fills
    /// the slot changes. The logical order is rebuilt on purpose — swapping
    /// `parked[slot]` in place shuffled the cards when the current one went to a
    /// higher position (its hole ended up where it should not).
    fn select_workspace(&mut self, i: usize) {
        let total = self.ws_count();
        if i >= total || i == self.ws_idx {
            return;
        }
        let old = self.ws_idx;
        let cur = Workspace::take(self);
        // `all[j]` = logical workspace j; the one in the middle is the one in front
        let mut parked = std::mem::take(&mut self.parked).into_iter();
        let mut all: Vec<Option<Workspace>> = (0..total)
            .map(|j| if j == old { None } else { parked.next() })
            .collect();
        all[old] = Some(cur);
        let new = all[i].take().expect("workspace for the logical index");
        self.ws_idx = i;
        self.parked = all
            .into_iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, w)| w.expect("only the target slot stays empty"))
            .collect();
        new.into_current(self);
        // entering acknowledges every badge of the workspace (`M-K`)
        for t in &mut self.tabs {
            t.ack_tab();
        }
        self.after_ws_change();
    }

    /// Closes a workspace with all of its tabs.
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

    /// The workspace in front lost its tabs: promotes the first parked one (or
    /// closes the app, if none is left) — I20.
    fn drop_empty_workspace(&mut self) {
        if self.parked.is_empty() {
            // last workspace: the app closes on the next tick (`about_to_wait`
            // exits as soon as `exiting` is set). Drawing here would index
            // `tabs[active]` with no tab and panic on the way out, and nothing
            // is saved when the process dies mid-frame — that is why the window
            // state was lost when the shell ended the session with `exit`/Ctrl+D
            // instead of the WM close button (B38)
            self.exiting = true;
            return;
        }
        let new = self.parked.remove(0);
        new.into_current(self);
        self.ws_idx = 0;
        self.after_ws_change();
    }

    /// After touching workspaces: the bar may appear/disappear (its height
    /// depends on the number of tabs *of this* workspace), so the grid is redone.
    fn after_ws_change(&mut self) {
        if let Some(w) = self.window.as_ref() {
            let s = w.inner_size();
            self.apply_size(s.width, s.height);
        }
        self.sync_title();
        self.redraw();
    }

    /// Ctrl+Shift+F2: opens the config file in the editor, in a new tab (P33).
    /// `VISUAL`/`EDITOR` beat `vim`; the directory is created because the editor
    /// only creates the file on save.
    fn open_config(&mut self) {
        let Some(path) = self.opts.cfg_path.clone() else {
            self.toast("config: no path (no HOME)".to_string());
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

    /// Switches tab (Alt+N). Out of range, does nothing.
    fn select_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.active {
            return;
        }
        self.active = i;
        // visiting acknowledges the badge (`M-K`): it did its job
        self.tabs[i].ack_tab();
        self.sync_title();
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }

    /// Window title = title of the active tab (I17), for the WM and the taskbar.
    fn sync_title(&mut self) {
        let title = self.tabs[self.active].title();
        if title == self.last_title {
            return;
        }
        self.last_title.clone_from(&title);
        // `TORTTY_NO_TITLE=1`: debug probe, not configuration. It exists because
        // of a winit stumble on X11: any pending X error on the connection
        // becomes a `panic` on the next property call, and the Mesa WSI may
        // leave one there (see DEBUG.md). Without touching the title, one can
        // still use the terminal and investigate.
        if std::env::var_os("TORTTY_NO_TITLE").is_some() {
            return;
        }
        if let Some(w) = self.window.as_ref() {
            w.set_title(&format!("{title} — tortty"));
        }
    }
}

/// A key in pieces, without depending on `KeyEvent`: the real keyboard and the
/// socket's `keys` verb go through the **same** shortcut and encoder path.
struct KeyIn<'a> {
    named: Option<NamedKey>,
    character: Option<&'a str>,
    text: Option<&'a str>,
    /// Digit of the physical key (0..=9); on the socket it comes from `spec`
    /// itself.
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
            mods: ModifiersState::empty(), // the caller sets `self.mods`
        }
    }
}

/// Does this socket verb only read? A read must not cost a frame: `benchrun.py`
/// polls `state` in a loop, and every poll used to buy a full present (B4).
fn debug_read_only(line: &str) -> bool {
    let cmd: Option<String> = serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|v| v.get("cmd").and_then(|c| c.as_str()).map(str::to_owned));
    matches!(
        cmd.as_deref(),
        Some("state" | "text" | "prof" | "log" | "osc")
    )
}

/// `ctrl+shift+n` -> bytes (minimal parser: the full keyboard path is winit's,
/// this verb is for test/automation).
/// `spec` of the `keys` verb: `ctrl+shift+t`, `alt+left`, `f5`, `a`.
///
/// Resolves modifiers and key name and hands over to the real encoder
/// (`keys::encode_parts`) — there is no parallel table here. A key the encoder
/// does not know returns empty, and the socket answers `keys: [0]`.
/// `spec` of the `keys` verb -> `KeyIn`: modifiers, key name and digit.
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
        "f13" => Some(NamedKey::F13),
        "f14" => Some(NamedKey::F14),
        "f15" => Some(NamedKey::F15),
        "f16" => Some(NamedKey::F16),
        "f17" => Some(NamedKey::F17),
        "f18" => Some(NamedKey::F18),
        "f19" => Some(NamedKey::F19),
        "f20" => Some(NamedKey::F20),
        "f21" => Some(NamedKey::F21),
        "f22" => Some(NamedKey::F22),
        "f23" => Some(NamedKey::F23),
        "f24" => Some(NamedKey::F24),
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
                eprintln!("tortty: {e}");
                event_loop.exit();
                return;
            },
        };
        // `ceil`: asking for `round` gives fewer pixels than the grid needs and
        // the window is born one column/row short (the WM does not give it back)
        // the padding enters the request: the grid is `cols*cw` and the rest is
        // padding
        // signals: `SIGUSR1` (dump) and `SIGTERM` (clean exit)
        instala_sinais();
        let (w, h, pos, maximized) = self.geometry_initial(event_loop, &fonts);
        let mut attrs = Window::default_attributes()
            .with_title("tortty")
            .with_inner_size(PhysicalSize::new(w, h))
            .with_maximized(maximized);
        if let Some((x, y)) = pos {
            attrs = attrs.with_position(PhysicalPosition::new(x, y));
        }
        // K19: explicit `WM_CLASS`, not inherited from the executable name — it
        // is what the `.desktop` `StartupWMClass` looks for to group the window
        // in the dock. A renamed binary (`tortty-rs`) cannot change WM_CLASS.
        #[cfg(all(unix, not(any(target_os = "macos", target_os = "android"))))]
        let attrs = {
            use winit::platform::x11::WindowAttributesExtX11 as _;
            attrs.with_name("tortty", "tortty")
        };
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("tortty: window: {e}");
                event_loop.exit();
                return;
            },
        };
        self.window = Some(window.clone());
        // the title goes out **before** touching the GPU: creating the swapchain
        // leaves a pending X error on the connection (the Mesa WSI creates and
        // destroys pixmaps) and winit treats any X error as fatal on the next
        // property call — the following `set_title` died with a `BadDrawable`
        // that was not its own. See DEBUG.md, "pending X error".
        self.sync_title();
        let renderer = match Renderer::new(&window, fonts, self.opts.vsync, &self.opts.cfg) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("tortty: GPU: {e}");
                event_loop.exit();
                return;
            },
        };
        // the real grid comes from the size the window got, not from the request
        let size = window.inner_size();
        self.renderer = Some(renderer);
        let proxy = self.proxy.clone();
        self.wake = Some(Arc::new(move || {
            let _ = proxy.send_event(UserEvent::Wake);
        }));
        // apply the size now and **only then** the child is born: with the child
        // born earlier, the first KB come out at the requested width and the
        // first WM resize shuffles what was already on screen
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
                    // whoever opened the debug surface has to know it is not up,
                    // on stderr and in the toast
                    eprintln!("tortty: debug socket: {e}");
                    self.toast(format!("debug socket: {e}"));
                },
            }
        }
        if let Some(path) = self.opts.debug_log.clone() {
            set_log(&path);
            // the epoch is what turns the `+ms` stamps into wall clock
            let epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            logln(&format!(
                "log start: {} epoch {} ms",
                log_stamp(),
                epoch.as_millis()
            ));
        }
        logln(&format!(
            "tortty {} — {}x{} cells, cell {}x{}, font {} {}px",
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
                // feed **before** asking about `?2026`: the state that matters is
                // that of the bytes that just arrived, not that of the previous
                // read (one draw per read was G16)
                self.drain();
                let sync = self
                    .tabs
                    .get(self.active)
                    .is_some_and(|t| t.term.sync_output);
                self.gate.request(std::time::Instant::now(), sync);
            },
            UserEvent::Debug(line, reply) => {
                let draws = !debug_read_only(&line);
                let out = self.debug_dispatch(&line);
                let _ = reply.send(out);
                // `resize`/`quit` need an event cycle to take effect. A verb that
                // only reads must not buy one: `benchrun.py` polls `state` in a
                // loop and every poll was paying a full present (B4).
                if draws {
                    self.ask_frame();
                }
            },
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // on the way out the last tab may already be gone, and every handler
        // below indexes `tabs[active]`: the panic killed the exit before
        // `exiting()` saved the window state (B38)
        if self.exiting {
            if matches!(
                event,
                WindowEvent::CloseRequested | WindowEvent::RedrawRequested
            ) {
                event_loop.exit();
            }
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Moved(pos) => {
                // outer position (frame): it is the one `with_position`
                // consumes — saving the client position walked the window down a
                // title bar on every run
                self.geom_pos = Some((pos.x, pos.y));
            },
            WindowEvent::Resized(size) => {
                // J5: maximized is WM state, not size; saving it here would lose
                // the size from before maximizing
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
            },
            WindowEvent::ModifiersChanged(m) => {
                let ctrl = self.mods.control_key();
                self.mods = m.state();
                // Ctrl is what shows and opens the `OSC 8` links: the underline
                // has to appear and vanish with the modifier, and no other event
                // arrives to ask for that frame
                if ctrl != self.mods.control_key() {
                    self.redraw();
                }
            },
            WindowEvent::KeyboardInput { event, .. } => {
                // the release only has value under the kitty protocol (flag 2);
                // the legacy path does not report releases, and `encode` returns
                // nothing
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
        // the reload needs a tick even with no event at all
        let mut when = now + Duration::from_millis(250);
        // coalesced frame: expired → one draw request; else wake up on it
        if self.gate.won(now) {
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }
        if let Some(t) = self.gate.when() {
            when = when.min(t);
        }

        // signals: the handler only raises the flag, whoever acts is here
        let (dump, finish) = signals();
        if dump {
            self.dump();
        }
        if finish {
            self.dump();
            self.exiting = true;
            event_loop.exit();
            return;
        }
        // `--quit-after`: closes by itself (automation) and leaves the dump
        if self.deadline.is_some_and(|p| now >= p) {
            self.deadline = None;
            self.dump();
            self.exiting = true;
            event_loop.exit();
            return;
        }

        // an expired toast leaves the screen (nobody else would take it out)
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.since.elapsed() > TOAST_DUR)
        {
            self.toast = None;
            if let Some(w) = self.window.as_ref() {
                w.request_redraw();
            }
        }

        // blink: wake up on the next phase flip; redraw only when it really
        // flips (one redraw every 500 ms, not one per tick)
        if self.blink_active() {
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
        // BEL flash (I13): wake up when the first label goes dark and redraw
        // once to remove the highlight
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

        // a tab whose shell died closes by itself; the last one closes the
        // workspace, and only the last workspace closes the app (I20). Here was
        // the `M-W`: the shortcut `tabs.len() == 1 → exit` skipped
        // `drop_empty_workspace`, so a `Ctrl+D`/`exit` in the visible tab killed
        // the whole app even with another workspace parked. The death of the
        // child has to enter the **same** path as
        // `close_tab_at`.
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
        // the tab may be gone already: closing the app by the shell exiting is
        // exactly that case (B38)
        if let Some(pty) = self.tabs.get_mut(self.active).and_then(|t| t.pty.as_mut()) {
            pty.kill();
        }
        self.debug = None;
    }
}

/// Lines of one tab for the dump: recent scrollback and the live viewport.
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
    // the offset goes in the header: the lines below are the live grid, not the
    // scrolled view — without it the dump does not tell whether the view was in
    // the history
    let off = if t.view_off > 0 {
        format!(" (history +{})", t.view_off)
    } else {
        String::new()
    };
    let mark = if ativa { "" } else { " [background]" };
    out.push(format!(
        "=== viewport {}x{}{off}{mark} ===",
        t.term.cols, t.term.rows
    ));
    for y in 0..t.term.rows {
        out.push(t.term.row_text(y).trim_end().to_string());
    }
    out
}

// ------------------------------------------------------------ signals

/// `SIGUSR1` asked for a dump; `SIGTERM` asked for a clean exit (dump + close).
static SIGNAL_DUMP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SIGNAL_TERM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_usr1(_: libc::c_int) {
    SIGNAL_DUMP.store(true, std::sync::atomic::Ordering::Relaxed);
}

extern "C" fn on_term(_: libc::c_int) {
    SIGNAL_TERM.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Installs `SIGUSR1`/`SIGTERM`. The handler only raises an `AtomicBool`:
/// `store` on `AtomicBool` is lock free and safe in a handler, and whoever acts
/// is the event loop (which wakes up every 250 ms anyway).
///
/// `unsafe`: `libc::signal` swaps the process handler — there is no safe API
/// for that, as with the `debugsock` `SO_PEERCRED`.
#[allow(unsafe_code)]
pub fn instala_sinais() {
    unsafe {
        libc::signal(libc::SIGUSR1, on_usr1 as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_term as *const () as libc::sighandler_t);
    }
}

/// Signal readout (once each): (dump, quit).
fn signals() -> (bool, bool) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        SIGNAL_DUMP.swap(false, Relaxed),
        SIGNAL_TERM.swap(false, Relaxed),
    )
}

/// cwd for the next tab: `/proc` answers where the program in front really is,
/// `OSC 7` only where the shell thinks it is — so `/proc` wins and the shell's
/// word is the last resort (a container, a stricter mount, a process tree the
/// walk missed). A path that is no longer a directory is not a cwd: spawning a
/// tab there would fail after the decision.
fn prefer_cwd(
    from_proc: Option<std::path::PathBuf>,
    from_osc: Option<&str>,
) -> Option<std::path::PathBuf> {
    if from_proc.is_some() {
        return from_proc;
    }
    let path = std::path::PathBuf::from(from_osc?);
    path.is_dir().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_tab_prefers_proc_and_takes_osc7_as_the_last_resort() {
        let here = std::env::temp_dir();
        let here_s = here.to_str().expect("temp dir is UTF-8");
        // `/proc` answered: the shell's word does not override it
        assert_eq!(
            prefer_cwd(Some(here.clone()), Some("/nope")),
            Some(here.clone())
        );
        // `/proc` did not: the shell is what is left
        assert_eq!(prefer_cwd(None, Some(here_s)), Some(here.clone()));
        // and a path that no longer exists does not become a cwd
        assert_eq!(prefer_cwd(None, Some("/nope/nope/nope")), None);
        assert_eq!(prefer_cwd(None, None), None);
    }
}
