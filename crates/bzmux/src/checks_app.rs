//! Checks de fumaça que usam o lado do app, mas **não** precisam de janela.
//!
//! O `bzmux-core` não conhece `cli.rs`, `keys.rs`, `mouse.rs`, `state.rs` nem
//! `debugsock.rs` — então o `checks::all()` do núcleo não alcança esses cinco,
//! e o `checks.py` do Python cobre os cinco (`argv_parsing`, `kitty_keyboard`,
//! `mouse_modes_and_sgr`, `window_state`, `debug_socket`). Esta lista fecha a
//! lacuna do `N1` sem display: mesma regra do núcleo — monta o cenário,
//! exercita, compara com o esperado, e falha dizendo o porquê.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;

use bzmux_core::checks::Check;
use winit::keyboard::{ModifiersState, NamedKey};

use crate::cli;
use crate::debugsock;
use crate::keys;
use crate::mouse;
use crate::state;

const NENHUM: ModifiersState = ModifiersState::empty();

/// O instalador e o ícone entram no binário: o check do atalho de desktop vale
/// também para o binário já instalado, longe da árvore do repositório.
const INSTALL_SH: &str = include_str!("../../../install.sh");
const ICONE: &str = include_str!("../../../assets/bzmux.svg");

pub fn all() -> Vec<Check> {
    vec![
        Check {
            name: "argv_parsing",
            run: argv_parsing,
        },
        Check {
            name: "kitty_keyboard",
            run: kitty_keyboard,
        },
        Check {
            name: "mouse_modes_and_sgr",
            run: mouse_modes_and_sgr,
        },
        Check {
            name: "window_state",
            run: window_state,
        },
        Check {
            name: "debug_socket",
            run: debug_socket,
        },
        Check {
            name: "desktop_shortcut",
            run: desktop_shortcut,
        },
    ]
}

fn argv(args: &[&str]) -> Result<cli::Overrides, String> {
    cli::parse(args.iter().map(|s| (*s).to_string()).collect())
}

fn argv_parsing() -> Result<(), String> {
    // `--help` e `--version` são informativas (0); erro de uso é 2 — inclusive
    // valor que não é número, que antes saía 0 e deixava script de automação
    // seguir com um comando inválido.
    let casos: [(&[&str], i32); 5] = [
        (&["--help"], 0),
        (&["-V"], 0),
        (&["--colo", "80"], 2),
        (&["--cols", "abc"], 2),
        (&["--cols"], 2),
    ];
    for (args, esperado) in casos {
        let msg = argv(args).err().unwrap_or_default();
        let obtido = cli::exit_code(&msg);
        if obtido != esperado {
            return Err(format!(
                "{}: código {obtido}, esperado {esperado}",
                args.join(" ")
            ));
        }
    }
    if argv(&["--help"]).is_ok() {
        return Err("--help devia sair pela mensagem, não virar opções".into());
    }
    let o = argv(&["--cols", "0", "--rows", "7"])?;
    if o.cols != Some(1) || o.rows != Some(7) {
        return Err(format!(
            "grade {:?}x{:?}: esperado 1x7 (coluna zero não existe)",
            o.cols, o.rows
        ));
    }
    // precedência CLI > arquivo > padrões, sem abrir janela
    let conf = std::env::temp_dir().join(format!("bzmux-check-{}.conf", std::process::id()));
    std::fs::write(&conf, "font_size 20\nscrollback 1234\n")
        .map_err(|e| format!("{}: {e}", conf.display()))?;
    let caminho = conf.to_string_lossy().to_string();
    let (sem_cli, _) = cli::Options::build(argv(&["--config", &caminho])?);
    let (com_cli, _) = cli::Options::build(argv(&[
        "--config",
        &caminho,
        "--font-size",
        "14",
        "--scrollback",
        "99",
    ])?);
    let _ = std::fs::remove_file(&conf);
    if (sem_cli.font_size_pt - 20.0).abs() > f32::EPSILON || sem_cli.scrollback != 1234 {
        return Err(format!(
            "arquivo ignorado: {:?} pt / scrollback {}",
            sem_cli.font_size_pt, sem_cli.scrollback
        ));
    }
    if (com_cli.font_size_pt - 14.0).abs() > f32::EPSILON || com_cli.scrollback != 99 {
        return Err(format!(
            "CLI não venceu o arquivo: {:?} pt / scrollback {}",
            com_cli.font_size_pt, com_cli.scrollback
        ));
    }
    Ok(())
}

fn tecla(named: Option<NamedKey>, ch: Option<&str>, mods: ModifiersState, flags: u32) -> String {
    String::from_utf8_lossy(
        &keys::encode_parts(named, ch, ch, mods, false, flags).unwrap_or_default(),
    )
    .to_string()
}

fn kitty_keyboard() -> Result<(), String> {
    // sem flag o caminho é o legado: ESC cru, ^A, F5 na forma do xterm
    let legado: [(&str, String); 3] = [
        ("escape", tecla(Some(NamedKey::Escape), None, NENHUM, 0)),
        ("ctrl+a", tecla(None, Some("a"), ModifiersState::CONTROL, 0)),
        ("f5", tecla(Some(NamedKey::F5), None, NENHUM, 0)),
    ];
    for (nome, obtido) in legado {
        let esperado = match nome {
            "escape" => "\x1b",
            "ctrl+a" => "\x01",
            _ => "\x1b[15~",
        };
        if obtido != esperado {
            return Err(format!("legado {nome}: {obtido:?}, esperado {esperado:?}"));
        }
    }
    let desambigua = keys::F_DISAMBIGUATE;
    if tecla(Some(NamedKey::Escape), None, NENHUM, desambigua) != "\x1b[27u" {
        return Err("flag 1 devia mandar ESC como CSI 27u".into());
    }
    if tecla(None, Some("a"), ModifiersState::CONTROL, desambigua) != "\x1b[97;5u" {
        return Err("flag 1 devia mandar Ctrl+a como CSI 97;5u".into());
    }
    // F3 tem forma própria no kitty: `CSI 1;2R`, e não `CSI 13;2~`
    if tecla(Some(NamedKey::F3), None, ModifiersState::SHIFT, desambigua) != "\x1b[1;2R" {
        return Err("Shift+F3 devia ser CSI 1;2R".into());
    }
    // flag 8 manda a letra sozinha também, e o maiúsculo vem do winit (65)
    let tudo = keys::F_ALL_KEYS;
    if tecla(None, Some("a"), NENHUM, tudo) != "\x1b[97u" {
        return Err("flag 8 devia mandar 'a' como CSI 97u".into());
    }
    if tecla(None, Some("A"), ModifiersState::SHIFT, tudo) != "\x1b[65;2u" {
        return Err("flag 8 + shift devia mandar CSI 65;2u".into());
    }
    // flag 16 embute o texto da tecla
    if tecla(None, Some("a"), NENHUM, tudo | keys::F_TEXT) != "\x1b[97;97u" {
        return Err("flag 16 devia embutir o texto".into());
    }
    Ok(())
}

fn rato(button: u8, col: usize, row: usize) -> mouse::Ev {
    mouse::Ev {
        button,
        col,
        row,
        pressed: true,
        motion: false,
        mods: 0,
    }
}

fn mouse_modes_and_sgr() -> Result<(), String> {
    let sgr = mouse::Proto {
        sgr: true,
        utf8: false,
    };
    let antigo = mouse::Proto::default();
    let casos: [(&str, Vec<u8>, &[u8]); 3] = [
        (
            "sgr press",
            mouse::encode(rato(mouse::LEFT, 9, 4), sgr),
            b"\x1b[<0;10;5M",
        ),
        (
            "sgr right release",
            mouse::encode(
                mouse::Ev {
                    pressed: false,
                    ..rato(mouse::RIGHT, 0, 0)
                },
                sgr,
            ),
            b"\x1b[<2;1;1m",
        ),
        (
            "antigo press",
            mouse::encode(rato(mouse::LEFT, 0, 0), antigo),
            b"\x1b[M\x20\x21\x21",
        ),
    ];
    for (nome, obtido, esperado) in casos {
        if obtido != esperado {
            return Err(format!("{nome}: {obtido:?}, esperado {esperado:?}"));
        }
    }
    // release de roda não existe no protocolo antigo (só no SGR)
    let roda = mouse::Ev {
        button: 64,
        col: 0,
        row: 0,
        pressed: false,
        motion: false,
        mods: 0,
    };
    if !mouse::encode(roda, antigo).is_empty() {
        return Err("release de roda devia sumir no protocolo antigo".into());
    }
    if mouse::encode(roda, sgr).is_empty() {
        return Err("release de roda devia sair no SGR".into());
    }
    // ?1005: a coordenada vai somada de 33 e codificada em UTF-8 — 300 vira
    // U+014D (`c5 8d`), enquanto o protocolo antigo satura em 222 (`ff`)
    let utf8 = mouse::Proto {
        sgr: false,
        utf8: true,
    };
    let longe = rato(mouse::LEFT, 300, 300);
    let bytes = mouse::encode(longe, utf8);
    if bytes != b"\x1b[M\x20\xc5\x8d\xc5\x8d" {
        return Err(format!("?1005 com coluna 300: {bytes:?}"));
    }
    let saturado = mouse::encode(longe, antigo);
    if saturado != b"\x1b[M\x20\xff\xff" {
        return Err(format!("antigo devia saturar em 222: {saturado:?}"));
    }
    Ok(())
}

fn window_state() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("bzmux-check-state-{}", std::process::id()));
    let arq = dir.join("window.json");
    let _ = std::fs::remove_file(&arq);
    let g = state::Geometry {
        x: Some(-120),
        y: Some(40),
        w: 1131,
        h: 656,
        maximizado: false,
    };
    state::save_to(&arq, &g);
    match state::load_from(&arq) {
        Some(lido) if lido == g => {},
        outro => return Err(format!("ida e volta perdeu a geometria: {outro:?}")),
    }
    // arquivo torto não pode virar pânico nem geometria inventada
    std::fs::write(&arq, "{ isto não é json").map_err(|e| e.to_string())?;
    if state::load_from(&arq).is_some() {
        return Err("arquivo torto devia ser ignorado".into());
    }
    let _ = std::fs::remove_dir_all(&dir);
    // `fit`: canto tem de estar num monitor e o tamanho tem de ser sadio
    let monitores = [(0, 0, 1920, 1080)];
    if state::fit(Some(100), Some(100), 980, 640, &monitores).is_none() {
        return Err("geometria sadia foi recusada".into());
    }
    if state::fit(Some(100), Some(100), state::MIN_W - 1, 640, &monitores).is_some() {
        return Err("janela estreita demais devia ser recusada".into());
    }
    if state::fit(Some(5000), Some(5000), 980, 640, &monitores).is_some() {
        return Err("canto fora de todo monitor devia ser recusado".into());
    }
    // Wayland (J6): sem posição, o tamanho sozinho continua valendo
    match state::fit(None, None, 900, 500, &monitores) {
        Some(g) if g.x.is_none() && g.w == 900 => Ok(()),
        outro => Err(format!("sem posição devia manter o tamanho: {outro:?}")),
    }
}

/// Sink de mentira que responde como o app: JSON com `cmd` vira `ok`, o resto
/// vira `err`. O socket em si não sabe que o winit existe — é essa a costura
/// que permite exercitá-lo aqui.
// a assinatura é a do trait `Sink` (que recebe a linha e o canal por valor),
// então o `allow` fica aqui e não no topo do crate
#[allow(clippy::needless_pass_by_value)]
fn sink_de_teste(linha: String, resposta: mpsc::Sender<String>) -> bool {
    let reply = match serde_json::from_str::<serde_json::Value>(&linha) {
        Ok(v) if v.get("cmd").is_some() => debugsock::ok(serde_json::json!({ "cmd": v["cmd"] })),
        Ok(_) => debugsock::err("falta o campo cmd"),
        Err(e) => debugsock::err(format!("json: {e}")),
    };
    resposta.send(reply).is_ok()
}

fn debug_socket() -> Result<(), String> {
    let caminho = std::env::temp_dir()
        .join(format!("bzmux-check-sock-{}.sock", std::process::id()))
        .to_string_lossy()
        .to_string();
    let _ = std::fs::remove_file(&caminho);
    let srv = debugsock::DebugServer::start(
        &caminho,
        sink_de_teste as fn(String, mpsc::Sender<String>) -> bool,
    )
    .map_err(|e| format!("start: {e}"))?;
    // modo 0600: o terminal de outro usuário não pode ler a tela
    {
        use std::os::unix::fs::PermissionsExt;
        let modo = std::fs::metadata(&caminho)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o777;
        if modo != 0o600 {
            return Err(format!("socket com modo {modo:o}, esperado 600"));
        }
    }
    let mut s = UnixStream::connect(&caminho).map_err(|e| format!("connect: {e}"))?;
    let mut f = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
    let mut ler = |esperado: &str| -> Result<String, String> {
        let mut l = String::new();
        f.read_line(&mut l).map_err(|e| e.to_string())?;
        if !l.contains(esperado) {
            return Err(format!("resposta {l:?} não tem {esperado:?}"));
        }
        Ok(l)
    };
    s.write_all(b"{\"cmd\":\"state\"}\n")
        .map_err(|e| e.to_string())?;
    ler("state")?;
    // pedido torto responde erro **e a conexão continua**: quem chama o socket
    // precisa do motivo, não de um EOF
    s.write_all(b"nao sou json\n").map_err(|e| e.to_string())?;
    ler("\"ok\":false")?;
    s.write_all(b"{\"cmd\":\"text\"}\n")
        .map_err(|e| e.to_string())?;
    ler("text")?;
    drop(srv);
    if std::path::Path::new(&caminho).exists() {
        return Err("socket devia sair do disco ao parar".into());
    }
    Ok(())
}

fn desktop_shortcut() -> Result<(), String> {
    // o `install.sh` é o dono do atalho (K18): o check cobra o contrato, não a
    // implementação — nome, Exec com o binário, ícone e categoria de terminal
    for campo in [
        "Exec=$BINDIR/$APP",
        "TryExec=$BINDIR/$APP",
        "Icon=$APP",
        "Categories=System;",
        "TerminalEmulator;",
    ] {
        if !INSTALL_SH.contains(campo) {
            return Err(format!("install.sh não escreve {campo:?} no .desktop"));
        }
    }
    if !INSTALL_SH.contains("share/applications/bzmux.desktop") {
        return Err("install.sh não instala o .desktop no prefixo".into());
    }
    // desinstalar tem de levar junto o cromo instalado
    let desinstala = INSTALL_SH
        .split("do_uninstall()")
        .nth(1)
        .ok_or("install.sh sem do_uninstall")?;
    for alvo in ["$DESKTOP", "$ICON"] {
        if !desinstala.contains(alvo) || !desinstala.contains("rm -f") {
            return Err(format!("desinstalar não remove {alvo}"));
        }
    }
    if !ICONE.contains("<svg") || !ICONE.contains("viewBox") {
        return Err("assets/bzmux.svg não é um SVG válido".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// O mesmo contrato do `selftest_completo` do núcleo: `cargo test` verde tem
    /// de significar checks verdes, senão eles só existem no binário.
    #[test]
    fn checks_do_app_passam() {
        let falhas: Vec<String> = super::all()
            .into_iter()
            .filter_map(|c| (c.run)().err().map(|e| format!("{}: {e}", c.name)))
            .collect();
        assert!(falhas.is_empty(), "{}", falhas.join("\n"));
    }
}
