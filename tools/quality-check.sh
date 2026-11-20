#!/bin/sh
# quality-check.sh — the quality ruler: formatting, lints and tests.
#
# It is the same command in the pre-commit and in CI, on purpose: if the hook and
# CI disagree, the hook is worthless.
#
#   tools/quality-check.sh                    # every step, in order
#   tools/quality-check.sh --fast             # without the build (the pre-commit)
#   tools/quality-check.sh --only test,build  # named steps, for parallel jobs
#   tools/quality-check.sh --fix              # applies fmt and clippy --fix first
#
# The steps are `seam`, `targets`, `fmt`, `clippy`, `test`, `build`, `selftest` and
# `parsebench`. `--only` exists so CI can run them in parallel instead of in one
# five-minute queue: the steps and their order stay defined here, one ruler.
#
# Exits with 0 only if everything asked for passes. Nothing here runs a GUI:
# `mc`/`vim` stay manual (docs/DEBUG.md).
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
cd "$HERE"

FAST=0
FIX=0
ONLY=""
while [ $# -gt 0 ]; do
    case "$1" in
        --fast) FAST=1 ;;
        --fix) FIX=1 ;;
        --only)
            shift
            [ $# -gt 0 ] || {
                echo "--only needs a list of steps" >&2
                exit 2
            }
            ONLY=$1
            ;;
        --only=*) ONLY=${1#--only=} ;;
        -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

# `--fast` is a named subset, not a second mechanism: one way to skip a step
if [ "$FAST" = 1 ] && [ -z "$ONLY" ]; then
    ONLY=seam,targets,fmt,clippy,test
fi

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

wanted() {
    [ -z "$ONLY" ] && return 0
    case ",$ONLY," in
        *",$1,"*) return 0 ;;
        *) return 1 ;;
    esac
}

run() {
    key=$1
    name=$2
    shift 2
    if wanted "$key"; then
        step "$name" "$@"
    fi
}

if [ "$FIX" = 1 ]; then
    cargo fmt --all
    # clippy's `--fix` refuses a dirty tree by default; here it is intentional
    cargo clippy --fix --allow-dirty --allow-staged --all-targets \
        --workspace --release >/dev/null 2>&1 || true
fi

# the platform seam first: it is a grep, it costs nothing, and it fails with the
# offending line instead of a compile error on another target
run seam "platform seam" ./tools/platform-gate.sh
# the package name has two authors (tools/targets.sh and install.sh); a copy that
# drifts fails at install time as a 404, so it is compared here, before that
run targets "package name table" ./tools/targets-check.sh
run fmt "cargo fmt --check" cargo fmt --all -- --check
run clippy "clippy -D warnings" cargo clippy --workspace --all-targets --release -- -D warnings
run test "cargo test" cargo test --workspace --quiet
run build "build --release" cargo build --release --workspace
# `--selftest` is the display-less emulator smoke test: 14 core areas
run selftest "selftest" ./target/release/tortty --selftest
# parsebench is the display-less benchmark: if it breaks, the number in
# docs/BENCHMARKS.md dies with it
run parsebench "parsebench" ./target/release/parsebench --frames 100 --cols 40 --rows 12

if [ "$failed" = 0 ]; then
    echo "all green"
else
    echo "something above is red" >&2
fi
exit "$failed"
