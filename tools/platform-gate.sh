#!/bin/sh
# platform-gate.sh — the seam: the operating system is named in one place.
#
# `crates/app/src/platform/` is the only module allowed to name a system: the
# syscalls, the paths, the signals, the shell quoting and the windowing details
# live there, behind a capability name (`try_lock`, `hangup`, `shell_quote`).
# Everywhere else the code asks for the capability. The rule is written in
# `crates/app/src/platform/mod.rs`; this is the ruler that keeps it true.
#
# Two halves, because there are two ways to name a system:
#   1. the syscall or the system crate (`libc::`, `std::os::unix`, `winit::platform`);
#   2. the `cfg` that branches on the target (`cfg(target_os = "linux")`), which is
#      the silent `#[cfg]` in the feature code that the rule forbids — it compiles
#      everywhere, so no cross-target build ever reports it.
#
# The core is out of scope for the second half on purpose: `config::Os::current()`
# picks the defaults template, and that is policy (data), not a syscall.
#
# It is a step of `tools/quality-check.sh`, so the pre-commit hook and CI apply
# exactly the same one.
#
# Exit 0 = the seam holds; the offending line is printed otherwise.
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
cd "$HERE"

SEAM=crates/app/src/platform/
SYSCALL='std::os::(unix|linux)|libc::|windows_sys::|winapi::|winit::platform::'
CFG='target_os|target_family|target_arch|cfg!?\(\s*(windows|unix)\s*\)'

status=0

leak=$(grep -rnE "$SYSCALL" crates/ --include='*.rs' | grep -v "^$SEAM" || true)
if [ -n "$leak" ]; then
    echo "the operating system is named outside $SEAM:" >&2
    echo "$leak" | sed 's/^/    /' >&2
    status=1
fi

leak=$(grep -rnE "$CFG" crates/app/ --include='*.rs' | grep -v "^$SEAM" || true)
if [ -n "$leak" ]; then
    echo "a cfg names the operating system outside $SEAM:" >&2
    echo "$leak" | sed 's/^/    /' >&2
    status=1
fi

if [ "$status" -ne 0 ]; then
    echo "ask the seam for the capability instead (docs/PORTABILITY.md)" >&2
fi
exit "$status"
