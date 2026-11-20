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
./install.sh --no-verify     # does not check the download against SHA256SUMS
./install.sh --local         # command becomes a link to target/release/tortty (dev)
./install.sh --no-shortcut   # without an entry in the system menu
./install.sh --check         # checks the existing installation
./install.sh --uninstall     # removes command, entry and icon
./install.sh --dry-run       # shows every command, runs nothing
```

Without a clone, the same script works from a pipe — it downloads the release
instead of building, and takes the version from the repository because there is
no `Cargo.toml` in a pipe to read:

```sh
curl -fsSL https://raw.githubusercontent.com/bazoocaze/tortty/main/install.sh | bash
curl -fsSL …/install.sh | bash -s -- --prefix /usr/local   # flags after -s --
```

`TORTTY_REPO` (default `bazoocaze/tortty`) is the repository both forms ask.

What it does, in order:

1. obtains the binary (builds, copies `--binary` or downloads `--from-release`);
2. with `--from-release`, takes the `SHA256SUMS` published beside the package and
   refuses a package that does not match it (`--no-verify` skips this). A release
   with no `SHA256SUMS`, a mirror that drops it, or a system with no `sha256sum`
   says so and installs: the check exists to stop a *wrong* download, not to
   break the ones it cannot judge;
3. warns if `tortty` already exists in `PATH` and is not the one from this
   installation;
4. installs to `$PREFIX/bin/tortty` (`install`, copying — swapping the binary in
   `target/` does not leave the installation pointing at nothing);
5. without `--no-shortcut`, writes `$PREFIX/share/applications/tortty.desktop`
   and `$PREFIX/share/icons/hicolor/scalable/apps/tortty.svg`;
6. verifies: runs `tortty --version` and fails if `ldd` shows a missing library;
7. warns if `$PREFIX/bin` is outside `PATH`.

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
tortty-<version>-linux-x86_64.tar.gz     (tortty + tortty.svg + LICENSE + README.md)
tortty-<version>-linux-aarch64.tar.gz
SHA256SUMS
```

The entry for a package needs a line of its own in `SHA256SUMS` for `install.sh`
to verify the download: one file per release, not one per package, because a user
who downloads by hand wants the list of everything at once.

The icon travels **inside** the package, beside the binary: `install.sh` looks for
it there (or in `assets/`, in a checkout) instead of downloading it from `main`,
which would put an unversioned file into a versioned install. Whoever extracts the
package by hand gets the binary, the icon, the license and the README — and the one
step left is putting `tortty` on the `PATH`.

Both are glibc builds, and a glibc build only runs where the C library is at least
as new as the one it was linked against. The release is linked on Ubuntu 22.04, so a
package needs **glibc 2.35** and runs on 22.04 and newer (24.04, 26.04, Debian 12+) —
measured, not assumed: the first release was linked on the 26.04 runner, demanded
`GLIBC_2.43` and refused to start on 24.04 LTS. `tools/build-release.sh` prints the
floor of every package it makes (`== needs glibc 2.35 (linked on: Ubuntu 22.04.5 LTS)`),
which is the one line that answers "does this run here?". Alpine and other musl
systems are not covered by these packages.

The name says the system and the machine, because that is the question whoever
installs has. It is **not** the rustc target: `x86_64-unknown-linux-gnu` puts
`unknown` (the vendor slot of a machine with no vendor to declare) and `gnu` (the
C library) in front of the reader, and neither answers "does this run here?". The
target still names **what to build** (`--target` takes it verbatim, CI included);
only the package name is written for the reader. The table lives in
`tools/targets.sh`, `install.sh` carries a copy to fetch with, and
`tools/targets-check.sh` (a `quality-check.sh` step) fails when the two disagree —
a drifted copy would only show up as a 404 at install time.

What generates that is `.github/workflows/release.yml`, which only calls
`tools/build-release.sh` — the **same** command that is run on the machine:

```sh
./tools/build-release.sh                      # packages the host into dist/
./tools/build-release.sh --target aarch64-unknown-linux-gnu
./tools/build-release.sh --check-version v0.1.0   # CI uses it: the tag has to match
```

The script checks the `Cargo.toml` version against the tag, runs the packaged
`tortty --version` — directly when the target is the machine it runs on, under
`qemu-<arch>-static` when it is a cross build, and it says so in the log when that
emulator does not exist instead of skipping the check silently — and leaves the
`.tar.gz` + `.sha256` in `dist/`.

Publishing is one command, `./tools/make-release.sh` (with `--dry-run` to see the
checks without tagging): it refuses a dirty tree, an unpushed `HEAD`, a red CI run
for that commit and a tag that already exists, then pushes `v<version>`. The
download repository comes from `TORTTY_REPO` (default `bazoocaze/tortty`).

A checkout asks for the version of its own `Cargo.toml`; a piped script cannot,
so it takes the tag from the redirect of `/releases/latest` and installs the
newest release. That is the one difference between the two forms — everything
after the download is the same code. macOS and Windows are
not in the matrix yet: they are new targets (`M-X`/`M-Y` of
`docs/BACKLOG.md`, with signing in `M-Z`): a target nobody has exercised yet.
