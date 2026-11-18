//! The event log: one line per event, four fixed columns.
//!
//! `--debug-log FILE` — or the socket's `log` verb — opens it; the rest of the
//! app calls [`line`]. The shape is the same everywhere, so `cut`, `awk` and
//! `grep` work without a parser:
//!
//! ```text
//! 2026-11-17 22:41:03.512  info  sync    tab 2 closed after 2.2 ms — feeds 3, 264 B
//! │                       │     │       └ the message, free text
//! │                       │     └ the area: one word, never carrying a space
//! │                       └ err | warn | info
//! └ local time, ISO 8601 date + HH:MM:SS.mmm (23 columns)
//! ```
//!
//! The hour is **local** because the person reading the line compares it with
//! their own clock; the prologue carries the epoch and the offset, and those are
//! what turn local time back into an instant when two logs have to be matched.
//! [`crate::platform::utc_offset_secs`] is the only part of this that knows a
//! system: the core has no clock, and neither does the rest of the app.
//!
//! There is no level filter on purpose: the log only exists when someone asked
//! for it (`--debug-log`), so hiding lines from it would hide them from the one
//! person who wanted them.

use std::sync::Mutex;

/// How much the line should matter to whoever reads the file. `Err` is something
/// that cost the user a feature (the window did not open, the font did not
/// switch); `Warn` is something that went on with a wound (the config was kept,
/// the socket is open to the machine); `Info` is everything that just happened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Err,
    Warn,
    Info,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Self::Err => "err",
            Self::Warn => "warn",
            Self::Info => "info",
        }
    }
}

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Opens the log at `path` (append: two runs share the file, and the prologue
/// says where one ends and the next begins), or closes it on `off`/empty.
pub fn set(path: &str) {
    let mut slot = LOG.lock().unwrap();
    if path.is_empty() || path == "off" {
        *slot = None;
        return;
    }
    *slot = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok();
}

/// Is a log open? The prologue is written only when there is one to write into.
pub fn on() -> bool {
    LOG.lock().is_ok_and(|slot| slot.is_some())
}

/// Writes one line. `area` is one word — `app`, `socket`, `config`, `font`,
/// `gpu`, `toast`, `sync`, `view`, `image`, `dump`, `prof` — and a tab-scoped
/// event says its tab in the message, because the tab is a subject and not a
/// kind of event.
pub fn line(level: Level, area: &str, msg: &str) {
    if let Ok(mut slot) = LOG.lock() {
        if let Some(f) = slot.as_mut() {
            use std::io::Write as _;
            let _ = writeln!(f, "{}", assemble(&stamp(), level, area, msg));
        }
    }
}

/// The four columns, assembled apart from the clock so a test can pin the shape.
fn assemble(stamp: &str, level: Level, area: &str, msg: &str) -> String {
    format!("{stamp}  {:<4}  {area:<8} {msg}", level.name())
}

/// `2026-11-17 22:41:03.512` in local time: 23 columns, sortable as text.
fn stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = i64::try_from(now.as_secs()).unwrap_or(0);
    let ms = now.subsec_millis();
    let offset = i64::from(crate::platform::utc_offset_secs().unwrap_or(0));
    let local = secs + offset;
    let (y, m, d) = civil(local.div_euclid(86_400));
    let sod = local.rem_euclid(86_400);
    let (hh, mm, ss) = (sod / 3600, (sod / 60) % 60, sod % 60);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}.{ms:03}")
}

/// Days since 1970-01-01 → `(year, month, day)`, proleptic Gregorian, no table
/// and no dependency: the algorithm is Howard Hinnant's `civil_from_days`.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// Milliseconds since the epoch: the number the prologue states once, so a local
/// stamp can be turned back into an instant without guessing the timezone.
pub fn epoch_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

/// The offset of the column's hour from UTC, as `-03:00` — the other half of the
/// prologue, and the one that makes the local stamps unambiguous.
pub fn tz() -> String {
    let secs = crate::platform::utc_offset_secs().unwrap_or(0);
    let (sign, secs) = if secs < 0 { ('-', -secs) } else { ('+', secs) };
    format!("{sign}{:02}:{:02}", secs / 3600, (secs / 60) % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_civil_date_of_a_known_day() {
        // a table of days since 1970-01-01, including both sides of a leap day
        // and the century rule 2000 (leap) vs 1900 (not, but out of range here)
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(19_782), (2024, 2, 29));
        assert_eq!(civil(-1), (1969, 12, 31));
    }

    #[test]
    fn a_line_puts_the_message_in_a_fixed_column() {
        let stamp = "2026-11-17 22:41:03.512";
        let line = assemble(stamp, Level::Info, "sync", "tab 2 open");
        assert_eq!(line, "2026-11-17 22:41:03.512  info  sync     tab 2 open");
        // the prefix is the same width whatever the level and the area are, so
        // `cut -c41-` takes the message out of any line
        for level in [Level::Err, Level::Warn, Level::Info] {
            for area in ["app", "config", "prof"] {
                let l = assemble(stamp, level, area, "x");
                assert_eq!(
                    &l[..40],
                    &format!("{stamp}  {:<4}  {area:<8} ", level.name())[..40]
                );
                assert_eq!(&l[40..], "x");
            }
        }
    }

    #[test]
    fn the_stamp_is_a_sortable_local_date() {
        let s = stamp();
        assert_eq!(s.len(), 23, "{s}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[19..20], ".");
        assert!(s[..4].chars().all(|c| c.is_ascii_digit()), "{s}");
        // the year is this one, unless the clock of the machine is off
        let year: i64 = s[..4].parse().unwrap();
        assert!((2024..2100).contains(&year), "{s}");
    }

    #[test]
    fn the_timezone_is_a_quarter_of_the_world() {
        let tz = tz();
        assert_eq!(tz.len(), 6, "{tz}");
        let sign = tz.as_bytes()[0];
        assert!(sign == b'+' || sign == b'-', "{tz}");
        let h: i32 = tz[1..3].parse().unwrap();
        let m: i32 = tz[4..6].parse().unwrap();
        assert!(h <= 14 && m < 60, "{tz}");
        // no real zone sits between -12 and +14, and none is off the quarter hour
        assert!((h * 60 + m) % 15 == 0, "{tz}");
    }
}
