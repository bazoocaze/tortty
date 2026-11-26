//! `OSC 99` — kitty's identified desktop notification, `B72`: parse, assemble,
//! notify, and the lifecycle that needs no click channel (`p=close`, the
//! `p=alive` and `p=?` queries). What stays silent is the click side: the
//! activation report (`a=report`) and close-event reports (`c=1`) need a click
//! surface and a way to find the tab that owns an id — see `docs/BACKLOG.md`.
//!
//! Wire (`kitty/desktop-notifications`): `OSC 99 ; metadata ; payload ST` —
//! both semicolons always present, metadata colon-separated `key=value` pairs.
//! The `p` key names what the payload is (`title`, `body`, `icon`, `buttons`,
//! `close`, `alive`, `?`); `i` is the notification id; `d=0` holds the
//! notification until a chunk with the same id says `d=1`; `e=1` says the
//! payload is Base64. A payload is at most 2048 bytes plain, 4096 encoded.
//! Unknown keys and values are ignored, by spec.
//!
//! No I/O, no clock: the assembler is a pure map, and the caller decides what
//! an [`Outcome`] becomes.

use std::collections::BTreeMap;

use crate::notif::{Notif, Shade, Source};

/// Payload cap, plain bytes (the spec's DoS answer).
const PLAIN_MAX: usize = 2048;
/// Payload cap, encoded bytes; also the cap of the assembled whole.
const ENCODED_MAX: usize = 4096;
/// How many notifications may sit half-assembled at once. An id that never
/// sends its `d=1` must not rent memory forever: the oldest pending assembly
/// leaves when the map is full.
const PENDING_MAX: usize = 16;

/// What one `OSC 99` escape became.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Assembled (possibly after chunks): display it.
    Notify(Notif),
    /// A chunk of an unfinished notification: stored, nothing to show.
    Held,
    /// `p=close`: retire the notification with this id — `None` is a no-op,
    /// by spec. The id may also name an assembly still held: it dies with it.
    Close(Option<String>),
    /// `p=alive`: the id echo (the caller's `i`, or `0`) — the caller knows
    /// which ids are alive and writes the reply.
    Alive(Option<String>),
    /// `p=?`: the id echo — the caller writes the support reply.
    Query(Option<String>),
    /// The escape said something this terminal gives no surface to (`p=icon`,
    /// `p=buttons`, the close-event request `c=1`) — ignored, by spec.
    Ignored,
    /// Refused: the payload broke a cap or the escape has no payload section.
    Refused(&'static str),
}

/// What the `p=?` reply declares this terminal supports. Payload types are
/// the two this terminal renders; occasions are none, and the spec demands
/// `o=always` in that case. No actions, no close events, no sounds, no
/// urgency, no expiry: the keys stay absent rather than lie.
pub const SUPPORT: &str = "p=title,body:o=always";

/// The `p` values that carry displayable text.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Piece {
    Title,
    Body,
}

fn piece(name: &str) -> Option<Piece> {
    match name {
        "title" => Some(Piece::Title),
        "body" => Some(Piece::Body),
        _ => None,
    }
}

/// The half-assembled notifications keyed by id.
#[derive(Default)]
pub struct Assembler {
    partial: BTreeMap<String, Partial>,
}

struct Partial {
    title: String,
    body: String,
}

impl Assembler {
    /// One escape in, one [`Outcome`] out. Never grows without a bound: the
    /// pending map is capped (`PENDING_MAX`, oldest dropped) and every text
    /// cap is checked before memory is rented.
    pub fn feed(&mut self, arg: &[u8]) -> Outcome {
        let Some((meta, payload)) = split_sections(arg) else {
            return Outcome::Refused("no payload section");
        };
        let (id, done, encoded, kind, saw_p, cmd) = read_meta(meta);
        // the commands ride on the metadata alone (their payload is empty by
        // spec): no caps, no assembly — the caller acts and answers. A close
        // also consumes an assembly still held under the id: the program
        // saying "forget it" means the whole of it.
        if let Some(cmd) = cmd {
            let outcome = cmd(id.clone());
            if let (Outcome::Close(Some(_)), Some(i)) = (&outcome, &id) {
                self.partial.remove(i);
            }
            return outcome;
        }
        let kind = match (saw_p, kind) {
            (false, None) => Piece::Title,
            (_, Some(k)) => k,
            (true, None) => return Outcome::Ignored,
        };
        // the caps are per escape, before decoding: an oversized piece never
        // reaches the assembler
        let cap = if encoded { ENCODED_MAX } else { PLAIN_MAX };
        if payload.len() > cap {
            return Outcome::Refused("payload over the cap");
        }
        let text = if encoded {
            let Some(bytes) = crate::graphics::base64_decode(&String::from_utf8_lossy(payload))
            else {
                return Outcome::Refused("base64 payload");
            };
            match String::from_utf8(bytes) {
                Ok(t) => t,
                Err(_) => return Outcome::Refused("payload not utf-8"),
            }
        } else {
            String::from_utf8_lossy(payload).into_owned()
        };
        // without an id the notification is unique by definition (the spec
        // forbids updating unidentified ones), so there is nothing to assemble
        // with: the piece displays by itself and `d=0` holds nothing
        let Some(id) = id else {
            return Outcome::Notify(self::notification(None, kind, text));
        };
        if !self.partial.contains_key(&id) {
            if self.partial.len() >= PENDING_MAX {
                // the oldest key leaves (BTreeMap iterates in order)
                if let Some(oldest) = self.partial.keys().next().cloned() {
                    self.partial.remove(&oldest);
                }
            }
            self.partial.insert(
                id.clone(),
                Partial {
                    title: String::new(),
                    body: String::new(),
                },
            );
        }
        let entry = self.partial.get_mut(&id).expect("just ensured");
        let target = match kind {
            Piece::Title => &mut entry.title,
            Piece::Body => &mut entry.body,
        };
        // the assembled whole is capped too: a piece that does not fit is
        // dropped, the assembly keeps what it had
        if target.len() + text.len() <= ENCODED_MAX {
            target.push_str(&text);
        }
        if !done {
            return Outcome::Held;
        }
        let whole = self.partial.remove(&id).expect("assembled above");
        let Partial { title, body } = whole;
        Outcome::Notify(Notif {
            source: Source::Osc99,
            id: Some(id),
            title: (!title.is_empty()).then_some(title),
            // a notification may be all body, or all title: the empty side
            // stays empty, and one that arrived with neither shows nothing —
            // but a piece just assembled is always there to fall back to
            body: if body.is_empty() { None } else { Some(body) },
            shade: Shade::Info,
        })
    }
}

fn split_sections(arg: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = arg.iter().position(|b| *b == b';')?;
    Some((&arg[..at], &arg[at + 1..]))
}

/// The metadata section, read once: the id, whether the notification is done,
/// whether the payload is Base64, and what the `p` key said. A `p` naming a
/// lifecycle command (close/alive/?) comes back in `cmd`; the other `p`
/// values come back in `kind` (an unknown one, as `None`).
type Cmd = fn(Option<String>) -> Outcome;
#[allow(clippy::type_complexity)] // the six facts of one metadata section
fn read_meta(meta: &[u8]) -> (Option<String>, bool, bool, Option<Piece>, bool, Option<Cmd>) {
    let mut id: Option<String> = None;
    let mut done = true;
    let mut encoded = false;
    let mut kind: Option<Piece> = None;
    let mut cmd: Option<Cmd> = None;
    let mut saw_p = false;
    for pair in String::from_utf8_lossy(meta).split(':') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match (key, value) {
            ("i", v) => id = Some(v.chars().take(crate::notif::FIELD_MAX).collect()),
            ("d", v) => done = v != "0",
            ("e", "1") => encoded = true,
            ("p", "close") => {
                saw_p = true;
                cmd = Some(Outcome::Close);
            },
            ("p", "alive") => {
                saw_p = true;
                cmd = Some(Outcome::Alive);
            },
            ("p", "?") => {
                saw_p = true;
                cmd = Some(Outcome::Query);
            },
            ("p", v) => {
                saw_p = true;
                kind = piece(v);
            },
            // `f`, `t`, `n`, `g`, `s`, `a`, `c`, `w`, `o`, `u` — filtering
            // and click-lifecycle keys with no surface here; unknown keys
            // too. The spec says: ignore and keep going.
            _ => {},
        }
    }
    (id, done, encoded, kind, saw_p, cmd)
}

/// A one-piece notification: the piece is whichever side it is, the other side
/// stays empty (an all-body notification shows only the body).
fn notification(id: Option<String>, kind: Piece, text: String) -> Notif {
    let title = matches!(kind, Piece::Title).then_some(text.clone());
    let body = matches!(kind, Piece::Body).then_some(text);
    Notif {
        source: Source::Osc99,
        id,
        title,
        body,
        shade: Shade::Info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(a: &mut Assembler, esc: &str) -> Outcome {
        a.feed(esc.as_bytes())
    }

    #[test]
    fn a_plain_title_and_body_become_one_notification() {
        let mut a = Assembler::default();
        assert_eq!(
            feed(&mut a, "p=title;Hello world"),
            Outcome::Notify(Notif {
                source: Source::Osc99,
                id: None,
                title: Some("Hello world".into()),
                body: None,
                shade: Shade::Info,
            })
        );
        assert!(matches!(
            feed(&mut a, "p=body;This is cool"),
            Outcome::Notify(_)
        ));
    }

    #[test]
    fn chunks_held_under_an_id_display_on_done() {
        let mut a = Assembler::default();
        assert!(matches!(
            feed(&mut a, "i=1:d=0:p=title;big "),
            Outcome::Held
        ));
        assert!(matches!(
            feed(&mut a, "i=1:d=0:p=title;title and "),
            Outcome::Held
        ));
        let out = feed(&mut a, "i=1:d=1:p=body;long body");
        assert!(matches!(out, Outcome::Notify(_)), "{out:?}");
        match out {
            Outcome::Notify(n) => {
                assert_eq!(n.id.as_deref(), Some("1"));
                assert_eq!(n.title.as_deref(), Some("big title and "));
                assert_eq!(n.body.as_deref(), Some("long body"));
            },
            _ => unreachable!(),
        }
        // the assembly is spent: the same id starts a fresh one
        assert!(matches!(
            feed(&mut a, "i=1:p=title;fresh"),
            Outcome::Notify(_)
        ));
    }

    #[test]
    fn base64_payloads_decode_unpadded() {
        let mut a = Assembler::default();
        // "pinned" base64, unpadded
        let out = feed(&mut a, "i=2:e=1:p=body;cGlubmVk");
        match out {
            Outcome::Notify(n) => assert_eq!(n.body.as_deref(), Some("pinned")),
            other => panic!("{other:?}"),
        }
        // padding is accepted too
        assert!(matches!(
            feed(
                &mut a,
                "i=3:e=1:p=body;cGlubmVk
"
            ),
            Outcome::Notify(_)
        ));
        // and garbage is refused, not shown
        assert!(matches!(
            feed(&mut a, "i=4:e=1:p=body;!!!"),
            Outcome::Refused(_)
        ));
    }

    #[test]
    fn oversized_payloads_are_refused_before_the_assembler_sees_them() {
        let mut a = Assembler::default();
        let plain = "x".repeat(2049);
        assert!(matches!(
            feed(&mut a, &format!("i=5:p=body;{plain}")),
            Outcome::Refused(_)
        ));
        let encoded = "x".repeat(4097);
        assert!(matches!(
            feed(&mut a, &format!("i=6:e=1:p=body;{encoded}")),
            Outcome::Refused(_)
        ));
        // 2048 plain fits, 4096 encoded fits
        assert!(matches!(
            feed(&mut a, &format!("i=7:p=body;{}", "x".repeat(2048))),
            Outcome::Notify(_)
        ));
    }

    #[test]
    fn surfaces_this_phase_has_no_room_for_are_ignored_silently() {
        let mut a = Assembler::default();
        assert!(matches!(feed(&mut a, "p=icon;png"), Outcome::Ignored));
        assert!(matches!(feed(&mut a, "p=buttons;yes"), Outcome::Ignored));
        // unknown keys do not stop the payload
        assert!(matches!(
            feed(&mut a, "z=9:w=1000:p=title;kept"),
            Outcome::Notify(_)
        ));
        // no payload section at all (the two semicolons are the wire's law)
        assert!(matches!(feed(&mut a, "p=title"), Outcome::Refused(_)));
    }

    #[test]
    fn the_lifecycle_commands_carry_their_id_and_skip_the_assembly() {
        let mut a = Assembler::default();
        assert_eq!(
            feed(&mut a, "i=8:p=close;"),
            Outcome::Close(Some("8".into()))
        );
        assert_eq!(
            feed(&mut a, "i=9:p=alive;"),
            Outcome::Alive(Some("9".into()))
        );
        assert_eq!(feed(&mut a, "i=10:p=?;"), Outcome::Query(Some("10".into())));
        // no id: close is a no-op by spec; the queries echo `i=0`
        assert_eq!(feed(&mut a, "p=close;"), Outcome::Close(None));
        assert_eq!(feed(&mut a, "p=?;"), Outcome::Query(None));
        // a close arrives with no payload to cap — even an oversized one
        assert_eq!(
            feed(&mut a, &format!("i=11:p=close;{}", "x".repeat(9000))),
            Outcome::Close(Some("11".into()))
        );
        // and a close does not leave a half-assembled id behind
        assert!(matches!(
            feed(&mut a, "i=12:d=0:p=title;chunk"),
            Outcome::Held
        ));
        assert_eq!(
            feed(&mut a, "i=12:p=close;"),
            Outcome::Close(Some("12".into()))
        );
        assert!(a.partial.is_empty());
    }

    #[test]
    fn an_assembly_that_never_finishes_cannot_rent_memory_forever() {
        let mut a = Assembler::default();
        for i in 0..(PENDING_MAX + 6) {
            feed(&mut a, &format!("i={i}:d=0:p=title;chunk {i}"));
        }
        // the oldest pending assemblies left: the map holds its cap
        assert_eq!(a.partial.len(), PENDING_MAX);
        assert!(!a.partial.contains_key("0"), "the oldest was dropped");
        assert!(a.partial.contains_key("21"), "the newest survived");
    }
}

#[cfg(test)]
mod wire_tests {
    use crate::term::Term;

    /// The wire path: the escape the shell prints reaches the dispatcher and
    /// comes out as a notification in the tab's list.
    #[test]
    fn the_escape_reaches_the_list() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b]99;;Hello world\x1b\\");
        t.feed(b"\x1b]99;i=1:p=body;body here\x1b\\");
        let trace = t.take_trace();
        for ev in &trace {
            println!("{ev:?}");
        }
        assert_eq!(t.notifications.len(), 2, "both notifications are in");
        assert!(t
            .notifications
            .entries()
            .any(|n| n.title.as_deref() == Some("Hello world")));
    }

    /// Phase two: `p=close` retires the entry, `p=alive` and `p=?` answer on
    /// the same tab's pty.
    #[test]
    fn the_lifecycle_answers_on_the_wire() {
        let mut t = Term::new(20, 4);
        t.feed(b"\x1b]99;i=1:p=title;first\x1b\\");
        t.feed(b"\x1b]99;i=2:p=title;second\x1b\\");
        t.feed(b"\x1b]99;i=1:p=close;\x1b\\");
        assert_eq!(t.notifications.len(), 1, "the closed one left");
        assert!(t
            .notifications
            .entries()
            .all(|n| n.id.as_deref() != Some("1")));
        // an absent id is ignored, by spec
        t.feed(b"\x1b]99;i=1:p=close;\x1b\\");
        assert_eq!(t.notifications.len(), 1);

        t.feed(b"\x1b]99;i=me:p=alive;\x1b\\");
        assert!(t.child_out.ends_with(b"\x1b]99;i=me:p=alive;2\x1b\\"));

        t.feed(b"\x1b]99;p=?;\x1b\\");
        assert!(t
            .child_out
            .ends_with(b"\x1b]99;i=0:p=?;p=title,body:o=always\x1b\\"));
    }
}
