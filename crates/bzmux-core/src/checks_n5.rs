//! N5: histórico, âncora, dano e seleção (equivalente ao `checks.py:409-640`).
//!
//! Um check monta o cenário, exercita e compara com o esperado. Nada aqui
//! precisa de janela, GPU ou pty.
//!
//! O que o Python já tem coberto por outro check daqui (`paste_markers`,
//! `selection_follows_history`) não aparece de novo: o objetivo é cobrir o que
//! falta, não duplicar nome nem asserção.

use crate::checks::Check;
use crate::color::Color;
use crate::config::Theme;
use crate::grid::Row;
use crate::term::Term;

/// Checks desta área, na ordem do relatório.
#[must_use]
pub fn all() -> Vec<Check> {
    vec![
        Check {
            name: "history",
            run: history,
        },
        Check {
            name: "scroll_anchor",
            run: scroll_anchor,
        },
        Check {
            name: "redraw_keeps_history_position",
            run: redraw_keeps_history_position,
        },
        Check {
            name: "draw_damage",
            run: draw_damage,
        },
        Check {
            name: "selection",
            run: selection,
        },
        Check {
            name: "truecolor",
            run: truecolor,
        },
        Check {
            name: "style_contrast",
            run: style_contrast,
        },
    ]
}

// por valor de propósito: quem chama monta o esperado na hora (`linhas(&["a"])`)
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

fn linhas(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_string()).collect()
}

/// Texto das `rows` linhas da visão com `off` linhas de histórico.
fn texto_da_visao(t: &Term, off: usize) -> Vec<String> {
    t.visible_rows(off).iter().map(|r| r.text()).collect()
}

/// `off` que mantém a linha absoluta `linha` no topo da visão.
///
/// É a conta do `anchored_view_off` do Python: `scrolled` é índice absoluto e
/// nunca volta — nem quando o histórico bate no teto e descarta linha velha —
/// então a conta absoluta não derrapa linha a linha como a diferença de
/// tamanho derrapava. O clamp é no tamanho do histórico: se a linha morreu
/// (CSI 3J) não há o que segurar e a visão volta para o vivo.
fn off_da_ancora(t: &Term, linha: i64) -> usize {
    (t.scrolled as i64 - linha).clamp(0, t.scrollback.len() as i64) as usize
}

/// Dano no molde do Python: `None` = a grade inteira, `Some(linhas)` = só elas.
///
/// O núcleo guarda o dano como `dirty`/`dirty_all`; consumir é papel do draw,
/// então a leitura que zera mora aqui (o app faz o mesmo em `app.rs`).
fn take_damage(t: &mut Term) -> Option<Vec<usize>> {
    if t.dirty_all {
        t.clear_damage();
        return None;
    }
    let linhas: Vec<usize> = t
        .dirty
        .iter()
        .enumerate()
        .filter(|(_, d)| **d)
        .map(|(y, _)| y)
        .collect();
    t.clear_damage();
    Some(linhas)
}

fn history() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    t.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\n");
    // a tela é uma janela: `off`=0 é o vivo, `off`>0 puxa o histórico para cima
    esperado("vivo", texto_da_visao(&t, 0), linhas(&["d", "e", ""]))?;
    esperado(
        "uma linha acima",
        texto_da_visao(&t, 1),
        linhas(&["c", "d", "e"]),
    )?;
    esperado("três linhas fora do topo", t.scrolled, 3)?;
    // a janela nunca passa de `rows` linhas (regressão: `off` > rows devolvia
    // `off` linhas e o desenho ficava mais lento quanto mais se subia)
    for off in [0, 1, t.rows, t.rows + 1, t.scrollback.len()] {
        esperado(
            &format!("janela com off={off}"),
            t.visible_rows(off).len(),
            t.rows,
        )?;
    }
    // fundo do histórico: pedir demais clampa no mais antigo, não no vivo
    esperado(
        "fundo do histórico",
        texto_da_visao(&t, 99),
        linhas(&["a", "b", "c"]),
    )?;
    esperado("histórico tem 3", t.scrollback.len(), 3)?;
    Ok(())
}

fn scroll_anchor() -> Result<(), String> {
    let mut t = Term::new(10, 3);
    for i in 0..10 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    // o usuário subiu duas linhas e parou de ler: a âncora é a linha do topo
    let ancora = t.scrolled as i64 - 2;
    let topo = t.visible_rows(2)[0].text();
    esperado("topo lido", topo.clone(), "6".to_string())?;
    let (antes_scrolled, antes_hist) = (t.scrolled, t.scrollback.len());
    for i in 10..15 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    // o output não arranca a visão da linha que se lê
    esperado(
        "a visão acompanha o output",
        t.visible_rows(off_da_ancora(&t, ancora))[0].text(),
        topo.clone(),
    )?;
    // e a conta incremental do app (D10) chega no mesmo offset
    esperado(
        "view_after_output",
        t.view_after_output(2, antes_scrolled, antes_hist),
        off_da_ancora(&t, ancora),
    )?;
    // no teto do histórico o `len()` para de crescer: a conta por diferença de
    // tamanho deixaria o texto derrapar; a absoluta não
    for i in 15..1100 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    esperado("histórico no teto", t.scrollback.len(), 1000)?;
    let ancora = t.scrolled as i64 - 3;
    let topo = t.visible_rows(3)[0].text();
    let (teto_scrolled, teto_hist) = (t.scrolled, t.scrollback.len());
    for i in 1100..1110 {
        t.feed(format!("{i}\r\n").as_bytes());
    }
    let off = off_da_ancora(&t, ancora);
    esperado("offset absoluto", off, 13)?;
    // a conta incremental do app tem que dar o mesmo **no teto**: era aqui que
    // ela subtraía as linhas descartadas e a visão derrapava — o leitor perdia
    // uma linha de conteúdo por linha nova, o oposto do D10
    esperado(
        "view_after_output no teto",
        t.view_after_output(3, teto_scrolled, teto_hist),
        off,
    )?;
    esperado(
        "mesma linha no topo",
        t.visible_rows(off)[0].text(),
        topo.clone(),
    )?;
    // a conta antiga (offset fixo em 3) mostraria outra linha: 10 de deriva
    if t.visible_rows(3)[0].text() == topo {
        return Err(
            "offset fixo em 3 devolveu a mesma linha: o teste não separa as contas".to_string(),
        );
    }
    // limpar o histórico (CSI 3J) destrói a âncora: não há o que ler, vivo
    t.feed(b"\x1b[3J");
    esperado("ED3 devolve o vivo", off_da_ancora(&t, ancora), 0)?;
    Ok(())
}

fn redraw_keeps_history_position() -> Result<(), String> {
    let mut t = Term::new(12, 4);
    t.scrollback_limit = 200;
    for i in 1..=40 {
        t.feed(format!("l{i:02}\r\n").as_bytes());
    }
    let off = 12usize; // o usuário rolou 12 linhas para trás
    let ancora = t.scrolled as i64 - off as i64;
    let antes = t.visible_rows(off)[0].text();
    esperado("linha lida", antes.clone(), "l26".to_string())?;
    let gen = t.hist_gen;
    // exatamente o começo do fullRender(clear) do pi
    t.feed(b"\x1b[?2026h\x1b[2J\x1b[H\x1b[3J");
    if !t.sync_output {
        return Err("?2026 não entrou em modo de saída sincronizada".to_string());
    }
    esperado("geração do histórico", t.hist_gen, gen + 1)?;
    esperado("3J esvazia o histórico", t.scrollback.len(), 0)?;
    // a âncora absoluta morreu: sem o pin, o clamp devolveria o vivo e, com o
    // buffer reescrito, a visão escoraria no topo (era o bug do B20)
    esperado("âncora absoluta morre", off_da_ancora(&t, ancora), 0)?;
    for i in 1..=40 {
        t.feed(format!("l{i:02}\r\n").as_bytes());
    }
    t.feed(b"\x1b[?2026l");
    if t.sync_output {
        return Err("?2026l não desligou a saída sincronizada".to_string());
    }
    if t.scrollback.len() < off {
        return Err(format!(
            "o app não reescreveu buffer suficiente: histórico com {}",
            t.scrollback.len()
        ));
    }
    // a distância do vivo sobrevive, mesmo com a âncora absoluta morta
    let pin = off.min(t.scrollback.len());
    esperado("distância preservada", pin, off)?;
    esperado(
        "a linha lida volta",
        t.visible_rows(pin)[0].text(),
        antes.clone(),
    )?;
    // já a âncora absoluta dá o topo do histórico novo — era o bug
    esperado(
        "âncora absoluta no topo",
        off_da_ancora(&t, ancora),
        t.scrollback.len(),
    )?;
    if t.visible_rows(t.scrollback.len())[0].text() == antes {
        return Err("a âncora absoluta devolveu a linha lida".to_string());
    }
    Ok(())
}

fn draw_damage() -> Result<(), String> {
    let mut t = Term::new(10, 5);
    // o primeiro draw é sempre a grade inteira
    if take_damage(&mut t).is_some() {
        return Err("primeiro draw deveria ser a grade inteira".to_string());
    }
    esperado("dano consumido", take_damage(&mut t), Some(Vec::new()))?;
    // um caractere numa linha: só ela
    t.feed(b"\x1b[3;1Hx");
    esperado("1 célula suja 1 linha", take_damage(&mut t), Some(vec![2]))?;
    esperado("dano consumido", take_damage(&mut t), Some(Vec::new()))?;
    // texto corrido em duas linhas
    t.feed(b"\x1b[1;1Habcdefghij\x1b[2;1Hyz");
    esperado("duas linhas", take_damage(&mut t), Some(vec![0, 1]))?;
    // EL e ECH sujam só a linha corrente
    t.feed(b"\x1b[2;3H\x1b[K");
    esperado("EL", take_damage(&mut t), Some(vec![1]))?;
    t.feed(b"\x1b[2;1H\x1b[2X");
    esperado("ECH", take_damage(&mut t), Some(vec![1]))?;
    // ED2 muda a tela toda
    t.feed(b"\x1b[2J");
    if take_damage(&mut t).is_some() {
        return Err("ED2 deveria sujar a tela toda".to_string());
    }
    // rolagem, resize e RIS: a grade inteira (as linhas mudam de posição)
    t.feed(b"\x1b[5;1H\n");
    if take_damage(&mut t).is_some() {
        return Err("rolagem move toda linha: dano tinha que ser a grade".to_string());
    }
    t.resize(8, 4);
    if take_damage(&mut t).is_some() {
        return Err("reflow move toda linha: dano tinha que ser a grade".to_string());
    }
    t.feed(b"\x1bc");
    if take_damage(&mut t).is_some() {
        return Err("RIS recomeça a grade: dano tinha que ser a grade".to_string());
    }
    Ok(())
}

fn selection() -> Result<(), String> {
    let mut t = Term::new(20, 5);
    t.feed(b"abc\r\ndef\r\nghi");
    let rows: Vec<&Row> = t.grid.iter().collect();
    esperado(
        "multi-linha",
        t.selection_text((0, 1), (1, 1), &rows),
        "bc\nde".to_string(),
    )?;
    esperado(
        "uma linha só",
        t.selection_text((0, 0), (0, 2), &rows),
        "abc".to_string(),
    )?;
    // arrastar de baixo para cima é normal: a ordem é normalizada
    esperado(
        "ordem invertida",
        t.selection_text((1, 1), (0, 1), &rows),
        "bc\nde".to_string(),
    )?;
    // pontas inclusivas e a coluna final entra inteira
    esperado(
        "coluna final",
        t.selection_text((0, 2), (1, 0), &rows),
        "c\nd".to_string(),
    )?;
    // espaço à direita de cada linha sai fora (é o que todo terminal copia)
    esperado(
        "sem espaço à direita",
        t.selection_text((0, 0), (0, 9), &rows),
        "abc".to_string(),
    )?;
    esperado(
        "uma célula",
        t.selection_text((2, 1), (2, 1), &rows),
        "h".to_string(),
    )?;
    Ok(())
}

fn truecolor() -> Result<(), String> {
    let mut t = Term::new(20, 5);
    t.feed(b"\x1b[38;2;18;52;86mA\x1b[48;2;250;128;114mB");
    esperado(
        "fg RGB na célula",
        t.grid[0].cells[0].fg,
        Color::Rgb(18, 52, 86),
    )?;
    esperado(
        "bg RGB na célula",
        t.grid[0].cells[1].bg,
        Color::Rgb(250, 128, 114),
    )?;
    // a forma com ':' (subparâmetros) tem que dar o mesmo RGB
    t.feed(b"\x1b[38:2::7:8:9mC");
    esperado("forma com ':'", t.grid[0].cells[2].fg, Color::Rgb(7, 8, 9))?;
    // 256 é índice, não RGB: o desenho resolve pela paleta do tema
    let mut t = Term::new(20, 5);
    t.feed(b"\x1b[38;5;196mA\x1b[0mB");
    esperado("índice 196", t.grid[0].cells[0].fg, Color::Idx(196))?;
    esperado("reset volta o fg", t.grid[0].cells[1].fg, Color::Default)?;
    esperado("reset volta o bg", t.grid[0].cells[1].bg, Color::Default)?;
    // a cor mora na célula: reflow não decodifica nem perde o RGB
    let mut t = Term::new(4, 2);
    t.feed(b"\x1b[38;2;1;2;3mXYZW");
    t.resize(8, 2);
    esperado(
        "RGB sobrevive ao resize",
        t.grid[0].cells[0].fg,
        Color::Rgb(1, 2, 3),
    )?;
    Ok(())
}

fn style_contrast() -> Result<(), String> {
    // Mesma conta do `style.contrast` do Python (WCAG 2.1): luminância
    // relativa e razão (L1+0.05)/(L2+0.05). O cromo da GUI (cards, abas,
    // toast) não existe no núcleo — o que existe aqui é o tema do terminal.
    fn canal(v: u8) -> f64 {
        let v = f64::from(v) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    fn luminancia((r, g, b): (u8, u8, u8)) -> f64 {
        0.2126 * canal(r) + 0.7152 * canal(g) + 0.0722 * canal(b)
    }
    fn contraste(fg: (u8, u8, u8), bg: (u8, u8, u8)) -> f64 {
        let (a, b) = (luminancia(fg), luminancia(bg));
        let (claro, escuro) = if a > b { (a, b) } else { (b, a) };
        (claro + 0.05) / (escuro + 0.05)
    }
    let tema = Theme::default();
    let fundo = tema.rgb(tema.bg, (0, 0, 0));
    // texto normal pede 4.5:1; cursor é componente de UI (WCAG 1.4.11: 3:1)
    let pares = [
        ("fg sobre o fundo", tema.fg, 4.5),
        ("cursor sobre o fundo", tema.cursor, 3.0),
    ];
    for (o_que, cor, minimo) in pares {
        let razao = contraste(tema.rgb(cor, fundo), fundo);
        if razao < minimo {
            return Err(format!(
                "{o_que}: contraste {razao:.2} abaixo do mínimo {minimo:.1}"
            ));
        }
    }
    Ok(())
}
