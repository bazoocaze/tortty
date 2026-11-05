#!/usr/bin/env bash
# install.sh — installs bzmux (Rust) as the `bzmux` command.
#
# Unlike the bzmux Python installer, there is no dependency step here:
# the binary is single and only needs libc/libm (+ the GPU driver at
# runtime). Installing = copying one file and, if you want, a menu entry.
#
# Usage (from the cloned repository):
#   ./install.sh                 builds (release) and installs into ~/.local/bin/bzmux
#   ./install.sh --prefix DIR    another prefix (the command lands in DIR/bin/bzmux)
#   ./install.sh --binary FILE   installs an already built binary (does not build)
#   ./install.sh --from-release  downloads the binary from GitHub Releases (no cargo)
#   ./install.sh --local         command points at target/release/bzmux (dev)
#   ./install.sh --no-shortcut   does not create the system menu entry
#   ./install.sh --check         checks the existing installation
#   ./install.sh --uninstall     removes command, entry and icon
#   ./install.sh --dry-run       prints every command and executes nothing
#
# Living with the bzmux Python: it installs through pipx under the name `bzmux`; this
# installer takes the name and warns if it finds the pipx shim on the path. After
# installing the Python one under the new name (`tortty-python`), run `pipx uninstall bzmux`
# so the old shim does not linger.
#
# MIT (see LICENSE).
set -euo pipefail

APP=bzmux
# Repository the --from-release mode downloads from; change it when the repo goes public.
REPO=${BZMUX_REPO:-bazoocaze/rust_term}
PREFIX=${BZMUX_PREFIX:-$HOME/.local}

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { warn "bzmux: $*"; exit 1; }

DRY=0; SHORTCUT=1; MODE=install; LOCAL=0; FROM_RELEASE=0; BINARY=""
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
        --dry-run)     DRY=1; shift ;;
        -h|--help)     awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "$0"; exit 0 ;;
        *) die "invalid option: $1 (use -h)" ;;
    esac
done

BINDIR=$PREFIX/bin
DESKTOP=$PREFIX/share/applications/bzmux.desktop
ICON=$PREFIX/share/icons/hicolor/scalable/apps/bzmux.svg

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

version() {
    if [ -f "$ROOT/Cargo.toml" ]; then
        sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1
    else
        printf '0.0.0'
    fi
}

target_triple() {
    local m
    m=$(uname -m)
    case "$m" in
        x86_64|amd64)   printf 'x86_64-unknown-linux-gnu' ;;
        aarch64|arm64)  printf 'aarch64-unknown-linux-gnu' ;;
        *) die "architecture with no published binary: $m (build with --binary)" ;;
    esac
}

# -- coexistence warnings ------------------------------------------------
check_conflict() {
    local cur real mine
    cur=$(command -v "$APP" 2>/dev/null || true)
    [ -n "$cur" ] || return 0
    real=$(readlink -f "$cur" 2>/dev/null || printf %s "$cur")
    mine=$(readlink -f "$BINDIR/$APP" 2>/dev/null || printf %s "$BINDIR/$APP")
    [ "$real" = "$mine" ] && return 0
    # the pipx shim in ~/.local/bin has no "pipx" in the path: its target does
    case "$cur$real" in
        *pipx*) warn "== '$APP' is the **Python** bzmux today ($real)"
                warn "   this installer takes the name over: install the Python one as 'tortty-python'"
                warn "   and then run 'pipx uninstall bzmux' (the venv stays in"
                warn "   ~/.local/share/pipx/venvs/bzmux until you remove it)" ;;
        *)      warn "== '$APP' already exists at $real; this installation goes to $BINDIR/$APP"
                warn "   and whoever answers on the PATH is the first one listed (check with: $APP --version)" ;;
    esac
}

# -- release download ----------------------------------------------------
fetch_release() {
    local look tag file url tmp
    look=$(version); tag="v$ver"
    file="bzmux-$ver-$(target_triple).tar.gz"
    url="https://github.com/$REPO/releases/download/$tag/$file"
    command -v curl >/dev/null 2>&1 || die "--from-release needs curl"
    tmp=$(mktemp -d)
    say "== downloading $url"
    curl -fsSL "$url" -o "$tmp/pkg.tgz" \
        || die "no release at $url (private repo or missing tag: use --binary or build)"
    tar -xzf "$tmp/pkg.tgz" -C "$tmp"
    local bin
    bin=$(find "$tmp" -type f -name "$APP" -perm -u+x | head -1)
    [ -n "$bin" ] || die "the downloaded package has no '$APP' binary"
    printf '%s' "$bin"
}

# -- installation --------------------------------------------------------
do_install() {
    local src=""
    if [ "$LOCAL" = 1 ] && [ "$FROM_RELEASE" = 1 ]; then
        die "--local and --from-release do not combine"
    fi

    if [ -n "$BINARY" ]; then
        [ -f "$BINARY" ] || die "binary not found: $BINARY"
        src=$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")
        say "== using binary: $src"
    elif [ "$FROM_RELEASE" = 1 ]; then
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
        local icon_src="$ROOT/assets/bzmux.svg"
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
Name=bzmux
GenericName=Terminal
Comment=Terminal with workspaces and tabs
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
        *)              say "link:     $cur (another bzmux: $BINDIR does not come first on the PATH)" ;;
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
    say "done: '$APP' removed from $PREFIX (no other bzmux was touched)"
}

ROOT=$(here)
case "$MODE" in
    install)   do_install ;;
    check)     do_check ;;
    uninstall) do_uninstall ;;
esac
