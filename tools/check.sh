#!/bin/sh
# check.sh — the quality ruler: formatting, lints and tests.
#
# It is the same command in the pre-commit and in CI, on purpose: if the hook and CI
# disagree, the hook is worthless.
#
#   tools/check.sh            # fmt --check, clippy, tests, release build
#   tools/check.sh --fast     # without the release build (what the pre-commit runs)
#   tools/check.sh --fix      # applies fmt and clippy --fix before checking
#
# Exits with 0 only if everything passes. Nothing here runs a GUI: `mc`/`vim` stay
# manual (docs/DEBUG.md).
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
cd "$HERE"

FAST=0
FIX=0
for arg in "$@"; do
    case "$arg" in
        --fast) FAST=1 ;;
        --fix) FIX=1 ;;
        -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

failed=0
step() {
    name=$1
    shift
    printf '%-22s' "$name"
    if output=$("$@" 2>&1); then
        echo "ok"
    else
        echo "FAILED"
        echo "$output" | tail -30 | sed 's/^/    /'
        failed=1
    fi
}

if [ "$FIX" = 1 ]; then
    cargo fmt --all
    # clippy's `--fix` refuses a dirty tree by default; here it is intentional
    cargo clippy --fix --allow-dirty --allow-staged --all-targets \
        --workspace --release >/dev/null 2>&1 || true
fi

step "cargo fmt --check" cargo fmt --all -- --check
step "clippy -D warnings" cargo clippy --workspace --all-targets --release -- -D warnings
step "cargo test" cargo test --workspace --quiet

if [ "$FAST" = 0 ]; then
    step "build --release" cargo build --release --workspace
    # parsebench is the display-less benchmark: if it breaks, the number in
    # docs/BENCHMARKS.md dies with it
    # `--selftest` is the display-less emulator smoke test: 14 core areas
    step "selftest" ./target/release/bzmux --selftest
    step "parsebench" ./target/release/parsebench --frames 100 --cols 40 --rows 12
fi

if [ "$failed" = 0 ]; then
    echo "all green"
else
    echo "something above is red" >&2
fi
exit "$failed"
