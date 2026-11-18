//! The chrome as one scene (`M-Q`).
//!
//! Every overlay element — a tab of the bar, a workspace card, the toast — is
//! laid out **once**, here, in the renderer's origin space (`(0, 0)` is the first
//! grid cell). `render.rs` draws what this file answers and `app.rs` hit-tests
//! against the same numbers; before, each element was a drawing formula in one
//! file and a click formula in the other, and the two disagreed three times in a
//! single session (the wrong tab, the card starting on the wrong line, the double
//! click on line 1 becoming a tab click).
//!
//! What the layout owns: the label text and width of a tab, the height of a card
//! and how many fit, the clip at the bottom of the column. What it does not own:
//! colours, fonts and the content — it is geometry only, so it is testable
//! without a display.

/// Height of a workspace card, in rows of text (`B42`/`B43`).
pub const CARD_ROWS: f32 = 2.4;

/// Width of the scrollbar strip, in px. It sits over the grid's last column, so
/// the cell width is its ceiling: a wider strip would hide the column it
/// overlaps.
pub const SCROLLBAR_W: f32 = 8.0;

/// The thinnest the scrollbar's thumb may get, in px: below this it stops
/// reading as something one can grab.
const SCROLLBAR_MIN_THUMB: f32 = 12.0;

/// Gutter on both sides of the toast's text, in cells, so the box lines up with
/// the grid instead of with the glyphs.
const TOAST_PAD_X: f32 = 1.5;
/// The toast's vertical respiro, in px: independent of the line height of the
/// theme.
const TOAST_PAD_Y: f32 = 8.0;
/// Room left clear on each side of the toast, in cells.
const TOAST_MARGIN_X: f32 = 1.0;
/// Room left under the toast, in px.
const TOAST_MARGIN_Y: f32 = 28.0;
/// The accent strip on the toast's left edge: wide enough to read as a strip and
/// not as a crack in the border.
pub const TOAST_BAR: f32 = 4.0;
/// Detail lines beyond this are cut: a notification carrying a whole page must
/// not cover the grid it is telling you about.
const TOAST_DETAIL_LINES: usize = 4;

/// Everything that decides where the chrome lands.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Metrics {
    pub cw: f32,
    pub ch: f32,
    pub pad: f32,
    pub bar: f32,
    pub side: f32,
    pub cols: usize,
    pub rows: usize,
}

/// One tab, as the bar needs it: the title, whether it wears the bell, and the
/// `OSC 7501` state word when `status_show tab` puts it on the label (`B47`).
pub struct Tab<'a> {
    pub title: &'a str,
    pub alert: bool,
    pub state: Option<&'a str>,
}

/// A rectangle in origin space: `x, y, w, h`.
pub type Rect = [f32; 4];

/// Where an element landed, and which one it is (its index in the list it came
/// from: tabs, workspaces).
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Slot {
    pub i: usize,
    pub rect: Rect,
}

/// The chrome of one frame.
pub struct Layout {
    pub m: Metrics,
    pub tabs: Vec<Slot>,
    pub cards: Vec<Slot>,
}

/// The label of a tab: the number and the title, one cell of gutter each side —
/// and, when `status_show tab` is on, the state in front of the number, where the
/// bell's two cells also live: drawing and hit-test both read this, so a wider
/// label cannot disagree with the click (`M-Q`).
#[must_use]
pub fn tab_label(i: usize, t: &Tab<'_>) -> String {
    match t.state {
        Some(state) => format!(" ● {state} {} {} ", i + 1, t.title),
        None => format!(" {} {} ", i + 1, t.title),
    }
}

/// How many cells a tab takes: the label, plus the two cells the bell reserves
/// in front of it (`B8`).
#[must_use]
pub fn tab_cells(i: usize, t: &Tab<'_>) -> usize {
    tab_label(i, t).chars().count() + if t.alert { 2 } else { 0 }
}

/// The `status_show chip` box (`B47`): what it says, and where it lands.
///
/// It is the one surface that shows with a **single** tab — the bar is not drawn
/// then — and the one the user clicks away, because `done` and `error` have no
/// lifetime of their own. Drawing and the press read this same box, so a chip
/// drawn one cell off cannot become a chip that is dismissed somewhere else
/// (`M-Q`).
pub struct StateChip {
    pub word: String,
    pub detail: String,
}

impl StateChip {
    /// The cells the box takes: `[ ` + word + detail + ` ]`, plus the clear cell
    /// that keeps it off the window edge.
    #[must_use]
    pub fn cells(&self) -> usize {
        self.word.chars().count() + self.detail.chars().count() + 5
    }

    /// Where the box lands: the grid's last row, right-aligned, one cell clear of
    /// the edge. `None` when it does not fit the grid's width.
    #[must_use]
    pub fn rect(&self, m: Metrics) -> Option<Rect> {
        let cells = self.cells();
        if cells > m.cols || m.rows == 0 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)] // a grid size, far below 2^23
        let x = (m.cols - cells) as f32 * m.cw;
        #[allow(clippy::cast_precision_loss)]
        let y = (m.rows - 1) as f32 * m.ch;
        #[allow(clippy::cast_precision_loss)]
        let w = (cells - 1) as f32 * m.cw;
        Some([x, y, w, m.ch])
    }

    /// Is the pointer on the box? Window px, tested against the numbers the
    /// drawing used — the same rule as a tab and a card.
    #[must_use]
    pub fn hit(&self, m: Metrics, px: f64, py: f64) -> bool {
        let Some([left, top, width, height]) = self.rect(m) else {
            return false;
        };
        let (x, y) = m.in_origin(px, py);
        x >= left && x < left + width && y >= top && y < top + height
    }
}

/// The scrollbar (`M-M`): the strip on the grid's right edge, the thumb that
/// says where in the history the view is, and the press that moves it.
///
/// All three inputs are the ones the frame shows — how much is behind the view,
/// how tall the grid is, and where the view sits — so both sides build the same
/// value (`M-Q`). Before, the strip's `x` and the thumb's height were written
/// twice, once in window px for the click and once in origin space for the
/// painter, and the two agreed only by arithmetic.
pub struct Scrollbar {
    /// Lines of history behind the view.
    hist: usize,
    /// Rows of the grid, which is the track's height in cells.
    rows: usize,
    /// `0` follows the output; `hist` is the oldest line.
    at: usize,
}

impl Scrollbar {
    /// `None` when there is nothing to scroll — the same condition that leaves
    /// the strip undrawn, so a click cannot land on a bar that is not there.
    #[must_use]
    pub fn new(hist: usize, rows: usize, at: usize) -> Option<Scrollbar> {
        (hist > 0 && rows > 0).then_some(Scrollbar {
            hist,
            rows,
            at: at.min(hist),
        })
    }

    /// Width of the strip in px, capped by the cell.
    fn width(m: Metrics) -> f32 {
        SCROLLBAR_W.min(m.cw)
    }

    /// The track and the thumb, in origin space: the background the strip is
    /// painted on, and the solid part that marks the view.
    #[must_use]
    pub fn parts(&self, m: Metrics) -> (Rect, Rect) {
        let strip = Self::width(m);
        #[allow(clippy::cast_precision_loss)] // grid sizes and line counts
        let (rows, hist) = (self.rows as f32, self.hist as f32);
        #[allow(clippy::cast_precision_loss)]
        let x = m.cols as f32 * m.cw - strip;
        let track_h = rows * m.ch;
        let thumb_h = (track_h * rows / (rows + hist))
            .max(SCROLLBAR_MIN_THUMB)
            .min(track_h);
        #[allow(clippy::cast_precision_loss)]
        let off = self.at as f32 / hist;
        let y = (track_h - thumb_h) * (1.0 - off);
        ([x, 0.0, strip, track_h], [x, y, strip, thumb_h])
    }

    /// The pixel target of a press, in window px: the strip **plus the sliver
    /// between the grid's right edge and the window's**. `cols * cw` is rounded
    /// down, so the last column does not reach the border and a press there would
    /// fall through to the text and start a selection. `window_w` is the window's
    /// width in px.
    #[must_use]
    pub fn hit(&self, m: Metrics, window_w: f64, px: f64, py: f64) -> bool {
        let strip = Self::width(m);
        #[allow(clippy::cast_precision_loss)]
        let grid_right = m.pad + m.side + m.cols as f32 * m.cw;
        #[allow(clippy::cast_precision_loss)]
        let right = window_w as f32;
        let left = f64::from((grid_right - strip).min(right - strip).max(0.0));
        let top = f64::from(m.pad + m.bar);
        #[allow(clippy::cast_precision_loss)]
        let height = self.rows as f64 * f64::from(m.ch);
        px >= left && px < window_w && py >= top && py < top + height
    }

    /// The view offset a press at `py` asks for: the thumb follows the pointer,
    /// so a press on its middle does not jump. `None` when the thumb fills the
    /// track and there is nowhere to go.
    #[must_use]
    pub fn offset_at(&self, m: Metrics, py: f64) -> Option<i64> {
        let (track, thumb) = self.parts(m);
        let (height, thumb_h) = (f64::from(track[3]), f64::from(thumb[3]));
        let travel = height - thumb_h;
        if travel <= 0.0 {
            return None;
        }
        let top = f64::from(m.pad + m.bar + track[1]);
        let y = ((py - top) - thumb_h / 2.0).clamp(0.0, travel);
        #[allow(clippy::cast_precision_loss)]
        let hist = self.hist as f64;
        Some(((1.0 - y / travel) * hist).round() as i64)
    }
}

/// The toast box (`I12`/`K13`): the notice's text wrapped to the grid, and the
/// box it lands in.
///
/// It floats over everything and is drawn last, so a press on it is the box's and
/// never the grid's. The wrapping lives here and not in the painter because the
/// box's width **is** the wrapped text: a painter that wrapped on its own would
/// size the box from one measure and fill it from another.
pub struct Toast {
    pub rect: Rect,
    pub title: String,
    pub detail: Vec<String>,
}

impl Toast {
    /// Wrap the notice to the grid and place the box. `None` when both lines are
    /// empty: an empty box is a hole in the screen, not a notice.
    #[must_use]
    pub fn build(m: Metrics, title: &str, detail: &str) -> Option<Toast> {
        // title and detail wrap at the same column, so the box has straight sides
        #[allow(clippy::cast_precision_loss)] // a grid width, far below 2^23
        let n = (m.cols as f32 - 2.0 * TOAST_MARGIN_X - 2.0 * TOAST_PAD_X)
            .floor()
            .max(1.0) as usize;
        let title: String = title.chars().take(n).collect();
        let room = m.rows.saturating_sub(2).max(1);
        let detail = wrap(detail, n, TOAST_DETAIL_LINES.min(room));
        let widest = detail
            .iter()
            .map(|l| l.chars().count())
            .chain(std::iter::once(title.chars().count()))
            .max()
            .unwrap_or(0);
        if widest == 0 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)] // counts of cells and lines
        let (widest, lines) = (widest as f32, (1 + detail.len()) as f32);
        #[allow(clippy::cast_precision_loss)]
        let (cols, rows) = (m.cols as f32, m.rows as f32);
        let bw = widest * m.cw + 2.0 * TOAST_PAD_X * m.cw + TOAST_BAR;
        let bh = lines * m.ch + 2.0 * TOAST_PAD_Y;
        // centred: a notice is read where the eye lands, not in a corner. The text
        // was cut at `n` above, so the box always fits the grid.
        let x = ((cols * m.cw - bw) / 2.0).max(0.0);
        let y = (rows * m.ch - TOAST_MARGIN_Y - bh).max(0.0);
        Some(Toast {
            rect: [x, y, bw, bh],
            title,
            detail,
        })
    }

    /// Is the pointer on the box? Window px, tested against the numbers the
    /// drawing used — the same rule as a tab, a card and the chip.
    #[must_use]
    pub fn hit(&self, m: Metrics, px: f64, py: f64) -> bool {
        let [left, top, width, height] = self.rect;
        let (x, y) = m.in_origin(px, py);
        x >= left && x < left + width && y >= top && y < top + height
    }

    /// Where the title starts, in origin space: inside the padding and right of
    /// the accent strip.
    #[must_use]
    pub fn title_at(&self, m: Metrics) -> (f32, f32) {
        (
            self.rect[0] + TOAST_BAR + TOAST_PAD_X * m.cw,
            self.rect[1] + TOAST_PAD_Y,
        )
    }

    /// The top of detail line `i`, in origin space.
    #[must_use]
    pub fn line_at(&self, m: Metrics, i: usize) -> (f32, f32) {
        #[allow(clippy::cast_precision_loss)] // a line index
        let y = self.rect[1] + TOAST_PAD_Y + (i + 1) as f32 * m.ch;
        (self.title_at(m).0, y)
    }
}

/// Greedy word wrap at `n` cells, at most `max` lines. A word longer than the
/// line is cut, because a box that grew to fit it would leave the grid.
fn wrap(text: &str, n: usize, max: usize) -> Vec<String> {
    let n = n.max(1);
    let mut words: Vec<&str> = Vec::new();
    for word in text.split_whitespace() {
        let mut rest = word;
        while rest.chars().count() > n {
            let cut = rest.char_indices().nth(n).map_or(rest.len(), |(i, _)| i);
            words.push(&rest[..cut]);
            rest = &rest[cut..];
        }
        if !rest.is_empty() {
            words.push(rest);
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in words {
        let sep = usize::from(!line.is_empty());
        if !line.is_empty() && line.chars().count() + sep + word.chars().count() > n {
            out.push(std::mem::take(&mut line));
            if out.len() == max {
                return out;
            }
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() && out.len() < max {
        out.push(line);
    }
    out
}

/// What the pointer is standing on, as the chrome sees it.
///
/// The order of the variants is the order the drawing puts things on screen, from
/// the last painted to the first, and `Scene::hit` answers in it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Hit {
    /// A usable row of the open context menu.
    MenuRow(usize),
    /// The menu is open and the pointer is not on a row: the press closes it.
    Menu,
    /// The notice box (`I12`/`K13`).
    Toast,
    /// The `status_show chip` box (`B47`).
    Chip,
    /// A workspace card (`I9`).
    Card(usize),
    /// A tab of the bar (`I9`).
    Tab(usize),
    /// The bar's own strip, right of the last tab — and, with a single tab there
    /// is no bar, the margin above the grid. It holds no content: a right press
    /// there only has the bar's menu (`M-V`).
    Bar,
    /// The scrollbar (`M-M`), over the grid's last column.
    Scrollbar,
    /// Nothing of the chrome: the grid, and the child behind it.
    Grid,
}

/// The chrome of one frame, as the pointer sees it.
///
/// One value per press, built on demand from the same functions that draw: it
/// cannot go stale, and there is no second ordering to keep in step with the
/// drawing.
pub struct Scene<'a> {
    pub metrics: Metrics,
    /// Window width in px: the scrollbar's target reaches its right edge.
    pub window_w: f64,
    /// Pointer, in window px.
    pub at: (f64, f64),
    pub layout: &'a Layout,
    pub menu: Option<&'a Menu>,
    pub toast: Option<&'a Toast>,
    pub chip: Option<&'a StateChip>,
    pub scrollbar: Option<&'a Scrollbar>,
}

impl Scene<'_> {
    /// What claims the press at `at`.
    ///
    /// The order is the drawing's, backwards: `render::Renderer::build` paints
    /// the bar and the cards, then the scrollbar, then the bottom row (the search
    /// or the chip), then the toast, and last the menu — so a press is offered to
    /// them in exactly that reverse order. The one on top wins and the one under
    /// it does not hear about the click: before this, the chain in `app.rs` was a
    /// second ordering written by hand, and a menu drawn over a toast was pressed
    /// after it — the click dismissed the notice instead of choosing the row.
    #[must_use]
    pub fn hit(&self) -> Hit {
        let (x, y) = self.at;
        if let Some(m) = self.menu {
            return m.row_at(self.metrics, x, y).map_or(Hit::Menu, Hit::MenuRow);
        }
        if self.toast.is_some_and(|t| t.hit(self.metrics, x, y)) {
            return Hit::Toast;
        }
        if self.chip.is_some_and(|c| c.hit(self.metrics, x, y)) {
            return Hit::Chip;
        }
        if let Some(i) = self.layout.card_at(x, y) {
            return Hit::Card(i);
        }
        if let Some(i) = self.layout.tab_at(x, y) {
            return Hit::Tab(i);
        }
        if y < f64::from(self.metrics.pad + self.metrics.bar) {
            return Hit::Bar;
        }
        if self
            .scrollbar
            .is_some_and(|s| s.hit(self.metrics, self.window_w, x, y))
        {
            return Hit::Scrollbar;
        }
        Hit::Grid
    }
}

impl Metrics {
    #[must_use]
    pub fn card_h(&self) -> f32 {
        self.ch * CARD_ROWS
    }

    /// Bottom of the card column, in origin space: the grid's last row (plus the
    /// bottom padding, where a card may end — it is not text).
    #[must_use]
    pub fn card_bottom(&self) -> f32 {
        #[allow(clippy::cast_precision_loss)] // a grid row count, far below 2^23
        let rows = self.rows as f32;
        rows * self.ch + self.pad
    }

    /// Window px → origin space, where `(0, 0)` is the grid's first cell.
    #[must_use]
    pub fn in_origin(&self, px: f64, py: f64) -> (f32, f32) {
        #[allow(clippy::cast_possible_truncation)] // a window coordinate
        let (x, y) = (px as f32, py as f32);
        (x - self.pad - self.side, y - self.pad - self.bar)
    }

    /// The grid cell under the pointer (0, 0 when it is left of/above the grid).
    #[must_use]
    pub fn cell_at(&self, px: f64, py: f64) -> (usize, usize) {
        let (x, y) = self.in_origin(px, py);
        let col = if self.cw > 0.0 {
            (x / self.cw).floor().max(0.0)
        } else {
            0.0
        };
        let row = if self.ch > 0.0 {
            (y / self.ch).floor().max(0.0)
        } else {
            0.0
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cells = (col as usize, row as usize);
        cells
    }

    /// How many **whole** cards fit in the column, at least one, leaving the last
    /// text row free (`B55`: that row is the drag's walk strip, and a card cut by
    /// the bottom edge would be a place and a scroll trigger at once).
    #[must_use]
    pub fn cards_visible(&self) -> usize {
        let card_h = self.card_h();
        if card_h <= 0.0 {
            return 1;
        }
        #[allow(clippy::cast_precision_loss)]
        let rows = self.rows as f32;
        let band = self.bar + rows * self.ch - self.ch;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // the band is positive and bounded by the window height
        let n = (band / card_h) as usize;
        n.max(1)
    }
}

impl Layout {
    /// Lays out the bar, the cards and the toast. `ws` is the workspace count and
    /// `card_off` the first card in sight (`B42`).
    #[must_use]
    pub fn build(m: Metrics, tabs: &[Tab<'_>], ws: usize, card_off: usize) -> Layout {
        let mut slots = Vec::with_capacity(tabs.len());
        #[allow(clippy::cast_precision_loss)] // a column count, far below 2^23
        let total = m.cols as f32 * m.cw;
        let mut x = 0.0;
        for (i, t) in tabs.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let w = (tab_cells(i, t) as f32 * m.cw).min(total - x);
            if w <= 0.0 {
                break;
            }
            slots.push(Slot {
                i,
                rect: [x, -m.bar, w, m.ch],
            });
            x += w;
        }
        let card_h = m.card_h();
        let bottom = m.card_bottom();
        let visible = m.cards_visible();
        let mut cards = Vec::with_capacity(visible);
        for i in card_off..ws {
            #[allow(clippy::cast_precision_loss)]
            let y0 = -m.bar + (i - card_off) as f32 * card_h;
            if y0 >= bottom || cards.len() >= visible {
                break;
            }
            cards.push(Slot {
                i,
                rect: [-m.side, y0, m.side - m.pad, (y0 + card_h).min(bottom) - y0],
            });
        }
        Layout {
            m,
            tabs: slots,
            cards,
        }
    }

    /// Window px → origin space: the grid's `(0, 0)` is not the window's.
    fn origin(&self, px: f64, py: f64) -> (f32, f32) {
        self.m.in_origin(px, py)
    }

    /// The tab under the pointer, if any.
    #[must_use]
    pub fn tab_at(&self, px: f64, py: f64) -> Option<usize> {
        let (x, y) = self.origin(px, py);
        self.tabs
            .iter()
            .find(|s| {
                let [x0, y0, w, h] = s.rect;
                x >= x0 && x < x0 + w && y >= y0 && y < y0 + h
            })
            .map(|s| s.i)
    }

    /// The workspace card under the pointer, if any. The walk strip below the
    /// last card belongs to no card (`B55`) — the layout simply has no slot there.
    #[must_use]
    pub fn card_at(&self, px: f64, py: f64) -> Option<usize> {
        let (x, y) = self.origin(px, py);
        self.cards
            .iter()
            .find(|s| {
                let [x0, y0, w, h] = s.rect;
                x >= x0 && x < x0 + w && y >= y0 && y < y0 + h
            })
            .map(|s| s.i)
    }

    /// Where tab `i` sits and how wide it is, for the drag (`I10`).
    #[must_use]
    pub fn tab_slot(&self, i: usize) -> Option<[f32; 2]> {
        self.tabs
            .iter()
            .find(|s| s.i == i)
            .map(|s| [s.rect[0] + self.m.pad + self.m.side, s.rect[2]])
    }
}

/// One row a context menu can offer. The label lives here because the geometry
/// needs its width; what the row *does* is the app's (`MenuState`).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Item {
    Copy,
    Paste,
    OpenLink,
    NewTab,
    CloseTab,
    NewWorkspace,
    Rename,
    CloseWorkspace,
    MoveUp,
    MoveDown,
}

impl Item {
    /// The text of the row, which is also its width in cells.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Item::Copy => "Copy",
            Item::Paste => "Paste",
            Item::OpenLink => "Open link",
            Item::NewTab => "New tab",
            Item::CloseTab => "Close tab",
            Item::NewWorkspace => "New workspace",
            Item::Rename => "Rename",
            Item::CloseWorkspace => "Close workspace",
            Item::MoveUp => "Move up",
            Item::MoveDown => "Move down",
        }
    }
}

/// A context menu is a context menu: one box, drawn and hit the same way, and
/// told apart only by **which** rows it lists and **what** it hangs from. Every
/// menu the app has is one of these arrays, so adding a row is a variant of
/// [`Item`] plus a line here — never a second menu to draw.
///
/// The grid's (`M-V`), where the pointer is on a cell of the screen.
pub static GRID_MENU: [Item; 4] = [Item::Copy, Item::Paste, Item::OpenLink, Item::NewTab];
/// A workspace card's, hung from the pointer (the sidebar is not made of cells).
pub static CARD_MENU: [Item; 5] = [
    Item::NewWorkspace,
    Item::Rename,
    Item::MoveUp,
    Item::MoveDown,
    Item::CloseWorkspace,
];
/// A tab's.
pub static TAB_MENU: [Item; 2] = [Item::NewTab, Item::CloseTab];
/// The bar's empty strip, right of the last tab: the only thing it can do.
pub static BAR_MENU: [Item; 1] = [Item::NewTab];

/// What a menu hangs from.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Anchor {
    /// The cell under the pointer: the grid, where lining up with the text is
    /// what makes "Open link" mean the link it covers.
    Cell,
    /// The pointer itself: the sidebar and the bar, which are not made of cells.
    Pointer,
}

/// Cells of gutter on each side of a menu label.
const MENU_GUTTER: usize = 1;

/// The right-button menu of the grid (`M-V`): one row per item, hanging from the
/// pointer, in the same origin space as the rest of the chrome. Built on demand
/// (not kept between frames), so it can never disagree with the pointer.
pub struct Menu {
    /// The whole box, in origin space.
    pub rect: Rect,
    pub rows: Vec<Slot>,
    /// The rows it lists, in the order it lists them.
    pub items: &'static [Item],
}

impl Menu {
    /// `at` is the pointer in window px. The box hangs from [`Anchor`] — the cell
    /// under the pointer on the grid, the pointer itself on the sidebar and the
    /// bar — and is then kept inside the window, which is what takes the place of
    /// the flip on the bottom and the left edge: a menu with rows nobody can
    /// reach is worse than no menu.
    #[must_use]
    pub fn build(m: Metrics, items: &'static [Item], anchor: Anchor, at: (f64, f64)) -> Menu {
        let text = items
            .iter()
            .map(|i| i.label().chars().count())
            .max()
            .unwrap_or(0);
        let cells = (text + 2 * MENU_GUTTER).clamp(1, m.cols.max(1));
        let count = items.len().clamp(1, m.rows.max(1));
        #[allow(clippy::cast_precision_loss)]
        let (width, step) = (cells as f32 * m.cw, m.ch);
        #[allow(clippy::cast_precision_loss)]
        let height = step * count as f32;
        let (left, top) = match anchor {
            Anchor::Cell => {
                let (col, row) = m.cell_at(at.0, at.1);
                let col = col.min(m.cols.saturating_sub(cells));
                // a menu that would be cut by the bottom edge flips above the cell
                let row = if row + count > m.rows {
                    row.saturating_sub(count)
                } else {
                    row
                };
                #[allow(clippy::cast_precision_loss)] // cells, far below 2^23
                (col as f32 * m.cw, row as f32 * m.ch)
            },
            Anchor::Pointer => m.in_origin(at.0, at.1),
        };
        // the window in origin space: the sidebar and the bar are negative, and a
        // menu opened over them must still fit the screen it is drawn on
        #[allow(clippy::cast_precision_loss)]
        let (min_x, min_y) = (-(m.pad + m.side), -(m.pad + m.bar));
        #[allow(clippy::cast_precision_loss)]
        let (max_x, max_y) = (m.cols as f32 * m.cw - width, m.rows as f32 * m.ch - height);
        // `min(min, max)`: in a window too small for the box, the top/left wins
        let left = left.clamp(min_x.min(max_x), max_x);
        let top = top.clamp(min_y.min(max_y), max_y);
        let mut rows = Vec::with_capacity(count);
        for i in 0..count {
            #[allow(clippy::cast_precision_loss)]
            let y = top + i as f32 * step;
            rows.push(Slot {
                i,
                rect: [left, y, width, step],
            });
        }
        Menu {
            rect: [left, top, width, height],
            rows,
            items,
        }
    }

    /// The row under the pointer. `m` is needed for the origin shift, which the
    /// menu shares with every other overlay.
    #[must_use]
    pub fn row_at(&self, m: Metrics, px: f64, py: f64) -> Option<usize> {
        let (x, y) = m.in_origin(px, py);
        self.rows
            .iter()
            .find(|slot| {
                let [x0, y0, width, height] = slot.rect;
                x >= x0 && x < x0 + width && y >= y0 && y < y0 + height
            })
            .map(|slot| slot.i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `status_show tab`: the state word rides in the label, and the width follows
    /// it — the bar is laid out from `tab_cells` and hit-tested from the same
    /// rect, so a label that grew without the count would steal its neighbour's
    /// clicks (`M-Q`).
    #[test]
    fn a_tab_label_can_carry_the_state() {
        let plain = Tab {
            title: "zsh",
            alert: false,
            state: None,
        };
        let marked = Tab {
            title: "zsh",
            alert: false,
            state: Some("blocked"),
        };
        assert_eq!(tab_label(1, &plain), " 2 zsh ");
        assert_eq!(tab_label(1, &marked), " ● blocked 2 zsh ");
        assert_eq!(
            tab_cells(1, &marked) - tab_cells(1, &plain),
            "● blocked ".chars().count(),
            "the word costs what it occupies"
        );
        // the bell's two cells still add up on top of a label that already grew
        let quiet = Tab {
            title: "zsh",
            alert: false,
            state: Some("error"),
        };
        let both = Tab {
            title: "zsh",
            alert: true,
            state: Some("error"),
        };
        assert_eq!(tab_cells(1, &both), tab_cells(1, &quiet) + 2);
    }

    /// The chip is right-aligned on the grid's last row, one cell clear of the
    /// edge — and the box that is drawn is the box that is pressed (`M-Q`). A
    /// click a pixel left of it, or one row below, has to miss: the chip sits on
    /// the grid, and the child's own clicks are behind it.
    #[test]
    fn the_chip_box_is_where_the_chip_is_drawn() {
        let m = Metrics {
            cw: 10.0,
            ch: 20.0,
            pad: 4.0,
            bar: 30.0,
            side: 180.0,
            cols: 40,
            rows: 5,
        };
        let chip = StateChip {
            word: "blocked".to_string(),
            detail: " · git · 40%".to_string(),
        };
        // `[ ` + word + detail + ` ]`, plus the clear cell — counted in cells,
        // not bytes: the `·` is two of them
        let expect = 2 + "blocked".chars().count() + " · git · 40%".chars().count() + 2 + 1;
        assert_eq!(chip.cells(), expect);
        let [left, top, width, height] = chip.rect(m).expect("it fits");
        assert_eq!(top, 4.0 * m.ch, "the grid's last row");
        assert_eq!(height, m.ch, "one row tall");
        #[allow(clippy::cast_precision_loss)]
        let edge = (m.cols - 1) as f32 * m.cw;
        assert_eq!(left + width, edge, "one cell clear of the window edge");
        // window px: the grid starts below the bar and right of the sidebar
        let px = |origin: f32| f64::from(origin + m.pad + m.side);
        let py = |origin: f32| f64::from(origin + m.pad + m.bar);
        assert!(chip.hit(m, px(left) + 1.0, py(top) + 1.0), "inside the box");
        assert!(
            !chip.hit(m, px(left) - 1.0, py(top) + 1.0),
            "a pixel to the left"
        );
        assert!(
            !chip.hit(m, px(left) + 1.0, py(top + height)),
            "the row below"
        );
        // a grid narrower than the box: nothing drawn, so nothing to press
        let narrow = Metrics { cols: 5, ..m };
        assert_eq!(chip.rect(narrow), None);
        assert!(!chip.hit(narrow, px(0.0), py(top) + 1.0));
    }

    /// The grid's menu hangs from the pointer's cell and never leaves the grid: at
    /// the bottom it flips up, and its rows are the ones the pointer can hit.
    #[test]
    fn the_menu_hangs_from_the_pointer_and_stays_inside() {
        let m = metrics();
        // pad 4 + side 180 = 184: the grid starts there, and the bar adds 20
        let menu = Menu::build(m, &GRID_MENU, Anchor::Cell, (184.0 + 15.0, 24.0 + 15.0));
        assert_eq!(menu.rows.len(), GRID_MENU.len());
        assert_eq!(menu.rect[0], 10.0, "the pointer's cell, in origin space");
        assert_eq!(menu.rect[1], 0.0);
        // the widest label plus a cell of gutter on each side: "Open link" = 9
        assert_eq!(menu.rect[2], 11.0 * 10.0);
        // a click inside a row names it, one pixel below the box names nothing
        let y = 24.0 + 20.0 * 1.5;
        assert_eq!(menu.row_at(m, 184.0 + 20.0, y), Some(1));
        assert_eq!(menu.row_at(m, 184.0 + 20.0, 24.0 + 20.0 * 4.5), None);
        // near the bottom (row 9 of 10) it flips up instead of running off
        let low = Menu::build(m, &GRID_MENU, Anchor::Cell, (184.0, 24.0 + 9.5 * 20.0));
        assert_eq!(low.rect[1], 5.0 * 20.0);
        assert!(low.rect[1] + low.rect[3] <= 10.0 * 20.0);
    }

    /// A card's menu hangs from the **pointer**, not from a cell: the pointer's
    /// pixel is the box's corner, the sidebar being left of the grid (negative
    /// origin), and it still fits the window (`B64`, the reported bug: a click on
    /// the sidebar used to put the grid's menu at the grid's edge).
    #[test]
    fn a_card_menu_hangs_from_the_pointer_and_fits_the_window() {
        let m = metrics();
        // "Close workspace" is 15 cells + 2 of gutter = 17
        let menu = Menu::build(m, &CARD_MENU, Anchor::Pointer, (20.0, 24.0 + 40.0));
        assert_eq!(menu.rows.len(), CARD_MENU.len());
        assert_eq!(menu.rect[0], 20.0 - 184.0, "the pointer, over the sidebar");
        assert_eq!(menu.rect[1], 40.0);
        assert_eq!(menu.rect[2], 17.0 * 10.0);
        // the window in origin space runs from -(pad+side) to cols*cw: the box
        // never leaves it, neither at the right edge nor in the left margin
        let right = Menu::build(m, &CARD_MENU, Anchor::Pointer, (570.0, 100.0));
        assert!(
            right.rect[0] + right.rect[2] <= 40.0 * 10.0,
            "it never spills out on the right"
        );
        let margin = Menu::build(m, &CARD_MENU, Anchor::Pointer, (0.0, 100.0));
        assert_eq!(
            margin.rect[0], -184.0,
            "nor past the left edge of the window"
        );
        // a click in its first row names row 0, the one under the pointer
        let px = 20.0;
        assert_eq!(menu.row_at(m, px, 24.0 + 40.0 + 5.0), Some(0));
        // at the bottom of the window it is pulled up, not cut
        let low = Menu::build(m, &CARD_MENU, Anchor::Pointer, (20.0, 24.0 + 199.0));
        assert!(low.rect[1] + low.rect[3] <= 10.0 * 20.0);
        // and the bar's own strip, above the grid, stays inside the window too
        let bar = Menu::build(m, &BAR_MENU, Anchor::Pointer, (500.0, 6.0));
        assert_eq!(bar.rect[1], 6.0 - 24.0, "the pointer's pixel, in origin");
        assert!(bar.rect[1] >= -24.0, "above the bar is off-window");
    }

    fn metrics() -> Metrics {
        Metrics {
            cw: 10.0,
            ch: 20.0,
            pad: 4.0,
            bar: 20.0,
            side: 180.0,
            cols: 40,
            rows: 10,
        }
    }

    fn tabs() -> [Tab<'static>; 3] {
        [
            Tab {
                title: "one",
                alert: false,
                state: None,
            },
            Tab {
                title: "two",
                alert: true,
                state: None,
            },
            Tab {
                title: "three",
                alert: false,
                state: None,
            },
        ]
    }

    #[test]
    fn a_tab_with_the_bell_reserves_two_more_cells() {
        let t = tabs();
        assert_eq!(tab_cells(0, &t[0]), " 1 one ".chars().count());
        assert_eq!(tab_cells(1, &t[1]), " 2 two ".chars().count() + 2);
        assert_eq!(tab_label(2, &t[2]), " 3 three ");
    }

    #[test]
    fn the_tabs_start_after_the_sidebar_and_the_bar_row() {
        let m = metrics();
        let l = Layout::build(m, &tabs(), 1, 0);
        assert_eq!(l.tabs.len(), 3);
        // origin space: x = 0 is the first grid column, y = -bar the bar's row
        assert_eq!((l.tabs[0].rect[0], l.tabs[0].rect[1]), (0.0, -20.0));
        assert_eq!(l.tabs[0].rect[2], 70.0); // " 1 one " = 7 cells
        assert_eq!(l.tabs[1].rect[0], 70.0);
        assert_eq!(l.tabs[1].rect[2], 90.0); // " 2 two " + 2 cells of bell
    }

    #[test]
    fn the_pointer_finds_the_tab_it_is_over() {
        let l = Layout::build(metrics(), &tabs(), 1, 0);
        // window px: pad 4 + side 180 + 5 px into the first tab
        assert_eq!(l.tab_at(4.0 + 180.0 + 5.0, 4.0 + 10.0), Some(0));
        // 5 px into the second one
        assert_eq!(l.tab_at(4.0 + 180.0 + 75.0, 4.0 + 10.0), Some(1));
        // the bar is one row high: below it there is no tab
        assert_eq!(l.tab_at(4.0 + 180.0 + 5.0, 4.0 + 30.0), None);
        // and left of the grid's origin there is no tab either
        assert_eq!(l.tab_at(2.0, 4.0 + 10.0), None);
    }

    #[test]
    fn the_cards_leave_the_last_row_for_the_walk() {
        let m = metrics();
        // band = bar 20 + 10 rows * 20 - 20 = 200; card = 48 → 4 whole cards
        assert_eq!(m.cards_visible(), 4);
        let l = Layout::build(m, &tabs(), 9, 0);
        assert_eq!(l.cards.len(), 4);
        assert_eq!(l.cards[0].rect[1], -20.0);
        assert_eq!(l.cards[0].rect[3], 48.0);
        // the strip below the last card names no workspace (`B55`)
        let strip_y = 4.0 + 20.0 + 4.0 * 48.0 + 10.0;
        assert_eq!(l.card_at(4.0 + 10.0, strip_y), None);
        // and the second card is the second workspace
        assert_eq!(l.card_at(4.0 + 10.0, 4.0 + 20.0 + 48.0 + 10.0), Some(1));
    }

    #[test]
    fn scrolled_the_column_starts_at_the_offset() {
        let l = Layout::build(metrics(), &tabs(), 9, 3);
        assert_eq!(l.cards.len(), 4);
        assert_eq!(l.cards[0].i, 3);
        assert_eq!(l.card_at(4.0 + 10.0, 4.0 + 20.0 + 10.0), Some(3));
    }

    // ---- toast ----

    /// The detail of a notice fills the box up to the margin, breaks at the space
    /// and never leaves a line wider than the box.
    #[test]
    fn a_notice_detail_wraps_at_the_space() {
        assert_eq!(
            wrap("alpha beta gamma delta", 11, 4),
            vec!["alpha beta", "gamma delta"]
        );
    }

    /// A word with no space in it — a path, a URL — is cut where it reaches the
    /// margin instead of overflowing the box.
    #[test]
    fn a_word_longer_than_the_line_is_cut() {
        assert_eq!(
            wrap("/usr/local/share/very/long/path", 10, 4),
            vec!["/usr/local", "/share/ver", "y/long/pat", "h"]
        );
    }

    /// The cap is lines: a notice that would be taller than the grid keeps the
    /// first lines and drops the rest instead of covering the whole screen.
    #[test]
    fn the_detail_stops_at_the_line_cap() {
        assert_eq!(wrap("a b c d e f g", 1, 2), vec!["a", "b"]);
    }

    /// The box is as wide as the text it will hold, centred, and lifted off the
    /// bottom by the margin — and the box the press tests is that box (`M-Q`).
    /// The right and bottom edges belong to the grid, so a press there still
    /// selects.
    #[test]
    fn the_toast_box_is_where_the_toast_is_drawn() {
        let m = metrics();
        let t = Toast::build(m, "boom", "the disk is full").expect("it has text");
        assert_eq!(t.title, "boom");
        assert_eq!(t.detail, vec!["the disk is full"]);
        // two lines of 16 cells (the detail is the widest) with the gutter of 1.5
        // cell on each side and the accent strip
        let widest = "the disk is full".chars().count() as f32;
        assert_eq!(t.rect[2], widest * m.cw + 3.0 * m.cw + TOAST_BAR);
        assert_eq!(t.rect[3], 2.0 * m.ch + 2.0 * TOAST_PAD_Y);
        // centred on the grid, and the margin holds it off the bottom
        let centred = (m.cols as f32 * m.cw - t.rect[2]) / 2.0;
        assert_eq!(t.rect[0], centred);
        assert_eq!(t.rect[1] + t.rect[3], m.rows as f32 * m.ch - TOAST_MARGIN_Y);
        // the text starts inside the padding and right of the strip
        assert_eq!(
            t.title_at(m),
            (
                t.rect[0] + TOAST_BAR + TOAST_PAD_X * m.cw,
                t.rect[1] + TOAST_PAD_Y
            )
        );
        assert_eq!(t.line_at(m, 0).1, t.rect[1] + TOAST_PAD_Y + m.ch);
        // window px: the grid starts below the bar and right of the sidebar
        let px = f64::from(t.rect[0] + m.pad + m.side + 1.0);
        let py = f64::from(t.rect[1] + m.pad + m.bar + 1.0);
        assert!(t.hit(m, px, py), "inside the box");
        assert!(
            !t.hit(
                m,
                f64::from(m.pad + m.side) + f64::from(t.rect[0]) - 1.0,
                py
            ),
            "one pixel to the left"
        );
        assert!(
            !t.hit(
                m,
                px,
                f64::from(m.pad + m.bar) + f64::from(t.rect[1] + t.rect[3])
            ),
            "the bottom edge belongs to the grid"
        );
        // nothing to say, nothing drawn: an empty box is a hole in the screen
        assert!(Toast::build(m, "", "").is_none());
        // a title wider than the grid is cut, never a box wider than the grid
        let long = Toast::build(m, &"x".repeat(200), "").expect("one line");
        assert!(long.rect[2] <= m.cols as f32 * m.cw);
    }

    // ---- the chrome scene ----

    /// The scene of `metrics()` with nothing open: the bar of `tabs()`, one card
    /// column, history behind the view and the pointer where the caller says.
    fn scene_at<'a>(at: (f64, f64), layout: &'a Layout, parts: Parts<'a>) -> Scene<'a> {
        Scene {
            metrics: metrics(),
            window_w: 184.0 + 400.0,
            at,
            layout,
            menu: parts.menu,
            toast: parts.toast,
            chip: parts.chip,
            scrollbar: parts.scrollbar,
        }
    }

    #[derive(Default, Clone, Copy)]
    struct Parts<'a> {
        menu: Option<&'a Menu>,
        toast: Option<&'a Toast>,
        chip: Option<&'a StateChip>,
        scrollbar: Option<&'a Scrollbar>,
    }

    /// The menu is the last thing drawn, so it is the first to be pressed — even
    /// where a notice is drawn under it, and even where the pointer is not on a
    /// usable row (then the press closes it).
    #[test]
    fn the_menu_claims_the_press_before_the_notice_under_it() {
        let m = metrics();
        let layout = Layout::build(m, &tabs(), 1, 0);
        let toast = Toast::build(m, "boom", "the disk is full").expect("text");
        // the pointer on the middle of the notice
        let at = (
            f64::from(m.pad + m.side) + f64::from(toast.rect[0] + toast.rect[2] / 2.0),
            f64::from(m.pad + m.bar) + f64::from(toast.rect[1] + toast.rect[3] / 2.0),
        );
        // without a menu, the notice wins
        let s = scene_at(
            at,
            &layout,
            Parts {
                toast: Some(&toast),
                ..Parts::default()
            },
        );
        assert_eq!(s.hit(), Hit::Toast);
        // with one hanging from the pointer, the menu's row wins over the notice
        let menu = Menu::build(m, &GRID_MENU, Anchor::Pointer, at);
        let s = scene_at(
            at,
            &layout,
            Parts {
                menu: Some(&menu),
                toast: Some(&toast),
                ..Parts::default()
            },
        );
        assert!(matches!(s.hit(), Hit::MenuRow(_)), "{:?}", s.hit());
        // and the pointer off its rows is still the menu's: the press closes it
        let away = Menu::build(m, &GRID_MENU, Anchor::Pointer, (10.0, 30.0));
        let s = scene_at(
            at,
            &layout,
            Parts {
                menu: Some(&away),
                toast: Some(&toast),
                ..Parts::default()
            },
        );
        assert_eq!(s.hit(), Hit::Menu);
    }

    /// The chip is drawn on the grid's last row, over the strip and the text:
    /// where it stands, it is the chip that is pressed.
    #[test]
    fn the_chip_claims_its_box_and_the_strip_is_the_strips() {
        let m = metrics();
        let layout = Layout::build(m, &tabs(), 1, 0);
        let chip = StateChip {
            word: "done".to_string(),
            detail: String::new(),
        };
        let sb = Scrollbar::new(90, m.rows, 0);
        let rect = chip.rect(m).expect("it fits");
        // inside the box: the chip, not the grid under it
        let at = (
            f64::from(m.pad + m.side + rect[0] + 2.0),
            f64::from(m.pad + m.bar + rect[1] + 2.0),
        );
        let s = scene_at(
            at,
            &layout,
            Parts {
                chip: Some(&chip),
                scrollbar: sb.as_ref(),
                ..Parts::default()
            },
        );
        assert_eq!(s.hit(), Hit::Chip);
        // on the strip, one row up: the scrollbar, not the grid
        let strip_x = 184.0 + f64::from(m.cols as f32 * m.cw) - 2.0;
        let s = scene_at(
            (strip_x, 100.0),
            &layout,
            Parts {
                chip: Some(&chip),
                scrollbar: sb.as_ref(),
                ..Parts::default()
            },
        );
        assert_eq!(s.hit(), Hit::Scrollbar);
        // with nothing to scroll the strip is not there, and the same pixel is
        // the grid's again
        let s = scene_at(
            (strip_x, 100.0),
            &layout,
            Parts {
                chip: Some(&chip),
                ..Parts::default()
            },
        );
        assert_eq!(s.hit(), Hit::Grid);
    }

    /// The rest of the chrome answers where it is drawn: a card, a tab, the bar's
    /// strip, and the grid for everything else.
    #[test]
    fn the_card_the_tab_and_the_bar_answer_where_they_are_drawn() {
        let m = metrics();
        let layout = Layout::build(m, &tabs(), 1, 0);
        // the first card: the test layout puts it under the bar, on the sidebar
        assert_eq!(
            scene_at((14.0, 34.0), &layout, Parts::default()).hit(),
            Hit::Card(0)
        );
        // the first tab: the bar's row, right of the sidebar
        assert_eq!(
            scene_at((190.0, 10.0), &layout, Parts::default()).hit(),
            Hit::Tab(0)
        );
        // the bar's own strip, right of the last tab
        let last = layout.tabs.last().expect("three tabs");
        let strip = 184.0 + f64::from(last.rect[0] + last.rect[2]) + 2.0;
        assert_eq!(
            scene_at((strip, 10.0), &layout, Parts::default()).hit(),
            Hit::Bar
        );
        // and the screen itself, which is the child's
        assert_eq!(
            scene_at((300.0, 100.0), &layout, Parts::default()).hit(),
            Hit::Grid
        );
    }

    // ---- scrollbar ----

    /// The strip is the grid's last `SCROLLBAR_W` px and the thumb its full height
    /// when the view follows the output. The numbers are the ones the painter
    /// gets, and the press target is wider than the strip only by the sliver to
    /// the window's edge.
    #[test]
    fn the_scrollbar_says_the_same_thing_to_the_painter_and_to_the_press() {
        let m = metrics();
        let sb = Scrollbar::new(90, m.rows, 0).expect("there is history");
        let (track, thumb) = sb.parts(m);
        let strip = SCROLLBAR_W.min(m.cw);
        assert_eq!(track, [m.cols as f32 * m.cw - strip, 0.0, strip, 200.0]);
        // live: the thumb sits at the bottom (offset 0)
        assert_eq!(thumb[1] + thumb[3], track[3]);
        // one track height is 10 rows over 100 lines of history
        assert!((thumb[3] - 20.0).abs() < 0.01);
        // the press: the strip, plus the sliver between the grid and the window
        let window_w = f64::from(m.pad + m.side) + 400.0; // 40 cols * 10 = 400
        let strip_left = f64::from(m.pad + m.side) + f64::from(track[0]);
        let mid_y = f64::from(m.pad + m.bar) + 100.0;
        assert!(sb.hit(m, window_w, strip_left, mid_y), "on the strip");
        assert!(sb.hit(m, window_w, window_w - 1.0, mid_y), "the sliver");
        assert!(
            !sb.hit(m, window_w, strip_left - 1.0, mid_y),
            "one pixel left of the strip is text"
        );
        assert!(
            !sb.hit(m, window_w, strip_left, f64::from(m.pad + m.bar) - 1.0),
            "above the grid is the bar"
        );
        // nothing to scroll, no strip: the same `None` the drawing leaves
        assert!(Scrollbar::new(0, m.rows, 0).is_none());
        assert!(Scrollbar::new(90, 0, 0).is_none());
    }

    /// A press moves the view where the thumb goes, and the top of the track is
    /// the oldest line — dragging past the end clamps instead of running away.
    #[test]
    fn a_press_puts_the_thumb_where_the_pointer_is() {
        let m = metrics();
        let sb = Scrollbar::new(90, m.rows, 0).expect("there is history");
        let top = f64::from(m.pad + m.bar);
        // the top of the track is the oldest line
        assert_eq!(sb.offset_at(m, top).unwrap_or(0), 90);
        // the bottom is the live view
        assert_eq!(sb.offset_at(m, top + 199.0).unwrap_or(0), 0);
        // in the middle the thumb's middle is under the pointer: half the history
        let mid = sb.offset_at(m, top + 100.0).unwrap_or(0);
        assert!((44..=46).contains(&mid), "the middle is half, got {mid}");
        // above and below the track clamp, so a wild drag cannot pass the ends
        assert_eq!(sb.offset_at(m, top - 50.0).unwrap_or(0), 90);
        assert_eq!(sb.offset_at(m, top + 500.0).unwrap_or(0), 0);
    }
}
