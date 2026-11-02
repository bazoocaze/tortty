//! pty do filho: spawn, leitura em thread própria e resize.
//!
//! `portable-pty` em vez de `openpty` cru: é o que dá ConPTY no Windows no dia
//! em que essa plataforma entrar (decisão registrada no TASK). A thread leitora
//! empurra os bytes por canal e acorda o loop com o `proxy` — a mesma topologia
//! do watch do GLib no Python.

use std::io::{Read, Write};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::prof;

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    size: (u16, u16),
}

/// SHELL do filho, **sempre caminho**: config (vazio herda o ambiente) →
/// `$SHELL` → `/bin/bash`. Valor que não existe no PATH cai em `/bin/bash`,
/// como o `shutil.which(value or $SHELL or "/bin/bash") or "/bin/bash"` do
/// Python. Antes pegávamos o `$SHELL` cru: um `$SHELL` velho apontando para
/// caminho morto fazia o filho morrer na hora, e o padrão era `/bin/sh` (dash
/// no Debian) onde o Python usa `/bin/bash`.
#[must_use]
pub fn effective_shell(configured: Option<&str>, env_shell: Option<&str>) -> String {
    let cand = configured
        .filter(|s| !s.is_empty())
        .or_else(|| env_shell.filter(|s| !s.is_empty()));
    cand.and_then(which).unwrap_or_else(|| {
        if std::path::Path::new("/bin/bash").exists() {
            "/bin/bash".to_string()
        } else {
            "/bin/sh".to_string()
        }
    })
}

/// Caminho de um executável: absoluto/relativo é conferido direto, nome solto
/// é procurado no PATH (mesma ideia do `shutil::which`).
fn which(name: &str) -> Option<String> {
    if name.contains('/') {
        return std::path::Path::new(name)
            .is_file()
            .then(|| name.to_string());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|c| c.is_file())
        .map(|c| c.to_string_lossy().into_owned())
}

impl Pty {
    pub fn spawn(
        cmd: Option<&str>,
        configured_shell: Option<&str>,
        cwd: Option<&std::path::Path>,
        cols: u16,
        rows: u16,
        tx: Sender<Vec<u8>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pty, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty: {e}"))?;

        let shell = effective_shell(configured_shell, std::env::var("SHELL").ok().as_deref());
        let mut cb = match cmd {
            // o `--cmd` do Python passa por `sh -c`: mantém a mesma semântica
            Some(c) => {
                let mut cb = CommandBuilder::new("sh");
                cb.arg("-c");
                cb.arg(c);
                cb
            },
            None => CommandBuilder::new(&shell),
        };
        // o portable-pty cai em $HOME quando não há cwd; o Python herda o cwd
        // do processo (pty.fork), e é isso que o usuário espera. Aba nova passa
        // o cwd da aba de onde nasceu (I2), não o do bzmux.
        let dir = cwd
            .map(std::path::Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok());
        if let Some(dir) = dir {
            cb.cwd(dir);
        }
        cb.env("TERM", "xterm-256color");
        cb.env("COLORTERM", "truecolor");
        cb.env("TERM_PROGRAM", "bzmux");
        cb.env("SHELL", &shell);

        let child = pair
            .slave
            .spawn_command(cb)
            .map_err(|e| format!("spawn: {e}"))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("writer: {e}"))?;

        std::thread::Builder::new()
            .name("pty-read".into())
            .spawn(move || {
                // 64 KB no heap: no stack estoura o frame da thread
                let mut buf = vec![0u8; 65536];
                loop {
                    let t0 = Instant::now();
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if prof::enabled() {
                                prof::add_pty(t0.elapsed().as_nanos() as u64, n);
                            }
                            if tx.send(buf[..n].to_vec()).is_err() {
                                return;
                            }
                            wake();
                        },
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {},
                        Err(_) => break,
                    }
                }
                // sentinela de EOF: o app decide (o Python usa waitpid, e aqui
                // `Child::try_wait` faz o mesmo papel)
                let _ = tx.send(Vec::new());
                wake();
            })
            .map_err(|e| format!("thread: {e}"))?;

        Ok(Pty {
            master: pair.master,
            writer,
            child,
            size: (cols, rows),
        })
    }

    /// Escreve no pty. Devolve `false` quando o kernel não aceitou tudo
    /// (`EAGAIN` num pty não-bloqueante, ou erro): quem chama conta e repete.
    pub fn write(&mut self, data: &[u8]) -> bool {
        if data.is_empty() {
            return true;
        }
        let ok = self.writer.write_all(data).is_ok();
        let _ = self.writer.flush();
        ok
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if (cols, rows) == self.size || cols == 0 || rows == 0 {
            return;
        }
        self.size = (cols, rows);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    /// True quando o filho saiu (o shell fechou a aba).
    /// PID do processo que roda no pty (base do `/proc/<pid>/cwd`).
    pub fn child_pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    pub fn child_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod checks_shell {
    use super::effective_shell;

    #[test]
    fn shell_resolvido_e_um_caminho_que_existe() {
        // config vence o ambiente
        assert_eq!(
            effective_shell(Some("/bin/sh"), Some("/bin/bash")),
            "/bin/sh"
        );
        // vazio na config herda o $SHELL
        assert_eq!(effective_shell(Some(""), Some("/bin/bash")), "/bin/bash");
        // nome solto é procurado no PATH (vira caminho)
        assert!(
            effective_shell(Some("sh"), None).starts_with('/'),
            "precisa sair em caminho, não no nome"
        );
        // caminho morto na config, e $SHELL também morto: cai no /bin/bash
        let duro = effective_shell(Some("nao-existe-xyz"), Some("/x"));
        assert!(duro == "/bin/bash" || duro == "/bin/sh", "{duro}");
        // nada de nada: /bin/bash, não /bin/sh
        let padrao = effective_shell(None, None);
        assert_eq!(
            padrao,
            if std::path::Path::new("/bin/bash").exists() {
                "/bin/bash"
            } else {
                "/bin/sh"
            }
        );
    }
}
