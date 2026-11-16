//! Unix introspection socket: one JSON line per request and per reply.
//!
//! The protocol is `{"ok":true,...}` / `{"ok":false,"err":...}`, on a socket with
//! `0600` and the same uid only — it is what lets a measurement harness pin the
//! grid and read the state. The socket itself (bind, permissions, peer
//! credential, stale file) is the `platform` seam: this module owns the protocol
//! and the threading.
//!
//! The socket runs on its own thread, but **the UI thread is the one that
//! answers**: the request goes through the `Sink` and the reply comes back over a
//! channel. That way there is no concurrent access to `Term`/window.
//!
//! The `Sink` is what makes the server testable without a window: in the app it is
//! winit's `EventLoopProxy`, in the tests it is a channel — the socket itself does
//! not know (nor needs to know) that winit exists.

use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;
use crate::platform::{self, Conn, Listener};

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
        let listener = Listener::bind(std::path::Path::new(&path))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let sink: Arc<dyn Sink> = Arc::new(sink);

        std::thread::Builder::new()
            .name("debug-sock".into())
            .spawn(move || loop {
                let Ok(conn) = listener.accept() else {
                    continue;
                };
                if stop_thread.load(Ordering::SeqCst) {
                    break;
                }
                if !platform::same_user(&conn) {
                    continue;
                }
                let sink = Arc::clone(&sink);
                std::thread::spawn(move || serves(conn, sink.as_ref()));
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
        // `stop` first, then a throwaway connection to wake `accept`: the loop
        // checks the flag before serving whoever connected, exits, and the
        // listener it owns removes the file on its way out. Without the wake the
        // thread would hang on a listener whose file is gone (one leaked thread
        // per test case).
        self.stop.store(true, Ordering::SeqCst);
        let path = std::path::Path::new(&self.path);
        platform::wake(path);
        platform::remove_socket(path);
    }
}

/// `auto` = `$XDG_RUNTIME_DIR/tortty-<pid>.sock`.
pub fn resolve(value: &str) -> String {
    if value == "auto" {
        auto_path()
    } else {
        value.to_string()
    }
}

/// The runtime directory of the system plus the pid.
fn auto_path() -> String {
    auto_path_in(&platform::runtime_dir())
}

/// The name itself, with the base passed in: pure, so the test does not have to
/// touch the environment (which is global and would fight the tests running in
/// parallel).
fn auto_path_in(base: &str) -> String {
    format!("{base}/tortty-{}.sock", std::process::id())
}

fn serves(conn: Conn, sink: &dyn Sink) {
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
        inp: Conn,
        out: BufReader<Conn>,
    }

    impl Cli {
        fn connect(path: &Path) -> Cli {
            let inp = Conn::connect(path).expect("connect to the socket");
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
                // the line arrives with U+FFFD in place of the invalid byte
                // (lossy decode): the app sees valid JSON
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

    /// The seam is what the socket is built on: a path taken by a file that is not
    /// a socket has to be refused here too, and the file kept (L12 is decided in
    /// `platform`, this is the app path).
    #[test]
    fn start_refuses_a_path_that_is_not_a_socket() {
        let path = path("regular");
        std::fs::write(&path, b"not a socket").expect("create regular file");
        let sink = |_: String, _: Sender<String>| false;
        let Err(e) = DebugServer::start(path.to_str().unwrap(), sink) else {
            panic!("start had to fail");
        };
        assert!(e.contains("not a socket"), "unexpected error: {e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"not a socket");
        let _ = std::fs::remove_file(&path);
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
