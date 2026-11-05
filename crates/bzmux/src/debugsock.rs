//! Socket Unix de introspecção: uma linha JSON por pedido e por resposta.
//!
//! Mesmo protocolo do `bzmux/debugsock.py` (`{"ok":true,...}` / `{"ok":false,
//! "err":...}`, `0600`, só mesmo uid) — é o que permite usar o mesmo harness de
//! medição nos dois terminais.
//!
//! O socket roda em thread própria, mas **quem responde é a thread da UI**: o
//! pedido vai pelo `Sink` e a resposta volta por um canal. Assim não existe
//! acesso concorrente ao `Term`/janela.
//!
//! O `Sink` é o que torna o servidor testável sem janela: no app ele é o
//! `EventLoopProxy` do winit, nos testes é um canal — o socket em si não sabe
//! (nem precisa saber) que o winit existe.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

/// Quem enfileira o pedido para a thread da UI. Existe para o socket poder ser
/// exercitado num teste headless: sem esta costura, o único jeito de testar
/// seria criar um `EventLoop` — que exige display.
///
/// `Send + Sync + 'static` porque a `Arc` dele atravessa as threads de conexão.
pub trait Sink: Send + Sync + 'static {
    /// Entrega `line` (o JSON cru, sem newline) e o canal da resposta.
    /// `false` = ninguém vai responder: a conexão cai.
    fn enqueue(&self, line: String, reply: Sender<String>) -> bool;
}

impl Sink for EventLoopProxy<UserEvent> {
    fn enqueue(&self, line: String, reply: Sender<String>) -> bool {
        self.send_event(UserEvent::Debug(line, reply)).is_ok()
    }
}

/// Qualquer `Fn` que enfileire também é um `Sink`: o teste passa uma closure
/// que joga `(linha, canal)` no seu canal, e o app não precisa de código extra.
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
    /// `sink` é o `EventLoopProxy` no app (`DebugServer::start(&path,
    /// self.proxy.clone())` continua valendo) e um canal nos testes.
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
            .map_err(|e| format!("thread do socket: {e}"))?;
        Ok(DebugServer { path, stop })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for DebugServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // acorda o `accept` com uma conexão descartável: sem isso a thread do
        // socket fica pendurada num listener cujo arquivo já saiu (nos testes
        // isso vazaria uma thread por caso). O `stop` acima faz o laço sair
        // antes de atender quem se conectou.
        let _ = UnixStream::connect(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `auto` = `$XDG_RUNTIME_DIR/bzmux-<pid>.sock` (o mesmo nome do Python).
pub fn resolve(value: &str) -> String {
    if value == "auto" {
        auto_path()
    } else {
        value.to_string()
    }
}

/// `$XDG_RUNTIME_DIR` (ou `/tmp`, como no Python) mais o pid.
fn auto_path() -> String {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    auto_path_in(&base)
}

/// O nome em si, com a base explícita: puro, então o teste não precisa mexer no
/// ambiente (que é global e brigaria com os testes rodando em paralelo).
fn auto_path_in(base: &str) -> String {
    format!("{base}/bzmux-{}.sock", std::process::id())
}

fn clear_stale(path: &str) -> Result<(), String> {
    use std::os::unix::fs::FileTypeExt as _;
    let p = Path::new(path);
    let Ok(meta) = std::fs::metadata(p) else {
        return Ok(()); // não existe: nada a limpar
    };
    // arquivo que **não** é socket não é nosso: só o dono apaga (L12)
    if !meta.file_type().is_socket() {
        return Err(format!("caminho existe e não é socket: {path}"));
    }
    if UnixStream::connect(p).is_ok() {
        return Err(format!("outro processo está escutando em {path}"));
    }
    // socket órfão: ninguém escuta, o arquivo é lixo
    std::fs::remove_file(p).map_err(|e| format!("{path}: {e}"))?;
    Ok(())
}

/// `SO_PEERCRED`: só o mesmo uid fala com o socket. É o único `unsafe` do
/// projeto — `getsockopt` é FFI sem alternativa estável em Rust.
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
        // `read_until` em vez de `lines()`: byte que não é UTF-8 não pode
        // matar a conexão sem resposta — o pedido torto vira `err` do app
        // EOF (0 bytes) e erro de I/O acabam a conexão; byte lido é pedido
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

/// Resposta `ok` com campos extras.
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
    //! O socket sem janela: do outro lado do canal, um "app falso" que só
    //! responde JSON. Caminho único por teste (pid + contador) porque o
    //! `cargo test` roda casos em paralelo; nenhum `sleep` — quem espera é o
    //! canal (`recv_timeout`) ou o `read_timeout` do socket.

    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicU32;
    use std::sync::mpsc::{channel, Sender};
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    /// Caminho de socket exclusivo deste caso: pid + contador atômico.
    fn path(tag: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("bz-{tag}-{}-{n}.sock", std::process::id()))
    }

    /// Cliente do protocolo: escreve uma linha, lê uma linha. O
    /// `read_timeout` transforma "o servidor travou" em falha, não em hang.
    struct Cli {
        inp: UnixStream,
        out: BufReader<UnixStream>,
    }

    impl Cli {
        fn connect(path: &Path) -> Cli {
            let inp = UnixStream::connect(path).expect("conectar no socket");
            inp.set_read_timeout(Some(Duration::from_secs(10)))
                .expect("read timeout");
            let out = BufReader::new(inp.try_clone().expect("clone do stream"));
            Cli { inp, out }
        }

        fn send_raw(&mut self, bytes: &[u8]) {
            self.inp.write_all(bytes).expect("escrever pedido");
            self.inp.flush().expect("flush do pedido");
        }

        /// Uma linha JSON e o newline do framing.
        fn send(&mut self, req: &str) {
            let mut buf = String::with_capacity(req.len() + 1);
            buf.push_str(req);
            buf.push('\n');
            self.send_raw(buf.as_bytes());
        }

        /// Exatamente uma linha de resposta, sem o `\n`.
        fn line(&mut self) -> String {
            let mut s = String::new();
            self.out
                .read_line(&mut s)
                .expect("ler a resposta (timeout?)");
            assert!(s.ends_with('\n'), "resposta sem newline: {s:?}");
            s.pop();
            s
        }

        fn response(&mut self) -> serde_json::Value {
            let l = self.line();
            assert!(!l.contains('\n'), "resposta com mais de uma linha: {l:?}");
            serde_json::from_str(&l).expect("resposta não é JSON")
        }

        fn ask(&mut self, req: &str) -> serde_json::Value {
            self.send(req);
            self.response()
        }
    }

    /// "App falso" inline: a resposta sai do mesmo lado do canal que a UI
    /// usaria, sem thread e sem janela.
    fn fake(path: &Path, respond: impl Fn(&str) -> String + Send + Sync + 'static) -> DebugServer {
        let srv = DebugServer::start(
            path.to_str().expect("caminho utf8"),
            move |line: String, reply: Sender<String>| reply.send(respond(&line)).is_ok(),
        );
        srv.expect("start do socket")
    }

    #[test]
    fn frame_e_a_line_json_per_request() {
        // o app falso devolve a linha que recebeu: é assim que o teste vê que
        // o pedido atravessou o canal inteiro e sem sobra de newline
        let path = path("frame");
        let srv = fake(&path, |line| ok(json!({ "echo": line })));
        let mut cli = Cli::connect(&path);

        let v = cli.ask(r#"{"cmd":"echo","n":1}"#);
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["echo"], json!(r#"{"cmd":"echo","n":1}"#));

        // linha em branco não vira pedido (senão a resposta seguinte seria
        // lixo, e não o eco do pedido de verdade)
        cli.send_raw(b"\n   \n");
        let v = cli.ask(r#"{"cmd":"echo","n":2}"#);
        assert_eq!(v["echo"], json!(r#"{"cmd":"echo","n":2}"#));

        assert_eq!(srv.path(), path.to_str().unwrap());
        drop(cli);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn response_ok_e_err() {
        let path = path("okerr");
        let srv = fake(&path, |line| {
            if line.contains("\"cmd\":\"state\"") {
                ok(json!({ "cols": 80, "rows": 24 }))
            } else {
                err("cmd desconhecido")
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

        let v = cli.ask(r#"{"cmd":"nada"}"#);
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["err"], json!("cmd desconhecido"));
        assert!(v.get("cols").is_none());

        drop(srv);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn o_sink_receives_o_request_e_a_conexao_waits_a_response() {
        // aqui o "app" é outro lado de verdade: o pedido sai por um canal e a
        // resposta só volta quando o teste manda — o servidor tem de esperar
        let path = path("sink");
        let (tx, rx) = channel::<(String, Sender<String>)>();
        let srv = DebugServer::start(
            path.to_str().unwrap(),
            move |line: String, reply: Sender<String>| tx.send((line, reply)).is_ok(),
        )
        .expect("start do socket");
        let mut cli = Cli::connect(&path);

        cli.send(r#"{"cmd":"state"}"#);
        let (line, reply) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("pedido no canal do app");
        assert_eq!(line, r#"{"cmd":"state"}"#);
        reply
            .send(ok(json!({ "cols": 80 })))
            .expect("mandar a resposta");
        assert_eq!(cli.response()["cols"], json!(80));

        drop(srv);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn varios_pedidos_na_mesma_conexao() {
        let path = path("varios");
        let srv = fake(&path, |line| {
            let n = serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("n").and_then(serde_json::Value::as_u64));
            match n {
                Some(n) => ok(json!({ "n": n })),
                None => err("sem n"),
            }
        });
        let mut cli = Cli::connect(&path);

        // um de cada vez, no mesmo socket
        for n in 1..=3 {
            let v = cli.ask(&format!("{{\"n\":{n}}}"));
            assert_eq!(v["n"], json!(n));
        }
        // e dois pedidos num write só (o servidor tem de fatiar por linha)
        cli.send_raw(b"{\"n\":4}\n{\"n\":5}\n");
        assert_eq!(cli.response()["n"], json!(4));
        assert_eq!(cli.response()["n"], json!(5));
        // o pedido torto continua sendo um pedido: erro é resposta, não queda
        assert_eq!(cli.ask("null")["ok"], json!(false));

        drop(srv);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn utf8_invalido_becomes_request_skewed_e_not_queda() {
        // antes, `lines()` devolvia Err e a conexão morria sem resposta: quem
        // mandou lixo recebia EOF em vez de `{"ok":false,...}`
        let path = path("utf8");
        let srv = fake(&path, |line| {
            match serde_json::from_str::<serde_json::Value>(line) {
                // a linha chega com U+FFFD no lugar do byte inválido, como no
                // `decode("utf-8","replace")` do Python: o app vê JSON válido
                Ok(_) => ok(json!({ "echo": line })),
                Err(_) => err("json inválido"),
            }
        });
        let mut cli = Cli::connect(&path);
        // 0xFF sozinho não é UTF-8 válido
        cli.send_raw(b"{\"x\":\"\xff\"}\n");
        let v = cli.response();
        assert_eq!(v["ok"], json!(true), "conexão morreu em vez de responder");
        assert!(
            v["echo"].as_str().is_some_and(|s| s.contains('\u{fffd}')),
            "o byte inválido tinha virado U+FFFD: {}",
            v["echo"]
        );
        // e a conexão continua viva para o próximo pedido
        assert_eq!(cli.ask(r#"{"y":1}"#)["ok"], json!(true));
        drop(srv);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn o_file_do_socket_nasce_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = path("perm");
        let srv = fake(&path, |_| ok(json!({})));
        let mode = std::fs::metadata(&path)
            .expect("metadata do socket")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "socket de debug tem de ser só do dono");
        drop(srv);
        // o `Drop` leva o arquivo junto
        assert!(!path.exists(), "socket ficou para trás: {}", path.display());
    }

    #[test]
    fn clear_stale_not_clears_file_que_not_e_socket() {
        // L12: caminho ocupado por arquivo comum é recusado, não apagado
        let path = path("comum");
        std::fs::write(&path, b"nao sou socket").expect("criar arquivo comum");

        let e = clear_stale(path.to_str().unwrap()).expect_err("tinha de recusar");
        assert!(e.contains("não é socket"), "erro inesperado: {e}");
        assert!(path.exists(), "apagou arquivo alheio");

        // o servidor recusa pelo mesmo motivo e também não apaga
        let sink = |_: String, _: Sender<String>| false;
        let Err(e) = DebugServer::start(path.to_str().unwrap(), sink) else {
            panic!("start tinha de falhar");
        };
        assert!(e.contains("não é socket"), "erro inesperado: {e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"nao sou socket");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clear_stale_not_rouba_socket_alive() {
        let path = path("vivo");
        let listener = UnixListener::bind(&path).expect("bind do socket vivo");
        let e = clear_stale(path.to_str().unwrap()).expect_err("tinha de recusar");
        assert!(e.contains("escutando"), "erro inesperado: {e}");
        assert!(path.exists(), "roubou o socket de outro processo");

        // idem pelo `start`, que é o caminho do app
        let srv = DebugServer::start(path.to_str().unwrap(), |_: String, _: Sender<String>| false);
        assert!(srv.is_err(), "start tinha de recusar socket ocupado");

        drop(listener);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clear_stale_remove_socket_orfao() {
        let path = path("orfao");
        {
            // ninguém escutando depois deste bloco: o arquivo vira lixo
            let listener = UnixListener::bind(&path).expect("bind do órfão");
            drop(listener);
        }
        assert!(path.exists(), "o listener levou o arquivo junto");

        clear_stale(path.to_str().unwrap()).expect("órfão tinha de sair");
        assert!(!path.exists(), "socket órfão ficou para trás");

        // e caminho que não existe é sucesso, não erro
        clear_stale(path.to_str().unwrap()).expect("ausente não é erro");
    }

    #[test]
    fn resolve_auto_usa_xdg_runtime_dir_e_o_pid() {
        let pid = std::process::id();
        let name = format!("bzmux-{pid}.sock");
        // base explícita: puro, sem mexer no ambiente do processo
        assert_eq!(auto_path_in("/run/user/7"), format!("/run/user/7/{name}"));

        // `auto` de verdade: o nome leva o pid e a base é o XDG (ou /tmp)
        let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        assert_eq!(resolve("auto"), format!("{base}/{name}"));
        assert!(resolve("auto").ends_with(&name));

        // qualquer outro valor passa intacto (o app resolve o resto)
        assert_eq!(resolve("/tmp/bz.sock"), "/tmp/bz.sock");
    }
}
