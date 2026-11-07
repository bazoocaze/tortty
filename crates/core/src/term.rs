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

const REPLACEMENT: char = '\u{FFFD}';

/// Ceiling for the payload of an in-flight OSC: a sequence that never closes
/// must not grow without bound.
const MAX_OSC: usize = 4096;
/// How many distinct `OSC 8` URIs are kept (a span reuses its index).
pub const LINK_MAX: usize = 4096;
/// How many chars of an `OSC 9`/`777` notification are kept: it is shown in one
/// toast line, and a child that notifies in a loop must not grow it.
pub const NOTIF_MAX: usize = 512;
/// How many OSC the probe keeps, and how much of each (the socket's `osc` verb).
const OSC_SEEN: usize = 8;
const OSC_SEEN_MAX: usize = 256;
/// Ceiling for the title stack (`CSI 22/23 t`): an app that pushes without
/// popping must not grow it without bound.
const TITLE_STACK_MAX: usize = 16;

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

/// Where the extended color goes: text, background or underline.
#[derive(Copy, Clone)]
enum Slot {
    Fg,
    Bg,
    Ul,
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
    /// `(shape 1..6, blink)` from DECSCUSR; None = from the config.
    pub app_cursor_style: Option<(u8, bool)>,
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
    /// Theme colors: the renderer uses them and `OSC 4/10/11/12` queries/changes them.
    pub theme: Theme,
    /// Someone touched the colors via OSC: the app has to resend them to the renderer.
    pub theme_dirty: bool,
    pub cell_size: (u32, u32),
    pub bells: u32,
    /// `OSC 9`/`777` notifications waiting for the app (`M-K`): the count is
    /// the app's cursor (as `bells`/`bells_seen`), the text is the last one.
    pub notifs: u32,
    pub notif_text: String,
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
    pub abs_gen: u64,
    pub hist_gen: u64,
    last_char: char,
    state: State,
    csi: Vec<u8>,
    osc: Vec<u8>,
    /// APC body (`ESC _`): this is how the kitty image arrives.
    apc: Vec<u8>,
    /// Image files waiting for the app (`t=f`/`t=t`/`t=s`, `B1`): the payload
    /// named them, the bytes have not arrived yet.
    pub image_files: Vec<crate::graphics::FileRequest>,
    /// The in-flight `Str` is an APC (not DCS/PM/SOS, which are discarded).
    apc_active: bool,
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
            app_cursor_style: None,
            mouse_mode: 0,
            mouse_sgr: false,
            mouse_utf8: false,
            mouse_px: false,
            focus_report: false,
            bracketed_paste: false,
            sync_output: false,
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
            theme: Theme::default(),
            theme_dirty: false,
            cell_size: (10, 19),
            bells: 0,
            notifs: 0,
            notif_text: String::new(),
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
            last_char: ' ',
            state: State::Ground,
            graphics: crate::graphics::Graphics::new(),
            csi: Vec::with_capacity(64),
            osc: Vec::with_capacity(128),
            apc: Vec::new(),
            image_files: Vec::new(),
            apc_active: false,
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
        *self = Term::new(cols, rows);
        self.scrollback_limit = limit;
        self.hist_gen = hist;
        self.abs_gen = abs;
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
        self.origin = false;
        self.autowrap = true;
        self.top = 0;
        self.bottom = self.rows - 1;
        self.wrap_pending = false;
    }

    /// How far the view must move so the reader does not lose the line they
    /// were on, after `prev_scrolled`/`prev_hist` (D10).
    ///
    /// `d` rows left through the top since the previous read: the content moved
    /// up `d`, so the view moves down `d`. But if the scrollback hit its cap,
    /// the oldest `dropped` lines died — and those do not count.
    #[must_use]
    pub fn view_after_output(&self, off: usize, prev_scrolled: u64, prev_hist: usize) -> usize {
        if off == 0 {
            return 0;
        }
        let d = self.scrolled.saturating_sub(prev_scrolled) as usize;
        if d == 0 {
            return off;
        }
        // Only the distance from the live area matters: a row that left the
        // scrollback (by cap or by `CSI 3J`) is already handled by the clamp in
        // `sb`. Subtracting the discarded ones made the view move the wrong way
        // for every row the grid dropped — at the cap, `off` froze and the
        // reader lost one row of content per new row (the opposite of D10).
        let _ = prev_hist;
        (off + d).min(self.scrollback.len())
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(1);
        let rows = rows.max(1);
        if (cols, rows) == (self.cols, self.rows) {
            return;
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
        let mut re = reflow(all, cols, hist + cur_row, cur_col);
        let first_live = re.rows.len().saturating_sub(rows);
        let live = re.rows.split_off(first_live);
        self.scrollback.extend(re.rows);
        while self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
        }
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

    fn put_char(&mut self, ch: char) {
        let w = ch.width().unwrap_or(1);
        if w == 0 {
            // combining: a cell does not compose. Ignoring it beats drawing a
            // box.
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
        match n {
            2 | 3 => self.touch_all(),
            1 => self.mark_rows(0, self.cy + 1),
            _ => self.mark_rows(self.cy, self.rows),
        }
        match n {
            2 | 3 => {
                let cell = self.blank_cell();
                for y in 0..self.rows {
                    let mut row = Row::blank(self.cols);
                    row.cells.fill(cell);
                    self.grid[y] = row;
                }
                if n == 3 {
                    // H13: `ED 3` clears the scrollback **and** the images —
                    // without this an image was left hanging on an empty screen
                    self.graphics.clear();
                    self.scrollback.clear();
                    self.abs_gen += 1;
                    self.hist_gen += 1;
                }
                self.cx = 0;
                self.cy = 0;
            },
            1 => {
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
        self.touch(self.cy);
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
                6 => self.origin = on,
                7 => self.autowrap = on,
                25 => self.cursor_visible = on,
                47 | 1047 | 1049 => {
                    if on {
                        self.enter_alt(p == 1049);
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
                2026 => self.sync_output = on,
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

    fn enter_alt(&mut self, clear_scrollback: bool) {
        if self.alt.is_some() {
            return;
        }
        self.mark_global("enter_alt");
        self.save_cursor();
        self.abs_gen += 1;
        let blank = (0..self.rows).map(|_| Row::blank(self.cols)).collect();
        self.alt = Some(std::mem::replace(&mut self.grid, blank));
        if clear_scrollback {
            self.scrollback.clear();
            self.hist_gen += 1;
        }
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
            b'A' => {
                let k = (p(0, 1).max(1)) as isize;
                let (cy, cx) = (self.cy as isize, self.cx as isize);
                self.cursor_to(cy - k, cx);
            },
            b'B' => {
                let k = (p(0, 1).max(1)) as isize;
                let (cy, cx) = (self.cy as isize, self.cx as isize);
                self.cursor_to(cy + k, cx);
            },
            b'C' => {
                let k = (p(0, 1).max(1)) as isize;
                let (cy, cx) = (self.cy as isize, self.cx as isize);
                self.cursor_to(cy, cx + k);
            },
            b'D' => {
                let k = (p(0, 1).max(1)) as isize;
                let (cy, cx) = (self.cy as isize, self.cx as isize);
                self.cursor_to(cy, cx - k);
            },
            b'E' => {
                let k = (p(0, 1).max(1)) as isize;
                let cy = self.cy as isize;
                self.cursor_to(cy + k, 0);
            },
            b'F' => {
                let k = (p(0, 1).max(1)) as isize;
                let cy = self.cy as isize;
                self.cursor_to(cy - k, 0);
            },
            b'G' | b'`' => {
                let k = p(0, 1).max(1);
                let cy = self.cy as isize;
                self.cursor_to(cy, k as isize - 1);
            },
            b'H' | b'f' => {
                let r = p(0, 1).max(1);
                let c = p(1, 1).max(1);
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
                    self.child_out.extend_from_slice(b"\x1b[?62c");
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
            // `9;4;state[;progress]` is not a message — it is the mintty/ConEmu
            // progress form (which some TUIs send as a keepalive while busy).
            // Swallowed, not toasted: a 1 Hz repeater would pin the toast and
            // the badge forever, and `4;3` is a state, not a sentence.
            9 if Self::osc_progress(arg) => {},
            9 => self.osc_notify(arg),
            777 => self.osc_notify777(arg),
            104 => self.osc_reset_palette(arg),
            110 => self.reset_color(10),
            111 => self.reset_color(11),
            112 => self.reset_color(12),
            _ => {},
        }
        self.osc_record(data);
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
        self.state = State::Ground;
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
    fn osc_progress(arg: &[u8]) -> bool {
        let mut parts = arg.split(|&b| b == b';');
        if parts.next() != Some(b"4".as_slice()) {
            return false;
        }
        let rest: Vec<&[u8]> = parts.collect();
        (1..=2).contains(&rest.len())
            && rest
                .iter()
                .all(|p| !p.is_empty() && p.iter().all(u8::is_ascii_digit))
    }

    /// A notification the child wants seen (`M-K`): the text lands in
    /// `notif_text` and the count moves, exactly like `bells` — the app
    /// decides toast, badge and urgency. Capped: a child that notifies in a
    /// loop must not grow this without a limit.
    fn osc_notify_text(&mut self, text: &str) {
        let clean: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(NOTIF_MAX)
            .collect();
        if clean.is_empty() {
            return;
        }
        self.notif_text = clean;
        self.notifs = self.notifs.wrapping_add(1);
    }

    /// `OSC 9;message` (iTerm2): the whole payload is the message.
    fn osc_notify(&mut self, arg: &[u8]) {
        let text = String::from_utf8_lossy(arg).into_owned();
        self.osc_notify_text(&text);
    }

    /// `OSC 777;notify;title;body` (urxvt): only `notify` is a notification,
    /// the other subcommands stay discarded with everything else unknown.
    fn osc_notify777(&mut self, arg: &[u8]) {
        let text = String::from_utf8_lossy(arg);
        let mut it = text.splitn(3, ';');
        if it.next() != Some("notify") {
            return;
        }
        let title = it.next().unwrap_or_default();
        let body = it.next().unwrap_or_default();
        if title.is_empty() {
            self.osc_notify_text(body);
        } else if !body.is_empty() {
            self.osc_notify_text(&format!("{title}: {body}"));
        } else {
            self.osc_notify_text(title);
        }
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
                            if self.osc.len() < MAX_OSC {
                                self.osc.push(b);
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
                    } else {
                        self.state = State::Str;
                    }
                },
            }
        }
    }

    /// One byte inside DCS/APC/PM/SOS: only the APC keeps content.
    fn str_byte(&mut self, b: u8) {
        let body: &[u8] = if self.apc_active { &self.apc } else { &[] };
        if b == 0x1b {
            self.state = State::StrEsc;
        } else if b == 0x9c && st_c1(body) {
            // 8-bit ST also closes DCS/APC/PM/SOS
            if self.apc.last() == Some(&0xc2) {
                self.apc.pop();
            }
            self.state = State::Ground;
            if std::mem::take(&mut self.apc_active) {
                self.apc_done();
            }
        } else if self.apc_active && self.apc.len() < crate::graphics::MAX_APC {
            // cap: a transmission that never closes must not grow without bound
            self.apc.push(b);
        }
    }

    /// A complete APC arrived: kitty images go through here (H1).
    fn apc_done(&mut self) {
        let body = std::mem::take(&mut self.apc);
        let cursor = (
            u16::try_from(self.cx).unwrap_or(0),
            u16::try_from(self.cy).unwrap_or(0),
        );
        let mut out = self.graphics.handle(&body, cursor, self.cell_size);
        // `t=f`/`t=t`/`t=s`: the payload named a file — it waits for the app,
        // which answers with `serve_images` (`B1`)
        if let Some(req) = out.file.take() {
            self.image_files.push(req);
        }
        self.apc_output(out);
    }

    /// Frames the responses and moves the cursor — the same tail for an APC
    /// that arrived and for a file the app loaded later (`B1`).
    fn apc_output(&mut self, out: crate::graphics::Output) {
        for r in out.responses {
            self.child_out.extend_from_slice(b"\x1b_");
            self.child_out.extend_from_slice(&r);
            self.child_out.extend_from_slice(b"\x1b\\");
        }
        // H10: without `C=1` the cursor goes down the rows the image took
        if out.move_cursor {
            for _ in 0..out.lines {
                self.newline();
            }
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
            },
            b'_' => {
                // APC: only this kind is content (kitty image); DCS/PM/SOS stay
                // discarded until ST
                self.apc.clear();
                self.apc_active = true;
                self.state = State::Str;
            },
            b'P' | b'^' | b'X' => {
                self.apc_active = false;
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
            _ => {
                // ESC = / ESC > (keypad), ESC \ (ST), and the rest: no effect
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
    rows: Vec<Row>,
    cur_row: usize,
    cur_col: usize,
    /// The cursor fell at the end of a full row: the next character wraps
    /// (the VT's `wrap_pending`), it does not overwrite the last column.
    wrap: bool,
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
fn reflow(all: Vec<Row>, cols: usize, cur_abs: usize, cur_col: usize) -> Reflowed {
    let cols = cols.max(1);
    // -- 1. paragraphs, in reading order
    let mut paragraphs: Vec<Vec<Cell>> = Vec::new();
    let mut cur_to = 0usize;
    let mut cur_off = 0usize;
    let mut to: Vec<Cell> = Vec::new();
    for (i, mut row) in all.into_iter().enumerate() {
        if i == cur_abs {
            cur_to = paragraphs.len();
            cur_off = to.len() + cur_col.min(row.cells.len().saturating_sub(1));
        }
        let soft = row.wrapped;
        if !soft {
            while row.cells.last().is_some_and(Cell::is_blank) {
                row.cells.pop();
            }
        }
        to.append(&mut row.cells);
        if !soft {
            paragraphs.push(std::mem::take(&mut to));
        }
    }
    if !to.is_empty() || paragraphs.is_empty() {
        paragraphs.push(to);
    }

    // -- 2. rewrap
    let mut rows: Vec<Row> = Vec::new();
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
            rows.push(Row {
                cells: chunk,
                wrapped: !last,
            });
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
mod tests {
    use super::*;

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
    fn osc9_progress_state_is_not_a_notification() {
        // the mintty/ConEmu progress form (a TUI keepalive): swallowed
        let mut t = Term::new(20, 4);
        for body in ["\x1b]9;4;3\x07", "\x1b]9;4;0\x07", "\x1b]9;4;3;50\x1b\\"] {
            t.feed(body.as_bytes());
        }
        assert_eq!(t.notifs, 0, "progress never notifies");
        assert_eq!(t.notif_text, "");
        // but a message that merely starts with a digit still does
        t.feed(b"\x1b]9;4;3 done\x07");
        assert_eq!(t.notifs, 1);
        assert_eq!(t.notif_text, "4;3 done");
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
        assert_eq!(t.take_child_out(), b"\x1b[?62c".to_vec());
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
        let (sc, hist, off) = (t.scrolled, t.scrollback.len(), 3usize);
        let before = t.abs_row(t.scrolled as i64 - off as i64).map(Row::text);
        // 2 more rows arrive: it stays on the same absolute row
        t.feed(b"new1\r\nnew2\r\n");
        let off2 = t.view_after_output(off, sc, hist);
        let after = t.abs_row(t.scrolled as i64 - off2 as i64).map(Row::text);
        assert_eq!(before, after, "the view moved along with the output");
        // at the bottom (off=0) it stays at the bottom
        assert_eq!(t.view_after_output(0, sc, hist), 0);
    }

    #[test]
    fn view_discounts_what_fell_out_of_a_full_history() {
        let mut t = Term::new(20, 2);
        t.scrollback_limit = 3;
        for i in 0..5 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        let (sc, hist, off) = (t.scrolled, t.scrollback.len(), 2usize);
        // 5 rows at once: the scrollback (cap 3) drops the oldest
        for i in 5..10 {
            t.feed(format!("l{i}\r\n").as_bytes());
        }
        let off2 = t.view_after_output(off, sc, hist);
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
        for y in 0..6 {
            assert!(
                !t.grid[y].cells[0].is_cont(),
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
        assert_eq!((t.notifs, t.notif_text.as_str()), (1, "build ok"));
        // empty and control-only payloads are not notifications
        t.feed(b"\x1b]9;\x07");
        t.feed(b"\x1b]9;\x01\x02\x07");
        assert_eq!((t.notifs, t.notif_text.as_str()), (1, "build ok"));
    }

    #[test]
    fn osc777_notify_joins_title_and_body() {
        let mut t = term();
        t.feed(b"\x1b]777;notify;make;all done\x07");
        assert_eq!((t.notifs, t.notif_text.as_str()), (1, "make: all done"));
        t.feed(b"\x1b]777;notify;;body only\x07");
        assert_eq!(t.notif_text.as_str(), "body only");
    }

    #[test]
    fn osc777_ignores_anything_but_notify() {
        let mut t = term();
        t.feed(b"\x1b]777;foo;bar\x07");
        t.feed(b"\x1b]777\x07");
        assert_eq!((t.notifs, t.notif_text.as_str()), (0, ""));
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
