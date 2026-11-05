# Install

bzmux is **one binary**. There is no system dependency step: `cargo build
--release` links only against `libc`/`libm`/`libgcc_s`, and what is left at run
time is the GPU driver (Vulkan/GL) and a system mono font. The Python bzmux
needed `python3-gi`, `gir1.2-gtk-3.0`, cairo, Pango and pipx; here none of that.

## Command

```sh
./install.sh                 # builds (release) and installs to ~/.local/bin/bzmux
./install.sh --prefix DIR    # another prefix: the command lands in DIR/bin/bzmux
./install.sh --binary FILE   # installs an already built binary (does not build)
./install.sh --from-release  # downloads the binary from GitHub Releases (no cargo)
./install.sh --local         # command becomes a link to target/release/bzmux (dev)
./install.sh --no-shortcut   # without an entry in the system menu
./install.sh --check         # checks the existing installation
./install.sh --uninstall     # removes command, entry and icon
./install.sh --dry-run       # shows every command, runs nothing
```

What it does, in order:

1. obtains the binary (builds, copies `--binary` or downloads `--from-release`);
2. warns if `bzmux` already exists in `PATH` and is not the one from this
   installation;
3. installs to `$PREFIX/bin/bzmux` (`install`, copying — swapping the binary in
   `target/` does not leave the installation pointing at nothing);
4. without `--no-shortcut`, writes `$PREFIX/share/applications/bzmux.desktop`
   and `$PREFIX/share/icons/hicolor/scalable/apps/bzmux.svg`;
5. verifies: runs `bzmux --version` and fails if `ldd` shows a missing library;
6. warns if `$PREFIX/bin` is outside `PATH`.

`sudo` only comes in when the prefix is not writable (e.g. `--prefix /usr/local`).

The menu entry carries `StartupWMClass=bzmux` — winit sets `WM_CLASS` to the
binary name (`"bzmux", "bzmux"`), so the environment's window grouper associates
the shortcut with the right window.

And it carries `StartupNotify=false`, because the app does not answer the
startup notification contract (`DESKTOP_STARTUP_ID` → `_NET_STARTUP_INFO`): with
`true`, the panel opens a phantom "Starting bzmux" entry, leaves the pointer in
busy mode and only removes it on its timeout → [`BACKLOG.md: B7`](BACKLOG.md).
The "opening" notice lost with that depended on that answer, which never
existed; when (and if) the app completes the notification, the key goes back to
`true` together with the code that sustains it.

## Coexisting with the Python bzmux

The Python bzmux installs through pipx under the name `bzmux`. With Python
installing as `tortty-python` instead, the short name is free for the Rust one:

```sh
pipx uninstall bzmux          # removes the old shim (the venv goes with it)
./install.sh                  # now `bzmux` is the Rust one
tortty-python                       # the Python one, under the new name
```

While Python still installs as `bzmux`, the installer warns when it finds the
pipx shim in the path (the shim in `~/.local/bin` does not have "pipx" in the
name; the target does: `~/.local/share/pipx/venvs/bzmux/bin/bzmux`).

Who owns which name: `tortty-python` installs and removes **only**
`tortty-python.desktop`/`tortty-python.svg`; `bzmux.desktop`/`bzmux.svg` belong to this
installation. Verified with `tortty-python --install-shortcut` and
`--remove-shortcut` over the real files (same md5 before and after).

`tortty-python` versions before 2026-11-02 treated `bzmux` as an "old name" and
deleted the entry from here in `install()`; if the menu entry disappears after
running that installer, update `tortty-python` (the case has a regression in
`bzmux/checks.py`).

## Verify

```sh
./install.sh --check
# binary:   /home/USER/.local/bin/bzmux
# version:  bzmux 0.1.0
# link:     /home/USER/.local/bin/bzmux
# libs:     5 entries, missing: 0
# menu:     .../applications/bzmux.desktop

bzmux --version
bzmux --cmd 'stty size' --cols 100 --rows 30   # must say 30 100
```

`--cols/--rows` are in cells and the final grid is what the window managed: if
the WM gives fewer pixels, bzmux adjusts the window until it matches
(`docs/DEBUG.md`). The child is born after that adjustment, so `stty size` shows
the real size.

## Publish

`--from-release` expects, in the GitHub Releases of tag `v<version>`:

```
bzmux-<version>-x86_64-unknown-linux-gnu.tar.gz    (binary `bzmux` + LICENSE + README)
bzmux-<version>-aarch64-unknown-linux-gnu.tar.gz
SHA256SUMS
```

What generates that is `.github/workflows/release.yml`, which only calls
`tools/release.sh` — the **same** command that is run on the machine:

```sh
./tools/release.sh                      # packages the host into dist/
./tools/release.sh --target aarch64-unknown-linux-gnu
./tools/release.sh --check-version v0.1.0   # CI uses it: the tag has to match
```

The script checks the `Cargo.toml` version against the tag, runs
`bzmux --version` on the packaged binary (under `qemu-user-static` when it is a
cross build) and leaves the `.tar.gz` + `.sha256` in `dist/`.

Publishing is creating the tag:
`git tag -a v0.1.0 -m ... && git push origin v0.1.0`. The download repository
comes from `BZMUX_REPO` (default `bazoocaze/rust_term`). macOS and Windows are
not in the matrix yet: they are new targets (`M-X`/`M-Y` of
`docs/BACKLOG.md`, with signing in `M-Z`), not parity — Python never ran there.
