//! Damage: which grid lines must be rebuilt in this frame (A17/G17).
//!
//! Whoever writes marks the line, and drawing only remakes what got dirty. Here the mark
//! already exists in the core (`Term::dirty` / `dirty_all`); what was missing was
//! drawing using it instead of rebuilding the whole grid every frame.
//!
//! The decision lives here, in a pure module, for one reason: this is where it is
//! easy to get wrong. Marking "only the dirty line" is safe only when **nothing
//! global** changed — if the view scrolled, if the selection showed up, if the
//! theme changed or if the line count changed, every line is a different one.
//! That is the [`Signature`]: if it changed, the frame is whole; if it did not,
//! the line-by-line damage applies. Keeping that rule in one place (instead of
//! spreading it through `app` and `render`) is what avoids the classic bug of
//! "stale content shows up until the screen scrolls".

/// Everything that changes the whole screen at once.
///
/// Deliberately **not** here are things drawn outside the grid (cursor, tab bar,
/// sidebar, toast, history badge): they do not live in
/// [`crate::render::Renderer::por_linha`] and are rebuilt every frame anyway.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Signature {
    /// Which tab/terminal is on screen: switching tabs swaps all the content.
    pub tab: u64,
    pub cols: usize,
    /// Visible lines (changes when the tab bar shows up, for example).
    pub rows: usize,
    /// How many lines the history scrolled: changes the content of every line.
    pub view_off: usize,
    /// Visible selection, already clipped to the screen: `(row1, col1, row2, col2)`.
    pub sel: Option<(usize, usize, usize, usize)>,
    /// Ctrl held: the `OSC 8` links are underlined, which changes the instances
    /// of every line that carries one.
    pub links: bool,
    /// Blink phase **when** an SGR 5 cell is on screen. Without it the cursor
    /// blink would force a full rebuild twice a second.
    pub blink: bool,
    /// Theme generation: palette, fg/bg/cursor and font change the color of everything.
    pub theme: u64,
    pub fg: (u8, u8, u8),
    pub bg: (u8, u8, u8),
}

/// What to rebuild in this frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Damage {
    /// `All`: something screen-wide changed (or the core damage is global).
    All,
    /// `Rows`: only these lines (the others stay as they are).
    Rows(Vec<usize>),
}

/// Decides the frame damage and already updates the stored signature.
///
/// `dirty` is the core damage (one position per grid line). A line missing from
/// the vector counts as dirty: unknown damage never turns into a stale screen.
pub fn decide(
    current: &Signature,
    previous: &mut Signature,
    global: bool,
    dirty: &[bool],
    rows: usize,
) -> Damage {
    let changed = *previous != *current;
    previous.clone_from(current);
    if global || changed {
        return Damage::All;
    }
    let mut rows_dirty = Vec::new();
    for y in 0..rows {
        if dirty.get(y).copied().unwrap_or(true) {
            rows_dirty.push(y);
        }
    }
    Damage::Rows(rows_dirty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature() -> Signature {
        Signature {
            tab: 1,
            cols: 80,
            rows: 24,
            view_off: 0,
            sel: None,
            links: false,
            blink: false,
            theme: 1,
            fg: (0xd6, 0xd6, 0xd6),
            bg: (0, 0, 0),
        }
    }

    #[test]
    fn switching_tab_rebuilds_all() {
        // two tabs with the same geometry: reusing the other one's line would
        // show the wrong content
        let mut previous = signature();
        let mut next = signature();
        next.tab = 2;
        assert_eq!(
            decide(&next, &mut previous, false, &[false; 24], 24),
            Damage::All
        );
    }

    #[test]
    fn only_a_dirty_line_is_rebuilt() {
        let mut previous = signature();
        let mut dirty = vec![false; 24];
        dirty[7] = true;
        let d = decide(&signature(), &mut previous, false, &dirty, 24);
        assert_eq!(d, Damage::Rows(vec![7]));
    }

    #[test]
    fn nothing_dirty_rebuilds_nothing() {
        let mut previous = signature();
        let d = decide(&signature(), &mut previous, false, &[false; 24], 24);
        assert_eq!(d, Damage::Rows(vec![]));
    }

    #[test]
    fn global_damage_rebuilds_all() {
        let mut previous = signature();
        assert_eq!(
            decide(&signature(), &mut previous, true, &[false; 24], 24),
            Damage::All
        );
    }

    #[test]
    fn scrolling_the_view_rebuilds_all() {
        let mut previous = signature();
        let mut next = signature();
        next.view_off = 3;
        assert_eq!(
            decide(&next, &mut previous, false, &[false; 24], 24),
            Damage::All
        );
    }

    #[test]
    fn selection_rebuilds_all() {
        let mut previous = signature();
        let mut next = signature();
        next.sel = Some((3, 1, 3, 5));
        assert_eq!(
            decide(&next, &mut previous, false, &[false; 24], 24),
            Damage::All
        );
    }

    #[test]
    fn switching_theme_rebuilds_all() {
        let mut previous = signature();
        let mut next = signature();
        next.theme = 2;
        assert_eq!(
            decide(&next, &mut previous, false, &[false; 24], 24),
            Damage::All
        );
    }

    #[test]
    fn line_count_appeared_rebuilds_all() {
        // 24 → 23 lines (the tab bar stole one): everything is another line
        let mut previous = signature();
        let mut next = signature();
        next.rows = 23;
        assert_eq!(
            decide(&next, &mut previous, false, &[false; 24], 23),
            Damage::All
        );
    }

    #[test]
    fn blink_with_sgr5_cell_rebuilds_all() {
        let mut previous = signature();
        let mut next = signature();
        next.blink = true;
        assert_eq!(
            decide(&next, &mut previous, false, &[false; 24], 24),
            Damage::All
        );
    }

    #[test]
    fn unknown_damage_counts_as_dirty() {
        // `dirty` shorter than the lines: what is missing is rebuilt, not ignored
        let mut previous = signature();
        let d = decide(&signature(), &mut previous, false, &[false, true], 4);
        assert_eq!(d, Damage::Rows(vec![1, 2, 3]));
    }

    #[test]
    fn signature_is_stored_for_the_next_frame() {
        let mut previous = signature();
        let mut next = signature();
        next.view_off = 5;
        let _ = decide(&next, &mut previous, false, &[false; 24], 24);
        // the next frame, identical, is no longer "everything"
        let d = decide(&next, &mut previous, false, &[false; 24], 24);
        assert_eq!(d, Damage::Rows(vec![]));
    }
}
