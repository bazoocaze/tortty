#!/bin/sh
# devkill.sh — mata SÓ a instância de dev registrada no PID file.
#
# Confere `/proc/<pid>/exe` antes: se o PID foi reciclado ou o processo é o
# `bzmux` do pipx (app Python do usuário), não mata nada. Nunca use
# `pkill -f bzmux` (casa com o shell que chamou) nem `pkill -x bzmux` (casa com
# o console script do pipx).
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
PIDFILE=${BZMUX_RS_PIDFILE:-/tmp/bzmux-rs.pid}

[ -f "$PIDFILE" ] || { echo "sem pid file em $PIDFILE" >&2; exit 1; }
pid=$(cat "$PIDFILE")
[ -n "$pid" ] && [ -d "/proc/$pid" ] || { echo "pid $pid não existe" >&2; rm -f "$PIDFILE"; exit 0; }

exe=$(readlink "/proc/$pid/exe" 2>/dev/null || true)
case "$exe" in
    "$HERE"/target/release/bzmux|"$HERE"/target/debug/bzmux)
        ;;
    *)
        echo "recusando matar pid $pid: exe=$exe (não é o binário de dev)" >&2
        rm -f "$PIDFILE"
        exit 1
        ;;
esac

kill "$pid" 2>/dev/null || true
for _ in 1 2 3 4 5 6 7 8 9 10; do
    [ -d "/proc/$pid" ] || break
    sleep 0.1
done
[ -d "/proc/$pid" ] && kill -9 "$pid" 2>/dev/null || true
rm -f "$PIDFILE"
echo "morto: pid $pid"
