#!/bin/sh
# devrun.sh — starts the bzmux Rust development instance (PID file).
#
# Why a PID file and not `pkill -f bzmux`: the caller's command line
# contains the pattern, so `pkill -f` kills the shell itself; and `pkill -x bzmux`
# kills the pipx `bzmux` (the installed Python app), because the console
# script name is the same as the dev binary. It has already happened. Here the PID is the `$!`
# of the process itself, and devkill checks `/proc/<pid>/exe` before killing.
#
# Usage: tools/devrun.sh [bzmux args...]
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
BIN="$HERE/target/release/bzmux"
[ -x "$BIN" ] || BIN="$HERE/target/debug/bzmux"
[ -x "$BIN" ] || { echo "build first: cargo build" >&2; exit 1; }

PIDFILE=${BZMUX_RS_PIDFILE:-/tmp/bzmux-rs.pid}
LOGFILE=${BZMUX_RS_LOG:-/tmp/bzmux-rs.log}

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
