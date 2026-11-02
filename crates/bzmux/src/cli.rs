//! Linha de comando e arquivo de configuração.
//!
//! Ordem de precedência: **CLI > arquivo do usuário > padrões embutidos**
//! (`bzmux_core::config::DEFAULT_CONF`). O `--config ARQ` troca o caminho do
//! arquivo do usuário; sem ele vale `$XDG_CONFIG_HOME/bzmux/bzmux.conf`.

use std::path::PathBuf;

use bzmux_core::config::{Config, Error};

/// O que veio da linha de comando: só o que foi escrito (`None` = não veio).
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
}

pub struct Options {
    pub cmd: Option<String>,
    pub cols: usize,
    pub rows: usize,
    pub font_family: String,
    /// Tamanho em PONTOS, como o `font_size` do bzmux Python.
    pub font_size_pt: f32,
    /// DPI usado para pt -> px (o `Xft.dpi` do Python). 96 = padrão X11.
    pub dpi: f32,
    pub scrollback: usize,
    pub debug_socket: Option<String>,
    pub debug_log: Option<String>,
    pub vsync: bool,
    pub prof: bool,
    /// O tema que não é da grade: paleta, cores, bell, padding, cursor.
    pub cfg: Config,
    /// De onde veio o arquivo do usuário (para o auto-reload e para o log).
    pub cfg_path: Option<PathBuf>,
}

impl Options {
    /// Config do usuário + CLI. Devolve também os erros do arquivo (o app
    /// mostra no log e, quando houver toast, na tela).
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
        // CLI vence o arquivo, chave a chave
        let font_family = over.font_family.unwrap_or_else(|| cfg.font_family.clone());
        let font_size_pt = over.font_size_pt.unwrap_or(cfg.font_size);
        let scrollback = over.scrollback.unwrap_or(cfg.scrollback);
        let debug_log = over.debug_log.or_else(|| cfg.debug_log.clone());
        let opts = Options {
            cmd: over.cmd,
            cols: over.cols.unwrap_or(80).max(1),
            rows: over.rows.unwrap_or(24).max(1),
            font_family,
            font_size_pt,
            dpi: over.dpi.unwrap_or(96.0),
            scrollback,
            debug_socket: over.debug_socket,
            debug_log,
            vsync: !over.no_vsync,
            prof: over.prof,
            cfg,
            cfg_path: path,
        };
        (opts, errors)
    }
}

/// `$XDG_CONFIG_HOME/bzmux/bzmux.conf` (ou `~/.config/...`).
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("bzmux").join("bzmux.conf"))
}

pub const HELP: &str = "\
bzmux (Rust) — terminal com workspaces e abas

USO:
  bzmux [opções]

OPÇÕES:
  --cmd CMD             comando inicial (vazio = $SHELL)
  --cols N --rows N     grade inicial (default 80x24)
  -c, --config ARQ      arquivo de configuração (default ~/.config/bzmux/bzmux.conf)
  --font-family NOME    família (default do arquivo: Monospace)
  --font-size PONTOS    tamanho em pontos (default do arquivo: 12)
  --dpi N               dpi para converter pontos em pixels (default 96)
  --scrollback N        teto de linhas do histórico
  --debug-socket ARQ    socket JSON de introspecção (auto = XDG_RUNTIME_DIR)
  --debug-log ARQ       arquivo de log de diagnóstico (off = desligado)
  --no-vsync            desliga a sincronia vertical (mede vazão, não o monitor)
  --prof                liga os contadores de custo (parse/draw/pty)
  -h, --help            esta ajuda
  -V, --version         versão

O arquivo de configuração tem fonte, cores, bell, padding e scrollback:
os padrões e as chaves estão em docs/CONFIG.md.
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
            other => return Err(format!("opção desconhecida: {other}\n\n{HELP}")),
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
    it.next().ok_or_else(|| format!("falta o valor de {flag}"))
}

fn num(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<usize, String> {
    let v = need(it, flag)?;
    v.parse()
        .map_err(|_| format!("{flag} espera um inteiro: {v}"))
}

fn float(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<f32, String> {
    let v = need(it, flag)?;
    v.parse()
        .map_err(|_| format!("{flag} espera um número: {v}"))
}
