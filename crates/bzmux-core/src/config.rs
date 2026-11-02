//! Configuração: formato do kitty (`chave valor`), padrões embutidos e erro
//! por linha.
//!
//! A regra é a do bzmux Python (`config.py`): **merge chave a chave**, com os
//! padrões escritos como arquivo (`DEFAULT_CONF`) e o arquivo do usuário por
//! cima. Erro é por linha, não por arquivo: o que validou vale, a chave
//! inválida fica no padrão anterior e o erro sai com o número da linha (o app
//! mostra em toast). No reload a regra é mais dura: **qualquer** erro mantém a
//! configuração que já estava valendo — trocar tudo por metade é pior que
//! ignorar a edição.
//!
//! `#` comenta quando está no começo da linha ou depois de espaço, então
//! `foreground #e6e6e6` continua sendo uma cor e não um comentário.

use crate::color::Color;

/// Os padrões, escritos como o arquivo que o usuário editaria. É a **única**
/// fonte deles: `Config::base()` é o resultado de parsear este texto, e o teste
/// `padroes_parseiam_limpos` garante que ele não tem erro.
pub const DEFAULT_CONF: &str = "\
# bzmux — padrões embutidos (o seu ~/.config/bzmux/bzmux.conf entra por cima)

# programa das abas. Vazio = $SHELL do ambiente. O --cmd vence este valor.
shell

# fonte da grade, em PONTOS (o DPI converte para pixel)
font_family Monospace
font_size 12

# ligaduras tipográficas: off (nenhuma), standard (liga/calt: fi, ->, !=) ou
# all (standard + discricionárias). Só aparece em fonte que as tenha; o shaping
# de script (árabe, devanagari) roda sempre, com qualquer valor.
ligatures standard

# cursor: block, beam ou underline; o pisca é on/off (CSI 6 q de um app vence)
cursor_shape block
cursor_blink off

# linhas de histórico guardadas acima da tela
scrollback 10000

# margem em volta da grade, em px
window_padding_width 0

# BEL: none (ignora), visual (marca a janela) ou audible (apita)
bell none

# seleção com o mouse já vai para CLIPBOARD (PRIMARY é sempre)
copy_on_select off

# log de diagnóstico; `off` desliga (o --debug-log troca o caminho)
debug_log off

# cores: foreground/background/cursor e a paleta ANSI de 16.
# Os padrões são a paleta clássica do xterm (a mesma do bzmux Python): mc, htop
# e ls --color foram calibrados nela, e trocar por um \"tema bonito\" muda o
# significado das cores que o programa escolheu.
foreground #d6d6d6
background #000000
cursor_color #d6d6d6
color0 #000000
color1 #cd0000
color2 #00cd00
color3 #cdcd00
color4 #0000ee
color5 #cd00cd
color6 #00cdcd
color7 #e5e5e5
color8 #7f7f7f
color9 #ff0000
color10 #00ff00
color11 #ffff00
color12 #5c5cff
color13 #ff00ff
color14 #00ffff
color15 #ffffff
";

/// Como o BEL do app se manifesta.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Bell {
    #[default]
    None,
    Visual,
    Audible,
}

/// Ligaduras tipográficas. Só tem efeito em fonte que as tenha (Fira Code,
/// JetBrains Mono…): o desenho não funde célula nenhuma, pede ao HarfBuzz o
/// glifo de cada célula com o contexto do trecho.
///
/// `Off` desliga **as ligaduras**, não o shaping: árabe e devanágari dependem do
/// shaping para serem legíveis e continuam sendo shapeados (é o que o Pango
/// fazia no Python).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Ligatures {
    Off,
    #[default]
    Standard,
    /// `standard` + discricionárias (`dlig`, `clig`).
    All,
}

/// Forma do cursor quando o app não pediu outra por `CSI 6 q`.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum CursorShape {
    #[default]
    Block,
    Beam,
    Underline,
}

/// Uma linha que não passou: `linha`, `chave` e o motivo.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Error {
    pub line: usize,
    pub key: String,
    pub msg: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.key.is_empty() {
            write!(f, "linha {}: {}", self.line, self.msg)
        } else {
            write!(f, "linha {}: '{}': {}", self.line, self.key, self.msg)
        }
    }
}

/// Cores do tema como o terminal as conhece: é o que o desenho usa e o que
/// `OSC 4/10/11/12` consulta e altera em tempo de execução.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Theme {
    pub fg: Color,
    pub bg: Color,
    pub cursor: Color,
    pub palette: [Color; 16],
}

impl Default for Theme {
    /// O mesmo tema dos padrões embutidos, sem precisar parsear texto.
    fn default() -> Self {
        Theme {
            fg: Color::Rgb(0xe6, 0xe6, 0xe6),
            bg: Color::Rgb(0x16, 0x18, 0x1d),
            cursor: Color::Rgb(0xe6, 0xe6, 0xe6),
            palette: std::array::from_fn(|i| Color::Idx(i as u8)),
        }
    }
}

impl Theme {
    /// Paleta de 256 com as 16 primeiras vindo do tema.
    #[must_use]
    pub fn palette256(&self) -> Vec<(u8, u8, u8)> {
        let mut pal = crate::color::palette256();
        for (i, c) in self.palette.iter().enumerate() {
            if let Color::Rgb(r, g, b) = c {
                pal[i] = (*r, *g, *b);
            }
        }
        pal
    }

    /// RGB final de uma cor do tema (a paleta resolve os índices); `Default`
    /// cai no `default` que o chamador escolher (fundo para espaço, texto
    /// para glifo).
    #[must_use]
    pub fn rgb(&self, c: Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
        match c {
            Color::Default => default,
            outro => crate::color::resolve(outro, &self.palette256(), default),
        }
    }
}

/// A configuração efetiva.
#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    pub shell: Option<String>,
    pub font_family: String,
    pub font_size: f32,
    pub ligatures: Ligatures,
    pub cursor_shape: CursorShape,
    pub cursor_blink: bool,
    pub scrollback: usize,
    pub padding: u32,
    pub bell: Bell,
    pub copy_on_select: bool,
    pub debug_log: Option<String>,
    pub fg: Color,
    pub bg: Color,
    pub cursor: Color,
    pub palette: [Color; 16],
}

impl Default for Config {
    fn default() -> Self {
        Config::base()
    }
}

/// As chaves conhecidas: chave fora daqui é erro (com o resto do arquivo valendo).
const KNOWN: &[&str] = &[
    "shell",
    "font_family",
    "font_size",
    "ligatures",
    "cursor_shape",
    "cursor_blink",
    "scrollback",
    "window_padding_width",
    "bell",
    "copy_on_select",
    "debug_log",
    "foreground",
    "background",
    "cursor_color",
    "color0",
    "color1",
    "color2",
    "color3",
    "color4",
    "color5",
    "color6",
    "color7",
    "color8",
    "color9",
    "color10",
    "color11",
    "color12",
    "color13",
    "color14",
    "color15",
];

const FONT_MIN: f32 = 4.0;
const FONT_MAX: f32 = 96.0;
const SCROLLBACK_MAX: usize = 100_000;
const PADDING_MAX: u32 = 200;

impl Config {
    /// Os padrões (parse limpo de `DEFAULT_CONF`; sem arquivo de usuário).
    #[must_use]
    pub fn base() -> Config {
        let (cfg, errs) = Config::parse_text(&Config::fallback(), DEFAULT_CONF);
        debug_assert!(errs.is_empty(), "DEFAULT_CONF tem erro: {errs:?}");
        cfg
    }

    /// Valores mínimos de segurança, usados só se `DEFAULT_CONF` estiver
    /// quebrado (não deve acontecer: o teste garante).
    fn fallback() -> Config {
        Config {
            shell: None,
            font_family: "monospace".to_string(),
            font_size: 12.0,
            ligatures: Ligatures::Standard,
            cursor_shape: CursorShape::Block,
            cursor_blink: false,
            scrollback: 1000,
            padding: 0,
            bell: Bell::None,
            copy_on_select: false,
            debug_log: None,
            fg: Color::Rgb(0xd6, 0xd6, 0xd6),
            bg: Color::Rgb(0, 0, 0),
            cursor: Color::Rgb(0xd6, 0xd6, 0xd6),
            palette: [
                Color::Idx(0),
                Color::Idx(1),
                Color::Idx(2),
                Color::Idx(3),
                Color::Idx(4),
                Color::Idx(5),
                Color::Idx(6),
                Color::Idx(7),
                Color::Idx(8),
                Color::Idx(9),
                Color::Idx(10),
                Color::Idx(11),
                Color::Idx(12),
                Color::Idx(13),
                Color::Idx(14),
                Color::Idx(15),
            ],
        }
    }

    /// Padrões + texto do usuário. Devolve os erros (vazios = tudo aplicado).
    #[must_use]
    pub fn parse(text: &str) -> (Config, Vec<Error>) {
        Config::parse_text(&Config::base(), text)
    }

    /// Como `parse`, mas partindo de uma configuração qualquer (é o reload: a
    /// que estava valendo é a base).
    #[must_use]
    pub fn over(&self, text: &str) -> (Config, Vec<Error>) {
        Config::parse_text(self, text)
    }

    #[allow(clippy::too_many_lines)] // um braço por chave, de propósito
    fn parse_text(base: &Config, text: &str) -> (Config, Vec<Error>) {
        let mut cfg = base.clone();
        let mut errors: Vec<Error> = Vec::new();
        for (n, raw) in text.lines().enumerate() {
            let line = n + 1;
            let sem = raw.trim();
            // linha em branco ou comentário de linha inteira
            if sem.is_empty() || sem.starts_with('#') {
                continue;
            }
            let (key, rest) = match sem.split_once(char::is_whitespace) {
                Some((k, v)) => (k, v),
                None => (sem, ""),
            };
            let value = value_of(rest);
            let value = value.as_str();
            if !KNOWN.contains(&key) {
                errors.push(Error {
                    line,
                    key: key.to_string(),
                    msg: "chave desconhecida".to_string(),
                });
                continue;
            }
            let bad = |msg: String| Error {
                line,
                key: key.to_string(),
                msg,
            };
            match key {
                "shell" => {
                    cfg.shell = if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    };
                },
                "font_family" => {
                    if value.is_empty() {
                        errors.push(bad("família vazia".to_string()));
                    } else {
                        cfg.font_family = value.to_string();
                    }
                },
                "font_size" => match value.parse::<f32>() {
                    Ok(v) if (FONT_MIN..=FONT_MAX).contains(&v) => cfg.font_size = v,
                    Ok(v) => errors.push(bad(format!(
                        "{v} fora da faixa {FONT_MIN}..{FONT_MAX} pontos"
                    ))),
                    Err(_) => errors.push(bad(format!("\"{value}\" não é número"))),
                },
                "ligatures" => match value {
                    "off" => cfg.ligatures = Ligatures::Off,
                    "standard" => cfg.ligatures = Ligatures::Standard,
                    "all" => cfg.ligatures = Ligatures::All,
                    other => {
                        errors.push(bad(format!("\"{other}\" não é off, standard nem all")));
                    },
                },
                "cursor_shape" => match value {
                    "block" => cfg.cursor_shape = CursorShape::Block,
                    "beam" => cfg.cursor_shape = CursorShape::Beam,
                    "underline" => cfg.cursor_shape = CursorShape::Underline,
                    other => {
                        errors.push(bad(format!("\"{other}\" não é block, beam nem underline")));
                    },
                },
                "cursor_blink" => match parse_bool(value) {
                    Some(b) => cfg.cursor_blink = b,
                    None => errors.push(bad(format!("\"{value}\" não é on/off"))),
                },
                "scrollback" => match value.parse::<usize>() {
                    Ok(v) if v <= SCROLLBACK_MAX => cfg.scrollback = v,
                    Ok(v) => errors.push(bad(format!("{v} acima do teto {SCROLLBACK_MAX}"))),
                    Err(_) => errors.push(bad(format!("\"{value}\" não é número"))),
                },
                "window_padding_width" => match value.parse::<u32>() {
                    Ok(v) if v <= PADDING_MAX => cfg.padding = v,
                    Ok(v) => errors.push(bad(format!("{v} acima do teto {PADDING_MAX} px"))),
                    Err(_) => errors.push(bad(format!("\"{value}\" não é número"))),
                },
                "bell" => match value {
                    "none" => cfg.bell = Bell::None,
                    "visual" => cfg.bell = Bell::Visual,
                    "audible" => cfg.bell = Bell::Audible,
                    other => {
                        errors.push(bad(format!("\"{other}\" não é none, visual nem audible")));
                    },
                },
                "copy_on_select" => match parse_bool(value) {
                    Some(b) => cfg.copy_on_select = b,
                    None => errors.push(bad(format!("\"{value}\" não é on/off"))),
                },
                "debug_log" => {
                    cfg.debug_log = if value.is_empty() || value == "off" {
                        None
                    } else {
                        Some(value.to_string())
                    };
                },
                "foreground" | "background" | "cursor_color" => match parse_color(value) {
                    Some(c) => match key {
                        "foreground" => cfg.fg = c,
                        "background" => cfg.bg = c,
                        _ => cfg.cursor = c,
                    },
                    None => errors.push(bad(format!("\"{value}\" não é cor"))),
                },
                _ => {
                    // color0..color15
                    let idx: usize = key[5..].parse().unwrap_or(16);
                    match parse_color(value) {
                        Some(c) if idx < 16 => cfg.palette[idx] = c,
                        Some(_) => errors.push(bad("índice de paleta fora de 0..15".to_string())),
                        None => errors.push(bad(format!("\"{value}\" não é cor"))),
                    }
                },
            }
        }
        (cfg, errors)
    }

    /// O tema (cores) desta configuração.
    #[must_use]
    pub fn theme(&self) -> Theme {
        Theme {
            fg: self.fg,
            bg: self.bg,
            cursor: self.cursor,
            palette: self.palette,
        }
    }

    /// Paleta de 256 com as 16 primeiras vindas do tema.
    #[must_use]
    pub fn palette256(&self) -> Vec<(u8, u8, u8)> {
        self.theme().palette256()
    }
}

/// Valor de uma linha, com comentário no fim.
///
/// Regra: `#` que **abre** o valor é cor (`foreground #e6e6e6`); `#` depois de
/// espaço, com valor já começado, comenta até o fim (`font_size 12  # nota`).
/// É a única forma de ter cor em hexadecimal e comentário na mesma sintaxe.
fn value_of(rest: &str) -> String {
    let rest = rest.trim();
    if rest.starts_with('#') {
        return rest.split_whitespace().next().unwrap_or("").to_string();
    }
    let bytes = rest.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return rest[..i].trim().to_string();
        }
    }
    rest.to_string()
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" | "1" => Some(true),
        "off" | "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_color(v: &str) -> Option<Color> {
    let hex = v.strip_prefix('#')?;
    let n = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let d = |i: usize| n(&hex[i..=i]).map(|x| x * 17);
            Some(Color::Rgb(d(0)?, d(1)?, d(2)?))
        },
        6 => Some(Color::Rgb(n(&hex[0..2])?, n(&hex[2..4])?, n(&hex[4..6])?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padroes_parseiam_limpos() {
        let (cfg, errs) = Config::parse("");
        assert!(errs.is_empty(), "DEFAULT_CONF tem erro: {errs:?}");
        assert_eq!(cfg.font_family, "Monospace");
        assert!((cfg.font_size - 12.0).abs() < f32::EPSILON);
        assert_eq!(cfg.scrollback, 10_000);
        assert_eq!(cfg.bell, Bell::None);
        assert!(!cfg.copy_on_select);
        assert_eq!(cfg.debug_log, None);
        assert_eq!(cfg.shell, None);
        assert_eq!(cfg.palette.len(), 16);
        // a paleta clássica do xterm: mc/htop contam com ela (ver CONFIG.md)
        assert!(matches!(cfg.palette[1], Color::Rgb(0xcd, 0x00, 0x00)));
        assert_eq!(cfg.bg, Color::Rgb(0, 0, 0));
    }

    #[test]
    fn comentario_nao_come_cor() {
        let (cfg, errs) =
            Config::parse("# linha toda\nforeground #112233   # nota no fim\nbackground #abc\n");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(cfg.fg, Color::Rgb(0x11, 0x22, 0x33));
        // #abc expande como #aabbcc
        assert_eq!(cfg.bg, Color::Rgb(0xaa, 0xbb, 0xcc));
    }

    /// Os três valores de `ligatures` valem e o padrão é `standard` (ligadura
    /// ligada, como no kitty); valor errado não derruba o arquivo, só a linha.
    #[test]
    fn ligaduras_por_nome() {
        assert_eq!(Config::parse("").0.ligatures, Ligatures::Standard);
        for (texto, esperado) in [
            ("off", Ligatures::Off),
            ("standard", Ligatures::Standard),
            ("all", Ligatures::All),
        ] {
            let (cfg, errs) = Config::parse(&format!("ligatures {texto}\n"));
            assert!(errs.is_empty(), "{texto}: {errs:?}");
            assert_eq!(cfg.ligatures, esperado, "{texto}");
        }
        let (cfg, errs) = Config::parse("ligatures talvez\nfont_size 14\n");
        assert_eq!(
            cfg.ligatures,
            Ligatures::Standard,
            "linha ruim mantém o padrão"
        );
        assert!(
            (cfg.font_size - 14.0).abs() < f32::EPSILON,
            "o resto do arquivo vale"
        );
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].key, "ligatures");
    }

    #[test]
    fn repetida_vence_a_ultima() {
        let (cfg, _) = Config::parse("font_size 20\nfont_size 14\n");
        assert!((cfg.font_size - 14.0).abs() < f32::EPSILON);
    }

    #[test]
    fn chave_desconhecida_e_erro_e_o_resto_vale() {
        let (cfg, errs) = Config::parse("inventada 1\nfont_size 20\n");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].line, 1);
        assert_eq!(errs[0].key, "inventada");
        assert!((cfg.font_size - 20.0).abs() < f32::EPSILON, "o resto valeu");
    }

    #[test]
    fn valor_invalido_mantem_o_padrao() {
        let (cfg, errs) = Config::parse("font_size abc\nscrollback -3\nbell grita\n");
        assert_eq!(errs.len(), 3);
        assert!((cfg.font_size - 12.0).abs() < f32::EPSILON);
        assert_eq!(cfg.scrollback, 10_000);
        assert_eq!(cfg.bell, Bell::None);
        assert!(errs[0].to_string().starts_with("linha 1: 'font_size'"));
    }

    #[test]
    fn faixas_tem_teto() {
        let (_, errs) =
            Config::parse("font_size 200\nscrollback 999999\nwindow_padding_width 900\n");
        assert_eq!(errs.len(), 3);
        let (cfg, _) = Config::parse("window_padding_width 6\n");
        assert_eq!(cfg.padding, 6);
    }

    #[test]
    fn shell_vazio_e_none_e_debug_log_off_desliga() {
        let (cfg, _) = Config::parse("shell del\n");
        assert_eq!(cfg.shell.as_deref(), Some("del"));
        let (cfg, _) = Config::parse("shell\ndebug_log off\n");
        assert_eq!(cfg.shell, None);
        assert_eq!(cfg.debug_log, None);
    }

    #[test]
    fn paleta_do_tema_chega_na_256() {
        let (cfg, errs) = Config::parse("color1 #ff0000\ncolor8 #00ff00\n");
        assert_eq!(errs, [], "erros inesperados");
        let p = cfg.palette256();
        assert_eq!(p[1], (0xff, 0, 0));
        assert_eq!(p[8], (0, 0xff, 0));
        // o resto do cubo 6x6x6 continua intacto
        assert_eq!(p[196], (255, 0, 0));
        assert_eq!(p.len(), 256);
    }

    #[test]
    fn reload_com_erro_nao_troca_nada() {
        let (bom, _) = Config::parse("font_size 16\n");
        let (tentativa, errs) = bom.over("font_size 18\nbell grita\n");
        assert_ne!(errs, [], "o erro tinha que aparecer");
        // quem chama só aplica se `errs` estiver vazio; aqui provamos que a base
        // não é mutada e que o valor bom sobrevive
        assert!((bom.font_size - 16.0).abs() < f32::EPSILON);
        assert!((tentativa.font_size - 18.0).abs() < f32::EPSILON);
    }

    #[test]
    fn doc_de_configuracao_cita_todas_as_chaves() {
        // a doc não pode envelhecer sozinha: toda chave dos padrões tem que
        // aparecer em docs/CONFIG.md
        let doc =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CONFIG.md"))
                .expect("docs/CONFIG.md");
        for k in KNOWN {
            // a paleta aparece como intervalo na doc (`color0` … `color15`)
            let intervalo =
                k.starts_with("color") && doc.contains("color0") && doc.contains("color15");
            assert!(
                doc.contains(k) || intervalo,
                "docs/CONFIG.md não fala de '{k}'"
            );
        }
    }
}
