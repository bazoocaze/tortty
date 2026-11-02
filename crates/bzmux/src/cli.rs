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
    pub selftest: bool,
    pub quit_after: Option<u64>,
    pub dump_to: Option<String>,
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
    /// Segundos até fechar sozinho (`--quit-after`), usado pela automação.
    pub quit_after: Option<u64>,
    /// Onde o dump escreve (`--dump-to`); `None` = /tmp/bzmux-dump.txt.
    pub dump_to: Option<PathBuf>,
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
            quit_after: over.quit_after,
            dump_to: over.dump_to.map(PathBuf::from),
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
  --selftest            roda os checks do núcleo sem abrir janela e sai
  --quit-after SEG      fecha sozinho depois de SEG segundos (automação)
  --dump-to ARQ         no fim (e em SIGUSR1), escreve o texto de todas as abas
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
            "--selftest" => o.selftest = true,
            "--quit-after" => o.quit_after = Some(num(&mut it, "--quit-after")? as u64),
            "--dump-to" => o.dump_to = Some(need(&mut it, "--dump-to")?),
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

/// Equivalente ao `argv_parsing` do `checks.py`: o parsing de argv e o merge
/// CLI > arquivo > padrões não abrem janela, então cabem num teste comum (o
/// `--selftest` só roda o núcleo, que não conhece `cli.rs`).
#[cfg(test)]
mod checks_cli {
    use bzmux_core::config::Bell;

    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    /// `parse` devolve `Err` também para `--help`/`--version`; `Overrides`
    /// (de propósito) não tem `Debug`, então nada de `unwrap`/`expect` aqui.
    fn ok(v: &[&str]) -> Overrides {
        match parse(args(v)) {
            Ok(o) => o,
            Err(e) => panic!("{v:?} devia ser válido: {e}"),
        }
    }

    fn erro(v: &[&str]) -> String {
        match parse(args(v)) {
            Ok(_) => panic!("{v:?} devia ser erro de uso"),
            Err(e) => e,
        }
    }

    fn perto(a: f32, b: f32) -> bool {
        (a - b).abs() < f32::EPSILON
    }

    fn conf_temporario(nome: &str, texto: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bzmux-{}-{nome}.conf", std::process::id()));
        std::fs::write(&p, texto).expect("escrever o config de teste");
        p
    }

    #[test]
    fn argv_vazio_nao_pede_nada() {
        // sem argumento nada é sobrescrito: os padrões entram no `build`
        let o = ok(&[]);
        assert!(o.cmd.is_none(), "cmd");
        assert!(o.cols.is_none() && o.rows.is_none(), "grade");
        assert!(o.config.is_none(), "config");
        assert!(o.font_family.is_none() && o.font_size_pt.is_none() && o.dpi.is_none());
        assert!(o.scrollback.is_none(), "scrollback");
        assert!(o.debug_socket.is_none() && o.debug_log.is_none());
        assert!(o.quit_after.is_none() && o.dump_to.is_none());
        assert!(!o.no_vsync && !o.prof && !o.selftest);
    }

    #[test]
    fn argv_comando_e_grade() {
        let o = ok(&["--cmd", "mc", "--quit-after", "2"]);
        assert_eq!(o.cmd.as_deref(), Some("mc"));
        assert_eq!(o.quit_after, Some(2));
        let o = ok(&["--cols", "100", "--rows", "40"]);
        assert_eq!((o.cols, o.rows), (Some(100), Some(40)));
        // grade vazia não existe: 0 vira 1
        let o = ok(&["--cols", "0", "--rows", "0"]);
        assert_eq!((o.cols, o.rows), (Some(1), Some(1)));
    }

    #[test]
    fn argv_config_tem_dois_nomes() {
        for nome in ["-c", "--config"] {
            let o = ok(&[nome, "/tmp/x.conf"]);
            assert_eq!(o.config.as_deref(), Some("/tmp/x.conf"), "{nome}");
        }
    }

    #[test]
    fn argv_debug_socket_e_log() {
        // socket de introspecção: valor obrigatório, `auto` resolvido depois
        for v in ["auto", "/tmp/b.sock"] {
            let o = ok(&["--debug-socket", v]);
            assert_eq!(o.debug_socket.as_deref(), Some(v));
        }
        // `--debug-log` troca o caminho; `off` passa cru ao `Options::build`
        let o = ok(&["--debug-log", "/tmp/x.log"]);
        assert_eq!(o.debug_log.as_deref(), Some("/tmp/x.log"));
        let o = ok(&["--debug-log", "off"]);
        assert_eq!(o.debug_log.as_deref(), Some("off"));
    }

    #[test]
    fn argv_resto_das_opcoes() {
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
        assert!(o.font_size_pt.is_some_and(|v| perto(v, 13.5)));
        assert!(o.dpi.is_some_and(|v| perto(v, 120.0)));
        assert_eq!(o.scrollback, Some(500));
        assert!(o.no_vsync && o.prof && o.selftest);
        assert_eq!(o.dump_to.as_deref(), Some("/tmp/d.txt"));
        assert!(o.quit_after.is_none() && o.cmd.is_none());
    }

    #[test]
    fn argv_erro_de_uso() {
        // valor faltando: a mensagem nomeia a opção
        let e = erro(&["--cmd"]);
        assert!(e.contains("--cmd") && e.starts_with("falta"), "{e}");
        let e = erro(&["--debug-socket"]);
        assert!(e.contains("--debug-socket"), "{e}");
        // opção desconhecida: erro de uso com a ajuda junto
        let e = erro(&["--nada"]);
        assert!(e.starts_with("opção desconhecida"), "{e}");
        assert!(e.contains("--nada") && e.contains("--cmd"), "{e}");
        // valor que não é número
        let e = erro(&["--quit-after", "abc"]);
        assert!(e.contains("--quit-after") && e.contains("abc"), "{e}");
        let e = erro(&["--font-size", "grande"]);
        assert!(e.contains("--font-size") && e.contains("grande"), "{e}");
        let e = erro(&["--rows", "-1"]);
        assert!(e.contains("--rows"), "{e}");
    }

    #[test]
    fn argv_help_e_version() {
        // --help e --version saem pelo mesmo caminho (Err), sem abrir janela
        let h = erro(&["--help"]);
        assert!(h.starts_with("bzmux"), "{h}");
        assert!(h.contains("USO:") && h.contains("--selftest"), "{h}");
        assert_eq!(erro(&["-h"]), h);
        let v = erro(&["--version"]);
        assert_eq!(v, format!("bzmux {}", env!("CARGO_PKG_VERSION")));
        assert_eq!(erro(&["-V"]), v);
    }

    #[test]
    fn merge_do_arquivo_e_cli_vence() {
        // o usuário só sobrescreve o que escreveu; a CLI vence chave a chave
        let p = conf_temporario(
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
        assert!(perto(opts.font_size_pt, 14.0));
        assert_eq!(opts.scrollback, 200);
        assert_eq!(opts.cfg.bell, Bell::Visual);
        // `shell` é dado, não resolução: quem decide é o `pty::spawn`
        assert_eq!(opts.cfg.shell.as_deref(), Some("/bin/sh"));
        assert_eq!(opts.cfg_path.as_deref(), Some(p.as_path()));
        // o que nem arquivo nem CLI trazem cai no padrão do código
        assert_eq!((opts.cols, opts.rows), (80, 24));
        assert!(perto(opts.dpi, 96.0) && opts.vsync);
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
        assert!(perto(opts.font_size_pt, 20.0));
        assert_eq!(opts.scrollback, 50);
        // a chave que a CLI não tocou continua vindo do arquivo
        assert_eq!(opts.cfg.bell, Bell::Visual);

        // arquivo inexistente: valem os padrões, sem erro
        let sumido = p.with_extension("nao-existe");
        let (opts, errs) = Options::build(Overrides {
            config: Some(sumido.to_string_lossy().into_owned()),
            ..Overrides::default()
        });
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(opts.font_family, "Monospace");
        assert_eq!(opts.scrollback, 10_000);
        assert_eq!(opts.debug_log, None);
        assert_eq!(opts.cfg.shell, None);
        assert_eq!(opts.cfg_path.as_deref(), Some(sumido.as_path()));

        // erro é por linha: o válido antes do erro aplica, o inválido fica no
        // padrão; quem loga/mostra é o `main` (aqui a lista vem com a linha)
        let ruim = conf_temporario("ruim", "font_size 20\nbell beep\n");
        let (opts, errs) = Options::build(Overrides {
            config: Some(ruim.to_string_lossy().into_owned()),
            ..Overrides::default()
        });
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert_eq!(errs[0].line, 2);
        assert!(perto(opts.font_size_pt, 20.0));
        assert_eq!(opts.cfg.bell, Bell::None);

        for f in [&p, &ruim] {
            let _ = std::fs::remove_file(f);
        }
    }
}
