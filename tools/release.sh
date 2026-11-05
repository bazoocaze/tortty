#!/usr/bin/env bash
# release.sh — empacota o binário para distribuir (o que o CI faz por tag).
#
# Mesmo script no local e no CI: aqui se monta o `.tar.gz` que o
# `install.sh --from-release` sabe baixar, com nome previsível:
#
#   bzmux-<versão>-<triple>.tar.gz   (contém `bzmux`, LICENSE e README.md)
#
# Uso:
#   ./tools/release.sh                 compila e empacota para o host
#   ./tools/release.sh --target TRIPLE cross-compila (precisa do linker certo)
#   ./tools/release.sh --skip-build    empacota o que já está em target/<triple>/release
#   ./tools/release.sh --out DIR       onde deixar o pacote (default: dist/)
#
# O `--check-version v1.2.3` confere que a tag casa com a versão do Cargo.toml
# (o CI usa isso para não publicar tag apontando para outro código).
set -euo pipefail

APP=bzmux
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)
OUT=$ROOT/dist
TARGET=""
BUILD=1
CHECK=""

say()  { printf '%s\n' "$*"; }
die()  { printf '%s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --target)       TARGET=${2:?--target ask um triple}; shift 2 ;;
        --target=*)     TARGET=${1#*=}; shift ;;
        --out)          OUT=${2:?--out ask um diretório}; shift 2 ;;
        --out=*)        OUT=${1#*=}; shift ;;
        --skip-build)   BUILD=0; shift ;;
        --check-version) CHECK=${2:?--check-version ask a tag}; shift 2 ;;
        --check-version=*) CHECK=${1#*=}; shift ;;
        -h|--help)      awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "$0"; exit 0 ;;
        *) die "opção inválida: $1 (use -h)" ;;
    esac
done

VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1)
[ -n "$VERSION" ] || die "não achei a versão em Cargo.toml"

if [ -n "$CHECK" ]; then
    tag=${CHECK#v}
    [ "$tag" = "$VERSION" ] || die "tag $CHECK não bate com Cargo.toml ($VERSION)"
    say "== tag $CHECK confere com a versão $VERSION"
fi

if [ -z "$TARGET" ]; then
    TARGET=$(rustc -vV | sed -n 's/^host: //p')
fi

if [ "$BUILD" = 1 ]; then
    say "== compilando $APP $VERSION para $TARGET"
    if [ "$TARGET" = "$(rustc -vV | sed -n 's/^host: //p')" ]; then
        cargo build --release --locked --manifest-path "$ROOT/Cargo.toml"
    else
        cargo build --release --locked --target "$TARGET" --manifest-path "$ROOT/Cargo.toml"
    fi
fi

HOST=$(rustc -vV | sed -n 's/^host: //p')
BIN=$ROOT/target/release/$APP
[ "$TARGET" != "$HOST" ] && BIN=$ROOT/target/$TARGET/release/$APP
[ -x "$BIN" ] || die "binário não encontrado: $BIN (compilou?)"

# Sanidade: o binário tem que responder. Cross usa qemu se existir (é o que o
# CI faz); sem qemu, avisa e segue — empacotar sem executar é melhor que não
# empacotar, mas o aviso fica no log para ninguém confundir com verificação.
RUN=""
if [ "$TARGET" != "$HOST" ]; then
    case "$TARGET" in
        aarch64-*) QEMU=qemu-aarch64-static ;;
        x86_64-*)  QEMU=qemu-x86_64-static ;;
        *)         QEMU="" ;;
    esac
    if [ -n "$QEMU" ] && command -v "$QEMU" >/dev/null 2>&1; then
        RUN=$QEMU
    else
        say "== aviso: sem ${QEMU:-qemu} — não dá para rodar o binário $TARGET aqui"
    fi
fi
if [ -n "$RUN" ]; then
    "$RUN" "$BIN" --version || die "$BIN não roda nem no qemu"
fi

mkdir -p "$OUT"
STAGE=$(mktemp -d)
cp "$BIN" "$STAGE/$APP"
cp "$ROOT/LICENSE" "$STAGE/LICENSE"
cp "$ROOT/README.md" "$STAGE/README.md"
PKG="$OUT/$APP-$VERSION-$TARGET.tar.gz"
tar -czf "$PKG" -C "$STAGE" "$APP" LICENSE README.md
rm -rf "$STAGE"

say "== $PKG ($(du -h "$PKG" | cut -f1))"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUT" && sha256sum "$(basename "$PKG")" | tee "$(basename "$PKG").sha256")
fi
