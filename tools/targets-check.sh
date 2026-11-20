#!/bin/sh
# targets-check.sh — the package name has two authors, and they must agree.
#
# tools/targets.sh names the tarball when it is built (keyed by rustc target);
# install.sh rebuilds the same name to fetch it (keyed by `uname -m`). The two are
# copies on purpose: install.sh is run from a pipe, with no checkout to source
# anything from. A copy that drifts does not fail the build — it fails as a 404 on
# the release URL at install time, which is exactly how install.sh's dead
# `--from-release` path survived months of green checks. So the two tables are
# compared here, machine by machine, and both directions of a missing row are tested.
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
cd "$HERE"

# `die` in install.sh exits; here it has to come back so that a missing row is a
# failed check instead of a dead shell
die() {
    printf '%s\n' "$*" >&2
    return 1
}

. ./tools/targets.sh

# install.sh cannot be sourced (in a pipe `$0` is `bash`, and sourcing would run
# the installer): take just the table out of it, renamed so the two can coexist
fn=$(sed -n '/^pkg_suffix() {/,/^}/p' install.sh)
[ -n "$fn" ] || {
    echo "install.sh: no pkg_suffix() to compare (renamed?)" >&2
    exit 1
}
eval "$(printf '%s\n' "$fn" | sed 's/^pkg_suffix() {/installed_suffix() {/')"

fail=0
check() {  # $1 = rustc target, $2 = what `uname -m` answers
    built=$(pkg_suffix "$1") || {
        echo "targets.sh: no package name for $1" >&2
        fail=1
        return 0
    }
    installed=$(installed_suffix "$2") || {
        echo "install.sh: no package name for $2" >&2
        fail=1
        return 0
    }
    if [ "$built" != "$installed" ]; then
        printf 'drift: %-28s builds %s, installs as %s\n' "$1" "$built" "$installed" >&2
        fail=1
        return 0
    fi
    printf '%-28s -> %s\n' "$1" "$built"
}

check x86_64-unknown-linux-gnu x86_64
check aarch64-unknown-linux-gnu aarch64

# a row that only one side knows would be a silent 404 for that machine
if name=$(pkg_suffix aarch64-apple-darwin 2>/dev/null); then
    echo "targets.sh: aarch64-apple-darwin has no release yet, but names one ($name)" >&2
    fail=1
fi
if name=$(installed_suffix armv7l 2>/dev/null); then
    echo "install.sh: armv7l has no release, but names one ($name)" >&2
    fail=1
fi

exit "$fail"
