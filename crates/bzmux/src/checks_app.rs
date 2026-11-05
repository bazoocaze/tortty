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

const NONE: ModifiersState = ModifiersState::empty();

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
    let cases: [(&[&str], i32); 5] = [
        (&["--help"], 0),
        (&["-V"], 0),
        (&["--colo", "80"], 2),
        (&["--cols", "abc"], 2),
        (&["--cols"], 2),
    ];
    for (args, esperado) in cases {
        let msg = argv(args).err().unwrap_or_default();
        let obtained = cli::exit_code(&msg);
        if obtained != esperado {
            return Err(format!(
                "{}: código {obtained}, esperado {esperado}",
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
    let path = conf.to_string_lossy().to_string();
    let (without_cli, _) = cli::Options::build(argv(&["--config", &path])?);
    let (with_cli, _) = cli::Options::build(argv(&[
        "--config",
        &path,
        "--font-size",
        "14",
        "--scrollback",
        "99",
    ])?);
    let _ = std::fs::remove_file(&conf);
    if (without_cli.font_size_pt - 20.0).abs() > f32::EPSILON || without_cli.scrollback != 1234 {
        return Err(format!(
            "arquivo ignorado: {:?} pt / scrollback {}",
            without_cli.font_size_pt, without_cli.scrollback
        ));
    }
    if (with_cli.font_size_pt - 14.0).abs() > f32::EPSILON || with_cli.scrollback != 99 {
        return Err(format!(
            "CLI não venceu o arquivo: {:?} pt / scrollback {}",
            with_cli.font_size_pt, with_cli.scrollback
        ));
    }
    Ok(())
}

fn key(named: Option<NamedKey>, ch: Option<&str>, mods: ModifiersState, flags: u32) -> String {
    String::from_utf8_lossy(
        &keys::encode_parts(named, ch, ch, mods, false, flags).unwrap_or_default(),
    )
    .to_string()
}

fn kitty_keyboard() -> Result<(), String> {
    // sem flag o caminho é o legado: ESC cru, ^A, F5 na forma do xterm
    let legado: [(&str, String); 3] = [
        ("escape", key(Some(NamedKey::Escape), None, NONE, 0)),
        ("ctrl+a", key(None, Some("a"), ModifiersState::CONTROL, 0)),
        ("f5", key(Some(NamedKey::F5), None, NONE, 0)),
    ];
    for (name, obtained) in legado {
        let esperado = match name {
            "escape" => "\x1b",
            "ctrl+a" => "\x01",
            _ => "\x1b[15~",
        };
        if obtained != esperado {
            return Err(format!(
                "legado {name}: {obtained:?}, esperado {esperado:?}"
            ));
        }
    }
    let disambiguate = keys::F_DISAMBIGUATE;
    if key(Some(NamedKey::Escape), None, NONE, disambiguate) != "\x1b[27u" {
        return Err("flag 1 devia mandar ESC como CSI 27u".into());
    }
    if key(None, Some("a"), ModifiersState::CONTROL, disambiguate) != "\x1b[97;5u" {
        return Err("flag 1 devia mandar Ctrl+a como CSI 97;5u".into());
    }
    // F3 tem forma própria no kitty: `CSI 1;2R`, e não `CSI 13;2~`
    if key(
        Some(NamedKey::F3),
        None,
        ModifiersState::SHIFT,
        disambiguate,
    ) != "\x1b[1;2R"
    {
        return Err("Shift+F3 devia ser CSI 1;2R".into());
    }
    // flag 8 manda a letra sozinha também, e o maiúsculo vem do winit (65)
    let all = keys::F_ALL_KEYS;
    if key(None, Some("a"), NONE, all) != "\x1b[97u" {
        return Err("flag 8 devia mandar 'a' como CSI 97u".into());
    }
    if key(None, Some("A"), ModifiersState::SHIFT, all) != "\x1b[65;2u" {
        return Err("flag 8 + shift devia mandar CSI 65;2u".into());
    }
    // flag 16 embute o texto da tecla
    if key(None, Some("a"), NONE, all | keys::F_TEXT) != "\x1b[97;97u" {
        return Err("flag 16 devia embutir o texto".into());
    }
    Ok(())
}

fn mouse(button: u8, col: usize, row: usize) -> mouse::Ev {
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
    let legacy = mouse::Proto::default();
    let cases: [(&str, Vec<u8>, &[u8]); 3] = [
        (
            "sgr press",
            mouse::encode(mouse(mouse::LEFT, 9, 4), sgr),
            b"\x1b[<0;10;5M",
        ),
        (
            "sgr right release",
            mouse::encode(
                mouse::Ev {
                    pressed: false,
                    ..mouse(mouse::RIGHT, 0, 0)
                },
                sgr,
            ),
            b"\x1b[<2;1;1m",
        ),
        (
            "antigo press",
            mouse::encode(mouse(mouse::LEFT, 0, 0), legacy),
            b"\x1b[M\x20\x21\x21",
        ),
    ];
    for (name, obtained, esperado) in cases {
        if obtained != esperado {
            return Err(format!("{name}: {obtained:?}, esperado {esperado:?}"));
        }
    }
    // release de roda não existe no protocolo antigo (só no SGR)
    let run = mouse::Ev {
        button: 64,
        col: 0,
        row: 0,
        pressed: false,
        motion: false,
        mods: 0,
    };
    if !mouse::encode(run, legacy).is_empty() {
        return Err("release de roda devia sumir no protocolo antigo".into());
    }
    if mouse::encode(run, sgr).is_empty() {
        return Err("release de roda devia sair no SGR".into());
    }
    // ?1005: a coordenada vai somada de 33 e codificada em UTF-8 — 300 vira
    // U+014D (`c5 8d`), enquanto o protocolo antigo satura em 222 (`ff`)
    let utf8 = mouse::Proto {
        sgr: false,
        utf8: true,
    };
    let far = mouse(mouse::LEFT, 300, 300);
    let bytes = mouse::encode(far, utf8);
    if bytes != b"\x1b[M\x20\xc5\x8d\xc5\x8d" {
        return Err(format!("?1005 com coluna 300: {bytes:?}"));
    }
    let saturado = mouse::encode(far, legacy);
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
        other => return Err(format!("ida e volta perdeu a geometria: {other:?}")),
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
        other => Err(format!("sem posição devia manter o tamanho: {other:?}")),
    }
}

/// Sink de mentira que responde como o app: JSON com `cmd` vira `ok`, o resto
/// vira `err`. O socket em si não sabe que o winit existe — é essa a costura
/// que permite exercitá-lo aqui.
// a assinatura é a do trait `Sink` (que recebe a linha e o canal por valor),
// então o `allow` fica aqui e não no topo do crate
#[allow(clippy::needless_pass_by_value)]
fn sink_de_test(line: String, response: mpsc::Sender<String>) -> bool {
    let reply = match serde_json::from_str::<serde_json::Value>(&line) {
        Ok(v) if v.get("cmd").is_some() => debugsock::ok(serde_json::json!({ "cmd": v["cmd"] })),
        Ok(_) => debugsock::err("falta o campo cmd"),
        Err(e) => debugsock::err(format!("json: {e}")),
    };
    response.send(reply).is_ok()
}

fn debug_socket() -> Result<(), String> {
    let path = std::env::temp_dir()
        .join(format!("bzmux-check-sock-{}.sock", std::process::id()))
        .to_string_lossy()
        .to_string();
    let _ = std::fs::remove_file(&path);
    let srv = debugsock::DebugServer::start(
        &path,
        sink_de_test as fn(String, mpsc::Sender<String>) -> bool,
    )
    .map_err(|e| format!("start: {e}"))?;
    // modo 0600: o terminal de outro usuário não pode ler a tela
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o777;
        if mode != 0o600 {
            return Err(format!("socket com modo {mode:o}, esperado 600"));
        }
    }
    let mut s = UnixStream::connect(&path).map_err(|e| format!("connect: {e}"))?;
    let mut f = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
    let mut read = |esperado: &str| -> Result<String, String> {
        let mut l = String::new();
        f.read_line(&mut l).map_err(|e| e.to_string())?;
        if !l.contains(esperado) {
            return Err(format!("resposta {l:?} não tem {esperado:?}"));
        }
        Ok(l)
    };
    s.write_all(b"{\"cmd\":\"state\"}\n")
        .map_err(|e| e.to_string())?;
    read("state")?;
    // pedido torto responde erro **e a conexão continua**: quem chama o socket
    // precisa do motivo, não de um EOF
    s.write_all(b"nao sou json\n").map_err(|e| e.to_string())?;
    read("\"ok\":false")?;
    s.write_all(b"{\"cmd\":\"text\"}\n")
        .map_err(|e| e.to_string())?;
    read("text")?;
    drop(srv);
    if std::path::Path::new(&path).exists() {
        return Err("socket devia sair do disco ao parar".into());
    }
    Ok(())
}

fn desktop_shortcut() -> Result<(), String> {
    // o `install.sh` é o dono do atalho (K18): o check cobra o contrato, não a
    // implementação — nome, Exec com o binário, ícone e categoria de terminal
    for field in [
        "Exec=$BINDIR/$APP",
        "TryExec=$BINDIR/$APP",
        "Icon=$APP",
        "Categories=System;",
        "TerminalEmulator;",
    ] {
        if !INSTALL_SH.contains(field) {
            return Err(format!("install.sh não escreve {field:?} no .desktop"));
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
    for target in ["$DESKTOP", "$ICON"] {
        if !desinstala.contains(target) || !desinstala.contains("rm -f") {
            return Err(format!("desinstalar não remove {target}"));
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
