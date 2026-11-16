//! An X extension error must not bring the app down.
//!
//! Linux only: the hook lives in `winit::platform::x11`, which does not exist on
//! another target — one of the reasons the seam is named after the system and not
//! after the unix family.
//!
//! `winit` stores the last X error of the connection and `expect()`s it on the
//! next call that flushes (`flush_requests`). An error that is **not ours** — the
//! Mesa WSI leaves `BadDrawable` pending while creating and destroying pixmaps to
//! present — kills the process on the next window property (that is how
//! `sandbox tortty` exited with 134 in `set_window_title`; see `docs/BACKLOG.md`, B5).
//! GLFW (kitty) records the error and moves on; here the winit public hook does
//! the same, without touching the X handler from outside.
//!
//! What it swallows is **deliberately narrow**: only drawing error codes
//! (`BadDrawable`/`BadWindow`/`BadMatch`) and only on **extension** requests
//! (`request_code >= 128`), which is where the WSI comes from (Present/DRI3). A
//! core X request is still a real error and `winit` reports it.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

/// Xlib's `XErrorEvent`, field by field (X11/Xlib.h). `winit` hands the raw
/// pointer to the hook; reading the struct is the only way to know what failed.
#[repr(C)]
struct XErrorEvent {
    kind: i32,
    display: *mut c_void,
    resource_id: u64,
    serial: u64,
    error_code: u8,
    request_code: u8,
    minor_code: u8,
}

const BAD_WINDOW: u8 = 3;
const BAD_MATCH: u8 = 8;
const BAD_DRAWABLE: u8 = 9;
const BAD_ACCESS: u8 = 10;

/// First extension request (core ones go up to 127).
const MIN_EXTENT: u8 = 128;

/// A drawing error coming from the WSI is no reason to shut the terminal down.
///
/// Measured on `sandbox` without a GPU (`docs/DEBUG.md`): the WSI leaves `BadDrawable`
/// on `Present` **and** `BadAccess` on `DRI3Open` — the second one is what killed
/// it, because the previous policy only looked at extension requests for the
/// drawing codes. A drawing code counts on any request (the WSI also frees a
/// pixmap through a core request); `BadAccess` only on extensions, which is where
/// GPU permission shows up.
fn swallow(error_code: u8, request_code: u8) -> bool {
    match error_code {
        BAD_WINDOW | BAD_MATCH | BAD_DRAWABLE => true,
        BAD_ACCESS => request_code >= MIN_EXTENT,
        _ => false,
    }
}

static COUNTS: AtomicU32 = AtomicU32::new(0);

#[allow(unsafe_code)] // reading the raw `XErrorEvent` is the winit hook contract
fn decide(_display: *mut c_void, event: *mut c_void) -> bool {
    if event.is_null() {
        return false;
    }
    // SAFETY: winit passes a valid Xlib `XErrorEvent`, which has exactly this
    // layout (X11/Xlib.h); we only read it, never store the pointer.
    let e = unsafe { &*(event.cast::<XErrorEvent>()) };
    if std::env::var_os("TORTTY_XERR").is_some() {
        eprintln!(
            "XERR code={} request={} minor={} resource=0x{:x} swallow={}",
            e.error_code,
            e.request_code,
            e.minor_code,
            e.resource_id,
            swallow(e.error_code, e.request_code)
        );
    }
    if !swallow(e.error_code, e.request_code) {
        return false;
    }
    let n = COUNTS.fetch_add(1, Ordering::Relaxed);
    // one line for the first three and then sparse: a WSI error usually comes
    // repeated, and the log cannot become the problem
    if n < 3 || n.is_multiple_of(1000) {
        eprintln!(
            "tortty: X extension error ignored (code {}, request {}), occurrence {} \
             — GPU WSI; see docs/DEBUG.md",
            e.error_code,
            e.request_code,
            n + 1
        );
    }
    true
}

/// Installs the hook. Safe to call on any backend (Wayland never invokes it).
pub fn install() {
    winit::platform::x11::register_xlib_error_hook(Box::new(decide));
}

#[cfg(test)]
mod tests {
    use super::swallow;

    #[test]
    fn wsi_error_is_swallowed() {
        // Present (149) and DRI3 (130) are the two WSI doors; measured on sandbox
        assert!(swallow(9, 149), "BadDrawable on Present");
        assert!(
            swallow(10, 130),
            "BadAccess on DRI3Open: the one that killed sandbox"
        );
        assert!(swallow(3, 149));
        assert!(swallow(8, 130));
        // the WSI also frees a pixmap through a core request
        assert!(swallow(9, 54));
    }

    #[test]
    fn bad_access_outside_an_extension_is_not_swallowed() {
        // BadAccess is permission: on a core request it is still an error
        assert!(!swallow(10, 20));
        assert!(!swallow(10, 33));
    }

    #[test]
    fn other_error_codes_are_not_swallowed() {
        // BadAtom, BadValue, BadGC: neither drawing nor GPU permission
        assert!(!swallow(5, 149));
        assert!(!swallow(2, 149));
        assert!(!swallow(13, 149));
    }
}
