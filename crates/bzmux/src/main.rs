//! bzmux em Rust — experimento de viabilidade (ver TASK-bzmux-in-rust.md).
// o `json!` do `state` passa do limite padrão de expansão do macro
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
    // `bzmux --selftest | head` fecha o pipe no meio da lista: com o SIGPIPE
    // ignorado (o padrão do Rust) o `println!` seguinte vira panic. Aqui volta
    // ao padrão do Unix só neste caminho — o app em si escreve BEL no stdout e
    // não deve morrer por causa de pipe fechado.
    #[allow(unsafe_code)]
    // SAFETY: `signal` com `SIG_DFL` não instala handler nem lê memória; é o
    // único jeito de restaurar a disposição padrão do SIGPIPE.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let todos: Vec<_> = bzmux_core::checks::all()
        .into_iter()
        .chain(checks_app::all())
        .collect();
    let mut falhas = 0;
    for c in &todos {
        match (c.run)() {
            Ok(()) => println!("ok    {}", c.name),
            Err(e) => {
                falhas += 1;
                println!("FALHA {}: {e}", c.name);
            },
        }
    }
    let total = todos.len();
    println!();
    if falhas == 0 {
        println!("selftest: {total}/{total} ok");
        0
    } else {
        println!("selftest: {falhas} falha(s) de {total}");
        1
    }
}
