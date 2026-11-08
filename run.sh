#!/bin/sh
# run.sh — runs this checkout, not the installed `tortty`.
#
# `tortty` in PATH is whatever `install.sh` put there, which is older than the
# tree being edited. This builds the workspace binary and execs it with the
# arguments, so what appears on screen is what is in the tree. Debug is the
# default for the same reason it is the default of the probes: seconds to build
# against ~50 s of release, and nothing outside speed depends on the profile.
#
# A background instance with a PID file and the debug socket is tools/devrun.sh;
# that one manages a lifecycle, this one replaces the foreground process.
#
# Usage: ./run.sh [tortty args...]
#        PROFILE=release ./run.sh    (a measurement: the build costs ~50 s)
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
PROFILE=${PROFILE:-debug}
case "$PROFILE" in
    debug) FLAGS= ;;
    release) FLAGS=--release ;;
    *) echo "run.sh: PROFILE is debug or release, not '$PROFILE'" >&2; exit 2 ;;
esac

# no `--quiet`: a warning in the tree has to show up here, not only in the hook
cargo build -p tortty --manifest-path "$HERE/Cargo.toml" $FLAGS
exec "$HERE/target/$PROFILE/tortty" "$@"
