#!/bin/sh
# devrun.sh — sobe a instância de desenvolvimento do bzmux Rust (PID file).
#
# Por que PID file e não `pkill -f bzmux`: a linha de comando de quem chama
# contém o padrão, então `pkill -f` mata o próprio shell; e `pkill -x bzmux`
# mata o `bzmux` do pipx (o app Python instalado), porque o nome do console
# script é o mesmo do binário de dev. Já aconteceu. Aqui o PID é o `$!` do
# próprio processo, e o devkill confere `/proc/<pid>/exe` antes de matar.
#
# Uso: tools/devrun.sh [args do bzmux...]
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
BIN="$HERE/target/release/bzmux"
[ -x "$BIN" ] || BIN="$HERE/target/debug/bzmux"
[ -x "$BIN" ] || { echo "compila antes: cargo build" >&2; exit 1; }

PIDFILE=${BZMUX_RS_PIDFILE:-/tmp/bzmux-rs.pid}
LOGFILE=${BZMUX_RS_LOG:-/tmp/bzmux-rs.log}

if [ -f "$PIDFILE" ]; then
    old=$(cat "$PIDFILE")
    if [ -n "$old" ] && [ -d "/proc/$old" ]; then
        echo "já rodando: pid $old (use tools/devkill.sh)" >&2
        exit 1
    fi
    rm -f "$PIDFILE"
fi

nohup "$BIN" "$@" >/dev/null 2>"$LOGFILE" &
echo $! > "$PIDFILE"
echo "pid $(cat "$PIDFILE") — log $LOGFILE"
