#!/usr/bin/env bash
# install.sh — instala o bzmux (Rust) como o comando `bzmux`.
#
# Diferente do instalador do bzmux Python, aqui não existe passo de dependência:
# o binário é único e só precisa de libc/libm (+ o driver da GPU em tempo de
# execução). Instalar = copiar um arquivo e, se quiser, uma entrada no menu.
#
# Uso (do repositório clonado):
#   ./install.sh                 compila (release) e instala em ~/.local/bin/bzmux
#   ./install.sh --prefix DIR    outro prefixo (o comando fica em DIR/bin/bzmux)
#   ./install.sh --binary ARQ    instala um binário já compilado (não compila)
#   ./install.sh --from-release  baixa o binário do GitHub Releases (sem cargo)
#   ./install.sh --local         comando aponta para target/release/bzmux (dev)
#   ./install.sh --no-shortcut   não cria a entrada no menu do sistema
#   ./install.sh --check         confere a instalação existente
#   ./install.sh --uninstall     remove comando, entrada e ícone
#   ./install.sh --dry-run       mostra cada comando e não executa nada
#
# Conviver com o bzmux Python: ele instala pelo pipx com o nome `bzmux`; este
# instalador toma o nome e avisa se encontrar o shim do pipx no caminho. Depois
# de instalar o Python com o nome novo (`tortty-python`), rode `pipx uninstall bzmux`
# para não sobrar o shim antigo.
#
# MIT (ver LICENSE).
set -euo pipefail

APP=bzmux
# Repositório de onde o --from-release baixa; troque quando o repo for público.
REPO=${BZMUX_REPO:-bazoocaze/rust_term}
PREFIX=${BZMUX_PREFIX:-$HOME/.local}

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { warn "bzmux: $*"; exit 1; }

DRY=0; SHORTCUT=1; MODE=install; LOCAL=0; FROM_RELEASE=0; BINARY=""
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)      PREFIX=${2:?--prefix pede um diretório}; shift 2 ;;
        --prefix=*)    PREFIX=${1#*=}; shift ;;
        --binary)      BINARY=${2:?--binary pede um arquivo}; shift 2 ;;
        --binary=*)    BINARY=${1#*=}; shift ;;
        --from-release) FROM_RELEASE=1; shift ;;
        --local)       LOCAL=1; shift ;;
        --shortcut)    SHORTCUT=1; shift ;;
        --no-shortcut) SHORTCUT=0; shift ;;
        --uninstall)   MODE=uninstall; shift ;;
        --check)       MODE=check; shift ;;
        --dry-run)     DRY=1; shift ;;
        -h|--help)     awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "$0"; exit 0 ;;
        *) die "opção inválida: $1 (use -h)" ;;
    esac
done

BINDIR=$PREFIX/bin
DESKTOP=$PREFIX/share/applications/bzmux.desktop
ICON=$PREFIX/share/icons/hicolor/scalable/apps/bzmux.svg

run() {
    if [ "$DRY" = 1 ]; then say "+ $*"; else "$@"; fi
}

# sudo só quando o destino não é gravável (ex.: --prefix /usr/local)
SUDO=""
# -w só vale para diretório existente: sobe até o ancestral mais próximo
writable() {
    local d=$1
    while [ ! -e "$d" ]; do d=$(dirname "$d"); done
    [ -w "$d" ]
}
need_sudo() {
    writable "$1" && { SUDO=""; return 0; }
    [ "$(id -u)" = 0 ] && { SUDO=""; return 0; }
    if command -v sudo >/dev/null 2>&1; then SUDO="sudo"; return 0; fi
    die "sem permissão de escrita em $1 e sem sudo (use --prefix \$HOME/.local)"
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
        *) die "arquitetura sem binário publicado: $m (compile com --binary)" ;;
    esac
}

# -- avisos de convivência ------------------------------------------------
check_conflict() {
    local cur real mine
    cur=$(command -v "$APP" 2>/dev/null || true)
    [ -n "$cur" ] || return 0
    real=$(readlink -f "$cur" 2>/dev/null || printf %s "$cur")
    mine=$(readlink -f "$BINDIR/$APP" 2>/dev/null || printf %s "$BINDIR/$APP")
    [ "$real" = "$mine" ] && return 0
    # o shim do pipx em ~/.local/bin não tem "pipx" no caminho: quem tem é o alvo
    case "$cur$real" in
        *pipx*) warn "== '$APP' hoje é o bzmux **Python** ($real)"
                warn "   este instalador assume o nome: instale o Python como 'tortty-python'"
                warn "   e depois rode 'pipx uninstall bzmux' (a venv continua em"
                warn "   ~/.local/share/pipx/venvs/bzmux enquanto você não remover)" ;;
        *)      warn "== '$APP' já existe em $real; esta instalação fica em $BINDIR/$APP"
                warn "   e quem responde no PATH é o primeiro da lista (confira com: $APP --version)" ;;
    esac
}

# -- download do release --------------------------------------------------
fetch_release() {
    local ver tag file url tmp
    ver=$(version); tag="v$ver"
    file="bzmux-$ver-$(target_triple).tar.gz"
    url="https://github.com/$REPO/releases/download/$tag/$file"
    command -v curl >/dev/null 2>&1 || die "--from-release precisa de curl"
    tmp=$(mktemp -d)
    say "== baixando $url"
    curl -fsSL "$url" -o "$tmp/pkg.tgz" \
        || die "sem release em $url (repo privado ou tag ausente: use --binary ou compile)"
    tar -xzf "$tmp/pkg.tgz" -C "$tmp"
    local bin
    bin=$(find "$tmp" -type f -name "$APP" -perm -u+x | head -1)
    [ -n "$bin" ] || die "o pacote baixado não tem o binário '$APP'"
    printf '%s' "$bin"
}

# -- instalação -----------------------------------------------------------
do_install() {
    local src=""
    if [ "$LOCAL" = 1 ] && [ "$FROM_RELEASE" = 1 ]; then
        die "--local e --from-release não combinam"
    fi

    if [ -n "$BINARY" ]; then
        [ -f "$BINARY" ] || die "binário não encontrado: $BINARY"
        src=$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")
        say "== usando binário: $src"
    elif [ "$FROM_RELEASE" = 1 ]; then
        src=$(fetch_release)
    elif [ "$LOCAL" = 1 ]; then
        command -v cargo >/dev/null 2>&1 || die "--local precisa de cargo"
        run cargo build --release --manifest-path "$ROOT/Cargo.toml" --locked
        src="$ROOT/target/release/$APP"
        [ -f "$src" ] || die "build não gerou $src"
    elif [ -n "$ROOT" ] && [ -f "$ROOT/Cargo.toml" ]; then
        command -v cargo >/dev/null 2>&1 || die "cargo ausente (instale rustup, ou use --from-release)"
        say "== compilando (release, ~2 min na primeira vez)"
        run cargo build --release --manifest-path "$ROOT/Cargo.toml" --locked
        src="$ROOT/target/release/$APP"
        [ -f "$src" ] || die "build não gerou $src"
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
        say "== instalando: $BINDIR/$APP"
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
            say "+ escrever $DESKTOP (Exec=$BINDIR/$APP)"
        else
            local tmpd
            tmpd=$(mktemp)
            cat >"$tmpd" <<EOF
[Desktop Entry]
Type=Application
Name=bzmux
GenericName=Terminal
Comment=Terminal com workspaces e abas
Exec=$BINDIR/$APP
TryExec=$BINDIR/$APP
Icon=$APP
Terminal=false
Categories=System;TerminalEmulator;
Keywords=terminal;shell;console;
StartupNotify=true
EOF
            $SUDO command install -m 644 "$tmpd" "$DESKTOP"
            rm -f "$tmpd"
            command -v update-desktop-database >/dev/null 2>&1 \
                && $SUDO update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
        fi
    fi

    if [ "$DRY" = 1 ]; then say "+ $BINDIR/$APP --version"; return 0; fi

    # -- verificação ------------------------------------------------------
    say "== verificação"
    "$BINDIR/$APP" --version
    if command -v ldd >/dev/null 2>&1; then
        if ldd "$BINDIR/$APP" 2>/dev/null | grep -q 'not found'; then
            die "o binário tem biblioteca faltando: $(ldd "$BINDIR/$APP" | grep 'not found')"
        fi
    fi
    case ":$PATH:" in
        *":$BINDIR:"*) : ;;
        *) warn "== $BINDIR não está no PATH: adicione ao seu shell (export PATH=\"$BINDIR:\$PATH\")" ;;
    esac

    say
    say "pronto: rode '$APP' (atualizar: rode este instalador de novo)"
}

do_check() {
    local bin="$BINDIR/$APP"
    [ -x "$bin" ] || die "não instalado em $bin"
    say "binário:  $bin"
    say "versão:   $("$bin" --version)"
    say "tamanho:  $(du -h "$bin" | cut -f1)"
    local cur
    cur=$(command -v "$APP" 2>/dev/null || true)
    case "$cur" in
        "$BINDIR/$APP") say "link:     $cur (este prefixo, no PATH)" ;;
        "")             say "link:     (nada no PATH: adicione $BINDIR)" ;;
        *)              say "link:     $cur (outro bzmux: $BINDIR não vem antes no PATH)" ;;
    esac
    if command -v ldd >/dev/null 2>&1; then
        say "libs:     $(ldd "$bin" | awk '{print $1}' | grep -c . ) entradas, faltando: $(ldd "$bin" | grep -c 'not found')"
    fi
    [ -f "$DESKTOP" ] && say "menu:     $DESKTOP" || say "menu:     (sem entrada)"
    [ -f "$ICON" ] && say "ícone:    $ICON" || true
    if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
        say "tela:     ${DISPLAY:-${WAYLAND_DISPLAY}}"
    fi
}

do_uninstall() {
    need_sudo "$PREFIX"
    if [ -L "$BINDIR/$APP" ] || [ -f "$BINDIR/$APP" ]; then
        say "== removendo $BINDIR/$APP"
        run $SUDO rm -f "$BINDIR/$APP"
    else
        say "== nada em $BINDIR/$APP"
    fi
    for f in "$DESKTOP" "$ICON"; do
        [ -e "$f" ] && { run $SUDO rm -f "$f"; }
    done
    say "pronto: '$APP' removido de $PREFIX (nenhum outro bzmux foi tocado)"
}

ROOT=$(here)
case "$MODE" in
    install)   do_install ;;
    check)     do_check ;;
    uninstall) do_uninstall ;;
esac
