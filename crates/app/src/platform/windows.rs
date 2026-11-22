//! Windows: the minimal implementation behind [`super`].
//!
//! Scope, decided with the owner on 2026-11-22: enough for the window to open, the
//! GPU to draw and `portable-pty` to run the shell through ConPTY. What is not
//! here — the debug socket as a named pipe, `/proc`-style child cwd, `SIGHUP`,
//! the civil time zone — comes back as `None`/`Err` **with the reason**, which is
//! the seam's rule: the app asks a capability and gets an answer, never a silent
//! stub that pretends. Everything in this file is unmeasured until it runs in the
//! Windows 10 VM; the run is what turns it into a fact.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// One environment variable as a path, empty treated as absent (Windows sets
/// `USERPROFILE` and friends but a launcher can hand over `""`).
fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

// ---------------------------------------------------------------- paths

/// `%USERPROFILE%`.
#[must_use]
pub fn home() -> Option<PathBuf> {
    env_path("USERPROFILE")
}

/// `%APPDATA%\tortty`: roaming configuration, which is what a config file is.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    env_path("APPDATA").map(|p| p.join("tortty"))
}

/// `%LOCALAPPDATA%\tortty`: state that is allowed to be lost and must not roam.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    env_path("LOCALAPPDATA").map(|p| p.join("tortty"))
}

/// `%TEMP%`: where the automatic socket name would be built. There is no socket
/// yet, so this only answers the shape the caller expects.
#[must_use]
pub fn runtime_dir() -> String {
    std::env::temp_dir().to_string_lossy().to_string()
}

/// Where a dump goes when `--dump-to` said nothing.
#[must_use]
pub fn dump_path() -> PathBuf {
    std::env::temp_dir().join("tortty-dump.txt")
}

/// The same rule as Linux: only a path carrying `tty-graphics-protocol` **inside**
/// the temporary directory may be deleted by the protocol, decided after
/// canonicalization so `..` cannot walk out of it.
#[must_use]
pub fn temp_delete_allowed(path: &Path) -> bool {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canon.to_string_lossy().contains("tty-graphics-protocol")
        && canon.starts_with(std::env::temp_dir())
}

// ---------------------------------------------------------------- the socket

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "the debug socket is not implemented on Windows yet (a named pipe, M-X): \
         it is Linux-only for now",
    )
}

/// One end of a debug connection. Windows has no unix socket; the equivalent is a
/// named pipe, which the minimal port does not have. The type exists so the app
/// compiles and so `--debug-socket` fails with the reason instead of silently
/// listening to nobody.
pub struct Conn(());

impl Conn {
    /// Always `Err`: there is no pipe to connect to.
    pub fn connect(path: impl AsRef<Path>) -> io::Result<Conn> {
        let _ = path.as_ref();
        Err(unsupported())
    }

    /// Always `Err`, for the same reason. The `&self` stays because the seam fixes
    /// the shape and the Linux side uses it; the body has nothing to read.
    #[allow(clippy::unused_self)]
    pub fn try_clone(&self) -> io::Result<Conn> {
        Err(unsupported())
    }

    /// Test-only on Linux; here it is the same refusal.
    #[cfg(test)]
    #[allow(clippy::unused_self)]
    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        let _ = timeout;
        Err(unsupported())
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let _ = buf;
        Err(unsupported())
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = buf;
        Err(unsupported())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The listener that does not listen: `bind` is where the app learns the truth.
pub struct Listener;

impl Listener {
    /// Refused with the reason, so the app logs it and keeps running without a
    /// socket. Refusing is also what keeps the promise the seam makes: no silent
    /// `#[cfg]` in the feature code, one honest answer here.
    pub fn bind(path: &Path) -> Result<Listener, String> {
        Err(format!(
            "{}: the debug socket is not implemented on Windows yet (a named pipe, M-X)",
            path.display()
        ))
    }

    /// Unreachable: [`Listener::bind`] never returns a `Listener`.
    #[allow(clippy::unused_self)]
    pub fn accept(&self) -> io::Result<Conn> {
        Err(unsupported())
    }
}

/// Does this system have a debug socket at all? The self test asks before
/// checking permissions of a socket that cannot exist.
#[must_use]
pub fn has_debug_socket() -> bool {
    false
}

/// Nothing to wake: there is no listener.
pub fn wake(path: &Path) {
    let _ = path;
}

/// Nothing to remove: there is no file.
pub fn remove_socket(path: &Path) {
    let _ = path;
}

/// With no socket there is no peer to check. A named pipe would carry an ACL
/// instead, which is the question this answers on Linux.
#[must_use]
pub fn same_user(conn: &Conn) -> bool {
    let _ = conn;
    true
}

/// No socket, so nothing is owner-only: the answer the self test needs to skip
/// the check rather than fail it.
#[must_use]
pub fn socket_is_owner_only(path: &Path) -> bool {
    let _ = path;
    false
}

// ---------------------------------------------------------------- the session lock

/// `LockFileEx` through the standard library: an exclusive, non-blocking lock that
/// goes away with the handle — the same contract as `flock(2)` on Linux, without
/// an `unsafe` block.
#[must_use]
pub fn try_lock(file: &File) -> bool {
    file.try_lock().is_ok()
}

// ---------------------------------------------------------------- the clock

/// `None`: the standard library hands out UTC only and Windows keeps the civil
/// offset behind `GetTimeZoneInformation`, which would need `windows-sys` for one
/// field of one log line. The log therefore stamps UTC on Windows and says so.
#[must_use]
pub fn utc_offset_secs() -> Option<i32> {
    None
}

// ---------------------------------------------------------------- processes

/// Windows has no `SIGHUP`: closing the pty runs `ClosePseudoConsole`, which is
/// what ends the child. A program's last words (`B48`) are a Unix idea and the
/// limit is recorded in `docs/LIMITATIONS.md`.
pub fn hangup(pid: u32) {
    let _ = pid;
}

/// There is no `/proc` to read. Windows answers this through
/// `NtQueryInformationProcess` (and the console API for its hosted processes),
/// which the minimal port does not implement: the title falls back to the program
/// name instead of the directory.
#[must_use]
pub fn child_cwd(pid: u32) -> Option<PathBuf> {
    let _ = pid;
    None
}

/// No signals to install: the dump is asked for through the window or the socket,
/// and a close request already arrives as a `winit` event (`WM_CLOSE`).
pub fn install_triggers() {}

/// The triggers, read once each: `(dump, quit)`. Both are always false because
/// nothing on Windows sets them yet.
#[must_use]
pub fn take_triggers() -> (bool, bool) {
    (false, false)
}

/// No `SIGPIPE` on Windows: a write to a closed pipe fails at the call, and the
/// process is not killed. Nothing to reset.
pub fn default_sigpipe() {}

// ---------------------------------------------------------------- window and desktop

/// Windows honours `set_outer_position`, unlike Wayland, so the saved geometry is
/// requested and written (the `J6` rule is about the window manager that refuses).
#[must_use]
pub fn window_accepts_position() -> bool {
    true
}

/// `WM_CLASS` is an X11 idea and the next two calls are the X11 way of answering
/// it; `winit` sets the Windows class from the executable name, so there is
/// nothing to add here.
#[must_use]
pub fn name_window(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    attrs
}

/// No X11 window id: 0, the value the probes already read as "no id to capture".
#[must_use]
pub fn window_id(window: &winit::window::Window) -> u64 {
    let _ = window;
    0
}

/// Opens a URI with the shell handler. The empty title argument is not optional:
/// `start` reads the first quoted argument as the window title, so without it a
/// path with spaces opens a console titled with the path.
pub fn open_uri(uri: &str) -> io::Result<()> {
    std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(uri)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// `VISUAL` beats `EDITOR`, and `notepad` is what every Windows has.
#[must_use]
pub fn editor() -> String {
    std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "notepad".to_string())
}

/// One clipboard, no PRIMARY: the middle-button selection does not exist here.
#[must_use]
pub fn has_primary() -> bool {
    false
}

/// Never called: [`has_primary`] answers false and the app does not ask.
pub fn primary_get(board: &mut arboard::Clipboard) -> Option<String> {
    let _ = board;
    None
}

/// Never called, for the same reason.
pub fn primary_set(board: &mut arboard::Clipboard, text: &str) {
    let _ = (board, text);
}

/// A path quoted for the shell the child runs, which is `cmd.exe` by default.
///
/// The rule for what `CreateProcess` parses: wrap in double quotes, backslash-escape
/// the quotes inside, and double a trailing backslash so it cannot escape the
/// closing quote. `cmd.exe` metacharacters (`%`, `^`, `&`, `|`) are **not** escaped
/// — a documented limit: the child reads its command line through `cmd`, and a
/// filename carrying one of those is out of reach until the shell invocation is
/// rewritten.
#[must_use]
pub fn shell_quote(path: &str) -> String {
    fn safe(c: char) -> bool {
        c.is_ascii_alphanumeric() || "\\/._-+=@,:()".contains(c)
    }
    if !path.is_empty() && path.chars().all(safe) {
        return path.to_string();
    }
    let mut out = String::with_capacity(path.len() + 2);
    out.push('"');
    for c in path.chars() {
        match c {
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    // a trailing backslash would escape the closing quote
    if out.ends_with('\\') {
        out.push('\\');
    }
    out.push('"');
    out
}

/// `%COMSPEC%` (normally `cmd.exe`). PowerShell is a configuration choice, not the
/// default a terminal may assume.
#[must_use]
pub fn default_shell() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
}

/// There is no X error guard: the Xlib protocol does not exist here.
pub fn install_error_guard() {}
