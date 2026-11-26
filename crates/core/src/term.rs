//! Terminal state: grid, cursor and VT parser.
//!
//! No GUI, no pty, no GPU. It is what lets the parser benchmark run without a
//! display, and what guarantees the number measured is the parser's, not the
//! toolkit's.

use std::collections::VecDeque;
use unicode_width::UnicodeWidthChar;

use crate::cell::{
    Cell, BLINK, BOLD, FAINT, INV, ITALIC, OVERLINE, STRIKE, UL_CURLY, UL_DASHED, UL_DOTTED,
    UL_DOUBLE, UL_NONE, UL_SINGLE, WIDE_CONT,
};
use crate::charset::dec_graphics;
use crate::color::Color;
use crate::config::Theme;
use crate::grid::Row;
use crate::select::Span;

const REPLACEMENT: char = '\u{FFFD}';

/// Ceiling for the payload of an in-flight OSC: a sequence that never closes
/// must not grow without bound.
const MAX_OSC: usize = 4096;
/// The one OSC form that carries megabytes: iTerm2's inline image
/// (`1337;File=` + base64). Everything else keeps the 4 KiB cap.
const MAX_OSC_IMAGE: usize = 16 * 1024 * 1024;
const ITERM_PREFIX: &[u8] = b"1337;File=";
/// How many distinct `OSC 8` URIs are kept (a span reuses its index).
pub const LINK_MAX: usize = 4096;
/// How many OSC the probe keeps, and how much of each (the socket's `osc` verb).
const OSC_SEEN: usize = 8;
const OSC_SEEN_MAX: usize = 256;
/// Ceiling for the title stack (`CSI 22/23 t`): an app that pushes without
/// popping must not grow it without bound.
const TITLE_STACK_MAX: usize = 16;
/// Ceiling for the closed `?2026` traces waiting for the app to drain them: an
/// app that never asks must not make the history grow without bound.
const SYNC_TRACES_MAX: usize = 64;

/// What one `?2026` (synchronized output) transaction carried — the raw
/// material of the debug log.
///
/// Counters, never time: the core has no clock, so the *duration* belongs to
/// whoever feeds it. What lives here is the content of the window, which is the
/// question the log answers: what a repaint clears, resets and scrolls while it
/// holds the screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncTrace {
    /// Reads that carried the window, the one that opened it included.
    pub feeds: u32,
    /// Bytes of the reads that **found** the window open. The read that opens
    /// it is not charged: its length is not the window's.
    pub bytes: u64,
    /// `ED` per argument (index 0..=3): `2` erases the screen, `3` erases the
    /// scrollback too — the one that can make the view jump.
    pub ed: [u32; 4],
    /// Any `EL` (`CSI K`): the line-by-line redraw inside a repaint.
    pub el: u32,
    /// Cursor sent to the top-left cell (`CSI H`/`CSI f` at `1;1`).
    pub home: u32,
    /// `DECSTBM` (`CSI r`): the window played with the scroll region.
    pub stbm: u32,
    /// Alternate-screen switches (`?47`/`?1047`/`?1049`).
    pub alt: u32,
    /// `ESC c` — full reset (`RIS`).
    pub ris: u32,
    /// `CSI ! p` — soft reset (`DECSTR`).
    pub decstr: u32,
    /// Lines pushed into the scrollback while the window was open.
    pub hist: u64,
}

impl SyncTrace {
    /// Whether the window so far is a **repaint**: it erased the screen or put
    /// the cursor back on the first cell before drawing it again, so the rows it
    /// scrolls are the same text being redrawn and must not move the reader
    /// (`B41`). A window that only writes — the case of a program that appends
    /// inside `?2026` — is not a repaint, and its rows are real output (`B53`).
    ///
    /// `EL` does not count: a line-by-line append clears lines too, and that is
    /// exactly the shape the creep came in.
    #[must_use]
    pub fn repaints(&self) -> bool {
        self.ed[2] > 0 || self.ed[3] > 0 || self.home > 0 || self.ris > 0
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum State {
    Ground,
    Escape,
    EscInter,
    Csi,
    Osc,
    OscEsc,
    /// DCS/APC/PM/SOS: consumed until ST. Kitty images land here (phase 1
    /// ignores the content, but it must not leave junk on the grid).
    Str,
    StrEsc,
}

#[derive(Copy, Clone)]
struct SavedCursor {
    cx: usize,
    cy: usize,
    fg: Color,
    bg: Color,
    ul_color: Color,
    flags: u8,
    ul: u8,
    origin: bool,
    autowrap: bool,
}

impl Default for SavedCursor {
    fn default() -> Self {
        SavedCursor {
            cx: 0,
            cy: 0,
            fg: Color::Default,
            bg: Color::Default,
            ul_color: Color::Default,
            flags: 0,
            ul: UL_NONE,
            origin: false,
            autowrap: true,
        }
    }
}

/// An OSC that arrived, with the raw payload (no `ESC ]`, no terminator). It is
/// the probe behind the socket's `osc` verb: when the tab bar shows junk, the
/// question is "which bytes did the child send", and only the parser can answer.
#[derive(Clone, Debug)]
pub struct OscSeen {
    pub raw: Vec<u8>,
    /// The payload went past `OSC_SEEN_MAX` and was cut.
    pub truncated: bool,
    /// Title **after** applying this OSC (useful when the title did not change:
    /// the OSC was a color, a link, or junk that matched neither `0` nor `2`).
    pub title: String,
}

/// A remote clipboard write (`OSC 52`, `M-R`): the target and the decoded
/// text. Only setting exists — the query form is refused in the parser, so
/// the child's bytes can never read the clipboard back out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardOp {
    Clipboard(String),
    Primary(String),
}

/// The progress a program reports about itself (`OSC 9;4`, mintty/ConEmu): the
/// state and, when the state has one, the fraction. State `0` means "no
/// progress" and clears it, so this type never carries a `0`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// 1 normal, 2 error, 3 indeterminate, 4 warning/paused.
    pub state: u8,
    /// `0..=100`, or `None` for a state that has no measure.
    pub pct: Option<u8>,
}

/// Where the extended color goes: text, background or underline.
#[derive(Copy, Clone)]
enum Slot {
    Fg,
    Bg,
    Ul,
}

/// History counters taken before a read, so the view can stay on the line the
/// reader was reading (see `Term::view_after_output`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeedMark {
    /// Absolute rows that had left through the top.
    scrolled: u64,
    /// Of those, the ones a `?2026` repaint scrolled.
    redraw: u64,
    /// Lines the reader could look at.
    history: usize,
    /// `CSI 3J`/`RIS` generation: a new number means the history was erased.
    hist_gen: u64,
}

impl FeedMark {
    /// Rows that had already left through the top.
    #[must_use]
    pub fn scrolled(&self) -> u64 {
        self.scrolled
    }

    /// Of those, the ones a `?2026` window scrolled — the repaint half.
    #[must_use]
    pub fn redraw(&self) -> u64 {
        self.redraw
    }

    /// Lines the reader could look at when the mark was taken.
    #[must_use]
    pub fn history(&self) -> usize {
        self.history
    }

    /// The history generation: a new one means the history was erased.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.hist_gen
    }
}

#[allow(clippy::struct_excessive_bools)] // VT state is boolean by nature
pub struct Term {
    pub cols: usize,
    pub rows: usize,
    /// Live grid (one `Row` per line, contiguous cells).
    pub grid: Vec<Row>,
    /// Main buffer stashed while the app is on the alt screen.
    alt: Option<Vec<Row>>,
    /// Kitty images (H1–H13): protocol, images and placements.
    pub graphics: crate::graphics::Graphics,
    pub scrollback: VecDeque<Row>,
    pub scrollback_limit: usize,
    pub cx: usize,
    pub cy: usize,
    pub fg: Color,
    pub bg: Color,
    /// `SGR 58`.
    pub ul_color: Color,
    pub flags: u8,
    /// Underline style (`UL_*`).
    pub ul: u8,
    /// Scroll region (DECSTBM), inclusive.
    pub top: usize,
    pub bottom: usize,
    pub wrap_pending: bool,
    pub cursor_visible: bool,
    pub app_cursor: bool,
    /// `DECKPAM` (`ESC =`): the keypad application mode, answered by `DECRQM` as
    /// 66 and cleared by `DECSTR`/`RIS`. The encoder does **not** turn it into
    /// bytes (`LIMITATIONS.md`: the programs that ask for it cannot read
    /// `ESC O`).
    pub keypad_app: bool,
    /// `(shape 1..6, blink)` from DECSCUSR; None = from the config.
    pub app_cursor_style: Option<(u8, bool)>,
    /// While true, every row this feed writes is decoration (`Row::chrome`, `B48`).
    /// It only exists so that `feed_chrome` can leave the marker out of the
    /// snapshot without the app having to know where the row landed.
    chrome_feed: bool,
    /// `?1000`=1 press, `?1002`=2 with drag, `?1003`=3 everything (0 = no).
    pub mouse_mode: u8,
    /// `?1006`: decimal coordinates with an explicit button on release.
    pub mouse_sgr: bool,
    /// `?1005`: coordinates as UTF-8 in the legacy protocol.
    pub mouse_utf8: bool,
    /// `?1016`: the SGR form, but the coordinates are pixels inside the grid
    /// instead of cells. The app converts: only it knows the cell size.
    pub mouse_px: bool,
    pub focus_report: bool,
    pub bracketed_paste: bool,
    pub sync_output: bool,
    /// Trace of the `?2026` window that is open right now.
    sync_trace: Option<SyncTrace>,
    /// Traces of the windows that already closed, waiting for the app to drain
    /// them (capped: a log is a diagnostic, not a leak).
    sync_closed: Vec<SyncTrace>,
    pub autowrap: bool,
    /// LNM (`CSI 20 h`): LF returns to column 0 as well.
    pub lnm: bool,
    pub origin: bool,
    /// Kitty keyboard protocol flags (`CSI > u`, `CSI = u`). The app's
    /// encoder reads from here: the bits change **what** the key becomes.
    pub kbd_flags: u32,
    /// Stack of `CSI > u` / `CSI < n u`. Kitty caps it at 64 levels.
    kbd_stack: Vec<u32>,
    g0_dec: bool,
    g1_dec: bool,
    use_g1: bool,
    saved: SavedCursor,
    /// Replies to the child (DA, CPR, DECRQM...). The app drains and writes to the pty.
    pub child_out: Vec<u8>,
    pub title: String,
    /// Where the shell says it is (`OSC 7`, `file://host/path`). Only the app
    /// uses it, as the last resort for the cwd of a new tab.
    pub cwd: Option<String>,
    /// `OSC 8` URIs, by the index a cell carries (`0` is not in the list: a cell
    /// with `link == 0` has none).
    pub links: Vec<String>,
    /// The open `OSC 8` span: the index every new cell gets.
    link: u16,
    /// Titles saved by `CSI 22 t`, restored by `CSI 23 t` — how an app gives the
    /// title back when it leaves (`mc` pushes, sets its own with OSC 0 and pops).
    title_stack: Vec<String>,
    /// The last OSC that arrived, oldest to newest.
    pub osc_seen: VecDeque<OscSeen>,
    /// A remote clipboard write waiting for the app (`OSC 52`, `M-R`). The
    /// core decodes and validates; `clip.set`/`set_primary` is the app's.
    pub clipboard: Option<ClipboardOp>,
    /// The in-flight OSC went past `MAX_OSC` and lost bytes: a cut base64
    /// still decodes, and a mangled clipboard is worse than none.
    osc_over: bool,
    /// Theme colors: the renderer uses them and `OSC 4/10/11/12` queries/changes them.
    pub theme: Theme,
    /// Someone touched the colors via OSC: the app has to resend them to the renderer.
    pub theme_dirty: bool,
    pub cell_size: (u32, u32),
    pub bells: u32,
    /// How many notifications arrived (the app's cursor, as `bells`).
    ///
    /// Both counts only move forward: the app follows them with
    /// `while seen != count`, so a rewind is not a missed event but ~2^32
    /// laps of the pump — the tab hangs. `reset` preserves them for exactly
    /// this reason, and so does the unseen list below them.
    ///
    /// The content is `notifications`, not a slot: a tab that says two things
    /// keeps both, and the same `(source, id)` replaces its own entry.
    pub notifs: u32,
    /// What the child asked the user to look at, newest first.
    pub notifications: crate::notif::List,
    /// The emulation's decisions since the last drain (bounded: a full ring
    /// forgets the oldest — the emulation never stops for lack of a consumer).
    trace: crate::trace::Trace,
    /// Progress reported by the program (`OSC 9;4`): the tab label draws it.
    /// `None` = nothing to show; state `0` clears it.
    pub progress: Option<Progress>,
    /// `OSC 7501` program status records (`B47`): what each program in this tab
    /// says it is doing. One record per id, replaced whole by each report.
    pub status: crate::status::Status,
    /// `OSC 133` prompt marks (`M-R`): where a prompt, a command and its output
    /// start, and how the command ended. Absolute lines, so scrolling and output
    /// do not move them.
    pub marks: crate::prompt::Marks,
    /// Milliseconds since the app started, written by the app before every
    /// `feed` — the core has no clock, so whoever feeds it says what time it
    /// is (as `cell_size`: the caller passes what only it knows). Tests write
    /// it by hand. It only drives the `133;D` duration (`B74`).
    pub now_ms: u64,
    /// When the running command started (`133;B`, `133;C` as the fallback when
    /// the shell skips `B`), in `now_ms`. `None` = no command known: a `D`
    /// without a start still reports a failure, but with no duration.
    pub cmd_start_ms: Option<u64>,
    /// The `133;D` rule (`B74`), synced from the config by the app before every
    /// `feed`: a command that ran this long notifies, whatever the status was.
    /// A fast command means the owner was watching — the screen is the source
    /// of truth for what just happened — so speed alone is never news. `0` =
    /// the `D` never notifies.
    pub shell_notify_secs: u64,
    /// Something the child asked for that we refused, with the reason, for the
    /// app to show once (`B3`).
    refused: Option<&'static str>,
    /// Cap of the in-flight OSC: `MAX_OSC`, raised to `MAX_OSC_IMAGE` once the
    /// body is known to be an iTerm2 image (checked at the tenth byte).
    osc_cap: usize,
    pub respond_size: bool,
    tabstops: Vec<bool>,
    pub dirty: Vec<bool>,
    pub dirty_all: bool,
    /// How many times global damage was marked, and by whom. That is the first
    /// question when the whole grid rebuilds for no apparent reason.
    pub global_marks: u64,
    pub global_why: &'static str,
    /// Rows the whole screen moved up (+) or down (−) since the last
    /// `clear_damage`. Full-screen scrolling dirties no row at all: the content
    /// only changed place, and the renderer reuses the rows already built.
    pub scroll_shift: isize,
    /// Scrolling happened both ways: the net shift no longer describes what
    /// happened (a row that leaves becomes blank and never comes back), so the
    /// renderer has to rebuild everything.
    pub scroll_conflict: bool,
    /// Rows that left through the top since the start (absolute index).
    pub scrolled: u64,
    /// Rows that left through the top while a `?2026` window was open: a
    /// repaint of what is already on screen, not output the reader follows.
    pub scrolled_sync: u64,
    /// The `hist_gen` produced by the last history erase, when it happened
    /// inside a `?2026` window (a repaint, not a `clear`). `hist_gen` only
    /// grows, so a match means "this wipe came from inside a window".
    quiet_wipe_gen: u64,
    pub abs_gen: u64,
    pub hist_gen: u64,
    last_char: char,
    state: State,
    csi: Vec<u8>,
    osc: Vec<u8>,
    /// APC body (`ESC _`): this is how the kitty image arrives.
    apc: Vec<u8>,
    /// The body of a DCS in flight (`B3`): only the sixel kind keeps it, and it is
    /// what the decoder walks when the string closes.
    dcs: Vec<u8>,
    /// Image files waiting for the app (`t=f`/`t=t`/`t=s`, `B1`): the payload
    /// named them, the bytes have not arrived yet.
    pub image_files: Vec<crate::graphics::FileRequest>,
    /// The in-flight `Str` is an APC (not DCS/PM/SOS, which are discarded).
    apc_active: bool,
    dcs_active: bool,
    csi_prefix: u8,
    csi_inter: u8,
    esc_inter: u8,
    utf8: [u8; 4],
    utf8_len: u8,
    utf8_need: u8,
}

impl Term {
    pub fn new(cols: usize, rows: usize) -> Term {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Term {
            cols,
            rows,
            grid: (0..rows).map(|_| Row::blank(cols)).collect(),
            alt: None,
            scrollback: VecDeque::new(),
            scrollback_limit: 1000,
            cx: 0,
            cy: 0,
            fg: Color::Default,
            bg: Color::Default,
            ul_color: Color::Default,
            flags: 0,
            ul: UL_NONE,
            top: 0,
            bottom: rows - 1,
            wrap_pending: false,
            cursor_visible: true,
            app_cursor: false,
            keypad_app: false,
            app_cursor_style: None,
            chrome_feed: false,
            mouse_mode: 0,
            mouse_sgr: false,
            mouse_utf8: false,
            mouse_px: false,
            focus_report: false,
            bracketed_paste: false,
            sync_output: false,
            sync_trace: None,
            sync_closed: Vec::new(),
            autowrap: true,
            lnm: false,
            origin: false,
            kbd_flags: 0,
            kbd_stack: Vec::new(),
            g0_dec: false,
            g1_dec: false,
            use_g1: false,
            saved: SavedCursor::default(),
            child_out: Vec::new(),
            title: String::from("tortty"),
            cwd: None,
            links: Vec::new(),
            link: 0,
            title_stack: Vec::new(),
            osc_seen: VecDeque::with_capacity(OSC_SEEN),
            clipboard: None,
            osc_over: false,
            theme: Theme::default(),
            theme_dirty: false,
            cell_size: (10, 19),
            bells: 0,
            notifs: 0,
            notifications: crate::notif::List::default(),
            trace: crate::trace::Trace::default(),
            progress: None,
            status: crate::status::Status::default(),
            marks: crate::prompt::Marks::default(),
            now_ms: 0,
            cmd_start_ms: None,
            shell_notify_secs: crate::config::SHELL_NOTIFY_DEFAULT,
            refused: None,
            osc_cap: MAX_OSC,
            respond_size: false,
            tabstops: (0..cols).map(|i| i % 8 == 0).collect(),
            dirty: vec![true; rows],
            scroll_shift: 0,
            scroll_conflict: false,
            dirty_all: true,
            global_marks: 0,
            global_why: "",
            scrolled: 0,
            abs_gen: 0,
            hist_gen: 0,
            scrolled_sync: 0,
            quiet_wipe_gen: 0,
            last_char: ' ',
            state: State::Ground,
            graphics: crate::graphics::Graphics::new(),
            csi: Vec::with_capacity(64),
            osc: Vec::with_capacity(128),
            apc: Vec::new(),
            dcs: Vec::new(),
            image_files: Vec::new(),
            apc_active: false,
            dcs_active: false,
            csi_prefix: 0,
            csi_inter: 0,
            esc_inter: 0,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_need: 0,
        }
    }

    // ----------------------------------------------------------------- grid

    /// A blank cell inherits the current bg (BCE), like a real terminal.
    #[inline]
    fn blank_cell(&self) -> Cell {
        Cell {
            ch: ' ',
            fg: Color::Default,
            bg: self.bg,
            ul_color: Color::Default,
            flags: 0,
            ul: UL_NONE,
            link: 0,
        }
    }

    #[inline]
    pub fn touch(&mut self, y: usize) {
        if y < self.dirty.len() {
            self.dirty[y] = true;
        }
    }

    pub fn touch_all(&mut self) {
        self.mark_global("touch_all");
    }

    pub fn has_damage(&self) -> bool {
        self.dirty_all || self.dirty.iter().any(|&d| d)
    }

    /// Dirties a range of rows (exclusive end), clamped to the grid.
    pub fn mark_rows(&mut self, from: usize, to: usize) {
        for y in from..to.min(self.dirty.len()) {
            self.dirty[y] = true;
        }
    }

    /// Marks the whole grid dirty, recording who asked.
    pub fn mark_global(&mut self, why: &'static str) {
        self.dirty_all = true;
        self.global_marks = self.global_marks.saturating_add(1);
        self.global_why = why;
    }

    pub fn clear_damage(&mut self) {
        self.dirty_all = false;
        self.dirty.fill(false);
        self.scroll_shift = 0;
        self.scroll_conflict = false;
    }

    pub fn reset(&mut self) {
        let (cols, rows) = (self.cols, self.rows);
        let limit = self.scrollback_limit;
        let hist = self.hist_gen + 1;
        let abs = self.abs_gen + 1;
        // the unseen state survives: `bells`/`notifs` only move forward (the
        // app's pump hangs on a rewind) and a reset of the grid must not eat
        // news the user has not seen — the mark outlives `RIS` on the app's
        // side, and it would point at an empty list otherwise
        let bells = self.bells;
        let notifs = self.notifs;
        let notifications = std::mem::take(&mut self.notifications);
        // the log survives the reset (a `RIS` *is* one of the events it reports)
        let in_sync = self.sync_trace.is_some();
        let mut closed = std::mem::take(&mut self.sync_closed);
        if let Some(mut t) = self.sync_trace.take() {
            // `RIS` resets the DEC modes, so the window ends here without a close
            t.ris += 1;
            if closed.len() < SYNC_TRACES_MAX {
                closed.push(t);
            }
        }
        *self = Term::new(cols, rows);
        self.scrollback_limit = limit;
        self.hist_gen = hist;
        self.abs_gen = abs;
        self.bells = bells;
        self.notifs = notifs;
        self.notifications = notifications;
        self.sync_closed = closed;
        if in_sync {
            self.quiet_wipe_gen = hist;
        }
    }

    /// Counts one event in the open `?2026` window (nothing to do when none is
    /// open: the tracing costs one `Option` test per event, not per byte).
    fn sync_count(&mut self, f: impl FnOnce(&mut SyncTrace)) {
        if let Some(t) = self.sync_trace.as_mut() {
            f(t);
        }
    }

    /// The `?2026` window that is open now, for the app to stamp when it began.
    #[must_use]
    pub fn sync_open(&self) -> Option<&SyncTrace> {
        self.sync_trace.as_ref()
    }

    /// The `?2026` windows that closed since the last call (the app records
    /// them; whoever does not call this loses the trace, by design).
    pub fn take_sync_closed(&mut self) -> Vec<SyncTrace> {
        std::mem::take(&mut self.sync_closed)
    }

    /// DECSTR (soft reset): touches neither the grid nor the scrollback.
    fn soft_reset(&mut self) {
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.ul_color = Color::Default;
        self.flags = 0;
        self.ul = UL_NONE;
        self.cursor_visible = true;
        self.app_cursor = false;
        self.keypad_app = false;
        self.origin = false;
        self.autowrap = true;
        self.top = 0;
        self.bottom = self.rows - 1;
        self.wrap_pending = false;
    }

    /// The history as it was before a read: what `view_after_output` needs to
    /// keep the reader on the line they were reading (D10).
    #[must_use]
    pub fn mark(&self) -> FeedMark {
        FeedMark {
            scrolled: self.scrolled,
            redraw: self.scrolled_sync,
            history: self.scrollback.len(),
            hist_gen: self.hist_gen,
        }
    }

    /// How far the view must move so the reader does not lose the line they
    /// were on, after `mark` was taken before the read (D10).
    ///
    /// Only the rows **output** pushed count. Rows a synchronized update
    /// (`?2026`) scrolled are a repaint of what is already on screen: the app is
    /// saying "this is one frame", so the frame does not move the reader either.
    ///
    /// A history erased (`CSI 3J`, `RIS`) splits in two, and the difference is
    /// who asked for it: **inside** a window it is a repaint — the transcript is
    /// being rewritten, the reader stays where he was (clamped to what he could
    /// already see), because adding the reprinted lines to the offset throws the
    /// view at the top of an unrelated history. **Outside** a window it is a
    /// `clear`: the saved lines are gone for good and there is nothing above the
    /// live screen to look at.
    #[must_use]
    pub fn view_after_output(&self, off: usize, mark: FeedMark) -> usize {
        if off == 0 {
            return 0;
        }
        if self.hist_gen != mark.hist_gen {
            return if self.quiet_wipe_gen == self.hist_gen {
                off.min(mark.history)
            } else {
                0
            };
        }
        let quiet = self.scrolled - self.scrolled_sync;
        let d = quiet.saturating_sub(mark.scrolled - mark.redraw) as usize;
        if d == 0 {
            return off;
        }
        // Only the distance from the live area matters: a row that left the
        // scrollback (by cap) is already handled by the clamp in `sb`.
        // Subtracting the discarded ones made the view move the wrong way for
        // every row the grid dropped — at the cap, `off` froze and the reader
        // lost one row of content per new row (the opposite of D10).
        (off + d).min(self.scrollback.len())
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.resize_keep(cols, rows, None);
    }

    /// `resize` that carries a selection across the reflow (`M-P`).
    ///
    /// The anchors come back on the same characters of the same text; `None`
    /// means they could not be carried (the rows they named left the scrollback
    /// or the screen went to the alt buffer, where nothing reflows) and the
    /// selection dies, as it did before.
    ///
    /// The anchors are the caller's — the core does not keep a selection — so
    /// they arrive and leave by parameter, and the mapping is done here because
    /// it is the reflow that knows where each cell went.
    pub fn resize_keep(&mut self, cols: usize, rows: usize, sel: Option<Span>) -> Option<Span> {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if (cols, rows) == (self.cols, self.rows) {
            return sel;
        }
        self.mark_global("resize");
        self.abs_gen += 1;
        if let Some(alt) = self.alt.as_mut() {
            // The alt screen has no scrollback: whatever leaves the screen dies,
            // and no reflow (it is an app screen, not a text one).
            crop_rows(alt, cols, rows);
        }
        // Main buffer: joins the soft paragraphs and rewraps them at the new
        // width. The cursor is recomputed from the offset inside the paragraph,
        // not from `cy` — which stops meaning the same thing.
        let (cur_row, cur_col) = (self.cy, self.cx);
        let hist = self.scrollback.len();
        let all: Vec<Row> = self
            .scrollback
            .drain(..)
            .chain(std::mem::take(&mut self.grid))
            .collect();
        // the anchors arrive in the absolute space; the reflow works on indexes
        // of `all`, whose first row is the first absolute line that still exists
        let first_old = self.scrolled as i64 - hist as i64;
        let total_old = all.len();
        let mut carried = [false; 2];
        let mut pts: Vec<(usize, usize)> = Vec::new();
        if let Some(s) = sel {
            for (k, a) in [s.a, s.b].iter().enumerate() {
                let i = a.line - first_old;
                if i >= 0 && (i as usize) < total_old {
                    carried[k] = true;
                    pts.push((i as usize, a.col));
                }
            }
        }
        let mut re = reflow(all, cols, hist + cur_row, cur_col, &pts);
        let first_live = re.rows.len().saturating_sub(rows);
        let live = re.rows.split_off(first_live);
        self.scrollback.extend(re.rows);
        while self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
        }
        // rows the ceiling ate at the front: everything the reflow numbered is
        // shifted by this much in the space that survives
        let dropped = first_live.saturating_sub(self.scrollback.len());
        self.grid = live;
        // always `rows` live rows: the missing ones are born blank at the bottom
        while self.grid.len() < rows {
            self.grid.push(Row::blank(cols));
        }
        self.cy = re.cur_row.saturating_sub(first_live).min(rows - 1);
        self.cx = re.cur_col.min(cols - 1);
        // H12: the reflow changed how many rows exist above the grid. Whatever
        // fell outside the scrollback became unreachable (not even with the
        // wheel), so it is dropped; an image still in the band stays where it is
        // (an image does not follow reflow row by row, as in kitty).
        self.graphics
            .drop_above(-i32::try_from(self.scrollback.len()).unwrap_or(i32::MAX));
        let wrap = re.wrap;
        // the absolute index is still valid, but the content moved: an old
        // anchor now points at another row, so it dies (the `abs_gen` went up)
        self.scrolled = self.scrolled.max(self.scrollback.len() as u64);
        self.cols = cols;
        self.rows = rows;
        self.tabstops.resize(cols, false);
        for (i, t) in self.tabstops.iter_mut().enumerate() {
            if i % 8 == 0 {
                *t = true;
            }
        }
        self.top = 0;
        self.bottom = rows - 1;
        self.wrap_pending = wrap;
        self.dirty = vec![true; rows];
        // the base of the new absolute space, now that the ceiling and `scrolled`
        // have both had their say
        let base = self.scrolled as i64 - self.scrollback.len() as i64;
        let mut mapped = sel?;
        let mut it = re.points.into_iter();
        for (k, a) in [&mut mapped.a, &mut mapped.b].iter_mut().enumerate() {
            if !carried[k] {
                return None;
            }
            let (row, col) = it.next()?;
            if row < dropped {
                return None;
            }
            a.line = base + (row - dropped) as i64;
            a.col = col;
            a.gen = self.abs_gen;
        }
        Some(mapped)
    }

    // -------------------------------------------------------------- writing

    #[inline]
    fn dec_active(&self) -> bool {
        if self.use_g1 {
            self.g1_dec
        } else {
            self.g0_dec
        }
    }

    fn put_ascii_run(&mut self, run: &[u8]) {
        let mut i = 0usize;
        while i < run.len() {
            // a pending joiner (`B66`) wants the very next character, ASCII
            // included (`a\u{200D}b` asks the font for a ligature): that one goes
            // through the slow path instead of being written as a cell of its own
            if self.joiner_pending() {
                let ch = run[i] as char;
                self.put_char(ch);
                i += 1;
                continue;
            }
            if self.wrap_pending {
                self.do_wrap();
            }
            let room = self.cols - self.cx;
            if room == 0 {
                self.cx = self.cols - 1;
                if self.autowrap {
                    self.wrap_pending = true;
                    continue;
                }
                break;
            }
            let n = (run.len() - i).min(room);
            let fg = self.fg;
            let bg = self.bg;
            let ul_color = self.ul_color;
            let flags = self.flags;
            let ul = self.ul;
            let link = self.link;
            let dec = self.dec_active();
            let cx = self.cx;
            let cy = self.cy;
            // the fast path writes the row without passing through `put_char`, so
            // the decoration mark is set here — once per run, not per char
            if self.chrome_feed {
                self.grid[cy].chrome = true;
            }
            let row = &mut self.grid[cy];
            if dec {
                for k in 0..n {
                    let ch = dec_graphics(run[i + k] as char);
                    row.cells[cx + k] = Cell {
                        ch,
                        fg,
                        bg,
                        ul_color,
                        flags,
                        ul,
                        link,
                    };
                }
            } else {
                for k in 0..n {
                    let ch = run[i + k] as char;
                    row.cells[cx + k] = Cell {
                        ch,
                        fg,
                        bg,
                        ul_color,
                        flags,
                        ul,
                        link,
                    };
                }
            }
            // the cells just written replace whatever was there, marks included
            // (`M-U`): one pass per run, and only when the row has any
            if !row.tail.is_empty() {
                row.tail
                    .retain(|(c, _)| usize::from(*c) < cx || usize::from(*c) >= cx + n);
            }
            self.last_char = run[i + n - 1] as char;
            self.dirty[cy] = true;
            i += n;
            if cx + n >= self.cols {
                self.cx = self.cols - 1;
                if self.autowrap {
                    self.wrap_pending = true;
                }
            } else {
                self.cx = cx + n;
            }
        }
    }

    fn do_wrap(&mut self) {
        self.grid[self.cy].wrapped = true;
        self.cx = 0;
        self.newline();
        self.wrap_pending = false;
    }

    /// The cell a zero-width or joining character belongs to: the one behind the
    /// cursor, skipping the continuation half of a wide character. `None` at the
    /// very start of the grid, where there is nothing to belong to.
    fn cluster_target(&self) -> Option<(usize, usize)> {
        if self.cx > 0 {
            let mut col = self.cx - 1;
            if self.grid[self.cy].cells.get(col).is_some_and(Cell::is_cont) {
                col = col.saturating_sub(1);
            }
            Some((self.cy, col))
        } else if self.cy > 0 {
            let row = &self.grid[self.cy - 1];
            let mut col = row.cells.len().saturating_sub(1);
            if row.cells.get(col).is_some_and(Cell::is_cont) {
                col = col.saturating_sub(1);
            }
            Some((self.cy - 1, col))
        } else {
            None
        }
    }

    /// Is the cell before the cursor waiting for the character a joiner names? A
    /// cluster never crosses a row: a dangling joiner at the end of a line joins
    /// nothing, and the first character of the next line is a character.
    fn joiner_pending(&self) -> bool {
        self.cluster_target().is_some_and(|(row, col)| {
            row == self.cy && self.grid[row].tail_at(col).last() == Some(&'\u{200D}')
        })
    }

    /// Does `ch` continue the cluster of the cell before the cursor (`B66`)?
    /// It does when it is a combining mark, a joiner, a variation selector, a
    /// keycap, a tag, a skin-tone modifier, the base a joiner is waiting for, or
    /// the second half of a flag. Consumed here, so it takes no cell and moves
    /// the cursor nowhere — and when the sequence is now emoji presentation the
    /// cell grows to two columns (`❤️` next to `❤︎`).
    fn try_cluster(&mut self, ch: char) -> bool {
        let Some((row, col)) = self.cluster_target() else {
            return false;
        };
        let base = self.grid[row].cells[col].ch;
        let tail = self.grid[row].tail_at(col);
        let mark = crate::grid::is_mark(ch);
        let joined = row == self.cy && tail.last() == Some(&'\u{200D}');
        let flag = crate::grid::is_regional(ch)
            && crate::grid::is_regional(base)
            && !tail.iter().any(|c| crate::grid::is_regional(*c));
        // a sequence needs a base that was written; a plain mark does not (a
        // mark on a space is a mark on a space)
        if !mark && base == ' ' {
            return false;
        }
        if !(mark
            || joined
            || flag
            || crate::grid::is_structural(ch)
            || crate::grid::is_modifier(ch))
        {
            return false;
        }
        self.grid[row].add_tail(col, ch);
        if crate::grid::emoji_presentation(base, self.grid[row].tail_at(col)) {
            self.promote_wide(row, col);
        }
        true
    }

    /// A cluster with emoji presentation is two columns wide, so the cell takes
    /// the column to its right (`B66`). Only when the cursor stands there and a
    /// column is free: growing a cell that already has a character to its right
    /// would eat it, and there is no reflow for that.
    fn promote_wide(&mut self, row: usize, col: usize) {
        if row != self.cy || col + 1 != self.cx || col + 1 >= self.cols {
            return;
        }
        if self.grid[row].cells[col + 1].is_cont() {
            return;
        }
        let c = self.grid[row].cells[col];
        let cont = Cell { ch: WIDE_CONT, ..c };
        self.grid[row].clear_marks(col + 1);
        self.grid[row].cells[col + 1] = cont;
        self.dirty[row] = true;
        if col + 2 >= self.cols {
            self.cx = self.cols - 1;
            if self.autowrap {
                self.wrap_pending = true;
            }
        } else {
            self.cx = col + 2;
        }
    }

    /// `M-U`: a combining mark has no cell of its own — it lands on the character
    /// before the cursor and moves nothing. With nothing before it (start of the
    /// grid) it is dropped rather than inventing a cell to hold it.
    fn put_mark(&mut self, ch: char) {
        let before = if self.cx > 0 {
            Some((self.cy, self.cx - 1))
        } else if self.cy > 0 {
            Some((
                self.cy - 1,
                self.grid[self.cy - 1].cells.len().saturating_sub(1),
            ))
        } else {
            None
        };
        if let Some((row, col)) = before {
            if let Some(r) = self.grid.get_mut(row) {
                r.add_mark(col, ch);
            }
        }
    }

    fn put_char(&mut self, ch: char) {
        if self.try_cluster(ch) {
            return;
        }
        let w = ch.width().unwrap_or(1);
        if w == 0 {
            self.put_mark(ch);
            return;
        }
        if self.wrap_pending {
            self.do_wrap();
        }
        if w == 1 {
            let ch = if self.dec_active() {
                dec_graphics(ch)
            } else {
                ch
            };
            let cell = Cell {
                ch,
                fg: self.fg,
                bg: self.bg,
                ul_color: self.ul_color,
                flags: self.flags,
                ul: self.ul,
                link: self.link,
            };
            self.grid[self.cy].cells[self.cx] = cell;
            // the character that was there is gone, and its marks go with it
            self.grid[self.cy].clear_marks(self.cx);
            if self.chrome_feed {
                self.grid[self.cy].chrome = true;
            }
            self.dirty[self.cy] = true;
            self.last_char = ch;
            if self.cx == self.cols - 1 {
                if self.autowrap {
                    self.wrap_pending = true;
                }
            } else {
                self.cx += 1;
            }
            return;
        }
        // wide: needs two free columns
        if self.cx + 1 >= self.cols {
            if self.autowrap {
                self.do_wrap();
            } else {
                self.cx = self.cols.saturating_sub(2);
            }
        }
        let ch = if self.dec_active() {
            dec_graphics(ch)
        } else {
            ch
        };
        let pen = (
            self.fg,
            self.bg,
            self.ul_color,
            self.flags,
            self.ul,
            self.link,
        );
        let (fg, bg, ul_color, flags, ul, link) = pen;
        let first = Cell {
            ch,
            fg,
            bg,
            ul_color,
            flags,
            ul,
            link,
        };
        let cont = Cell {
            ch: WIDE_CONT,
            fg,
            bg,
            ul_color,
            flags,
            ul,
            link,
        };
        let cx = self.cx;
        let cy = self.cy;
        if self.chrome_feed {
            self.grid[cy].chrome = true;
        }
        self.grid[cy].cells[cx] = first;
        if cx + 1 < self.cols {
            self.grid[cy].cells[cx + 1] = cont;
        }
        self.dirty[cy] = true;
        self.last_char = ch;
        if cx + 2 >= self.cols {
            self.cx = self.cols - 1;
            if self.autowrap {
                self.wrap_pending = true;
            }
        } else {
            self.cx = cx + 2;
        }
    }

    // ---------------------------------------------------------------- cursor

    fn newline(&mut self) {
        if self.cy == self.bottom {
            self.scroll_up(1);
        } else if self.cy + 1 < self.rows {
            self.cy += 1;
        }
    }

    /// IND (ESC D): goes down without returning the column.
    fn index(&mut self) {
        self.newline();
    }

    /// RI (ESC M): goes up; at the top of the region, scrolls down.
    fn reverse_index(&mut self) {
        if self.cy == self.top {
            self.scroll_down(1);
        } else {
            self.cy = self.cy.saturating_sub(1);
        }
    }

    pub fn scroll_up(&mut self, n: usize) {
        let full = self.top == 0 && self.bottom + 1 == self.rows;
        if full {
            // the whole screen moved up: the renderer shifts the ready rows
            if self.scroll_shift < 0 {
                self.scroll_conflict = true;
            }
            #[allow(clippy::cast_possible_wrap)]
            {
                self.scroll_shift += n as isize;
            }
        } else {
            self.mark_global("scroll_up_region");
        }
        let keep = full && self.alt.is_none();
        // H12: the image moves up with the text. Full-screen scrolling sends the
        // placement to the scrollback (negative row) and it comes back when the
        // wheel goes up; region scrolling has no scrollback, so it is dropped.
        let floor = if self.top == 0 {
            Some(-i32::try_from(self.scrollback_limit + 1).unwrap_or(i32::MAX))
        } else {
            None
        };
        for _ in 0..n {
            self.graphics.scroll(
                u16::try_from(self.top).unwrap_or(0),
                u16::try_from(self.bottom).unwrap_or(0),
                floor,
            );
            let row = self.grid.remove(self.top);
            if keep {
                self.sync_count(|t| t.hist += 1);
                self.scrollback.push_back(row);
                while self.scrollback.len() > self.scrollback_limit {
                    self.scrollback.pop_front();
                    // what was pinned to that row died for good (H12)
                    self.graphics
                        .drop_above(-i32::try_from(self.scrollback.len()).unwrap_or(i32::MAX));
                }
            }
            let blank = Row::blank(self.cols);
            let at = self.bottom.min(self.grid.len());
            self.grid.insert(at, blank);
            if let Some(d) = self.dirty.get_mut(at) {
                *d = true; // new row: there is no ready version to shift
            }
            self.scrolled += 1;
            // A window is not proof of a repaint (`B53`): `pi` appends new lines
            // inside `?2026`, and counting those as redrawing froze the reader's
            // offset while 104 lines went into the history under it. What tells
            // the two apart is already in the trace — a repaint clears the screen
            // or sends the cursor home before drawing it again; an append only
            // writes and pushes rows out.
            if self.sync_trace.as_ref().is_some_and(SyncTrace::repaints) {
                self.scrolled_sync += 1;
            }
        }
        // damage travels with the content: the row that was dirty before the
        // scroll is another one now, and the renderer rotates its cache with the
        // same arithmetic. Without this, the row that took text not yet drawn
        // (write and `\n` in the same chunk, the `time sleep 1` case) rotated
        // with a stale cache and stayed blank until a full rebuild (B6).
        if full && !self.scroll_conflict {
            for _ in 0..n {
                self.dirty.rotate_left(1);
                if let Some(d) = self.dirty.last_mut() {
                    *d = true;
                }
            }
        }
        // the region may have shrunk (rows outside it do not move)
        while self.grid.len() > self.rows {
            self.grid.pop();
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let full = self.top == 0 && self.bottom + 1 == self.rows;
        if full {
            if self.scroll_shift > 0 {
                self.scroll_conflict = true;
            }
            #[allow(clippy::cast_possible_wrap)]
            {
                self.scroll_shift -= n as isize;
            }
        } else {
            self.mark_global("scroll_down_region");
        }
        for _ in 0..n {
            let at = self.bottom.min(self.grid.len().saturating_sub(1));
            if self.grid.len() > at {
                self.grid.remove(at);
            }
            self.grid.insert(self.top, Row::blank(self.cols));
            if let Some(d) = self.dirty.get_mut(self.top) {
                *d = true; // new row at the top: nothing ready to shift
            }
            while self.grid.len() > self.rows {
                self.grid.pop();
            }
        }
        // mirror of the upward scroll: damage goes down with the content
        if full && !self.scroll_conflict {
            for _ in 0..n {
                self.dirty.rotate_right(1);
                if let Some(d) = self.dirty.first_mut() {
                    *d = true;
                }
            }
        }
    }

    /// Absolute addressing: with `?6h` (DECOM) the row counts from the region
    /// top and stops at its bottom. Only `CUP`/`HVP`, `VPA` and the `DECSTBM`
    /// home come here — a caller that passes `self.cy` would add the offset a
    /// second time, so the rest uses the row as the child counts it.
    fn cursor_to(&mut self, r: isize, c: isize) {
        let (rlo, rhi) = if self.origin {
            (self.top as isize, self.bottom as isize)
        } else {
            (0, self.rows as isize - 1)
        };
        let r = if self.origin {
            r + self.top as isize
        } else {
            r
        };
        self.cy = r.clamp(rlo, rhi) as usize;
        self.cx = c.clamp(0, self.cols as isize - 1) as usize;
        self.wrap_pending = false;
    }

    /// The row as the child counts it — what `CUP` and `VPA` expect back, so a
    /// column move keeps its row under `?6h` (xterm's `CursorRow`).
    fn cursor_row(&self) -> isize {
        let cy = self.cy as isize;
        if self.origin {
            cy - self.top as isize
        } else {
            cy
        }
    }

    /// `CUU`: the margin stops the walk, but a cursor already above it goes to
    /// the screen top — the same with origin on or off (xterm's `CursorUp`).
    fn cursor_up(&mut self, n: isize) {
        let min = if self.cy < self.top {
            0
        } else {
            self.top as isize
        };
        self.cy = (self.cy as isize - n).max(min) as usize;
        self.wrap_pending = false;
    }

    /// `CUD`, the mirror: a cursor below the region goes down to the last row.
    fn cursor_down(&mut self, n: isize) {
        let max = if self.cy > self.bottom {
            self.rows as isize - 1
        } else {
            self.bottom as isize
        };
        self.cy = (self.cy as isize + n).min(max) as usize;
        self.wrap_pending = false;
    }

    fn forward_tab(&mut self, n: usize) {
        for _ in 0..n {
            let mut x = self.cx + 1;
            while x < self.cols && !self.tabstops.get(x).copied().unwrap_or(false) {
                x += 1;
            }
            self.cx = x.min(self.cols - 1);
        }
        self.wrap_pending = false;
    }

    fn backward_tab(&mut self, n: usize) {
        for _ in 0..n {
            let mut x = self.cx;
            loop {
                if x == 0 {
                    break;
                }
                x -= 1;
                if self.tabstops.get(x).copied().unwrap_or(false) {
                    break;
                }
            }
            self.cx = x;
        }
        self.wrap_pending = false;
    }

    fn save_cursor(&mut self) {
        self.saved = SavedCursor {
            cx: self.cx,
            cy: self.cy,
            fg: self.fg,
            bg: self.bg,
            ul_color: self.ul_color,
            flags: self.flags,
            ul: self.ul,
            origin: self.origin,
            autowrap: self.autowrap,
        };
    }

    fn restore_cursor(&mut self) {
        let s = self.saved;
        self.cx = s.cx.min(self.cols - 1);
        self.cy = s.cy.min(self.rows - 1);
        self.fg = s.fg;
        self.bg = s.bg;
        self.ul_color = s.ul_color;
        self.flags = s.flags;
        self.ul = s.ul;
        self.origin = s.origin;
        self.autowrap = s.autowrap;
        self.wrap_pending = false;
    }

    // -------------------------------------------------------------- erasing

    fn erase_display(&mut self, n: u32) {
        let which = usize::try_from(n).map_or(3, |v| v.min(3));
        self.sync_count(|t| t.ed[which] += 1);
        match n {
            2 | 3 => self.touch_all(),
            1 => self.mark_rows(0, self.cy + 1),
            _ => self.mark_rows(self.cy, self.rows),
        }
        match n {
            2 | 3 => {
                self.graphics.erase(0..self.rows, 0..self.cols);
                let cell = self.blank_cell();
                for y in 0..self.rows {
                    let mut row = Row::blank(self.cols);
                    row.cells.fill(cell);
                    self.grid[y] = row;
                }
                // The alternate screen has no saved lines and no say about what
                // it covers: a `clear` (`ED 3`) up there used to take the
                // reader's history and the pictures under the app with it (`B63`).
                if n == 3 && self.alt.is_none() {
                    // H13: `ED 3` clears the scrollback **and** the images —
                    // without this an image was left hanging on an empty screen
                    self.graphics.clear();
                    self.scrollback.clear();
                    self.abs_gen += 1;
                    self.hist_gen += 1;
                    // the reader keeps his place only when this is a repaint
                    if self.sync_trace.is_some() {
                        self.quiet_wipe_gen = self.hist_gen;
                    }
                }
                self.cx = 0;
                self.cy = 0;
            },
            1 => {
                self.graphics.erase(0..self.cy, 0..self.cols);
                self.graphics.erase(self.cy..self.cy + 1, 0..self.cx + 1);
                let cell = self.blank_cell();
                for x in 0..=self.cx.min(self.cols - 1) {
                    self.grid[self.cy].cells[x] = cell;
                }
                for y in 0..self.cy {
                    let mut row = Row::blank(self.cols);
                    row.cells.fill(cell);
                    self.grid[y] = row;
                }
            },
            _ => {
                self.graphics.erase(self.cy + 1..self.rows, 0..self.cols);
                self.graphics
                    .erase(self.cy..self.cy + 1, self.cx..self.cols);
                let cell = self.blank_cell();
                for x in self.cx..self.cols {
                    self.grid[self.cy].cells[x] = cell;
                }
                for y in self.cy + 1..self.rows {
                    let mut row = Row::blank(self.cols);
                    row.cells.fill(cell);
                    self.grid[y] = row;
                }
            },
        }
    }

    fn erase_line(&mut self, n: u32) {
        self.sync_count(|t| t.el += 1);
        self.touch(self.cy);
        let cols = match n {
            2 => 0..self.cols,
            1 => 0..self.cx + 1,
            _ => self.cx..self.cols,
        };
        self.graphics.erase(self.cy..self.cy + 1, cols);
        let cell = self.blank_cell();
        match n {
            2 => {
                let mut row = Row::blank(self.cols);
                row.cells.fill(cell);
                self.grid[self.cy] = row;
            },
            1 => {
                for x in 0..=self.cx.min(self.cols - 1) {
                    self.grid[self.cy].cells[x] = cell;
                }
            },
            _ => {
                for x in self.cx..self.cols {
                    self.grid[self.cy].cells[x] = cell;
                }
            },
        }
    }

    fn insert_lines(&mut self, n: usize) {
        self.mark_rows(self.cy, self.bottom + 1);
        if !(self.top..=self.bottom).contains(&self.cy) {
            return;
        }
        for _ in 0..n {
            let at = self.bottom.min(self.grid.len().saturating_sub(1));
            if at >= self.cy && self.grid.len() > at {
                self.grid.remove(at);
            }
            self.grid.insert(self.cy, Row::blank(self.cols));
            if self.grid.len() > self.rows {
                self.grid.pop();
            }
        }
    }

    fn delete_lines(&mut self, n: usize) {
        self.mark_rows(self.cy, self.bottom + 1);
        if !(self.top..=self.bottom).contains(&self.cy) {
            return;
        }
        for _ in 0..n {
            self.grid.remove(self.cy);
            let at = self.bottom.min(self.grid.len());
            self.grid.insert(at, Row::blank(self.cols));
            if self.grid.len() > self.rows {
                self.grid.pop();
            }
        }
    }

    fn insert_chars(&mut self, n: usize) {
        self.touch(self.cy);
        let cell = self.blank_cell();
        let x = self.cx;
        let row = &mut self.grid[self.cy];
        for _ in 0..n.min(self.cols) {
            row.cells.insert(x, cell);
            row.cells.truncate(self.cols);
        }
    }

    fn delete_chars(&mut self, n: usize) {
        self.touch(self.cy);
        let cell = self.blank_cell();
        let x = self.cx;
        let row = &mut self.grid[self.cy];
        let end = (x + n).min(self.cols);
        row.cells.drain(x..end);
        while row.cells.len() < self.cols {
            row.cells.push(cell);
        }
    }

    fn erase_chars(&mut self, n: usize) {
        self.touch(self.cy);
        let cell = self.blank_cell();
        let end = (self.cx + n).min(self.cols);
        self.graphics.erase(self.cy..self.cy + 1, self.cx..end);
        for x in self.cx..end {
            self.grid[self.cy].cells[x] = cell;
        }
    }

    // ------------------------------------------------------------------ SGR

    /// Full SGR (C1-C8 of `docs/MIGRATION.md`).
    fn sgr(&mut self, body: &[u8]) {
        let mut toks = Toks::new(body);
        while let Some(tok) = toks.next() {
            if tok.contains(&b':') {
                self.sgr_sub(tok);
                continue;
            }
            match parse_u32(tok) {
                0 => self.pen_reset(),
                1 => self.flags |= BOLD,
                2 => self.flags |= FAINT,
                3 => self.flags |= ITALIC,
                4 => self.ul = UL_SINGLE,
                5 | 6 => self.flags |= BLINK,
                7 => self.flags |= INV,
                9 => self.flags |= STRIKE,
                // xterm uses 21 as "double"; ECMA-48 would say "no bold". What
                // shows up in practice (bat, delta) is double.
                21 => self.ul = UL_DOUBLE,
                22 => self.flags &= !(BOLD | FAINT),
                23 => self.flags &= !ITALIC,
                24 => self.ul = UL_NONE,
                25 => self.flags &= !BLINK,
                27 => self.flags &= !INV,
                29 => self.flags &= !STRIKE,
                30..=37 => self.fg = Color::Idx((parse_u32(tok) - 30) as u8),
                38 => self.extended(&mut toks, Slot::Fg),
                39 => self.fg = Color::Default,
                40..=47 => self.bg = Color::Idx((parse_u32(tok) - 40) as u8),
                48 => self.extended(&mut toks, Slot::Bg),
                49 => self.bg = Color::Default,
                53 => self.flags |= OVERLINE,
                55 => self.flags &= !OVERLINE,
                58 => self.extended(&mut toks, Slot::Ul),
                59 => self.ul_color = Color::Default,
                90..=97 => self.fg = Color::Idx((parse_u32(tok) - 90 + 8) as u8),
                100..=107 => self.bg = Color::Idx((parse_u32(tok) - 100 + 8) as u8),
                _ => {},
            }
        }
    }

    fn pen_reset(&mut self) {
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.ul_color = Color::Default;
        self.flags = 0;
        self.ul = UL_NONE;
    }

    fn slot_set(&mut self, slot: Slot, color: Color) {
        match slot {
            Slot::Fg => self.fg = color,
            Slot::Bg => self.bg = color,
            Slot::Ul => self.ul_color = color,
        }
    }

    /// Extended color after 38/48/58: `;5;n` (256) or `;2;r;g;b` (truecolor).
    fn extended(&mut self, toks: &mut Toks<'_>, slot: Slot) {
        let color = match toks.next_num() {
            Some(5) => match toks.next_num() {
                Some(i) if i <= 255 => Color::Idx(i as u8),
                _ => return,
            },
            Some(2) => {
                let (Some(r), Some(g), Some(b)) =
                    (toks.next_num(), toks.next_num(), toks.next_num())
                else {
                    return;
                };
                Color::rgb(r, g, b)
            },
            _ => return,
        };
        self.slot_set(slot, color);
    }

    /// SGR with subparameters (`38:2::r:g:b`, `48:5:196`, `4:3`).
    fn sgr_sub(&mut self, tok: &[u8]) {
        let mut vals = [0u32; 8];
        let mut n = 0usize;
        for part in tok.split(|&b| b == b':') {
            if n >= vals.len() {
                break;
            }
            vals[n] = parse_u32(part);
            n += 1;
        }
        if n == 0 {
            return;
        }
        match vals[0] {
            38 | 48 | 58 => {
                let slot = match vals[0] {
                    38 => Slot::Fg,
                    48 => Slot::Bg,
                    _ => Slot::Ul,
                };
                let color = if n >= 2 && vals[1] == 2 {
                    if n < 5 {
                        return;
                    }
                    // the empty field is the omitted "color space": the last trio counts
                    Color::rgb(vals[n - 3], vals[n - 2], vals[n - 1])
                } else if n >= 3 && vals[1] == 5 {
                    Color::Idx(vals[2].min(255) as u8)
                } else {
                    return;
                };
                self.slot_set(slot, color);
            },
            4 => {
                // 4:0 removes the underline; 4:1..4:5 pick the style
                self.ul = match vals[1] {
                    0 => UL_NONE,
                    2 => UL_DOUBLE,
                    3 => UL_CURLY,
                    4 => UL_DOTTED,
                    5 => UL_DASHED,
                    _ => UL_SINGLE,
                };
            },
            0 => self.pen_reset(),
            _ => {},
        }
    }

    // ---------------------------------------------------------------- modes

    fn set_mode(&mut self, body: &[u8], private: bool, on: bool) {
        for t in body.split(|&b| b == b';') {
            let p = parse_u32(t);
            self.set_one(p, private, on);
        }
    }

    fn set_one(&mut self, p: u32, private: bool, on: bool) {
        if private {
            match p {
                1 => self.app_cursor = on,
                6 => {
                    // DECOM moves the cursor home as it flips: to the region top
                    // when it goes on, to the screen top when it goes off
                    self.origin = on;
                    self.cursor_to(0, 0);
                },
                7 => self.autowrap = on,
                25 => self.cursor_visible = on,
                47 | 1047 | 1049 => {
                    self.sync_count(|t| t.alt += 1);
                    if on {
                        self.enter_alt();
                    } else {
                        self.leave_alt();
                    }
                },
                1000 | 1002 | 1003 => {
                    self.mouse_mode = if on {
                        match p {
                            1000 => 1,
                            1002 => 2,
                            _ => 3,
                        }
                    } else {
                        0
                    }
                },
                1006 => self.mouse_sgr = on,
                1005 => self.mouse_utf8 = on,
                1016 => self.mouse_px = on,
                1004 => self.focus_report = on,
                2004 => self.bracketed_paste = on,
                2026 => {
                    if on && !self.sync_output {
                        // a window opens: the trace starts here, and the read
                        // that carries it counts as the first feed
                        self.sync_trace = Some(SyncTrace {
                            feeds: 1,
                            ..SyncTrace::default()
                        });
                    } else if !on {
                        // the close is what the app can time: the trace leaves
                        // with whatever the window carried
                        if let Some(t) = self.sync_trace.take() {
                            if self.sync_closed.len() < SYNC_TRACES_MAX {
                                self.sync_closed.push(t);
                            }
                        }
                    }
                    self.sync_output = on;
                },
                _ => {},
            }
        }
        if !private && p == 20 {
            // IRM (4) stays out: it would put a test in the run path of
            // `put_ascii_run`, which is the parser's hot path. LNM is a flag
            // read where LF already goes, so it costs nothing.
            self.lnm = on;
        }
    }

    /// The switch hands the screen to the child and takes it back; the history
    /// is not part of the exchange. `1049` asks for the *alternate* screen to be
    /// cleared, not the main one's saved lines — and the lines written on the
    /// alternate screen never enter that history either (`scroll_up` keeps them
    /// out), so nothing can refill it. Clearing here cost the reader every line
    /// that had scrolled out the first time an `mc` or a `less` started (`B63`).
    fn enter_alt(&mut self) {
        if self.alt.is_some() {
            return;
        }
        self.mark_global("enter_alt");
        self.graphics.enter_alt();
        self.save_cursor();
        self.abs_gen += 1;
        let blank = (0..self.rows).map(|_| Row::blank(self.cols)).collect();
        self.alt = Some(std::mem::replace(&mut self.grid, blank));
        self.cx = 0;
        self.cy = 0;
        self.top = 0;
        self.bottom = self.rows - 1;
        self.wrap_pending = false;
    }

    fn leave_alt(&mut self) {
        let Some(saved_grid) = self.alt.take() else {
            return;
        };
        self.mark_global("leave_alt");
        self.graphics.leave_alt();
        self.abs_gen += 1;
        self.grid = saved_grid;
        if self.grid.len() != self.rows
            || self.grid.first().map(super::grid::Row::len) != Some(self.cols)
        {
            crop_rows(&mut self.grid, self.cols, self.rows);
        }
        self.restore_cursor();
        self.top = 0;
        self.bottom = self.rows - 1;
    }

    fn mode_state(&self, p: u32, private: bool) -> u32 {
        if !private {
            return 0;
        }
        match p {
            1 => onoff(self.app_cursor),
            66 => onoff(self.keypad_app),
            6 => onoff(self.origin),
            7 => onoff(self.autowrap),
            25 => onoff(self.cursor_visible),
            47 | 1047 | 1049 => onoff(self.alt.is_some()),
            1000 => onoff(self.mouse_mode == 1),
            1002 => onoff(self.mouse_mode == 2),
            1003 => onoff(self.mouse_mode == 3),
            1004 => onoff(self.focus_report),
            1005 => onoff(self.mouse_utf8),
            1006 => onoff(self.mouse_sgr),
            1016 => onoff(self.mouse_px),
            2004 => onoff(self.bracketed_paste),
            2026 => onoff(self.sync_output),
            _ => 0,
        }
    }

    // ------------------------------------------------------------ dispatch

    #[allow(clippy::too_many_lines)] // dispatch table: splitting it only hides the map
    fn csi_dispatch(&mut self, final_: u8) {
        let body = std::mem::take(&mut self.csi);
        let prefix = self.csi_prefix;
        let inter = self.csi_inter;
        self.csi_prefix = 0;
        self.csi_inter = 0;
        let private = prefix == b'?';
        let p = |idx: usize, default: u32| param(&body, idx, default);

        match final_ {
            b'm' => self.sgr(&body),
            b'A' => self.cursor_up((p(0, 1).max(1)) as isize),
            b'B' => self.cursor_down((p(0, 1).max(1)) as isize),
            b'C' => {
                let k = (p(0, 1).max(1)) as isize;
                let cx = self.cx as isize;
                self.cursor_to(self.cursor_row(), cx + k);
            },
            b'D' => {
                let k = (p(0, 1).max(1)) as isize;
                let cx = self.cx as isize;
                self.cursor_to(self.cursor_row(), cx - k);
            },
            b'E' => {
                self.cursor_down((p(0, 1).max(1)) as isize);
                self.cx = 0;
            },
            b'F' => {
                self.cursor_up((p(0, 1).max(1)) as isize);
                self.cx = 0;
            },
            b'G' | b'`' => {
                let k = p(0, 1).max(1) as isize;
                self.cursor_to(self.cursor_row(), k - 1);
            },
            b'H' | b'f' => {
                let r = p(0, 1).max(1);
                let c = p(1, 1).max(1);
                if r == 1 && c == 1 {
                    // the jump that a repaint makes before drawing over itself
                    self.sync_count(|t| t.home += 1);
                }
                self.cursor_to(r as isize - 1, c as isize - 1);
            },
            b'I' => self.forward_tab(p(0, 1).max(1) as usize),
            b'Z' => self.backward_tab(p(0, 1).max(1) as usize),
            b'J' => self.erase_display(p(0, 0)),
            b'K' => self.erase_line(p(0, 0)),
            b'L' => self.insert_lines(p(0, 1).max(1) as usize),
            b'M' => self.delete_lines(p(0, 1).max(1) as usize),
            b'P' => self.delete_chars(p(0, 1).max(1) as usize),
            b'@' => self.insert_chars(p(0, 1).max(1) as usize),
            b'X' => self.erase_chars(p(0, 1).max(1) as usize),
            b'S' => self.scroll_up(p(0, 1).max(1) as usize),
            b'T' => self.scroll_down(p(0, 1).max(1) as usize),
            b'b' => {
                let n = p(0, 1).max(1) as usize;
                let ch = self.last_char;
                for _ in 0..n {
                    self.put_char(ch);
                }
            },
            b'c' => {
                if prefix == b'>' {
                    // DA2: the reply MUST start with '>'
                    self.child_out.extend_from_slice(b"\x1b[>1;100;0c");
                } else if body.is_empty() || body == b"0" {
                    self.child_out.extend_from_slice(b"\x1b[?62;4c");
                }
            },
            b'd' => {
                let r = p(0, 1).max(1);
                let cx = self.cx as isize;
                self.cursor_to(r as isize - 1, cx);
            },
            b'g' => match p(0, 0) {
                0 => {
                    if self.cx < self.tabstops.len() {
                        self.tabstops[self.cx] = false;
                    }
                },
                3 => {
                    self.tabstops.fill(false);
                },
                _ => {},
            },
            b'h' => self.set_mode(&body, private, true),
            b'l' => self.set_mode(&body, private, false),
            b'n' => match p(0, 0) {
                5 => self.child_out.extend_from_slice(b"\x1b[0n"),
                6 => {
                    if private {
                        self.child_out.extend_from_slice(
                            format!("\x1b[?{};{}R", self.cy + 1, self.cx + 1).as_bytes(),
                        );
                    } else {
                        self.child_out.extend_from_slice(
                            format!("\x1b[{};{}R", self.cy + 1, self.cx + 1).as_bytes(),
                        );
                    }
                },
                _ => {},
            },
            b'p' => {
                if inter == b'$' {
                    let ps = p(0, 0);
                    let st = self.mode_state(ps, private);
                    let pre = if private { "?" } else { "" };
                    self.child_out
                        .extend_from_slice(format!("\x1b[{pre}{ps};{st}$y").as_bytes());
                } else if inter == b'!' {
                    self.sync_count(|t| t.decstr += 1);
                    self.soft_reset(); // DECSTR
                }
            },
            b'q' => {
                if inter == b' ' {
                    let ps = p(0, 0);
                    self.app_cursor_style = if (1..=6).contains(&ps) {
                        let shape = ((ps - 1) / 2) as u8;
                        Some((shape, ps % 2 == 1))
                    } else {
                        None
                    };
                }
            },
            b'r' => {
                self.sync_count(|t| t.stbm += 1);
                let t = p(0, 1).max(1) as isize - 1;
                let b = p(1, self.rows as u32).max(1) as isize - 1;
                if t >= 0 && t < b && b < self.rows as isize {
                    self.top = t as usize;
                    self.bottom = b as usize;
                    self.cursor_to(0, 0);
                }
            },
            b's' => {
                if prefix == 0 {
                    self.save_cursor();
                }
            },
            // `u` has four owners: SCORC (no prefix) and the kitty keyboard
            // protocol (`?` queries, `>` pushes, `<` pops, `=` adjusts with
            // mode 1=replace, 2=add, 3=remove).
            b'u' => match prefix {
                0 => self.restore_cursor(),
                b'?' => self
                    .child_out
                    .extend_from_slice(format!("\x1b[?{}u", self.kbd_flags).as_bytes()),
                b'>' => {
                    if self.kbd_stack.len() < 64 {
                        self.kbd_stack.push(self.kbd_flags);
                    }
                    self.kbd_flags = p(0, 0);
                },
                b'<' => {
                    for _ in 0..p(0, 1).max(1) {
                        self.kbd_flags = self.kbd_stack.pop().unwrap_or(0);
                    }
                },
                b'=' => {
                    let flags = p(0, 0);
                    match p(1, 1) {
                        2 => self.kbd_flags |= flags,
                        3 => self.kbd_flags &= !flags,
                        _ => self.kbd_flags = flags,
                    }
                },
                _ => {},
            },
            b't' => match p(0, 0) {
                16 => {
                    let (cw, ch) = self.cell_size;
                    self.child_out
                        .extend_from_slice(format!("\x1b[6;{ch};{cw}t").as_bytes());
                },
                14 => {
                    // text area in PIXELS: it is what `kitten icat` asks for
                    // before sending the image (without it, it gives up with
                    // "terminal does not support reporting screen sizes").
                    // It is not answered here either — but here it is free,
                    // because the parser already carries the cell size.
                    let (cw, ch) = self.cell_size;
                    let w = self.cols as u32 * cw;
                    let h = self.rows as u32 * ch;
                    self.child_out
                        .extend_from_slice(format!("\x1b[4;{h};{w}t").as_bytes());
                },
                18 => {
                    self.child_out.extend_from_slice(
                        format!("\x1b[8;{};{}t", self.rows, self.cols).as_bytes(),
                    );
                },
                // the title stack: `Ps` is 0 (icon+title), 1 (icon) or 2 (title)
                // in xterm, and here there is one title only (`OSC 1` is ignored),
                // so every Ps saves the same thing and stays symmetric with its
                // `23` — without this the app that sets a title and pops on exit
                // left the tab stuck on its title forever (B37)
                22 if p(1, 0) <= 2 => self.title_push(),
                23 if p(1, 0) <= 2 => self.title_pop(),
                _ => {},
            },
            _ => {},
        }
        // give the buffer back (no realloc on the next escape)
        self.csi = body;
        self.csi.clear();
    }

    fn osc_dispatch(&mut self, data: &[u8]) {
        // "0;title", "2;title", "8;;url", "4;idx;rgb"...
        let mut it = data.splitn(2, |&b| b == b';');
        let cmd = it.next().unwrap_or_default();
        let arg = it.next().unwrap_or_default();
        match parse_u32(cmd) {
            0 | 2 => {
                // a cut in the middle of UTF-8 already becomes `�` through
                // `from_utf8_lossy`; the rest of the junk is what `sanitize_title`
                // removes (B4).
                let clean = sanitize_title(&String::from_utf8_lossy(arg));
                if !clean.is_empty() {
                    self.title = clean;
                }
            },
            // 4;index;color defines the palette, 4;index;? queries it
            4 => self.osc_palette(arg),
            10 => self.osc_color(10, arg),
            11 => self.osc_color(11, arg),
            12 => self.osc_color(12, arg),
            7 => self.osc_cwd(arg),
            8 => self.osc_link(arg),
            52 => self.osc_clipboard(arg),
            // `9;4;state[;progress]` is not a message — it is the mintty/ConEmu
            // progress form (which some TUIs send as a keepalive while busy).
            // Swallowed, not toasted: a 1 Hz repeater would pin the toast and
            // the badge forever, and `4;3` is a state, not a sentence.
            // 0 clears, 1 normal, 2 error, 3 indeterminate, 4 warning/paused. A
            // digit state we do not know is dropped whole: not the progress form,
            // and not a toast either (a 1 Hz repeater would pin one forever)
            9 => match Self::osc_progress(arg) {
                // `0`-`4` are the states this terminal knows; a digit it does not
                // know is not the progress form and is dropped whole
                Some((state @ 0..=4, pct)) => {
                    // the spec's mapping: the progress form feeds the root record
                    // while no rich report owns it, and it is why the form is
                    // swallowed instead of toasted
                    let outcome = self.status.from_progress(state, pct);
                    // the bar is always re-derived from the record (`state=0`
                    // removes it): one truth, not two
                    self.sync_progress();
                    self.trace.push(crate::trace::Event::Progress {
                        state,
                        pct,
                        outcome,
                        bar: self.progress.map(|p| (p.state, p.pct)),
                    });
                    // a final state is news, exactly like the same state reported
                    // by `7501` — the form carries no text, so the state's word is
                    // the text. A repeater changes nothing and says nothing.
                    if outcome == crate::trace::Outcome::Applied {
                        let word = self
                            .status
                            .root()
                            .filter(|r| r.state.notifies())
                            .map(|r| r.state.name().to_string());
                        if let Some(word) = word {
                            let shade = self.status.root().map_or(crate::notif::Shade::Info, |r| {
                                crate::notif::Shade::of(r.state)
                            });
                            self.notify(&crate::notif::Notif {
                                source: crate::notif::Source::Progress,
                                id: None,
                                title: None,
                                body: Some(word),
                                shade,
                            });
                        }
                    }
                },
                Some((state, pct)) => {
                    // a digit we give no meaning to: not this form, dropped whole
                    self.trace.push(crate::trace::Event::Progress {
                        state,
                        pct,
                        outcome: crate::trace::Outcome::Unknown,
                        bar: self.progress.map(|p| (p.state, p.pct)),
                    });
                },
                None => self.osc_notify(arg),
            },
            777 => self.osc_notify777(arg),
            // `OSC 133` prompt marks (`M-R`): the substrate of shell integration
            133 => self.osc_prompt(arg),
            // `OSC 7501` Program Status Protocol (`B47`): what the program says
            // it is doing, instead of the terminal guessing from the screen
            7501 => self.osc_status(arg),
            // iTerm2 inline image (`1337;File=…:base64`) — `B3`'s cheap half
            1337 => self.osc_iterm(arg),
            104 => self.osc_reset_palette(arg),
            110 => self.reset_color(10),
            111 => self.reset_color(11),
            112 => self.reset_color(12),
            _ => {},
        }
        self.osc_record(data);
    }

    /// `OSC 7501` (`B47`): a program reporting what it is doing.
    ///
    /// The root record drives the tab's progress bar (`OSC 9;4` presentation,
    /// already drawn), and the states that want the user become the notification
    /// `B8` already shows: badge on the tab, card on the workspace, toast with the
    /// decoded text.
    fn osc_status(&mut self, arg: &[u8]) {
        if arg == b"?" {
            // feature detection: the reply carries nothing from any report
            self.child_out
                .extend_from_slice(crate::status::Status::detection_reply());
            return;
        }
        let Some(applied) = self.status.report(arg) else {
            self.sync_progress();
            self.trace.push(crate::trace::Event::Status {
                body: String::from_utf8_lossy(arg).into_owned(),
                what: "no change",
            });
            return;
        };
        if applied.state.notifies() {
            // the title says *who and why* (`blocked · test`), the body is the
            // message the program wrote; the shade is the state's own, so a
            // `blocked` wears yellow and an `error` red
            let title = match applied.app.as_deref() {
                Some(app) if app != applied.text => {
                    format!("{} · {app}", applied.state.name())
                },
                _ => applied.state.name().to_string(),
            };
            self.notify(&crate::notif::Notif {
                source: crate::notif::Source::Osc7501,
                // the root record has no name: an unnamed report is not an
                // identity, so each one is its own entry
                id: (!applied.id.is_empty()).then(|| applied.id.clone()),
                title: Some(title),
                body: Some(applied.text.clone()),
                shade: crate::notif::Shade::of(applied.state),
            });
        }
        self.sync_progress();
    }

    /// What the wire delivered and the parser decided since the last call: the
    /// app stamps and writes it (`drain_tab`); the core only accumulates.
    pub fn take_trace(&mut self) -> Vec<crate::trace::Event> {
        self.trace.take()
    }

    /// The text of the newest notification, which is what a surface with one line
    /// shows. Empty when there is none.
    #[must_use]
    pub fn last_notification(&self) -> String {
        self.notifications
            .entries()
            .next()
            .map_or_else(String::new, crate::notif::Notif::label)
    }

    /// The app's own notification (`bell = visual`): the **list** belongs to the
    /// emulation's tab, but the policy that decides whether a BEL is a
    /// notification belongs to the app, so the app pushes it here instead of the
    /// parser doing it. The count does not move: the app already follows `bells`
    /// and would otherwise see the same event twice.
    pub fn push_notification(&mut self, source: crate::notif::Source) {
        self.notifications
            .push(source, None, None, None, crate::notif::Shade::Info);
    }

    fn sync_progress(&mut self) {
        use crate::status::State;
        // the bar follows the newest in-flight record (`B75`), not the root
        // only: a program that names its parts gets progress all the same
        let bar = match self.status.bar() {
            Some(r) if r.state.in_flight() => Some(Progress {
                // `9;4`'s shades: no percentage is indeterminate (cyan), a wait
                // for the user is the warning one, the rest is the normal one.
                // The shade is derived, never stored as a second meaning.
                state: match r.state {
                    State::Working if r.progress.is_none() => 3,
                    State::Working => 1,
                    _ => 4,
                },
                pct: r.progress,
            }),
            _ => None,
        };
        self.progress = bar;
    }

    /// `OSC 133` (`M-R`): the shell says where the prompt, the command and the
    /// output start, and how the command ended.
    ///
    /// The letter is all that matters: `B`/`C` may carry kitty's extensions
    /// (`;cl=line`, `;aid=…`) and `D` carries the exit status. An unknown letter
    /// is ignored — the protocol is a stream of hints, not a command, and a
    /// terminal that refuses unknown ones would break the next extension.
    fn osc_prompt(&mut self, arg: &[u8]) {
        use crate::prompt::{Mark, MarkKind};
        let mut it = arg.split(|&b| b == b';');
        let kind = match it.next().unwrap_or_default() {
            b"A" => MarkKind::PromptStart,
            b"B" => MarkKind::CommandStart,
            b"C" => MarkKind::OutputStart,
            b"D" => MarkKind::CommandEnd,
            _ => return,
        };
        // the status travels on `D`, as the first argument after the letter
        let status = if kind == MarkKind::CommandEnd {
            it.next()
                .and_then(|s| std::str::from_utf8(s).ok())
                .and_then(|s| s.trim().parse::<i32>().ok())
        } else {
            None
        };
        if kind == MarkKind::PromptStart {
            // a new prompt: the command that produced every record is gone, so
            // the whole record goes with it. What survives is the notification
            // (the tab's list and its mark), which is where "the user has not
            // seen it" lives now (`B47` kept it in the record, which nothing
            // draws any more)
            self.status.dismiss();
            self.sync_progress();
            self.trace.push(crate::trace::Event::Prompt);
        }
        if kind == MarkKind::CommandStart {
            self.cmd_start_ms = Some(self.now_ms);
        } else if kind == MarkKind::OutputStart && self.cmd_start_ms.is_none() {
            // shells that skip `B` still open with `C`: the wait started there
            self.cmd_start_ms = Some(self.now_ms);
        } else if kind == MarkKind::CommandEnd {
            self.command_done(status);
        }
        let line = self.scrolled as i64 + self.cy as i64;
        let first = self.abs_first();
        self.marks.push(Mark { line, kind, status }, first);
    }

    /// `133;D`: the shell says the command ended. A long command notifies,
    /// whatever the status was (`B74`) — through the same pipeline as every
    /// other source, so presence decides whether it marks the tab or only
    /// joins the list. Raised here, on the `D`, because the next prompt (`A`)
    /// drops the status records: read afterwards, there would be nothing to
    /// read.
    fn command_done(&mut self, status: Option<i32>) {
        let start = self.cmd_start_ms.take();
        let status = status.unwrap_or(0);
        let elapsed = self.now_ms.saturating_sub(start.unwrap_or(self.now_ms));
        if self.shell_notify_secs == 0 || elapsed < self.shell_notify_secs.saturating_mul(1000) {
            return;
        }
        let shade = if status == 0 {
            crate::notif::Shade::Info
        } else {
            crate::notif::Shade::Err
        };
        self.notify(&crate::notif::Notif {
            source: crate::notif::Source::Osc133,
            id: None,
            title: None,
            body: Some(format!("exit {status} · {}", Self::dur_text(elapsed))),
            shade,
        });
    }

    /// What the `133;D` entry carries as its duration: one decimal below a
    /// minute (`12.4 s`), minutes past it (`3m 07s`). Whole seconds would lie
    /// about a command that ran 10.9 s against a 10 s floor.
    fn dur_text(ms: u64) -> String {
        if ms < 60_000 {
            format!("{:.1} s", ms as f64 / 1000.0)
        } else {
            format!("{}m {:02}s", ms / 60_000, ms % 60_000 / 1000)
        }
    }

    /// Previous/next prompt start for the navigation (`M-R`), as an absolute
    /// line. `back` walks towards the history.
    #[must_use]
    pub fn prompt_line(&self, from: i64, back: bool) -> Option<i64> {
        if back {
            self.marks.prompt_before(from)
        } else {
            self.marks.prompt_after(from)
        }
    }

    /// `CSI 22 ; Ps t`: saves the title for the matching `23`.
    fn title_push(&mut self) {
        if self.title_stack.len() < TITLE_STACK_MAX {
            self.title_stack.push(self.title.clone());
        }
    }

    /// `CSI 23 ; Ps t`: gives the saved title back. An empty stack is not an
    /// error and not a reason to invent one: a `23` with nothing pushed leaves
    /// the title as it is.
    fn title_pop(&mut self) {
        if let Some(t) = self.title_stack.pop() {
            self.title = t;
        }
    }

    /// iTerm2's inline image: the bytes go through the same direct transmission
    /// the kitty path uses, and a refusal lands in `refused` for the app to show
    /// — the protocol has no error channel of its own (`B3`).
    fn osc_iterm(&mut self, arg: &[u8]) {
        let cursor = (
            u16::try_from(self.cx).unwrap_or(0),
            u16::try_from(self.cy).unwrap_or(0),
        );
        let grid = (
            u16::try_from(self.cols).unwrap_or(0),
            u16::try_from(self.rows).unwrap_or(0),
        );
        let mut out = self.graphics.iterm(arg, cursor, self.cell_size, grid);
        if let Some(msg) = out.refused.take() {
            self.refused = Some(msg);
        }
        self.apc_output(out);
    }

    /// What the child asked for and we refused, once (`B3`): the app turns it
    /// into a toast — silence would look like a broken image.
    pub fn take_refused(&mut self) -> Option<&'static str> {
        self.refused.take()
    }

    /// Stores the payload in the probe (a ring of `OSC_SEEN`), with the resulting title.
    fn osc_record(&mut self, data: &[u8]) {
        let truncated = data.len() > OSC_SEEN_MAX;
        let raw = data[..data.len().min(OSC_SEEN_MAX)].to_vec();
        if self.osc_seen.len() == OSC_SEEN {
            self.osc_seen.pop_front();
        }
        self.osc_seen.push_back(OscSeen {
            raw,
            truncated,
            title: self.title.clone(),
        });
    }

    /// Closes the in-flight OSC: dispatches what arrived and returns to the grid.
    fn osc_end(&mut self) {
        let data = std::mem::take(&mut self.osc);
        self.osc_dispatch(&data);
        self.osc = data;
        self.osc.clear();
        self.osc_over = false;
        self.osc_cap = MAX_OSC;
        self.state = State::Ground;
    }

    /// `OSC 52 ; Pc ; Pd`: the child sets the clipboard (`M-R`, the `ssh`
    /// case). `c` is CLIPBOARD, `p` is PRIMARY; anything else is ignored. The
    /// query form (`Pd == ?`, which would hand our clipboard to the child)
    /// is refused, and so is a body cut at `MAX_OSC` — cut base64 still
    /// decodes. An empty payload clears, as in xterm. The app drains the
    /// text with `take_clipboard`.
    fn osc_clipboard(&mut self, arg: &[u8]) {
        if self.osc_over {
            return;
        }
        let mut it = arg.splitn(2, |&b| b == b';');
        let (Some(pc), Some(pd)) = (it.next(), it.next()) else {
            return;
        };
        if pd == b"?" {
            // the query: answering would leak our clipboard to the child
            return;
        }
        let text = if pd.is_empty() {
            String::new()
        } else {
            let Ok(b64) = std::str::from_utf8(pd) else {
                return;
            };
            let Some(raw) = crate::graphics::base64_decode(b64) else {
                return;
            };
            let Some(text) = Self::clipboard_text(&raw) else {
                return;
            };
            text
        };
        let op = match pc {
            b"c" => ClipboardOp::Clipboard(text),
            b"p" => ClipboardOp::Primary(text),
            _ => return,
        };
        self.clipboard = Some(op);
    }

    /// A remote clipboard write (`OSC 52`), if one arrived since the last
    /// drain. The app sets CLIPBOARD/PRIMARY from it.
    pub fn take_clipboard(&mut self) -> Option<ClipboardOp> {
        self.clipboard.take()
    }

    /// Clipboard text: UTF-8 without C0 controls (tab/LF/CR are text, the
    /// rest is not). Binary or control junk from a remote never lands on the
    /// system clipboard silently — it is refused, not scrubbed.
    fn clipboard_text(data: &[u8]) -> Option<String> {
        let s = std::str::from_utf8(data).ok()?;
        if s.chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
        {
            return None;
        }
        Some(s.to_string())
    }

    /// `OSC 4;index;color` (defines) or `OSC 4;index;?` (queries).
    fn osc_palette(&mut self, arg: &[u8]) {
        let mut it = arg.splitn(2, |&b| b == b';');
        let Some(idx) = it.next().and_then(parse_u32_full) else {
            return;
        };
        let Some(spec) = it.next() else { return };
        let idx = idx as usize;
        if idx >= 16 {
            return;
        }
        if spec == b"?" {
            let (r, g, b) = self.theme.rgb(self.theme.palette[idx], (0, 0, 0));
            self.reply_color(4, Some(idx), r, g, b);
            return;
        }
        if let Some(c) = parse_osc_color(spec) {
            self.theme.palette[idx] = c;
            self.theme_dirty = true;
        }
    }

    /// `OSC 7`: `file://host/path` — where the shell says it is. The host is
    /// ignored (the path is what the shell sees) and the payload is
    /// percent-decoded, since a directory with a space arrives as `%20`. An
    /// empty payload is the shell saying it does not know, and that clears.
    fn osc_cwd(&mut self, arg: &[u8]) {
        if arg.is_empty() {
            self.cwd = None;
            return;
        }
        let Some(rest) = arg.strip_prefix(b"file://") else {
            // a payload that is not the documented form is not a reason to
            // throw away a cwd that was already known
            return;
        };
        // `//host/path`: the authority ends at the first `/`
        let path = rest
            .iter()
            .position(|&b| b == b'/')
            .map_or(&rest[rest.len()..], |i| &rest[i..]);
        if path.is_empty() {
            self.cwd = None;
            return;
        }
        let decoded = percent_decode(path);
        let clean: String = String::from_utf8_lossy(&decoded)
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        self.cwd = if clean.is_empty() { None } else { Some(clean) };
    }

    /// `OSC 8`: `8;params;uri` opens a hyperlink over the cells that follow, and
    /// an empty `uri` closes it. The `params` (`id=`, `foo=`) are ignored: they
    /// exist for the running app to correlate its own spans, and nothing here
    /// needs the correlation.
    /// The mintty/ConEmu progress form hiding inside `OSC 9`: `4;state` with
    /// an optional `;progress`, all digits. An iTerm2 message has no such
    /// shape.
    fn osc_progress(arg: &[u8]) -> Option<(u8, Option<u8>)> {
        let mut parts = arg.split(|&b| b == b';');
        if parts.next() != Some(b"4".as_slice()) {
            return None;
        }
        let rest: Vec<&[u8]> = parts.collect();
        if !(1..=2).contains(&rest.len())
            || !rest
                .iter()
                .all(|p| !p.is_empty() && p.iter().all(u8::is_ascii_digit))
        {
            return None;
        }
        let num = |p: &[u8]| -> u8 {
            p.iter()
                .fold(0u32, |acc, d| acc * 10 + u32::from(d - b'0'))
                .min(255) as u8
        };
        Some((num(rest[0]), rest.get(1).map(|p| num(p).min(100))))
    }

    /// A notification the child wants seen (`M-K`): one entry in the tab's list
    /// and the count moves, exactly like `bells` — what the app does about it
    /// (mark, urgency, beep) is the app's business.
    fn notify(&mut self, notif: &crate::notif::Notif) {
        if self.notifications.push(
            notif.source,
            notif.id.as_deref(),
            notif.title.as_deref(),
            notif.body.as_deref(),
            notif.shade,
        ) {
            self.notifs = self.notifs.wrapping_add(1);
            self.trace.push(crate::trace::Event::Notify {
                source: notif.source,
                id: notif.id.clone(),
                shade: notif.shade,
                text: notif.text(),
            });
        }
    }

    /// `OSC 9;message` (iTerm2): the whole payload is the message.
    fn osc_notify(&mut self, arg: &[u8]) {
        let text = String::from_utf8_lossy(arg).into_owned();
        self.notify(&crate::notif::Notif {
            source: crate::notif::Source::Osc9,
            id: None,
            title: None,
            body: Some(text),
            shade: crate::notif::Shade::Info,
        });
    }

    /// `OSC 777;notify;title;body` (urxvt): only `notify` is a notification,
    /// the other subcommands stay discarded with everything else unknown.
    fn osc_notify777(&mut self, arg: &[u8]) {
        let text = String::from_utf8_lossy(arg);
        let mut it = text.splitn(3, ';');
        if it.next() != Some("notify") {
            return;
        }
        // the two parts are a title and a body, not one string: the list keeps
        // them apart and flattens only when it has one line to fill
        let title = it.next().unwrap_or_default();
        let body = it.next().unwrap_or_default();
        self.notify(&crate::notif::Notif {
            source: crate::notif::Source::Osc777,
            id: None,
            title: (!title.is_empty()).then(|| title.to_string()),
            body: (!body.is_empty()).then(|| body.to_string()),
            shade: crate::notif::Shade::Info,
        });
    }

    fn osc_link(&mut self, arg: &[u8]) {
        let uri = arg
            .iter()
            .position(|&b| b == b';')
            .map_or(&arg[arg.len()..], |i| &arg[i + 1..]);
        if uri.is_empty() {
            self.link = 0;
            return;
        }
        // the URI comes from the child and ends up in a menu/`xdg-open`: a
        // control character in it is never part of a real link
        let clean: String = String::from_utf8_lossy(uri)
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        if clean.is_empty() {
            self.link = 0;
            return;
        }
        // repeated URIs are the normal case (`ls` re-links every name): the span
        // reuses the index instead of growing the table
        if let Some(i) = self.links.iter().position(|l| *l == clean) {
            self.link = i as u16 + 1;
            return;
        }
        if self.links.len() >= LINK_MAX {
            // a child that invents a URI per cell must not grow this without a
            // limit: past the cap the text is still text, it just has no link
            self.link = 0;
            return;
        }
        self.links.push(clean);
        self.link = self.links.len() as u16;
    }

    /// URI of the link a cell carries, if any.
    #[must_use]
    pub fn link_uri(&self, id: u16) -> Option<&str> {
        id.checked_sub(1)
            .and_then(|i| self.links.get(i as usize))
            .map(String::as_str)
    }

    /// `OSC 10/11/12`: text color, background and cursor — query (`?`) or define.
    fn osc_color(&mut self, which: u32, arg: &[u8]) {
        let current = match which {
            10 => self.theme.fg,
            11 => self.theme.bg,
            _ => self.theme.cursor,
        };
        if arg == b"?" {
            let (r, g, b) = self.theme.rgb(current, (0, 0, 0));
            self.reply_color(which, None, r, g, b);
            return;
        }
        let Some(c) = parse_osc_color(arg) else {
            return;
        };
        match which {
            10 => self.theme.fg = c,
            11 => self.theme.bg = c,
            _ => self.theme.cursor = c,
        }
        self.theme_dirty = true;
    }

    /// Color reply: `OSC 4;i;rgb:…` or `OSC 1x;rgb:…` (4 digits, like xterm;
    /// the 2-digit `rgb:rr/gg/bb` is also accepted on the other side).
    fn reply_color(&mut self, which: u32, idx: Option<usize>, r: u8, g: u8, b: u8) {
        let body = match idx {
            Some(i) => format!("4;{i};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}"),
            None => format!("{which};rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}"),
        };
        self.child_out
            .extend_from_slice(format!("\x1b]{body}\x1b\\").as_bytes());
    }

    /// `OSC 104` (whole palette or the listed indices) goes back to the default.
    fn osc_reset_palette(&mut self, arg: &[u8]) {
        let idxs: Vec<usize> = if arg.is_empty() {
            (0..16).collect()
        } else {
            arg.split(|&b| b == b';')
                .filter_map(parse_u32_full)
                .map(|i| i as usize)
                .filter(|i| *i < 16)
                .collect()
        };
        for i in idxs {
            self.theme.palette[i] = Color::Idx(i as u8);
        }
        self.theme_dirty = true;
    }

    fn reset_color(&mut self, which: u32) {
        let d = Theme::default();
        match which {
            10 => self.theme.fg = d.fg,
            11 => self.theme.bg = d.bg,
            _ => self.theme.cursor = d.cursor,
        }
        self.theme_dirty = true;
    }

    // ---------------------------------------------------------------- parser

    pub fn feed(&mut self, bytes: &[u8]) {
        // one test per read, not per byte: the trace only grows while a `?2026`
        // window is open, which is the rare case
        if let Some(t) = self.sync_trace.as_mut() {
            t.feeds += 1;
            t.bytes += bytes.len() as u64;
        }
        let mut i = 0usize;
        while i < bytes.len() {
            match self.state {
                State::Ground => {
                    let start = i;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c < 0x20 || c == 0x7f || c >= 0x80 {
                            break;
                        }
                        i += 1;
                    }
                    if i > start {
                        self.put_ascii_run(&bytes[start..i]);
                    }
                    if i < bytes.len() {
                        let c = bytes[i];
                        i += 1;
                        if c >= 0x80 {
                            self.utf8_byte(c);
                        } else {
                            self.control(c);
                        }
                    }
                },
                State::Escape => {
                    let b = bytes[i];
                    i += 1;
                    self.escape(b);
                },
                State::EscInter => {
                    let b = bytes[i];
                    i += 1;
                    self.escape_inter(b);
                },
                State::Csi => {
                    let b = bytes[i];
                    i += 1;
                    self.csi_byte(b);
                },
                State::Osc => {
                    let b = bytes[i];
                    i += 1;
                    match b {
                        0x07 => self.osc_end(),
                        0x1b => self.state = State::OscEsc,
                        // 8-bit ST (the `\x9c` many clients send instead of
                        // `ESC \`): without it the OSC never closed and
                        // swallowed everything until the next BEL.
                        0x9c if st_c1(&self.osc) => {
                            if self.osc.last() == Some(&0xc2) {
                                self.osc.pop(); // U+009C in UTF-8
                            }
                            self.osc_end();
                        },
                        _ => {
                            if self.osc.len() < self.osc_cap {
                                self.osc.push(b);
                                // the only OSC that may grow past 4 KiB, decided
                                // once: at the tenth byte the form is known
                                if self.osc_cap == MAX_OSC && self.osc == ITERM_PREFIX {
                                    self.osc_cap = MAX_OSC_IMAGE;
                                }
                            } else {
                                self.osc_over = true;
                            }
                        },
                    }
                },
                State::OscEsc => {
                    let b = bytes[i];
                    i += 1;
                    // `ESC \` is ST. Any other ESC in the body is a broken
                    // sequence: dispatch what already came and treat the byte as
                    // the one that would follow an ESC. Pushing the ESC back into
                    // the body (as was done) glued the `ESC ]` of the next title
                    // inside this one's title — the B4 junk.
                    self.osc_end();
                    if b != b'\\' {
                        self.escape(b);
                    }
                },
                State::Str => {
                    let b = bytes[i];
                    i += 1;
                    self.str_byte(b);
                },
                State::StrEsc => {
                    let b = bytes[i];
                    i += 1;
                    if b == b'\\' {
                        self.state = State::Ground;
                        if std::mem::take(&mut self.apc_active) {
                            self.apc_done();
                        }
                        if std::mem::take(&mut self.dcs_active) {
                            self.dcs_done();
                        }
                    } else {
                        self.state = State::Str;
                    }
                },
            }
        }
    }

    /// One byte inside DCS/APC/PM/SOS: the APC keeps its body for the kitty
    /// protocol, the DCS keeps its own for sixel (`B3`), and PM/SOS keep nothing.
    fn str_byte(&mut self, b: u8) {
        let body: &[u8] = if self.apc_active { &self.apc } else { &[] };
        // the DCS of a sixel is a picture: it needs a body to decode, so the two
        // kinds share the cap and the closing, and differ in what they do with it
        let keep = self.apc_active || self.dcs_active;
        if b == 0x1b {
            self.state = State::StrEsc;
        } else if b == 0x9c && st_c1(body) {
            // 8-bit ST also closes DCS/APC/PM/SOS
            if self.apc.last() == Some(&0xc2) {
                self.apc.pop();
            }
            if self.dcs.last() == Some(&0xc2) {
                self.dcs.pop();
            }
            self.state = State::Ground;
            if std::mem::take(&mut self.apc_active) {
                self.apc_done();
            }
            if std::mem::take(&mut self.dcs_active) {
                self.dcs_done();
            }
        } else if keep {
            // cap: a transmission that never closes must not grow without bound
            if self.apc_active && self.apc.len() < crate::graphics::MAX_APC {
                self.apc.push(b);
            }
            if self.dcs_active && self.dcs.len() < crate::graphics::MAX_APC {
                self.dcs.push(b);
            }
        }
    }

    /// A complete APC arrived: kitty images go through here (H1).
    fn apc_done(&mut self) {
        let body = std::mem::take(&mut self.apc);
        let cursor = (
            u16::try_from(self.cx).unwrap_or(0),
            u16::try_from(self.cy).unwrap_or(0),
        );
        let grid = (
            u16::try_from(self.cols).unwrap_or(0),
            u16::try_from(self.rows).unwrap_or(0),
        );
        let mut out = self.graphics.handle(&body, cursor, self.cell_size, grid);
        // `t=f`/`t=t`/`t=s`: the payload named a file — it waits for the app,
        // which answers with `serve_images` (`B1`)
        if let Some(req) = out.file.take() {
            self.image_files.push(req);
        }
        self.apc_output(out);
    }

    /// A complete DCS arrived: `DECRQSS`/`XTGETTCAP` are answered from the
    /// state the terminal really keeps (`B73`), the sixel kind becomes a
    /// picture, and everything else (the fonts) is still not ours.
    fn dcs_done(&mut self) {
        let dcs = std::mem::take(&mut self.dcs);
        if dcs.starts_with(b"$q") {
            self.dcs_decrqss(&dcs[2..]);
            return;
        }
        if dcs.starts_with(b"+q") {
            Self::dcs_xtgettcap(&dcs[2..], &mut self.child_out);
            return;
        }
        let cursor = (
            u16::try_from(self.cx).unwrap_or(0),
            u16::try_from(self.cy).unwrap_or(0),
        );
        let grid = (
            u16::try_from(self.cols).unwrap_or(0),
            u16::try_from(self.rows).unwrap_or(0),
        );
        let out = self.graphics.sixel(&dcs, cursor, self.cell_size, grid);
        self.apc_output(out);
    }

    /// `DECRQSS` (`DCS $ q Pt ST`): what a setting of ours is. Only what the
    /// terminal really keeps is answerable — `m` (the pen), `r` (the scroll
    /// region), `SP q` (the cursor style) — and everything else gets the
    /// standard negative, which is the honest answer for a setting we do not
    /// have (`"p` asks for a conformance level we never claimed).
    fn dcs_decrqss(&mut self, pt: &[u8]) {
        let body = match pt {
            b"m" => Some(self.sgr_report()),
            b"r" => Some(format!("{};{}r", self.top + 1, self.bottom + 1)),
            b" q" => {
                // `DECSCUSR`: `ps = shape * 2 + blink`. Without one the cursor
                // still has a shape — the shipped default (steady block) — and
                // that is what is answered until the app syncs the configured
                // one (`CONFIG.md`: the core never sees the config).
                let ps = self
                    .app_cursor_style
                    .map_or(2, |(shape, blink)| shape * 2 + u8::from(!blink) + 1);
                Some(format!("{ps} q"))
            },
            _ => None,
        };
        match body {
            Some(pt) => self
                .child_out
                .extend_from_slice(format!("\x1bP1$r{pt}\x1b\\").as_bytes()),
            None => self.child_out.extend_from_slice(b"\x1bP0$r\x1b\\"),
        }
    }

    /// The pen as a status string: `0` plus whatever is set — attributes,
    /// then the foreground, then the background — so a program that saved it
    /// can send it back. What has no classic number keeps the colon shape the
    /// setter takes (`4:3` is curly, as in the `SGR 4:n` the grid draws).
    fn sgr_report(&self) -> String {
        use crate::cell::{BLINK, BOLD, FAINT, INV, ITALIC, OVERLINE, STRIKE};
        let mut p = vec!["0".to_string()];
        if self.flags & BOLD != 0 {
            p.push("1".to_string());
        }
        if self.flags & FAINT != 0 {
            p.push("2".to_string());
        }
        if self.flags & ITALIC != 0 {
            p.push("3".to_string());
        }
        match self.ul {
            crate::cell::UL_SINGLE => p.push("4".to_string()),
            crate::cell::UL_DOUBLE => p.push("4:2".to_string()),
            crate::cell::UL_CURLY => p.push("4:3".to_string()),
            crate::cell::UL_DOTTED => p.push("4:4".to_string()),
            crate::cell::UL_DASHED => p.push("4:5".to_string()),
            _ => {},
        }
        if self.flags & BLINK != 0 {
            p.push("5".to_string());
        }
        if self.flags & INV != 0 {
            p.push("7".to_string());
        }
        if self.flags & STRIKE != 0 {
            p.push("9".to_string());
        }
        if self.flags & OVERLINE != 0 {
            p.push("53".to_string());
        }
        Self::color_report(&mut p, self.fg, 30, 38);
        Self::color_report(&mut p, self.bg, 40, 48);
        if let Color::Rgb(red, green, blue) = self.ul_color {
            p.push(format!("58;2;{red};{green};{blue}"));
        }
        p.join(";") + "m"
    }

    /// One colour as SGR parameters: the low eight on their classic numbers,
    /// the high eight on the bright ones, truecolor on `38/48;2`. `Default`
    /// contributes nothing — it is the absence of a setting, not a colour.
    fn color_report(p: &mut Vec<String>, col: Color, base: u8, ext: u8) {
        match col {
            Color::Default => {},
            Color::Idx(n) if n < 8 => p.push((base + n).to_string()),
            Color::Idx(n) => p.push((base + 60 + n - 8).to_string()),
            Color::Rgb(red, green, blue) => {
                p.push(format!("{ext};2;{red};{green};{blue}"));
            },
        }
    }

    /// `XTGETTCAP` (`DCS + q <hex names> ST`): termcap names the program wants
    /// the value of. The table only holds what is true here: the palette is
    /// 256 entries (`Co`, as the `TERM` we export says), truecolor is 8 bits
    /// per channel (`RGB`, as the `COLORTERM` we export says), and the name is
    /// ours (`TN`, as the `TERM_PROGRAM` we export says — a kitty check fails
    /// here, which is correct: this is not kitty). The key names stay out on
    /// purpose: what a key sends depends on the keyboard flags in force
    /// (legacy vs kitty protocol), so a static answer would be a lie in two
    /// modes out of three — they get the negative like every unknown name.
    fn dcs_xtgettcap(hex: &[u8], child_out: &mut Vec<u8>) {
        let names = Self::hex_decode(hex).unwrap_or_default();
        let mut entries = Vec::new();
        for name in names.split(|&b| b == b';') {
            let value = match name {
                b"Co" => Some("256"),
                b"TN" => Some("tortty"),
                b"RGB" => Some("8"),
                _ => None,
            };
            if let Some(v) = value {
                // the name goes back in hex, as it arrived: `Co` echoes as
                // `436F`, not as the decoded word
                entries.push(format!(
                    "{}={}",
                    Self::hex_encode(name),
                    Self::hex_encode(v.as_bytes())
                ));
            }
        }
        if entries.is_empty() {
            child_out.extend_from_slice(b"\x1bP0+r\x1b\\");
        } else {
            // entries are hex already; the `;` between them is literal
            child_out.extend_from_slice(format!("\x1bP1+r{}\x1b\\", entries.join(";")).as_bytes());
        }
    }

    /// Even-length ASCII hex to bytes; `None` on any other byte (an odd tail,
    /// a `Z`). Both cases spell the word the same in upper and lower case.
    fn hex_decode(hex: &[u8]) -> Option<Vec<u8>> {
        if !hex.len().is_multiple_of(2) {
            return None;
        }
        let digit = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        };
        let (pairs, _) = hex.as_chunks::<2>();
        pairs
            .iter()
            .map(|c| Some(digit(c[0])? << 4 | digit(c[1])?))
            .collect()
    }

    /// Bytes to uppercase ASCII hex, the case the other terminals answer in.
    fn hex_encode(bytes: &[u8]) -> String {
        const H: &[u8; 16] = b"0123456789ABCDEF";
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            s.push(H[(b >> 4) as usize] as char);
            s.push(H[(b & 15) as usize] as char);
        }
        s
    }

    /// Frames the responses and moves the cursor — the same tail for an APC
    /// that arrived and for a file the app loaded later (`B1`).
    fn apc_output(&mut self, out: crate::graphics::Output) {
        for r in out.responses {
            self.child_out.extend_from_slice(b"\x1b_");
            self.child_out.extend_from_slice(&r);
            self.child_out.extend_from_slice(b"\x1b\\");
        }
        // H10: without `C=1` the cursor goes down the rows the image took. The
        // picture itself moves up with the content, so a `scroll_up` here is what
        // gives a tall image the room it needs (`B3`).
        if out.move_cursor {
            let lines = out.lines.min(u16::try_from(self.rows).unwrap_or(u16::MAX));
            for _ in 0..lines {
                self.newline();
            }
            self.cx = 0;
        }
        self.touch_all();
    }

    /// Serves pending image-file loads (`B1`): the app reads (and, for `t=t`,
    /// deletes) the file — the core never touches the filesystem. Called from
    /// the drain so the answers ride the same frame's `child_out`.
    pub fn serve_images(&mut self, mut load: impl FnMut(&str, bool) -> Option<Vec<u8>>) {
        for req in std::mem::take(&mut self.image_files) {
            let out = match load(&req.path, req.delete) {
                Some(data) => self.graphics.provide(&req, &data),
                None => crate::graphics::Graphics::missing(&req),
            };
            self.apc_output(out);
        }
    }

    fn control(&mut self, b: u8) {
        match b {
            0x07 => self.bells = self.bells.wrapping_add(1),
            0x08 => {
                self.cx = self.cx.saturating_sub(1);
                self.wrap_pending = false;
            },
            0x09 => {
                self.wrap_pending = false;
                self.forward_tab(1);
            },
            0x0a..=0x0c => {
                self.wrap_pending = false;
                if self.lnm {
                    self.cx = 0;
                }
                self.newline();
            },
            0x0d => {
                self.cx = 0;
                self.wrap_pending = false;
            },
            0x0e => self.use_g1 = true,
            0x0f => self.use_g1 = false,
            0x18 | 0x1a => {
                self.state = State::Ground;
                self.csi.clear();
            },
            0x1b => {
                self.state = State::Escape;
                self.esc_inter = 0;
            },
            _ => {},
        }
    }

    fn escape(&mut self, b: u8) {
        match b {
            b'[' => {
                self.state = State::Csi;
                self.csi.clear();
                self.csi_prefix = 0;
                self.csi_inter = 0;
            },
            b']' => {
                self.state = State::Osc;
                self.osc.clear();
                self.osc_over = false;
            },
            b'_' => {
                // APC: only this kind is content (kitty image); DCS/PM/SOS stay
                // discarded until ST
                self.apc.clear();
                self.apc_active = true;
                self.state = State::Str;
            },
            b'P' => {
                self.apc_active = false;
                self.dcs.clear();
                self.dcs_active = true;
                self.state = State::Str;
            },
            b'^' | b'X' => {
                self.apc_active = false;
                self.dcs_active = false;
                self.state = State::Str;
            },
            0x20..=0x2f => {
                self.esc_inter = b;
                self.state = State::EscInter;
            },
            b'7' => {
                self.save_cursor();
                self.state = State::Ground;
            },
            b'8' => {
                self.restore_cursor();
                self.state = State::Ground;
            },
            b'D' => {
                self.state = State::Ground;
                self.wrap_pending = false;
                self.index();
            },
            b'E' => {
                self.state = State::Ground;
                self.cx = 0;
                self.wrap_pending = false;
                self.index();
            },
            b'M' => {
                self.state = State::Ground;
                self.reverse_index();
            },
            b'H' => {
                self.state = State::Ground;
                if self.cx < self.tabstops.len() {
                    self.tabstops[self.cx] = true;
                }
            },
            b'c' => {
                self.state = State::Ground;
                self.reset();
            },
            // DECKPAM / DECKPNM: the keypad application mode. The state is kept
            // (DECRQM 66 answers it, DECSTR/RIS clear it) and it goes no further:
            // see `keypad_app` for why the bytes do not change.
            b'=' => {
                self.keypad_app = true;
                self.state = State::Ground;
            },
            b'>' => {
                self.keypad_app = false;
                self.state = State::Ground;
            },
            _ => {
                // ESC \ (ST) and the rest: no effect
                self.state = State::Ground;
            },
        }
    }

    fn escape_inter(&mut self, b: u8) {
        match self.esc_inter {
            b'(' => {
                self.g0_dec = b == b'0';
                self.state = State::Ground;
            },
            b')' => {
                self.g1_dec = b == b'0';
                self.state = State::Ground;
            },
            b'#' if b == b'8' => {
                // DECALN: screen full of 'E' (used by alignment tests)
                let cell = Cell {
                    ch: 'E',
                    ..Cell::BLANK
                };
                for y in 0..self.rows {
                    self.grid[y].cells.fill(cell);
                }
                self.touch_all();
                self.state = State::Ground;
            },
            _ => self.state = State::Ground,
        }
    }

    fn csi_byte(&mut self, b: u8) {
        match b {
            0x1b => {
                self.state = State::Escape;
                self.esc_inter = 0;
            },
            0x00..=0x1f => {},
            0x20..=0x2f => self.csi_inter = b,
            0x40..=0x7e => {
                self.state = State::Ground;
                self.csi_dispatch(b);
            },
            _ => {
                if self.csi.is_empty() && matches!(b, b'?' | b'>' | b'=' | b'<') {
                    self.csi_prefix = b;
                } else if self.csi.len() < 128 {
                    self.csi.push(b);
                }
            },
        }
    }

    fn utf8_byte(&mut self, b: u8) {
        loop {
            if self.utf8_need == 0 {
                if b < 0x80 {
                    self.put_char(b as char);
                    return;
                }
                self.utf8_need = match b {
                    0xc2..=0xdf => 1,
                    0xe0..=0xef => 2,
                    0xf0..=0xf4 => 3,
                    _ => {
                        self.put_char(REPLACEMENT);
                        return;
                    },
                };
                self.utf8[0] = b;
                self.utf8_len = 1;
                return;
            }
            if b & 0xc0 == 0x80 {
                self.utf8[self.utf8_len as usize] = b;
                self.utf8_len += 1;
                self.utf8_need -= 1;
                if self.utf8_need == 0 {
                    let len = self.utf8_len as usize;
                    match std::str::from_utf8(&self.utf8[..len]) {
                        Ok(s) => {
                            if let Some(ch) = s.chars().next() {
                                self.put_char(ch);
                            }
                        },
                        Err(_) => self.put_char(REPLACEMENT),
                    }
                }
                return;
            }
            // invalid continuation: emit U+FFFD and reprocess this byte
            self.put_char(REPLACEMENT);
            self.utf8_need = 0;
            self.utf8_len = 0;
        }
    }

    // -------------------------------------------------------------- reading

    pub fn row_text(&self, y: usize) -> String {
        match self.grid.get(y) {
            Some(row) => row.text(),
            None => String::new(),
        }
    }

    /// Visible rows, with `off` scrollback rows above the top of the live area.
    pub fn visible_rows(&self, off: usize) -> Vec<&Row> {
        let sb = self.scrollback.len();
        let off = off.min(sb);
        let start = sb - off;
        let mut out = Vec::with_capacity(self.rows);
        for i in start..start + self.rows {
            if i < sb {
                out.push(&self.scrollback[i]);
            } else if let Some(r) = self.grid.get(i - sb) {
                out.push(r);
            }
        }
        out
    }

    pub fn history_text(&self, start: usize, count: usize) -> Vec<String> {
        self.scrollback
            .iter()
            .skip(start)
            .take(count)
            .map(super::grid::Row::text)
            .collect()
    }

    pub fn take_child_out(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.child_out)
    }

    pub fn in_alt_screen(&self) -> bool {
        self.alt.is_some()
    }

    /// The screen that is the terminal's own: the live grid, or the one stashed
    /// under the alt screen while an app owns the view. The snapshot reads this
    /// one — a photograph of `vim` is not what anyone wants back (`B45`).
    pub fn primary(&self) -> &[Row] {
        self.alt.as_deref().unwrap_or(&self.grid)
    }

    /// Feeds a **decoration**: the rows it writes are marked (`Row::chrome`) and
    /// never travel in the snapshot (`B48`). It is for the line that separates a
    /// restored picture from the run that follows it — content the terminal
    /// wrote, not content the child wrote.
    pub fn feed_chrome(&mut self, bytes: &[u8]) {
        self.chrome_feed = true;
        self.feed(bytes);
        self.chrome_feed = false;
    }

    /// The whole content (history + primary screen) as ANSI: see
    /// [`crate::snapshot`]. Feeding it into a fresh `Term` rebuilds the picture.
    pub fn snapshot(&self) -> Vec<u8> {
        crate::snapshot::snapshot(self)
    }
}

#[inline]
fn onoff(v: bool) -> u32 {
    if v {
        1
    } else {
        2
    }
}

/// Numbers of the CSI body, without allocation: `/` is a separator and `:` is a subparameter.
struct Toks<'a> {
    body: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> Toks<'a> {
    fn new(body: &'a [u8]) -> Self {
        Toks {
            body,
            pos: 0,
            done: false,
        }
    }
    fn next(&mut self) -> Option<&'a [u8]> {
        if self.done {
            return None;
        }
        let start = self.pos;
        let mut i = start;
        while i < self.body.len() && self.body[i] != b';' {
            i += 1;
        }
        if i >= self.body.len() {
            self.done = true;
        } else {
            self.pos = i + 1;
        }
        Some(&self.body[start..i])
    }
    fn next_num(&mut self) -> Option<u32> {
        self.next().map(parse_u32)
    }
}

#[inline]
/// Color of `OSC 4/10/11/12`: `rgb:RR/GG/BB`, `rgb:RRRR/GGGG/BBBB` or `#RRGGBB`.
fn parse_osc_color(spec: &[u8]) -> Option<Color> {
    let s = std::str::from_utf8(spec).ok()?.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            return Some(Color::rgb(
                u32::from_str_radix(&hex[0..2], 16).ok()?,
                u32::from_str_radix(&hex[2..4], 16).ok()?,
                u32::from_str_radix(&hex[4..6], 16).ok()?,
            ));
        }
        return None;
    }
    let rest = s.strip_prefix("rgb:")?;
    let mut it = rest.split('/');
    let mut channel = || -> Option<u32> {
        let v = it.next()?;
        match v.len() {
            // xterm sends 4 digits and the value is the duplicated byte (rrrr)
            4 => u32::from_str_radix(&v[0..2], 16).ok(),
            2 | 1 => u32::from_str_radix(v, 16).ok(),
            _ => None,
        }
    };
    let (r, g, b) = (channel()?, channel()?, channel()?);
    Some(Color::rgb(r, g, b))
}

/// Like `parse_u32`, but `None` when the text is not digits only (`OSC 4`
/// needs to tell "no index" from "index 0").
fn parse_u32_full(t: &[u8]) -> Option<u32> {
    let s = std::str::from_utf8(t).ok()?;
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn parse_u32(t: &[u8]) -> u32 {
    let mut v: u32 = 0;
    for &c in t {
        if c.is_ascii_digit() {
            v = v.saturating_mul(10).saturating_add(u32::from(c - b'0'));
        } else {
            break;
        }
    }
    v
}

/// Percent-decoding of a URI path: `%XX` becomes the byte. Anything else — a
/// lone `%`, a truncated escape, a non-hex digit — stays as it is written, so a
/// path that already has a `%` does not turn into a different path.
fn percent_decode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0usize;
    while i < s.len() {
        let hex = |b: u8| (b as char).to_digit(16).map(|v| v as u8);
        match (
            s[i],
            s.get(i + 1).copied().and_then(hex),
            s.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push(hi * 16 + lo);
                i += 3;
            },
            (b, _, _) => {
                out.push(b);
                i += 1;
            },
        }
    }
    out
}

/// Title fit to draw: no control character, no invisible/direction mark. The
/// OSC payload comes from the child and the cwd can be a directory name with any
/// byte; neither may touch the label layout (a `U+202E` would reverse the rest of
/// the tab text).
pub fn sanitize_title(s: &str) -> String {
    s.chars().filter(|c| !invisible(*c)).collect()
}

/// What does not draw and only exists to move the layout around: control (C0,
/// C1, DEL), zero-width, direction marks and BOM.
fn invisible(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
        )
}

/// Does the buffer end in the middle of a UTF-8 sequence? A `0x9c` right after
/// would be a continuation byte (`М` = `d0 9c`), not the 8-bit ST.
fn middle_utf8(buf: &[u8]) -> bool {
    let mut i = buf.len();
    let mut cont = 0usize;
    while i > 0 && buf[i - 1] & 0xc0 == 0x80 {
        i -= 1;
        cont += 1;
    }
    if i == 0 {
        return false;
    }
    let needed = match buf[i - 1] {
        0xc2..=0xdf => 1,
        0xe0..=0xef => 2,
        0xf0..=0xf4 => 3,
        _ => 0,
    };
    cont < needed
}

/// Does `0x9c` close the string? Yes when it is the 8-bit ST, and no when it is a
/// UTF-8 continuation byte. `c2 9c` is U+009C itself, which is also ST.
fn st_c1(buf: &[u8]) -> bool {
    buf.last() == Some(&0xc2) || !middle_utf8(buf)
}

/// Parameter `idx` of the body: an empty token is 0; a missing token, `default`.
fn param(body: &[u8], idx: usize, default: u32) -> u32 {
    let mut n = 0usize;
    let mut start = 0usize;
    loop {
        let mut j = start;
        while j < body.len() && body[j] != b';' {
            j += 1;
        }
        if n == idx {
            return parse_u32(&body[start..j]);
        }
        if j >= body.len() {
            return default;
        }
        n += 1;
        start = j + 1;
    }
}

/// Crops/pads the rows to the new size (no reflow at this stage).
fn crop_rows(grid: &mut Vec<Row>, cols: usize, rows: usize) {
    for r in grid.iter_mut() {
        r.cells.resize(cols, Cell::BLANK);
    }
    if grid.len() > rows {
        let excess = grid.len() - rows;
        grid.drain(0..excess);
    } else {
        while grid.len() < rows {
            grid.push(Row::blank(cols));
        }
    }
}

/// What the reflow returned: absolute rows (scrollback + grid) and the cursor.
struct Reflowed {
    /// Where the points the caller asked about landed, in the order asked.
    points: Vec<(usize, usize)>,
    rows: Vec<Row>,
    cur_row: usize,
    cur_col: usize,
    /// The cursor fell at the end of a full row: the next character wraps
    /// (the VT's `wrap_pending`), it does not overwrite the last column.
    wrap: bool,
}

/// Where the cell `off` cells into a paragraph lands: row `first + ci` of the
/// reflowed vector, and the column inside it.
fn chunk_at(chunks: &[(usize, usize)], first: usize, off: usize, cols: usize) -> (usize, usize) {
    let last = chunks.len() - 1;
    let ci = chunks.iter().position(|(_, b)| off < *b).unwrap_or(last);
    let (a, _) = chunks[ci];
    (first + ci, off.saturating_sub(a).min(cols - 1))
}

/// The grid flattened into paragraphs, in reading order: the cells and the tails
/// that live beside each row, kept in the same order so the rewrap can put a
/// cluster back on the cell it belongs to (`B68`).
struct Flat {
    paragraphs: Vec<Vec<Cell>>,
    tails: Vec<Vec<Vec<char>>>,
    /// decoration by paragraph (`Row::chrome`): a paragraph holds several rows
    decoration: Vec<bool>,
    /// paragraph and cell offset where the cursor is
    cur_to: usize,
    cur_off: usize,
    /// `(paragraph, cell offset)` of each point, in the order asked
    pts_at: Vec<(usize, usize)>,
}

fn flatten(all: Vec<Row>, cur_abs: usize, cur_col: usize, pts: &[(usize, usize)]) -> Flat {
    let mut paragraphs: Vec<Vec<Cell>> = Vec::new();
    let mut ptails: Vec<Vec<Vec<char>>> = Vec::new();
    let mut decoration: Vec<bool> = Vec::new();
    let mut cur_to = 0usize;
    let mut cur_off = 0usize;
    let mut pts_at: Vec<(usize, usize)> = vec![(0, 0); pts.len()];
    let mut to: Vec<Cell> = Vec::new();
    let mut to_tails: Vec<Vec<char>> = Vec::new();
    for (i, mut row) in all.into_iter().enumerate() {
        if i == cur_abs {
            cur_to = paragraphs.len();
            cur_off = to.len() + cur_col.min(row.cells.len().saturating_sub(1));
        }
        for (k, (line, col)) in pts.iter().enumerate() {
            if *line == i {
                pts_at[k] = (
                    paragraphs.len(),
                    to.len() + (*col).min(row.cells.len().saturating_sub(1)),
                );
            }
        }
        let soft = row.wrapped;
        let mut tails: Vec<Vec<char>> = (0..row.cells.len())
            .map(|c| row.tail_at(c).to_vec())
            .collect();
        if !soft {
            while row.cells.last().is_some_and(Cell::is_blank) {
                row.cells.pop();
            }
            tails.truncate(row.cells.len());
        }
        to.append(&mut row.cells);
        to_tails.append(&mut tails);
        if !soft {
            decoration.push(row.chrome);
            paragraphs.push(std::mem::take(&mut to));
            ptails.push(std::mem::take(&mut to_tails));
        }
    }
    if !to.is_empty() || paragraphs.is_empty() {
        paragraphs.push(to);
        ptails.push(to_tails);
    }
    Flat {
        paragraphs,
        tails: ptails,
        decoration,
        cur_to,
        cur_off,
        pts_at,
    }
}

/// Joins the "soft" (`wrapped`) rows into paragraphs and rewraps everything at
/// the new width — the `resize` reflow.
///
/// Two rules worth knowing:
///
/// - trailing whitespace only goes away if the background is the default one
///   (A23). A space with a painted background is content: the `vim` status line,
///   `git diff --color`, `bat --style=full` are full-width painted rows, and
///   erasing them eats the drawing;
/// - a wide pair (`CJK`, emoji) is not split in half: if the cut would fall
///   between the cell and its continuation, the row breaks one column earlier.
fn reflow(
    all: Vec<Row>,
    cols: usize,
    cur_abs: usize,
    cur_col: usize,
    pts: &[(usize, usize)],
) -> Reflowed {
    let cols = cols.max(1);
    let Flat {
        paragraphs,
        tails: ptails,
        decoration,
        cur_to,
        cur_off,
        pts_at,
    } = flatten(all, cur_abs, cur_col, pts);

    // -- 2. rewrap
    let mut rows: Vec<Row> = Vec::new();
    let mut points: Vec<(usize, usize)> = vec![(0, 0); pts.len()];
    let mut cur_row = 0usize;
    let mut cur_col_new = 0usize;
    let mut wrap = false;
    for (pi, cells) in paragraphs.iter().enumerate() {
        let first = rows.len();
        let mut chunks: Vec<(usize, usize)> = Vec::new();
        let mut s = 0usize;
        loop {
            let mut e = (s + cols).min(cells.len());
            if e < cells.len() && e > s && cells[e].is_cont() {
                e -= 1; // the whole pair goes down
            }
            chunks.push((s, e));
            if e >= cells.len() {
                break;
            }
            s = e;
        }
        for (ci, (a, b)) in chunks.iter().enumerate() {
            let mut chunk: Vec<Cell> = cells[*a..*b].to_vec();
            chunk.resize(cols, Cell::BLANK);
            let last = ci + 1 == chunks.len();
            let mut row = Row::filled(chunk, !last, decoration[pi]);
            for (local, i) in (*a..*b).enumerate() {
                for ch in ptails[pi].get(i).map_or(&[][..], Vec::as_slice) {
                    row.add_tail(local, *ch);
                }
            }
            rows.push(row);
        }
        for (k, (para, off)) in pts_at.iter().enumerate() {
            if *para == pi {
                points[k] = chunk_at(&chunks, first, *off, cols);
            }
        }
        if pi == cur_to {
            let last_i = chunks.len() - 1;
            let ci = chunks
                .iter()
                .position(|(_, b)| cur_off < *b)
                .unwrap_or(last_i);
            let (a, b) = chunks[ci];
            cur_row = first + ci;
            if cur_off >= cells.len() && b - a == cols && ci == chunks.len() - 1 {
                cur_col_new = cols - 1;
                wrap = true;
            } else {
                cur_col_new = cur_off.saturating_sub(a).min(cols - 1);
            }
        }
    }
    // what is AFTER the cursor is blank area: it is not worth pushing empty
    // rows into the scrollback just because the screen changed size (a resize
    // runs the same loop in `_reflow`). Without this, maximizing from 36 to 48
    // rows pushed the 12 empty ones at the bottom into the scrollback, and they
    // came back as 12 rows above the prompt — the content "started mid-screen".
    while rows.len() > cur_row + 1
        && rows
            .last()
            .is_some_and(|r| r.cells.iter().all(Cell::is_blank))
    {
        rows.pop();
    }
    Reflowed {
        points,
        rows,
        cur_row,
        cur_col: cur_col_new,
        wrap,
    }
}

#[cfg(test)]
mod scroll_shift_tests {
    use super::*;

    /// A scrolled full screen reports the net shift, not global damage.
    #[test]
    fn full_scroll_reports_shift() {
        let mut t = Term::new(10, 4);
        t.clear_damage();
        t.feed(b"\x1b[4;1H\n"); // LF on the last row scrolls the screen
        assert_eq!(t.scroll_shift, 1);
        assert!(!t.dirty_all);
        t.feed(b"\n\n");
        assert_eq!(t.scroll_shift, 3);
        t.clear_damage();
        assert_eq!(t.scroll_shift, 0, "consuming the damage zeroes the shift");
    }

    /// Damage has to scroll along with the content: a write not drawn yet that
    /// scrolls in the same chunk would land on the wrong row (B6).
    #[test]
    fn damage_scrolls_together_with_the_content() {
        let mut t = Term::new(10, 4);
        t.feed(b"\x1b[4;1H");
        t.clear_damage();
        t.feed(b"abc"); // text on the last row, not drawn yet
        assert!(t.dirty[3]);
        t.feed(b"\r\n"); // scrolls one: "abc" went to row 2
        assert_eq!(t.scroll_shift, 1);
        assert!(t.dirty[2], "the row that took the text scrolls dirty along");
        assert!(t.dirty[3], "the new row at the bottom is born dirty too");
        assert!(!t.dirty[0], "the rest stays ready to shift");
        // and downwards, the mirror
        t.clear_damage();
        t.feed(b"\x1b[1;1Hxyz");
        assert!(t.dirty[0]);
        t.feed(b"\x1bM"); // RI at the top scrolls down one
        assert!(t.dirty[1], "downwards: the damage of row 0 goes to row 1");
    }

    /// Scrolling up and then down is not a zero shift: the row that left does
    /// not come back, so the renderer has to rebuild everything.
    #[test]
    fn scroll_both_ways_conflicts() {
        let mut t = Term::new(10, 4);
        t.clear_damage();
        t.feed(b"\x1b[4;1H\n"); // scrolls up
        t.feed(b"\x1b[1;1H\x1bM"); // RI at the top: scrolls down
        assert!(
            t.scroll_conflict,
            "opposite directions must mark a conflict"
        );
    }

    /// Region scrolling does not shift the whole screen: only global damage works.
    #[test]
    fn region_scroll_is_global_damage() {
        let mut t = Term::new(10, 4);
        t.feed(b"\x1b[2;3r");
        t.clear_damage();
        t.feed(b"\x1b[3;1H\n");
        assert!(t.dirty_all);
        assert_eq!(t.scroll_shift, 0);
    }
}

#[cfg(test)]
mod damage_precision_tests {
    use super::*;

    fn marks(t: &Term) -> (u64, &'static str) {
        (t.global_marks, t.global_why)
    }

    /// IL/DL change from the cursor to the end of the region: no full screen.
    #[test]
    fn insert_and_delete_lines_mark_only_the_tail() {
        let mut t = Term::new(10, 6);
        t.clear_damage();
        t.feed(b"\x1b[3;1H\x1b[L"); // IL on row 3
        assert_eq!(marks(&t).0, 0, "IL is not global damage");
        assert!(
            t.dirty[2] && t.dirty[5],
            "from the cursor to the end of the region"
        );
        assert!(
            !t.dirty[0] && !t.dirty[1],
            "above the cursor nothing changed"
        );
        t.clear_damage();
        t.feed(b"\x1b[3;1H\x1b[M"); // DL on row 3
        assert_eq!(marks(&t).0, 0, "DL is not global damage");
        assert!(!t.dirty[0] && !t.dirty[1]);
    }

    /// ED 0 and ED 1 reach one side of the cursor; ED 2 is the whole screen.
    #[test]
    fn erase_display_marks_the_right_side() {
        let mut t = Term::new(10, 6);
        t.feed(b"\x1b[3;1H");
        t.clear_damage();
        t.feed(b"\x1b[0J");
        assert_eq!(marks(&t).0, 0, "ED0 is not global damage");
        assert!(!t.dirty[0] && t.dirty[2] && t.dirty[5]);
        t.clear_damage();
        t.feed(b"\x1b[3;1H\x1b[1J");
        assert_eq!(marks(&t).0, 0, "ED1 is not global damage");
        assert!(t.dirty[0] && t.dirty[2] && !t.dirty[5]);
        t.clear_damage();
        t.feed(b"\x1b[2J");
        assert_eq!(marks(&t).1, "touch_all", "ED2 is the whole screen");
    }
}

#[cfg(test)]
mod compose_tests {
    use super::*;

    /// `M-U`: a combining mark composes into the cell before it instead of
    /// taking one of its own.
    #[test]
    fn a_combining_mark_lands_on_the_character_before_it() {
        let mut t = Term::new(10, 2);
        t.feed("e\u{301}".as_bytes());
        assert_eq!(t.grid[0].cells[0].ch, 'e');
        assert_eq!(t.grid[0].marks_at(0), ['\u{301}']);
        assert_eq!(t.cx, 1, "the mark takes no cell of its own");
        assert_eq!(t.grid[0].cells[1].ch, ' ');
        // more marks on the same cell keep arriving, up to the cluster cap
        t.feed("\u{302}\u{303}\u{304}".as_bytes());
        assert_eq!(t.grid[0].marks_at(0).len(), 4);
        assert!(t.grid[0].marks_at(0).len() <= crate::grid::MAX_TAIL);
        // writing a new character over the cell replaces its marks
        t.feed(b"\x08x");
        assert_eq!(t.grid[0].cells[0].ch, 'x');
        assert_eq!(t.grid[0].marks_at(0).len(), 0);
    }
}

#[cfg(test)]
mod cluster_tests {
    use super::*;

    /// `B66`: a ZWJ family is one two-column cell, not six columns of faces.
    #[test]
    fn a_zwj_family_takes_two_columns() {
        let mut t = Term::new(20, 2);
        t.feed("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}".as_bytes());
        assert_eq!(t.cx, 2, "one wide cell");
        assert_eq!(t.grid[0].cells[0].ch, '\u{1F468}');
        assert_eq!(
            t.grid[0].tail_at(0),
            ['\u{200D}', '\u{1F469}', '\u{200D}', '\u{1F467}'],
            "the joiners and the members belong to the cell"
        );
        assert!(t.grid[0].cells[1].is_cont());
        // and the whole sequence is what a copy takes out of the row
        assert_eq!(t.row_text(0), "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
    }

    /// A family of four is seven codepoints: the tail has room for it, which
    /// three marks would not have.
    #[test]
    fn a_big_family_fits_one_cell() {
        let mut t = Term::new(20, 2);
        t.feed("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}".as_bytes());
        assert_eq!(t.cx, 2);
        assert_eq!(t.grid[0].tail_at(0).len(), 6);
    }

    /// Two regional indicators are one flag of two columns, not two letters.
    #[test]
    fn a_flag_pair_is_one_cell() {
        let mut t = Term::new(20, 2);
        t.feed("\u{1F1E7}\u{1F1F7}".as_bytes());
        assert_eq!(t.cx, 2);
        assert_eq!(t.grid[0].cells[0].ch, '\u{1F1E7}');
        assert_eq!(t.grid[0].tail_at(0), ['\u{1F1F7}']);
        assert!(t.grid[0].cells[1].is_cont());
        // a third indicator starts a new flag
        t.feed("x\u{1F1FA}".as_bytes());
        assert_eq!(t.grid[0].cells[2].ch, 'x');
    }

    /// The skin tone tints the base beside it; it is not a second picture.
    #[test]
    fn a_skin_tone_tints_the_base() {
        let mut t = Term::new(20, 2);
        t.feed("\u{1F44D}\u{1F3FD}".as_bytes());
        assert_eq!(t.cx, 2);
        assert_eq!(t.grid[0].tail_at(0), ['\u{1F3FD}']);
        assert!(!t.grid[0].cells[2].is_cont());
    }

    /// `VS16` asks for the emoji picture and `VS15` for the text glyph, and the
    /// width follows: the same heart is two columns in one and one in the other.
    #[test]
    fn vs16_is_wide_and_vs15_is_not() {
        let mut emoji = Term::new(20, 2);
        emoji.feed("\u{2764}\u{FE0F}".as_bytes());
        assert_eq!(emoji.cx, 2, "emoji presentation is two columns");
        assert!(emoji.grid[0].cells[1].is_cont());
        let mut text = Term::new(20, 2);
        text.feed("\u{2764}\u{FE0E}".as_bytes());
        assert_eq!(text.cx, 1, "text presentation is one column");
        assert!(!text.grid[0].cells[1].is_cont());
    }

    /// A keycap is one character with a box around it, two columns wide.
    #[test]
    fn a_keycap_is_one_cell() {
        let mut t = Term::new(20, 2);
        t.feed("1\u{FE0F}\u{20E3}".as_bytes());
        assert_eq!(t.cx, 2);
        assert_eq!(t.grid[0].cells[0].ch, '1');
        assert_eq!(t.grid[0].tail_at(0), ['\u{FE0F}', '\u{20E3}']);
    }

    /// A joiner that starts a line has nothing to join and is not a cell of its
    /// own: it is dropped, and the character after it lands normally.
    #[test]
    fn a_joiner_with_no_base_is_dropped() {
        let mut t = Term::new(20, 2);
        t.feed("\u{200D}x".as_bytes());
        assert_eq!(t.grid[0].cells[0].ch, 'x');
        assert_eq!(t.cx, 1);
    }

    /// A joiner at the end of a line joins nothing: the character on the next
    /// line is a character, not a family member of the cell above.
    #[test]
    fn a_joiner_does_not_reach_the_next_row() {
        let mut t = Term::new(4, 2);
        t.feed("ab\u{200D}\r\ncd".as_bytes());
        assert_eq!(t.grid[0].tail_at(1), ['\u{200D}']);
        assert_eq!(t.grid[1].cells[0].ch, 'c', "the next row starts clean");
        assert_eq!(t.cx, 2);
    }

    /// Text with a joiner but no emoji in it stays one column: `a\u{200D}b` is a
    /// text ligature request, not a picture.
    #[test]
    fn a_joiner_without_emoji_stays_narrow() {
        let mut t = Term::new(20, 2);
        t.feed("a\u{200D}b".as_bytes());
        assert_eq!(t.cx, 1);
        assert_eq!(t.grid[0].tail_at(0), ['\u{200D}', 'b']);
    }
}

#[cfg(test)]
mod dcs_query_tests {
    use super::*;

    #[test]
    fn decrqss_answers_the_pen_it_has() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b[1;31m");
        t.feed(b"\x1bP$qm\x1b\\");
        assert_eq!(t.take_child_out(), b"\x1bP1$r0;1;31m\x1b\\".to_vec());
    }

    #[test]
    fn decrqss_without_attributes_is_zero() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1bP$qm\x1b\\");
        assert_eq!(t.take_child_out(), b"\x1bP1$r0m\x1b\\".to_vec());
    }

    #[test]
    fn decrqss_answers_region_and_cursor_shape() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b[2;3r");
        t.feed(b"\x1bP$qr\x1b\\");
        assert_eq!(t.take_child_out(), b"\x1bP1$r2;3r\x1b\\".to_vec());
        // no DECSCUSR seen: the shipped default (steady block, `2`)
        t.feed(b"\x1bP$q q\x1b\\");
        assert_eq!(t.take_child_out(), b"\x1bP1$r2 q\x1b\\".to_vec());
        // a steady underline the program asked for
        t.feed(b"\x1b[4 q");
        t.feed(b"\x1bP$q q\x1b\\");
        assert_eq!(t.take_child_out(), b"\x1bP1$r4 q\x1b\\".to_vec());
    }

    #[test]
    fn decrqss_refuses_what_it_does_not_keep() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1bP$q\"p\x1b\\");
        assert_eq!(t.take_child_out(), b"\x1bP0$r\x1b\\".to_vec());
    }

    #[test]
    fn xtgettcap_answers_the_table_and_refuses_the_rest() {
        let mut t = Term::new(20, 4);
        // `Co` is asked, `ZZ` is not: the answer carries only the known one,
        // with a literal `=` (the reply is hex words joined by raw separators,
        // which is what the requesters split on)
        t.feed(b"\x1bP+q436F3B5A5A\x1b\\");
        assert_eq!(t.take_child_out(), b"\x1bP1+r436F=323536\x1b\\".to_vec());
        // nothing known: the standard negative
        t.feed(b"\x1bP+q5A5A\x1b\\");
        assert_eq!(t.take_child_out(), b"\x1bP0+r\x1b\\".to_vec());
        // not hex at all: the negative, not a guess
        t.feed(b"\x1bP+qZZ\x1b\\");
        assert_eq!(t.take_child_out(), b"\x1bP0+r\x1b\\".to_vec());
    }
}

#[cfg(test)]
mod done_tests {
    use super::*;

    /// A command from `B` to `D` with the clock in the test's hand.
    fn run(secs: u64, status: i32) -> Term {
        let mut t = Term::new(20, 4);
        t.now_ms = 0;
        t.feed(b"\x1b]133;B\x07sleep\r\n");
        t.now_ms = secs * 1000;
        t.feed(format!("\x1b]133;D;{status}\x07").as_bytes());
        t
    }

    #[test]
    fn a_fast_success_is_silence() {
        let t = run(2, 0);
        assert_eq!(t.notifs, 0);
        assert!(t.notifications.is_empty());
    }

    #[test]
    fn a_long_success_notifies() {
        let t = run(32, 0);
        assert_eq!(t.notifs, 1);
        let n = t.notifications.entries().next().unwrap();
        assert_eq!(n.source, crate::notif::Source::Osc133);
        assert_eq!(n.text(), "exit 0 · 32.0 s");
        assert_eq!(n.shade, crate::notif::Shade::Info);
    }

    #[test]
    fn a_fast_failure_is_silence() {
        // a fast command means the owner was watching: the screen already said
        // it, so the `D` says nothing, whatever the status was
        let t = run(1, 2);
        assert_eq!(t.notifs, 0);
        assert!(t.notifications.is_empty());
    }

    #[test]
    fn a_long_failure_notifies_as_an_error() {
        let t = run(32, 2);
        assert_eq!(t.notifs, 1);
        let n = t.notifications.entries().next().unwrap();
        assert_eq!(n.text(), "exit 2 · 32.0 s");
        assert_eq!(n.shade, crate::notif::Shade::Err);
    }

    #[test]
    fn a_d_without_a_start_has_no_wait_to_report() {
        let mut t = Term::new(20, 4);
        t.now_ms = 5_000;
        t.feed(b"\x1b]133;D;1\x07");
        // no start, no wait: below any floor, so silence even on failure
        assert_eq!(t.notifs, 0);
    }

    #[test]
    fn zero_means_the_d_never_notifies() {
        let mut t = Term::new(20, 4);
        t.shell_notify_secs = 0;
        t.now_ms = 0;
        t.feed(b"\x1b]133;B\x07sleep 600\r\n");
        t.now_ms = 600_000;
        t.feed(b"\x1b]133;D;0\x07");
        assert_eq!(t.notifs, 0);
    }

    #[test]
    fn none_and_off_mean_zero() {
        let (cfg, errs) = crate::config::Config::parse("shell_notify_secs none\n");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(cfg.shell_notify_secs, 0);
        let (cfg, errs) = crate::config::Config::parse("shell_notify_secs off\n");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(cfg.shell_notify_secs, 0);
        let (cfg, errs) = crate::config::Config::parse("shell_notify_secs 30\n");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(cfg.shell_notify_secs, 30);
        let (_, errs) = crate::config::Config::parse("shell_notify_secs soon\n");
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn minutes_get_their_own_shape() {
        assert_eq!(Term::dur_text(9_500), "9.5 s");
        assert_eq!(Term::dur_text(187_000), "3m 07s");
    }
}

#[cfg(test)]
mod marks_tests {
    use super::*;
    use crate::prompt::MarkKind;

    #[test]
    fn the_shell_marks_a_prompt_and_its_command() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b]133;A\x07$ ");
        t.feed(b"\x1b]133;B\x07ls\r\n");
        t.feed(b"\x1b]133;C\x07out\r\n");
        t.feed(b"\x1b]133;D;3\x07");
        let kinds: Vec<_> = t.marks.all().iter().map(|m| m.kind).collect();
        assert_eq!(
            kinds,
            [
                MarkKind::PromptStart,
                MarkKind::CommandStart,
                MarkKind::OutputStart,
                MarkKind::CommandEnd
            ]
        );
        // the status of the command that ended
        assert_eq!(t.marks.all()[3].status, Some(3));
        // the marks are on the lines the cursor was on: A/B on line 0, C after
        // the newline, D on the line of the last output
        assert_eq!(t.marks.all()[0].line, 0);
        assert_eq!(t.marks.all()[2].line, 1);
    }

    #[test]
    fn an_unknown_133_letter_is_ignored() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b]133;Z;1;2\x07hi");
        assert!(t.marks.is_empty());
        // and the text after it is untouched
        assert_eq!(t.row_text(0).trim_end(), "hi");
    }

    #[test]
    fn the_navigation_walks_the_prompts_it_has() {
        let mut t = Term::new(20, 4);
        for _ in 0..3 {
            t.feed(b"\x1b]133;A\x07$ ");
            t.feed(b"\x1b]133;B\x07echo hi\r\n");
            t.feed(b"\x1b]133;C\x07hi\r\n");
            t.feed(b"\x1b]133;D;0\x07");
        }
        let lines: Vec<i64> = t
            .marks
            .all()
            .iter()
            .filter(|m| m.kind == MarkKind::PromptStart)
            .map(|m| m.line)
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(t.prompt_line(lines[2], true), Some(lines[1]));
        assert_eq!(t.prompt_line(lines[0], true), None);
        assert_eq!(t.prompt_line(lines[0], false), Some(lines[1]));
    }

    #[test]
    fn a_reset_forgets_the_marks() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b]133;A\x07$ ");
        assert_eq!(t.marks.len(), 1);
        t.feed(b"\x1bc"); // RIS
        assert!(t.marks.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Anchor;

    fn term() -> Term {
        Term::new(10, 4)
    }

    /// Solid-color 2x2 RGBA.
    fn rgba2x2() -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode([9, 8, 7, 255].repeat(4))
    }

    fn apc(body: &str) -> Vec<u8> {
        format!("\x1b_{body}\x1b\\").into_bytes()
    }

    #[test]
    fn kitty_apc_becomes_image_reply_and_moves_cursor_down() {
        let mut t = Term::new(20, 4);
        t.feed(&apc(&format!("Ga=T,f=32,s=2,v=2,i=3;{}", rgba2x2())));
        assert_eq!(t.graphics.images().len(), 1);
        assert_eq!(t.graphics.placements().len(), 1);
        // the reply goes to the child in the same format it sent
        assert_eq!(t.take_child_out(), b"\x1b_Gi=3;OK\x1b\\".to_vec());
        // H10: without `C=1` the cursor goes down the rows the image took
        assert_eq!(t.cy, 1);
        assert_eq!(t.cx, 0);
        // none of this may dirty the grid
        assert!(screen(&t)[0].trim().is_empty(), "{:?}", screen(&t));
    }

    #[test]
    fn file_apc_waits_for_serve_and_then_replies_like_direct() {
        use base64::Engine as _;
        let path = base64::engine::general_purpose::STANDARD.encode("/tmp/x.rgba");
        let mut t = Term::new(20, 4);
        t.feed(&apc(&format!("Ga=T,f=32,s=2,v=2,t=f,i=3;{path}")));
        assert_eq!(t.image_files.len(), 1);
        assert!(t.graphics.images().is_empty(), "no bytes yet, no image");
        assert!(t.take_child_out().is_empty(), "no verdict before the load");
        // the app read the file: the image lands and the child hears OK
        t.serve_images(|p, d| {
            assert_eq!((p, d), ("/tmp/x.rgba", false));
            Some([9, 8, 7, 255].repeat(4))
        });
        assert_eq!(t.graphics.images().len(), 1);
        assert_eq!(t.graphics.placements().len(), 1);
        assert_eq!(t.take_child_out(), b"\x1b_Gi=3;OK\x1b\\".to_vec());
        assert!(t.image_files.is_empty());
    }

    #[test]
    fn osc9_progress_state_and_fraction_reach_the_tab() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b]9;4;1;37\x07");
        assert_eq!(
            t.progress,
            Some(Progress {
                state: 1,
                pct: Some(37)
            })
        );
        // a state with no measure arrives without the fraction
        t.feed(b"\x1b]9;4;3\x07");
        assert_eq!(
            t.progress,
            Some(Progress {
                state: 3,
                pct: None
            })
        );
        // 0 removes it, and asks for nothing
        t.feed(b"\x1b]9;4;0\x07");
        assert_eq!(t.progress, None);
        assert_eq!(t.notifs, 0);
    }

    /// The trace is the debugging voice: a refusal says WHY it was refused.
    /// Before this, the lock that killed the keepalive (the owner's bug) was
    /// invisible — not a fact, not in a list, not on the bar.
    #[test]
    fn the_trace_says_why_the_form_was_refused() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b]9;4;1;30\x07");
        t.feed(b"\x1b]9;4;1;30\x07"); // the same fact twice
        t.feed(b"\x1b]7501;state=blocked:kind=auth:app=pi\x1b\\");
        t.feed(b"\x1b]9;4;1;40\x07"); // locked out
        t.feed(b"\x1b]9;4;7\x07"); // a digit with no meaning
        let trace = t.take_trace();
        assert_eq!(trace.len(), 5);
        assert!(matches!(
            trace[0],
            crate::trace::Event::Progress {
                outcome: crate::trace::Outcome::Applied,
                ..
            }
        ));
        assert!(matches!(
            trace[1],
            crate::trace::Event::Progress {
                outcome: crate::trace::Outcome::Repeat,
                ..
            }
        ));
        assert!(matches!(
            trace[2],
            crate::trace::Event::Notify {
                source: crate::notif::Source::Osc7501,
                ..
            }
        ));
        assert!(matches!(
            trace[3],
            crate::trace::Event::Progress {
                outcome: crate::trace::Outcome::Locked,
                ..
            }
        ));
        assert!(matches!(
            trace[4],
            crate::trace::Event::Progress {
                outcome: crate::trace::Outcome::Unknown,
                ..
            }
        ));
        // the drain takes everything: no event is left behind
        assert_eq!(t.take_trace(), [], "no event is left behind");
    }

    /// The emulation cannot stop for lack of log consumption: a ring left full
    /// forgets the oldest event and the parser goes on, byte after byte.
    #[test]
    fn an_undrained_trace_never_stops_the_emulation() {
        let mut t = Term::new(20, 4);
        for i in 0..(crate::trace::TRACE_MAX as u32 + 20) {
            t.feed(format!("\x1b]9;4;1;{}\x07", i % 101).as_bytes());
        }
        // the ring held its cap and the emulation is alive: the last state is
        // on the grid's record, not lost to a stuck log
        assert_eq!(t.take_trace().len(), crate::trace::TRACE_MAX);
        // 84 feeds applied (84 distinct percentages, the repeats are Repeat);
        // the last one was 83
        assert_eq!(
            t.progress.map(|p| p.pct),
            Some(Some(83)),
            "the parser kept working"
        );
    }

    /// The keepalive came back after the `7501` reports settled: the lock is the
    /// rich report itself, not the fact that one was ever heard (the bug the
    /// owner hit — one `pi` startup report and the `9;4;3` was dead for the
    /// tab's whole life).
    #[test]
    fn the_keepalive_returns_after_a_report_settles() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b]9;4;3\x07");
        assert_eq!(t.progress.map(|p| (p.state, p.pct)), Some((3, None)));
        t.feed(b"\x1b]7501;state=working:app=pi\x1b\\");
        t.feed(b"\x1b]7501;state=done:app=pi\x1b\\");
        // the reports are over and neither carried a `kind`: the root is free
        t.feed(b"\x1b]9;4;3\x07");
        assert_eq!(
            t.progress.map(|p| (p.state, p.pct)),
            Some((3, None)),
            "the indeterminate band is back"
        );
    }

    #[test]
    fn osc9_progress_is_clamped_and_an_unknown_state_is_still_not_a_message() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b]9;4;1;250\x07");
        assert_eq!(
            t.progress,
            Some(Progress {
                state: 1,
                pct: Some(100)
            })
        );
        // a digit state we give no meaning to is the same form: swallowed,
        // never a toast (a 1 Hz repeater would pin one forever)
        t.feed(b"\x1b]9;4;9;50\x07");
        assert_eq!(
            t.progress,
            Some(Progress {
                state: 1,
                pct: Some(100)
            })
        );
        assert_eq!(t.notifs, 0);
        // `RIS` clears it: the state belongs to the emulation
        t.feed(b"\x1bc");
        assert_eq!(t.progress, None);
    }

    #[test]
    fn ris_keeps_the_unseen_counters_and_their_list() {
        // the app follows `bells`/`notifs` with `while seen != count`: a rewind
        // is ~2^32 laps of the pump, so the counts — and the unseen news they
        // stand for — survive the reset, only the emulation state goes
        let mut t = Term::new(20, 4);
        t.feed(b"\x07");
        t.feed(b"\x1b]9;done\x07");
        assert_eq!((t.bells, t.notifs), (1, 1));
        t.feed(b"\x1bc"); // RIS
        assert_eq!((t.bells, t.notifs), (1, 1), "counts only move forward");
        assert_eq!(t.last_notification(), "done");
        assert_eq!(t.grid[0].cells[0].ch, ' ', "but the grid is new");
    }

    #[test]
    fn osc9_progress_state_is_not_a_notification() {
        // the mintty/ConEmu progress form (a TUI keepalive): swallowed
        let mut t = Term::new(20, 4);
        for body in ["\x1b]9;4;3\x07", "\x1b]9;4;0\x07", "\x1b]9;4;3;50\x1b\\"] {
            t.feed(body.as_bytes());
        }
        assert_eq!(t.notifs, 0, "progress never notifies");
        assert!(t.notifications.is_empty());
        // but a message that merely starts with a digit still does
        t.feed(b"\x1b]9;4;3 done\x07");
        assert_eq!(t.notifs, 1);
        assert_eq!(t.last_notification(), "4;3 done");
    }

    #[test]
    fn unreadable_image_file_answers_enoent() {
        use base64::Engine as _;
        let path = base64::engine::general_purpose::STANDARD.encode("/no/such.png");
        let mut t = Term::new(20, 4);
        t.feed(&apc(&format!("Ga=T,f=100,t=f,i=5;{path}")));
        t.serve_images(|_, _| None);
        assert!(t.graphics.images().is_empty());
        assert_eq!(t.take_child_out(), b"\x1b_Gi=5;ENOENT\x1b\\".to_vec());
    }

    #[test]
    fn c_1_keeps_the_cursor_where_it_was() {
        let mut t = Term::new(20, 4);
        t.feed(&apc(&format!("Ga=T,f=32,s=2,v=2,i=1,C=1;{}", rgba2x2())));
        assert_eq!(t.cy, 0);
        assert_eq!(t.graphics.placements().len(), 1);
    }

    /// `B3`: a sixel arrives as a DCS and becomes a picture at the cursor cell;
    /// the cursor ends below it, because a producer like `chafa` sends no newline
    /// of its own and the prompt would land on top of the picture.
    #[test]
    fn a_sixel_dcs_becomes_an_image_and_the_cursor_goes_below_it() {
        let mut t = Term::new(20, 6);
        t.cell_size = (8, 16);
        t.feed(b"\x1b[2;3H");
        // four columns of six rows, red, opaque
        t.feed(b"\x1bP0;0;0q#1;2;100;0;0!4~\x1b\\");
        let imgs: Vec<_> = t.graphics.images().values().collect();
        assert_eq!(imgs.len(), 1);
        assert_eq!((imgs[0].width, imgs[0].height), (4, 6));
        assert_eq!(imgs[0].format, crate::graphics::Format::Sixel);
        let p = &t.graphics.placements()[0];
        assert_eq!((p.col, p.line), (2, 1), "at the cursor cell, one-based");
        assert_eq!((p.cols, p.rows), (1, 1), "4x6 px against an 8x16 cell");
        assert_eq!((t.cx, t.cy), (0, 2), "the cursor ends under the picture");
        assert_eq!(t.take_child_out(), Vec::<u8>::new(), "and answers nothing");
        // `P2=1` is the transparent background
        t.feed(b"\x1bP0;1;0q#1;2;100;0;0@\x1b\\");
        let transparent = t
            .graphics
            .images()
            .values()
            .filter(|i| i.format == crate::graphics::Format::SixelAlpha)
            .count();
        assert_eq!(transparent, 1);
        // and a DCS that is not a sixel still stores nothing
        t.feed(b"\x1bP$q\"p\x1b\\");
        assert_eq!(t.graphics.images().len(), 2);
    }

    /// `B3`: a picture taller than what is left of the screen scrolls the content
    /// up, and its placement rides with it — that is the room the picture took.
    #[test]
    fn a_tall_sixel_scrolls_the_content_up_to_fit() {
        let mut t = Term::new(20, 6);
        t.cell_size = (8, 16);
        t.feed(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
        // four bands of six pixels: 24 px against a 16 px cell is two rows
        t.feed(b"\x1bP0;0;0q#1;2;100;0;0!1~-!1~-!1~-!1~\x1b\\");
        let p = &t.graphics.placements()[0];
        assert_eq!(p.rows, 2, "two cell rows of six-pixel bands");
        assert_eq!(p.line, 3, "the picture rode up with the two scrolled lines");
        assert_eq!((t.cx, t.cy), (0, 5), "and the cursor sits on the last row");
    }

    /// A picture is drawn where cells are: erasing those cells takes it away.
    /// `clear` and a full-screen app that paints over (no alternate screen, like
    /// `mc`) rely on this — without it the picture stays visible in every cell
    /// they leave blank.
    #[test]
    fn erasing_the_cells_takes_the_picture_with_them() {
        let mut t = Term::new(20, 6);
        t.cell_size = (8, 16);
        // one band: six pixels, one 16 px cell row
        let pic = b"\x1bP0;0;0q#1;2;100;0;0!1~\x1b\\";
        t.feed(b"\x1b[2;9H");
        t.feed(pic);
        assert_eq!(t.graphics.placements().len(), 1, "placed at row 1, col 8");
        // EL from the first cell to the end of the row: the picture is in the
        // erased span, so it goes
        t.feed(b"\x1b[2;1H\x1b[K");
        assert!(t.graphics.placements().is_empty(), "EL took it");
        // a picture further right survives an erase to its left
        t.feed(b"\x1b[3;13H");
        t.feed(pic);
        t.feed(b"\x1b[3;1H\x1b[4X");
        assert_eq!(
            t.graphics.placements().len(),
            1,
            "ECH of the first four cells leaves a picture starting at col 12"
        );
        // and the whole screen clearing removes what is on it
        t.feed(b"\x1b[2J");
        assert!(t.graphics.placements().is_empty(), "ED 2 took it");
    }

    /// The alternate screen is its own place: an app that clears it (`vim`,
    /// `less`, `top`) must not erase the pictures of the screen it covers, and
    /// the pictures it draws itself must not follow it out.
    #[test]
    fn the_alternate_screen_has_its_own_pictures() {
        let mut t = Term::new(20, 6);
        t.cell_size = (8, 16);
        let pic = b"\x1bP0;0;0q#1;2;100;0;0!1~\x1b\\";
        t.feed(b"\x1b[1;1H");
        t.feed(pic);
        assert_eq!(t.graphics.placements().len(), 1);
        t.feed(b"\x1b[?1049h\x1b[H\x1b[2J");
        assert!(
            t.graphics.placements().is_empty(),
            "the alt screen starts empty even after its clear"
        );
        t.feed(pic);
        assert_eq!(t.graphics.placements().len(), 1, "the alt one has its own");
        t.feed(b"\x1b[?1049l");
        let left = t.graphics.placements();
        assert_eq!(left.len(), 1, "leaving brings the main one back");
        assert_eq!(left[0].line, 0, "in the line it was drawn on");
    }

    /// The screen is lent, not traded: an app taking the alternate screen keeps
    /// the history that had scrolled out, and what it writes up there stays up
    /// there. `mc`, `less` and `vim` all enter this way (`B63`).
    #[test]
    fn the_alternate_screen_keeps_the_history() {
        let mut t = Term::new(10, 3);
        for i in 0..6 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        let before = t.history_text(0, 64);
        assert_eq!(before.len(), 4, "four of the six lines scrolled out");
        t.feed(b"\x1b[?1049h");
        assert_eq!(t.history_text(0, 64), before, "the switch takes nothing");
        for i in 0..5 {
            t.feed(format!("alt {i}\r\n").as_bytes());
        }
        t.feed(b"\x1b[?1049l");
        assert_eq!(
            t.history_text(0, 64),
            before,
            "coming back brings the same history, with no alt line in it"
        );
        // and a `clear` up there is the alternate screen's business alone
        t.feed(b"\x1bP0;0;0q#1;2;100;0;0!1~\x1b\\");
        let picture = t.graphics.placements().len();
        let filled = t.history_text(0, 64);
        assert_eq!(picture, 1, "a picture on the main screen");
        t.feed(b"\x1b[?1049h\x1b[3J");
        assert_eq!(
            t.history_text(0, 64),
            filled,
            "`ED 3` in the alt screen neither"
        );
        t.feed(b"\x1b[?1049l");
        assert_eq!(t.history_text(0, 64), filled);
        assert_eq!(
            t.graphics.placements().len(),
            picture,
            "nor does it take the picture it was covering"
        );
    }

    /// The screen clear reaches only the screen: a picture that scrolled into the
    /// history stays there, and comes back when the reader scrolls up to it.
    #[test]
    fn a_clear_leaves_the_pictures_in_the_history() {
        let mut t = Term::new(20, 4);
        t.cell_size = (8, 16);
        t.feed(b"\x1b[1;1H");
        t.feed(b"\x1bP0;0;0q#1;2;100;0;0!1~\x1b\\");
        // three newlines push it two lines up, into the history of a 4-row grid
        t.feed(b"\r\n\r\n\r\n");
        let line = t.graphics.placements()[0].line;
        assert!(line < 0, "the picture went into the history (line {line})");
        t.feed(b"\x1b[H\x1b[2J");
        assert_eq!(
            t.graphics.placements().len(),
            1,
            "ED 2 clears the screen, not the history"
        );
    }

    #[test]
    fn dcs_and_a_non_kitty_apc_store_nothing() {
        let mut t = Term::new(20, 4);
        // DCS (`ESC P`): discarded until ST, and it must not become an image
        t.feed(b"\x1bP1;2;3+abc\x1b\\");
        // APC that does not start with `G`: not from the protocol
        t.feed(&apc("x=1,f=32"));
        assert_eq!(t.graphics.images().len(), 0);
        assert_eq!(t.take_child_out(), Vec::<u8>::new());
        // and the parser stays alive after that
        t.feed(b"ok");
        assert!(screen(&t)[0].starts_with("ok"), "{:?}", screen(&t));
    }

    #[test]
    fn csi_14_t_reports_the_area_in_pixels() {
        // `kitten icat` asks before sending the image; without an answer it
        // gives up with "terminal does not support reporting screen sizes"
        let mut t = Term::new(20, 4);
        t.cell_size = (10, 19);
        t.feed(b"\x1b[14t");
        assert_eq!(t.take_child_out(), b"\x1b[4;76;200t".to_vec());
    }

    #[test]
    fn ed_3_clears_the_images() {
        let mut t = Term::new(20, 4);
        t.feed(&apc(&format!("Ga=T,f=32,s=2,v=2,i=1;{}", rgba2x2())));
        assert_eq!(t.graphics.images().len(), 1);
        t.feed(b"\x1b[3J");
        assert_eq!(t.graphics.images().len(), 0);
        assert_eq!(t.graphics.placements(), []);
    }

    #[test]
    fn image_follows_the_scrollback_and_reflow_prunes_the_unreachable() {
        // An image follows the history
        let mut t = Term::new(20, 4);
        t.feed(&apc(&format!("Ga=T,f=32,s=2,v=2,i=7;{}", rgba2x2())));
        for _ in 0..6 {
            t.feed(b"\r\n");
        }
        let p = t.graphics.placements()[0];
        assert!(p.line < 0, "left through the top: {}", p.line);
        assert!(p.line + i32::try_from(t.scrollback.len()).unwrap() >= 0);
        t.resize(40, 4);
        assert!(
            t.graphics
                .placements()
                .iter()
                .all(|p| p.line >= -i32::try_from(t.scrollback.len()).unwrap()),
            "{:?}",
            t.graphics.placements()
        );
    }

    fn screen(t: &Term) -> Vec<String> {
        (0..t.rows).map(|y| t.row_text(y)).collect()
    }

    #[test]
    fn kitty_keyboard_protocol_pushes_pops_and_queries() {
        let mut t = term();
        // query: the app asks and the terminal replies with the state (0)
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?0u".to_vec());
        // push saves the current one and sets the new one
        t.feed(b"\x1b[>5u");
        assert_eq!(t.kbd_flags, 5);
        t.feed(b"\x1b[?u");
        assert_eq!(t.take_child_out(), b"\x1b[?5u".to_vec());
        // pop brings back what was there before
        t.feed(b"\x1b[<u");
        assert_eq!(t.kbd_flags, 0);
        // popping an empty stack yields 0 (it does not blow up)
        t.feed(b"\x1b[<1u");
        assert_eq!(t.kbd_flags, 0);
    }

    #[test]
    fn kitty_keyboard_protocol_set_modes() {
        let mut t = term();
        t.feed(b"\x1b[=3u"); // default mode = 1 (replace)
        assert_eq!(t.kbd_flags, 3);
        t.feed(b"\x1b[=4;2u"); // adds
        assert_eq!(t.kbd_flags, 7);
        t.feed(b"\x1b[=4;3u"); // removes
        assert_eq!(t.kbd_flags, 3);
        t.feed(b"\x1b[=0;1u"); // replaces with nothing
        assert_eq!(t.kbd_flags, 0);
    }

    #[test]
    fn kitty_keyboard_protocol_stack_with_two_levels() {
        let mut t = term();
        t.feed(b"\x1b[>1u\x1b[>11u");
        assert_eq!(t.kbd_flags, 11);
        t.feed(b"\x1b[<1u");
        assert_eq!(t.kbd_flags, 1);
        t.feed(b"\x1b[<u");
        assert_eq!(t.kbd_flags, 0);
    }

    #[test]
    fn scorc_without_prefix_still_restores_the_cursor() {
        let mut t = term();
        t.feed(b"\x1b[2;3H\x1b[s\x1b[H\x1b[u");
        assert_eq!((t.cy, t.cx), (1, 2));
        assert!(
            t.take_child_out().is_empty(),
            "SCORC does not talk to the child"
        );
    }

    #[test]
    fn simple_text_and_breaks() {
        let mut t = term();
        t.feed(b"abc");
        assert_eq!(t.row_text(0), "abc");
        assert_eq!((t.cy, t.cx), (0, 3));
        // '\n' goes down without returning the column
        t.feed(b"\nxy");
        assert_eq!(t.row_text(1), "   xy");
    }

    #[test]
    fn autowrap_marks_a_soft_break() {
        let mut t = term();
        t.feed(b"0123456789A");
        assert_eq!(t.row_text(0), "0123456789");
        assert_eq!(t.row_text(1), "A");
        assert!(t.grid[0].wrapped);
        assert!(!t.grid[1].wrapped);
    }

    #[test]
    fn crlf_after_a_full_row_does_not_skip_two() {
        // the P15 bug: filling the row and then \r\n advanced two
        let mut t = term();
        t.feed(b"0123456789\r\nx");
        assert_eq!(t.row_text(1), "x");
        assert_eq!(t.row_text(2), "");
    }

    #[test]
    fn sgr_truecolor_per_cell() {
        let mut t = term();
        t.feed(b"\x1b[38;2;1;2;3m\x1b[48;2;4;5;6mA");
        let c = t.grid[0].cells[0];
        assert_eq!(c.ch, 'A');
        assert_eq!(c.fg, Color::Rgb(1, 2, 3));
        assert_eq!(c.bg, Color::Rgb(4, 5, 6));
    }

    #[test]
    fn sgr_256_and_basic() {
        let mut t = term();
        t.feed(b"\x1b[38;5;196m\x1b[1;4mA");
        let c = t.grid[0].cells[0];
        assert_eq!(c.fg, Color::Idx(196));
        assert_eq!(c.flags & BOLD, BOLD);
        assert_eq!(c.ul, UL_SINGLE);
        t.feed(b"\x1b[0m");
        assert_eq!(t.fg, Color::Default);
        assert_eq!(t.flags, 0);
    }

    #[test]
    fn sgr_text_attributes() {
        let mut t = term();
        t.feed(b"\x1b[3;2;5;9;53mA");
        let c = t.grid[0].cells[0];
        for (bit, name) in [
            (ITALIC, "italic"),
            (FAINT, "faint"),
            (BLINK, "blink"),
            (STRIKE, "strike"),
            (OVERLINE, "overline"),
        ] {
            assert_eq!(c.flags & bit, bit, "{name} was not set");
        }
        // 23/22/25/29/55 turn them off one by one, without touching the others
        t.feed(b"\x1b[23mB");
        let c = t.grid[0].cells[1];
        assert_eq!(c.flags & ITALIC, 0);
        assert_eq!(c.flags & FAINT, FAINT);
        // 22 turns off bold AND faint (that is what xterm does)
        t.feed(b"\x1b[1;22mC");
        let c = t.grid[0].cells[2];
        assert_eq!(c.flags & (BOLD | FAINT), 0);
    }

    #[test]
    fn sgr_underline_styles() {
        let mut t = term();
        let cases: [(&[u8], usize, u8); 7] = [
            (b"\x1b[4m", 0, UL_SINGLE),
            (b"\x1b[4:2m", 1, UL_DOUBLE),
            (b"\x1b[4:3m", 2, UL_CURLY),
            (b"\x1b[4:4m", 3, UL_DOTTED),
            (b"\x1b[4:5m", 4, UL_DASHED),
            (b"\x1b[21m", 5, UL_DOUBLE),
            (b"\x1b[4:0m", 6, UL_NONE),
        ];
        for (seq, col, expected) in cases {
            t.feed(seq);
            t.feed(&[b'A' + col as u8]);
            assert_eq!(t.grid[0].cells[col].ul, expected, "seq {seq:?}");
        }
    }

    #[test]
    fn sgr_underline_color() {
        let mut t = term();
        t.feed(b"\x1b[4m\x1b[58;5;196mA");
        assert_eq!(t.grid[0].cells[0].ul_color, Color::Idx(196));
        let mut t = term();
        t.feed(b"\x1b[58:2::1:2:3mB");
        assert_eq!(t.grid[0].cells[0].ul_color, Color::Rgb(1, 2, 3));
        t.feed(b"\x1b[59mC");
        assert_eq!(t.grid[0].cells[1].ul_color, Color::Default);
        // 0 zeroes everything, including the underline and its color
        let mut t = term();
        t.feed(b"\x1b[4;58;5;9;3;9m\x1b[0mD");
        let c = t.grid[0].cells[0];
        assert_eq!(c.ul, UL_NONE);
        assert_eq!(c.ul_color, Color::Default);
        assert_eq!(c.flags, 0);
    }

    #[test]
    fn sgr_subparameter_with_color_space() {
        let mut t = term();
        t.feed(b"\x1b[38:2::10:20:30mX");
        assert_eq!(t.grid[0].cells[0].fg, Color::Rgb(10, 20, 30));
        let mut t = term();
        t.feed(b"\x1b[48:5:196mX");
        assert_eq!(t.grid[0].cells[0].bg, Color::Idx(196));
    }

    #[test]
    fn erase_and_cursor() {
        let mut t = term();
        t.feed(b"abc\r\nxyz\x1b[2J");
        assert_eq!(screen(&t), vec!["", "", "", ""]);
        assert_eq!((t.cy, t.cx), (0, 0));
        t.feed(b"abc\x1b[1K");
        assert_eq!(t.row_text(0), "");
    }

    #[test]
    fn alt_screen_holds_the_main_buffer() {
        let mut t = term();
        t.feed(b"main\r\n");
        t.feed(b"\x1b[?1049h");
        assert!(t.in_alt_screen());
        t.feed(b"inside");
        assert_eq!(t.row_text(0), "inside");
        t.feed(b"\x1b[?1049l");
        assert!(!t.in_alt_screen());
        assert_eq!(t.row_text(0), "main");
        assert_eq!((t.cy, t.cx), (1, 0));
    }

    #[test]
    fn charset_dec_draws_frame() {
        let mut t = term();
        t.feed(b"\x1b(0lqk\x1b(B");
        assert_eq!(t.row_text(0), "┌─┐");
    }

    #[test]
    fn decstbm_scrolls_inside_the_region() {
        let mut t = term();
        t.feed(b"1\r\n2\r\n3\r\n4");
        t.feed(b"\x1b[2;3r"); // rows 2..3 (1-based)
        t.feed(b"\x1b[3;1H\n");
        assert_eq!(t.row_text(0), "1");
        assert_eq!(t.row_text(1), "3");
        assert_eq!(t.row_text(2), "");
        assert_eq!(t.row_text(3), "4");
    }

    #[test]
    fn il_dl_ich_dch() {
        let mut t = term();
        t.feed(b"abc\r\n");
        t.feed(b"\x1b[1;1H\x1b[1@");
        assert_eq!(t.row_text(0), " abc");
        t.feed(b"\x1b[1P");
        assert_eq!(t.row_text(0), "abc");
        t.feed(b"\x1b[L");
        assert_eq!(t.row_text(0), "");
        assert_eq!(t.row_text(1), "abc");
        t.feed(b"\x1b[M");
        assert_eq!(t.row_text(0), "abc");
    }

    #[test]
    fn save_and_restore_cursor() {
        let mut t = term();
        t.feed(b"\x1b[2;3H\x1b7\x1b[4;1H\x1b8X");
        assert_eq!(t.grid[1].cells[2].ch, 'X');
    }

    #[test]
    fn osc_queries_and_defines_colors() {
        let mut t = term();
        t.theme.fg = Color::Rgb(0x11, 0x22, 0x33);
        t.theme.palette[3] = Color::Rgb(0xff, 0x80, 0x00);
        // query of fg (OSC 10), bg (OSC 11) and palette (OSC 4;3)
        t.feed(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b]4;3;?\x1b\\");
        let out = String::from_utf8_lossy(&t.take_child_out()).to_string();
        assert!(out.contains("]10;rgb:1111/2222/3333"), "{out}");
        assert!(out.contains("]11;rgb:1616/1818/1d1d"), "{out}");
        assert!(out.contains("]4;3;rgb:ffff/8080/0000"), "{out}");
        // defines the palette and the background, and marks it dirty for the renderer
        assert!(!t.theme_dirty);
        t.feed(b"\x1b]4;1;rgb:12/34/56\x1b\\\x1b]11;#000000\x1b\\");
        assert_eq!(t.theme.palette[1], Color::Rgb(0x12, 0x34, 0x56));
        assert_eq!(t.theme.bg, Color::Rgb(0, 0, 0));
        assert!(t.theme_dirty);
        // 104 returns the palette to the default; 111 returns the background
        t.feed(b"\x1b]104;1\x1b\\\x1b]111\x1b\\");
        assert_eq!(t.theme.palette[1], Color::Idx(1));
        assert_eq!(t.theme.bg, Theme::default().bg);
    }

    #[test]
    fn osc_title() {
        let mut t = term();
        t.feed(b"\x1b]0;my title\x07");
        assert_eq!(t.title, "my title");
        t.feed(b"\x1b]2;other\x1b\\");
        assert_eq!(t.title, "other");
    }

    #[test]
    fn title_stack_gives_the_title_back_when_the_app_leaves() {
        // the sequence `mc` sends: push, pop, push, `OSC 0` with its own title,
        // and a pop on exit. Without the pop the tab kept the `mc` title forever
        // and never went back to the cwd fallback (B37).
        let mut t = term();
        t.feed(b"\x1b[22;0t\x1b[23;0t\x1b[22;0t");
        assert_eq!(t.title, "tortty", "the pushes change nothing on their own");
        t.feed(b"\x1b]0;mc [user@host]:/tmp\x07");
        assert_eq!(t.title, "mc [user@host]:/tmp");
        t.feed(b"\x1b[23;0t");
        assert_eq!(t.title, "tortty");
    }

    #[test]
    fn title_stack_nests_like_vim_pushes_it() {
        // vim 9 saves all three (`0`, `2`, `1`) and restores in the same order
        let mut t = term();
        t.feed(b"\x1b]2;before\x07");
        t.feed(b"\x1b[22;0t\x1b[22;2t\x1b[22;1t");
        t.feed(b"\x1b]2;[No Name] - vim\x07");
        t.feed(b"\x1b[23;2t\x1b[23;1t\x1b[23;0t");
        assert_eq!(t.title, "before");
        // an extra pop with an empty stack keeps the title instead of clearing it
        t.feed(b"\x1b[23;0t");
        assert_eq!(t.title, "before");
    }

    #[test]
    fn title_stack_does_not_grow_without_bound() {
        let mut t = term();
        let mut pushes = Vec::new();
        for _ in 0..TITLE_STACK_MAX + 10 {
            pushes.extend_from_slice(b"\x1b[22;0t");
        }
        t.feed(&pushes);
        assert_eq!(t.title_stack.len(), TITLE_STACK_MAX);
        // the deepest save is the one that survives a full unwind
        t.feed(b"\x1b]2;inside\x07");
        for _ in 0..TITLE_STACK_MAX + 10 {
            t.feed(b"\x1b[23;0t");
        }
        assert_eq!(t.title, "tortty");
        assert_eq!(t.title_stack.len(), 0);
    }

    #[test]
    fn osc_title_terminates_on_8_bit_st() {
        let mut t = term();
        // a bare `0x9c` and U+009C itself in UTF-8 both close the OSC; without
        // this the escape stayed open and swallowed everything until the next BEL
        // (B4)
        t.feed(b"\x1b]0;c1\x9c");
        assert_eq!(t.title, "c1");
        t.feed(b"\x1b]0;utf8\xc2\x9c");
        assert_eq!(t.title, "utf8");
        // and the following text shows up on the grid again
        t.feed(b"Xafter");
        assert_eq!(t.grid[0].cells[0].ch, 'X');
    }

    #[test]
    fn osc_title_with_9c_utf8_continuation_is_not_cut() {
        let mut t = term();
        // `d0 9c` is `М` and `e2 90 9c` is `␜`: the 0x9c here is a UTF-8
        // continuation, not ST — cutting in the middle would lose the rest of
        // the title
        t.feed(b"\x1b]0;\xd0\x9c\xd0\xbe \xe2\x90\x9c \x07");
        assert_eq!(t.title, "Мо ␜ ");
    }

    #[test]
    fn osc_does_not_swallow_the_next_title() {
        let mut t = term();
        // OSC closed with an 8-bit ST followed by the next title, separated by
        // another escape: the title must be only the last request
        t.feed(b"\x1b]0;JUNK\x9c\x1b]8;;http://x\x1b\\\x1b]0;CLEAN\x07");
        assert_eq!(t.title, "CLEAN");
    }

    #[test]
    fn esc_in_the_osc_body_ends_the_sequence() {
        let mut t = term();
        // an ESC that is not ST ends the OSC (it does not go back into the
        // body): the title is what came before, and the `[31m` after it becomes
        // real SGR
        t.feed(b"\x1b]2;title\x1b[31mX");
        assert_eq!(t.title, "title");
        assert_eq!(t.grid[0].cells[0].ch, 'X');
        assert_eq!(t.grid[0].cells[0].fg, Color::Idx(1));
    }

    #[test]
    fn osc_title_without_control_characters() {
        let mut t = term();
        t.feed(b"\x1b]0;A\x01B\x7fC\x07");
        assert_eq!(t.title, "ABC");
        // invisible and direction marks go too: `U+202E` would reverse the rest
        // of the tab label
        t.feed("\x1b]0;A\u{202e}B\u{200b}C\u{feff}\x07".as_bytes());
        assert_eq!(t.title, "ABC");
        // control alone does not zero the title (an empty `OSC 0;` was already a
        // no-op)
        t.feed(b"\x1b]0;\x01\x07");
        assert_eq!(t.title, "ABC");
    }

    #[test]
    fn directory_name_does_not_bring_control_into_the_title() {
        // a Linux file name has any byte but `/` and NUL: the title coming from
        // the cwd goes through the same rule as the OSC payload
        assert_eq!(sanitize_title("proj\r\n\u{1b}[31m\u{202e}x"), "proj[31mx");
        assert_eq!(sanitize_title("ok"), "ok");
    }

    #[test]
    fn eight_bit_st_closes_dcs_without_swallowing_the_grid() {
        let mut t = term();
        t.feed(b"\x1bP1$qm\x9cok");
        assert_eq!(t.grid[0].cells[0].ch, 'o');
    }

    #[test]
    fn probe_keeps_the_osc_received() {
        let mut t = term();
        t.feed(b"\x1b]0;one\x07");
        t.feed(b"\x1b]8;;http://x\x07");
        let seen: Vec<&[u8]> = t.osc_seen.iter().map(|o| o.raw.as_slice()).collect();
        assert_eq!(seen, vec![b"0;one".as_slice(), b"8;;http://x".as_slice()]);
        assert_eq!(t.osc_seen[0].title, "one");
        // a link does not touch the title
        assert_eq!(t.osc_seen[1].title, "one");
        // bounded ring: two extra OSC do not grow the history
        for _ in 0..OSC_SEEN + 2 {
            t.feed(b"\x1b]2;x\x07");
        }
        assert_eq!(t.osc_seen.len(), OSC_SEEN);
        assert!(t.osc_seen.back().is_some_and(|o| !o.truncated));
    }

    #[test]
    fn replies_to_the_child() {
        let mut t = term();
        t.feed(b"\x1b[c");
        assert_eq!(t.take_child_out(), b"\x1b[?62;4c".to_vec());
        t.feed(b"\x1b[>c");
        assert_eq!(t.take_child_out(), b"\x1b[>1;100;0c".to_vec());
        t.feed(b"\x1b[2;3H\x1b[6n");
        assert_eq!(t.take_child_out(), b"\x1b[2;3R".to_vec());
        t.feed(b"\x1b[?2026$p");
        assert_eq!(t.take_child_out(), b"\x1b[?2026;2$y".to_vec());
    }

    #[test]
    fn autowrap_off_does_not_wrap() {
        let mut t = term();
        t.feed(b"\x1b[?7l0123456789AB");
        assert_eq!(t.row_text(0), "012345678B");
        assert_eq!(t.row_text(1), "");
    }

    #[test]
    fn scrollback_holds_what_left_the_screen() {
        let mut t = term();
        for i in 0..8 {
            t.feed(format!("line{i}\r\n").as_bytes());
        }
        assert!(!t.scrollback.is_empty());
        let hist = t.history_text(0, 10);
        assert_eq!(hist[0], "line0");
    }

    /// Text of all rows (scrollback + screen), in reading order.
    fn all(t: &Term) -> String {
        t.scrollback
            .iter()
            .chain(t.grid.iter())
            .map(Row::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn view_stays_in_place_while_output_arrives() {
        let mut t = Term::new(20, 4);
        for i in 0..10 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        // the reader went 3 rows back
        let (mark, off) = (t.mark(), 3usize);
        let before = t.abs_row(t.scrolled as i64 - off as i64).map(Row::text);
        // 2 more rows arrive: it stays on the same absolute row
        t.feed(b"new1\r\nnew2\r\n");
        let off2 = t.view_after_output(off, mark);
        let after = t.abs_row(t.scrolled as i64 - off2 as i64).map(Row::text);
        assert_eq!(before, after, "the view moved along with the output");
        // at the bottom (off=0) it stays at the bottom
        assert_eq!(t.view_after_output(0, mark), 0);
    }

    #[test]
    fn a_window_that_only_appends_moves_the_reader() {
        // the `pi` shape (`B53`): the lines are wrapped in `?2026` and they are
        // new output, not a redrawn screen — so the offset has to grow with them,
        // or the reader slides under a frozen number
        let mut t = Term::new(20, 4);
        for i in 0..10 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        let (mark, off) = (t.mark(), 3usize);
        let before = t.scrolled;
        t.feed(b"\x1b[?2026h");
        for i in 0..6 {
            t.feed(format!("r{i}\r\n").as_bytes());
        }
        t.feed(b"\x1b[?2026l");
        let pushed = usize::try_from(t.scrolled - before).expect("rows fit in usize");
        assert!(pushed > 0, "the window scrolled rows");
        assert_eq!(t.scrolled_sync, 0, "an append is not a repaint");
        assert_eq!(
            t.view_after_output(off, mark),
            (off + pushed).min(t.scrollback.len()),
            "the reader stayed on the text it was reading"
        );
        // output outside the window still moves it (D10)
        t.feed(b"\x1b[?2026h\x1b[?2026l");
        let mark = t.mark();
        t.feed(b"out1\r\nout2\r\n");
        assert_eq!(t.view_after_output(off, mark), off + 2);
    }

    #[test]
    fn a_window_that_repaints_does_not_move_the_reader() {
        let mut t = Term::new(20, 4);
        for i in 0..10 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        let (mark, off) = (t.mark(), 3usize);
        // `2J` + home before the rows: the screen is being drawn again
        t.feed(b"\x1b[?2026h\x1b[2J\x1b[H");
        for i in 0..6 {
            t.feed(format!("r{i}\r\n").as_bytes());
        }
        t.feed(b"\x1b[?2026l");
        assert!(t.scrolled_sync > 0, "the window did scroll rows");
        assert_eq!(
            t.view_after_output(off, mark),
            off,
            "a repaint moved the reader"
        );
    }

    #[test]
    fn erasing_the_history_does_not_throw_the_view_at_the_top() {
        // the `pi` case: `2J` + `3J` + a reprinted transcript inside one window,
        // with the reader 59 rows up
        let mut t = Term::new(20, 4);
        for i in 0..60 {
            t.feed(format!("old{i}\r\n").as_bytes());
        }
        let off = 50usize.min(t.scrollback.len());
        let mark = t.mark();
        t.feed(b"\x1b[?2026h\x1b[2J\x1b[3J\x1b[H");
        for i in 0..200 {
            t.feed(format!("new{i}\r\n").as_bytes());
        }
        t.feed(b"\x1b[?2026l");
        assert!(t.scrollback.len() > 150, "the history was rebuilt");
        assert_eq!(
            t.view_after_output(off, mark),
            off,
            "the view was carried to the top of the new history"
        );
        // a `3J` outside any window is a `clear`: the saved lines are gone, so the
        // view goes back to the live screen instead of freezing over nothing
        let mark = t.mark();
        t.feed(b"\x1b[3J");
        assert_eq!(
            t.view_after_output(off, mark),
            0,
            "nothing above to look at"
        );
    }

    #[test]
    fn view_discounts_what_fell_out_of_a_full_history() {
        let mut t = Term::new(20, 2);
        t.scrollback_limit = 3;
        for i in 0..5 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        let (mark, off) = (t.mark(), 2usize);
        // 5 rows at once: the scrollback (cap 3) drops the oldest
        for i in 5..10 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        let off2 = t.view_after_output(off, mark);
        assert!(
            off2 <= t.scrollback.len(),
            "it cannot point past the scrollback"
        );
    }

    #[test]
    fn reflow_narrows_and_widens_preserving_the_text() {
        let mut t = term(); // 10x4
        t.feed(b"abcdefghijklmnopqrstuvwxy"); // 25 cells
        assert_eq!(screen(&t)[..3], ["abcdefghij", "klmnopqrst", "uvwxy"]);
        t.resize(5, 4);
        // 25 cells become 5 rows of 5: only the first goes to the scrollback
        // (the screen keeps the end of the paragraph, like any terminal). The
        // cursor is on the last row, so no blank goes down into the scrollback.
        assert_eq!(all(&t), "abcde\nfghij\nklmno\npqrst\nuvwxy");
        assert_eq!(t.scrollback.len(), 1);
        assert_eq!(t.row_text(3), "uvwxy");
        // back: rewraps again at the original width
        t.resize(10, 4);
        assert_eq!(screen(&t)[..3], ["abcdefghij", "klmnopqrst", "uvwxy"]);
    }

    #[test]
    fn reflow_puts_the_cursor_on_the_right_cell() {
        let mut t = term();
        t.feed(b"abcdefghijklmno"); // 15 cells → cy=1, cx=5
        assert_eq!((t.cy, t.cx), (1, 5));
        t.resize(5, 4);
        // 15 cells: 3 rows of 5, cursor at the end of the third (pending wrap).
        // The 3 rows fit on screen, so there is no scrollback and the cursor
        // stays on row 2.
        assert_eq!((t.cy, t.cx), (2, 4));
        assert!(t.wrap_pending);
        t.feed(b"!");
        assert_eq!(t.row_text(2), "klmno");
        assert_eq!(t.row_text(3), "!", "the pending wrap has to count");
    }

    #[test]
    fn reflow_carries_the_selection_to_the_same_text() {
        let mut t = Term::new(20, 4);
        t.feed(b"alpha beta gamma delta");
        // "beta" in the first row of the paragraph
        let sel = Span::new(t.anchor_at(0, 0, 6), t.anchor_at(0, 0, 9));
        assert_eq!(t.span_text(&sel).as_deref(), Some("beta"));
        let kept = t
            .resize_keep(10, 4, Some(sel))
            .expect("the anchor survives");
        assert_eq!(t.cols, 10);
        assert_eq!(
            t.span_text(&kept).as_deref(),
            Some("beta"),
            "the same characters, on a row that moved"
        );
        // and back to the original width
        let back = t
            .resize_keep(20, 4, Some(kept))
            .expect("the anchor survives the way back");
        assert_eq!(t.span_text(&back).as_deref(), Some("beta"));
    }

    #[test]
    fn reflow_carries_a_selection_that_spans_a_wrap() {
        let mut t = Term::new(20, 4);
        t.feed(b"alpha beta gamma delta");
        // from "beta" to the end of "gamma": two rows after the narrowing
        let sel = Span::new(t.anchor_at(0, 0, 6), t.anchor_at(0, 0, 16));
        let before = t.span_text(&sel).expect("alive");
        let kept = t
            .resize_keep(10, 4, Some(sel))
            .expect("the anchor survives");
        assert_eq!(t.span_text(&kept), Some(before));
    }

    #[test]
    fn reflow_lets_the_selection_die_when_the_ceiling_eats_its_rows() {
        let mut t = Term::new(20, 3);
        t.scrollback_limit = 2;
        for i in 0..8 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        // the oldest row the scrollback still has: narrowing doubles the rows
        // and the ceiling throws this one away
        let sel = Span::new(
            Anchor {
                line: t.abs_first(),
                col: 0,
                gen: t.abs_gen,
            },
            Anchor {
                line: t.abs_first(),
                col: 3,
                gen: t.abs_gen,
            },
        );
        assert!(t.span_text(&sel).is_some());
        assert!(
            t.resize_keep(4, 3, Some(sel)).is_none(),
            "a row that left the history cannot be carried"
        );
    }

    #[test]
    fn reflow_marks_a_soft_break_only_on_middle_rows() {
        let mut t = term();
        t.feed(b"abcdefghijklmnopqrstuvwxy\r\nshort");
        t.resize(5, 8);
        // "abcdefghijklmnopqrstuvwxy" becomes 5 rows: 4 soft + 1 hard
        assert!(t.grid[0].wrapped && t.grid[1].wrapped && t.grid[3].wrapped);
        assert!(
            !t.grid[4].wrapped,
            "the end of the paragraph is not a soft break"
        );
        assert!(!t.grid[5].wrapped, "the next row is another paragraph");
        assert_eq!(t.row_text(5), "short");
    }

    #[test]
    fn reflow_does_not_eat_spaces_with_a_painted_bg() {
        // A23: a space with a painted background is content (status line, git diff)
        let mut t = term();
        t.feed(b"\x1b[48;2;200;0;0mabc       "); // 3 letters + 7 painted spaces
        t.resize(20, 4);
        assert_eq!(t.grid[0].cells[9].bg, Color::Rgb(200, 0, 0));
        // the same without paint: the trailing spaces go away, "abc" remains
        let mut u = term();
        u.feed(b"abc       ");
        u.resize(20, 4);
        assert_eq!(u.row_text(0), "abc");
        assert_eq!(u.grid[0].cells[5].bg, Color::Default);
    }

    #[test]
    fn reflow_does_not_split_a_wide_pair_in_the_middle() {
        let mut t = Term::new(10, 4);
        // 5 ideograms: 10 cells (each one takes 2)
        for _ in 0..5 {
            t.feed("漢".as_bytes());
        }
        t.resize(5, 6);
        for (y, row) in t.grid.iter().enumerate().take(6) {
            assert!(
                !row.cells[0].is_cont(),
                "row {y} starts with half a wide pair"
            );
        }
        let text: String = (0..6).map(|y| t.row_text(y)).collect();
        assert_eq!(text, "漢漢漢漢漢", "no ideogram may vanish in the reflow");
    }

    #[test]
    fn reflow_keeps_the_history_in_order() {
        let mut t = Term::new(20, 4);
        for i in 0..50 {
            t.feed(format!("line {i:02}\r\n").as_bytes());
        }
        t.resize(10, 4);
        let after = all(&t);
        assert!(
            after.contains("line 00") && after.contains("line 49"),
            "the reflow must not lose the ends of the history"
        );
        // the rows stay in increasing order
        let pos: Vec<usize> = (0..50)
            .map(|i| after.find(&format!("line {i:02}")).expect("row vanished"))
            .collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "history out of order");
    }

    /// Every cluster in reading order: base and the codepoints beside it.
    fn clusters(t: &Term) -> Vec<(char, Vec<char>)> {
        let mut v = Vec::new();
        for row in t.scrollback.iter().chain(t.grid.iter()) {
            for (x, c) in row.cells.iter().enumerate() {
                if c.is_cont() {
                    continue;
                }
                if !row.tail_at(x).is_empty() {
                    v.push((c.ch, row.tail_at(x).to_vec()));
                }
            }
        }
        v
    }

    /// The reflow cuts a row by column and the tail is addressed by column, so
    /// the cut carries it (`B68`). Without that, one window resize turned every
    /// emoji back into its first codepoint — and the snapshot saved that.
    #[test]
    fn a_resize_carries_the_cluster() {
        let mut t = term();
        t.feed(
            "\u{1F1E7}\u{1F1F7} \u{1F44D}\u{1F3FD} \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"
                .as_bytes(),
        );
        let before = clusters(&t);
        assert_eq!(before.len(), 3, "three clusters: {before:?}");
        for cols in [40, 8, 40, 25, 5] {
            t.resize(cols, 6);
            assert_eq!(clusters(&t), before, "cols {cols}");
        }
    }

    #[test]
    fn resize_preserves_runs_into_the_scrollback() {
        let mut t = term();
        for i in 0..6 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        t.resize(20, 3);
        assert_eq!(t.rows, 3);
        assert!(t.scrollback.len() >= 3);
        // what left through the top went to the scrollback; the live area slid
        assert_eq!(t.row_text(0), "l4");
        assert_eq!(t.row_text(1), "l5");
        assert_eq!(t.row_text(2), "");
    }

    /// Shrinking and growing back (what the WM does when maximizing) must not
    /// move the content: the prompt row is still the first one.
    #[test]
    fn grow_after_shrink_does_not_take_the_prompt_to_the_middle() {
        let mut t = Term::new(20, 12);
        t.feed(b"prompt");
        for (c, r) in [(20, 16), (20, 12), (20, 16)] {
            t.resize(c, r);
            assert_eq!(t.row_text(0), "prompt", "{c}x{r}: the text moved down");
            assert_eq!(t.cy, 0, "{c}x{r}: the cursor moved down");
        }
        assert!(
            t.scrollback.is_empty(),
            "an empty row below the cursor does not go to the scrollback: {:?}",
            t.scrollback.len()
        );
        // and text WITH content ABOVE the cursor still goes, as before
        let mut u = Term::new(20, 12);
        for i in 0..14 {
            u.feed(format!("l{i}\r\n").as_bytes());
        }
        u.resize(20, 6);
        assert!(
            !u.scrollback.is_empty(),
            "what leaves through the top has to go to the scrollback"
        );
    }

    #[test]
    fn utf8_incremental_spans_chunks() {
        let mut t = term();
        let bytes = "▀".as_bytes();
        t.feed(&bytes[..1]);
        t.feed(&bytes[1..2]);
        t.feed(&bytes[2..]);
        assert_eq!(t.grid[0].cells[0].ch, '▀');
    }

    #[test]
    fn a_wide_char_takes_two_cells() {
        let mut t = term();
        t.feed("漢".as_bytes());
        assert_eq!(t.grid[0].cells[0].ch, '漢');
        assert!(t.grid[0].cells[1].is_cont());
        assert_eq!(t.cx, 2);
    }

    /// `B47`: a notification that came from `OSC 7501` carries its state, so the
    /// toast can be yellow or red instead of always the receipt green, and the
    /// title names the state and the app that asked. `OSC 9` has neither.
    #[test]
    fn a_status_notification_says_its_state_and_its_app() {
        let mut t = term();
        let msg = "VGVzdCBibG9ja2Vk"; // "Test blocked"
        t.feed(format!("\x1b]7501;state=blocked:app=test:msg={msg}\x1b\\").as_bytes());
        assert_eq!(t.notifs, 1);
        {
            let n = t.notifications.entries().next().expect("one entry");
            assert_eq!(n.source, crate::notif::Source::Osc7501);
            assert_eq!(n.title.as_deref(), Some("blocked · test"));
            assert_eq!(n.body.as_deref(), Some("Test blocked"));
            assert_eq!(n.shade, crate::notif::Shade::Warn);
        }
        // `error` is the same road with another shade
        let mut e = term();
        // `msg` is base64 in the spec, and one that does not decode discards the
        // whole report: "boom" is `Ym9vbQ==`
        e.feed(b"\x1b]7501;state=error:msg=Ym9vbQ==\x1b\\");
        assert_eq!(
            e.notifications.entries().next().map(|n| n.shade),
            Some(crate::notif::Shade::Err)
        );
        // a plain notification says nothing about itself and is its own entry:
        // the `7501` one is still there
        t.feed(b"\x1b]9;build ok\x1b\\");
        assert_eq!(t.notifications.len(), 2);
        assert_eq!(t.last_notification(), "build ok");
        // `done` asks for the user as well, and still lands here
        let mut d = term();
        d.feed(b"\x1b]7501;state=done:app=cargo\x1b\\");
        assert_eq!(
            d.notifications.entries().next().map(|n| n.title.as_deref()),
            // `app == text` here, so the title is only the word
            Some(Some("done"))
        );
        // `working`/`idle` do not notify at all
        let mut w = term();
        w.feed(b"\x1b]7501;state=working:app=cargo\x1b\\");
        assert_eq!(w.notifs, 0);
    }

    /// A new prompt takes the records and the bar, and it must **not** hand the
    /// root back to `9;4`: the terminal already heard a `7501` report.
    #[test]
    fn a_new_prompt_takes_the_state_and_the_bar() {
        let mut t = term();
        t.feed(b"\x1b]7501;state=done:app=cargo\x1b\\");
        // a record under the root goes with it: the whole tab was read
        t.feed(b"\x1b]7501;state=blocked:id=sub/task:app=git\x1b\\");
        assert_eq!(t.status.len(), 2);
        t.feed(b"\x1b]133;A\x07");
        assert_eq!(t.status.len(), 0);
        assert_eq!(t.progress, None, "the bar goes with the record");
        // the prompt left the root free: the lock was that report's `kind`, and
        // it went with the records — so the keepalive may map again
        t.feed(b"\x1b]9;4;1;40\x1b\\");
        assert_eq!(t.status.len(), 1);
        assert_eq!(t.progress.map(|p| (p.state, p.pct)), Some((1, Some(40))));
        // ...and the program reporting again takes the bar back, by design
        t.feed(b"\x1b]7501;state=working:app=cargo:progress=40\x1b\\");
        assert_eq!(t.progress.map(|p| (p.state, p.pct)), Some((1, Some(40))));
    }

    /// `idle` is not in flight: nothing draws it, and the bar is gone.
    #[test]
    fn an_idle_record_has_nothing_to_draw() {
        let mut t = term();
        t.feed(b"\x1b]7501;state=idle:app=cargo\x1b\\");
        assert_eq!(
            t.status.root().map(|r| r.state),
            Some(crate::status::State::Idle)
        );
        assert_eq!(t.progress, None);
    }

    /// `9;4;0` ends the **record**, not only the bar: the old path cleared
    /// `progress` and returned before `from_progress` could remove the root.
    #[test]
    fn osc9_4_zero_clears_the_record_too() {
        let mut t = term();
        t.feed(b"\x1b]9;4;1;40\x1b\\");
        assert_eq!(t.status.len(), 1);
        t.feed(b"\x1b]9;4;0\x1b\\");
        assert_eq!(t.status.len(), 0, "the record goes with the bar");
        assert_eq!(t.progress, None, "nothing is left to draw");
    }

    /// `9;4;2` is a final state: it asks the user like `7501 error` does, and a
    /// finished command has no bar.
    #[test]
    fn osc9_4_error_notifies_and_takes_the_bar() {
        let mut t = term();
        t.feed(b"\x1b]9;4;1;40\x1b\\");
        assert_eq!(t.progress.map(|p| p.state), Some(1));
        t.feed(b"\x1b]9;4;2\x1b\\");
        assert_eq!(t.notifs, 1, "a finished command is news");
        assert_eq!(t.progress, None, "and it has no bar any more");
    }

    /// A new prompt ends the record, so the next command gets its own notice —
    /// otherwise the idempotence rule would swallow it (the same `done` twice).
    #[test]
    fn a_new_prompt_forgets_the_record_so_the_next_command_notifies() {
        let mut t = term();
        t.feed(b"\x1b]7501;state=done:app=cargo\x1b\\");
        assert_eq!(t.notifs, 1);
        // the same report again is the same fact: silence
        t.feed(b"\x1b]7501;state=done:app=cargo\x1b\\");
        assert_eq!(t.notifs, 1);
        // the shell came back: the command that produced it is gone
        t.feed(b"\x1b]133;A\x07");
        assert_eq!(t.status.len(), 0);
        // so the next command ending the same way is news again
        t.feed(b"\x1b]7501;state=done:app=cargo\x1b\\");
        assert_eq!(t.notifs, 2);
    }

    #[test]
    fn rep_repeats_the_last_character() {
        let mut t = term();
        t.feed(b"a\x1b[3b");
        assert_eq!(t.row_text(0), "aaaa");
    }

    #[test]
    fn lnm_makes_lf_return_to_the_column_too() {
        let mut t = term();
        // without LNM the LF only goes down, and the next line starts where the
        // cursor was
        t.feed(b"ab\ncd");
        assert_eq!(t.row_text(1), "  cd");
        let mut u = term();
        u.feed(b"\x1b[20hab\ncd");
        assert_eq!(u.row_text(1), "cd");
        // and it turns off again: ANSI mode 20 is not private, so no `?`
        let mut v = term();
        v.feed(b"\x1b[20h\x1b[20lab\ncd");
        assert_eq!(v.row_text(1), "  cd");
    }

    #[test]
    fn osc9_counts_the_notification_and_keeps_its_text() {
        let mut t = term();
        t.feed(b"\x1b]9;build ok\x07");
        assert_eq!((t.notifs, t.last_notification().as_str()), (1, "build ok"));
        // empty and control-only payloads are not notifications
        t.feed(b"\x1b]9;\x07");
        t.feed(b"\x1b]9;\x01\x02\x07");
        assert_eq!((t.notifs, t.last_notification().as_str()), (1, "build ok"));
    }

    #[test]
    fn osc777_keeps_title_and_body_apart() {
        let mut t = term();
        t.feed(b"\x1b]777;notify;make;all done\x07");
        {
            let n = t.notifications.entries().next().expect("one entry");
            assert_eq!(n.source, crate::notif::Source::Osc777);
            assert_eq!(n.title.as_deref(), Some("make"));
            assert_eq!(n.body.as_deref(), Some("all done"));
            assert_eq!(n.text(), "make — all done");
        }
        t.feed(b"\x1b]777;notify;;body only\x07");
        assert_eq!(t.last_notification(), "body only");
        assert_eq!(t.notifications.len(), 2, "no id: a new entry each time");
    }

    #[test]
    fn osc777_ignores_anything_but_notify() {
        let mut t = term();
        t.feed(b"\x1b]777;foo;bar\x07");
        t.feed(b"\x1b]777\x07");
        assert_eq!(t.notifs, 0);
        assert!(t.notifications.is_empty());
    }

    #[test]
    fn osc52_sets_the_clipboard_from_base64() {
        use base64::Engine as _;
        // the `ssh` case from `M-R`: a remote program copies, the system
        // clipboard gets the text
        let mut t = term();
        let b64 = base64::engine::general_purpose::STANDARD.encode("hi from ssh");
        let seq = format!("\x1b]52;c;{b64}\x07");
        t.feed(seq.as_bytes());
        assert_eq!(
            t.take_clipboard(),
            Some(ClipboardOp::Clipboard("hi from ssh".to_string()))
        );
        // the drain is once: a second take finds nothing
        assert_eq!(t.take_clipboard(), None);
    }

    #[test]
    fn osc52_primary_goes_to_primary_and_empty_clears() {
        use base64::Engine as _;
        let mut t = term();
        let b64 = base64::engine::general_purpose::STANDARD.encode("sel");
        let seq = format!("\x1b]52;p;{b64}\x07");
        t.feed(seq.as_bytes());
        assert_eq!(
            t.take_clipboard(),
            Some(ClipboardOp::Primary("sel".to_string()))
        );
        // an empty payload clears, as in xterm
        t.feed(b"\x1b]52;c;\x07");
        assert_eq!(
            t.take_clipboard(),
            Some(ClipboardOp::Clipboard(String::new()))
        );
    }

    #[test]
    fn osc52_query_never_answers() {
        let mut t = term();
        // `?` would hand our clipboard to the child: refused, with no reply
        t.feed(b"\x1b]52;c;?\x07");
        assert_eq!(t.take_clipboard(), None);
        assert!(t.take_child_out().is_empty(), "no answer leaks outward");
    }

    #[test]
    fn osc52_ignores_anything_but_clipboard_and_primary() {
        use base64::Engine as _;
        let mut t = term();
        let b64 = base64::engine::general_purpose::STANDARD.encode("hi");
        for pc in ["s", "x", ""] {
            let seq = format!("\x1b]52;{pc};{b64}\x07");
            t.feed(seq.as_bytes());
            assert_eq!(t.take_clipboard(), None, "Pc={pc:?} sets nothing");
        }
        // no payload at all is not a clear
        t.feed(b"\x1b]52;c\x07");
        assert_eq!(t.take_clipboard(), None);
        // skewed base64 is not a clipboard
        t.feed(b"\x1b]52;c;!!!\x07");
        assert_eq!(t.take_clipboard(), None);
    }

    #[test]
    fn osc52_refuses_binary_and_control_characters() {
        use base64::Engine as _;
        let e = base64::engine::general_purpose::STANDARD;
        let mut t = term();
        // not UTF-8: refused, not scrubbed
        let seq = format!("\x1b]52;c;{}\x07", e.encode([0xff, 0xfe]));
        t.feed(seq.as_bytes());
        assert_eq!(t.take_clipboard(), None);
        // NUL inside text: refused too
        let seq = format!("\x1b]52;c;{}\x07", e.encode(b"a\x00b"));
        t.feed(seq.as_bytes());
        assert_eq!(t.take_clipboard(), None);
        // tab and newlines are text, not junk
        let seq = format!("\x1b]52;c;{}\x07", e.encode("a\tb\nc"));
        t.feed(seq.as_bytes());
        assert_eq!(
            t.take_clipboard(),
            Some(ClipboardOp::Clipboard("a\tb\nc".to_string()))
        );
    }

    #[test]
    fn osc52_cut_at_the_osc_cap_is_refused() {
        // a body past `MAX_OSC` still decodes (cut base64 is valid base64),
        // so a truncated write must not land a mangled clipboard
        let mut t = term();
        let mut seq = b"\x1b]52;c;".to_vec();
        seq.extend(std::iter::repeat_n(b'A', 5000));
        seq.extend_from_slice(b"\x07");
        t.feed(&seq);
        assert_eq!(t.take_clipboard(), None);
    }

    #[test]
    fn osc7_stores_the_shell_cwd() {
        let mut t = term();
        t.feed(b"\x1b]7;file://host/home/j/user/a%20b\x07");
        assert_eq!(t.cwd.as_deref(), Some("/home/j/user/a b"));
        // an empty payload is the shell saying it does not know
        t.feed(b"\x1b]7;\x07");
        assert_eq!(t.cwd, None);
        // junk does not throw away what is known
        t.feed(b"\x1b]7;file://h/tmp\x07");
        t.feed(b"\x1b]7;junk\x07");
        assert_eq!(t.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn osc7_keeps_a_lone_percent_as_written() {
        let mut t = term();
        t.feed(b"\x1b]7;file://h/tmp/100%\x07");
        assert_eq!(t.cwd.as_deref(), Some("/tmp/100%"));
        t.feed(b"\x1b]7;file://h/tmp/%zz\x07");
        assert_eq!(t.cwd.as_deref(), Some("/tmp/%zz"));
    }

    #[test]
    fn osc8_links_the_cells_between_open_and_close() {
        let mut t = term();
        t.feed(b"\x1b]8;;https://a.example/x\x1b\\hi\x1b]8;;\x1b\\yo");
        assert_eq!(t.grid[0].cells[0].link, 1);
        assert_eq!(t.grid[0].cells[1].link, 1);
        assert_eq!(t.grid[0].cells[2].link, 0, "the close ends the span");
        assert_eq!(t.link_uri(1), Some("https://a.example/x"));
        assert_eq!(t.link_uri(0), None);
        assert_eq!(t.link_uri(9), None);
    }

    #[test]
    fn osc8_reuses_the_index_and_carries_the_link_to_a_wide_char() {
        let mut t = term();
        t.feed(b"\x1b]8;;https://a\x1b\\a\x1b]8;;\x1b\\ \x1b]8;;https://a\x1b\\b");
        // same URI, same index: the table does not grow per span
        assert_eq!(t.links.len(), 1);
        assert_eq!(t.grid[0].cells[0].link, 1);
        assert_eq!(t.grid[0].cells[2].link, 1);
        // both halves of a wide char carry it, so a redraw of the pair keeps it
        let mut u = term();
        u.feed("\x1b]8;;https://b\x1b\\漢".as_bytes());
        assert_eq!(u.grid[0].cells[0].link, 1);
        assert_eq!(u.grid[0].cells[1].link, 1);
        assert!(u.grid[0].cells[1].is_cont());
    }

    #[test]
    fn osc8_drops_control_characters_and_stops_at_the_cap() {
        let mut t = term();
        t.feed(b"\x1b]8;;https://a\x07x\x1b]8;;\x07");
        assert_eq!(t.link_uri(1), Some("https://a"), "BEL ends the OSC");
        // past the cap the text stays text, without a link
        let mut u = term();
        for i in 0..(LINK_MAX + 4) {
            u.feed(format!("\x1b]8;;https://e/{i}\x07\x1b]8;;\x07").as_bytes());
        }
        assert_eq!(u.links.len(), LINK_MAX);
        u.feed(b"\x1b]8;;https://e/over\x07Q\x1b]8;;\x07");
        assert_eq!(u.grid[0].cells[0].link, 0);
    }

    #[test]
    fn the_sync_trace_counts_what_the_window_cleared() {
        let mut t = term();
        t.feed(b"\x1b[?2026h");
        assert!(t.sync_open().is_some());
        t.feed(b"\x1b[2J\x1b[H\x1b[2K\x1b[?1049h\x1b[4;1r\x1b[!p");
        assert_eq!(
            t.take_sync_closed().len(),
            0,
            "still open, nothing to drain"
        );
        t.feed(b"\x1b[?2026l");
        let closed = t.take_sync_closed();
        assert_eq!(closed.len(), 1);
        let tr = &closed[0];
        assert_eq!(tr.ed[2], 1, "2J");
        assert_eq!(tr.el, 1, "2K");
        assert_eq!(tr.home, 1, "H");
        assert_eq!(tr.alt, 1, "?1049h");
        assert_eq!(tr.stbm, 1, "CSI r");
        assert_eq!(tr.decstr, 1, "CSI ! p");
        assert_eq!((tr.ris, tr.ed[3]), (0, 0), "no reset, no 3J");
        // the read that opens the window counts as a feed but not as bytes
        assert_eq!(tr.feeds, 3);
        assert_eq!(
            tr.bytes,
            (b"\x1b[2J\x1b[H\x1b[2K\x1b[?1049h\x1b[4;1r\x1b[!p".len() + b"\x1b[?2026l".len())
                as u64
        );
        // a second window is a second record
        t.feed(b"\x1b[?2026h\x1b[3J\x1b[?2026l");
        let closed = t.take_sync_closed();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].ed[3], 1, "3J inside the second window");
        assert_eq!(closed[0].feeds, 1, "one read carried the whole window");
        assert_eq!(closed[0].bytes, 0);
    }

    #[test]
    fn a_reset_inside_the_window_closes_the_trace() {
        let mut t = term();
        t.feed(b"\x1b[?2026h\x1b[2J");
        t.feed(b"\x1bc");
        assert!(!t.sync_output);
        let closed = t.take_sync_closed();
        assert_eq!(closed.len(), 1, "RIS ends the window");
        assert_eq!(closed[0].ris, 1);
        assert_eq!(closed[0].ed[2], 1);
    }

    #[test]
    fn the_sync_trace_counts_the_lines_the_window_pushed_to_the_history() {
        let mut t = term();
        t.feed(b"\x1b[1000;1H"); // last row: the next line feeds scroll up
        let before = t.scrollback.len();
        t.feed(b"\x1b[?2026h");
        for _ in 0..3 {
            t.feed(b"\r\n");
        }
        t.feed(b"\x1b[?2026l");
        let closed = t.take_sync_closed();
        assert_eq!(closed.len(), 1);
        let hist = (t.scrollback.len() - before) as u64;
        assert!(
            hist > 0,
            "the cursor was at the bottom: lines went to history"
        );
        assert_eq!(closed[0].hist, hist);
    }

    #[test]
    fn deckpam_sets_the_keypad_application_mode() {
        let mut t = term();
        assert!(!t.keypad_app, "the numeric keypad is the default");
        assert_eq!(t.mode_state(66, true), 2);
        t.feed(b"\x1b="); // DECKPAM
        assert!(t.keypad_app);
        assert_eq!(t.mode_state(66, true), 1, "DECRQM has to see the mode");
        t.feed(b"\x1b>"); // DECKPNM
        assert!(!t.keypad_app);
        // ESC = inside a sequence of its own does not eat what follows
        t.feed(b"\x1b=7");
        assert!(t.keypad_app);
        assert_eq!(t.row_text(0), "7");
    }

    #[test]
    fn a_soft_reset_and_a_ris_take_the_keypad_back_to_numeric() {
        let mut t = term();
        t.feed(b"\x1b=");
        t.feed(b"\x1b[!p"); // DECSTR
        assert!(!t.keypad_app);
        t.feed(b"\x1b=");
        t.feed(b"\x1bc"); // RIS
        assert!(!t.keypad_app);
    }

    /// `?6h` addresses the absolute moves from the region top; a relative move
    /// starts from the row the cursor is on, so it must not take the offset a
    /// second time (B58: `CUU` walked two rows down, `CHA` changed the row).
    #[test]
    fn origin_mode_offsets_the_absolute_moves_only() {
        let mut t = Term::new(20, 10);
        t.feed(b"\x1b[4;9r"); // DECSTBM: rows 3..8, 0-based
        t.feed(b"\x1b[?6h");
        assert_eq!(t.cy, 3, "DECSTBM homes to the region top with origin on");
        t.feed(b"\x1b[3;1H"); // CUP 3 is the third row of the region
        assert_eq!(t.cy, 5);
        t.feed(b"\x1b[A");
        assert_eq!(t.cy, 4, "CUU 1 walks up one row");
        t.feed(b"\x1b[B");
        assert_eq!(t.cy, 5, "CUD 1 walks down one row");
        t.feed(b"\x1b[99A");
        assert_eq!(t.cy, 3, "CUU stops at the region top");
        t.feed(b"\x1b[99B");
        assert_eq!(t.cy, 8, "CUD stops at the region bottom");
        t.feed(b"\x1b[2E"); // CNL
        assert_eq!(
            (t.cy, t.cx),
            (8, 0),
            "CNL keeps to the margin, first column"
        );
        t.feed(b"\x1b[2F"); // CPL
        assert_eq!(
            (t.cy, t.cx),
            (6, 0),
            "CPL keeps to the margin, first column"
        );
        t.feed(b"\x1b[5G"); // CHA
        assert_eq!((t.cy, t.cx), (6, 4), "CHA moves the column, not the row");
        t.feed(b"\x1b[H");
        assert_eq!(
            (t.cy, t.cx),
            (3, 0),
            "home is the region top with origin on"
        );
        t.feed(b"\x1b[2d");
        assert_eq!(t.cy, 4, "VPA counts from the region top");
        t.feed(b"\x1b[99d");
        assert_eq!(t.cy, 8, "VPA stops at the region bottom");
    }

    /// The margins stop a relative walk from inside the region — with origin on
    /// or off — but they are not a fence around it: a cursor above the region
    /// reaches the screen top and one below reaches the last row (xterm's
    /// `CursorUp`/`CursorDown`).
    #[test]
    fn relative_moves_see_the_margins_only_from_inside() {
        let mut t = Term::new(20, 10);
        t.feed(b"\x1b[4;9r"); // rows 3..8
        assert_eq!(t.cy, 0, "without origin the DECSTBM home is the screen top");
        t.feed(b"\x1b[5;1H"); // row 4, inside
        t.feed(b"\x1b[99A");
        assert_eq!(t.cy, 3, "a walk from inside stops at the top margin");
        t.feed(b"\x1b[99B");
        assert_eq!(t.cy, 8, "a walk from inside stops at the bottom margin");
        t.feed(b"\x1b[2;1H"); // row 1, above the region
        t.feed(b"\x1b[5A");
        assert_eq!(t.cy, 0, "above the margin the screen top is the limit");
        t.feed(b"\x1b[10;1H"); // row 9, below the region
        t.feed(b"\x1b[5B");
        assert_eq!(t.cy, 9, "below the margin the last row is the limit");
        t.feed(b"\x1b[5;1H");
        t.feed(b"\x1b[5C\x1b[99D");
        assert_eq!((t.cy, t.cx), (4, 0), "CUF/CUB keep the row they found");
    }

    #[test]
    fn pixel_mouse_mode_answers_decrqm() {
        let mut t = term();
        // DECRQM answers 2 (known, reset) before and 1 (set) after
        assert_eq!(t.mode_state(1016, true), 2);
        t.feed(b"\x1b[?1016h");
        assert!(t.mouse_px);
        assert_eq!(t.mode_state(1016, true), 1);
        t.feed(b"\x1b[?1016l");
        assert!(!t.mouse_px);
    }
}
