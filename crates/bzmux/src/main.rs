//! bzmux em Rust — experimento de viabilidade (ver TASK-bzmux-in-rust.md).

mod app;
mod atlas;
mod cli;
mod debugsock;
mod glyph;
mod keys;
mod prof;
mod pty;
mod render;

use winit::event_loop::{ControlFlow, EventLoop};

use app::{App, UserEvent};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match cli::parse(args) {
        Ok(o) => o,
        Err(msg) => {
            println!("{}", msg);
            // ajuda/versão saem em código 0; erro de uso em 2
            let code = if msg.contains("opção desconhecida") || msg.starts_with("falta") {
                2
            } else {
                0
            };
            std::process::exit(code);
        }
    };
    if opts.prof {
        prof::set_on(true);
    }

    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("bzmux: event loop: {}", e);
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = App::new(opts, proxy);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("bzmux: {}", e);
        std::process::exit(1);
    }
}
