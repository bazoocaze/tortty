#!/bin/sh
# devkill.sh — kills this repository's dev instance (and leftovers).
#
# The criterion is `/proc/<pid>/exe` pointing at `target/{release,debug}/tortty`
# of **this** checkout. That is specific enough not to touch another checkout or
# another build. Never use `pkill -f tortty` (it matches the
# shell that called it); `pkill -x` is only as safe as the name being unique,
# and names change — the exe check does not.
#
# It also sweeps the leftovers: the PID file only keeps the last instance, and having
# two running on the same socket gives phantom debugging (the one that answers is not
# the one you just built).
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
PIDFILE=${TORTTY_RS_PIDFILE:-/tmp/tortty-rs.pid}

is_dev() {
    exe=$(readlink "/proc/$1/exe" 2>/dev/null || true)
    # `cargo build` replaces the file: the old process shows up as
    # `.../tortty (deleted)` — that is exactly the leftover worth killing
    exe=${exe% (deleted)}
    case "$exe" in
        "$HERE"/target/release/tortty|"$HERE"/target/debug/tortty) return 0 ;;
        *) return 1 ;;
    esac
}

kill_pid() {
    pid=$1
    kill "$pid" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        [ -d "/proc/$pid" ] || break
        sleep 0.1
    done
    [ -d "/proc/$pid" ] && kill -9 "$pid" 2>/dev/null || true
}

dead=0

# 1) the registered PID (checked first: the PID may have been recycled)
if [ -f "$PIDFILE" ]; then
    pid=$(cat "$PIDFILE" 2>/dev/null || true)
    if [ -n "${pid:-}" ] && [ -d "/proc/$pid" ]; then
        if is_dev "$pid"; then
            kill_pid "$pid"
            echo "killed: pid $pid"
            dead=1
        else
            echo "refusing to kill pid $pid: exe=$(readlink "/proc/$pid/exe" 2>/dev/null)" >&2
        fi
    fi
    rm -f "$PIDFILE"
fi

# 2) leftovers from this checkout
for d in /proc/[0-9]*; do
    pid=${d#/proc/}
    [ "$pid" = "$$" ] && continue
    if is_dev "$pid"; then
        kill_pid "$pid"
        echo "killed (leftover): pid $pid"
        dead=1
    fi
done

[ "$dead" = 1 ] || echo "no dev instance running"
