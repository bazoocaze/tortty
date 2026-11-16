#!/bin/sh
# platform-gate.sh — the seam: the operating system is named in one place.
#
# `crates/app/src/platform/` is the only module allowed to name a system: the
# syscalls, the paths, the signals, the shell quoting and the windowing details
# live there, behind a capability name (`try_lock`, `hangup`, `shell_quote`).
# Everywhere else the code asks for the capability. The rule is written in
# `crates/app/src/platform/mod.rs`; this is the ruler that keeps it true.
#
# It is a step of `tools/quality-check.sh`, so the pre-commit hook and CI apply
# exactly the same one.
#
# Exit 0 = the seam holds; the mismatch is printed otherwise.
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
cd "$HERE"

SEAM=crates/app/src/platform/
PATTERN='std::os::(unix|linux)|libc::|windows_sys::|winapi::|winit::platform::'

leak=$(grep -rnE "$PATTERN" crates/ --include='*.rs' | grep -v "^$SEAM" || true)
if [ -n "$leak" ]; then
    echo "the operating system is named outside $SEAM:" >&2
    echo "$leak" | sed 's/^/    /' >&2
    echo "ask the seam for the capability instead (docs/PORTABILITY.md)" >&2
    exit 1
fi
