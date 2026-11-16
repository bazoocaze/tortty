# Portability (Linux, Windows, macOS)

The decision is made and what is left is engineering: the route is **`winit` +
`wgpu` + our own core in Rust**, with no system toolkit, no
Pango/cairo/fontconfig and no interpreter. What exists today is
[`FEATURES.md`](FEATURES.md).

This document is the **analysis** of each target; it is not a queue. The queue is
[`BACKLOG.md`](BACKLOG.md): Windows is `M-X`, macOS is `M-Y` and the signing is
`M-Z`.

## Why this route answers the original question

Shipping across win/lin/mac is the hard part of a desktop app, and the blocker
is the **system stack**: a toolkit, a text stack and bindings, each one installed
a different way on each platform. In this route that stack does not exist:

| | previous build | Rust/wgpu |
|---|---|---|
| binary | an interpreter plus libs to install | **10.1 MB, one file** |
| OS deps | GTK3, Pango, cairo, PyGObject, fontconfig | `libc`, `libm`, `libgcc_s` (+ graphics driver at runtime) |
| font | Pango + fontconfig | `fontdb` + `swash` (scans the system, no fontconfig) |
| GPU | cairo (glamor/software) | Vulkan / Metal / D3D12 / GL through `wgpu` |
| window | GTK3 | `winit` (X11, Wayland, Win32, Cocoa) |
| pty | POSIX only | `portable-pty` (POSIX pty **and** ConPTY) |

Practical consequence: **"copy the executable and run"** works within the same
system and architecture. It does not work across systems — a Linux binary does
not run on macOS nor on Windows; you have to **compile** for each target (and
that is what the CI matrix below does for free).

## State per platform

| target | render | pty | tested? |
|---|---|---|---|
| Linux X11 + Vulkan (RADV) | ● | ● | **yes**, it is the development environment |
| Linux Wayland | should work (`winit` native) | ● | **no** — still needs exercising |
| Linux + GL (`WGPU_BACKEND=gl`) | should work | ● | **no** — valve for an old GPU |
| Linux + llvmpipe (software) | should work, slow | ● | **no** — next portability test |
| Windows 10/11 | D3D12 (or Vulkan) | ConPTY | **no** — needs a Windows runner |
| macOS Apple Silicon | Metal | POSIX pty | **no** — needs a macOS runner |
| macOS Intel | Metal | POSIX pty | **no** — `macos-13` runner |

## The seam, and what each target still has to answer

The boundary is **one module**: `crates/app/src/platform/`, whose rule and whose
gate are in `AGENTS.md` (`tools/platform-gate.sh` fails if `std::os::unix`,
`libc::`, `windows_sys::`, `winapi::` or `winit::platform::` appears anywhere else
in `crates/`). The app asks for a capability — `platform::try_lock`,
`platform::hangup`, `platform::shell_quote`, `platform::open_uri` — and the name of
a system exists only there. Only the Linux implementation was written, and it is
the measured one.

Measured on 2026-11-16, with the seam in place: a build for Windows stops with
**one** error instead of 35, and for macOS with **one** instead of 4. That error is
the `compile_error!` naming `M-X`/`M-Y`. Note what the 4 macOS errors were — they
are the reason the implementation is named `linux` and not `unix`: `libc::ucred`
and `SO_PEERCRED` do not exist there, and `winit::platform::x11` is not compiled
either. **macOS is not a free ride for being a unix.**

What follows is the list a second implementation has to answer, ordered by
severity. None of them is architectural.

1. **The debug socket** is `platform::Listener`/`Conn` (Linux: `UnixListener`,
   `0600`, `SO_PEERCRED`, and the same uid only). Windows has no unix socket: it
   needs a named pipe or TCP on `127.0.0.1`, behind the same two types and the same
   one-line JSON framing. `tools/benchrun.py` will pick the transport by the path.
2. **The runtime directory** of `--debug-socket auto` is `platform::runtime_dir()`
   (Linux: `$XDG_RUNTIME_DIR` or `/tmp`; Windows `%LOCALAPPDATA%\Temp`, macOS
   `$TMPDIR`).
3. **The shell** is `platform::default_shell()` (Linux: `/bin/bash` then `/bin/sh`)
   and `platform::shell_quote` quotes what the child receives; on Windows the shell
   is `$COMSPEC` (`cmd.exe`), `--cmd` needs `cmd /c`, and quoting is another
   function. `portable-pty` already abstracts the spawn.
4. **Font scan**: `fontdb` covers each system's font directories, and the
   per-OS template names the family (or the fallback list) for that system:
   `Menlo` on macOS, `Cascadia Mono, Consolas` on Windows. What is still
   Linux-shaped is the **preference chain** behind a generic `font_family`
   (DejaVu, Liberation, Noto): it is the last resort, and on a system without
   them the chain ends on fontdb's own "monospace".
5. **The directories** are the seam's: `config_dir` (Linux `$XDG_CONFIG_HOME` or
   `~/.config`; Windows `%APPDATA%`, macOS `~/Library/Application Support`),
   `state_dir` (`$XDG_STATE_HOME` or `~/.local/state`; Windows `%LOCALAPPDATA%`),
   `dump_path`, `home`.
6. **Clipboard and IME**: when they come in, they are platform-specific
   (`arboard`/`winit::Ime`); `winit` already exposes IME on all targets.
7. **Bracketed paste / terminal modes**: nothing to do, it is the parser's.
8. **Wayland**: the client does not choose the window position (it never will).
   Geometry persistence on Wayland stores size and maximized, not `x`/`y`.
9. **No answer exists for three capabilities**, and each one has to be honest
   instead of emulated: the cwd of the front process (Linux reads
   `/proc/<pid>/cwd`; Windows has no `/proc`), the farewell of `SIGHUP` (`B48` —
   ConPTY kills the tree), and an editor from `$VISUAL`/`$EDITOR`. The seam returns
   `Option`/`Err` with the reason; the feature code says so in `LIMITATIONS.md`.

## Distribution

Recommended path, in this order:

1. **GitHub Actions with a matrix** — **it already exists for Linux**
   (`.github/workflows/release.yml`, tag `v*`): `x86_64` and `aarch64`, using
   `tools/build-release.sh` (the same command as locally) and publishing
   `tortty-<version>-<triple>.tar.gz` + `SHA256SUMS`, which is exactly what
   `./install.sh --from-release` downloads. `aarch64` runs under
   `qemu-user-static` in CI so that `--version` counts as verification. When
   M6/M7 close, `windows-latest` and `macos-14`/`macos-13` enter as two more
   lines of the same matrix — in a public repo it is free; in a private one, a
   macOS minute counts 10x.
2. **`lipo`** for a universal binary on macOS (arm64 + x86_64) — a simple `.app`
   or a loose binary + `.dmg`/`.zip`.
3. **Signing**: on Apple Silicon every executable needs a signature, but the
   linker already applies an *ad-hoc signature* on arm64 — what comes out of
   `cargo build` runs. What does **not** run without intervention is a binary
   downloaded from the network that is not notarized (Gatekeeper): the user needs
   "Open Anyway". To avoid that friction there are two ways out, and **neither is
   a "store"**:
   - **Apple Developer Program** (US$ 99/year) → notarization → double click
     opens with no warning. It is the only "clean" way.
   - **an installation that bypasses the quarantine**: a `curl | sh` script that
     builds from source, or a Homebrew *tap* that builds. Then there is no
     `com.apple.quarantine` on the file and the Gatekeeper does not get involved.
     Less comfortable, zero cost.
4. **Windows**: a `.zip` with the `.exe` works directly (SmartScreen complains
   about the missing signature — `signtool` with an OV/EV certificate solves it,
   if it is worth it). ConPTY is native since Windows 10 1809.
5. **Linux**: static binary (`musl`) per architecture, `.deb`/`.rpm`/AUR later.
   It is not a priority: on Linux the user compiles.

Cost of Windows/macOS without a store: **one `.zip` on GitHub Releases**. What
hurts is notarization (macOS) and signing (Windows), both optional for own use
and for whoever uses `curl | sh`.

## Decision

Single route, no bifurcation: `winit` + `wgpu` + our own core. Linux X11 is the
development target and the only tested one; the other two enter through CI when
the feature set is complete enough (not before: a port without features is
rework). What is **not** done: cross-compiling macOS from Linux (`osxcross`
requires the Apple SDK, it is fragile and does not pass review), nor keeping the
GTK stack as an alternative.
