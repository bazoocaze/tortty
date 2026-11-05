//! Child pty: spawn, read on its own thread and resize.
//!
//! `portable-pty` instead of raw `openpty`: it is what gives ConPTY on Windows the
//! day that platform comes in (decision recorded in the TASK). The reader thread
//! pushes the bytes through a channel and wakes the loop with the `proxy` — the
//! same topology as the GLib watch in Python.

use std::io::{Read, Write};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::prof;

/// `PtySize` with the pixel size derived from the cell (what the kernel
/// reports in `ws_xpixel`/`ws_ypixel`).
fn pixels(cols: u16, rows: u16, cell: (f32, f32)) -> PtySize {
    let w = (f32::from(cols) * cell.0).round();
    let h = (f32::from(rows) * cell.1).round();
    PtySize {
        rows,
        cols,
        pixel_width: u16::try_from(w as u32).unwrap_or(0),
        pixel_height: u16::try_from(h as u32).unwrap_or(0),
    }
}

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    size: (u16, u16),
}

/// SHELL of the child, **always a path**: config (empty inherits the environment)
/// → `$SHELL` → `/bin/bash`. A value that does not exist in PATH falls back to
/// `/bin/bash`, like `shutil.which(value or $SHELL or "/bin/bash") or "/bin/bash"`
/// in Python. We used to take the raw `$SHELL`: a stale `$SHELL` pointing at a
/// dead path made the child die on the spot, and the default was `/bin/sh` (dash
/// on Debian) where Python uses `/bin/bash`.
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

/// Path of an executable: absolute/relative is checked directly, a bare name is
/// looked up in PATH (same idea as `shutil::which`).
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
    #[allow(clippy::too_many_arguments)] // pty spawn signature: all of them are data, not state
    pub fn spawn(
        cmd: Option<&str>,
        configured_shell: Option<&str>,
        cwd: Option<&std::path::Path>,
        cols: u16,
        rows: u16,
        // cell size in px: becomes `ws_xpixel`/`ws_ypixel`, from where
        // `kitten icat` takes the resolution (`TIOCGWINSZ`); at zero it gives up
        // with "terminal does not support reporting screen sizes"
        cell: (f32, f32),
        tx: Sender<Vec<u8>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Pty, String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(pixels(cols, rows, cell))
            .map_err(|e| format!("openpty: {e}"))?;

        let shell = effective_shell(configured_shell, std::env::var("SHELL").ok().as_deref());
        let mut cb = match cmd {
            // the Python `--cmd` goes through `sh -c`: keeps the same semantics
            Some(c) => {
                let mut cb = CommandBuilder::new("sh");
                cb.arg("-c");
                cb.arg(c);
                cb
            },
            None => CommandBuilder::new(&shell),
        };
        // portable-pty falls back to $HOME when there is no cwd; Python inherits
        // the process cwd (pty.fork), and that is what the user expects. A new tab
        // passes the cwd of the tab it was born from (I2), not the bzmux one.
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
                // 64 KB on the heap: on the stack it blows the thread frame
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
                // EOF sentinel: the app decides (Python uses waitpid, and here
                // `Child::try_wait` plays the same role)
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

    /// Writes to the pty. Returns `false` when the kernel did not accept
    /// everything (`EAGAIN` on a non-blocking pty, or an error): the caller
    /// counts it and retries.
    pub fn write(&mut self, data: &[u8]) -> bool {
        if data.is_empty() {
            return true;
        }
        let ok = self.writer.write_all(data).is_ok();
        let _ = self.writer.flush();
        ok
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell: (f32, f32)) {
        if (cols, rows) == self.size || cols == 0 || rows == 0 {
            return;
        }
        self.size = (cols, rows);
        let _ = self.master.resize(pixels(cols, rows, cell));
    }

    /// True when the child exited (the shell closed the tab).
    /// PID of the process running in the pty (base of `/proc/<pid>/cwd`).
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
    fn resolved_shell_is_a_path_that_exists() {
        // config wins over the environment
        assert_eq!(
            effective_shell(Some("/bin/sh"), Some("/bin/bash")),
            "/bin/sh"
        );
        // empty in the config inherits $SHELL
        assert_eq!(effective_shell(Some(""), Some("/bin/bash")), "/bin/bash");
        // a bare name is looked up in PATH (becomes a path)
        assert!(
            effective_shell(Some("sh"), None).starts_with('/'),
            "must come out as a path, not as the name"
        );
        // dead path in the config, and $SHELL also dead: falls back to /bin/bash
        let hard = effective_shell(Some("does-not-exist-xyz"), Some("/x"));
        assert!(hard == "/bin/bash" || hard == "/bin/sh", "{hard}");
        // nothing at all: /bin/bash, not /bin/sh
        let default = effective_shell(None, None);
        assert_eq!(
            default,
            if std::path::Path::new("/bin/bash").exists() {
                "/bin/bash"
            } else {
                "/bin/sh"
            }
        );
    }
}
