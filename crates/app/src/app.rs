//! The app: window, pty, keyboard, drawing and the introspection socket.

use std::io::{Read as _, Write as _};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tortty_core::config::{Bell, RestoreState};
use tortty_core::keymap::Action;
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
use crate::platform;
use crate::prof;
use crate::pty::Pty;
use crate::render::{Renderer, Severity, ToastText};

/// Is the pointer inside the box the renderer drew? Half-open on the right and
/// on the bottom, like a cell: the press that lands exactly on the border
/// belongs to the neighbour, and to the grid.
fn inside_box(rect: [f32; 4], px: f64, py: f64) -> bool {
    let [x, y, w, h] = rect.map(f64::from);
    px >= x && px < x + w && py >= y && py < y + h
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
    /// A notification arrived with this tab out of sight (`B8`): the card of its
    /// workspace keeps the signal until someone enters it, and the toast comes
    /// when the tab becomes visible.
    card_alert: bool,
    urgency: bool,
    beeps: u32,
}

struct Toast {
    title: String,
    /// Second level: the line and the key, the path, the text the program sent.
    detail: String,
    severity: Severity,
    /// How long it stays. Comes from the severity — a receipt is read faster
    /// than an error — except for a notification, which keeps the time the badge
    /// floor assumes.
    life: Duration,
    since: std::time::Instant,
    /// How many equal messages arrived in a row (the toast adds up, it does not
    /// stack).
    times: u32,
}

impl Toast {
    fn new(title: &str, detail: &str, severity: Severity) -> Toast {
        Toast {
            title: title.to_string(),
            detail: detail.to_string(),
            severity,
            life: match severity {
                Severity::Ok => TOAST_OK,
                Severity::Warn | Severity::Err => TOAST_LONG,
            },
            since: std::time::Instant::now(),
            times: 1,
        }
    }

    /// Same words twice in a row: it is the same notice, and the count is what
    /// grew. The clock is **not** restarted — a program repeating itself must not
    /// pin the box to the screen (`B8`).
    fn same(&self, title: &str, detail: &str) -> bool {
        self.title == title && self.detail == detail
    }

    fn expired(&self) -> bool {
        self.since.elapsed() > self.life
    }
}

/// A receipt is read in a glance: 3 s. A warning or an error carries something to
/// read and act on: 10 s. No key takes the toast out — a notice that vanishes
/// before it can be read is a notice that was never delivered — but a click does,
/// because that is the owner saying they have read it.
const TOAST_OK: Duration = Duration::from_secs(3);
const TOAST_LONG: Duration = Duration::from_secs(10);

/// Text of the configuration notice (K13): what went wrong, and — on a reload —
/// the fact the owner asks first: the running configuration did **not** change.
/// `kept` is false at startup, where there is no previous configuration to keep
/// (the good lines apply, the bad ones fall back to the default).
fn notice_config(errs: &[tortty_core::config::Error], kept: bool) -> String {
    let Some(first) = errs.first() else {
        return String::new();
    };
    let rest = match errs.len() {
        1 => String::new(),
        n => format!(" (+{} more)", n - 1),
    };
    if kept {
        format!("kept the previous one — {first}{rest}")
    } else {
        format!("{first}{rest}")
    }
}

/// Blink period (cursor and SGR 5). Half "on", half "off".
const BLINK_MS: u64 = 500;

/// How long the tab label stays highlighted after a BEL (I13).
const FLASH_DUR: Duration = Duration::from_millis(800);
/// A notification stays immune to keys this long (`M-K`): whoever is typing
/// must not lose it. It is the notification toast's own time, because the toast
/// outlives the key that would dismiss it — what the floor now guards is the
/// badge, which has no other expiry.
const NOTIF_FLOOR: Duration = Duration::from_secs(8);

/// One card per this long while a drag holds the pointer in the band below the
/// last card (`B55`). Long enough to read the list walking, short enough not to
/// feel stuck; a per-frame step would cross eight workspaces in 130 ms.
const CARD_WALK: Duration = Duration::from_millis(500);

/// Slot, in `parked`, of the workspace that takes the place of the one in front
/// when it closes (`B62`).
///
/// The logical order is `parked[0..ws_idx]`, the front, `parked[ws_idx..]`, so
/// the successor is `parked[ws_idx]` — and the predecessor, `parked[ws_idx - 1]`,
/// when the front was the last. `None` = nothing left: the app closes.
pub(crate) fn successor_slot(ws_idx: usize, parked: usize) -> Option<usize> {
    if parked == 0 {
        return None;
    }
    if ws_idx < parked {
        return Some(ws_idx);
    }
    Some(ws_idx.saturating_sub(1))
}

/// What a run reads from the state file (`B61`): the window's geometry, and the
/// session layout.
///
/// The two are different questions. The layout belongs to the session, and only
/// the window that owns it (`B46`) may restore and rewrite it — two windows
/// reopening and saving one tab list would erase each other's work. The size and
/// the place belong to the window: a second window is the same terminal, and
/// opening it at the first-run default was the bug.
///
/// Two opt-outs stay, and both are the command line saying what it wants: a grid
/// (`--cols`/`--rows`, a measurement) and `restore_state = none` ("this terminal
/// has no state", documented as no geometry either).
#[must_use]
pub(crate) fn state_use(
    grid_explicito: bool,
    restore: tortty_core::config::RestoreState,
    owner: bool,
) -> (bool, bool) {
    use tortty_core::config::RestoreState;
    let geometry = !grid_explicito && restore != RestoreState::None;
    (geometry, geometry && owner)
}

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

/// A context menu open at the pointer (`M-V`): which rows it lists, what it
/// hangs from, and what it was opened **on**. The rows are [`chrome::Item`]s, so
/// the grid's, a card's and a tab's menu are the same struct with a different
/// list — nothing here knows which menu it is.
struct MenuState {
    /// Where it was opened, in window px: the box hangs from here.
    at: (f64, f64),
    /// The rows, in the order they are drawn and hit.
    items: &'static [crate::chrome::Item],
    /// Whether the box hangs from a cell or from the pointer itself.
    anchor: crate::chrome::Anchor,
    /// The card or tab it acts on (the grid's menu, and the bar's strip, have no
    /// subject).
    subject: Option<usize>,
    /// The `OSC 8` target under the pointer when it opened, if any.
    link: Option<String>,
    /// Which rows were usable, decided once: they belong to the click.
    enabled: Vec<bool>,
    /// The row the pointer is on, for the highlight.
    hover: Option<usize>,
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
    /// The selection being dragged is a **block** (`B5`): decided by Alt at the
    /// press and kept for the whole drag, because the shape cannot change under
    /// the hand that is drawing it.
    pub sel_block: bool,
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
    /// When each animated image of this tab is due (`B2`). Per tab because the
    /// clock only runs for the tab on screen: a parked animation waits, it does
    /// not jump six frames when you come back.
    pub anim_due: std::collections::HashMap<u32, std::time::Instant>,
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
            sel_block: false,
            press: None,
            dragging: false,
            click: None,
            scroll_frac: 0.0,
            child_frac: (0.0, 0.0),
            sync_at: None,
            bells_seen: 0,
            notifs_seen: 0,
            anim_due: std::collections::HashMap::new(),
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
            } else {
                // out of sight: the card is the signal, and the content waits
                // for the visit (`B8`)
                out.card_alert = true;
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
        tab_cwd(self.pty.as_ref(), self.term.cwd.as_deref())
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
    /// A tab here notified while the workspace was parked (`B8`): the card says
    /// so until the workspace is entered.
    card_alert: bool,
}

impl Workspace {
    /// Wraps the current session to park it.
    fn take(app: &mut App) -> Workspace {
        Workspace {
            tabs: std::mem::take(&mut app.tabs),
            active: app.active,
            name: std::mem::take(&mut app.ws_name),
            card_alert: std::mem::take(&mut app.card_alert),
        }
    }

    /// Hands the contents back into the slot of the current session.
    fn into_current(self, app: &mut App) {
        app.tabs = self.tabs;
        app.active = self.active.min(app.tabs.len().saturating_sub(1));
        app.ws_name = self.name;
        app.card_alert = self.card_alert;
    }
}

/// Parks the workspace being left at the logical position it occupied and
/// answers the logical index of the new front (the last one, since the new
/// workspace is appended to the order).
///
/// `parked` holds the workspaces that are **not** in front, in logical order,
/// so the slot of the front is exactly `at`. Inserting anywhere else renumbers
/// every workspace after it (`B54`).
/// Where the workspace in front ends up when the one at `from` is moved to
/// `to` (`B49`): it is only a place in the list, so it follows the removal and
/// the insertion like any other — unless it is the one being moved.
fn moved_front(front: usize, from: usize, to: usize) -> usize {
    if from == front {
        to
    } else if from < front && to >= front {
        front - 1
    } else if from > front && to <= front {
        front + 1
    } else {
        front
    }
}

/// The neighbour above, clamped at the first workspace. The pure half of
/// `Super+Up` and of `Super+Shift+Up` (`B49`): the App half is
/// `select_workspace`/`move_workspace`, which already ignore a move to
/// themselves, so the walk and the carry both stop at the first one.
fn ws_above(i: usize) -> usize {
    i.saturating_sub(1)
}

/// The neighbour below, clamped at the last one. The pure half of `Super+Down`
/// and of `Super+Shift+Down` (`B49`).
fn ws_below(i: usize, total: usize) -> usize {
    (i + 1).min(total.saturating_sub(1))
}

/// Removes the workspace at `from` and inserts it at `to`, carrying the one in
/// front with it (`B49`). The pure half of the move: it is the whole of it that
/// can be checked without a window.
fn reorder(all: &mut Vec<Workspace>, front: &mut usize, from: usize, to: usize) {
    *front = moved_front(*front, from, to);
    let moved = all.remove(from);
    all.insert(to, moved);
}

fn park_front(parked: &mut Vec<Workspace>, front: Workspace, at: usize) -> usize {
    parked.insert(at.min(parked.len()), front);
    parked.len()
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
    /// Name of the workspace in front — empty until the owner renames it
    /// (`I8`); an empty name is a valid state, not a missing one (`B43`).
    ws_name: String,
    /// The workspaces that are not in front. The one in front does not live
    /// here: it **is** `tabs`/`active`. Invariant: `parked.len() + 1 ==
    /// ws_count()`, with the logical indices in order (the one in front falls
    /// in the middle).
    parked: Vec<Workspace>,
    /// A tab of the workspace in front notified while it was out of sight
    /// (`B8`): the card stays alert until the workspace is entered, even after
    /// the tab that notified was visited — the card is the workspace's signal.
    card_alert: bool,
    /// Logical index of the workspace in front, in `0..ws_count()`.
    ws_idx: usize,
    /// Tab pressed on the bar, to drag and reorder (I10): index and the x of
    /// the press, so the drag only starts after actually moving.
    bar_press: Option<(usize, f64)>,
    /// Card being dragged on the sidebar: the logical index it sits at now and
    /// the pixel of the press (`B49`). The press point travels with it, like
    /// `bar_press` does, so the 3 px threshold is not spent again by every
    /// motion event.
    card_press: Option<(usize, (f64, f64))>,
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
    /// The scrollback search (`M-D`). While it is `Some`, the keyboard belongs to
    /// the query and the screen shows the matches; `Esc` closes it.
    search: Option<tortty_core::search::Search>,
    /// Bumped whenever the highlight can change: it is what makes the renderer
    /// rebuild the rows even with an unmoved view (`M-D`).
    search_gen: u64,
    /// Instant of the last card click, for the double click (I8).
    card_click: Option<(usize, std::time::Instant)>,
    /// Last wheel turn handled on the bar (instant and direction). X11 sends
    /// **two** events per notch (press and release), and without this one notch
    /// switches tab and the second undoes it.
    bar_wheel: Option<(std::time::Instant, i8)>,
    /// Travel still owed to the bar, in detents: the fraction of a notch a
    /// touchpad sent that has not become a tab change yet (2026-11-07).
    bar_frac: f32,
    /// First card shown in the sidebar (`B42`): the list is drawn from here, and
    /// the wheel over the column walks it. Without it a workspace past the bottom
    /// of the window exists and cannot be reached by the mouse.
    card_off: usize,
    /// Travel still owed to the card list, in detents (a touchpad fraction), for
    /// the same reason as `bar_frac`.
    card_frac: f32,
    /// When the drag last walked the card list by itself (`B55`): `Some` while
    /// the pointer rests in a band at one end of the column, and the instant is
    /// the last automatic step — so `CARD_WALK` later it steps again.
    card_walk: Option<std::time::Instant>,
    /// Which way the band walks: `1` down from the bottom row, `-1` up from the
    /// top one.
    card_walk_dir: i64,
    /// Workspaces and tabs to reopen, read from the state file before the first
    /// tab is born (`M-I`). `None` = open one tab, as always.
    restore: Option<crate::state::Layout>,
    /// The `flock` that makes this window the owner of the state (`B46`): it
    /// restores, applies the remembered geometry and is the only one that
    /// writes. `None` = a scratch window, which forgets nothing because it owns
    /// nothing. It lives as long as the app: dropping it releases the lock.
    owner: Option<crate::state::Owner>,
    /// Last detent handed to a mouse-reporting child (instant and direction), for
    /// the same reason as `bar_wheel`: the press and the release of one notch
    /// arrive as two `MouseWheel` events, and both would be reported (`B40`).
    wheel_pair: Option<(std::time::Instant, i8)>,
    mods: ModifiersState,
    /// Last mouse position in physical pixels (the base of `cell_at`).
    cursor_px: (f64, f64),
    /// Button currently held, to report release/motion with it.
    held: Option<u8>,
    /// The press was consumed by chrome drawn over the grid — the toast or the
    /// status chip (`B47`) — so the release that ends that same gesture is not
    /// reported to the child. Without it a child holding the mouse (`?1000`)
    /// receives a release it never saw a press for, which lands in its input as
    /// a stray `CSI < … m`.
    press_ours: bool,
    clip: Clip,
    /// The grid's context menu (`M-V`), when it is open.
    menu_open: Option<MenuState>,
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
    /// Notice on screen (config error, notification). One at a time: it goes away
    /// alone, with a click, or gives its turn to the next one.
    toast: Option<Toast>,
    /// Notices that arrived while another one was on screen (the python's
    /// revealer had the same queue: an aviso does not overwrite the one being
    /// read).
    toast_queue: Vec<Toast>,
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
            search: None,
            search_gen: 0,
            opts,
            over,
            proxy,
            window: None,
            renderer: None,
            tabs: vec![tab],
            active: 0,
            ws_name: String::new(),
            parked: Vec::new(),
            ws_idx: 0,
            bar_press: None,
            card_press: None,
            scroll_drag: false,
            consumed: None,
            rename: None,
            card_click: None,
            bar_wheel: None,
            bar_frac: 0.0,
            card_off: 0,
            card_frac: 0.0,
            card_walk: None,
            card_walk_dir: 1,
            card_alert: false,
            restore: None,
            owner: None,
            wheel_pair: None,
            mods: ModifiersState::empty(),
            cursor_px: (0.0, 0.0),
            held: None,
            press_ours: false,
            clip: Clip::new(),
            menu_open: None,
            wake: None,
            spawned: false,
            cfg_mtime,
            cfg_checked: std::time::Instant::now(),
            blink_epoch: std::time::Instant::now(),
            blink_phase: true,
            blink_text: false,
            gate: crate::frame::Gate::default(),
            toast: None,
            toast_queue: Vec::new(),
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
                parts.push(if name.is_empty() {
                    format!("=== workspace {} ===", i_ws + 1)
                } else {
                    format!("=== workspace {}: {name} ===", i_ws + 1)
                });
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

    /// Writes the dump where `--dump-to` said (else the system's default).
    fn dump(&mut self) {
        let path = self
            .opts
            .dump_to
            .clone()
            .unwrap_or_else(platform::dump_path);
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
        self.window.as_ref().map_or(0, |w| platform::window_id(w))
    }

    /// pty write with counters (`prof`): bytes and repeats per `EAGAIN`.
    /// Loads an image file for the kitty protocol (`B1`): `t=f` reads, `t=t`
    /// reads and deletes, `t=s` reads shared memory like a file. `~` expands
    /// to the home directory — the only expansion, done here so the core
    /// never resolves paths. Reads at most one byte over the cap: anything
    /// bigger answers `EINVAL:too big` from the core instead of growing here.
    /// The `t=t` delete follows the protocol: only a path inside a known
    /// temporary directory carrying `tty-graphics-protocol` — anything else
    /// is read but kept, since the path comes from the child (over `ssh`,
    /// from a remote).
    fn read_image_file(path: &str, delete: bool) -> Option<Vec<u8>> {
        let path = match path.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => {
                platform::home()?.join(rest.trim_start_matches('/'))
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
        if delete && platform::temp_delete_allowed(&path) {
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
                    ativa: true,
                    alert: self.card_alert,
                    progress: self.tabs[self.active].term.progress,
                    status: self.status_word(&self.tabs[self.active].term),
                });
            } else {
                let slot = if i < self.ws_idx { i } else { i - 1 };
                let w = &self.parked[slot];
                cards.push(crate::render::WsCard {
                    name: editing.unwrap_or_else(|| w.name.clone()),
                    title: w.tabs[w.active].title(),
                    ativa: false,
                    alert: w.card_alert,
                    progress: w.tabs[w.active].term.progress,
                    status: self.status_word(&w.tabs[w.active].term),
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

    /// The state word for the tab label and the card, under `status_show tab`;
    /// `None` in the other modes and when the root record is `idle` — a word for
    /// "nothing is happening" would never leave the screen.
    fn status_word(&self, term: &tortty_core::Term) -> Option<String> {
        if self.opts.cfg.status_show != tortty_core::config::StatusShow::Tab {
            return None;
        }
        let root = term.status.root()?;
        (root.state != tortty_core::status::State::Idle).then(|| root.state.name().to_string())
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
                progress: t.term.progress,
                status: self.status_word(&t.term),
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

    /// Index of the tab under the cursor, if it is in the bar strip (`M-Q`: the
    /// rects come from the same layout the bar is drawn from).
    fn tab_at(&self, px: f64, py: f64) -> Option<usize> {
        self.chrome_layout().tab_at(px, py)
    }

    /// Workspace under the cursor, if it is on the sidebar (`I9`).
    ///
    /// The walk strip below the last card names no card: it is the place a drag
    /// scrolls from (`B55`), and the layout simply has no slot there.
    fn card_at(&self, px: f64, py: f64) -> Option<usize> {
        self.chrome_layout().card_at(px, py)
    }

    /// Opens the context menu the pointer is standing on (`M-V`, `B64`): a card's
    /// on the sidebar, a tab's on the bar, the bar's own strip right of the last
    /// tab, and the grid's over the screen — which is the child's, when the child
    /// asked for the mouse (then `Shift` asks for ours, and nothing opens without
    /// it).
    ///
    /// The geometry is built on demand from the pointer, so it can never go
    /// stale; what is kept is what the menu was opened **on** (the link under the
    /// pointer, which card or tab) and the rows that were usable at that moment —
    /// they belong to the click, not to the frame. Answers whether it opened, so
    /// the caller knows whether the press is ours.
    fn open_menu(&mut self) -> bool {
        let (px, py) = self.cursor_px;
        let (me, layout) = (self.chrome_metrics(), self.chrome_layout());
        let (items, anchor, subject) = if let Some(i) = layout.card_at(px, py) {
            (
                &crate::chrome::CARD_MENU[..],
                crate::chrome::Anchor::Pointer,
                Some(i),
            )
        } else if let Some(i) = layout.tab_at(px, py) {
            (
                &crate::chrome::TAB_MENU[..],
                crate::chrome::Anchor::Pointer,
                Some(i),
            )
        } else if py < f64::from(me.pad + me.bar) {
            (
                &crate::chrome::BAR_MENU[..],
                crate::chrome::Anchor::Pointer,
                None,
            )
        } else if self.in_grid(px, py) {
            if !(self.mods.shift_key() || self.tabs[self.active].term.mouse_mode == 0) {
                return false;
            }
            (
                &crate::chrome::GRID_MENU[..],
                crate::chrome::Anchor::Cell,
                None,
            )
        } else {
            // the sidebar's free row and the window's margin act on nothing
            return false;
        };
        // a link can only be under the pointer where there are cells to hold it
        let link = match anchor {
            crate::chrome::Anchor::Cell => {
                let (col, row) = self.cell_at(px, py);
                self.link_at(row, col)
            },
            crate::chrome::Anchor::Pointer => None,
        };
        let enabled = items
            .iter()
            .map(|i| self.item_usable(*i, subject, link.is_some()))
            .collect();
        self.menu_open = Some(MenuState {
            at: self.cursor_px,
            items,
            anchor,
            subject,
            link,
            enabled,
            hover: None,
        });
        self.redraw();
        true
    }

    /// Whether the pointer is over the grid's own area (not the sidebar, not the
    /// bar, not the margin).
    fn in_grid(&self, px: f64, py: f64) -> bool {
        let metrics = self.chrome_metrics();
        let (x, y) = metrics.in_origin(px, py);
        #[allow(clippy::cast_precision_loss)]
        let (width, height) = (
            metrics.cols as f32 * metrics.cw,
            metrics.rows as f32 * metrics.ch,
        );
        x >= 0.0 && y >= 0.0 && x < width && y < height
    }

    /// Whether one row of a menu can do anything right now. `subject` is the card
    /// or tab it would act on; `link` says whether the pointer is on a link.
    fn item_usable(&self, item: crate::chrome::Item, subject: Option<usize>, link: bool) -> bool {
        use crate::chrome::Item;
        match item {
            Item::Copy => self.selection_text().is_some_and(|t| !t.trim().is_empty()),
            Item::OpenLink => link,
            Item::Paste | Item::NewTab | Item::NewWorkspace | Item::Rename => true,
            // closing the last one closes the window (`I20`): that is a keyboard
            // decision, not one a menu row should make by accident
            Item::CloseTab => self.tabs.len() > 1,
            Item::CloseWorkspace => self.ws_count() > 1,
            Item::MoveUp => subject.is_some_and(|i| i > 0),
            Item::MoveDown => subject.is_some_and(|i| i + 1 < self.ws_count()),
        }
    }

    fn close_menu(&mut self) {
        if self.menu_open.take().is_some() {
            self.redraw();
        }
    }

    /// Runs row `i` of the open menu. The row names itself (`chrome::Item`), so a
    /// menu is a list and not an index table; `subject` is the card or the tab it
    /// acts on, which the item knows nothing about.
    fn menu_action(&mut self, i: usize) {
        use crate::chrome::Item;
        let Some(state) = self.menu_open.take() else {
            return;
        };
        let Some(item) = state.items.get(i).copied() else {
            self.redraw();
            return;
        };
        if !state.enabled.get(i).copied().unwrap_or(false) {
            self.redraw();
            return;
        }
        let subject = state.subject;
        match item {
            Item::Copy => self.copy(),
            Item::Paste => self.paste(),
            Item::OpenLink => {
                if let Some(uri) = state.link {
                    self.open_link(&uri);
                }
            },
            Item::NewTab => self.new_tab(),
            Item::CloseTab => {
                if let Some(i) = subject {
                    self.close_tab_at(i);
                }
            },
            Item::NewWorkspace => self.new_workspace(),
            Item::Rename => {
                if let Some(i) = subject {
                    self.start_rename(i);
                }
            },
            Item::CloseWorkspace => {
                if let Some(i) = subject {
                    self.close_workspace(i);
                }
            },
            Item::MoveUp => {
                if let Some(i) = subject {
                    self.move_workspace(i, ws_above(i));
                }
            },
            Item::MoveDown => {
                if let Some(i) = subject {
                    self.move_workspace(i, ws_below(i, self.ws_count()));
                }
            },
        }
        self.redraw();
    }

    /// Keys of the open menu: arrows walk the rows, `Enter` runs the one under
    /// the hand, `Escape` closes. Anything else closes it and goes on its way
    /// (the child keeps its keyboard).
    fn menu_key(&mut self, k: &KeyIn<'_>) -> bool {
        let Some(state) = self.menu_open.as_mut() else {
            return false;
        };
        let usable: Vec<usize> = (0..state.enabled.len())
            .filter(|i| state.enabled[*i])
            .collect();
        match k.named {
            Some(NamedKey::Escape) => {
                self.close_menu();
                true
            },
            Some(NamedKey::Enter | NamedKey::Space) => {
                if let Some(i) = state.hover.or_else(|| usable.first().copied()) {
                    self.menu_action(i);
                } else {
                    self.close_menu();
                }
                true
            },
            Some(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let down = k.named == Some(NamedKey::ArrowDown);
                let at = state
                    .hover
                    .and_then(|h| usable.iter().position(|i| *i == h));
                let next = match (at, down) {
                    (Some(i), true) => usable
                        .get(i + 1)
                        .copied()
                        .or_else(|| usable.first().copied()),
                    (Some(i), false) => i
                        .checked_sub(1)
                        .and_then(|p| usable.get(p).copied())
                        .or_else(|| usable.last().copied()),
                    (None, _) => usable.first().copied(),
                };
                state.hover = next;
                self.redraw();
                true
            },
            _ => {
                self.close_menu();
                false
            },
        }
    }

    /// What the renderer needs to draw the open menu (`M-V`).
    fn menu_view(&self) -> Option<crate::render::MenuView> {
        let state = self.menu_open.as_ref()?;
        Some(crate::render::MenuView {
            layout: self.menu_layout()?,
            enabled: state.enabled.clone(),
            hover: state.hover,
        })
    }

    /// The chrome layout of the open menu (built from the pointer each time).
    fn menu_layout(&self) -> Option<crate::chrome::Menu> {
        self.menu_open
            .as_ref()
            .map(|m| crate::chrome::Menu::build(self.chrome_metrics(), m.items, m.anchor, m.at))
    }

    /// The pointer is on the sidebar column (`B42`).
    ///
    /// Broader than `card_at` on purpose: the wheel over the column belongs to
    /// the card list even where a card is missing (a short list) or cut (the end
    /// of the window), and there it would fall through to the grid.
    fn in_sidebar(&self, px: f64, py: f64) -> bool {
        let (cw, _ch, pad) = self.cell_metrics();
        let side = f64::from(self.side_w(cw));
        // the column starts on the tab bar's line, beside the grid
        side > 0.0 && px >= f64::from(pad) && px < f64::from(pad) + side && py >= f64::from(pad)
    }

    /// How many **whole** cards fit in the sidebar column, at least one, leaving
    /// the last text row free.
    ///
    /// The column runs from the bar's line to the bottom of the grid: it is
    /// taller than the grid by one bar, and the bottom padding is not part of it
    /// (a card there would hang past the last row of text). The free row is what
    /// a drag aims at to walk the list (`B55`): a card cut by the bottom edge
    /// would be a place and a scroll trigger at the same time, and pushing past
    /// it turned the walk on with the hand still over a card.
    fn cards_visible(&self) -> usize {
        self.chrome_metrics().cards_visible()
    }

    /// The metrics the chrome is laid out with (`M-Q`): the renderer builds the
    /// same ones, and both ask `chrome::Layout` — one formula, two readers.
    fn chrome_metrics(&self) -> crate::chrome::Metrics {
        let (cw, ch, pad) = self.cell_metrics();
        let (cols, rows) = self
            .tabs
            .get(self.active)
            .map_or((self.opts.cols, self.opts.rows), |t| {
                (t.term.cols, t.term.rows)
            });
        crate::chrome::Metrics {
            cw,
            ch,
            pad,
            bar: self.bar_h(ch),
            side: self.side_w(cw),
            cols,
            rows,
        }
    }

    /// The chrome of the current state: what the renderer will draw next and what
    /// a click is tested against (`M-Q`).
    fn chrome_layout(&self) -> crate::chrome::Layout {
        let info = self.tabinfo();
        let specs: Vec<crate::chrome::Tab<'_>> = info
            .iter()
            .map(|t| crate::chrome::Tab {
                title: &t.title,
                alert: t.alert,
                state: t.status.as_deref(),
            })
            .collect();
        crate::chrome::Layout::build(
            self.chrome_metrics(),
            &specs,
            self.ws_count(),
            self.card_off,
        )
    }

    /// Walks the sidebar card list by `delta` cards, clamped to it (`B42`).
    fn scroll_cards(&mut self, delta: i64) {
        let max = self.ws_count().saturating_sub(self.cards_visible());
        let at = (self.card_off as i64 + delta).clamp(0, max as i64);
        #[allow(clippy::cast_sign_loss)] // clamped to `0..=max` above
        let at = at as usize;
        if at != self.card_off {
            self.card_off = at;
            self.redraw();
        }
    }

    /// Steps the list once when the drag has rested in the band long enough
    /// (`B55`), and answers when the next step is due — `None` when no drag is
    /// walking, so the loop has nothing to wake up for.
    fn card_walk_tick(&mut self, now: std::time::Instant) -> Option<std::time::Instant> {
        let at = self.card_walk?;
        if now.duration_since(at) >= CARD_WALK {
            self.card_walk = Some(now);
            self.scroll_cards(self.card_walk_dir);
            self.ask_frame();
        }
        self.card_walk.map(|at| at + CARD_WALK)
    }

    /// `B2`: the animation clock. The core holds the frames and the gaps; this
    /// decides *when* — one wake-up per frame, and only for the tab on screen
    /// (a parked animation is paused, not fast-forwarded).
    fn anim_tick(&mut self, now: std::time::Instant) -> Option<std::time::Instant> {
        let mut changed = false;
        let wake = {
            let t = &mut self.tabs[self.active];
            let due = &mut t.anim_due;
            let g = &mut t.term.graphics;
            let live: Vec<u32> = g
                .images()
                .values()
                .filter(|i| i.anim.mode >= 2 && i.gap_ms(i.current()).is_some())
                .map(|i| i.id)
                .collect();
            let mut wake: Option<std::time::Instant> = None;
            for id in live {
                let gap = g
                    .images()
                    .get(&id)
                    .and_then(|i| i.gap_ms(i.current()))
                    .map_or(40, u64::from);
                let at = *due
                    .entry(id)
                    .or_insert(now + std::time::Duration::from_millis(gap));
                if now >= at {
                    if g.advance(id) {
                        changed = true;
                    }
                    let step = g
                        .images()
                        .get(&id)
                        .and_then(|i| i.gap_ms(i.current()))
                        .map_or(40, u64::from);
                    due.insert(id, now + std::time::Duration::from_millis(step));
                }
                let at = due[&id];
                wake = Some(wake.map_or(at, |w| w.min(at)));
            }
            due.retain(|id, _| {
                g.images()
                    .get(id)
                    .is_some_and(|i| i.anim.mode >= 2 && i.gap_ms(i.current()).is_some())
            });
            wake
        };
        if changed {
            self.ask_frame();
        }
        wake
    }

    /// The strip at each end of the column where a drag walks the list (`B55`):
    /// the free row below the cards walks down, the bar's row and the first card
    /// row walk up.
    ///
    /// The bottom one is a place of its own — no card is drawn there — so pushing
    /// the pointer past the last card scrolls, and being **on** a card still
    /// means "drop it here". Above the first card there is no such room (the
    /// cards start at the top of the column), so the walk up takes the first
    /// card's top row too, and everything above it, up to the window's edge:
    /// a hand that drags to the top of the window finds it.
    fn card_band(&self, x: f64, y: f64) -> Option<i64> {
        let (cw, ch, pad) = self.cell_metrics();
        let side = f64::from(self.side_w(cw));
        if side <= 0.0
            || self.card_press.is_none()
            || x < f64::from(pad)
            || x >= f64::from(pad) + side
        {
            return None;
        }
        let top = f64::from(pad) + f64::from(self.bar_h(ch));
        let cards_end = top + self.cards_visible() as f64 * f64::from(ch) * 2.4;
        if y >= cards_end {
            return (self.card_off + self.cards_visible() < self.ws_count()).then_some(1);
        }
        if y < top + f64::from(ch) {
            return (self.card_off > 0).then_some(-1);
        }
        None
    }

    /// Keeps the offset inside the list: closing a workspace or growing the
    /// window can leave it past the end (`B42`).
    fn clamp_cards(&mut self) {
        if self.tabs.is_empty() {
            self.card_off = 0;
            return;
        }
        let max = self.ws_count().saturating_sub(self.cards_visible());
        self.card_off = self.card_off.min(max);
    }

    /// Scrolls the list the least it takes to put card `i` in it (`B42`):
    /// switching workspace by keyboard must not leave the card out of sight.
    fn reveal_card(&mut self, i: usize) {
        let n = self.cards_visible();
        if i < self.card_off {
            self.card_off = i;
        } else if i >= self.card_off + n {
            self.card_off = i + 1 - n;
        }
    }

    /// URI of the cell under the pointer, in the visible grid: what the child
    /// declared with `OSC 8`, else the URL the text there spells (`B69`). The
    /// declared one wins — it is the only one who knows, and a detector reading
    /// over it would be guessing at its expense.
    fn link_at(&self, row: usize, col: usize) -> Option<String> {
        let tab = &self.tabs[self.active];
        let lines = tab.term.visible_rows(tab.view_off);
        let id = lines.get(row)?.cells.get(col)?.link;
        if let Some(uri) = tab.term.link_uri(id) {
            return Some(uri.to_string());
        }
        tortty_core::links::scan(&lines)
            .into_iter()
            .find(|l| l.covers(row, col))
            .map(|l| l.uri)
    }

    /// Hands the URI to the desktop (`xdg-open`): we do not speak portal or
    /// D-Bus, and a browser that is already running does not need another.
    fn open_link(&mut self, uri: &str) {
        // only the schemes a terminal link plausibly carries: the URI comes from
        // the child, and `xdg-open` would happily execute a `.desktop` handler
        // for anything else. The list is the scanner's, so an underline never
        // offers a click that ends here.
        if !tortty_core::links::opens(uri) {
            self.toast("Link refused", uri, Severity::Err);
            return;
        }
        match platform::open_uri(uri) {
            Ok(()) => self.toast("Opening", uri, Severity::Ok),
            Err(e) => self.toast("Link failed", &e.to_string(), Severity::Err),
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
    /// Jumps the view to the previous/next prompt (`M-R`). The prompt lands on
    /// the first grid row; at the ends nothing moves (a key that does something
    /// invisible reads as a key that did nothing).
    fn prompt_jump(&mut self, back: bool) {
        let t = &self.tabs[self.active];
        let top = t.term.scrolled as i64 - t.view_off as i64;
        // walking up looks strictly above the first row in sight; walking down
        // looks from that row on, so the prompt already at the top is not "next"
        let from = if back { top - 1 } else { top };
        let Some(line) = t.term.prompt_line(from, back) else {
            return;
        };
        let max = t.term.scrollback.len();
        let off = (t.term.scrolled as i64 - line).clamp(0, max as i64);
        #[allow(clippy::cast_sign_loss)] // clamped to `0..=max` above
        let off = off as usize;
        if off == self.tabs[self.active].view_off {
            return;
        }
        self.tabs[self.active].view_off = off;
        self.ask_frame();
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
        // the search bar owns the keyboard while it is open (`M-D`)
        if self.search_key(k) {
            return true;
        }
        // the context menu takes Escape, the arrows, Enter and Space, and any
        // other key closes it (`M-V`) — a menu that swallows the keyboard is a
        // dialog, and this is not one
        if self.menu_open.is_some() && self.menu_key(k) {
            return true;
        }
        // The shortcut table (`M-H`): one lookup, no `match` on the key here. A
        // chord nobody bound reaches the encoder (and the child).
        if let Some(chord) = k.chord() {
            if let Some(action) = self.opts.cfg.bindings.get(chord) {
                self.run_action(action);
                return true;
            }
        }
        // a real key returns to live; a bare modifier does not (otherwise the
        // Shift of Shift+PgUp already reset the view before scrolling)
        if self.tabs[self.active].view_off > 0
            && !matches!(
                k.named,
                Some(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super)
            )
        {
            self.tabs[self.active].view_off = 0;
        }
        false
    }

    /// What a bound chord does. One arm per action of the table; the args carry
    /// the number for the two families that take one.
    fn run_action(&mut self, action: Action) {
        let rows = self.tabs[self.active].term.rows as i64;
        match action {
            Action::Copy => self.copy(),
            Action::Paste => self.paste(),
            Action::NewTab => self.new_tab(),
            Action::CloseTab => self.close_tab(),
            Action::NewWorkspace => self.new_workspace(),
            Action::CloseWorkspace => {
                let i = self.ws_idx;
                self.close_workspace(i);
            },
            Action::PrevPrompt => self.prompt_jump(true),
            Action::NextPrompt => self.prompt_jump(false),
            Action::OpenConfig => self.open_config(),
            Action::ScrollPageUp => self.scroll_view(rows - 1),
            Action::ScrollPageDown => self.scroll_view(-(rows - 1)),
            // `Shift+Home` with nothing above does nothing: a key that moves
            // nothing is not a key that jumps
            Action::ScrollTop if self.tabs[self.active].term.scrollback.is_empty() => {},
            Action::ScrollTop => {
                self.scroll_view(self.tabs[self.active].term.scrollback.len() as i64);
            },
            Action::ScrollBottom => {
                self.scroll_view(-(self.tabs[self.active].term.scrollback.len() as i64));
            },
            Action::Search => self.open_search(),
            Action::SelectTab(n) => self.select_tab(n as usize - 1),
            Action::SelectWorkspace(n) => self.select_workspace(n as usize - 1),
            Action::WsPrev => self.select_workspace(ws_above(self.ws_idx)),
            Action::WsNext => self.select_workspace(ws_below(self.ws_idx, self.ws_count())),
            Action::WsMoveUp => {
                let i = self.ws_idx;
                self.move_workspace(i, ws_above(i));
            },
            Action::WsMoveDown => {
                let i = self.ws_idx;
                self.move_workspace(i, ws_below(i, self.ws_count()));
            },
        }
    }

    /// Motion: only matters to the app when it asked for `?1002`/`?1003`.
    fn on_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_px = (x, y);
        // the scrollbar owns the motion while it is being dragged
        if self.scroll_drag {
            self.scroll_to_y(y);
            return;
        }
        // a drag on the sidebar reorders the workspaces (`B49`), on the bar the
        // tabs (I10), and on the grid it is selection
        if self.card_press.is_some() {
            self.card_drag(x, y);
            return;
        }
        if self.bar_press.is_some() {
            self.bar_drag();
            return;
        }
        // Alt+drag is a local selection (a block) even when the child asked
        // for the mouse: the press decided the gesture (`select_input` set
        // `press` and `dragging`, which only it does), the motion only
        // extends it.
        if self.tabs[self.active].term.mouse_mode == 0 || self.tabs[self.active].dragging {
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
        // a press we consumed takes its own release with it: the child must not
        // see the end of a gesture it never saw the start of
        if state == ElementState::Released && std::mem::take(&mut self.press_ours) {
            return;
        }
        if state == ElementState::Pressed {
            self.blink_reset();
            self.press_ours = false;
            // the toast is drawn last, so it is above the chrome and the grid: a
            // press on it dismisses it and goes no further, with or without the
            // child asking for the mouse
            if self.toast_clicked(self.cursor_px.0, self.cursor_px.1) {
                self.press_ours = true;
                return;
            }
        }
        // `M-V`: the menu is above everything it covers, for both buttons — a
        // release must not reach a child whose press it never saw
        if self.menu_open.is_some() {
            if state == ElementState::Pressed && btn == crate::mouse::LEFT {
                match self.menu_layout().and_then(|m| {
                    m.row_at(self.chrome_metrics(), self.cursor_px.0, self.cursor_px.1)
                }) {
                    Some(i) => self.menu_action(i),
                    None => self.close_menu(),
                }
            } else if state == ElementState::Pressed {
                self.close_menu();
            }
            return;
        }
        // `status_show chip`: a left press on the chip dismisses it (`B47`).
        // `done` and `error` have no lifetime of their own, so without this the
        // chip would stay until the tab closed. It is chrome drawn over the last
        // row, so it takes the press even when the child holds the mouse — the
        // same rule the toast follows, and the reason the box only exists where
        // the renderer drew it.
        if btn == crate::mouse::LEFT
            && state == ElementState::Pressed
            && self.chip_clicked(self.cursor_px.0, self.cursor_px.1)
        {
            self.press_ours = true;
            return;
        }
        // a context menu: right button. Which one, and whether the press is even
        // ours, is the pointer's business (`open_menu`): over the grid the child
        // may have asked for the mouse, over the sidebar and the bar it never did
        if btn == crate::mouse::RIGHT && state == ElementState::Pressed && self.open_menu() {
            return;
        }
        // Alt+Left drags a local block selection instead of reporting to the
        // child (`B5` already gives Alt that meaning without the mouse). The
        // press decides the gesture: it starts one only when the child holds
        // no button, and its release ends it even with Alt already up.
        if btn == crate::mouse::LEFT
            && ((state == ElementState::Pressed && self.mods.alt_key() && self.held.is_none())
                || (state == ElementState::Released && self.tabs[self.active].dragging))
        {
            self.select_input(state, button);
            return;
        }
        if self.tabs[self.active].term.mouse_mode == 0 {
            self.select_input(state, button);
            return;
        }
        match state {
            ElementState::Pressed => {
                // a click on a workspace card or on the bar does not go to the
                // child either: it is chrome, and it behaves exactly as it does
                // without the mouse reported (`mc` is where the missing rename
                // showed up)
                if let Some(i) = self.card_at(self.cursor_px.0, self.cursor_px.1) {
                    self.press_card(i);
                    self.held = None;
                    return;
                }
                if let Some(i) = self.tab_at(self.cursor_px.0, self.cursor_px.1) {
                    self.press_tab(i);
                    self.held = None;
                    return;
                }
                // F19: the app draws live; a click on the history only makes
                // sense after going back to it
                self.tabs[self.active].view_off = 0;
                self.held = Some(btn);
                self.report_mouse(btn, true, false);
            },
            ElementState::Released => {
                // the sidebar drag ends here as well: this path is the one taken
                // when the child is holding the mouse
                self.card_press = None;
                self.card_walk = None;
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
        // the wheel on the sidebar walks the card list (`B42`): a workspace past
        // the bottom of the window has no other way in with the mouse
        if self.in_sidebar(self.cursor_px.0, self.cursor_px.1) {
            let ch = self.renderer.as_ref().map_or(19.0, |r| r.fonts.ch);
            let lines = match delta {
                MouseScrollDelta::LineDelta(_, y) => y,
                MouseScrollDelta::PixelDelta(p) => p.y as f32 / ch,
            };
            let n = Self::wheel_notches(&mut self.card_frac, &mut self.bar_wheel, lines);
            if n == 0 {
                return;
            }
            // up (positive) goes back one card, like the history above
            self.scroll_cards(-i64::from(n));
            return;
        }
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

    /// A press on a sidebar card: switches workspace (`I9`), and the second
    /// press on the same card inside 400 ms opens the rename (`I8`). One copy,
    /// used by both mouse paths — the card is chrome and does not go silent
    /// because the child asked for the mouse (`?1000/1002/1003`).
    fn press_card(&mut self, i: usize) {
        self.bar_press = None;
        // the drag starts from here (`B49`); it only moves after 3 px, so the
        // double click that renames is never a drag by accident
        self.card_press = Some((i, self.cursor_px));
        self.card_walk = None;
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
    }

    /// A press on the tab bar: switches tab (it does not select text) and keeps
    /// the press for the reorder drag (`I10`). Same rule as `press_card`.
    fn press_tab(&mut self, i: usize) {
        self.card_press = None;
        self.card_walk = None;
        self.bar_press = Some((i, self.cursor_px.0));
        self.select_tab(i);
        self.tabs[self.active].dragging = false;
        self.tabs[self.active].press = None;
        self.redraw();
    }

    /// Terminal mouse: press sets the anchor, drag becomes selection, release
    /// copies. A single click paints no cell and does not touch PRIMARY.
    fn select_input(&mut self, state: ElementState, button: winit::event::MouseButton) {
        use winit::event::MouseButton::{Left, Middle};
        match (state, button) {
            (ElementState::Pressed, Left) => {
                // the sidebar and the bar are chrome: they answer first, with
                // or without the child asking for the mouse
                if let Some(i) = self.card_at(self.cursor_px.0, self.cursor_px.1) {
                    self.press_card(i);
                    return;
                }
                if let Some(i) = self.tab_at(self.cursor_px.0, self.cursor_px.1) {
                    self.press_tab(i);
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
                // Alt draws a rectangle from the press (`B5`); a block has no
                // word and no line, so the click count is not consulted
                t.sel_block = self.mods.alt_key();
                let count = if t.sel_block {
                    1
                } else {
                    match t.click {
                        Some((prev, n, when)) if prev == at && when.elapsed().as_millis() < 400 => {
                            n % 3 + 1
                        },
                        _ => 1,
                    }
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
                // releasing on the bar ends the reorder drag (I10), and on the
                // sidebar the workspace one (`B49`)
                self.bar_press = None;
                self.card_press = None;
                self.card_walk = None;
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
        let (cw, ..) = self.cell_metrics();
        // where the press started and where we are: x pixel of tab i in the drag
        // (`M-Q`: the width is the bar's, bell included)
        let Some([_, width]) = self.chrome_layout().tab_slot(i) else {
            return;
        };
        let width = f64::from(width);
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

    /// Reorder drag on the sidebar (`B49`): while the button is held on a card,
    /// the card under the pointer is the place the dragged one takes — the list
    /// moves as the hand passes, which is the one feedback that says where it
    /// will land.
    ///
    /// Nothing is auto-scrolled except at the two ends of the column, where a
    /// row is the way to walk it (`B55`): the order only changes over a card,
    /// because a place has to be named by one.
    fn card_drag(&mut self, x: f64, y: f64) {
        let Some((i, (px, py))) = self.card_press else {
            return;
        };
        if (x - px).abs() <= 3.0 && (y - py).abs() <= 3.0 {
            return;
        }
        // `B55`: in the row at either end the list walks by itself, one card per
        // `CARD_WALK`, so a workspace can be moved to a place that is off the
        // column. The order does not change while it walks: a place is a card
        // under the pointer, and the row is on the card the walk covers — the
        // drop happens on the way back, over the card that came into sight.
        if let Some(dir) = self.card_band(x, y) {
            // crossing from one row to the other keeps the cadence going
            self.card_walk_dir = dir;
            self.card_walk.get_or_insert_with(std::time::Instant::now);
            return;
        }
        self.card_walk = None;
        // off the column (or past the last card) the drag does not move: the
        // pointer has to be over a card to mean a place
        let Some(dest) = self.card_at(x, y) else {
            return;
        };
        if dest == i {
            return;
        }
        self.move_workspace(i, dest);
        self.card_press = Some((dest, (px, py)));
        self.ask_frame();
    }

    /// Reorders the workspaces (`B49`): removes the one at `from` and inserts it
    /// at `to`; the number renumbers itself, because it is positional.
    ///
    /// The front is a storage detail (`tabs`/`active`), not a place in the
    /// order. So the whole list is gathered in logical order, moved as one
    /// vector by `reorder` and scattered back with the front in whatever slot
    /// the move gave it. Nothing is spawned and nothing is killed: a move is a
    /// move.
    fn move_workspace(&mut self, from: usize, to: usize) {
        let total = self.ws_count();
        if total == 0 || from >= total {
            return;
        }
        let to = to.min(total - 1);
        if from == to {
            return;
        }
        let cur = Workspace::take(self);
        let mut parked = std::mem::take(&mut self.parked).into_iter();
        let mut all: Vec<Option<Workspace>> = (0..total)
            .map(|j| {
                if j == self.ws_idx {
                    None
                } else {
                    parked.next()
                }
            })
            .collect();
        all[self.ws_idx] = Some(cur);
        let mut all: Vec<Workspace> = all
            .into_iter()
            .map(|w| w.expect("one workspace per logical index"))
            .collect();
        let mut front = self.ws_idx;
        reorder(&mut all, &mut front, from, to);
        let front_ws = all.remove(front);
        self.parked = all;
        self.ws_idx = front;
        front_ws.into_current(self);
        self.reveal_card(self.ws_idx);
        self.sync_title();
        self.redraw();
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
        let other =
            self.tabs[self.active]
                .term
                .anchor_at(self.tabs[self.active].view_off, row, col);
        let new = if self.tabs[self.active].sel_block {
            Span::block(anchor, other)
        } else {
            Span::new(anchor, other)
        };
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

    /// A remote clipboard write (`OSC 52`, `M-R`): the core decoded and
    /// validated, here it lands on the system clipboard. `c` is CLIPBOARD,
    /// `p` is PRIMARY — the same targets `Ctrl+Shift+C` and the release copy
    /// use, so a remote `ssh` copy behaves like a local one. Never the
    /// reverse: the query form is refused in the parser, so a remote can
    /// write but never read.
    fn set_clipboard(&mut self, op: tortty_core::term::ClipboardOp) {
        use tortty_core::term::ClipboardOp::{Clipboard, Primary};
        match op {
            Clipboard(text) => self.clip.set(&text),
            Primary(text) => self.clip.set_primary(&text),
        }
    }

    /// A file dropped on the window: the path goes into the child as a paste,
    /// quoted for the shell. Nothing else happens — no new tab, no `cd`: the
    /// terminal does not know what is running, and a program that is not a shell
    /// still gets a path it can read.
    fn drop_path(&mut self, path: &std::path::Path) {
        let txt = platform::shell_quote(&path.to_string_lossy());
        self.blink_reset();
        self.tabs[self.active].view_off = 0;
        self.write_paste(&txt);
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
                if pumped.card_alert {
                    self.parked[s].card_alert = true;
                }
                // no toast: a parked tab must not write over the foreground
                // `OSC 52` still lands: a remote copy in a parked tab copies
                if let Some(msg) = self.parked[s].tabs[j].term.take_refused() {
                    self.toast("Image refused", msg, Severity::Err);
                }
                if let Some(op) = self.parked[s].tabs[j].term.take_clipboard() {
                    self.set_clipboard(op);
                }
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
            if pumped.card_alert {
                self.card_alert = true;
            }
            if let Some(msg) = self.tabs[i].term.take_refused() {
                // `B3`: the protocol that asks has no error channel, so the
                // owner is told instead of a silence that reads as a broken image
                self.toast("Image refused", msg, Severity::Err);
            }
            if let Some(text) = pumped.toast {
                // it keeps the 8 s the badge floor assumes (`M-K`), and the shade
                // is the program's own state when it said one (`B47`)
                let (title, severity) = Self::notif_notice(&self.tabs[i].term);
                self.toast_for(&title, &text, severity, Some(NOTIF_FLOOR));
            }
            // `OSC 52`: the child set the clipboard (a remote `ssh` copies),
            // parked or front — the drain is per tab
            if let Some(op) = self.tabs[i].term.take_clipboard() {
                self.set_clipboard(op);
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
                let after = self.tabs[i].term.view_after_output(off, mark);
                if after != off {
                    // `B53`: who moved the reader. The window lines cannot say —
                    // a program that scrolls *between* frames leaves no trace
                    // there, and that is exactly the case under investigation.
                    let t = &self.tabs[i].term;
                    logln(&format!(
                        "{who} view {off}→{after} — scrolled {}→{} (sync {}→{}), gen {}→{}, \
                         hist {}→{}, open {}",
                        mark.scrolled(),
                        t.scrolled,
                        mark.redraw(),
                        t.scrolled_sync,
                        mark.generation(),
                        t.hist_gen,
                        mark.history(),
                        t.scrollback.len(),
                        t.sync_open().is_some(),
                    ));
                }
                self.tabs[i].view_off = after;
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

    /// Shows a notice on screen: a title, an optional detail and a shade. An
    /// equal message in a row becomes a count ("×3"), not a second box — whoever
    /// saves the file ten times does not want ten notices.
    ///
    /// A **different** notice does not overwrite the one being read: it waits in
    /// the queue and takes the screen when its turn comes.
    fn toast(&mut self, title: &str, detail: &str, severity: Severity) {
        self.toast_for(title, detail, severity, None);
    }

    /// Same, with the time on screen chosen by the caller. A notification keeps
    /// the 8 s the badge floor assumes, which is neither the receipt's 3 s nor
    /// the error's 10 s.
    fn toast_for(&mut self, title: &str, detail: &str, severity: Severity, life: Option<Duration>) {
        let mut t = Toast::new(title, detail, severity);
        if let Some(life) = life {
            t.life = life;
        }
        logln(&format!(
            "toast: {severity:?} {title}{} {detail}",
            if detail.is_empty() { "" } else { " —" }
        ));
        // The same words again — on screen or already waiting — only raise the
        // count; anything else waits behind whatever is being read. A repeat must
        // not restart the clock, or a program that repeats itself pins the box to
        // the screen (`B8`).
        let seen = self
            .toast
            .iter_mut()
            .chain(self.toast_queue.iter_mut())
            .find(|t| t.same(title, detail));
        if let Some(seen) = seen {
            seen.times += 1;
            self.ask_frame();
            return;
        }
        self.toast_queue.push(t);
        self.next_toast();
    }

    /// Gives the screen to the next notice when it is free. Called on arrival and
    /// when the visible one leaves.
    fn next_toast(&mut self) {
        if self.toast.is_some() {
            return;
        }
        if let Some(t) = (!self.toast_queue.is_empty()).then(|| self.toast_queue.remove(0)) {
            self.toast = Some(t);
        }
        self.ask_frame();
    }

    /// Takes the notice out of the screen (the clock ran out or the owner
    /// clicked) and gives the turn to whatever was waiting.
    fn drop_toast(&mut self) {
        if self.toast.take().is_none() {
            return;
        }
        self.next_toast();
    }

    /// A press on the `status_show chip` box dismisses it (`B47`).
    ///
    /// Three conditions decide whether there is a box to press, and they are the
    /// renderer's: `status_show chip` is on, the search bar is not holding the
    /// last row (`M-D` wins it), and the root record is not `idle`. The box itself
    /// is `chrome::StateChip` — the same one the drawing used, so what is seen is
    /// what is pressed.
    fn chip_clicked(&mut self, px: f64, py: f64) -> bool {
        if self.opts.cfg.status_show != tortty_core::config::StatusShow::Chip
            || self.search.is_some()
        {
            return false;
        }
        let Some((_, word, detail)) = self.tabs[self.active].term.status_chip() else {
            return false;
        };
        let chip = crate::chrome::StateChip { word, detail };
        if !chip.hit(self.chrome_metrics(), px, py) {
            return false;
        }
        self.tabs[self.active].term.dismiss_status();
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
        true
    }

    /// A click inside the box dismisses it. The box floats over the grid and is
    /// drawn last, so it is above everything: the press does not start a
    /// selection and does not reach the child.
    fn toast_clicked(&mut self, px: f64, py: f64) -> bool {
        let Some(rect) = self.renderer.as_ref().and_then(|r| r.toast_rect) else {
            return false;
        };
        if !inside_box(rect, px, py) {
            return false;
        }
        self.drop_toast();
        true
    }

    /// Toast for the renderer, with the count when there was a repeat.
    fn toast_text(&self) -> Option<ToastText> {
        let t = self.toast.as_ref()?;
        if t.expired() {
            return None;
        }
        let detail = if t.detail.is_empty() {
            String::new()
        } else {
            t.detail.clone()
        };
        let title = if t.times > 1 {
            format!("{} (×{})", t.title, t.times)
        } else {
            t.title.clone()
        };
        Some(ToastText {
            title,
            detail,
            severity: t.severity,
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
            // — the notice has to appear on screen with the first line, the
            // count, and the fact that what is running did not change.
            self.toast(
                "Configuration error",
                &notice_config(&errs, true),
                Severity::Err,
            );
            return;
        }
        logln(&format!("config: {} reloaded", path.display()));
        // The file is saved while the terminal runs, and the owner needs to know
        // it took effect — not only when the previous save was refused. The
        // family and the size are the proof that this is the file in force (the
        // python says "Configuration applied" with the same two facts).
        let applied = format!("{} {} pt", new.font_family, new.font_size_pt);
        self.toast("Configuration applied", &applied, Severity::Ok);
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
        self.clamp_cards();
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
        let search = self.search_view();
        let menu = self.menu_view();
        let frame = crate::render::Frame {
            tab: self.tabs[self.active].id,
            term: &self.tabs[self.active].term,
            view_off: self.tabs[self.active].view_off,
            sel: self.tabs[self.active].sel,
            blink,
            toast: toast.as_ref(),
            tabs: tabs.as_deref(),
            sidebar: sidebar.as_deref(),
            cards_off: self.card_off,
            links: self.mods.control_key(),
            search: search.as_ref(),
            menu: menu.as_ref(),
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
                t.sel = t.term.resize_keep(cols, rows, t.sel);
                if let Some(pty) = t.pty.as_mut() {
                    pty.resize(cols as u16, rows as u16, cell);
                }
                t.view_off = 0;
            }
        }
        if !self.spawned {
            if let Some(wake) = self.wake.clone() {
                self.spawned = true;
                if let Some(layout) = self.restore.take() {
                    self.restore_layout(&layout, &wake, cols, rows);
                } else {
                    self.spawn_shell(cols, rows, wake);
                }
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
                    "rows_built": self.renderer.as_ref().map_or(0, |r| r.rows_built),
                    "cells_built": self.renderer.as_ref().map_or(0, |r| r.cells_built),
                    // kitty images: without this there is no way to check the
                    // placement without looking at the screen
                    "images": self.tabs[self.active].term.graphics.images().len(),
                    // `B2`: an animation is a clock, so a probe has to be able to
                    // read which frame is on screen without watching pixels
                    "anims": self.tabs[self.active]
                        .term
                        .graphics
                        .images()
                        .values()
                        .filter(|i| i.frame_count() > 1)
                        .map(|i| serde_json::json!({
                            "id": i.id,
                            "frames": i.frame_count(),
                            "current": i.current(),
                            "mode": i.anim.mode,
                            "gap": i.gap_ms(i.current()),
                        }))
                        .collect::<Vec<_>>(),
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
                    // the notice on screen and how many are waiting behind it:
                    // a probe checking the box does not have to read pixels
                    "toast": self.toast.as_ref().filter(|t| !t.expired()).map(|t| {
                        serde_json::json!({
                            "title": t.title,
                            "detail": t.detail,
                            "severity": format!("{:?}", t.severity),
                            "times": t.times,
                        })
                    }),
                    "toast_queue": self.toast_queue.len(),
                    "placements": self.tabs[self.active]
                        .term
                        .graphics
                        .placements()
                        .iter()
                        .map(|p| {
                            serde_json::json!({
                                "id": p.image_id,
                                "pid": p.id,
                                "z": p.z,
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
                    // sidebar scroll (B42): a probe can drive the wheel and check
                    // the list without looking at the screen
                    "cards_off": self.card_off,
                    "cards_visible": self.cards_visible(),
                    // badges and card alerts (`B8`): the tab keeps its own and
                    // the card is the workspace's, so a probe can tell them apart
                    "tabs_alert": self.tabs.iter().map(Tab::notif_pending).collect::<Vec<_>>(),
                    "cards_alert": self.wscards().iter().map(|c| c.alert).collect::<Vec<_>>(),
                    "tabs_progress": self.tabs.iter()
                        .map(|t| t.term.progress.map(|p| serde_json::json!([p.state, p.pct])))
                        .collect::<Vec<_>>(),
                    "search": self.search.as_ref().map(|s| {
                        serde_json::json!([s.query, s.at, s.hits.len()])
                    }),
                    // the open context menu (`B64`): which rows, which card or
                    // tab it acts on, and where the box landed in window px —
                    // a probe can check the anchor without reading pixels
                    "menu": self.menu_open.as_ref().and_then(|state| {
                        let m = self.menu_layout()?;
                        let metrics = self.chrome_metrics();
                        let (pad, side, bar) = (
                            f64::from(metrics.pad),
                            f64::from(metrics.side),
                            f64::from(metrics.bar),
                        );
                        Some(serde_json::json!({
                            "items": m.items.iter().map(|i| i.label()).collect::<Vec<_>>(),
                            "enabled": state.enabled,
                            "subject": state.subject,
                            "at": [state.at.0, state.at.1],
                            "rect": [
                                f64::from(m.rect[0]) + pad + side,
                                f64::from(m.rect[1]) + pad + bar,
                                m.rect[2],
                                m.rect[3],
                            ],
                        }))
                    }),
                    "status": self.tabs[self.active]
                        .term
                        .status
                        .records()
                        .map(|(id, r)| serde_json::json!([
                            id,
                            r.state.name(),
                            r.progress,
                            r.app,
                            r.msg,
                        ]))
                        .collect::<Vec<_>>(),
                    "marks": self.tabs[self.active]
                        .term
                        .marks
                        .all()
                        .iter()
                        .rev()
                        .take(8)
                        .rev()
                        .map(|m| serde_json::json!([m.line, m.kind as u8, m.status]))
                        .collect::<Vec<_>>(),
                    "errors": self.cfg_errors,
                    "xid": self.xid(),
                    "config": self.opts.cfg_path.as_ref().map_or_else(String::new, |p| p.display().to_string()),
                    // which template of the defaults applied: the state verb is how
                    // a probe answers "why is this chord bound here"
                    "config_os": tortty_core::config::Os::current().name(),
                    // where the state is written: a probe reads this instead of
                    // guessing which of the three shapes to look for
                    "status_show": match self.opts.cfg.status_show {
                        tortty_core::config::StatusShow::Chip => "chip",
                        tortty_core::config::StatusShow::Tab => "tab",
                        tortty_core::config::StatusShow::None => "none",
                    },
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
            // what a file dropped on the window does, without needing a drag
            // source: the same path the XDND event takes (`drop_path`)
            "drop" => match req.get("path").and_then(|v| v.as_str()) {
                Some(path) => {
                    self.drop_path(std::path::Path::new(path));
                    debugsock::ok(serde_json::json!({}))
                },
                None => debugsock::err("drop needs path (string)"),
            },
            // the text Ctrl+Shift+C would put in the clipboard, without the
            // clipboard: the shape of a selection (a block, `B5`) is only visible
            // in what it copies
            "sel" => debugsock::ok(serde_json::json!({
                "text": self.selection_text(),
                "block": self.tabs[self.active].sel.is_some_and(|s| s.is_block()),
            })),
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
        // `B46`: only the window that opened the session owns the state, and only
        // the owner restores the session and writes the file. The kernel says who
        // that is (a `flock` held for the life of the process), so a second
        // window is a scratch window instead of a second owner — this is what
        // stops two runs from overwriting each other's `window.json`.
        //
        // Three runs never even try the lock, because they are not the session:
        // a grid on the command line (`--cols`/`--rows`, a measurement), a
        // command of its own (`--cmd`, a probe), and `restore_state = none`,
        // which is the owner saying "this terminal has no state".
        let asks_for_state = !self.opts.grid_explicito
            && self.opts.cmd.is_none()
            && self.opts.cfg.restore_state != RestoreState::None;
        self.owner = asks_for_state.then(crate::state::lock).flatten();
        // `B61`: the lock is one decision (who restores and writes the session),
        // the window's size and place are another — a second window opens where
        // the last one was, not at the first-run default.
        let (read_geometry, read_layout) = state_use(
            self.opts.grid_explicito,
            self.opts.cfg.restore_state,
            self.owner.is_some(),
        );
        self.remember_geom = read_geometry;
        if !read_geometry {
            // a measurement wants the grid it asked for; `restore_state = none`
            // wants the first-run default
            let size = if self.opts.grid_explicito {
                default
            } else {
                crate::state::DEFAULT
            };
            self.geom_normal = size;
            return (size.0, size.1, None, false);
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
        let maximized = remembered.as_ref().is_some_and(|s| s.geometry.maximized);
        // the layout of the last session (`M-I`): only the owner has one, and a
        // probe (`--cmd`) opens a single tab of its own
        self.restore = remembered
            .as_ref()
            .filter(|_| read_layout)
            .and_then(|s| s.layout.clone());
        // J2/J4: with no state (or invalid state: monitor unplugged, garbage in
        // the file) it opens at the 980x640 default, not at the grid size —
        // whoever wants an exact grid asks for `--cols/--rows`, which turns the
        // remembering off. Invalid remembered geometry falls back to this same
        // default.
        let geom = remembered
            .and_then(|s| {
                let g = s.geometry;
                crate::state::fit(g.x, g.y, g.w, g.h, &monitors)
            })
            .unwrap_or_default();
        self.geom_normal = (geom.w.max(1), geom.h.max(1));
        // J6: only X11 accepts a window position; on Wayland it does not exist
        let pos = geom.x.zip(geom.y).filter(|_| platform::backend_x11());
        (geom.w.max(1), geom.h.max(1), pos, maximized)
    }

    /// Saves position, normal size, maximized state (`J2`–`J6`) and the layout of
    /// the session (`M-I`). Failing here cannot get in the way of closing —
    /// `state::save` already swallows I/O errors.
    ///
    /// The layout arrives as an argument, not from `session_layout()`, because it
    /// is not the same thing read at two moments of the closing: the cwd of a tab
    /// comes from `/proc/<pid>/cwd`, and `last_words` is what ends those children
    /// — taken afterwards, every tab is saved without a directory (`B51`). Making
    /// it a parameter is what leaves the order to the compiler: there is no way to
    /// call this before the layout exists, and `exiting` reads it first.
    fn save_geometry(&mut self, layout: crate::state::Layout) {
        if self.owner.is_none() {
            // not the owner: a measurement, a probe, `restore_state = none`, or a
            // second window (`B46`). Writing here is what used to erase the layout
            // of a session that was still open.
            return;
        }
        let maximized = self.window.as_ref().is_some_and(|w| w.is_maximized());
        let (x, y) = match self.geom_pos.filter(|_| platform::backend_x11()) {
            Some((x, y)) => (Some(x), Some(y)),
            None => (None, None), // Wayland: no position to remember (J6)
        };
        // the owner writes what it opened: `--cmd` and `none` never get here, and a
        // second window does not either, so this tab list is the session's own
        let layout = Some(layout);
        crate::state::save(&crate::state::State {
            geometry: crate::state::Geometry {
                x,
                y,
                w: self.geom_normal.0,
                h: self.geom_normal.1,
                maximized,
            },
            layout,
        });
        if self.opts.cfg.restore_state == RestoreState::Content {
            crate::state::save_snapshots(&self.session_snapshots());
        } else {
            crate::state::save_snapshots(&[]); // the key turned the content off
        }
    }

    /// The content of every tab, ready for `state::save_snapshots` (`B45`).
    fn session_snapshots(&self) -> Vec<(usize, usize, Vec<u8>)> {
        let mut out = Vec::new();
        let mut parked = self.parked.iter();
        for j in 0..self.ws_count() {
            let tabs: &[Tab] = if j == self.ws_idx {
                &self.tabs
            } else if let Some(w) = parked.next() {
                &w.tabs
            } else {
                continue;
            };
            for (k, t) in tabs.iter().enumerate() {
                out.push((j, k, t.term.snapshot()));
            }
        }
        out
    }

    /// The workspaces and tabs as they are now, for the next run (`M-I`).
    fn session_layout(&self) -> crate::state::Layout {
        use crate::state::{Layout, WorkspaceMemory};
        let memory = |name: &str, tabs: &[Tab]| WorkspaceMemory {
            name: name.to_string(),
            cwds: tabs
                .iter()
                .map(|t| t.cwd().map(|p| p.display().to_string()))
                .collect(),
        };
        let mut workspaces = Vec::with_capacity(self.ws_count());
        let mut parked = self.parked.iter();
        for j in 0..self.ws_count() {
            if j == self.ws_idx {
                workspaces.push(memory(&self.ws_name, &self.tabs));
            } else if let Some(w) = parked.next() {
                workspaces.push(memory(&w.name, &w.tabs));
            }
        }
        Layout {
            workspaces,
            ws: self.ws_idx,
            tab: self.active,
        }
    }

    /// Reopens the workspaces and tabs of the last session (`M-I`).
    ///
    /// The one in front is the last one **built** — it is the one that lives in
    /// `tabs`/`active`, and building it anywhere else would mean parking it again
    /// — but it keeps the number the file gave it. A directory that no longer
    /// exists is not fatal: that tab opens on the shell's default, like a new one.
    fn restore_layout(
        &mut self,
        l: &crate::state::Layout,
        wake: &std::sync::Arc<dyn Fn() + Send + Sync>,
        cols: usize,
        rows: usize,
    ) {
        let (order, front) = restore_order(l.workspaces.len(), l.ws);
        for (i, j) in order.iter().copied().enumerate() {
            if i > 0 {
                let cur = Workspace::take(self);
                self.parked.push(cur);
                self.ws_idx = self.parked.len();
                let mut tab = Tab::new(cols, rows);
                tab.term.scrollback_limit = self.opts.scrollback;
                self.tabs = vec![tab];
                self.active = 0;
            }
            let w = &l.workspaces[j];
            // the default name of the runs before `B43` was `workspace {n}`:
            // reading it back would put the word on the card again, so it is
            // dropped as if the workspace had never been renamed
            self.ws_name = drop_legacy_default(&w.name, j);
            for (k, cwd) in w.cwds.iter().enumerate() {
                if k > 0 {
                    let mut tab = Tab::new(cols, rows);
                    tab.term.scrollback_limit = self.opts.scrollback;
                    self.tabs.push(tab);
                }
                // a path that is gone falls back to the default directory
                let dir = cwd
                    .as_ref()
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_dir());
                self.replay_snapshot(j, k);
                self.spawn_tab(k, cols, rows, wake.clone(), dir.as_deref(), None);
            }
            self.active = if j == front {
                l.tab.min(w.cwds.len().saturating_sub(1))
            } else {
                0
            };
        }
        // the rebuild leaves `ws_idx` at the last position, which turned the
        // workspace in front into the last one and shifted every other left on
        // the next run (`B44`)
        self.ws_idx = front;
    }

    /// Puts the content of the previous run back into tab `tab` of workspace
    /// `ws` (`restore_state = content`, `B45`).
    ///
    /// It runs **before** the shell exists, so what the shell writes lands below
    /// the marker: above it is a photograph, below it is this run. The marker is
    /// the whole contract — a restored screen that looks live would be a lie.
    fn replay_snapshot(&mut self, ws: usize, tab: usize) {
        if self.opts.cfg.restore_state != RestoreState::Content {
            return;
        }
        let Some(bytes) = crate::state::load_snapshot(ws, tab) else {
            return;
        };
        let t = &mut self.tabs[tab];
        t.term.feed(&bytes);
        let rows = t.term.rows;
        // bottom left, one scroll for the marker to have a line of its own, and
        // the prompt of the new shell takes the line after it
        // decoration, not content (`B48`): the marker is drawn on the screen and
        // stays out of the next snapshot, or every restart would save the marker of
        // the previous one instead of what the tab held
        t.term.feed_chrome(
            format!(
                "\x1b[0m\x1b[{rows};1H\r\n\x1b[2m── content restored from the previous run ──\x1b[0m\r\n"
            )
            .as_bytes(),
        );
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
        // the one being left keeps its **place**: appended at the end of the
        // list, it would drag every workspace after it one position to the left
        // — which is what made the order of a `Super+N` look random (`B54`)
        self.ws_idx = park_front(&mut self.parked, cur, self.ws_idx);
        let mut tab = Tab::new(cols, rows);
        tab.term.scrollback_limit = self.opts.scrollback;
        self.tabs = vec![tab];
        self.active = 0;
        self.ws_name.clear();
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
        // not terminal payload. An empty result is the "never renamed" state,
        // and it is what clearing the field means (`B43`): the card goes back to
        // the automatic title instead of refusing the key
        let new = tortty_core::term::sanitize_title(&text).trim().to_string();
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
    /// Opens the search bar over the scrollback (`M-D`). It starts from the
    /// last query, which is what makes `Ctrl+Shift+F` useful the second time.
    fn open_search(&mut self) {
        let mut s = self.search.take().unwrap_or_default();
        s.run(&self.tabs[self.active].term);
        self.show_current(&s);
        self.search = Some(s);
        self.search_gen += 1;
        self.redraw();
    }

    fn close_search(&mut self) {
        self.search = None;
        self.search_gen += 1;
        self.redraw();
    }

    /// The keys of the search bar: text goes into the query, `Enter` walks the
    /// matches, `Esc` closes. `true` = the child does not see the key.
    fn search_key(&mut self, k: &KeyIn<'_>) -> bool {
        if self.search.is_none() {
            return false;
        }
        match k.named {
            Some(NamedKey::Escape) => {
                self.close_search();
                return true;
            },
            Some(NamedKey::Enter) => {
                let back = k.mods.shift_key();
                if let Some(s) = self.search.as_mut() {
                    if let Some(hit) = s.step(back) {
                        let _ = hit;
                    }
                    let s = s.clone();
                    self.show_current(&s);
                }
                self.search_gen += 1;
                self.redraw();
                return true;
            },
            Some(NamedKey::Backspace) => {
                let term = &self.tabs[self.active].term;
                let alive = match self.search.as_mut() {
                    Some(s) => s.pop(term),
                    None => false,
                };
                if alive {
                    self.after_query();
                } else {
                    self.close_search();
                }
                return true;
            },
            // The space bar arrives as a named key, never as text, so the
            // catch-all below would swallow it and nobody could search for
            // `error: ` or `foo bar`. Inside the bar it is a character.
            Some(NamedKey::Space)
                if !(k.mods.control_key() || k.mods.alt_key() || k.mods.super_key()) => {},
            Some(_) => {
                // Tab, arrows and the like stay with the child? No: the bar is
                // modal, an arrow would move the cursor in the program behind it.
                let _ = k;
                return true;
            },
            None => {},
        }
        // a printable character goes into the query; a chord does not (Ctrl+C
        // must still interrupt the program)
        if k.mods.control_key() || k.mods.alt_key() || k.mods.super_key() {
            return false;
        }
        // a named key that got this far is the space bar, which carries no text
        let space = if k.named == Some(NamedKey::Space) {
            Some(" ")
        } else {
            None
        };
        let Some(text) = k.text.or(k.character).or(space) else {
            return true;
        };
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() {
            return true;
        }
        let term = &self.tabs[self.active].term;
        if let Some(s) = self.search.as_mut() {
            for c in clean.chars() {
                s.push(c, term);
            }
        }
        self.after_query();
        true
    }

    /// After the query changed: the view goes to the first match, and the rows
    /// are rebuilt.
    fn after_query(&mut self) {
        let s = self.search.clone();
        if let Some(s) = s.as_ref() {
            self.show_current(s);
        }
        self.search_gen += 1;
        self.redraw();
    }

    /// Sends the view to the current match: the line lands on the first row, so
    /// the match is never half-hidden at the top.
    fn show_current(&mut self, s: &tortty_core::search::Search) {
        let Some(hit) = s.current() else {
            return;
        };
        let t = &self.tabs[self.active];
        let off = (t.term.scrolled as i64 - hit.line).clamp(0, t.term.scrollback.len() as i64);
        // clamped to `0..=len` above
        #[allow(clippy::cast_sign_loss)]
        let off = off as usize;
        self.tabs[self.active].view_off = off;
    }

    /// What the renderer draws for the search (`M-D`): the matches that are in
    /// sight, in **view rows**, and the bar's text.
    fn search_view(&self) -> Option<crate::render::SearchView> {
        let s = self.search.as_ref()?;
        let t = &self.tabs[self.active];
        let rows = t.term.rows;
        let cols = t.term.cols;
        let top = t.term.scrolled as i64 - t.view_off as i64;
        let mut hits = Vec::new();
        for (i, h) in s.hits.iter().enumerate() {
            let row = h.line - top;
            if row < 0 || row >= rows as i64 || h.col >= cols {
                continue;
            }
            #[allow(clippy::cast_sign_loss)]
            let row = row as usize;
            hits.push((row, h.col, h.len, i == s.at));
        }
        // the count chip only exists once there is a query: `[ ]` next to an
        // empty field would say nothing
        let count = (!s.query.is_empty()).then(|| s.count());
        Some(crate::render::SearchView {
            hits,
            query: s.query.clone(),
            count,
        })
    }

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
        // entering clears the CARD and nothing else (`B8`): the tab that is now
        // in front was visited and acknowledges its own badge, and every other
        // tab keeps its own until somebody visits it — the card cannot speak for
        // them
        self.card_alert = false;
        self.visit_active();
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

    /// The workspace in front lost its tabs: the one that takes the place is the
    /// **next** in the logical order, or the previous when it was the last (or
    /// closes the app, if none is left) — I20, `B62`.
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
        // `B62`: the list closes around the hole instead of collapsing to the
        // top — the workspace that was at `ws_idx + 1` takes the slot and keeps
        // the number, which is the rule the tabs already follow
        let Some(slot) = successor_slot(self.ws_idx, self.parked.len()) else {
            self.exiting = true;
            return;
        };
        let new = self.parked.remove(slot);
        new.into_current(self);
        self.ws_idx = slot;
        self.after_ws_change();
    }

    /// After touching workspaces: the bar may appear/disappear (its height
    /// depends on the number of tabs *of this* workspace), so the grid is redone.
    fn after_ws_change(&mut self) {
        if let Some(w) = self.window.as_ref() {
            let s = w.inner_size();
            self.apply_size(s.width, s.height);
        }
        // the card of the workspace now in front has to be in sight (`B42`): the
        // keyboard can enter one the column is not showing
        self.reveal_card(self.ws_idx);
        self.sync_title();
        self.redraw();
    }

    /// Ctrl+Shift+F2: opens the config file in the editor, in a new tab (P33).
    /// `VISUAL`/`EDITOR` beat `vim`; the directory is created because the editor
    /// only creates the file on save.
    fn open_config(&mut self) {
        let Some(path) = self.opts.cfg_path.clone() else {
            self.toast("Configuration", "no path (no HOME)", Severity::Err);
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let editor = platform::editor();
        let cmd = format!(
            "{} {}",
            editor,
            platform::shell_quote(&path.to_string_lossy())
        );
        self.new_tab_cmd(Some(&cmd));
    }

    /// The notice a notification becomes: title, text and shade. `OSC 7501`
    /// brings the state and the `app` that asked, so a `blocked` wears yellow
    /// and an `error` red — the same three tones as the tab's bar. A plain
    /// `OSC 9`/`777` notification carries no state and stays the receipt green.
    fn notif_notice(term: &tortty_core::Term) -> (String, Severity) {
        use tortty_core::status::State;
        let severity = match term.notif_state {
            Some(State::Blocked) => Severity::Warn,
            Some(State::Error) => Severity::Err,
            _ => Severity::Ok,
        };
        let title = term
            .notif_title
            .clone()
            .unwrap_or_else(|| "Notification".to_string());
        (title, severity)
    }

    /// Switches tab (Alt+N). Out of range, does nothing.
    /// The tab in front was visited: show what it has to say and acknowledge
    /// the badge (`M-K`). The content comes **now**, not when it arrived (`B8`):
    /// a notification that waited in a parked tab must still be readable when
    /// someone gets there.
    fn visit_active(&mut self) {
        if self.tabs[self.active].notif_pending() {
            let text = self.tabs[self.active].term.notif_text.clone();
            if !text.is_empty() {
                let (title, severity) = Self::notif_notice(&self.tabs[self.active].term);
                self.toast_for(&title, &text, severity, Some(NOTIF_FLOOR));
            }
        }
        self.tabs[self.active].ack_tab();
    }

    fn select_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.active {
            return;
        }
        self.active = i;
        self.visit_active();
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

impl KeyIn<'_> {
    /// The chord this event is, when the key is one the config can name: what
    /// the shortcut table (`M-H`) is looked up with.
    fn chord(&self) -> Option<tortty_core::keymap::Chord> {
        use tortty_core::keymap::{Chord, Id, ALT, CTRL, SHIFT, SUPER};
        // a digit under Ctrl/Alt/Super is read from the **physical** key: on
        // ABNT2 `Ctrl+Shift+2` arrives as `@` and `Ctrl+Shift+6` as `¨`, and the
        // shortcut that is meant is still 2 and 6 (I16)
        let shortcut = self.mods.control_key() || self.mods.alt_key() || self.mods.super_key();
        let id = match (self.digit.filter(|_| shortcut), self.named, self.character) {
            (Some(d), _, _) => Id::Char(char::from(b'0' + d as u8)),
            (None, Some(n), _) => Id::Named(named_name(n)?),
            // a character key: `Character("T")` with shift becomes `t`, so a
            // `shift+t` chord matches it
            (None, None, Some(s)) => Id::Char(s.chars().next()?.to_ascii_lowercase()),
            _ => return None,
        };
        let mut mods = tortty_core::keymap::NONE;
        for (bit, on) in [
            (CTRL, self.mods.control_key()),
            (ALT, self.mods.alt_key()),
            (SHIFT, self.mods.shift_key()),
            (SUPER, self.mods.super_key()),
        ] {
            if on {
                mods |= bit;
            }
        }
        Some(Chord { id, mods })
    }
}

/// The name of a named key in the config's vocabulary (`up`, `pgup`, `f5`).
fn named_name(n: NamedKey) -> Option<&'static str> {
    Some(match n {
        NamedKey::Enter => "enter",
        NamedKey::Escape => "esc",
        NamedKey::Tab => "tab",
        NamedKey::Backspace => "backspace",
        NamedKey::Space => "space",
        NamedKey::Insert => "insert",
        NamedKey::Delete => "delete",
        NamedKey::Home => "home",
        NamedKey::End => "end",
        NamedKey::PageUp => "pgup",
        NamedKey::PageDown => "pgdn",
        NamedKey::ArrowUp => "up",
        NamedKey::ArrowDown => "down",
        NamedKey::ArrowLeft => "left",
        NamedKey::ArrowRight => "right",
        other => return f_name(other),
    })
}

/// F1..F24, the ones the config names.
fn f_name(n: NamedKey) -> Option<&'static str> {
    use NamedKey::{
        F1, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F2, F20, F21, F22, F23, F24, F3, F4,
        F5, F6, F7, F8, F9,
    };
    Some(match n {
        F1 => "f1",
        F2 => "f2",
        F3 => "f3",
        F4 => "f4",
        F5 => "f5",
        F6 => "f6",
        F7 => "f7",
        F8 => "f8",
        F9 => "f9",
        F10 => "f10",
        F11 => "f11",
        F12 => "f12",
        F13 => "f13",
        F14 => "f14",
        F15 => "f15",
        F16 => "f16",
        F17 => "f17",
        F18 => "f18",
        F19 => "f19",
        F20 => "f20",
        F21 => "f21",
        F22 => "f22",
        F23 => "f23",
        F24 => "f24",
        _ => return None,
    })
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
        // `SIGUSR1` (dump) and `SIGTERM` (clean exit). Which trigger the system
        // has instead is `platform`'s business.
        platform::install_triggers();
        let (w, h, pos, maximized) = self.geometry_initial(event_loop, &fonts);
        let mut attrs = Window::default_attributes()
            .with_title("tortty")
            .with_inner_size(PhysicalSize::new(w, h))
            .with_maximized(maximized);
        if let Some((x, y)) = pos {
            attrs = attrs.with_position(PhysicalPosition::new(x, y));
        }
        // K19: the window class `StartupWMClass` looks for is set by the
        // platform, not inherited from the executable name.
        let attrs = platform::name_window(attrs);
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
                    // whoever opened the surface has to know it is standing, and
                    // standing is a warning: anything on the machine can drive
                    // this window through it
                    self.toast(
                        "Security warning",
                        &format!("debug socket enabled: {}", s.path()),
                        Severity::Warn,
                    );
                    self.debug = Some(s);
                },
                Err(e) => {
                    // whoever opened the debug surface has to know it is not up,
                    // on stderr and in the toast
                    eprintln!("tortty: debug socket: {e}");
                    self.toast("Debug socket", &e, Severity::Err);
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
                .map_or_default(|r| r.fonts.family.clone()),
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
            // a file dropped on the window becomes its path, written into the
            // child like a paste: it is what the owner means when dragging a file
            // onto a terminal, and the quoting is ours because the path is not
            WindowEvent::DroppedFile(path) => {
                self.drop_path(&path);
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
            self.toast(
                "Configuration error",
                &notice_config(&errs, false),
                Severity::Err,
            );
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

        // the handler only raises the flag, whoever acts is here
        let (dump, finish) = platform::take_triggers();
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

        // an expired toast leaves the screen (nobody else would take it out),
        // and the turn goes to whatever was waiting in the queue
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.drop_toast();
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

        // `B55`: a drag resting in the band below the cards walks the list
        if let Some(t) = self.card_walk_tick(now) {
            when = when.min(t);
        }
        // `B2`: an animation frame is due
        if let Some(t) = self.anim_tick(now) {
            when = when.min(t);
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
        // the layout goes first: it is read out of `/proc/<pid>/cwd`, and
        // `last_words` is what takes the children away (`B51`)
        let layout = self.session_layout();
        self.last_words();
        self.save_geometry(layout);
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

/// How long the children get to say goodbye before the window is gone.
const FAREWELL_MS: u64 = 250;

impl App {
    /// Closing the window is a hangup, and the last words of the child belong to
    /// the picture (`B48`): `pi` leaves the alt screen and prints
    /// `To resume this session: pi --session …`, `cargo` prints its last line.
    ///
    /// The snapshot used to be written **before** the child had been told
    /// anything, so a farewell could never be in it — not "sometimes", always.
    /// Now the hangup goes out first and the ptys are drained for a quarter of a
    /// second (or until every child is gone), which is what puts those bytes in
    /// the grid before `save_geometry` reads it.
    fn last_words(&mut self) {
        let writes_content =
            self.opts.cfg.restore_state == RestoreState::Content && self.opts.cmd.is_none();
        if !writes_content {
            return; // nothing is written, nothing to wait for
        }
        for t in &mut self.tabs {
            if let Some(p) = t.pty.as_mut() {
                p.hangup();
            }
        }
        for w in &mut self.parked {
            for t in &mut w.tabs {
                if let Some(p) = t.pty.as_mut() {
                    p.hangup();
                }
            }
        }
        let deadline = std::time::Instant::now() + Duration::from_millis(FAREWELL_MS);
        loop {
            self.drain();
            if std::time::Instant::now() >= deadline || self.children_gone() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// True when every pty of the session has no child left.
    fn children_gone(&mut self) -> bool {
        let front = self
            .tabs
            .iter_mut()
            .all(|t| t.pty.as_mut().is_none_or(Pty::child_exited));
        let parked = self.parked.iter_mut().all(|w| {
            w.tabs
                .iter_mut()
                .all(|t| t.pty.as_mut().is_none_or(Pty::child_exited))
        });
        front && parked
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

/// cwd for the next tab: `/proc` answers where the program in front really is,
/// `OSC 7` only where the shell thinks it is — so `/proc` wins and the shell's
/// word is the last resort (a container, a stricter mount, a process tree the
/// walk missed). A path that is no longer a directory is not a cwd: spawning a
/// tab there would fail after the decision.
/// The directory the tab is in: the cwd of what runs in front of its pty, with
/// the `OSC 7` the program announced as the fallback (`B39` will make that the
/// first candidate).
///
/// The `/proc` half only answers **while the child lives**, which is why
/// `App::exiting` reads the layout before the hangup (`B51`): a tab read after
/// the farewell has no directory left to give.
fn tab_cwd(pty: Option<&Pty>, from_osc: Option<&str>) -> Option<std::path::PathBuf> {
    let from_proc = pty.and_then(Pty::child_pid).and_then(platform::child_cwd);
    prefer_cwd(from_proc, from_osc)
}

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

/// The order in which a saved layout is rebuilt: the workspaces that are not in
/// front, in the file's order, and the one in front **last** (it is the one that
/// lives in `tabs`/`active`; the others go to `parked`). The second value is the
/// logical index the front keeps afterwards — being built last is a matter of
/// where it is stored, and it must not renumber the session (`B44`).
fn restore_order(total: usize, ws: usize) -> (Vec<usize>, usize) {
    let front = ws.min(total.saturating_sub(1));
    let order = (0..total).filter(|j| *j != front).chain([front]).collect();
    (order, front)
}

/// A name remembered by a run before `B43` was not a name: the card was born
/// saying `workspace {n}`. Reading that back would paint the word on the card
/// again, and there is no way to tell it from an owner who typed it — so the
/// one string the program itself used to write is dropped, and only for the
/// workspace whose number it names.
fn drop_legacy_default(name: &str, idx: usize) -> String {
    if name == format!("workspace {}", idx + 1) {
        String::new()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A receipt, a warning and an error do not share the time on screen: the
    /// first is read in a glance, the others carry something to act on.
    #[test]
    fn an_error_stays_longer_than_a_receipt() {
        let ok = Toast::new("Opening", "https://x", Severity::Ok);
        let err = Toast::new("Link failed", "xdg-open: no method", Severity::Err);
        let warn = Toast::new("Security warning", "debug socket enabled", Severity::Warn);
        assert_eq!(ok.life, TOAST_OK);
        assert_eq!(err.life, TOAST_LONG);
        assert_eq!(warn.life, TOAST_LONG);
        assert!(ok.life < warn.life);
    }

    /// Two notices are the same only when both lines are: the title alone would
    /// merge "link refused" of two different links into one count.
    #[test]
    fn the_same_notice_needs_both_lines() {
        let t = Toast::new("Link refused", "http://a", Severity::Err);
        assert!(t.same("Link refused", "http://a"));
        assert!(!t.same("Link refused", "http://b"), "another link");
        assert!(!t.same("Link failed", "http://a"), "another title");
    }

    /// The click lands inside the box the renderer reported and nowhere else: the
    /// right and bottom edges belong to the grid, so a press there still selects.
    #[test]
    fn a_click_lands_only_inside_the_box() {
        let toast = [100.0, 40.0, 200.0, 30.0];
        assert!(inside_box(toast, 100.0, 40.0), "the top-left corner");
        assert!(inside_box(toast, 150.0, 55.0), "the middle");
        assert!(!inside_box(toast, 99.0, 55.0), "one pixel to the left");
        assert!(!inside_box(toast, 300.0, 55.0), "the right edge");
        assert!(!inside_box(toast, 150.0, 70.0), "the bottom edge");
    }

    #[test]
    fn a_legacy_workspace_name_is_dropped_on_restore() {
        // what the old default wrote, for the workspace it numbered
        assert_eq!(drop_legacy_default("workspace 3", 2), String::new());
        // the number is the workspace's, so another one's default is a name
        assert_eq!(drop_legacy_default("workspace 3", 1), "workspace 3");
        // and a real name survives untouched
        assert_eq!(drop_legacy_default("build", 2), "build");
        assert_eq!(drop_legacy_default("", 0), "");
    }

    #[test]
    fn a_restored_session_keeps_the_order_of_the_file() {
        // the front is built last, and the parked prefix is every other
        // workspace in the file's order
        assert_eq!(restore_order(3, 0), (vec![1, 2, 0], 0));
        assert_eq!(restore_order(3, 1), (vec![0, 2, 1], 1));
        assert_eq!(restore_order(3, 2), (vec![0, 1, 2], 2));
        assert_eq!(restore_order(1, 0), (vec![0], 0));
        // an index out of range in a hand-edited file climbs back into the list
        assert_eq!(restore_order(3, 9), (vec![0, 1, 2], 2));
        // read through the mapping the cards use (`slot` in `wscards`), the
        // session opens with the logical order of the file — 0, 1, 2, and not
        // the 1, 2, 0 that leaving the front last produced
        for front in 0..3 {
            let (order, front) = restore_order(3, front);
            let logical = |i: usize| {
                if i == front {
                    order[order.len() - 1]
                } else {
                    order[if i < front { i } else { i - 1 }]
                }
            };
            assert_eq!((0..3).map(logical).collect::<Vec<_>>(), vec![0, 1, 2]);
        }
    }

    /// A workspace with one empty tab — no pty, no window: enough for the
    /// bookkeeping of the order.
    fn workspace(name: &str) -> Workspace {
        Workspace {
            tabs: vec![Tab::new(10, 4)],
            active: 0,
            name: name.to_string(),
            card_alert: false,
        }
    }

    #[test]
    fn a_notification_out_of_sight_raises_the_card_not_the_toast() {
        // `B8`: with the tab in front the content goes out at arrival; out of
        // sight the card keeps the signal and the content waits for the visit
        let mut front = Tab::new(20, 5);
        front.term.feed(b"\x1b]9;done\x07");
        let mut out = PumpOut::default();
        front.pump(Bell::None, true, &mut out);
        assert_eq!(out.toast.as_deref(), Some("done"));
        assert!(!out.card_alert);

        let mut back = Tab::new(20, 5);
        back.term.feed(b"\x1b]9;done\x07");
        let mut out = PumpOut::default();
        back.pump(Bell::None, false, &mut out);
        assert!(out.toast.is_none(), "a parked tab must not toast");
        assert!(out.card_alert, "the card is the signal while it waits");
        // and the badge survives: it was delivered, not acknowledged
        assert!(back.notif_pending());
    }

    #[test]
    fn a_moved_workspace_carries_the_order_around_it() {
        // `B49`: the front is a place in the list, so a move on either side of
        // it shifts it by one — and nothing else does
        assert_eq!(
            moved_front(1, 1, 3),
            3,
            "the moved one is the front: it lands where it was dropped"
        );
        assert_eq!(moved_front(2, 0, 3), 1, "a move from the left past it");
        assert_eq!(
            moved_front(1, 3, 0),
            2,
            "a move from the right to before it"
        );
        assert_eq!(moved_front(1, 3, 4), 1, "a move entirely on its right");
        assert_eq!(moved_front(3, 0, 1), 3, "a move entirely on its left");
    }

    /// A walk stops at both ends of the sidebar (`B49`): above the first one
    /// is the first one, below the last one is the last one — and the carry
    /// uses the same clamp, so moving the front past either end is a no-op.
    #[test]
    fn a_workspace_walk_clamps_at_both_ends() {
        assert_eq!(ws_above(0), 0, "already first");
        assert_eq!(ws_above(2), 1);
        assert_eq!(ws_below(0, 3), 1);
        assert_eq!(ws_below(2, 3), 2, "already last");
        assert_eq!(ws_below(0, 1), 0, "a single workspace");
    }

    #[test]
    fn a_dragged_workspace_lands_where_it_was_dropped() {
        // `B49`: three in the list, the middle one in front
        let mut all = vec![workspace("0"), workspace("1"), workspace("2")];
        let mut front = 1;
        reorder(&mut all, &mut front, 0, 2);
        let order: Vec<&str> = all.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(order, vec!["1", "2", "0"], "the dragged one comes out");
        assert_eq!(front, 0, "the front keeps the text it was showing");

        // — and when the front is the one dragged, it lands where it was dropped
        let mut all = vec![workspace("0"), workspace("1"), workspace("2")];
        let mut front = 1;
        reorder(&mut all, &mut front, 1, 2);
        let order: Vec<&str> = all.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(order, vec!["0", "2", "1"]);
        assert_eq!(front, 2, "the front followed itself");
    }

    #[test]
    fn a_new_workspace_leaves_the_others_where_they_were() {
        // `B54`: three workspaces, the middle one in front, so `parked` holds
        // the other two in logical order
        let mut parked = vec![workspace("0"), workspace("2")];
        let new_front = park_front(&mut parked, workspace("1"), 1);
        let order: Vec<&str> = parked.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(
            order,
            vec!["0", "1", "2"],
            "the one left behind keeps its place in the order"
        );
        assert_eq!(new_front, 3, "and the new workspace is the last one");
    }

    #[test]
    fn the_directory_of_a_tab_dies_with_its_child() {
        // `B51`: `save_geometry` used to read the layout **after** `last_words`,
        // and a tab's directory is `/proc/<pid>/cwd`. This is the window that was
        // being lost — every tab came back with `cwds: [null]`.
        let (tx, _rx) = std::sync::mpsc::channel();
        let wake: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(|| {});
        let dir = std::env::temp_dir();
        let mut pty = Pty::spawn(
            Some("exec sleep 30"),
            None,
            Some(dir.as_path()),
            80,
            24,
            (8.0, 16.0),
            tx,
            wake,
        )
        .expect("a pty with a shell in it");
        assert_eq!(tab_cwd(Some(&pty), None), Some(dir.clone()));
        pty.hangup();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline && tab_cwd(Some(&pty), None).is_some() {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            tab_cwd(Some(&pty), None),
            None,
            "a tab read after the hangup has no directory — the layout goes first"
        );
    }

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

    /// `t=t` deletes what the protocol allows: a temp dir plus the marker.
    /// Anything else is read but kept — the path comes from the child.
    /// Each test owns its dir: they run in parallel threads.
    fn image_tmp(tag: &str, name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("tortty-img-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, b"pixels").unwrap();
        (dir, p)
    }

    #[test]
    fn temp_file_with_marker_is_deleted() {
        let (dir, p) = image_tmp("del", "kitty-tty-graphics-protocol-1");
        let ps = p.to_str().expect("temp dir is UTF-8");
        assert_eq!(App::read_image_file(ps, true).unwrap(), b"pixels");
        assert!(!p.exists(), "the temp file is gone");
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn temp_file_without_marker_is_kept() {
        let (dir, p) = image_tmp("keep", "plain-pixels");
        let ps = p.to_str().expect("temp dir is UTF-8");
        assert_eq!(App::read_image_file(ps, true).unwrap(), b"pixels");
        assert!(p.exists(), "no marker, no delete");
        std::fs::remove_file(&p).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn marker_outside_temp_is_kept() {
        // `/var/tmp` is real but not a known temp dir for the protocol
        let name = format!("tortty-keep-{}-tty-graphics-protocol", std::process::id());
        let p = std::path::PathBuf::from("/var/tmp").join(&name);
        std::fs::write(&p, b"pixels").unwrap();
        assert_eq!(
            App::read_image_file(p.to_str().unwrap(), true).unwrap(),
            b"pixels"
        );
        assert!(p.exists(), "outside temp, no delete");
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn dotdot_escape_is_kept() {
        // the bytes sit outside temp; the `..` path must not launder that
        let pid = std::process::id();
        let name = format!("tortty-esc-{pid}-tty-graphics-protocol");
        let real = std::path::PathBuf::from("/var/tmp").join(&name);
        std::fs::write(&real, b"pixels").unwrap();
        let dir = std::env::temp_dir().join(format!("tortty-esc-{pid}"));
        std::fs::create_dir_all(&dir).unwrap();
        let via = dir.join(format!("../../var/tmp/{name}"));
        assert_eq!(
            App::read_image_file(via.to_str().unwrap(), true).unwrap(),
            b"pixels"
        );
        assert!(real.exists(), "canonicalized out of temp, no delete");
        std::fs::remove_file(&real).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
