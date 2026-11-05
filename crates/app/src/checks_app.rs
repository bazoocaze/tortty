//! Smoke checks that use the app side, but **do not** need a window.
//!
//! `tortty-core` does not know `cli.rs`, `keys.rs`, `mouse.rs`, `state.rs` or
//! `debugsock.rs` — so the core `checks::all()` does not reach those five, and
//! Python's `checks.py` covers all five (`argv_parsing`, `kitty_keyboard`,
//! `mouse_modes_and_sgr`, `window_state`, `debug_socket`). This list closes the
//! `N1` gap without a display: same rule as the core — build the scenario, exercise
//! it, compare with the expected result, and fail saying why.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;

use tortty_core::checks::Check;
use winit::keyboard::{ModifiersState, NamedKey};

use crate::cli;
use crate::debugsock;
use crate::keys;
use crate::mouse;
use crate::state;

const NONE: ModifiersState = ModifiersState::empty();

/// The installer and the icon go into the binary: the desktop shortcut check
/// also holds for the already installed binary, far from the repo tree.
const INSTALL_SH: &str = include_str!("../../../install.sh");
const ICON: &str = include_str!("../../../assets/tortty.svg");

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
    // `--help` and `--version` are informative (0); usage error is 2 — including
    // a value that is not a number, which used to exit 0 and let automation
    // scripts go on with an invalid command.
    let cases: [(&[&str], i32); 5] = [
        (&["--help"], 0),
        (&["-V"], 0),
        (&["--colo", "80"], 2),
        (&["--cols", "abc"], 2),
        (&["--cols"], 2),
    ];
    for (args, expected) in cases {
        let msg = argv(args).err().unwrap_or_default();
        let obtained = cli::exit_code(&msg);
        if obtained != expected {
            return Err(format!(
                "{}: code {obtained}, expected {expected}",
                args.join(" ")
            ));
        }
    }
    if argv(&["--help"]).is_ok() {
        return Err("--help should leave through the message, not become options".into());
    }
    let o = argv(&["--cols", "0", "--rows", "7"])?;
    if o.cols != Some(1) || o.rows != Some(7) {
        return Err(format!(
            "grid {:?}x{:?}: expected 1x7 (column zero does not exist)",
            o.cols, o.rows
        ));
    }
    // precedence CLI > file > defaults, without opening a window
    let conf = std::env::temp_dir().join(format!("tortty-check-{}.conf", std::process::id()));
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
            "file ignored: {:?} pt / scrollback {}",
            without_cli.font_size_pt, without_cli.scrollback
        ));
    }
    if (with_cli.font_size_pt - 14.0).abs() > f32::EPSILON || with_cli.scrollback != 99 {
        return Err(format!(
            "CLI did not win over the file: {:?} pt / scrollback {}",
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
    // without the flag the path is the legacy one: raw ESC, ^A, F5 in xterm form
    let legacy: [(&str, String); 3] = [
        ("escape", key(Some(NamedKey::Escape), None, NONE, 0)),
        ("ctrl+a", key(None, Some("a"), ModifiersState::CONTROL, 0)),
        ("f5", key(Some(NamedKey::F5), None, NONE, 0)),
    ];
    for (name, obtained) in legacy {
        let expected = match name {
            "escape" => "\x1b",
            "ctrl+a" => "\x01",
            _ => "\x1b[15~",
        };
        if obtained != expected {
            return Err(format!(
                "legacy {name}: {obtained:?}, expected {expected:?}"
            ));
        }
    }
    let disambiguate = keys::F_DISAMBIGUATE;
    if key(Some(NamedKey::Escape), None, NONE, disambiguate) != "\x1b[27u" {
        return Err("flag 1 should send ESC as CSI 27u".into());
    }
    if key(None, Some("a"), ModifiersState::CONTROL, disambiguate) != "\x1b[97;5u" {
        return Err("flag 1 should send Ctrl+a as CSI 97;5u".into());
    }
    // F3 has its own form in kitty: `CSI 1;2R`, not `CSI 13;2~`
    if key(
        Some(NamedKey::F3),
        None,
        ModifiersState::SHIFT,
        disambiguate,
    ) != "\x1b[1;2R"
    {
        return Err("Shift+F3 should be CSI 1;2R".into());
    }
    // flag 8 sends the letter alone too, and the uppercase comes from winit (65)
    let all = keys::F_ALL_KEYS;
    if key(None, Some("a"), NONE, all) != "\x1b[97u" {
        return Err("flag 8 should send 'a' as CSI 97u".into());
    }
    if key(None, Some("A"), ModifiersState::SHIFT, all) != "\x1b[65;2u" {
        return Err("flag 8 + shift should send CSI 65;2u".into());
    }
    // flag 16 embeds the key text
    if key(None, Some("a"), NONE, all | keys::F_TEXT) != "\x1b[97;97u" {
        return Err("flag 16 should embed the text".into());
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
            "legacy press",
            mouse::encode(mouse(mouse::LEFT, 0, 0), legacy),
            b"\x1b[M\x20\x21\x21",
        ),
    ];
    for (name, obtained, expected) in cases {
        if obtained != expected {
            return Err(format!("{name}: {obtained:?}, expected {expected:?}"));
        }
    }
    // a wheel release does not exist in the old protocol (only in SGR)
    let run = mouse::Ev {
        button: 64,
        col: 0,
        row: 0,
        pressed: false,
        motion: false,
        mods: 0,
    };
    if !mouse::encode(run, legacy).is_empty() {
        return Err("a wheel release should vanish in the old protocol".into());
    }
    if mouse::encode(run, sgr).is_empty() {
        return Err("a wheel release should go out in SGR".into());
    }
    // ?1005: the coordinate goes with 33 added and encoded in UTF-8 — 300 becomes
    // U+014D (`c5 8d`), while the old protocol saturates at 222 (`ff`)
    let utf8 = mouse::Proto {
        sgr: false,
        utf8: true,
    };
    let far = mouse(mouse::LEFT, 300, 300);
    let bytes = mouse::encode(far, utf8);
    if bytes != b"\x1b[M\x20\xc5\x8d\xc5\x8d" {
        return Err(format!("?1005 with column 300: {bytes:?}"));
    }
    let saturated = mouse::encode(far, legacy);
    if saturated != b"\x1b[M\x20\xff\xff" {
        return Err(format!("legacy should saturate at 222: {saturated:?}"));
    }
    Ok(())
}

fn window_state() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("tortty-check-state-{}", std::process::id()));
    let file = dir.join("window.json");
    let _ = std::fs::remove_file(&file);
    let g = state::Geometry {
        x: Some(-120),
        y: Some(40),
        w: 1131,
        h: 656,
        maximized: false,
    };
    state::save_to(&file, &g);
    match state::load_from(&file) {
        Some(lido) if lido == g => {},
        other => return Err(format!("round trip lost the geometry: {other:?}")),
    }
    // a crooked file must not become a panic nor an invented geometry
    std::fs::write(&file, "{ this is not json").map_err(|e| e.to_string())?;
    if state::load_from(&file).is_some() {
        return Err("a crooked file should be ignored".into());
    }
    let _ = std::fs::remove_dir_all(&dir);
    // `fit`: the corner has to be on a monitor and the size has to be healthy
    let monitors = [(0, 0, 1920, 1080)];
    if state::fit(Some(100), Some(100), 980, 640, &monitors).is_none() {
        return Err("a healthy geometry was refused".into());
    }
    if state::fit(Some(100), Some(100), state::MIN_W - 1, 640, &monitors).is_some() {
        return Err("a too narrow window should be refused".into());
    }
    if state::fit(Some(5000), Some(5000), 980, 640, &monitors).is_some() {
        return Err("a corner outside every monitor should be refused".into());
    }
    // Wayland (J6): no position, the size alone still counts
    match state::fit(None, None, 900, 500, &monitors) {
        Some(g) if g.x.is_none() && g.w == 900 => Ok(()),
        other => Err(format!(
            "without position it should keep the size: {other:?}"
        )),
    }
}

/// A fake sink that answers like the app: JSON with `cmd` becomes `ok`, the rest
/// becomes `err`. The socket itself does not know winit exists — that is the seam
/// that allows exercising it here.
// the signature is the `Sink` trait one (it receives the line and the channel by
// value), so the `allow` lives here and not at the top of the crate
#[allow(clippy::needless_pass_by_value)]
fn test_sink(line: String, response: mpsc::Sender<String>) -> bool {
    let reply = match serde_json::from_str::<serde_json::Value>(&line) {
        Ok(v) if v.get("cmd").is_some() => debugsock::ok(serde_json::json!({ "cmd": v["cmd"] })),
        Ok(_) => debugsock::err("missing the cmd field"),
        Err(e) => debugsock::err(format!("json: {e}")),
    };
    response.send(reply).is_ok()
}

fn debug_socket() -> Result<(), String> {
    let path = std::env::temp_dir()
        .join(format!("tortty-check-sock-{}.sock", std::process::id()))
        .to_string_lossy()
        .to_string();
    let _ = std::fs::remove_file(&path);
    let srv =
        debugsock::DebugServer::start(&path, test_sink as fn(String, mpsc::Sender<String>) -> bool)
            .map_err(|e| format!("start: {e}"))?;
    // mode 0600: another user's terminal cannot read the screen
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o777;
        if mode != 0o600 {
            return Err(format!("socket with mode {mode:o}, expected 600"));
        }
    }
    let mut s = UnixStream::connect(&path).map_err(|e| format!("connect: {e}"))?;
    let mut f = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
    let mut read = |expected: &str| -> Result<String, String> {
        let mut l = String::new();
        f.read_line(&mut l).map_err(|e| e.to_string())?;
        if !l.contains(expected) {
            return Err(format!("response {l:?} does not contain {expected:?}"));
        }
        Ok(l)
    };
    s.write_all(b"{\"cmd\":\"state\"}\n")
        .map_err(|e| e.to_string())?;
    read("state")?;
    // a crooked request answers an error **and the connection survives**: whoever
    // calls the socket needs the reason, not an EOF
    s.write_all(b"not json at all\n")
        .map_err(|e| e.to_string())?;
    read("\"ok\":false")?;
    s.write_all(b"{\"cmd\":\"text\"}\n")
        .map_err(|e| e.to_string())?;
    read("text")?;
    drop(srv);
    if std::path::Path::new(&path).exists() {
        return Err("the socket should leave the disk when it stops".into());
    }
    Ok(())
}

fn desktop_shortcut() -> Result<(), String> {
    // `install.sh` owns the shortcut (K18): the check demands the contract, not
    // the implementation — name, Exec with the binary, icon and terminal category
    for field in [
        "Exec=$BINDIR/$APP",
        "TryExec=$BINDIR/$APP",
        "Icon=$APP",
        "Categories=System;",
        "TerminalEmulator;",
    ] {
        if !INSTALL_SH.contains(field) {
            return Err(format!(
                "install.sh does not write {field:?} in the .desktop"
            ));
        }
    }
    if !INSTALL_SH.contains("share/applications/tortty.desktop") {
        return Err("install.sh does not install the .desktop under the prefix".into());
    }
    // uninstalling has to take the installed chrome along
    let uninstall = INSTALL_SH
        .split("do_uninstall()")
        .nth(1)
        .ok_or("install.sh without do_uninstall")?;
    for target in ["$DESKTOP", "$ICON"] {
        if !uninstall.contains(target) || !uninstall.contains("rm -f") {
            return Err(format!("uninstalling does not remove {target}"));
        }
    }
    if !ICON.contains("<svg") || !ICON.contains("viewBox") {
        return Err("assets/tortty.svg is not a valid SVG".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The same contract as the core `selftest_completo`: a green `cargo test` has
    /// to mean green checks, otherwise they only exist in the binary.
    #[test]
    fn app_checks_pass() {
        let failures: Vec<String> = super::all()
            .into_iter()
            .filter_map(|c| (c.run)().err().map(|e| format!("{}: {e}", c.name)))
            .collect();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
