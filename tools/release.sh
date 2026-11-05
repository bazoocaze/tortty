#!/usr/bin/env bash
# release.sh — packages the binary for distribution (what CI does per tag).
#
# Same script locally and in CI: here it builds the `.tar.gz` that
# `install.sh --from-release` knows how to download, with a predictable name:
#
#   bzmux-<version>-<triple>.tar.gz   (contains `bzmux`, LICENSE and README.md)
#
# Usage:
#   ./tools/release.sh                 builds and packages for the host
#   ./tools/release.sh --target TRIPLE cross-compiles (needs the right linker)
#   ./tools/release.sh --skip-build    packages what is already in target/<triple>/release
#   ./tools/release.sh --out DIR       where to leave the package (default: dist/)
#
# `--check-version v1.2.3` checks that the tag matches the version in Cargo.toml
# (CI uses it to avoid publishing a tag pointing at other code).
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
        --target)       TARGET=${2:?--target needs a triple}; shift 2 ;;
        --target=*)     TARGET=${1#*=}; shift ;;
        --out)          OUT=${2:?--out needs a directory}; shift 2 ;;
        --out=*)        OUT=${1#*=}; shift ;;
        --skip-build)   BUILD=0; shift ;;
        --check-version) CHECK=${2:?--check-version needs the tag}; shift 2 ;;
        --check-version=*) CHECK=${1#*=}; shift ;;
        -h|--help)      awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "$0"; exit 0 ;;
        *) die "invalid option: $1 (use -h)" ;;
    esac
done

VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1)
[ -n "$VERSION" ] || die "could not find the version in Cargo.toml"

if [ -n "$CHECK" ]; then
    tag=${CHECK#v}
    [ "$tag" = "$VERSION" ] || die "tag $CHECK does not match Cargo.toml ($VERSION)"
    say "== tag $CHECK matches version $VERSION"
fi

if [ -z "$TARGET" ]; then
    TARGET=$(rustc -vV | sed -n 's/^host: //p')
fi

if [ "$BUILD" = 1 ]; then
    say "== building $APP $VERSION for $TARGET"
    if [ "$TARGET" = "$(rustc -vV | sed -n 's/^host: //p')" ]; then
        cargo build --release --locked --manifest-path "$ROOT/Cargo.toml"
    else
        cargo build --release --locked --target "$TARGET" --manifest-path "$ROOT/Cargo.toml"
    fi
fi

HOST=$(rustc -vV | sed -n 's/^host: //p')
BIN=$ROOT/target/release/$APP
[ "$TARGET" != "$HOST" ] && BIN=$ROOT/target/$TARGET/release/$APP
[ -x "$BIN" ] || die "binary not found: $BIN (did it build?)"

# Sanity: the binary has to answer. Cross builds use qemu when it exists (that is what
# CI does); without qemu, warn and continue — packaging without executing is better than
# not packaging, but the warning stays in the log so nobody mistakes it for verification.
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
        say "== warning: no ${QEMU:-qemu} — cannot run the $TARGET binary here"
    fi
fi
if [ -n "$RUN" ]; then
    "$RUN" "$BIN" --version || die "$BIN does not even run under qemu"
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
