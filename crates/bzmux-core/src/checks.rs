//! Checks de fumaça sem display (`bzmux --selftest`).
//!
//! Espelham o que o `checks.py` do bzmux Python verifica, mas aqui rodam no
//! binário: uma função por área, todas usando só o núcleo (sem janela, GPU ou
//! pty). O objetivo é ter **um comando** que responda "o emulador ainda está
//! inteiro?" em qualquer máquina, inclusive no CI e por ssh.
//!
//! Regra: cada check monta o cenário, exercita e **compara com o esperado** —
//! nada de "não panicou". Quando falha, devolve o porquê em texto.

use crate::cell::{BOLD, ITALIC, UL_CURLY};
use crate::color::Color;
use crate::config::Config;
use crate::select::Span;
use crate::term::Term;

/// Um check: nome (o mesmo conceito do `checks.py`) e a função.
pub struct Check {
    pub name: &'static str,
    pub run: fn() -> Result<(), String>,
}

/// Todos os checks, na ordem em que aparecem no relatório.
#[must_use]
pub fn all() -> Vec<Check> {
    let mut v = vec![
        Check {
            name: "parser_text_and_cursor",
            run: parser_text_and_cursor,
        },
        Check {
            name: "wrap_and_control",
            run: wrap_and_control,
        },
        Check {
            name: "sgr_and_erasing",
            run: sgr_and_erasing,
        },
        Check {
            name: "truecolor_and_palette",
            run: truecolor_and_palette,
        },
        Check {
            name: "charset_and_replies",
            run: charset_and_replies,
        },
        Check {
            name: "alt_screen_and_scroll",
            run: alt_screen_and_scroll,
        },
        Check {
            name: "history_and_scrollback",
            run: history_and_scrollback,
        },
        Check {
            name: "reflow",
            run: reflow,
        },
        Check {
            name: "selection_follows_history",
            run: selection_follows_history,
        },
        Check {
            name: "paste_markers",
            run: paste_markers,
        },
        Check {
            name: "csi_with_intermediate",
            run: csi_with_intermediate,
        },
        Check {
            name: "config_file",
            run: config_file,
        },
        Check {
            name: "theme_osc",
            run: theme_osc,
        },
        Check {
            name: "damage_tracking",
            run: damage_tracking,
        },
    ];
    // N2 (config/CLI) e N5 (histórico/rolagem/seleção) moram em módulos
    // próprios só para o arquivo não virar 3 mil linhas.
    v.extend(crate::checks_n2::all());
    v.extend(crate::checks_n5::all());
    v
}

/// Roda tudo. Devolve `(passaram, falhas)` com o texto de cada falha.
#[must_use]
pub fn run_all() -> (usize, Vec<String>) {
    let mut ok = 0;
    let mut falhas = Vec::new();
    for c in all() {
        match (c.run)() {
            Ok(()) => ok += 1,
            Err(e) => falhas.push(format!("{}: {e}", c.name)),
        }
    }
    (ok, falhas)
}

fn term() -> Term {
    Term::new(20, 5)
}

// por valor de propósito: quem chama monta o esperado na hora (`"abc".to_string()`)
#[allow(clippy::needless_pass_by_value)]
fn esperado<T: PartialEq + std::fmt::Debug>(
    o_que: &str,
    obtido: T,
    esperado: T,
) -> Result<(), String> {
    if obtido == esperado {
        Ok(())
    } else {
        Err(format!("{o_que}: esperado {esperado:?}, obtido {obtido:?}"))
    }
}

fn parser_text_and_cursor() -> Result<(), String> {
    let mut t = term();
    t.feed(b"abc");
    esperado("texto", t.row_text(0), "abc".to_string())?;
    esperado("cursor", (t.cy, t.cx), (0, 3))?;
    // CUP, CR, LF, BS e TAB
    t.feed(b"\x1b[3;5Hxy\r\nz\tw");
    esperado("CUP+CRLF", t.row_text(3), "z       w".to_string())?;
    // o cursor está depois do "w" (coluna 9); BS volta para a 8 e sobrescreve
    t.feed(b"\x08!");
    esperado("backspace", t.row_text(3), "z       !".to_string())?;
    t.feed(b"\x1b[1;1H\x1b[2X");
    esperado("ECH apaga 2", t.row_text(0), "  c".to_string())?;
    Ok(())
}

fn wrap_and_control() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    t.feed(b"0123456789A");
    esperado(
        "quebra",
        (t.row_text(0), t.row_text(1)),
        ("0123456789".to_string(), "A".to_string()),
    )?;
    if !t.grid[0].wrapped || t.grid[1].wrapped {
        return Err("marca de quebra mole errada".to_string());
    }
    // autowrap desligado não quebra: sobrescreve a última coluna
    t.feed(b"\x1b[?7l");
    t.feed(b"\x1b[1;1H0123456789XY");
    esperado("sem autowrap", t.row_text(0), "012345678Y".to_string())?;
    t.feed(b"\x1b[?7h");
    Ok(())
}

fn sgr_and_erasing() -> Result<(), String> {
    let mut t = term();
    t.feed(b"\x1b[1;3;4:3;38;2;1;2;3mA");
    let c = t.grid[0].cells[0];
    if c.flags & BOLD == 0 || c.flags & ITALIC == 0 || c.ul != UL_CURLY {
        return Err("atributos de SGR não subiram".to_string());
    }
    esperado("truecolor", c.fg, Color::Rgb(1, 2, 3))?;
    t.feed(b"\x1b[0m\x1b[2J");
    esperado("ED2 limpa", t.row_text(0), String::new())?;
    esperado("ED2 volta o cursor", (t.cy, t.cx), (0, 0))?;
    Ok(())
}

fn truecolor_and_palette() -> Result<(), String> {
    let (cfg, errs) = Config::parse("color1 #ff0000\nforeground #010203\n");
    if !errs.is_empty() {
        return Err(format!("config com erro: {errs:?}"));
    }
    let pal = cfg.palette256();
    esperado("paleta do tema", pal[1], (0xff, 0, 0))?;
    esperado("cubo 6x6x6 intacto", pal[196], (255, 0, 0))?;
    let tema = cfg.theme();
    esperado("fg do tema", tema.rgb(tema.fg, (0, 0, 0)), (1, 2, 3))?;
    Ok(())
}

fn charset_and_replies() -> Result<(), String> {
    let mut t = term();
    t.feed(b"\x1b(0lqk\x1b(B");
    let s: String = t.grid[0].cells[0..3].iter().map(|c| c.ch).collect();
    esperado(
        "DEC special graphics",
        s,
        "\u{250c}\u{2500}\u{2510}".to_string(),
    )?;
    t.feed(b"\x1b[c");
    let out = String::from_utf8_lossy(&t.take_child_out()).to_string();
    if !out.contains("\x1b[?") {
        return Err(format!("DA1 não respondeu: {out:?}"));
    }
    t.feed(b"\x1b[6n");
    let out = String::from_utf8_lossy(&t.take_child_out()).to_string();
    if !out.contains('R') {
        return Err(format!("CPR não respondeu: {out:?}"));
    }
    Ok(())
}

fn alt_screen_and_scroll() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    t.feed(b"um\ndois");
    t.feed(b"\x1b[?1049h");
    if !t.in_alt_screen() {
        return Err("não entrou na tela alternativa".to_string());
    }
    t.feed(b"no alt\x1b[?1049l");
    esperado("volta do alt", t.row_text(0), "um".to_string())?;
    // região de scroll (DECSTBM): só o miolo rola
    t.feed(b"\x1b[2;3r\x1b[3;1H\n");
    esperado("topo intocado", t.row_text(0), "um".to_string())?;
    t.feed(b"\x1b[r");
    Ok(())
}

fn history_and_scrollback() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    for i in 0..8 {
        t.feed(format!("l{i}\r\n").as_bytes());
    }
    if t.scrollback.len() < 5 {
        return Err(format!("histórico curto: {}", t.scrollback.len()));
    }
    esperado("última linha viva", t.row_text(2), String::new())?;
    // ED3 esvazia o histórico e sobe a geração (âncora morre)
    let g = t.abs_gen;
    t.feed(b"\x1b[3J");
    esperado("ED3 esvazia", t.scrollback.len(), 0)?;
    if t.abs_gen <= g {
        return Err("ED3 não invalidou as âncoras".to_string());
    }
    Ok(())
}

fn reflow() -> Result<(), String> {
    let mut t = Term::new(10, 4);
    t.feed(b"abcdefghijklmnopqrstuvwxy");
    t.resize(5, 4);
    let junto: String = t
        .scrollback
        .iter()
        .chain(t.grid.iter())
        .map(crate::grid::Row::text)
        .collect::<String>();
    esperado(
        "texto preservado",
        junto,
        "abcdefghijklmnopqrstuvwxy".to_string(),
    )?;
    t.resize(10, 4);
    esperado("volta ao tamanho", t.row_text(0), "abcdefghij".to_string())?;
    Ok(())
}

fn selection_follows_history() -> Result<(), String> {
    let mut t = Term::new(10, 4);
    t.feed(b"linha-a\r\nlinha-b\r\n");
    let a = t.anchor_at(0, 0, 0);
    let b = t.anchor_at(0, 0, 6);
    let span = Span::new(a, b);
    // rola a tela: a âncora continua apontando o mesmo texto
    for _ in 0..3 {
        t.feed(b"\r\n");
    }
    let achado = t.span_text(&span).ok_or("âncora morreu sem motivo")?;
    esperado("seleção sobrevive ao scroll", achado, "linha-a".to_string())?;
    Ok(())
}

fn paste_markers() -> Result<(), String> {
    let mut t = term();
    if t.bracketed_paste {
        return Err("bracketed paste ligado por padrão".to_string());
    }
    t.feed(b"\x1b[?2004h");
    if !t.bracketed_paste {
        return Err("?2004 não ligou".to_string());
    }
    let payload = String::from_utf8_lossy(&t.paste_payload("oi")).to_string();
    esperado("payload", payload, "\x1b[200~oi\x1b[201~".to_string())?;
    t.feed(b"\x1b[?2004l");
    Ok(())
}

fn csi_with_intermediate() -> Result<(), String> {
    let mut t = term();
    // DECSCUSR com intermediário (espaço) e CSI ! p (soft reset)
    // DECSCUSR: 4 = sublinhado fixo → (forma 1 = underline, pisca false)
    t.feed(b"\x1b[4 q");
    esperado("cursor underline", t.app_cursor_style, Some((1, false)))?;
    t.feed(b"\x1b[1;1;1m\x1b[!p");
    esperado("soft reset limpa SGR", t.flags, 0)?;
    Ok(())
}

fn config_file() -> Result<(), String> {
    let (cfg, errs) = Config::parse("font_size 15\nfont_size 16\nbell visual\n");
    if !errs.is_empty() {
        return Err(format!("erros inesperados: {errs:?}"));
    }
    esperado("repetida vence", cfg.font_size, 16.0)?;
    esperado("bell", format!("{:?}", cfg.bell), "Visual".to_string())?;
    let (_, errs) = Config::parse("nao_existe 1\nfont_size abc\n");
    if errs.len() != 2 {
        return Err(format!("esperava 2 erros, veio {errs:?}"));
    }
    Ok(())
}

fn theme_osc() -> Result<(), String> {
    let mut t = term();
    let padrao = t.theme.bg;
    t.feed(b"\x1b]11;?\x1b\\");
    let out = String::from_utf8_lossy(&t.take_child_out()).to_string();
    if !out.contains("]11;rgb:") {
        return Err(format!("OSC 11 não respondeu: {out:?}"));
    }
    t.feed(b"\x1b]11;#102030\x1b\\");
    esperado("OSC 11 define", t.theme.bg, Color::Rgb(0x10, 0x20, 0x30))?;
    if !t.theme_dirty {
        return Err("theme_dirty não subiu".to_string());
    }
    t.feed(b"\x1b]111\x1b\\");
    esperado("OSC 111 volta", t.theme.bg, padrao)?;
    Ok(())
}

fn damage_tracking() -> Result<(), String> {
    let mut t = term();
    t.feed(b"a\r\nb");
    if !t.dirty[0] {
        return Err("linha com texto não está suja".to_string());
    }
    // o marcador de "tudo sujo" existe para resize/RIS
    t.dirty_all = false;
    t.resize(12, 5);
    if !t.dirty_all {
        return Err("resize não marcou a tela toda suja".to_string());
    }
    Ok(())
}
