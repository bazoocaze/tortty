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
        #[allow(clippy::cast_possible_truncation)]
        let (x, y) = (px as f32, py as f32);
        (x - self.m.pad - self.m.side, y - self.m.pad - self.m.bar)
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

#[cfg(test)]
mod tests {
    use super::*;

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
