//! `OSC 7501`, the Program Status Protocol (`B47`).
//!
//! The program says what it is doing — `idle`, `working`, `done`, `blocked`,
//! `error` — instead of the terminal guessing from the screen: `terraform` is
//! *blocked on a permission* is a different thing from "something happened", and
//! it is what `B8` had no source for.
//!
//! The spec (v0.4, 2026-10-10) is implemented with its **hard limits**: a report
//! that breaks one is discarded whole, before any record is touched, so a hostile
//! or buggy program cannot make memory grow. Records belong to the emulation (not
//! to the app), so they live here and the app only presents them.

use std::collections::BTreeMap;

/// Whole sequence, `OSC` through `ST`. The core already cuts an OSC at this
/// size, so a longer body never arrives.
pub const SEQ_MAX: usize = 4096;
/// Key length.
pub const KEY_MAX: usize = 16;
/// `msg`: encoded and decoded.
pub const MSG_ENC_MAX: usize = 2732;
pub const MSG_MAX: usize = 2048;
/// `title`: encoded and decoded.
pub const TITLE_ENC_MAX: usize = 256;
pub const TITLE_MAX: usize = 192;
/// `app`.
pub const APP_MAX: usize = 32;
/// `id`: total, per segment, and levels.
pub const ID_MAX: usize = 128;
pub const SEG_MAX: usize = 32;
pub const SEGS_MAX: usize = 8;
/// Records per terminal; the least recently updated leaves to make room.
pub const RECORDS_MAX: usize = 256;

/// The states a program may report (`clear` is not a state: it removes).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum State {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

impl State {
    /// What a report's `state=` says, `None` for an unknown value (the whole
    /// report is ignored in that case: a state added later must not become
    /// `idle` on an older terminal).
    fn parse(v: &str) -> Option<State> {
        Some(match v {
            "idle" => State::Idle,
            "working" => State::Working,
            "done" => State::Done,
            "blocked" => State::Blocked,
            "error" => State::Error,
            _ => return None,
        })
    }

    /// Does a record in this state ask something of the user? (`done` is "the
    /// result is ready and you have not seen it", so it does.)
    #[must_use]
    pub fn wants_you(self) -> bool {
        matches!(self, State::Done | State::Blocked | State::Error)
    }

    /// The state as the protocol spells it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Working => "working",
            State::Done => "done",
            State::Blocked => "blocked",
            State::Error => "error",
        }
    }
}

/// Why a `blocked` record is blocked.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    Permission,
    Question,
    Auth,
}

impl Kind {
    fn parse(v: &str) -> Option<Kind> {
        Some(match v {
            "permission" => Kind::Permission,
            "question" => Kind::Question,
            "auth" => Kind::Auth,
            _ => return None,
        })
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Kind::Permission => "permission",
            Kind::Question => "question",
            Kind::Auth => "auth",
        }
    }
}

/// One record: the keys of the last report that used that id.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Record {
    pub state: State,
    pub kind: Option<Kind>,
    pub progress: Option<u8>,
    pub app: Option<String>,
    pub title: Option<String>,
    pub msg: Option<String>,
    /// Last update, for the cap: the smallest goes first.
    pub seq: u64,
}

/// What the app shows after a report: the record that asks for the user.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Applied {
    pub id: String,
    pub state: State,
    pub kind: Option<Kind>,
    /// `msg`, else `title`, else the app's name: one line, already decoded.
    pub text: String,
    /// The `app` of the report or of its nearest ancestor (`/`-related).
    pub app: Option<String>,
    /// The record is the root one (no `id`).
    pub root: bool,
}

/// The records of one terminal.
#[derive(Default)]
pub struct Status {
    map: BTreeMap<String, Record>,
    seq: u64,
    /// A real `7501` report arrived: `OSC 9;4` stops feeding the root record
    /// until the next full reset (spec).
    seen: bool,
}

/// The result of one report: the record changed, or the report was ignored.
enum Outcome {
    Ignored,
    Applied(Applied),
    Cleared,
}

impl Status {
    /// A report body (`state=…:id=…:…`, without `7501;`).
    ///
    /// `None` = nothing to show: a malformed report, an unknown state, a broken
    /// limit, or a `clear`. Only the states that want the user come back with an
    /// `Applied`.
    pub fn report(&mut self, body: &[u8]) -> Option<Applied> {
        match self.apply(body) {
            // `idle` and `working` change the record and nothing else: the user
            // is not called for work in progress
            Outcome::Applied(a) if a.state.wants_you() => Some(a),
            Outcome::Applied(_) | Outcome::Ignored | Outcome::Cleared => None,
        }
    }

    /// The records, root first (the `BTreeMap` order puts `""` there).
    pub fn records(&self) -> impl Iterator<Item = (&String, &Record)> {
        self.map.iter()
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Record> {
        self.map.get(id)
    }

    #[must_use]
    pub fn root(&self) -> Option<&Record> {
        self.map.get("")
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    #[must_use]
    pub fn seen_report(&self) -> bool {
        self.seen
    }

    /// `RIS` (and a new `Term`): every record goes.
    pub fn clear(&mut self) {
        self.map.clear();
        self.seen = false;
    }

    /// The user said they read it (`status_show chip`, click): every record goes.
    ///
    /// `seen` survives on purpose — it says this terminal has *heard* a `7501`
    /// report, which stays true after the record is dismissed; forgetting it
    /// would let a later `9;4` take the root back.
    pub fn dismiss(&mut self) {
        self.map.clear();
    }

    /// A new shell prompt (`OSC 133;A`) or the process leaving: `working` and
    /// `blocked` are about the command that is gone; `done` and `error` stay for
    /// the user to find (the spec's lifetime rules).
    pub fn forget_transient(&mut self) {
        self.map
            .retain(|_, r| !matches!(r.state, State::Working | State::Blocked));
    }

    /// `OSC 9;4;state[;pct]` feeding the root record (the spec allows the
    /// mapping, and it is why the form is swallowed today).
    ///
    /// Stops once a real report arrived: a mapped `9;4` would wipe the `kind` and
    /// the `msg` of the record a program wrote.
    pub fn from_progress(&mut self, state: u8, pct: Option<u8>) {
        if self.seen {
            return;
        }
        let (state, progress) = match state {
            0 => {
                self.map.remove("");
                return;
            },
            1 => (State::Working, pct),
            2 => (State::Error, Some(100)),
            4 => (State::Blocked, pct),
            // indeterminate (3) and anything else: busy, no percentage
            _ => (State::Working, None),
        };
        self.seq += 1;
        let seq = self.seq;
        self.map.insert(
            String::new(),
            Record {
                state,
                kind: None,
                progress,
                app: None,
                title: None,
                msg: None,
                seq,
            },
        );
    }

    /// The feature-detection answer (`7501;?`), with the lists this terminal
    /// accepts.
    #[must_use]
    pub fn detection_reply() -> &'static [u8] {
        b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\"
    }

    fn apply(&mut self, body: &[u8]) -> Outcome {
        let Ok(text) = std::str::from_utf8(body) else {
            return Outcome::Ignored;
        };
        // `OSC` through `ST` is capped at the parser; the body is smaller
        if text.len() + 7 > SEQ_MAX {
            return Outcome::Ignored;
        }
        let Some(fields) = pairs(text) else {
            return Outcome::Ignored;
        };
        // `clear` is not a state: it removes the record (and its children) before
        // any state parsing, and it is checked before a record is touched
        if fields.clear {
            return self.clear_record(fields.id.as_deref());
        }
        // every limit is checked before a record is touched
        let Some(state) = fields.state.as_deref().and_then(State::parse) else {
            return Outcome::Ignored;
        };
        // no `id` addresses the root record: a program that only reports itself
        // skips the whole id business
        let id = fields.id.unwrap_or_default();
        // absent (or empty) free text is legal: the record simply has none
        let (msg, title) = (fields.msg, fields.title);
        let app = fields.app.filter(|a| valid_app(a));
        // `kind` only means something with `blocked`; `progress` only with
        // `working`/`blocked` — anything else is treated as absent
        let kind = if state == State::Blocked {
            fields.kind.as_deref().and_then(Kind::parse)
        } else {
            None
        };
        let progress = if matches!(state, State::Working | State::Blocked) {
            fields
                .progress
                .as_deref()
                .and_then(|p| p.parse::<u8>().ok())
                .filter(|p| *p <= 100)
        } else {
            None
        };
        self.seen = true;
        self.seq += 1;
        let seq = self.seq;
        // a record without `app` takes it from the nearest ancestor that has one
        let app = app.or_else(|| self.ancestor_app(&id));
        let record = Record {
            state,
            kind,
            progress,
            app: app.clone(),
            title: title.clone(),
            msg: msg.clone(),
            seq,
        };
        if !self.map.contains_key(&id) && self.map.len() >= RECORDS_MAX {
            self.evict();
        }
        self.map.insert(id.clone(), record);
        let text = msg
            .or(title)
            .or_else(|| app.clone())
            .unwrap_or_else(|| state.name().to_string());
        Outcome::Applied(Applied {
            text,
            state,
            kind,
            app,
            root: id.is_empty(),
            id,
        })
    }

    /// `state=clear`: the record and everything beneath it, or every record.
    fn clear_record(&mut self, id: Option<&str>) -> Outcome {
        match id {
            None => self.map.clear(),
            Some(id) => {
                let below = format!("{id}/");
                self.map.retain(|k, _| k != id && !k.starts_with(&below));
            },
        }
        Outcome::Cleared
    }

    /// The `app` of the nearest ancestor (`build/test` → `build` → root).
    fn ancestor_app(&self, id: &str) -> Option<String> {
        let mut path = id;
        loop {
            path = path.rfind('/').map_or("", |i| &path[..i]);
            if let Some(app) = self.map.get(path).and_then(|r| r.app.clone()) {
                return Some(app);
            }
            if path.is_empty() {
                return None;
            }
        }
    }

    /// Removes the least recently updated record (the cap).
    fn evict(&mut self) {
        let victim = self
            .map
            .iter()
            .min_by_key(|(_, r)| r.seq)
            .map(|(k, _)| k.clone());
        if let Some(k) = victim {
            self.map.remove(&k);
        }
    }
}

/// The pairs of one report, already parsed and validated.
struct Fields {
    state: Option<String>,
    id: Option<String>,
    kind: Option<String>,
    progress: Option<String>,
    app: Option<String>,
    title: Option<String>,
    msg: Option<String>,
    clear: bool,
}

/// Reads the `key=value` pairs.
///
/// `None` = the report breaks a limit or a free-text value does not decode: the
/// whole report is discarded, as the spec requires. A malformed **pair** is only
/// skipped, and an unknown key is ignored (that is how the protocol extends).
fn pairs(text: &str) -> Option<Fields> {
    let mut f = Fields {
        state: None,
        id: None,
        kind: None,
        progress: None,
        app: None,
        title: None,
        msg: None,
        clear: false,
    };
    for pair in text.split(':') {
        let pair = pair.trim();
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key.is_empty() || key.len() > KEY_MAX || !key.bytes().all(|b| b.is_ascii_lowercase()) {
            continue;
        }
        // the value set excludes `:` and `;`, so nothing needs escaping; a byte
        // outside it makes the pair malformed (and the report continues)
        if !value.bytes().all(valid_value_byte) {
            continue;
        }
        match key {
            "state" => {
                f.clear = value == "clear";
                f.state = Some(value.to_string());
            },
            "id" => {
                if !valid_id(value) {
                    // a malformed id must not fall back to the root record
                    return None;
                }
                f.id = Some(value.to_string());
            },
            "kind" => f.kind = Some(value.to_string()),
            "progress" => f.progress = Some(value.to_string()),
            "app" => f.app = Some(value.to_string()),
            "title" => f.title = decode_free(value, TITLE_ENC_MAX, TITLE_MAX)?,
            "msg" => f.msg = decode_free(value, MSG_ENC_MAX, MSG_MAX)?,
            // unknown keys are ignored: this is how the protocol is extended
            _ => {},
        }
    }
    Some(f)
}

fn valid_value_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b',' | b'+' | b'/' | b'=' | b'-')
}

/// `segment ("/" segment)*`, each `[A-Za-z0-9_.+-]{1,32}`, 128 bytes, 8 levels.
fn valid_id(id: &str) -> bool {
    if id.len() > ID_MAX {
        return false;
    }
    let mut levels = 0;
    for seg in id.split('/') {
        levels += 1;
        if levels > SEGS_MAX || seg.is_empty() || seg.len() > SEG_MAX {
            return false;
        }
        if !seg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
        {
            return false;
        }
    }
    true
}

fn valid_app(app: &str) -> bool {
    !app.is_empty()
        && app.len() <= APP_MAX
        && app
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'+' | b'-'))
}

/// A free-text value: the encoded size is checked before decoding, then the
/// decoded size, then control characters — the three ways the spec discards a
/// whole report.
// Two levels on purpose: `None` discards the report, `Some(None)` is "present but
// empty" — collapsing them would keep a broken `msg` from breaking the record.
#[allow(clippy::option_option)]
fn decode_free(value: &str, enc_max: usize, max: usize) -> Option<Option<String>> {
    if value.len() > enc_max {
        return None;
    }
    let bytes = crate::graphics::base64_decode(value)?;
    if bytes.len() > max {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    if text.chars().any(control) {
        return None;
    }
    Some(if text.is_empty() { None } else { Some(text) })
}

/// The ranges the spec forbids in `msg`/`title`: C0, DEL and the C1 block.
fn control(c: char) -> bool {
    let c = c as u32;
    matches!(c, 0x00..=0x1f | 0x7f | 0x80..=0x9f)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Base64 of a UTF-8 text, without padding (the spec allows it).
    fn b64(text: &str) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(text)
    }

    #[test]
    fn one_program_keeps_one_record() {
        let mut s = Status::default();
        assert_eq!(
            s.report(
                format!("state=working:app=brew:msg={}", b64("Installing updates")).as_bytes()
            ),
            None,
            "working does not call the user"
        );
        let r = s.root().expect("the root record exists");
        assert_eq!(r.state, State::Working);
        assert_eq!(r.app.as_deref(), Some("brew"));
        assert_eq!(r.msg.as_deref(), Some("Installing updates"));
        // a report replaces its record completely
        s.report(format!("state=done:app=brew:msg={}", b64("Upgraded 12 packages")).as_bytes());
        let r = s.root().expect("still there");
        assert_eq!(r.state, State::Done);
        assert_eq!(r.progress, None);
    }

    #[test]
    fn blocked_asks_for_the_user_with_the_kind_and_the_message() {
        let mut s = Status::default();
        let a = s
            .report(
                format!(
                    "state=blocked:kind=permission:app=terraform:msg={}",
                    b64("Apply 3 to add, 1 to change, 0 to destroy?")
                )
                .as_bytes(),
            )
            .expect("blocked wants you");
        assert_eq!(a.state, State::Blocked);
        assert_eq!(a.kind, Some(Kind::Permission));
        assert_eq!(a.app.as_deref(), Some("terraform"));
        assert_eq!(a.text, "Apply 3 to add, 1 to change, 0 to destroy?");
        assert!(a.root);
    }

    #[test]
    fn several_records_and_the_root_inherits_down() {
        let mut s = Status::default();
        s.report(format!("state=working:app=deploy:msg={}", b64("Deploying v2.4.1")).as_bytes());
        s.report(
            format!(
                "state=working:id=us-east:title={}:progress=40:msg={}",
                b64("US East"),
                b64("Pushing image")
            )
            .as_bytes(),
        );
        s.report(
            format!(
                "state=blocked:kind=permission:id=eu-west:title={}:msg={}",
                b64("EU West"),
                b64("Approve deploy to eu-west (production)?")
            )
            .as_bytes(),
        );
        assert_eq!(s.len(), 3);
        // the child took `app` from the root: "the nearest ancestor that has one"
        assert_eq!(s.get("us-east").unwrap().app.as_deref(), Some("deploy"));
        assert_eq!(s.get("us-east").unwrap().progress, Some(40));
        assert_eq!(s.get("eu-west").unwrap().title.as_deref(), Some("EU West"));
        // and the root is still working: both are true
        assert_eq!(s.root().unwrap().state, State::Working);
    }

    #[test]
    fn clear_takes_the_record_and_everything_beneath_it() {
        let mut s = Status::default();
        s.report(b"state=working:app=build");
        s.report(b"state=working:id=build");
        s.report(b"state=working:id=build/test");
        s.report(b"state=working:id=build/other");
        s.report(b"state=clear:id=build");
        let left: Vec<&str> = s.records().map(|(k, _)| k.as_str()).collect();
        assert_eq!(left, [""], "the root and nothing beneath `build`");
        // with no id, everything goes
        s.report(b"state=clear");
        assert!(s.is_empty());
    }

    #[test]
    fn the_state_is_required_and_an_unknown_one_ignores_the_report() {
        let mut s = Status::default();
        assert_eq!(s.report(b"app=brew"), None);
        assert!(s.is_empty());
        assert_eq!(s.report(b"state=paused:app=brew"), None, "a later state");
        assert!(s.is_empty());
    }

    #[test]
    fn a_malformed_id_does_not_fall_back_to_the_root() {
        let mut s = Status::default();
        s.report(b"state=done:app=ok");
        // a byte outside the value set is a **malformed pair**, which the spec
        // skips: the report then has no id, which is the root record
        s.report(b"state=working:app=brew");
        assert_eq!(s.root().unwrap().app.as_deref(), Some("brew"));
        // an id that breaks the grammar (nine levels, a 33-byte segment) ignores
        // the whole report instead: a malformed id must not overwrite the root
        assert_eq!(s.report(b"state=clear:id=a/b/c/d/e/f/g/h/i"), None);
        assert_eq!(s.root().unwrap().state, State::Working, "still there");
        let long = "a".repeat(SEG_MAX + 1);
        assert_eq!(
            s.report(format!("state=done:id={long}").as_bytes()),
            None,
            "past the segment limit"
        );
        assert_eq!(s.root().unwrap().state, State::Working);
        // and a well-formed child is a record of its own
        s.report(b"state=done:id=build/test");
        assert_eq!(s.get("build/test").unwrap().state, State::Done);
        assert_eq!(s.root().unwrap().state, State::Working);
    }

    #[test]
    fn a_message_with_a_control_character_discards_the_whole_report() {
        let mut s = Status::default();
        s.report(b"state=working:app=keep");
        let bad = b64("two\nlines");
        assert_eq!(
            s.report(format!("state=done:app=other:msg={bad}").as_bytes()),
            None
        );
        // nothing of that report was applied: the root is untouched
        assert_eq!(s.root().unwrap().state, State::Working);
        assert_eq!(s.root().unwrap().app.as_deref(), Some("keep"));
    }

    #[test]
    fn a_limit_breaks_the_whole_report() {
        let mut s = Status::default();
        // `msg` above 2732 encoded bytes
        let huge = "A".repeat(MSG_ENC_MAX + 4);
        assert_eq!(s.report(format!("state=done:msg={huge}").as_bytes()), None);
        assert!(s.is_empty());
        // `title` above 256 encoded bytes
        let long = "A".repeat(TITLE_ENC_MAX + 4);
        assert_eq!(
            s.report(format!("state=done:title={long}").as_bytes()),
            None
        );
        // a key longer than 16 bytes is only a malformed pair: skipped
        let a = s.report(b"state=done:averyverylongkeyname=1");
        assert!(a.is_some(), "the rest of the report still works");
    }

    #[test]
    fn the_cap_evicts_the_least_recently_updated() {
        let mut s = Status::default();
        for i in 0..RECORDS_MAX {
            s.report(format!("state=working:id=r{i}").as_bytes());
        }
        assert_eq!(s.len(), RECORDS_MAX);
        s.report(b"state=working:id=root");
        assert_eq!(s.len(), RECORDS_MAX);
        s.report(b"state=working:id=fresh");
        assert_eq!(s.len(), RECORDS_MAX);
        assert!(s.get("fresh").is_some());
        assert!(s.get("r0").is_none(), "the oldest paid for it");
    }

    #[test]
    fn a_new_prompt_drops_working_and_blocked_but_not_done() {
        let mut s = Status::default();
        s.report(b"state=working:id=a");
        s.report(b"state=blocked:id=b");
        s.report(b"state=done:id=c");
        s.report(b"state=error:id=d");
        s.forget_transient();
        let left: Vec<&str> = s.records().map(|(k, _)| k.as_str()).collect();
        assert_eq!(left, ["c", "d"]);
    }

    #[test]
    fn osc_9_4_feeds_the_root_until_a_real_report_arrives() {
        let mut s = Status::default();
        s.from_progress(1, Some(37));
        assert_eq!(s.root().unwrap().state, State::Working);
        assert_eq!(s.root().unwrap().progress, Some(37));
        // the real report owns the record now
        s.report(b"state=blocked:kind=auth:app=brew");
        s.from_progress(1, Some(80));
        assert_eq!(s.root().unwrap().state, State::Blocked);
        assert_eq!(s.root().unwrap().progress, None);
        // `0` clears, but only while the mapping is alive
        s.report(b"state=clear");
        assert!(s.root().is_none());
    }

    #[test]
    fn the_feature_detection_answer_names_both_lists() {
        let reply = std::str::from_utf8(Status::detection_reply()).unwrap();
        assert!(reply.starts_with("\x1b]7501;?"));
        assert!(reply.ends_with("\x1b\\"));
        assert!(reply.contains("states=idle,working,done,blocked,error"));
        assert!(reply.contains("kinds=permission,question,auth"));
    }
}
