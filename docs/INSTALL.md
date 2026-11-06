# Install

tortty is **one binary**. There is no system dependency step: `cargo build
--release` links only against `libc`/`libm`/`libgcc_s`, and what is left at run
time is the GPU driver (Vulkan/GL) and a system mono font: no toolkit, no
interpreter and no bindings to install.

## Command

```sh
./install.sh                 # builds (release) and installs to ~/.local/bin/tortty
./install.sh --prefix DIR    # another prefix: the command lands in DIR/bin/tortty
./install.sh --binary FILE   # installs an already built binary (does not build)
./install.sh --from-release  # downloads the binary from GitHub Releases (no cargo)
./install.sh --local         # command becomes a link to target/release/tortty (dev)
./install.sh --no-shortcut   # without an entry in the system menu
./install.sh --check         # checks the existing installation
./install.sh --uninstall     # removes command, entry and icon
./install.sh --dry-run       # shows every command, runs nothing
```

What it does, in order:

1. obtains the binary (builds, copies `--binary` or downloads `--from-release`);
2. warns if `tortty` already exists in `PATH` and is not the one from this
   installation;
3. installs to `$PREFIX/bin/tortty` (`install`, copying — swapping the binary in
   `target/` does not leave the installation pointing at nothing);
4. without `--no-shortcut`, writes `$PREFIX/share/applications/tortty.desktop`
   and `$PREFIX/share/icons/hicolor/scalable/apps/tortty.svg`;
5. verifies: runs `tortty --version` and fails if `ldd` shows a missing library;
6. warns if `$PREFIX/bin` is outside `PATH`.

`sudo` only comes in when the prefix is not writable (e.g. `--prefix /usr/local`).

The menu entry carries `StartupWMClass=tortty` — winit sets `WM_CLASS` to the
binary name (`"tortty", "tortty"`), so the environment's window grouper associates
the shortcut with the right window.

And it carries `StartupNotify=false`, because the app does not answer the
startup notification contract (`DESKTOP_STARTUP_ID` → `_NET_STARTUP_INFO`): with
`true`, the panel opens a phantom "Starting tortty" entry, leaves the pointer in
busy mode and only removes it on its timeout (the case is `B7` in
[`HISTORY.md`](HISTORY.md), 2026-11-03). The "opening" notice lost with that
depended on that answer, which never existed; when (and if) the app completes the
notification, the key goes back to `true` together with the code that sustains
it.

## Coexistence

No step of this installer looks at, warns about or removes another installation:
the names do not collide, so there is nothing to coordinate. An older installer
of a terminal with a similar name did delete `tortty.desktop`/`tortty.svg` when it
mistook the name for an old one of its own — if the menu entry disappears, update
that installer.

## Verify

```sh
./install.sh --check
# binary:   /home/USER/.local/bin/tortty
# version:  tortty 0.1.0
# link:     /home/USER/.local/bin/tortty
# libs:     5 entries, missing: 0
# menu:     .../applications/tortty.desktop

tortty --version
tortty --cmd 'stty size' --cols 100 --rows 30   # must say 30 100
```

`--cols/--rows` are in cells and the final grid is what the window managed: if
the WM gives fewer pixels, tortty adjusts the window until it matches
(`docs/DEBUG.md`). The child is born after that adjustment, so `stty size` shows
the real size.

## Publish

`--from-release` expects, in the GitHub Releases of tag `v<version>`:

```
tortty-<version>-x86_64-unknown-linux-gnu.tar.gz    (binary `tortty` + LICENSE + README)
tortty-<version>-aarch64-unknown-linux-gnu.tar.gz
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
`tortty --version` on the packaged binary (under `qemu-user-static` when it is a
cross build) and leaves the `.tar.gz` + `.sha256` in `dist/`.

Publishing is creating the tag:
`git tag -a v0.1.0 -m ... && git push origin v0.1.0`. The download repository
comes from `TORTTY_REPO` (default `bazoocaze/tortty`). macOS and Windows are
not in the matrix yet: they are new targets (`M-X`/`M-Y` of
`docs/BACKLOG.md`, with signing in `M-Z`): a target nobody has exercised yet.
