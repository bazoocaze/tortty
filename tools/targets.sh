#!/bin/sh
# targets.sh — the one table that turns a machine into the name on the package.
#
# The name on the tarball is read by whoever installs, not by the compiler. A rustc
# target puts two words of compiler vocabulary in front of them: `unknown`, which is
# the vendor slot of a machine that has no vendor to declare, and `gnu`, which is the
# C library. Neither answers the question the reader has ("does this run here?").
# So the name says the system and the machine, and nothing else: `linux-x86_64`.
#
# There are two readers and they must agree: tools/build-release.sh writes the name
# from a rustc target, install.sh rebuilds the same name from `uname -m` to fetch it.
# install.sh runs from a pipe with no checkout, so it cannot source this file — it
# carries the same table, and tools/targets-check.sh fails when the two drift apart.
#
# A second system adds a row here and the matching row in install.sh. The rustc
# target stays the key because it is what `cargo build --target` takes, and `--target`
# is the one place the compiler's spelling is still required.

# $1 = rustc target (`cargo build --target` value); prints the package suffix
pkg_suffix() {
    case "$1" in
        x86_64-unknown-linux-gnu)  printf 'linux-x86_64' ;;
        aarch64-unknown-linux-gnu) printf 'linux-aarch64' ;;
        *) return 1 ;;
    esac
}
