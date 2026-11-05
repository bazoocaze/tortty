//! Command line and configuration file.
//!
//! Precedence: **CLI > user file > built-in defaults**
//! (`bzmux_core::config::DEFAULT_CONF`). `--config FILE` replaces the path of the
//! user file; without it `$XDG_CONFIG_HOME/bzmux/bzmux.conf` applies.

use std::path::PathBuf;

use bzmux_core::config::{Config, Error};

/// What came from the command line: only what was written (`None` = absent).
#[derive(Default, Clone)]
pub struct Overrides {
    pub cmd: Option<String>,
    pub cols: Option<usize>,
    pub rows: Option<usize>,
    pub font_family: Option<String>,
    pub font_size_pt: Option<f32>,
    pub dpi: Option<f32>,
    pub scrollback: Option<usize>,
    pub debug_socket: Option<String>,
    pub debug_log: Option<String>,
    pub config: Option<String>,
    pub no_vsync: bool,
    pub prof: bool,
    pub selftest: bool,
    pub quit_after: Option<u64>,
    pub dump_to: Option<String>,
}

pub struct Options {
    pub cmd: Option<String>,
    pub cols: usize,
    pub rows: usize,
    pub font_family: String,
    /// Size in POINTS, like `font_size` in the bzmux Python.
    pub font_size_pt: f32,
    /// DPI used for pt -> px (the Python `Xft.dpi`). 96 = X11 default.
    pub dpi: f32,
    pub scrollback: usize,
    pub debug_socket: Option<String>,
    pub debug_log: Option<String>,
    pub vsync: bool,
    pub prof: bool,
    /// Seconds until self-close (`--quit-after`), used by automation.
    pub quit_after: Option<u64>,
    /// Where the dump writes (`--dump-to`); `None` = /tmp/bzmux-dump.txt.
    pub dump_to: Option<PathBuf>,
    /// The theme that is not the grid: palette, colors, bell, padding, cursor.
    pub cfg: Config,
    /// Where the user file came from (for auto-reload and for the log).
    pub cfg_path: Option<PathBuf>,
    /// `--cols`/`--rows` asked on the command line: the requested grid wins over
    /// the remembered geometry (what automation and the bench need).
    pub grid_explicito: bool,
}

impl Options {
    /// User config + CLI. Also returns the file errors (the app shows them in
    /// the log and, once there is a toast, on screen).
    #[must_use]
    pub fn build(over: Overrides) -> (Options, Vec<Error>) {
        let path = over.config.clone().map(PathBuf::from).or_else(default_path);
        let mut errors: Vec<Error> = Vec::new();
        let mut cfg = Config::base();
        if let Some(p) = path.as_ref() {
            match std::fs::read_to_string(p) {
                Ok(text) => {
                    let (c, e) = cfg.over(&text);
                    cfg = c;
                    errors = e;
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                Err(e) => errors.push(Error {
                    line: 0,
                    key: String::new(),
                    msg: format!("{}: {e}", p.display()),
                }),
            }
        }
        // CLI wins over the file, key by key
        let font_family = over.font_family.unwrap_or_else(|| cfg.font_family.clone());
        let font_size_pt = over.font_size_pt.unwrap_or(cfg.font_size);
        let scrollback = over.scrollback.unwrap_or(cfg.scrollback);
        let debug_log = over.debug_log.or_else(|| cfg.debug_log.clone());
        // empty path means "no socket asked for": otherwise `bind("")` fails
        // with a message that says nothing (Python treats empty as off)
        let debug_socket = over.debug_socket.filter(|p| !p.is_empty());
        let grid_explicito = over.cols.is_some() || over.rows.is_some();
        let opts = Options {
            grid_explicito,
            cmd: over.cmd,
            cols: over.cols.unwrap_or(80).max(1),
            rows: over.rows.unwrap_or(24).max(1),
            font_family,
            font_size_pt,
            dpi: over.dpi.unwrap_or(96.0),
            scrollback,
            debug_socket,
            debug_log,
            vsync: !over.no_vsync,
            prof: over.prof,
            quit_after: over.quit_after,
            dump_to: over.dump_to.map(PathBuf::from),
            cfg,
            cfg_path: path,
        };
        (opts, errors)
    }
}

/// `$XDG_CONFIG_HOME/bzmux/bzmux.conf` (or `~/.config/...`).
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("bzmux").join("bzmux.conf"))
}

pub const HELP: &str = "\
bzmux (Rust) — terminal with workspaces and tabs

USAGE:
  bzmux [options]

OPTIONS:
  --cmd CMD             initial command (empty = $SHELL)
  --cols N --rows N     initial grid (default 80x24)
  -c, --config FILE     configuration file (default ~/.config/bzmux/bzmux.conf)
  --font-family NAME    family (file default: Monospace)
  --font-size POINTS    size in points (file default: 12)
  --dpi N               dpi to convert points into pixels (default 96)
  --scrollback N        cap on history lines
  --debug-socket FILE   JSON introspection socket (auto = XDG_RUNTIME_DIR)
  --debug-log FILE      diagnostic log file (off = disabled)
  --no-vsync            disables vertical sync (measures throughput, not the monitor)
  --prof                enables the cost counters (parse/draw/pty)
  --selftest            runs the core checks without opening a window, then exits
  --quit-after SECS     closes by itself after SECS seconds (automation)
  --dump-to FILE        at the end (and on SIGUSR1), writes the text of every tab
  -h, --help            this help
  -V, --version         version

The configuration file holds font, colors, bell, padding and scrollback:
the defaults and the keys are in docs/CONFIG.md.
";

pub fn parse(args: Vec<String>) -> Result<Overrides, String> {
    let mut o = Overrides::default();
    let mut it = args.into_iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Err(HELP.to_string()),
            "-V" | "--version" => {
                return Err(format!("bzmux {}", env!("CARGO_PKG_VERSION")));
            },
            "--cmd" => o.cmd = Some(need(&mut it, "--cmd")?),
            "--cols" => o.cols = Some(num(&mut it, "--cols")?),
            "--rows" => o.rows = Some(num(&mut it, "--rows")?),
            "-c" | "--config" => o.config = Some(need(&mut it, "--config")?),
            "--font-family" => o.font_family = Some(need(&mut it, "--font-family")?),
            "--font-size" => o.font_size_pt = Some(float(&mut it, "--font-size")?),
            "--dpi" => o.dpi = Some(float(&mut it, "--dpi")?),
            "--scrollback" => o.scrollback = Some(num(&mut it, "--scrollback")?),
            "--debug-socket" => o.debug_socket = Some(need(&mut it, "--debug-socket")?),
            "--debug-log" => o.debug_log = Some(need(&mut it, "--debug-log")?),
            "--no-vsync" => o.no_vsync = true,
            "--prof" => o.prof = true,
            "--selftest" => o.selftest = true,
            "--quit-after" => o.quit_after = Some(num(&mut it, "--quit-after")? as u64),
            "--dump-to" => o.dump_to = Some(need(&mut it, "--dump-to")?),
            other => return Err(format!("unknown option: {other}\n\n{HELP}")),
        }
    }
    if let Some(c) = o.cols {
        o.cols = Some(c.max(1));
    }
    if let Some(r) = o.rows {
        o.rows = Some(r.max(1));
    }
    Ok(o)
}

fn need(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    it.next()
        .ok_or_else(|| format!("missing the value of {flag}"))
}

fn num(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<usize, String> {
    let v = need(it, flag)?;
    v.parse()
        .map_err(|_| format!("{flag} expects an integer: {v}"))
}

fn float(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<f32, String> {
    let v = need(it, flag)?;
    v.parse()
        .map_err(|_| format!("{flag} expects a number: {v}"))
}

/// Equivalent of `argv_parsing` in `checks.py`: argv parsing and the merge
/// CLI > file > defaults open no window, so they fit in an ordinary test
/// (`--selftest` only runs the core, which does not know `cli.rs`).
/// Exit code for the `parse` message: `--help`/`--version` are informative
/// (0), **everything else is a usage error (2)** — including a value that is
/// not a number, which used to exit 0 and let automation scripts walk over an
/// invalid command.
#[must_use]
pub fn exit_code(msg: &str) -> i32 {
    // `--help` and `--version` **start** with the program name; the usage error
    // message also carries the help glued at the end, so looking for "USAGE:" in
    // the text classified an error as help
    let informative = msg.starts_with("bzmux ");
    if informative {
        0
    } else {
        2
    }
}

#[cfg(test)]
mod checks_cli {
    use bzmux_core::config::Bell;

    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    /// `parse` also returns `Err` for `--help`/`--version`; `Overrides`
    /// (on purpose) has no `Debug`, so no `unwrap`/`expect` here.
    fn ok(v: &[&str]) -> Overrides {
        match parse(args(v)) {
            Ok(o) => o,
            Err(e) => panic!("{v:?} should be valid: {e}"),
        }
    }

    fn error(v: &[&str]) -> String {
        match parse(args(v)) {
            Ok(_) => panic!("{v:?} should be a usage error"),
            Err(e) => e,
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < f32::EPSILON
    }

    fn temp_conf(name: &str, text: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bzmux-{}-{name}.conf", std::process::id()));
        std::fs::write(&p, text).expect("write the test config");
        p
    }

    #[test]
    fn argv_empty_asks_nothing() {
        // with no argument nothing is overridden: the defaults come in `build`
        let o = ok(&[]);
        assert!(o.cmd.is_none(), "cmd");
        assert!(o.cols.is_none() && o.rows.is_none(), "grid");
        assert!(o.config.is_none(), "config");
        assert!(o.font_family.is_none() && o.font_size_pt.is_none() && o.dpi.is_none());
        assert!(o.scrollback.is_none(), "scrollback");
        assert!(o.debug_socket.is_none() && o.debug_log.is_none());
        assert!(o.quit_after.is_none() && o.dump_to.is_none());
        assert!(!o.no_vsync && !o.prof && !o.selftest);
    }

    #[test]
    fn argv_command_and_grid() {
        let o = ok(&["--cmd", "mc", "--quit-after", "2"]);
        assert_eq!(o.cmd.as_deref(), Some("mc"));
        assert_eq!(o.quit_after, Some(2));
        let o = ok(&["--cols", "100", "--rows", "40"]);
        assert_eq!((o.cols, o.rows), (Some(100), Some(40)));
        // an empty grid does not exist: 0 becomes 1
        let o = ok(&["--cols", "0", "--rows", "0"]);
        assert_eq!((o.cols, o.rows), (Some(1), Some(1)));
    }

    #[test]
    fn argv_config_has_two_nomes() {
        for name in ["-c", "--config"] {
            let o = ok(&[name, "/tmp/x.conf"]);
            assert_eq!(o.config.as_deref(), Some("/tmp/x.conf"), "{name}");
        }
    }

    #[test]
    fn argv_debug_socket_and_log() {
        // introspection socket: value required, `auto` resolved later
        for v in ["auto", "/tmp/b.sock"] {
            let o = ok(&["--debug-socket", v]);
            assert_eq!(o.debug_socket.as_deref(), Some(v));
        }
        // `--debug-log` swaps the path; `off` passes through raw to `Options::build`
        let o = ok(&["--debug-log", "/tmp/x.log"]);
        assert_eq!(o.debug_log.as_deref(), Some("/tmp/x.log"));
        let o = ok(&["--debug-log", "off"]);
        assert_eq!(o.debug_log.as_deref(), Some("off"));
    }

    #[test]
    fn argv_rest_of_options() {
        let o = ok(&[
            "--font-family",
            "Hack",
            "--font-size",
            "13.5",
            "--dpi",
            "120",
            "--scrollback",
            "500",
            "--no-vsync",
            "--prof",
            "--selftest",
            "--dump-to",
            "/tmp/d.txt",
        ]);
        assert_eq!(o.font_family.as_deref(), Some("Hack"));
        assert!(o.font_size_pt.is_some_and(|v| near(v, 13.5)));
        assert!(o.dpi.is_some_and(|v| near(v, 120.0)));
        assert_eq!(o.scrollback, Some(500));
        assert!(o.no_vsync && o.prof && o.selftest);
        assert_eq!(o.dump_to.as_deref(), Some("/tmp/d.txt"));
        assert!(o.quit_after.is_none() && o.cmd.is_none());
    }

    #[test]
    fn argv_usage_error() {
        // missing value: the message names the option
        let e = error(&["--cmd"]);
        assert!(e.contains("--cmd") && e.starts_with("missing"), "{e}");
        let e = error(&["--debug-socket"]);
        assert!(e.contains("--debug-socket"), "{e}");
        // unknown option: usage error with the help attached
        let e = error(&["--nope"]);
        assert!(e.starts_with("unknown option"), "{e}");
        assert!(e.contains("--nope") && e.contains("--cmd"), "{e}");
        // value that is not a number
        let e = error(&["--quit-after", "abc"]);
        assert!(e.contains("--quit-after") && e.contains("abc"), "{e}");
        let e = error(&["--font-size", "big"]);
        assert!(e.contains("--font-size") && e.contains("big"), "{e}");
        let e = error(&["--rows", "-1"]);
        assert!(e.contains("--rows"), "{e}");
    }

    #[test]
    fn usage_exit_code() {
        // informative = 0 (piping `--help` into a script sees no error)
        assert_eq!(exit_code(&error(&["--help"])), 0);
        assert_eq!(exit_code(&error(&["--version"])), 0);
        // usage error = 2, including a value that is not a number (used to exit 0)
        for v in [
            &["--cols", "abc"][..],
            &["--quit-after", "x"][..],
            &["--nope"][..],
            &["--cmd"][..],
        ] {
            assert_eq!(exit_code(&error(v)), 2, "{v:?}");
        }
    }

    #[test]
    fn argv_help_and_version() {
        // --help and --version leave through the same path (Err), without a window
        let h = error(&["--help"]);
        assert!(h.starts_with("bzmux"), "{h}");
        assert!(h.contains("USAGE:") && h.contains("--selftest"), "{h}");
        assert_eq!(error(&["-h"]), h);
        let v = error(&["--version"]);
        assert_eq!(v, format!("bzmux {}", env!("CARGO_PKG_VERSION")));
        assert_eq!(error(&["-V"]), v);
    }

    #[test]
    fn merge_file_and_cli_wins() {
        // the user only overrides what they wrote; the CLI wins key by key
        let p = temp_conf(
            "merge",
            "shell /bin/sh\nfont_family DejaVu Sans Mono\nfont_size 14\n\
             scrollback 200\nbell visual\n",
        );
        let base = Overrides {
            config: Some(p.to_string_lossy().into_owned()),
            ..Overrides::default()
        };
        let (opts, errs) = Options::build(base.clone());
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(opts.font_family, "DejaVu Sans Mono");
        assert!(near(opts.font_size_pt, 14.0));
        assert_eq!(opts.scrollback, 200);
        assert_eq!(opts.cfg.bell, Bell::Visual);
        // `shell` is data, not resolution: `pty::spawn` decides
        assert_eq!(opts.cfg.shell.as_deref(), Some("/bin/sh"));
        assert_eq!(opts.cfg_path.as_deref(), Some(p.as_path()));
        // what neither file nor CLI brings falls back to the code default
        assert_eq!((opts.cols, opts.rows), (80, 24));
        assert!(near(opts.dpi, 96.0) && opts.vsync);
        assert_eq!(opts.cmd, None);

        let cli = Overrides {
            font_family: Some("Hack".to_string()),
            font_size_pt: Some(20.0),
            scrollback: Some(50),
            ..base
        };
        let (opts, errs) = Options::build(cli);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(opts.font_family, "Hack");
        assert!(near(opts.font_size_pt, 20.0));
        assert_eq!(opts.scrollback, 50);
        // the key the CLI did not touch keeps coming from the file
        assert_eq!(opts.cfg.bell, Bell::Visual);

        // missing file: the defaults apply, no error
        let gone = p.with_extension("does-not-exist");
        let (opts, errs) = Options::build(Overrides {
            config: Some(gone.to_string_lossy().into_owned()),
            ..Overrides::default()
        });
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(opts.font_family, "Monospace");
        assert_eq!(opts.scrollback, 10_000);
        assert_eq!(opts.debug_log, None);
        assert_eq!(opts.cfg.shell, None);
        assert_eq!(opts.cfg_path.as_deref(), Some(gone.as_path()));

        // the error is per line: the valid line before it applies, the invalid
        // one stays at the default; `main` logs/shows it (here the list carries the line)
        let bad = temp_conf("bad", "font_size 20\nbell beep\n");
        let (opts, errs) = Options::build(Overrides {
            config: Some(bad.to_string_lossy().into_owned()),
            ..Overrides::default()
        });
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(errs[0].line, 2);
        assert!(near(opts.font_size_pt, 20.0));
        assert_eq!(opts.cfg.bell, Bell::None);

        for f in [&p, &bad] {
            let _ = std::fs::remove_file(f);
        }
    }
}
