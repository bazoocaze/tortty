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

impl Pty {
    pub fn spawn(
        cmd: Option<String>,
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
            .map_err(|e| format!("openpty: {}", e))?;

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut cb = match &cmd {
            // o `--cmd` do Python passa por `sh -c`: mantém a mesma semântica
            Some(c) => {
                let mut cb = CommandBuilder::new("sh");
                cb.arg("-c");
                cb.arg(c);
                cb
            }
            None => CommandBuilder::new(&shell),
        };
        // o portable-pty cai em $HOME quando não há cwd; o Python herda o cwd
        // do processo (pty.fork), e é isso que o usuário espera
        if let Ok(cwd) = std::env::current_dir() {
            cb.cwd(cwd);
        }
        cb.env("TERM", "xterm-256color");
        cb.env("COLORTERM", "truecolor");
        cb.env("TERM_PROGRAM", "bzmux");
        cb.env("SHELL", &shell);

        let child = pair
            .slave
            .spawn_command(cb)
            .map_err(|e| format!("spawn: {}", e))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("reader: {}", e))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("writer: {}", e))?;

        std::thread::Builder::new()
            .name("pty-read".into())
            .spawn(move || {
                let mut buf = [0u8; 65536];
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
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                // sentinela de EOF: o app decide (o Python usa waitpid, e aqui
                // `Child::try_wait` faz o mesmo papel)
                let _ = tx.send(Vec::new());
                wake();
            })
            .map_err(|e| format!("thread: {}", e))?;

        Ok(Pty {
            master: pair.master,
            writer,
            child,
            size: (cols, rows),
        })
    }

    pub fn write(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let _ = self.writer.write_all(data);
        let _ = self.writer.flush();
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
    pub fn child_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
