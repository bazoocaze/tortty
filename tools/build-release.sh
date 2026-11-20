#!/usr/bin/env bash
# build-release.sh — packages the binary for distribution (what CI does per tag).
#
# Same script locally and in CI: here it builds the `.tar.gz` that
# `install.sh --from-release` knows how to download, with a predictable name:
#
#   tortty-<version>-<system>-<machine>.tar.gz   (`tortty`, tortty.svg, LICENSE, README.md)
#
# The suffix comes from `tools/targets.sh`, shared with install.sh: the rustc
# target decides *what to build* (`--target` still takes it verbatim), but the
# name says what the reader needs — `linux-x86_64`, not `x86_64-unknown-linux-gnu`.
#
# Usage:
#   ./tools/build-release.sh              builds and packages for the host
#   ./tools/build-release.sh --target T   cross-compiles (needs the right linker)
#   ./tools/build-release.sh --skip-build packages what is in target/<triple>/release
#   ./tools/build-release.sh --out DIR    where to leave the package (default: dist/)
#
# `--check-version v1.2.3` checks that the tag matches the version in Cargo.toml
# (CI uses it to avoid publishing a tag pointing at other code).
set -euo pipefail

APP=tortty
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)
OUT=$ROOT/dist
TARGET=""
BUILD=1
CHECK=""

say()  { printf '%s\n' "$*"; }
die()  { printf '%s\n' "$*" >&2; exit 1; }

. "$ROOT/tools/targets.sh"

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

# Sanity: the binary has to answer. A native build runs it directly; a cross build runs
# it under qemu when qemu exists, and without qemu warns and continues — packaging
# without executing is better than not packaging, but the warning stays in the log so
# nobody mistakes it for verification.
if [ "$TARGET" = "$HOST" ]; then
    "$BIN" --version || die "$BIN does not even run on this machine"
else
    case "$TARGET" in
        aarch64-*) QEMU=qemu-aarch64-static ;;
        x86_64-*)  QEMU=qemu-x86_64-static ;;
        *)         QEMU="" ;;
    esac
    if [ -n "$QEMU" ] && command -v "$QEMU" >/dev/null 2>&1; then
        "$QEMU" "$BIN" --version || die "$BIN does not even run under $QEMU"
    else
        say "== warning: no ${QEMU:-qemu} — cannot run the $TARGET binary here"
    fi
fi

SUFFIX=$(pkg_suffix "$TARGET") || die "no package name for $TARGET (see tools/targets.sh)"
[ -f "$ROOT/assets/$APP.svg" ] || die "missing icon: assets/$APP.svg"

mkdir -p "$OUT"
STAGE=$(mktemp -d)
cp "$BIN" "$STAGE/$APP"
cp "$ROOT/LICENSE" "$STAGE/LICENSE"
cp "$ROOT/README.md" "$STAGE/README.md"
cp "$ROOT/assets/$APP.svg" "$STAGE/$APP.svg"
PKG="$OUT/$APP-$VERSION-$SUFFIX.tar.gz"
tar -czf "$PKG" -C "$STAGE" "$APP" LICENSE README.md "$APP.svg"
rm -rf "$STAGE"

say "== $PKG ($(du -h "$PKG" | cut -f1))"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUT" && sha256sum "$(basename "$PKG")" | tee "$(basename "$PKG").sha256")
fi
