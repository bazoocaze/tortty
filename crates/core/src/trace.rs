//! The emulation's voice: what the wire delivered and what the parser decided.
//!
//! The core has no log — no file, no clock, no global state. What it has is
//! **trace events**: bounded facts accumulated while parsing (`Trace`), drained
//! by the app with [`crate::Term::take_trace`], which stamps and writes them.
//! A full ring drops the oldest event and keeps parsing: the emulation never
//! stops (and never grows) for lack of a consumer.

use std::collections::VecDeque;

/// How many events the ring holds while nobody drains it. Generous against a
/// busy frame; small enough that an unconsumed tab cannot hoard memory.
pub const TRACE_MAX: usize = 64;

/// The outcome of one `OSC 9;4` progress form on the root record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// The record changed (appeared, moved, or was removed by `0`).
    Applied,
    /// The same state and the same percentage: the same fact told twice.
    Repeat,
    /// A real `7501` report owns the root in flight: the form stays out.
    Locked,
    /// A digit state this terminal gives no meaning to: not this form at all.
    Unknown,
}

/// One decision of the emulation, as the wire delivered it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// The `9;4` progress form: the state, the measure, why it was treated
    /// that way, and the bar the tab shows now.
    Progress {
        state: u8,
        pct: Option<u8>,
        outcome: Outcome,
        bar: Option<(u8, Option<u8>)>,
    },
    /// A notification entered the tab's list.
    Notify {
        source: crate::notif::Source,
        id: Option<String>,
        shade: crate::notif::Shade,
        text: String,
    },
    /// A `7501` report that changed nothing (a repeat, a malformed body) or a
    /// `clear` — nothing to notify, but the fact that it arrived is the trace.
    Status { body: String, what: &'static str },
    /// `133;A`: a new prompt, every record dropped.
    Prompt,
    /// An `OSC 99` that did not become a notification: a chunk held for its
    /// `d=1`, a payload with no surface to draw on, or a refusal (`B72`).
    Osc99(&'static str),
    /// An `OSC 99` lifecycle command (`p=close`/`p=alive`/`p=?`): what it
    /// was and what became of it (`B72` phase two).
    Osc99Cmd { cmd: &'static str, detail: String },
}

impl Event {
    /// The one-line shape of the log, without subject or stamp (those are the
    /// app's: it knows the tab and the clock).
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Progress {
                state,
                pct,
                outcome,
                bar,
            } => {
                let outcome = match outcome {
                    Outcome::Applied => "applied",
                    Outcome::Repeat => "repeat",
                    Outcome::Locked => "locked",
                    Outcome::Unknown => "unknown",
                };
                format!(
                    "OSC 9;4 state={state} pct={} {outcome} bar={bar:?}",
                    pct.map_or("-".to_string(), |p| p.to_string()),
                )
            },
            Self::Notify {
                source,
                id,
                shade,
                text,
            } => format!(
                "{} id={} {shade:?} \"{text}\"",
                source.name(),
                id.as_deref().unwrap_or("-"),
            ),
            Self::Status { body, what } => {
                format!("OSC 7501 \"{body}\" ({what})")
            },
            Self::Prompt => "OSC 133;A — records dropped".to_string(),
            Self::Osc99(what) => format!("OSC 99 {what}"),
            Self::Osc99Cmd { cmd, detail } => format!("OSC 99 {cmd} {detail}"),
        }
    }
}

/// The bounded ring of events nobody has drained yet.
#[derive(Default)]
pub struct Trace {
    ring: VecDeque<Event>,
}

impl Trace {
    /// One more fact. Never fails and never blocks: a full ring forgets the
    /// oldest event — the emulation does not stop for lack of a consumer.
    pub fn push(&mut self, event: Event) {
        if self.ring.len() >= TRACE_MAX {
            self.ring.pop_front();
        }
        self.ring.push_back(event);
    }

    /// Drains everything accumulated (the app reads it once and it is gone).
    pub fn take(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.ring).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_ring_forgets_the_oldest_and_keeps_the_newest() {
        let mut t = Trace::default();
        for i in 0..(TRACE_MAX + 8) as u8 {
            t.push(Event::Prompt);
            let _ = i;
        }
        assert_eq!(t.take().len(), TRACE_MAX);
        assert!(t.take().is_empty(), "the drain takes everything");
    }
}
