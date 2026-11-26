//! The notifications of one terminal: what the child asked the user to look at.
//!
//! A notification is a **call to come back**, not a payload: the state lives in
//! the terminal's text, and this list only answers "what was said". Two
//! consequences shape everything here:
//!
//! - it is bounded ([`LIST_MAX`]) and keyed by `(source, id)`: a program that
//!   updates one thing in a loop occupies **one** entry, not eight;
//! - `title` and `body` are kept apart, because a desktop notification wants both
//!   and only the one-line rendering flattens them ([`Notif::text`]).
//!
//! The `id` of one protocol means nothing to another (`i=1` from `OSC 99` and
//! `id=1` from `OSC 7501` are unrelated), so the key carries the source.

use std::collections::VecDeque;

/// Entries kept per terminal. A program that notifies in a loop must not grow
/// this: the oldest pays for the newest.
pub const LIST_MAX: usize = 8;
/// One field of one entry (`title` or `body`), after cleaning.
pub const FIELD_MAX: usize = 512;

/// Who raised it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Source {
    Bell,
    /// `OSC 9;4`: the progress form, which only speaks in the tab's doorway when
    /// it reaches a final state.
    Progress,
    Osc9,
    Osc777,
    Osc99,
    Osc7501,
    /// `OSC 133;D`: the shell reporting how the command ended (`B74`).
    Osc133,
}

impl Source {
    /// How a list shows where an entry came from — and the whole label of an
    /// entry that carries no text (the BEL).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Source::Bell => "BEL",
            Source::Progress => "OSC 9;4",
            Source::Osc9 => "OSC 9",
            Source::Osc777 => "OSC 777",
            Source::Osc99 => "OSC 99",
            Source::Osc7501 => "OSC 7501",
            Source::Osc133 => "OSC 133",
        }
    }
}

/// A rendering hint, not a meaning: the list may colour the entry with it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Shade {
    Info,
    Warn,
    Err,
}

impl Shade {
    /// The shade a report's state wears, kept next to the state table so the two
    /// cannot disagree: `error` is red, a wait for the user is yellow.
    #[must_use]
    pub fn of(state: crate::status::State) -> Shade {
        use crate::status::State;
        match state {
            State::Error => Shade::Err,
            State::Blocked | State::Paused => Shade::Warn,
            State::Done | State::Idle | State::Working => Shade::Info,
        }
    }
}

/// One notification.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Notif {
    pub source: Source,
    /// Identity **inside the source**: a later notification with the same
    /// `(source, id)` replaces this one. `None` = always a new entry.
    pub id: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub shade: Shade,
}

impl Notif {
    /// One line, for a surface that has room for one only.
    #[must_use]
    pub fn text(&self) -> String {
        match (self.title.as_deref(), self.body.as_deref()) {
            (Some(title), Some(body)) => format!("{title} — {body}"),
            (Some(one), None) | (None, Some(one)) => one.to_string(),
            (None, None) => String::new(),
        }
    }

    /// What the entry shows: its text, or — for a notification that says nothing,
    /// the BEL — where it came from.
    #[must_use]
    pub fn label(&self) -> String {
        let text = self.text();
        if text.is_empty() {
            self.source.name().to_string()
        } else {
            text
        }
    }
}

/// The notifications of one terminal, newest first.
#[derive(Default)]
pub struct List {
    entries: VecDeque<Notif>,
}

impl List {
    /// Pushes one notification, replacing the entry with the same
    /// `(source, id)` — the newest position is the one that counts as "last".
    ///
    /// Both free fields are cleaned: control characters out (the child writes
    /// them), each capped at [`FIELD_MAX`], and an empty one becomes `None` — a
    /// `Some("")` would make the flattening add a separator for nothing.
    /// `false` = there was nothing to keep (a message of control characters
    /// only): the caller must not count it as an event.
    pub fn push(
        &mut self,
        source: Source,
        id: Option<&str>,
        title: Option<&str>,
        body: Option<&str>,
        shade: Shade,
    ) -> bool {
        let id = id.and_then(clean);
        let title = title.and_then(clean);
        let body = body.and_then(clean);
        // `OSC 9` has no title and no id; the BEL has neither. An entry that says
        // nothing at all is still an entry: it is the BEL's whole content.
        if title.is_none() && body.is_none() && source != Source::Bell {
            return false;
        }
        if let Some(key) = id.as_deref() {
            self.entries
                .retain(|e| !(e.source == source && e.id.as_deref() == Some(key)));
        } else if source == Source::Bell {
            // the BEL has no id and always says the same thing ("nothing"): one
            // entry per tab, the newest bell
            self.entries.retain(|e| e.source != Source::Bell);
        }
        while self.entries.len() >= LIST_MAX {
            self.entries.pop_front();
        }
        self.entries.push_back(Notif {
            source,
            id,
            title,
            body,
            shade,
        });
        true
    }

    /// Newest first: the first entry is what the tab's mark is about.
    pub fn entries(&self) -> impl Iterator<Item = &Notif> {
        self.entries.iter().rev()
    }

    /// Retires the entry with this `(source, id)` — the `OSC 99` `p=close` —
    /// and says whether there was one (a close of an absent id is ignored,
    /// by spec).
    pub fn remove(&mut self, source: Source, id: &str) -> bool {
        let at = self
            .entries
            .iter()
            .position(|n| n.source == source && n.id.as_deref() == Some(id));
        at.is_some_and(|at| self.entries.remove(at).is_some())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// A free field from the child: control characters out, cap, empty → `None`.
fn clean(text: &str) -> Option<String> {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control())
        .take(FIELD_MAX)
        .collect();
    let text = clean.trim();
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(source: Source, id: Option<&str>, body: &str) -> List {
        let mut list = List::default();
        list.push(source, id, None, Some(body), Shade::Info);
        list
    }

    #[test]
    fn two_messages_are_two_entries_newest_first() {
        let mut list = one(Source::Osc9, None, "first");
        list.push(Source::Osc9, None, None, Some("second"), Shade::Info);
        let texts: Vec<String> = list.entries().map(Notif::label).collect();
        assert_eq!(texts, ["second", "first"]);
    }

    /// The same `(source, id)` is the same notification being updated: it
    /// replaces and moves to the front. `OSC 99` relies on this for progress.
    #[test]
    fn the_same_id_replaces_and_moves_to_the_front() {
        let mut list = one(Source::Osc7501, Some("build"), "working");
        list.push(
            Source::Osc7501,
            Some("build"),
            None,
            Some("done"),
            Shade::Info,
        );
        assert_eq!(list.len(), 1);
        assert_eq!(list.entries().next().unwrap().body.as_deref(), Some("done"));
        // another source with the same id is another notification: `i=1` of the
        // `OSC 99` means nothing to the `7501`
        list.push(Source::Osc99, Some("build"), None, Some("99"), Shade::Info);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn an_entry_without_an_id_is_always_new() {
        let mut list = one(Source::Osc9, None, "a");
        list.push(Source::Osc9, None, None, Some("a"), Shade::Info);
        assert_eq!(list.len(), 2, "no id, no identity");
    }

    #[test]
    fn the_bell_keeps_one_entry_and_labels_itself() {
        let mut list = List::default();
        list.push(Source::Bell, None, None, None, Shade::Info);
        list.push(Source::Bell, None, None, None, Shade::Info);
        assert_eq!(list.len(), 1, "the BEL has no id: it replaces itself");
        assert_eq!(list.entries().next().unwrap().label(), "BEL");
    }

    #[test]
    fn the_cap_keeps_the_last_eight() {
        let mut list = List::default();
        for i in 0..12 {
            list.push(Source::Osc9, None, None, Some(&i.to_string()), Shade::Info);
        }
        assert_eq!(list.len(), LIST_MAX);
        assert_eq!(list.entries().next().unwrap().body.as_deref(), Some("11"));
    }

    #[test]
    fn a_field_with_control_characters_and_a_long_one_are_cleaned() {
        let mut list = List::default();
        let long = "a".repeat(FIELD_MAX + 40);
        list.push(
            Source::Osc9,
            None,
            Some("ti\ntle"),
            Some(&long),
            Shade::Info,
        );
        let n = list.entries().next().unwrap();
        assert_eq!(n.title.as_deref(), Some("title"));
        assert_eq!(n.body.as_deref().map(str::len), Some(FIELD_MAX));
        // a message that is only control characters says nothing: not an entry
        list.push(Source::Osc9, None, None, Some("\u{1}\u{2}"), Shade::Info);
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn the_title_and_the_body_stay_apart_and_flatten_with_a_dash() {
        let mut list = List::default();
        list.push(
            Source::Osc777,
            None,
            Some("make"),
            Some("all done"),
            Shade::Info,
        );
        let n = list.entries().next().unwrap();
        assert_eq!(n.title.as_deref(), Some("make"));
        assert_eq!(n.body.as_deref(), Some("all done"));
        assert_eq!(n.text(), "make — all done");
    }
}
