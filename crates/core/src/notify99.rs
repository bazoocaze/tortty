//! `OSC 99` — kitty's identified desktop notification, phase one of `B72`:
//! parse, assemble, notify. No lifecycle yet — `c=1` close events, `a=report`
//! and the `p=?` support query need a click channel and a reply, and stay
//! silent here (see `docs/BACKLOG.md`).
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
    /// The escape said something this terminal gives no surface to (`p=icon`,
    /// `p=buttons`, a close event, the support query) — ignored, by spec.
    Ignored,
    /// Refused: the payload broke a cap or the escape has no payload section.
    Refused(&'static str),
}

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
        let mut id: Option<String> = None;
        let mut done = true;
        let mut encoded = false;
        let mut kind: Option<Piece> = None;
        let mut saw_p = false;
        for pair in String::from_utf8_lossy(meta).split(':') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            match (key, value) {
                ("i", v) => id = Some(v.chars().take(crate::notif::FIELD_MAX).collect()),
                ("d", v) => done = v != "0",
                ("e", "1") => encoded = true,
                // the quickstart of the spec (`99;;Hello world`) has no `p`:
                // absent means title. A `p` the terminal gives no meaning to
                // (icon, buttons, close, alive, ?) stays ignored.
                ("p", v) => {
                    saw_p = true;
                    kind = piece(v);
                },
                // `f`, `t`, `n`, `g`, `s`, `a`, `c`, `w`, `o`, `u` — filtering
                // and lifecycle keys with no surface in phase one; unknown
                // keys too. The spec says: ignore and keep going.
                _ => {},
            }
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
        assert!(matches!(feed(&mut a, "i=8:p=close;"), Outcome::Ignored));
        assert!(matches!(feed(&mut a, "i=8:p=?;"), Outcome::Ignored));
        // unknown keys do not stop the payload
        assert!(matches!(
            feed(&mut a, "z=9:w=1000:p=title;kept"),
            Outcome::Notify(_)
        ));
        // no payload section at all (the two semicolons are the wire's law)
        assert!(matches!(feed(&mut a, "p=title"), Outcome::Refused(_)));
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
}
