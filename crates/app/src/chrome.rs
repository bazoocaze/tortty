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

/// One tab, as the bar needs it: the title and whether it wears the bell.
pub struct Tab<'a> {
    pub title: &'a str,
    pub alert: bool,
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

/// The label of a tab: the number and the title, one cell of gutter each side.
#[must_use]
pub fn tab_label(i: usize, t: &Tab<'_>) -> String {
    format!(" {} {} ", i + 1, t.title)
}

/// How many cells a tab takes: the label, plus the two cells the bell reserves
/// in front of it (`B8`).
#[must_use]
pub fn tab_cells(i: usize, t: &Tab<'_>) -> usize {
    tab_label(i, t).chars().count() + if t.alert { 2 } else { 0 }
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
            },
            Tab {
                title: "two",
                alert: true,
            },
            Tab {
                title: "three",
                alert: false,
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
}
