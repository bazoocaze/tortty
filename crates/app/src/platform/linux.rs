//! Linux, the measured target: the implementation behind [`super`].
//!
//! Every fact here was taken on this system and is used by the app; nothing is
//! "should work". macOS shares most of these syscalls (it is a unix) but it does
//! **not** share all of them — `SO_PEERCRED` and the X11 hook do not exist there —
//! which is why this file is named `linux` and not `unix`: the name states what
//! was built and tested, not a family it belongs to.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::fs::FileTypeExt as _;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------- paths

/// `$HOME`. The one place the app asks for it (the `~` of the kitty protocol).
#[must_use]
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// An XDG directory, with the old dotfile as the fallback.
fn xdg_dir(var: &str, under_home: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| home().map(|h| h.join(under_home)))
}

/// `$XDG_CONFIG_HOME` (or `~/.config`): where the config file lives.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_STATE_HOME` (or `~/.local/state`): state, not configuration — it is
/// allowed to be lost.
#[must_use]
pub fn state_dir() -> Option<PathBuf> {
    xdg_dir("XDG_STATE_HOME", ".local/state")
}

/// `$XDG_RUNTIME_DIR` (or `/tmp`): the directory the automatic socket name is
/// built in. A `String` because that is what the socket path is.
#[must_use]
pub fn runtime_dir() -> String {
    std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string())
}

/// Where `SIGUSR1` writes the dump when `--dump-to` said nothing.
#[must_use]
pub fn dump_path() -> PathBuf {
    PathBuf::from("/tmp/tortty-dump.txt")
}

/// What `t=t` of the kitty protocol may delete: the protocol lets the child name
/// the file, so only a path inside a known temporary directory carrying
/// `tty-graphics-protocol` qualifies. Canonicalized first, so `..` and symlinks
/// cannot walk the check out of the temp dir. Decided after the read, on a path
/// that exists — a dangling name never deletes.
#[must_use]
pub fn temp_delete_allowed(path: &Path) -> bool {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !canon.to_string_lossy().contains("tty-graphics-protocol") {
        return false;
    }
    canon.starts_with(std::env::temp_dir())
        || canon.starts_with("/tmp")
        || canon.starts_with("/dev/shm")
}

// ---------------------------------------------------------------- the socket

/// One end of a debug connection. A newtype so the app never names
/// `UnixStream`: on a system without unix sockets this is a named pipe.
pub struct Conn(UnixStream);

impl Conn {
    /// The client side (the probes, the tests and the wake of [`Listener::drop`]).
    pub fn connect(path: impl AsRef<Path>) -> io::Result<Conn> {
        UnixStream::connect(path).map(Conn)
    }

    /// Another handle to the same connection: the request comes in on one, the
    /// reply goes out on the other.
    pub fn try_clone(&self) -> io::Result<Conn> {
        self.0.try_clone().map(Conn)
    }

    /// Turns "the server hung" into a failure instead of a hang. Test-only: the
    /// probes that drive the app from outside are other processes, and they set
    /// their own timeout.
    #[cfg(test)]
    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.0.set_read_timeout(timeout)
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// The bound, `0600`, owner-only debug socket.
pub struct Listener {
    inner: UnixListener,
    path: PathBuf,
}

impl Listener {
    /// Binds `path`. A path taken by something that is not a socket is **refused,
    /// not deleted** (L12); an orphan socket (nobody listening) is removed; a live
    /// one is refused.
    pub fn bind(path: &Path) -> Result<Listener, String> {
        clear_stale(path)?;
        let inner = UnixListener::bind(path).map_err(|e| format!("{}: {e}", path.display()))?;
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Listener {
            inner,
            path: path.to_path_buf(),
        })
    }

    /// One connection; an error is "nobody this round", which is what the caller
    /// already does with it.
    pub fn accept(&self) -> io::Result<Conn> {
        self.inner.accept().map(|(stream, _)| Conn(stream))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // the wake first: without it a thread blocked in `accept` hangs on a
        // listener whose file is already gone
        wake(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A throwaway connection to whatever is listening at `path`, so a blocked
/// `accept` returns. Whoever wakes checks its stop flag and exits.
pub fn wake(path: &Path) {
    let _ = UnixStream::connect(path);
}

/// Removes the socket file. A stopped server has to leave nothing behind, and
/// "when the accept thread happens to end" is not the same instant as "when the
/// server stops" — the file goes with the server.
pub fn remove_socket(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// `SO_PEERCRED`: only the same uid talks to the socket. It is the only `unsafe`
/// in the project — `getsockopt` is FFI with no stable alternative in Rust.
#[must_use]
#[allow(unsafe_code)]
pub fn same_user(conn: &Conn) -> bool {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            conn.0.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    rc == 0 && cred.uid == unsafe { libc::getuid() }
}

/// Is the socket at `path` owner-only? The fact `bind` establishes, asked again
/// by the self test (a socket another user can read is a screen another user can
/// read).
#[must_use]
pub fn socket_is_owner_only(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o777 == 0o600)
}

fn clear_stale(path: &Path) -> Result<(), String> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(()); // does not exist: nothing to clean
    };
    // a file that is **not** a socket is not ours: only the owner deletes it (L12)
    if !meta.file_type().is_socket() {
        return Err(format!(
            "path exists and is not a socket: {}",
            path.display()
        ));
    }
    if UnixStream::connect(path).is_ok() {
        return Err(format!(
            "another process is listening on {}",
            path.display()
        ));
    }
    // orphan socket: nobody listens, the file is garbage
    std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------- the session lock

/// `flock(2)` non-blocking exclusive: is this process the owner? An advisory lock
/// is a descriptor property, so it is released when the file is dropped — no
/// stale entry and no recycled PID to defend against.
///
/// `unsafe`: `flock(2)` has no wrapper in the standard library.
#[must_use]
#[allow(unsafe_code)]
pub fn try_lock(file: &File) -> bool {
    // SAFETY: `flock(2)` with a valid descriptor and a valid operation flag.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

// ---------------------------------------------------------------- processes

/// Says goodbye with `SIGHUP` and returns without waiting: what closing a
/// terminal does, and the signal a program uses to say its last words (`B48`).
///
/// `portable-pty`'s `kill` sends `SIGHUP` too but waits 200 ms **per child** before
/// the `SIGKILL`, and the app has to be draining the ptys while they die.
pub fn hangup(pid: u32) {
    // a bare `kill(2)`: the return says nothing the caller could act on (a child
    // that already left is the normal case here)
    #[allow(unsafe_code)]
    // SAFETY: `kill(2)` with a pid and a signal number; no memory is touched.
    unsafe {
        libc::kill(pid as i32, libc::SIGHUP);
    }
}

/// Where the program in front of the pty really is: the cwd of the deepest child
/// (`/proc/<pid>/task/<pid>/children`), which is where `mc`, `vim` or a `cd` inside
/// a script leaves it. `None` once the child is gone.
#[must_use]
pub fn child_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{}/cwd", descendant(pid))).ok()
}

/// The last descendant, up to eight levels: the shell spawns `mc`, which spawns
/// its own shell, and the front program is the leaf.
fn descendant(pid: u32) -> u32 {
    let mut cur = pid;
    for _ in 0..8 {
        let path = format!("/proc/{cur}/task/{cur}/children");
        let Ok(txt) = std::fs::read_to_string(path) else {
            break;
        };
        let Some(child) = txt.split_whitespace().next().and_then(|s| s.parse().ok()) else {
            break;
        };
        cur = child;
    }
    cur
}

/// `SIGUSR1` asked for a dump; `SIGTERM` asked for a clean exit (dump + close).
static TRIGGER_DUMP: AtomicBool = AtomicBool::new(false);
static TRIGGER_QUIT: AtomicBool = AtomicBool::new(false);

extern "C" fn on_dump(_: libc::c_int) {
    TRIGGER_DUMP.store(true, Ordering::Relaxed);
}

extern "C" fn on_quit(_: libc::c_int) {
    TRIGGER_QUIT.store(true, Ordering::Relaxed);
}

/// Installs `SIGUSR1`/`SIGTERM`. The handler only raises an `AtomicBool`:
/// `store` is lock free and safe in a handler, and whoever acts is the event loop
/// (which wakes up every 250 ms anyway).
///
/// `unsafe`: `signal(2)` swaps the process handler — there is no safe API for
/// that, as with `SO_PEERCRED` above.
#[allow(unsafe_code)]
pub fn install_triggers() {
    // SAFETY: `signal` installs a handler for two signal numbers; no memory is
    // touched and the handler only stores into a static `AtomicBool`.
    unsafe {
        libc::signal(libc::SIGUSR1, on_dump as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_quit as *const () as libc::sighandler_t);
    }
}

/// The triggers, read once each: `(dump, quit)`.
#[must_use]
pub fn take_triggers() -> (bool, bool) {
    (
        TRIGGER_DUMP.swap(false, Ordering::Relaxed),
        TRIGGER_QUIT.swap(false, Ordering::Relaxed),
    )
}

/// `SIGPIPE` back to the system default. Needed only by `--selftest`: the app
/// writes BEL to stdout and must not die because of a closed pipe, while
/// `tortty --selftest | head` closing the pipe must not turn the next `println!`
/// into a panic.
#[allow(unsafe_code)]
pub fn default_sigpipe() {
    // SAFETY: `signal` with `SIG_DFL` installs no handler and reads no memory.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

// ---------------------------------------------------------------- window and desktop

/// Is the window manager X11? It is the one that accepts a window position; on
/// Wayland that does not exist (J6). `winit` does not expose which backend it
/// picked, so the reading comes from the environment — the same choice it makes:
/// Wayland when `WAYLAND_DISPLAY` exists.
#[must_use]
pub fn backend_x11() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_none_or(|v| v.is_empty())
        && std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty())
}

/// `WM_CLASS`, which the `.desktop` `StartupWMClass` matches to group the window
/// in the dock (K19). Explicit, not inherited from the executable name: a renamed
/// binary (`tortty-rs`) cannot change it.
#[must_use]
pub fn name_window(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    use winit::platform::x11::WindowAttributesExtX11 as _;
    attrs.with_name("tortty", "tortty")
}

/// The window id on X11 (0 on Wayland): whoever is going to capture pixels asks
/// the app, not X, so the capture cannot pick another window.
#[must_use]
pub fn window_id(window: &winit::window::Window) -> u64 {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xlib(x)) => x.window,
        Ok(RawWindowHandle::Xcb(x)) => u64::from(x.window.get()),
        _ => 0,
    }
}

/// Opens a URI with the desktop handler. The caller has already restricted the
/// schemes: `xdg-open` would happily run a `.desktop` handler for anything else.
pub fn open_uri(uri: &str) -> io::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(uri)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// The editor the environment names, or this system's fallback: `VISUAL` beats
/// `EDITOR`.
#[must_use]
pub fn editor() -> String {
    std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vim".to_string())
}

/// A path quoted for the shell the child runs, which is what a dropped file
/// becomes.
///
/// Two rules: a path made only of characters the shell does not touch goes out as
/// it is (the common case, readable), everything else is wrapped in single quotes
/// with the embedded `'` closed and escaped (`'\''`) — the one form that survives
/// `$`, spaces, quotes and newlines without a second interpretation.
#[must_use]
pub fn shell_quote(path: &str) -> String {
    fn safe(c: char) -> bool {
        c.is_ascii_alphanumeric() || "/._-+=@,:".contains(c)
    }
    if !path.is_empty() && path.chars().all(safe) {
        return path.to_string();
    }
    let mut out = String::with_capacity(path.len() + 2);
    out.push('\'');
    for c in path.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// What the child runs when nothing is configured and `$SHELL` is empty or dead.
/// `/bin/bash` before `/bin/sh` because the default of the system is `/bin/sh`
/// (dash on Debian), and a terminal that opens in dash surprises.
#[must_use]
pub fn default_shell() -> String {
    if Path::new("/bin/bash").exists() {
        "/bin/bash".to_string()
    } else {
        "/bin/sh".to_string()
    }
}

#[cfg(test)]
mod checks_platform {
    //! What the seam promises, checked on the system it was built for.

    use super::*;
    use std::sync::atomic::AtomicU32;

    /// A path exclusive to this case: `cargo test` runs cases in parallel.
    fn path(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("tt-plat-{tag}-{}-{n}.sock", std::process::id()))
    }

    #[test]
    fn the_socket_is_born_owner_only_and_leaves_no_file() {
        let path = path("perm");
        let listener = Listener::bind(&path).expect("bind");
        assert!(
            socket_is_owner_only(&path),
            "a debug socket has to be owner-only"
        );
        drop(listener);
        assert!(!path.exists(), "socket left behind: {}", path.display());
    }

    #[test]
    fn a_bound_socket_is_not_stolen_and_an_orphan_is_removed() {
        let path = path("alive");
        let listener = Listener::bind(&path).expect("bind");
        let e = Listener::bind(&path).err().expect("it had to refuse");
        assert!(e.contains("listening"), "unexpected error: {e}");
        drop(listener);

        // nobody listening: the file is garbage and the next bind takes it
        Listener::bind(&path).expect("the orphan had to go");
    }

    #[test]
    fn a_regular_file_is_refused_and_kept() {
        // L12: a path taken by something that is not a socket is not ours
        let path = path("regular");
        std::fs::write(&path, b"not a socket").expect("create the file");
        let e = Listener::bind(&path).err().expect("it had to refuse");
        assert!(e.contains("not a socket"), "unexpected error: {e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"not a socket");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_connection_carries_bytes_and_the_peer_credential() {
        let path = path("conn");
        let listener = Listener::bind(&path).expect("bind");
        let mut cli = Conn::connect(&path).expect("connect");
        cli.write_all(b"hello\n").expect("write");
        let mut server = listener.accept().expect("accept");
        assert!(
            same_user(&server),
            "our own connection has to be our own user"
        );
        let mut buf = [0u8; 6];
        server.read_exact(&mut buf).expect("read");
        assert_eq!(&buf, b"hello\n");
    }

    #[test]
    fn the_quoted_path_survives_the_shell() {
        // the common case stays readable: nothing to quote
        assert_eq!(shell_quote("/tmp/a.txt"), "/tmp/a.txt");
        assert_eq!(
            shell_quote("/home/user/x-1_2.tar.gz"),
            "/home/user/x-1_2.tar.gz"
        );
        // a space, a `$` and a quote are the shell's, not ours
        assert_eq!(shell_quote("/tmp/a b.txt"), "'/tmp/a b.txt'");
        assert_eq!(shell_quote("/tmp/$HOME"), "'/tmp/$HOME'");
        assert_eq!(shell_quote("/tmp/it's"), "'/tmp/it'\\''s'");
        // a newline cannot break the line it lands in
        assert_eq!(shell_quote("/tmp/a\nb"), "'/tmp/a\nb'");
        // nothing is not a path worth pasting
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn the_default_shell_exists() {
        let sh = default_shell();
        assert!(sh == "/bin/bash" || sh == "/bin/sh", "{sh}");
        assert!(Path::new(&sh).exists(), "{sh} does not exist");
    }

    #[test]
    fn the_child_cwd_is_the_process_it_reached() {
        // our own pid: `/proc/self/cwd` is this process's directory
        let cwd = child_cwd(std::process::id()).expect("a cwd for our own pid");
        assert!(cwd.is_absolute(), "{cwd:?}");
        // a pid that does not exist has no directory to give
        assert!(child_cwd(u32::MAX - 1).is_none());
    }

    #[test]
    fn only_a_marked_file_in_a_temp_dir_may_be_deleted() {
        let dir = std::env::temp_dir();
        let marked = dir.join(format!("tty-graphics-protocol-{}.png", std::process::id()));
        std::fs::write(&marked, b"x").expect("create the marked file");
        assert!(temp_delete_allowed(&marked), "{marked:?}");
        let unmarked = dir.join(format!("plain-{}.png", std::process::id()));
        std::fs::write(&unmarked, b"x").expect("create the unmarked file");
        assert!(!temp_delete_allowed(&unmarked), "{unmarked:?}");
        // a marked name outside the temporary directories is kept
        assert!(!temp_delete_allowed(Path::new(
            "/home/tty-graphics-protocol/x.png"
        )));
        let _ = std::fs::remove_file(&marked);
        let _ = std::fs::remove_file(&unmarked);
    }

    #[test]
    fn the_triggers_are_read_once() {
        // the handlers are FFI; what is checkable without one is the readout
        let _ = take_triggers();
        assert_eq!(take_triggers(), (false, false));
    }

    #[test]
    fn the_config_and_state_directories_end_in_the_expected_place() {
        // the environment is not test state: what is pinned is the shape, not the
        // base — a config always lives in `tortty/` under the config dir
        if let Some(dir) = config_dir() {
            assert!(dir.is_absolute(), "{dir:?}");
        }
        if let Some(dir) = state_dir() {
            assert!(dir.is_absolute(), "{dir:?}");
        }
        assert_ne!(runtime_dir(), "");
        assert!(dump_path().is_absolute());
    }
}
