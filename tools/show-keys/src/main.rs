//! show-keys — the facts of one key press, straight from the window system.
//!
//! A shortcut that does not fire is a question about what the window system
//! handed over, and the answer is not the same on every layout: ABNT2 types `[`
//! where the US layout has `]`, and Shift turns `[` into `{`. Guessing from the
//! label on the keycap is how a fix becomes fragile, so this prints the facts
//! and nothing else: physical key, logical key, text, modifiers, location.
//!
//! It depends on `winit` **only** — no `tortty-core`, no config, no chord
//! table. It is a witness, not a judge: what it prints is what the terminal
//! receives, so a disagreement between this and the terminal is a fact about
//! the terminal, not about this tool.
//!
//! ```text
//! cargo run -p show-keys
//! cargo run -p show-keys -- --quit-after 5   # for automation, no window close
//! ```
//!
//! Each key goes to stdout as one line, so `show-keys > keys.txt` (or a pipe
//! into `grep`) is the comparison table.

use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyLocation, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

// `key_without_modifiers` comes from a platform extension trait: it exists on
// X11, Wayland, macOS and Orbital, and nowhere on Windows (winit's
// `platform::modifier_supplement` is gated). The column says so instead of the
// tool refusing to build there.
#[cfg(not(target_os = "windows"))]
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quit_after = match parse(&args) {
        Ok(v) => v,
        Err(msg) => {
            println!("{msg}");
            std::process::exit(if msg.starts_with("show-keys ") { 0 } else { 2 });
        },
    };
    let event_loop = match EventLoop::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("show-keys: event loop: {e}");
            std::process::exit(1);
        },
    };
    let mut app = ShowKeys {
        window: None,
        mods: ModifiersState::empty(),
        quit_after: quit_after.map(|s| Instant::now() + Duration::from_secs(s)),
        mods_printed: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("show-keys: {e}");
        std::process::exit(1);
    }
}

const HELP: &str = "\
show-keys — print what the window system hands to each key press

USAGE:
  show-keys [--quit-after SECONDS]

A window opens and takes the focus; every key pressed in it is printed to
stdout, one line per event. Close the window to stop (or let --quit-after end
it, which is what automation uses).

COLUMNS:
  state    down, rep (held down) or up; `*` = synthetic
  phys     winit's physical key: a position index whose names are US labels,
           so on ABNT2 the key printed `[` arrives as `BracketRight`
  logical  what the layout produced, after Shift/AltGr and dead keys
  unshift  the same key with every modifier ignored: the layout's own keycap
           (`[` on the ABNT2 key that types `[`), never a dead key
  text     the text the key produced, when it produced any
  mods     c/a/s/w = ctrl/alt/shift/super held at that moment, `-` = none
  loc      Std, Num (keypad), L, R (the two Shift/Ctrl/Alt)
";

fn parse(args: &[String]) -> Result<Option<u64>, String> {
    let mut quit_after = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Err(HELP.to_string()),
            "--quit-after" => {
                let v = it
                    .next()
                    .ok_or_else(|| "--quit-after needs the seconds".to_string())?;
                quit_after = Some(
                    v.parse()
                        .map_err(|_| format!("--quit-after expects an integer: {v}"))?,
                );
            },
            other => return Err(format!("unknown option: {other}\n\n{HELP}")),
        }
    }
    Ok(quit_after)
}

struct ShowKeys {
    window: Option<Window>,
    /// The modifiers in force: `winit` sends them on their own event, not inside
    /// the key event, so the key has to be read against this.
    mods: ModifiersState,
    /// `--quit-after`: the instant the loop stops by itself.
    quit_after: Option<Instant>,
    /// Last modifiers printed, so a change is one line and not one per key.
    mods_printed: Option<ModifiersState>,
}

impl ApplicationHandler for ShowKeys {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("show-keys — press keys, they land in the terminal")
            .with_inner_size(LogicalSize::new(520.0, 160.0));
        match event_loop.create_window(attrs) {
            Ok(w) => {
                w.focus_window();
                self.window = Some(w);
                println!("show-keys — close the window to stop");
                println!(
                    "{:<5} {:<16} {:<16} {:<16} {:<6} {:<7} loc",
                    "state", "phys", "logical", "unshift", "text", "mods"
                );
                println!("      c/a/s/w = ctrl/alt/shift/super   loc: Std Num L R   * = synthetic");
            },
            Err(e) => {
                eprintln!("show-keys: window: {e}");
                event_loop.exit();
            },
        }
        if let Some(deadline) = self.quit_after {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
                if self.mods_printed != Some(self.mods) {
                    self.mods_printed = Some(self.mods);
                    // the columns before `mods` are empty on this line, so it is
                    // one pad and the value: the reader sees the change in place
                    let pad = " ".repeat(MODS_COL);
                    println!("{pad}{}", mods_text(self.mods));
                }
            },
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => print_key(&event, self.mods, is_synthetic),
            _ => {},
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quit_after.is_some_and(|d| Instant::now() >= d) {
            event_loop.exit();
        }
    }
}

fn print_key(ev: &KeyEvent, mods: ModifiersState, synthetic: bool) {
    let state = match (ev.state, ev.repeat) {
        (ElementState::Pressed, false) => "down",
        (ElementState::Pressed, true) => "rep",
        (ElementState::Released, _) => "up",
    };
    let state = if synthetic {
        format!("{state}*")
    } else {
        state.to_string()
    };
    println!(
        "{:<5} {:<16} {:<16} {:<16} {:<6} {:<7} {loc}",
        state,
        phys_text(ev.physical_key),
        logical_text(&ev.logical_key),
        unshift_text(ev),
        ev.text
            .as_deref()
            .map_or("-".to_string(), |t| format!("{t:?}")),
        mods_text(mods),
        loc = loc_text(ev.location),
    );
}

/// Column where `mods` starts: the widths of the columns before it, plus the
/// single space between them.
const MODS_COL: usize = 5 + 1 + 16 + 1 + 16 + 1 + 16 + 1 + 6 + 1;

/// The keycap of the layout for this physical key, with Shift/AltGr ignored:
/// what a shortcut *should* be named by. On a dead key this still names the
/// character the key carries, which is the whole point of the column.
#[cfg(not(target_os = "windows"))]
fn unshift_text(ev: &KeyEvent) -> String {
    logical_text(&ev.key_without_modifiers())
}

#[cfg(target_os = "windows")]
fn unshift_text(_ev: &KeyEvent) -> String {
    // no such extension on Windows: the column says nothing instead of guessing
    "-".to_string()
}

/// The physical key is the **position**: on ABNT2 the key that types `[` is the
/// one the US layout uses for `]`, and that is exactly the name a shortcut
/// written for the US layout would mean.
fn phys_text(p: PhysicalKey) -> String {
    match p {
        PhysicalKey::Code(code) => format!("{code:?}"),
        PhysicalKey::Unidentified(native) => format!("{native:?}"),
    }
}

fn logical_text(k: &Key) -> String {
    match k {
        Key::Named(n) => format!("{n:?}"),
        Key::Character(s) => format!("Char({s:?})"),
        Key::Dead(c) => format!("Dead({})", c.map_or("-".to_string(), |c| format!("{c:?}"))),
        Key::Unidentified(native) => format!("Unid({native:?})"),
    }
}

/// `KeyLocation`, shortened to fit the table: the two Shift/Ctrl/Alt pairs and
/// the keypad are the cases that matter (I16 lives in the keypad digits).
fn loc_text(l: KeyLocation) -> &'static str {
    match l {
        KeyLocation::Standard => "Std",
        KeyLocation::Left => "L",
        KeyLocation::Right => "R",
        KeyLocation::Numpad => "Num",
    }
}

fn mods_text(m: ModifiersState) -> String {
    let mut out = String::new();
    for (on, name) in [
        (m.control_key(), "c"),
        (m.alt_key(), "a"),
        (m.shift_key(), "s"),
        (m.super_key(), "w"),
    ] {
        if on {
            if !out.is_empty() {
                out.push('+');
            }
            out.push_str(name);
        }
    }
    if out.is_empty() {
        "-".to_string()
    } else {
        out
    }
}
