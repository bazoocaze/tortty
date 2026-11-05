//! TorTTY — fast, GPU based terminal. The feasibility record is in
//! TASK-tortty-in-rust.md.
// the `state` `json!` goes past the macro's default expansion limit
#![recursion_limit = "256"]

mod app;
mod atlas;
mod checks_app;
mod cli;
mod clip;
mod damage;
mod debugsock;
mod frame;
mod glyph;
mod keys;
mod mouse;
mod prof;
mod pty;
mod render;
mod shape;
mod state;
mod xguard;

use winit::event_loop::{ControlFlow, EventLoop};

use app::{App, UserEvent};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let over = match cli::parse(args) {
        Ok(o) => o,
        Err(msg) => {
            println!("{msg}");
            std::process::exit(cli::exit_code(&msg));
        },
    };
    if over.selftest {
        std::process::exit(selftest());
    }
    // CLI wins over the file; the file wins over the built-in defaults
    let (opts, cfg_errors) = cli::Options::build(over.clone());
    for e in &cfg_errors {
        eprintln!("tortty: config: {e}");
    }
    if opts.prof {
        prof::set_on(true);
    }

    // before any window: a GPU WSI X error cannot bring the app down
    xguard::install();
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("tortty: event loop: {e}");
            std::process::exit(1);
        },
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = App::new(opts, over, proxy);
    app.startup_errors(cfg_errors);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("tortty: {e}");
        std::process::exit(1);
    }
}

/// `--selftest`: one command that answers "is the emulator still whole?" without
/// a display, without a GPU and without a pty. It is what Python's `checks.py`
/// did separately.
fn selftest() -> i32 {
    // `tortty --selftest | head` closes the pipe in the middle of the list: with
    // SIGPIPE ignored (the Rust default) the next `println!` becomes a panic. Here
    // it goes back to the Unix default only on this path — the app itself writes
    // BEL to stdout and must not die because of a closed pipe.
    #[allow(unsafe_code)]
    // SAFETY: `signal` with `SIG_DFL` installs no handler and reads no memory; it
    // is the only way to restore the default SIGPIPE disposition.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let all: Vec<_> = tortty_core::checks::all()
        .into_iter()
        .chain(checks_app::all())
        .collect();
    let mut failures = 0;
    for c in &all {
        match (c.run)() {
            Ok(()) => println!("ok    {}", c.name),
            Err(e) => {
                failures += 1;
                println!("FAIL {}: {e}", c.name);
            },
        }
    }
    let total = all.len();
    println!();
    if failures == 0 {
        println!("selftest: {total}/{total} ok");
        0
    } else {
        println!("selftest: {failures} failure(s) out of {total}");
        1
    }
}
