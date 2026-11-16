//! The platform seam: the **only** module that names an operating system.
//!
//! Window, GPU, pty, fonts and clipboard are already portable behind their
//! crates (`winit`, `wgpu`, `portable-pty`, `fontdb`, `arboard`). What is not
//! portable is a handful of syscalls, paths, signals and shell details, and those
//! used to be spread over `debugsock`, `state`, `pty`, `main`, `app` and
//! `xguard` — 35 compile errors on a Windows target, 4 on macOS, each one
//! discovered by a build.
//!
//! The criterion, and it is the whole rule:
//!
//! > `platform` names the operating system; the rest of the code names the
//! > **capability**.
//!
//! So the app asks `platform::try_lock`, `platform::hangup`,
//! `platform::shell_quote` — never `flock`, `SIGHUP` or POSIX quoting. What the
//! implementation does not have (a Windows lock is not `flock`, macOS keeps no
//! `/proc`) comes back as `Option`/`Err` **with the reason**, instead of a silent
//! `#[cfg]` in the feature code.
//!
//! Policy per system is *not* here: the keyboard chords, the default font and the
//! shell name are data, and they live in `crates/core/defaults/` (the four
//! template files, chosen by `config::Os`).
//!
//! `tools/platform-gate.sh` (a step of `tools/quality-check.sh`, so hook and CI
//! agree) fails as soon as `std::os::unix`, `std::os::linux`, `libc::`,
//! `windows_sys::`, `winapi::` or `winit::platform::` appears outside this
//! directory — which is what keeps the seam from rotting.
//!
//! Linux is the measured target: it is where the app is developed and where every
//! number in `docs/BENCHMARKS.md` was taken. Windows is `M-X` and macOS is `M-Y`
//! in `docs/BACKLOG.md`; the analysis of each is in `docs/PORTABILITY.md`.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod x11;

#[cfg(target_os = "linux")]
pub use linux::*;
#[cfg(target_os = "linux")]
pub use x11::install as install_error_guard;

#[cfg(not(target_os = "linux"))]
compile_error!(
    "tortty has no `platform` implementation for this target: only Linux is measured (M-X Windows, M-Y macOS in docs/BACKLOG.md, analysis in docs/PORTABILITY.md)"
);
