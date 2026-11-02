//! Linha de comando. Sem arquivo de configuração nesta fase (nota no TASK).

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
}

impl Default for Options {
    fn default() -> Self {
        Options {
            cmd: None,
            cols: 80,
            rows: 24,
            font_family: "monospace".to_string(),
            font_size_pt: 12.0,
            dpi: 96.0,
            scrollback: 1000,
            debug_socket: None,
            debug_log: None,
            vsync: true,
            prof: false,
        }
    }
}

pub const HELP: &str = "\
bzmux (Rust) — terminal com workspaces e abas

USO:
  bzmux [opções]

OPÇÕES:
  --cmd CMD             comando inicial (vazio = $SHELL)
  --cols N --rows N     grade inicial (default 80x24)
  --font-family NOME    família (default monospace)
  --font-size PONTOS    tamanho em pontos (default 12)
  --dpi N               dpi para converter pontos em pixels (default 96)
  --scrollback N        teto de linhas do histórico (default 1000)
  --debug-socket ARQ    socket JSON de introspecção (auto = XDG_RUNTIME_DIR)
  --debug-log ARQ       arquivo de log de diagnóstico (off = desligado)
  --no-vsync            desliga a sincronia vertical (mede vazão, não o monitor)
  --prof                liga os contadores de custo (parse/draw/pty)
  -h, --help            esta ajuda
  -V, --version         versão
";

pub fn parse(args: Vec<String>) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.into_iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Err(HELP.to_string()),
            "-V" | "--version" => {
                return Err(format!("bzmux {}", env!("CARGO_PKG_VERSION")));
            },
            "--cmd" => o.cmd = Some(need(&mut it, "--cmd")?),
            "--cols" => o.cols = num(&mut it, "--cols")?,
            "--rows" => o.rows = num(&mut it, "--rows")?,
            "--font-family" => o.font_family = need(&mut it, "--font-family")?,
            "--font-size" => o.font_size_pt = float(&mut it, "--font-size")?,
            "--dpi" => o.dpi = float(&mut it, "--dpi")?,
            "--scrollback" => o.scrollback = num(&mut it, "--scrollback")?,
            "--debug-socket" => o.debug_socket = Some(need(&mut it, "--debug-socket")?),
            "--debug-log" => o.debug_log = Some(need(&mut it, "--debug-log")?),
            "--no-vsync" => o.vsync = false,
            "--prof" => o.prof = true,
            other => return Err(format!("opção desconhecida: {other}\n\n{HELP}")),
        }
    }
    o.cols = o.cols.max(1);
    o.rows = o.rows.max(1);
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
