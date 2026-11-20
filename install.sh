#!/usr/bin/env bash
# install.sh — installs tortty as the `tortty` command.
#
# There is no dependency step: the binary is single and only needs libc/libm (+
# the GPU driver at runtime). Installing = copying one file and, if you want, a
# menu entry.
#
# Usage (from the cloned repository):
#   ./install.sh                 builds (release) and installs into ~/.local/bin/tortty
#   ./install.sh --prefix DIR    another prefix (the command lands in DIR/bin/tortty)
#   ./install.sh --binary FILE   installs an already built binary (does not build)
#   ./install.sh --from-release  downloads the binary from GitHub Releases (no cargo)
#   ./install.sh --no-verify     does not check the download against SHA256SUMS
#   ./install.sh --local         command points at target/release/tortty (dev)
#   ./install.sh --no-shortcut   does not create the system menu entry
#   ./install.sh --check         checks the existing installation
#   ./install.sh --uninstall     removes command, entry and icon
#   ./install.sh --dry-run       prints every command and executes nothing
#
# MIT (see LICENSE).
set -euo pipefail

APP=tortty
# Repository --from-release downloads from; TORTTY_REPO overrides it (forks, mirrors).
REPO=${TORTTY_REPO:-bazoocaze/tortty}
PREFIX=${TORTTY_PREFIX:-$HOME/.local}

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { warn "tortty: $*"; exit 1; }

DRY=0; SHORTCUT=1; MODE=install; LOCAL=0; FROM_RELEASE=0; BINARY=""; VERIFY=1
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)      PREFIX=${2:?--prefix needs a directory}; shift 2 ;;
        --prefix=*)    PREFIX=${1#*=}; shift ;;
        --binary)      BINARY=${2:?--binary needs a file}; shift 2 ;;
        --binary=*)    BINARY=${1#*=}; shift ;;
        --from-release) FROM_RELEASE=1; shift ;;
        --local)       LOCAL=1; shift ;;
        --shortcut)    SHORTCUT=1; shift ;;
        --no-shortcut) SHORTCUT=0; shift ;;
        --uninstall)   MODE=uninstall; shift ;;
        --check)       MODE=check; shift ;;
        --no-verify)   VERIFY=0; shift ;;
        --dry-run)     DRY=1; shift ;;
        -h|--help)     awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "$0"; exit 0 ;;
        *) die "invalid option: $1 (use -h)" ;;
    esac
done

BINDIR=$PREFIX/bin
DESKTOP=$PREFIX/share/applications/tortty.desktop
ICON=$PREFIX/share/icons/hicolor/scalable/apps/tortty.svg

run() {
    if [ "$DRY" = 1 ]; then say "+ $*"; else "$@"; fi
}

# sudo only when the destination is not writable (e.g. --prefix /usr/local)
SUDO=""
# -w only holds for an existing directory: walk up to the nearest ancestor
writable() {
    local d=$1
    while [ ! -e "$d" ]; do d=$(dirname "$d"); done
    [ -w "$d" ]
}
need_sudo() {
    writable "$1" && { SUDO=""; return 0; }
    [ "$(id -u)" = 0 ] && { SUDO=""; return 0; }
    # target inside $HOME and not writable: sudo does not help (you own it), the
    # case is a read-only home — say what to do instead of trying to escalate
    case "$1" in
        "$HOME"/*)
            die "no write access to $1 (read-only home?); use --prefix <writable dir>" ;;
    esac
    if command -v sudo >/dev/null 2>&1; then
        SUDO="sudo"
        # a container with no-new-privileges kills sudo and dumps its error;
        # warn beforehand, without blocking interactive use (password)
        sudo -n true 2>/dev/null ||
            say "== warning: sudo does not run without a password here; if it fails, use --prefix \$HOME/.local"
        return 0
    fi
    die "no write permission on $1 and no sudo (use --prefix \$HOME/.local)"
}

here() { cd "$(dirname "${BASH_SOURCE[0]:-$0}")" 2>/dev/null && pwd; }

# Empty when there is no checkout: `0.0.0` used to be answered here as a
# placeholder, and a piped install then asked GitHub for a `v0.0.0` release.
version() {
    if [ -f "$ROOT/Cargo.toml" ]; then
        sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1
    fi
}

# The name a release ships under, from what the machine calls itself. It is a copy
# of `tools/targets.sh` on purpose: a piped install has no checkout to source, and
# `tools/targets-check.sh` fails when the two drift apart. `--target` and rustc
# spellings (`x86_64-unknown-linux-gnu`) stay out of the name on purpose — see there.
pkg_suffix() {
    local m=${1:-$(uname -m)}
    case "$m" in
        x86_64|amd64)   printf 'linux-x86_64' ;;
        aarch64|arm64)  printf 'linux-aarch64' ;;
        *) die "no published package for this machine: $m (use --binary, or build from a clone)" ;;
    esac
}

# -- already on the PATH? ------------------------------------------------
check_conflict() {
    local cur real mine
    cur=$(command -v "$APP" 2>/dev/null || true)
    [ -n "$cur" ] || return 0
    real=$(readlink -f "$cur" 2>/dev/null || printf %s "$cur")
    mine=$(readlink -f "$BINDIR/$APP" 2>/dev/null || printf %s "$BINDIR/$APP")
    [ "$real" = "$mine" ] && return 0
    warn "== '$APP' already exists at $real; this installation goes to $BINDIR/$APP"
    warn "   and whoever answers on the PATH is the first one listed (check with: $APP --version)"
}

# -- release download ----------------------------------------------------
# The tag to ask for. A clone answers with the version in `Cargo.toml`; a script
# in a pipe (`curl … | bash`) has no file to read, so the only version it can
# know is the one the repository publishes. `/releases/latest` answers with a
# redirect to `…/releases/tag/v<version>`, which is the tag — no API call, no
# token, and it keeps working when the project changes its name.
latest_tag() {
    local url="https://github.com/$REPO/releases/latest" final
    final=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$url" 2>/dev/null) || return 1
    case "$final" in
        */releases/tag/v*) printf '%s' "${final##*/releases/tag/}" ;;
        *) return 1 ;;
    esac
}

release_tag() {
    local v
    v=$(version)
    case "$v" in
        ""|0.0.0) latest_tag ;;
        *) printf 'v%s' "$v" ;;
    esac
}

# The URL `--from-release` would fetch. Kept apart from `fetch_release` because the
# dry run has to name it without touching the network — and because a function's
# stdout *is* its return value, so anything written for the reader goes to stderr.
release_url() {
    local tag
    tag=$(release_tag) || return 1
    printf '%s' "https://github.com/$REPO/releases/download/$tag/tortty-${tag#v}-$(pkg_suffix).tar.gz"
}

fetch_release() {
    local url tmp bin pkg
    command -v curl >/dev/null 2>&1 || die "--from-release needs curl"
    url=$(release_url) || die "no release to download: https://github.com/$REPO/releases has none (use --binary, or build from a clone)"
    pkg=${url##*/}
    tmp=$(mktemp -d)
    printf '== downloading %s\n' "$url" >&2
    curl -fsSL "$url" -o "$tmp/$pkg" \
        || die "no package at $url (use --binary, or build from a clone)"
    verify_release "$tmp/$pkg" "$pkg" "${url%/*}"
    tar -xzf "$tmp/$pkg" -C "$tmp"
    bin=$(find "$tmp" -type f -name "$APP" -perm -u+x | head -1)
    [ -n "$bin" ] || die "the downloaded package has no '$APP' binary"
    printf '%s' "$bin"
}

# A download crosses a CDN, a proxy or a mirror; the checksum turns "the bytes arrived"
# into "these are the bytes the release published". A release that carries no SHA256SUMS
# (or a system without sha256sum) warns and installs — refusing there would break mirrors
# and older releases; a *mismatch*, which is the case that matters, stops the install.
verify_release() {
    local file=$1 pkg=$2 base=$3 sums line
    [ "$VERIFY" = 1 ] || return 0
    if ! command -v sha256sum >/dev/null 2>&1; then
        warn "== no sha256sum here: the download is not verified"
        return 0
    fi
    sums=$(mktemp)
    if ! curl -fsSL "$base/SHA256SUMS" -o "$sums" 2>/dev/null; then
        warn "== the release publishes no SHA256SUMS: the download is not verified"
        rm -f "$sums"
        return 0
    fi
    line=$(awk -v p="$pkg" '$2 == p' "$sums")
    rm -f "$sums"
    if [ -z "$line" ]; then
        warn "== SHA256SUMS has no line for $pkg: the download is not verified"
        return 0
    fi
    printf '%s\n' "$line" | ( cd "$(dirname "$file")" && sha256sum -c --quiet - ) \
        || die "$pkg does not match the published checksum (use --no-verify to install anyway)"
    # stderr: this function's caller reads stdout as the binary path
    warn "== checksum ok: $pkg"
}

# -- installation --------------------------------------------------------
do_install() {
    local src="" url=""
    if [ "$LOCAL" = 1 ] && [ "$FROM_RELEASE" = 1 ]; then
        die "--local and --from-release do not combine"
    fi

    if [ -n "$BINARY" ]; then
        [ -f "$BINARY" ] || die "binary not found: $BINARY"
        src=$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")
        say "== using binary: $src"
    elif [ "$FROM_RELEASE" = 1 ]; then
        if [ "$DRY" = 1 ]; then
            url=$(release_url) || die "no release to download: https://github.com/$REPO/releases has none (use --binary, or build from a clone)"
            say "== dry run: would download $url"
            say "== dry run: nothing downloaded, nothing installed (the plan continues once the package is unpacked)"
            return 0
        fi
        src=$(fetch_release)
    elif [ "$LOCAL" = 1 ]; then
        command -v cargo >/dev/null 2>&1 || die "--local needs cargo"
        run cargo build --release --manifest-path "$ROOT/Cargo.toml" --locked
        src="$ROOT/target/release/$APP"
        [ -f "$src" ] || die "build did not produce $src"
    elif [ -n "$ROOT" ] && [ -f "$ROOT/Cargo.toml" ]; then
        command -v cargo >/dev/null 2>&1 || die "cargo missing (install rustup, or use --from-release)"
        say "== building (release, ~2 min the first time)"
        run cargo build --release --manifest-path "$ROOT/Cargo.toml" --locked
        src="$ROOT/target/release/$APP"
        [ -f "$src" ] || die "build did not produce $src"
    else
        src=$(fetch_release)
    fi

    check_conflict
    need_sudo "$PREFIX"
    run mkdir -p "$BINDIR"

    if [ "$LOCAL" = 1 ]; then
        say "== link: $BINDIR/$APP -> $src"
        run $SUDO ln -sfn "$src" "$BINDIR/$APP"
    else
        say "== installing: $BINDIR/$APP"
        run $SUDO command install -m 755 "$src" "$BINDIR/$APP"
    fi

    if [ "$SHORTCUT" = 1 ]; then
        # The icon travels inside the package, beside the binary; in a checkout it
        # is in assets/. Taking it from `main` over the network used to be the third
        # option, and it mixed an unversioned file into a versioned install.
        local icon_src="$ROOT/assets/$APP.svg"
        [ -f "$icon_src" ] || icon_src="$(dirname "$src")/$APP.svg"
        [ -f "$icon_src" ] ||
            warn "== warning: no $APP.svg beside the binary; the menu entry keeps the generic icon"
        if [ -f "$icon_src" ]; then
            run $SUDO mkdir -p "$(dirname "$ICON")"
            run $SUDO command install -m 644 "$icon_src" "$ICON"
            if command -v gtk-update-icon-cache >/dev/null 2>&1 && [ "$DRY" = 0 ]; then
                $SUDO gtk-update-icon-cache -q -t -f "$PREFIX/share/icons/hicolor" 2>/dev/null || true
            fi
        fi
        run $SUDO mkdir -p "$(dirname "$DESKTOP")"
        if [ "$DRY" = 1 ]; then
            say "+ write $DESKTOP (Exec=$BINDIR/$APP)"
        else
            local tmpd
            tmpd=$(mktemp)
            cat >"$tmpd" <<EOF
[Desktop Entry]
Type=Application
Name=TorTTY
GenericName=Terminal
Comment=Fast, GPU based terminal
Exec=$BINDIR/$APP
TryExec=$BINDIR/$APP
Icon=$APP
StartupWMClass=$APP
Terminal=false
Categories=System;TerminalEmulator;
Keywords=terminal;shell;console;
# no startup notification: the app does not honor the contract (B7)
StartupNotify=false
EOF
            $SUDO command install -m 644 "$tmpd" "$DESKTOP"
            rm -f "$tmpd"
            command -v update-desktop-database >/dev/null 2>&1 \
                && $SUDO update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
        fi
    fi

    if [ "$DRY" = 1 ]; then say "+ $BINDIR/$APP --version"; return 0; fi

    # -- verification -----------------------------------------------------
    say "== verification"
    "$BINDIR/$APP" --version
    if command -v ldd >/dev/null 2>&1; then
        if ldd "$BINDIR/$APP" 2>/dev/null | grep -q 'not found'; then
            die "the binary has a missing library: $(ldd "$BINDIR/$APP" | grep 'not found')"
        fi
    fi
    case ":$PATH:" in
        *":$BINDIR:"*) : ;;
        *) warn "== $BINDIR is not on the PATH: add it to your shell (export PATH=\"$BINDIR:\$PATH\")" ;;
    esac

    say
    say "done: run '$APP' (to update: run this installer again)"
}

do_check() {
    local bin="$BINDIR/$APP"
    [ -x "$bin" ] || die "not installed at $bin"
    say "binary:   $bin"
    say "version:  $("$bin" --version)"
    say "size:     $(du -h "$bin" | cut -f1)"
    local cur
    cur=$(command -v "$APP" 2>/dev/null || true)
    case "$cur" in
        "$BINDIR/$APP") say "link:     $cur (this prefix, on the PATH)" ;;
        "")             say "link:     (nothing on the PATH: add $BINDIR)" ;;
        *)              say "link:     $cur (another $APP: $BINDIR does not come first on the PATH)" ;;
    esac
    if command -v ldd >/dev/null 2>&1; then
        say "libs:     $(ldd "$bin" | awk '{print $1}' | grep -c . ) entries, missing: $(ldd "$bin" | grep -c 'not found')"
    fi
    [ -f "$DESKTOP" ] && say "menu:     $DESKTOP" || say "menu:     (no entry)"
    [ -f "$ICON" ] && say "icon:     $ICON" || true
    if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
        say "screen:   ${DISPLAY:-${WAYLAND_DISPLAY}}"
    fi
}

do_uninstall() {
    need_sudo "$PREFIX"
    if [ -L "$BINDIR/$APP" ] || [ -f "$BINDIR/$APP" ]; then
        say "== removing $BINDIR/$APP"
        run $SUDO rm -f "$BINDIR/$APP"
    else
        say "== nothing at $BINDIR/$APP"
    fi
    for f in "$DESKTOP" "$ICON"; do
        [ -e "$f" ] && { run $SUDO rm -f "$f"; }
    done
    say "done: '$APP' removed from $PREFIX"
}

ROOT=$(here)
case "$MODE" in
    install)   do_install ;;
    check)     do_check ;;
    uninstall) do_uninstall ;;
esac
