//! Erro X de extensão não pode derrubar o app.
//!
//! O `winit` guarda o último erro X da conexão e o `expect()`a na chamada
//! seguinte que faz flush (`flush_requests`). Um erro que **não é nosso** — o
//! WSI do Mesa deixa `BadDrawable` pendente ao criar e destruir pixmaps para
//! apresentar — mata o processo na próxima propriedade de janela (foi assim que
//! `sandbox bzmux` saía com 134 em `set_window_title`; ver `docs/BACKLOG.md`, B5).
//! A GLFW (kitty) registra o erro e segue; aqui o hook público do `winit` faz o
//! mesmo, sem tocar no tratador do X por fora.
//!
//! O que ele engole é **estreito de propósito**: só código de erro de desenho
//! (`BadDrawable`/`BadWindow`/`BadMatch`) e só em requisição de **extensão**
//! (`request_code >= 128`), que é de onde vem o WSI (Present/DRI3). Requisição
//! do núcleo do X continua sendo erro de verdade e o `winit` a reporta.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

/// `XErrorEvent` da Xlib, campo a campo (X11/Xlib.h). O `winit` entrega o
/// ponteiro cru no hook; ler a struct é a única forma de saber o que falhou.
#[repr(C)]
struct XErrorEvent {
    kind: i32,
    display: *mut c_void,
    resource_id: u64,
    serial: u64,
    error_code: u8,
    request_code: u8,
    minor_code: u8,
}

const BAD_WINDOW: u8 = 3;
const BAD_MATCH: u8 = 8;
const BAD_DRAWABLE: u8 = 9;
const BAD_ACCESS: u8 = 10;

/// Primeira requisição de extensão (as do núcleo vão até 127).
const EXTENSAO_MIN: u8 = 128;

/// Um erro de desenho vindo do WSI não é motivo para encerrar o terminal.
///
/// Medido em `sandbox` sem GPU (`docs/DEBUG.md`): o WSI deixa `BadDrawable` na
/// `Present` **e** `BadAccess` na `DRI3Open` — o segundo é o que matava, porque
/// a política anterior só olhava requisição de extensão para os códigos de
/// desenho. Código de desenho vale em qualquer requisição (o WSI também libera
/// pixmap por requisição do núcleo); `BadAccess` só em extensão, que é onde
/// permissão de GPU se manifesta.
fn swallow(error_code: u8, request_code: u8) -> bool {
    match error_code {
        BAD_WINDOW | BAD_MATCH | BAD_DRAWABLE => true,
        BAD_ACCESS => request_code >= EXTENSAO_MIN,
        _ => false,
    }
}

static CONTADOS: AtomicU32 = AtomicU32::new(0);

#[allow(unsafe_code)] // ler o `XErrorEvent` cru é o contrato do hook do winit
fn decide(_display: *mut c_void, event: *mut c_void) -> bool {
    if event.is_null() {
        return false;
    }
    // SAFETY: o winit passa um `XErrorEvent` válido da Xlib, que tem exatamente
    // este layout (X11/Xlib.h); só lemos, sem guardar o ponteiro.
    let e = unsafe { &*(event.cast::<XErrorEvent>()) };
    if std::env::var_os("BZMUX_XERR").is_some() {
        eprintln!(
            "XERR codigo={} requisicao={} menor={} recurso=0x{:x} engolir={}",
            e.error_code,
            e.request_code,
            e.minor_code,
            e.resource_id,
            swallow(e.error_code, e.request_code)
        );
    }
    if !swallow(e.error_code, e.request_code) {
        return false;
    }
    let n = CONTADOS.fetch_add(1, Ordering::Relaxed);
    // uma linha nas três primeiras e depois esparso: erro do WSI costuma vir
    // repetido, e o log não pode virar o problema
    if n < 3 || n.is_multiple_of(1000) {
        eprintln!(
            "bzmux: erro X de extensão ignorado (código {}, requisição {}), ocorrência {} — WSI da GPU; ver docs/DEBUG.md",
            e.error_code,
            e.request_code,
            n + 1
        );
    }
    true
}

/// Instala o hook. Seguro de chamar em qualquer backend (Wayland nunca invoca).
pub fn install() {
    winit::platform::x11::register_xlib_error_hook(Box::new(decide));
}

#[cfg(test)]
mod tests {
    use super::swallow;

    #[test]
    fn error_do_wsi_e_engolido() {
        // Present (149) e DRI3 (130) são as duas portas do WSI; medido no sandbox
        assert!(swallow(9, 149), "BadDrawable na Present");
        assert!(
            swallow(10, 130),
            "BadAccess na DRI3Open: o que matava no sandbox"
        );
        assert!(swallow(3, 149));
        assert!(swallow(8, 130));
        // o WSI também libera pixmap por requisição do núcleo
        assert!(swallow(9, 54));
    }

    #[test]
    fn bad_access_fora_de_extensao_not_e_engolido() {
        // BadAccess é permissão: em requisição do núcleo continua sendo erro
        assert!(!swallow(10, 20));
        assert!(!swallow(10, 33));
    }

    #[test]
    fn other_codigo_de_error_not_e_engolido() {
        // BadAtom, BadValue, BadGC: não é desenho nem permissão de GPU
        assert!(!swallow(5, 149));
        assert!(!swallow(2, 149));
        assert!(!swallow(13, 149));
    }
}
