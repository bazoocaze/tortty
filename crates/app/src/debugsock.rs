//! Unix introspection socket: one JSON line per request and per reply.
//!
//! Same protocol as `bzmux/debugsock.py` (`{"ok":true,...}` / `{"ok":false,
//! "err":...}`, `0600`, same uid only) — it is what lets both terminals run the
//! same measurement harness.
//!
//! The socket runs on its own thread, but **the UI thread is the one that
//! answers**: the request goes through the `Sink` and the reply comes back over a
//! channel. That way there is no concurrent access to `Term`/window.
//!
//! The `Sink` is what makes the server testable without a window: in the app it is
//! winit's `EventLoopProxy`, in the tests it is a channel — the socket itself does
//! not know (nor needs to know) that winit exists.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

/// Whoever queues the request for the UI thread. It exists so the socket can be
/// exercised in a headless test: without this seam the only way to test it would
/// be to create an `EventLoop` — which needs a display.
///
/// `Send + Sync + 'static` because its `Arc` crosses the connection threads.
pub trait Sink: Send + Sync + 'static {
    /// Delivers `line` (the raw JSON, no newline) and the reply channel.
    /// `false` = nobody will answer: the connection drops.
    fn enqueue(&self, line: String, reply: Sender<String>) -> bool;
}

impl Sink for EventLoopProxy<UserEvent> {
    fn enqueue(&self, line: String, reply: Sender<String>) -> bool {
        self.send_event(UserEvent::Debug(line, reply)).is_ok()
    }
}

/// Any `Fn` that enqueues is also a `Sink`: the test passes a closure that
/// throws `(line, channel)` into its channel, and the app needs no extra code.
impl<F> Sink for F
where
    F: Fn(String, Sender<String>) -> bool + Send + Sync + 'static,
{
    fn enqueue(&self, line: String, reply: Sender<String>) -> bool {
        self(line, reply)
    }
}

pub struct DebugServer {
    path: String,
    stop: Arc<AtomicBool>,
}

impl DebugServer {
    /// `sink` is the `EventLoopProxy` in the app (`DebugServer::start(&path,
    /// self.proxy.clone())` still holds) and a channel in the tests.
    pub fn start(path: &str, sink: impl Sink) -> Result<DebugServer, String> {
        let path = resolve(path);
        clear_stale(&path)?;
        let listener = UnixListener::bind(&path).map_err(|e| format!("{path}: {e}"))?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let sink: Arc<dyn Sink> = Arc::new(sink);

        std::thread::Builder::new()
            .name("debug-sock".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if stop_thread.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(conn) = conn else { continue };
                    if !same_uid(&conn) {
                        continue;
                    }
                    let sink = Arc::clone(&sink);
                    std::thread::spawn(move || serves(conn, sink.as_ref()));
                }
            })
            .map_err(|e| format!("socket thread: {e}"))?;
        Ok(DebugServer { path, stop })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for DebugServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // wakes `accept` with a throwaway connection: without this the socket
        // thread hangs on a listener whose file is already gone (in the tests
        // that would leak one thread per case). The `stop` above makes the loop
        // exit before serving whoever connected.
        let _ = UnixStream::connect(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `auto` = `$XDG_RUNTIME_DIR/tortty-<pid>.sock` (the same name as the Python).
pub fn resolve(value: &str) -> String {
    if value == "auto" {
        auto_path()
    } else {
        value.to_string()
    }
}

/// `$XDG_RUNTIME_DIR` (or `/tmp`, as in the Python) plus the pid.
fn auto_path() -> String {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    auto_path_in(&base)
}

/// The name itself, with the base passed in: pure, so the test does not have to
/// touch the environment (which is global and would fight the tests running in
/// parallel).
fn auto_path_in(base: &str) -> String {
    format!("{base}/tortty-{}.sock", std::process::id())
}

fn clear_stale(path: &str) -> Result<(), String> {
    use std::os::unix::fs::FileTypeExt as _;
    let p = Path::new(path);
    let Ok(meta) = std::fs::metadata(p) else {
        return Ok(()); // does not exist: nothing to clean
    };
    // a file that is **not** a socket is not ours: only the owner deletes it (L12)
    if !meta.file_type().is_socket() {
        return Err(format!("path exists and is not a socket: {path}"));
    }
    if UnixStream::connect(p).is_ok() {
        return Err(format!("another process is listening on {path}"));
    }
    // orphan socket: nobody listens, the file is garbage
    std::fs::remove_file(p).map_err(|e| format!("{path}: {e}"))?;
    Ok(())
}

/// `SO_PEERCRED`: only the same uid talks to the socket. It is the only `unsafe`
/// in the project — `getsockopt` is FFI with no stable alternative in Rust.
#[allow(unsafe_code)]
fn same_uid(conn: &UnixStream) -> bool {
    use std::os::unix::io::AsRawFd;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            conn.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    rc == 0 && cred.uid == unsafe { libc::getuid() }
}

fn serves(conn: UnixStream, sink: &dyn Sink) {
    let Ok(mut out) = conn.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(conn);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        // `read_until` instead of `lines()`: a byte that is not UTF-8 must not
        // kill the connection without an answer — a broken request becomes the
        // app's `err`. EOF (0 bytes) and an I/O error end the connection; a byte
        // read is a request.
        if !matches!(reader.read_until(b'\n', &mut buf), Ok(n) if n > 0) {
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end_matches(['\n', '\r']);
        if line.trim().is_empty() {
            continue;
        }
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        if !sink.enqueue(line.to_string(), tx) {
            break;
        }
        let Ok(reply) = rx.recv() else { break };
        if out.write_all(reply.as_bytes()).is_err() || out.write_all(b"\n").is_err() {
            break;
        }
        let _ = out.flush();
    }
}

/// `ok` reply with extra fields.
pub fn ok(fields: serde_json::Value) -> String {
    match fields {
        serde_json::Value::Object(mut m) => {
            m.insert("ok".into(), serde_json::Value::Bool(true));
            serde_json::Value::Object(m).to_string()
        },
        _ => format!("{{\"ok\":true,\"value\":{fields}}}"),
    }
}

pub fn err(msg: impl Into<String>) -> String {
    serde_json::json!({ "ok": false, "err": msg.into() }).to_string()
}

#[cfg(test)]
mod checks_socket {
    //! The socket without a window: on the other side of the channel, a "fake
    //! app" that only answers JSON. One path per test (pid + counter) because
    //! `cargo test` runs cases in parallel; no `sleep` — the ones that wait are
    //! the channel (`recv_timeout`) or the socket's `read_timeout`.

    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU32;
    use std::sync::mpsc::{channel, Sender};
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    /// Socket path exclusive to this case: pid + atomic counter.
    fn path(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("bz-{tag}-{}-{n}.sock", std::process::id()))
    }

    /// Client of the protocol: writes one line, reads one line. `read_timeout`
    /// turns "the server hung" into a failure, not into a hang.
    struct Cli {
        inp: UnixStream,
        out: BufReader<UnixStream>,
    }

    impl Cli {
        fn connect(path: &Path) -> Cli {
            let inp = UnixStream::connect(path).expect("connect to the socket");
            inp.set_read_timeout(Some(Duration::from_secs(10)))
                .expect("read timeout");
            let out = BufReader::new(inp.try_clone().expect("clone the stream"));
            Cli { inp, out }
        }

        fn send_raw(&mut self, bytes: &[u8]) {
            self.inp.write_all(bytes).expect("write the request");
            self.inp.flush().expect("flush the request");
        }

        /// One JSON line and the framing newline.
        fn send(&mut self, req: &str) {
            let mut buf = String::with_capacity(req.len() + 1);
            buf.push_str(req);
            buf.push('\n');
            self.send_raw(buf.as_bytes());
        }

        /// Exactly one reply line, without the `\n`.
        fn line(&mut self) -> String {
            let mut s = String::new();
            self.out
                .read_line(&mut s)
                .expect("read the reply (timeout?)");
            assert!(s.ends_with('\n'), "reply without newline: {s:?}");
            s.pop();
            s
        }

        fn response(&mut self) -> serde_json::Value {
            let l = self.line();
            assert!(!l.contains('\n'), "reply with more than one line: {l:?}");
            serde_json::from_str(&l).expect("reply is not JSON")
        }

        fn ask(&mut self, req: &str) -> serde_json::Value {
            self.send(req);
            self.response()
        }
    }

    /// Inline "fake app": the reply leaves from the same side of the channel the
    /// UI would use, with no thread and no window.
    fn fake(path: &Path, respond: impl Fn(&str) -> String + Send + Sync + 'static) -> DebugServer {
        let srv = DebugServer::start(
            path.to_str().expect("utf8 path"),
            move |line: String, reply: Sender<String>| reply.send(respond(&line)).is_ok(),
        );
        srv.expect("socket start")
    }

    #[test]
    fn framing_is_one_json_line_per_request() {
        // the fake app echoes the line it received: that is how the test sees
        // the request cross the whole channel with no leftover newline
        let path = path("frame");
        let srv = fake(&path, |line| ok(json!({ "echo": line })));
        let mut cli = Cli::connect(&path);

        let v = cli.ask(r#"{"cmd":"echo","n":1}"#);
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["echo"], json!(r#"{"cmd":"echo","n":1}"#));

        // a blank line does not become a request (otherwise the next reply
        // would be garbage instead of the echo of the real request)
        cli.send_raw(b"\n   \n");
        let v = cli.ask(r#"{"cmd":"echo","n":2}"#);
        assert_eq!(v["echo"], json!(r#"{"cmd":"echo","n":2}"#));

        assert_eq!(srv.path(), path.to_str().unwrap());
        drop(cli);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn response_ok_and_err() {
        let path = path("okerr");
        let srv = fake(&path, |line| {
            if line.contains("\"cmd\":\"state\"") {
                ok(json!({ "cols": 80, "rows": 24 }))
            } else {
                err("unknown cmd")
            }
        });
        let mut cli = Cli::connect(&path);

        let v = cli.ask(r#"{"cmd":"state"}"#);
        assert_eq!(v["ok"], json!(true));
        assert_eq!(
            (v["cols"].as_u64(), v["rows"].as_u64()),
            (Some(80), Some(24))
        );
        assert!(v.get("err").is_none());

        let v = cli.ask(r#"{"cmd":"nothing"}"#);
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["err"], json!("unknown cmd"));
        assert!(v.get("cols").is_none());

        drop(srv);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sink_receives_the_request_and_the_connection_waits_for_a_reply() {
        // here the "app" is a real other side: the request leaves over a channel
        // and the reply only comes back when the test sends it — the server has
        // to wait
        let path = path("sink");
        let (tx, rx) = channel::<(String, Sender<String>)>();
        let srv = DebugServer::start(
            path.to_str().unwrap(),
            move |line: String, reply: Sender<String>| tx.send((line, reply)).is_ok(),
        )
        .expect("start the socket");
        let mut cli = Cli::connect(&path);

        cli.send(r#"{"cmd":"state"}"#);
        let (line, reply) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("request on the app channel");
        assert_eq!(line, r#"{"cmd":"state"}"#);
        reply
            .send(ok(json!({ "cols": 80 })))
            .expect("send the reply");
        assert_eq!(cli.response()["cols"], json!(80));

        drop(srv);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn several_requests_on_the_same_connection() {
        let path = path("several");
        let srv = fake(&path, |line| {
            let n = serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("n").and_then(serde_json::Value::as_u64));
            match n {
                Some(n) => ok(json!({ "n": n })),
                None => err("missing n"),
            }
        });
        let mut cli = Cli::connect(&path);

        // one at a time, on the same socket
        for n in 1..=3 {
            let v = cli.ask(&format!("{{\"n\":{n}}}"));
            assert_eq!(v["n"], json!(n));
        }
        // and two requests in a single write (the server has to slice by line)
        cli.send_raw(b"{\"n\":4}\n{\"n\":5}\n");
        assert_eq!(cli.response()["n"], json!(4));
        assert_eq!(cli.response()["n"], json!(5));
        // a broken request is still a request: an error is a reply, not a drop
        assert_eq!(cli.ask("null")["ok"], json!(false));

        drop(srv);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn invalid_utf8_becomes_a_broken_request_not_a_drop() {
        // before, `lines()` returned Err and the connection died with no reply:
        // whoever sent garbage got EOF instead of `{"ok":false,...}`
        let path = path("utf8");
        let srv = fake(&path, |line| {
            match serde_json::from_str::<serde_json::Value>(line) {
                // the line arrives with U+FFFD in place of the invalid byte, as
                // in the Python `decode("utf-8","replace")`: the app sees valid
                // JSON
                Ok(_) => ok(json!({ "echo": line })),
                Err(_) => err("invalid json"),
            }
        });
        let mut cli = Cli::connect(&path);
        // a lone 0xFF is not valid UTF-8
        cli.send_raw(b"{\"x\":\"\xff\"}\n");
        let v = cli.response();
        assert_eq!(v["ok"], json!(true), "connection died instead of replying");
        assert!(
            v["echo"].as_str().is_some_and(|s| s.contains('\u{fffd}')),
            "the invalid byte had become U+FFFD: {}",
            v["echo"]
        );
        // and the connection stays alive for the next request
        assert_eq!(cli.ask(r#"{"y":1}"#)["ok"], json!(true));
        drop(srv);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn socket_file_is_born_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = path("perm");
        let srv = fake(&path, |_| ok(json!({})));
        let mode = std::fs::metadata(&path)
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "a debug socket has to be owner-only");
        drop(srv);
        // `Drop` takes the file with it
        assert!(!path.exists(), "socket left behind: {}", path.display());
    }

    #[test]
    fn clear_stale_does_not_clear_file_that_is_not_a_socket() {
        // L12: a path taken by a regular file is refused, not deleted
        let path = path("regular");
        std::fs::write(&path, b"not a socket").expect("create regular file");

        let e = clear_stale(path.to_str().unwrap()).expect_err("it had to refuse");
        assert!(e.contains("not a socket"), "unexpected error: {e}");
        assert!(path.exists(), "deleted someone else's file");

        // the server refuses for the same reason and does not delete either
        let sink = |_: String, _: Sender<String>| false;
        let Err(e) = DebugServer::start(path.to_str().unwrap(), sink) else {
            panic!("start had to fail");
        };
        assert!(e.contains("not a socket"), "unexpected error: {e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"not a socket");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clear_stale_does_not_steal_an_alive_socket() {
        let path = path("alive");
        let listener = UnixListener::bind(&path).expect("bind of the alive socket");
        let e = clear_stale(path.to_str().unwrap()).expect_err("it had to refuse");
        assert!(e.contains("listening"), "unexpected error: {e}");
        assert!(path.exists(), "stole another process's socket");

        // same through `start`, which is the app path
        let srv = DebugServer::start(path.to_str().unwrap(), |_: String, _: Sender<String>| false);
        assert!(srv.is_err(), "start had to refuse a taken socket");

        drop(listener);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clear_stale_removes_orphan_socket() {
        let path = path("orphan");
        {
            // nobody listening after this block: the file becomes garbage
            let listener = UnixListener::bind(&path).expect("bind of the orphan");
            drop(listener);
        }
        assert!(path.exists(), "the listener took the file with it");

        clear_stale(path.to_str().unwrap()).expect("the orphan had to go");
        assert!(!path.exists(), "orphan socket left behind");

        // and a path that does not exist is success, not an error
        clear_stale(path.to_str().unwrap()).expect("missing is not an error");
    }

    #[test]
    fn resolve_auto_uses_xdg_runtime_dir_and_the_pid() {
        let pid = std::process::id();
        let name = format!("tortty-{pid}.sock");
        // explicit base: pure, without touching the process environment
        assert_eq!(auto_path_in("/run/user/7"), format!("/run/user/7/{name}"));

        // real `auto`: the name carries the pid and the base is XDG (or /tmp)
        let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        assert_eq!(resolve("auto"), format!("{base}/{name}"));
        assert!(resolve("auto").ends_with(&name));

        // any other value passes through untouched (the app resolves the rest)
        assert_eq!(resolve("/tmp/bz.sock"), "/tmp/bz.sock");
    }
}
