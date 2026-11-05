#!/bin/sh
# devrun.sh — starts the tortty Rust development instance (PID file).
#
# Why a PID file and not `pkill -f tortty`: the caller's command line
# contains the pattern, so `pkill -f` kills the shell itself. `pkill -x` on the
# old name (`bzmux`) killed the installed Python app instead of the dev binary,
# because both answered to the same name; it has already happened. Here the PID
# is the `$!` of the process itself, and devkill checks `/proc/<pid>/exe` before
# killing.
#
# Usage: tools/devrun.sh [tortty args...]
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
# the debug binary unless there is none: nothing in the socket/xdotool probes
# and in manual poking depends on the profile, and building release costs ~50 s
BIN="$HERE/target/debug/tortty"
[ -x "$BIN" ] || BIN="$HERE/target/release/tortty"
[ -x "$BIN" ] || { echo "build first: cargo build" >&2; exit 1; }

PIDFILE=${TORTTY_RS_PIDFILE:-/tmp/tortty-rs.pid}
LOGFILE=${TORTTY_RS_LOG:-/tmp/tortty-rs.log}

if [ -f "$PIDFILE" ]; then
    old=$(cat "$PIDFILE")
    if [ -n "$old" ] && [ -d "/proc/$old" ]; then
        echo "already running: pid $old (use tools/devkill.sh)" >&2
        exit 1
    fi
    rm -f "$PIDFILE"
fi

nohup "$BIN" "$@" >/dev/null 2>"$LOGFILE" &
echo $! > "$PIDFILE"
echo "pid $(cat "$PIDFILE") — log $LOGFILE"
