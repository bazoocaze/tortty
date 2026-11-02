//! bzmux em Rust — experimento de viabilidade (ver TASK-bzmux-in-rust.md).

mod app;
mod atlas;
mod cli;
mod clip;
mod debugsock;
mod frame;
mod glyph;
mod keys;
mod mouse;
mod prof;
mod pty;
mod render;

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
    // CLI vence o arquivo; o arquivo vence os padrões embutidos
    let (opts, cfg_errors) = cli::Options::build(over.clone());
    for e in &cfg_errors {
        eprintln!("bzmux: config: {e}");
    }
    if opts.prof {
        prof::set_on(true);
    }

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("bzmux: event loop: {e}");
            std::process::exit(1);
        },
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = App::new(opts, over, proxy);
    app.startup_errors(cfg_errors);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("bzmux: {e}");
        std::process::exit(1);
    }
}

/// `--selftest`: um comando que responde "o emulador ainda está inteiro?" sem
/// display, sem GPU e sem pty. É o que o `checks.py` do Python fazia à parte.
fn selftest() -> i32 {
    let mut falhas = 0;
    for c in bzmux_core::checks::all() {
        match (c.run)() {
            Ok(()) => println!("ok    {}", c.name),
            Err(e) => {
                falhas += 1;
                println!("FALHA {}: {e}", c.name);
            },
        }
    }
    let total = bzmux_core::checks::all().len();
    println!();
    if falhas == 0 {
        println!("selftest: {total}/{total} ok");
        0
    } else {
        println!("selftest: {falhas} falha(s) de {total}");
        1
    }
}
