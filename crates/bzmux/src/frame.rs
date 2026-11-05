//! Draw gate: at most one frame every ~16 ms, and nothing while `?2026`
//! (synchronized output) is open — G16/B28/G15.
//!
//! Why it exists: `request_redraw` on every pty read gave one drawing per read
//! chunk under a continuous flood, and the drawing cost started competing with
//! the reading (the screen still froze, because the redraw has lower priority
//! than the pty watch in Python). It is the same decision as `view.py:286-320`:
//! coalesce and pace. With `?2026` open the app will still redraw, so drawing now
//! shows the **middle** of each repaint — hold until the close, with a 200 ms cap
//! for an app that forgets to close.
//!
//! It lives in a pure type without `winit` on purpose: the pacing is testable
//! without a display, and `app.rs` only asks "due?" and "when?".

use std::time::{Duration, Instant};

/// Coalesced frame pacing: ~60 Hz.
pub const FRAME_MS: u64 = 16;
/// Cap of the `?2026` transaction: an app that opens and never closes does not freeze the screen.
pub const SYNC_MS: u64 = 200;

#[derive(Debug, Default)]
pub struct Gate {
    /// When the next frame may go out; `None` = nothing pending.
    frame_at: Option<Instant>,
    /// End of the ongoing `?2026` window (the safety cap).
    sync_until: Option<Instant>,
    /// `?2026` was open on the last read: that is how the close is seen.
    sync_was: bool,
}

impl Gate {
    /// More pty output arrived. `sync` is the `?2026` of the visible tab **now**
    /// (that is why the caller reads the parser after feeding it, not before).
    pub fn request(&mut self, now: Instant, sync: bool) {
        if sync {
            // a live window is kept; a blown cap opens another, otherwise the app
            // that never closes `?2026` would draw on every read
            let until = match self.sync_until {
                Some(a) if a > now => a,
                _ => now + Duration::from_millis(SYNC_MS),
            };
            self.sync_until = Some(until);
            self.sync_was = true;
            self.frame_at = Some(until);
            return;
        }
        // the transaction closed: the final state goes out now, without waiting for the pace
        let closed = std::mem::take(&mut self.sync_was);
        self.sync_until = None;
        if closed {
            self.frame_at = Some(now);
            return;
        }
        // coalesced: whoever already has a scheduled time does not reschedule (that is the point)
        self.frame_at
            .get_or_insert(now + Duration::from_millis(FRAME_MS));
    }

    /// Is the frame due? Returns `true` once and releases the gate.
    pub fn won(&mut self, now: Instant) -> bool {
        if self.frame_at.is_some_and(|t| now >= t) {
            self.frame_at = None;
            return true;
        }
        false
    }

    /// When to wake up to draw (for `ControlFlow::WaitUntil`).
    #[must_use]
    pub fn when(&self) -> Option<Instant> {
        self.frame_at
    }
}

#[cfg(test)]
mod checks_frame {
    use super::{Gate, FRAME_MS, SYNC_MS};
    use std::time::{Duration, Instant};

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn back_to_back_reads_draw_once() {
        let mut g = Gate::default();
        let t = t0();
        g.request(t, false);
        // another read 5 ms later reschedules nothing: a single frame
        g.request(t + Duration::from_millis(5), false);
        let time = g.when().expect("there is a frame scheduled");
        assert_eq!(time, t + Duration::from_millis(FRAME_MS));
        assert!(!g.won(t + Duration::from_millis(FRAME_MS - 1)));
        assert!(g.won(t + Duration::from_millis(FRAME_MS)));
        // and the gate released: it does not become due twice at the same time
        assert!(!g.won(t + Duration::from_millis(FRAME_MS)));
        // new output after the frame schedules another
        g.request(t + Duration::from_millis(FRAME_MS + 1), false);
        assert!(g.when().is_some());
    }

    #[test]
    fn sync_open_holds_until_the_close() {
        let mut g = Gate::default();
        let t = t0();
        g.request(t, true);
        // nothing goes out at the normal pace: the transaction is open
        assert!(!g.won(t + Duration::from_millis(FRAME_MS)));
        // one more request does not stretch the window
        g.request(t + Duration::from_millis(100), true);
        assert_eq!(g.when(), Some(t + Duration::from_millis(SYNC_MS)));
        // closed: the final state goes out right away
        g.request(t + Duration::from_millis(120), false);
        assert_eq!(g.when(), Some(t + Duration::from_millis(120)));
        assert!(g.won(t + Duration::from_millis(120)));
    }

    #[test]
    fn sync_that_never_closes_draws_at_the_cap_and_not_on_each_read() {
        let mut g = Gate::default();
        let t = t0();
        g.request(t, true);
        assert!(g.won(t + Duration::from_millis(SYNC_MS)));
        // still open: a new window (otherwise it would draw on every read from
        // here on, which is the bug the gate exists to avoid)
        let after = t + Duration::from_millis(SYNC_MS + 1);
        g.request(after, true);
        assert_eq!(g.when(), Some(after + Duration::from_millis(SYNC_MS)));
        assert!(!g.won(after + Duration::from_millis(FRAME_MS)));
    }

    #[test]
    fn without_request_there_is_no_frame() {
        let mut g = Gate::default();
        assert_eq!(g.when(), None);
        assert!(!g.won(t0()));
    }
}
