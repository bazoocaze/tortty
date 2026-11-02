//! N2: configuração e linha de comando (equivalente ao `checks.py:125-287`).
//!
//! Um check monta o cenário, exercita e compara com o esperado. Nada aqui
//! precisa de janela, GPU ou pty.
//!
//! O que **não** tem equivalente no Rust fica de fora de propósito: o Python
//! valida `shell` executável e resolve `effective_shell`/`tab_command` no
//! `config.py`, enquanto aqui o `shell` só é resolvido no `pty::spawn` (precisa
//! de pty e de `$SHELL`), e o Python tem a noção de *warning* (chave repetida),
//! que o Rust não tem — aqui só existe `Error`.

use crate::checks::Check;
use crate::config::{Bell, Config, CursorShape};
use crate::grid::Row;
use crate::term::Term;

/// Checks desta área, na ordem do relatório.
#[must_use]
pub fn all() -> Vec<Check> {
    vec![
        Check {
            name: "config_file",
            run: config_file,
        },
        Check {
            name: "config_options",
            run: config_options,
        },
        Check {
            name: "scrollback_and_bell",
            run: scrollback_and_bell,
        },
    ]
}

/// Todas as chaves de `config::KNOWN` (que é privado). A lista existe para o
/// `DEFAULT_CONF` não perder uma chave em silêncio: os padrões são a única
/// fonte dos valores, então toda chave precisa estar escrita lá.
const CHAVES: &[&str] = &[
    "shell",
    "font_family",
    "font_size",
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

/// `checks.py:config_file` — formato do kitty, merge com os padrões embutidos
/// e validação com erro por linha.
fn config_file() -> Result<(), String> {
    // os padrões embarcam no binário como arquivo (o mesmo tratamento do
    // `DEFAULT_CONF` do kitty): parseiam limpos e são a única fonte
    let (padroes, errs) = Config::parse("");
    if !errs.is_empty() {
        return Err(format!("DEFAULT_CONF tem erro: {errs:?}"));
    }
    esperado(
        "padrões = parse do DEFAULT_CONF",
        padroes.clone(),
        Config::base(),
    )?;
    let linhas: Vec<&str> = crate::config::DEFAULT_CONF
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    for k in CHAVES {
        if !linhas
            .iter()
            .any(|l| l.split_whitespace().next() == Some(*k))
        {
            return Err(format!("DEFAULT_CONF não documenta a chave '{k}'"));
        }
    }
    // vazio = herda o $SHELL do ambiente (o pty resolve; aqui é só `None`)
    esperado("shell vazio", padroes.shell, None)?;
    // comentário de linha inteira e no fim da linha, espaço nas pontas, chave
    // repetida (a última vence) e chave desconhecida no meio
    let (cfg, errs) =
        Config::parse("# cabeçalho\n  shell   /bin/sh   # inline\n\nfoo bar\nshell /bin/bash\n");
    esperado(
        "última repetida vence",
        cfg.shell.as_deref(),
        Some("/bin/bash"),
    )?;
    esperado("uma linha inválida", errs.len(), 1)?;
    esperado(
        "erro na linha do 'foo'",
        (errs[0].line, errs[0].key.as_str()),
        (4, "foo"),
    )?;
    // merge chave a chave: o usuário só sobrescreve o que escreveu
    let (cfg, errs) = Config::parse("shell /bin/sh\n");
    esperado("shell do usuário", cfg.shell.as_deref(), Some("/bin/sh"))?;
    esperado(
        "fonte segue a de fábrica",
        cfg.font_family.as_str(),
        "Monospace",
    )?;
    esperado("sem arquivo, sem erro", errs.len(), 0)?;
    // chave desconhecida é erro, mas o resto do arquivo VALE (carga inicial)
    let (cfg, errs) = Config::parse("foo bar\nshell /bin/sh\n");
    esperado("o resto aplicou", cfg.shell.as_deref(), Some("/bin/sh"))?;
    esperado("chave desconhecida é erro", errs.len(), 1)?;
    // valor inválido fica no padrão (a chave não aplica) e o erro vai pro toast
    let (cfg, errs) = Config::parse("# nada aqui\nfont_size 200\n");
    esperado("inválido não aplica", cfg.font_size, padroes.font_size)?;
    esperado("erro na linha 2", errs.first().map(|e| e.line), Some(2))?;
    if !errs.iter().any(|e| e.msg.contains("fora da faixa")) {
        return Err(format!("erro sem o motivo da faixa: {errs:?}"));
    }
    // reload: quem chama só aplica com `errs` vazio, então qualquer erro
    // mantém a configuração que já estava valendo
    let (bom, _) = Config::parse("font_size 16\n");
    let (_, errs) = bom.over("font_size 18\nbell grita\n");
    if errs.len() != 1 {
        return Err(format!("reload inválido tinha que dar 1 erro: {errs:?}"));
    }
    esperado("a base não é mutada pelo reload", bom.font_size, 16.0)?;
    Ok(())
}

/// `checks.py:config_options` — fonte, cursor, histórico, padding, bell,
/// `copy_on_select` e `debug_log`, chave a chave: padrão, valor válido e erro.
fn config_options() -> Result<(), String> {
    // sem arquivo do usuário valem os padrões
    let (cfg, errs) = Config::parse("");
    if !errs.is_empty() {
        return Err(format!("padrões com erro: {errs:?}"));
    }
    esperado("font_family", cfg.font_family.as_str(), "Monospace")?;
    esperado("font_size", cfg.font_size, 12.0)?;
    esperado("cursor_shape", cfg.cursor_shape, CursorShape::Block)?;
    esperado("cursor_blink", cfg.cursor_blink, false)?;
    esperado("scrollback", cfg.scrollback, 10_000)?;
    esperado("window_padding_width", cfg.padding, 0)?;
    esperado("bell", cfg.bell, Bell::None)?;
    esperado("copy_on_select", cfg.copy_on_select, false)?;
    esperado("debug_log", cfg.debug_log.as_deref(), None)?;
    // valor válido de cada chave: família com espaço e fonte fracionária
    let (cfg, errs) = Config::parse(
        "font_family DejaVu Sans Mono\nfont_size 12.5\ncursor_shape beam\n\
         cursor_blink yes\nscrollback 200\nwindow_padding_width 4\n\
         bell visual\ncopy_on_select on\n",
    );
    if !errs.is_empty() {
        return Err(format!("valores válidos deram erro: {errs:?}"));
    }
    esperado(
        "família com espaço",
        cfg.font_family.as_str(),
        "DejaVu Sans Mono",
    )?;
    esperado("fonte fracionária", cfg.font_size, 12.5)?;
    esperado("cursor beam", cfg.cursor_shape, CursorShape::Beam)?;
    esperado("blink yes", cfg.cursor_blink, true)?;
    esperado("scrollback", cfg.scrollback, 200)?;
    esperado("padding", cfg.padding, 4)?;
    esperado("bell visual", cfg.bell, Bell::Visual)?;
    esperado("copy_on_select on", cfg.copy_on_select, true)?;
    // debug_log: caminho livre e `off` desligando
    let (cfg, errs) = Config::parse("debug_log /tmp/outro.log\n");
    esperado(
        "debug_log caminho",
        cfg.debug_log.as_deref(),
        Some("/tmp/outro.log"),
    )?;
    esperado("debug_log sem erro", errs.len(), 0)?;
    let (cfg, errs) = Config::parse("debug_log off\n");
    esperado("debug_log off", cfg.debug_log.as_deref(), None)?;
    esperado("debug_log off sem erro", errs.len(), 0)?;
    // erro é por linha: o válido aplica, o inválido fica no padrão anterior
    for (lixo, motivo) in [
        ("font_size abc", "não é número"),
        ("font_size 0", "fora da faixa"),
        ("font_family", "família vazia"),
        ("cursor_shape square", "não é block"),
        ("cursor_blink talvez", "não é on/off"),
        ("scrollback -1", "não é número"),
        ("window_padding_width 8px", "não é número"),
        ("bell beep", "não é none"),
        ("copy_on_select 2", "não é on/off"),
    ] {
        let (cfg, errs) = Config::parse(&format!("font_size 20\n{lixo}\n"));
        esperado("a linha válida antes do erro aplicou", cfg.font_size, 20.0)?;
        match errs.as_slice() {
            [e] if e.line == 2 && e.msg.contains(motivo) => {},
            outro => {
                return Err(format!(
                    "'{lixo}': esperava 1 erro na linha 2 com '{motivo}', veio {outro:?}"
                ));
            },
        }
    }
    // a carga inicial mostra primeiro erro + contagem dos outros; aqui a lista
    // sai em ordem de linha e o válido no meio aplica
    let (cfg, errs) = Config::parse("bell beep\nfont_size 20\ncursor_shape square\n");
    esperado("bell ficou no padrão", cfg.bell, Bell::None)?;
    esperado("font_size do meio aplicou", cfg.font_size, 20.0)?;
    esperado(
        "um erro por linha",
        errs.iter().map(|e| e.line).collect::<Vec<_>>(),
        vec![1, 3],
    )?;
    if !errs[0].msg.contains("não é none") || !errs[1].msg.contains("não é block") {
        return Err(format!("motivos errados: {errs:?}"));
    }
    Ok(())
}

/// `checks.py:scrollback_and_bell` — teto de histórico configurável, RIS não é
/// dono da configuração, BEL contado (não silenciado) e DECSCUSR virando
/// pedido do app.
fn scrollback_and_bell() -> Result<(), String> {
    let mut t = Term::new(12, 3);
    t.scrollback_limit = 2;
    for i in 0..20 {
        t.feed(format!("linha {i}\r\n").as_bytes());
    }
    // corta pela mais antiga, não pela que chega
    esperado("teto do histórico", t.scrollback.len(), 2)?;
    esperado(
        "as duas últimas que saíram",
        t.scrollback.iter().map(Row::text).collect::<Vec<_>>(),
        vec!["linha 16".to_string(), "linha 17".to_string()],
    )?;
    // o vivo não é afetado pelo teto
    esperado("linha viva 18", t.row_text(0), "linha 18".to_string())?;
    esperado("linha viva 19", t.row_text(1), "linha 19".to_string())?;
    // reduzir o teto corta na rolagem seguinte (o corte imediato é do app,
    // que mexe no `scrollback_limit` da aba e poda o `VecDeque` na hora)
    t.scrollback_limit = 0;
    t.feed(b"x\r\n");
    esperado("teto zero esvazia", t.scrollback.len(), 0)?;
    t.scrollback_limit = 2;
    for i in 0..10 {
        t.feed(format!("linha {i}\r\n").as_bytes());
    }
    esperado("histórico encheu de novo", t.scrollback.len(), 2)?;
    // DECRQM tem que enxergar os modos de mouse que existem: `?1005` estava
    // fora da tabela e respondia 0 com o modo ligado
    let mut q = Term::new(10, 3);
    q.feed(b"\x1b[?1005h");
    q.take_child_out();
    q.feed(b"\x1b[?1005$p");
    esperado(
        "DECRQM 1005 ligado",
        q.take_child_out(),
        b"\x1b[?1005;1$y".to_vec(),
    )?;
    q.feed(b"\x1b[?1005l");
    q.take_child_out();
    q.feed(b"\x1b[?1005$p");
    esperado(
        "DECRQM 1005 desligado",
        q.take_child_out(),
        b"\x1b[?1005;2$y".to_vec(),
    )?;
    // RIS (ESC c) limpa a tela mas não é dono da configuração
    t.feed(b"\x1bc");
    esperado("RIS preserva o teto", t.scrollback_limit, 2)?;
    esperado("RIS limpa o histórico", t.scrollback.len(), 0)?;
    // BEL é contado, não silenciado: quem decide (visual/audible/none) é a view
    esperado("sem BEL ainda", t.bells, 0)?;
    t.feed(b"\x07");
    esperado("BEL contado", t.bells, 1)?;
    t.feed(b"\x07\x07");
    esperado("BEL acumula", t.bells, 3)?;
    // `bell none` na config não impede a contagem: o emulador só conta
    let (cfg, errs) = Config::parse("bell none\n");
    esperado("bell none", (cfg.bell, errs.len()), (Bell::None, 0))?;
    t.feed(b"\x07");
    esperado("config não silencia o contador", t.bells, 4)?;
    // DECSCUSR (CSI 6 q) vira pedido do app; 0 volta para a config
    // (0 = block, 1 = underline, 2 = beam; o bool é o "pisca")
    t.feed(b"\x1b[4 q");
    esperado(
        "cursor underline fixo",
        t.app_cursor_style,
        Some((1, false)),
    )?;
    t.feed(b"\x1b[5 q");
    esperado("cursor beam piscando", t.app_cursor_style, Some((2, true)))?;
    t.feed(b"\x1b[0 q");
    esperado("CSI 0 q volta para a config", t.app_cursor_style, None)?;
    Ok(())
}
