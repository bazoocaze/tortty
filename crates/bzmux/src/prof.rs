//! Cost counters per stage (the `PYTERM_PROF` of the bzmux Python).
//!
//! Turned on by `--prof` or by the socket verb `prof`. Atomics because the pty
//! cost is measured on the reader thread and the parse/draw one on the UI thread.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub static ON: AtomicBool = AtomicBool::new(false);
pub static PARSE_NS: AtomicU64 = AtomicU64::new(0);
pub static PARSE_CALLS: AtomicU64 = AtomicU64::new(0);
pub static PARSE_BYTES: AtomicU64 = AtomicU64::new(0);
pub static DRAW_NS: AtomicU64 = AtomicU64::new(0);
pub static DRAW_CALLS: AtomicU64 = AtomicU64::new(0);
pub static FRAMES: AtomicU64 = AtomicU64::new(0);
pub static PTY_NS: AtomicU64 = AtomicU64::new(0);
pub static PTY_READS: AtomicU64 = AtomicU64::new(0);
pub static PTY_BYTES: AtomicU64 = AtomicU64::new(0);
pub static WRITE_CALLS: AtomicU64 = AtomicU64::new(0);
pub static WRITE_BYTES: AtomicU64 = AtomicU64::new(0);
/// Writes that came back without accepting everything (`EAGAIN`/short) and were repeated.
pub static EAGAIN: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

pub fn set_on(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

#[inline]
pub fn add_parse(ns: u64, bytes: usize) {
    PARSE_NS.fetch_add(ns, Ordering::Relaxed);
    PARSE_CALLS.fetch_add(1, Ordering::Relaxed);
    PARSE_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

#[inline]
pub fn add_draw(ns: u64) {
    DRAW_NS.fetch_add(ns, Ordering::Relaxed);
    DRAW_CALLS.fetch_add(1, Ordering::Relaxed);
    FRAMES.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn add_write(bytes: usize) {
    WRITE_CALLS.fetch_add(1, Ordering::Relaxed);
    WRITE_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

#[inline]
pub fn add_eagain() {
    EAGAIN.fetch_add(1, Ordering::Relaxed);
}

#[inline]
pub fn add_pty(ns: u64, bytes: usize) {
    PTY_NS.fetch_add(ns, Ordering::Relaxed);
    PTY_READS.fetch_add(1, Ordering::Relaxed);
    PTY_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
}

fn ms(ns: u64, call: u64) -> f64 {
    if call == 0 {
        0.0
    } else {
        ns as f64 / call as f64 / 1e6
    }
}

/// Line in the same format as Python (`prof: draw Nx X.XXms/call (...) | pty ...`),
/// so the same `grep` works in both terminals.
pub fn summary() -> String {
    let dc = DRAW_CALLS.load(Ordering::Relaxed);
    let pc = PARSE_CALLS.load(Ordering::Relaxed);
    let rc = PTY_READS.load(Ordering::Relaxed);
    let dn = DRAW_NS.load(Ordering::Relaxed);
    let pn = PARSE_NS.load(Ordering::Relaxed);
    let rn = PTY_NS.load(Ordering::Relaxed);
    format!(
        "draw {}x {:.2}ms/call ({:.0}ms) | parse {}x {:.2}ms/call ({:.0}ms, {:.1} MB) | pty {}x {:.2}ms/call ({:.0}ms, {:.1} MB)",
        dc,
        ms(dn, dc),
        dn as f64 / 1e6,
        pc,
        ms(pn, pc),
        pn as f64 / 1e6,
        PARSE_BYTES.load(Ordering::Relaxed) as f64 / 1e6,
        rc,
        ms(rn, rc),
        rn as f64 / 1e6,
        PTY_BYTES.load(Ordering::Relaxed) as f64 / 1e6,
    )
}

pub fn reset() {
    for c in [
        &PARSE_NS,
        &PARSE_CALLS,
        &PARSE_BYTES,
        &DRAW_NS,
        &DRAW_CALLS,
        &FRAMES,
        &PTY_NS,
        &PTY_READS,
        &PTY_BYTES,
    ] {
        c.store(0, Ordering::Relaxed);
    }
}

/// Fields of the `prof` verb (the `benchrun` reads `draw` and `pty` from here).
pub fn fields() -> serde_json::Value {
    let dc = DRAW_CALLS.load(Ordering::Relaxed);
    let pc = PARSE_CALLS.load(Ordering::Relaxed);
    let rc = PTY_READS.load(Ordering::Relaxed);
    serde_json::json!({
        "on": enabled(),
        "draw_calls": dc,
        "draw_ms_call": ms(DRAW_NS.load(Ordering::Relaxed), dc),
        "parse_calls": pc,
        "parse_ms_call": ms(PARSE_NS.load(Ordering::Relaxed), pc),
        "parse_mb": PARSE_BYTES.load(Ordering::Relaxed) as f64 / 1e6,
        "pty_calls": rc,
        "pty_ms_call": ms(PTY_NS.load(Ordering::Relaxed), rc),
        "pty_mb": PTY_BYTES.load(Ordering::Relaxed) as f64 / 1e6,
        "writes": WRITE_CALLS.load(Ordering::Relaxed),
        "write_kb": WRITE_BYTES.load(Ordering::Relaxed) as f64 / 1e3,
        "eagain": EAGAIN.load(Ordering::Relaxed),
        "text": summary(),
    })
}
