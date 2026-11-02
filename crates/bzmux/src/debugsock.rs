//! Socket Unix de introspecção: uma linha JSON por pedido e por resposta.
//!
//! Mesmo protocolo do `bzmux/debugsock.py` (`{"ok":true,...}` / `{"ok":false,
//! "err":...}`, `0600`, só mesmo uid) — é o que permite usar o mesmo harness de
//! medição nos dois terminais.
//!
//! O socket roda em thread própria, mas **quem responde é a thread da UI**: o
//! pedido vai pelo `EventLoopProxy` e a resposta volta por um canal. Assim não
//! existe acesso concorrente ao `Term`/janela.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

pub struct DebugServer {
    path: String,
    stop: Arc<AtomicBool>,
}

impl DebugServer {
    pub fn start(
        path: &str,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Result<DebugServer, String> {
        let path = resolve(path);
        clear_stale(&path)?;
        let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {}", path, e))?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();

        std::thread::Builder::new()
            .name("debug-sock".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if stop_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(conn) = conn else { continue };
                    if !same_uid(&conn) {
                        continue;
                    }
                    let proxy = proxy.clone();
                    std::thread::spawn(move || serve(conn, proxy));
                }
            })
            .map_err(|e| format!("thread do socket: {}", e))?;
        Ok(DebugServer { path, stop })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for DebugServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `auto` = `$XDG_RUNTIME_DIR/bzmux-<pid>.sock` (o mesmo nome do Python).
pub fn resolve(value: &str) -> String {
    if value == "auto" {
        let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        format!("{}/bzmux-{}.sock", base, std::process::id())
    } else {
        value.to_string()
    }
}

fn clear_stale(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(());
    }
    match UnixStream::connect(p) {
        Ok(_) => Err(format!("outro processo está escutando em {}", path)),
        Err(_) => {
            let _ = std::fs::remove_file(p); // socket órfão
            Ok(())
        }
    }
}

fn same_uid(conn: &UnixStream) -> bool {
    use std::os::unix::io::AsRawFd;
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            conn.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    rc == 0 && cred.uid == unsafe { libc::getuid() }
}

fn serve(conn: UnixStream, proxy: EventLoopProxy<UserEvent>) {
    let Ok(mut out) = conn.try_clone() else { return };
    let reader = BufReader::new(conn);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        if proxy.send_event(UserEvent::Debug(line, tx)).is_err() {
            break;
        }
        let reply = match rx.recv() {
            Ok(r) => r,
            Err(_) => break,
        };
        if out.write_all(reply.as_bytes()).is_err() || out.write_all(b"\n").is_err() {
            break;
        }
        let _ = out.flush();
    }
}

/// Resposta `ok` com campos extras.
pub fn ok(fields: serde_json::Value) -> String {
    match fields {
        serde_json::Value::Object(mut m) => {
            m.insert("ok".into(), serde_json::Value::Bool(true));
            serde_json::Value::Object(m).to_string()
        }
        _ => format!("{{\"ok\":true,\"value\":{}}}", fields),
    }
}

pub fn err(msg: impl Into<String>) -> String {
    serde_json::json!({ "ok": false, "err": msg.into() }).to_string()
}
