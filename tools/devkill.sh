#!/bin/sh
# devkill.sh — mata a instância de dev deste repositório (e as sobras).
#
# O critério é `/proc/<pid>/exe` apontar para `target/{release,debug}/bzmux`
# **deste** checkout. Isso é específico o bastante para não tocar no `bzmux` do
# pipx (app Python do usuário) nem em outro checkout. Nunca use
# `pkill -f bzmux` (casa com o shell que chamou) nem `pkill -x bzmux` (casa com
# o console script do pipx).
#
# Também varre as sobras: o PID file só guarda a última instância, e ficar com
# duas rodando no mesmo socket dá depuração fantasma (a que responde não é a
# que você acabou de compilar).
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
PIDFILE=${BZMUX_RS_PIDFILE:-/tmp/bzmux-rs.pid}

eh_dev() {
    exe=$(readlink "/proc/$1/exe" 2>/dev/null || true)
    # `cargo build` troca o arquivo: o processo antigo aparece como
    # `.../bzmux (deleted)` — é justamente a sobra que interessa matar
    exe=${exe% (deleted)}
    case "$exe" in
        "$HERE"/target/release/bzmux|"$HERE"/target/debug/bzmux) return 0 ;;
        *) return 1 ;;
    esac
}

matar() {
    pid=$1
    kill "$pid" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        [ -d "/proc/$pid" ] || break
        sleep 0.1
    done
    [ -d "/proc/$pid" ] && kill -9 "$pid" 2>/dev/null || true
}

morto=0

# 1) o PID registrado (confere antes: PID pode ter sido reciclado)
if [ -f "$PIDFILE" ]; then
    pid=$(cat "$PIDFILE" 2>/dev/null || true)
    if [ -n "${pid:-}" ] && [ -d "/proc/$pid" ]; then
        if eh_dev "$pid"; then
            matar "$pid"
            echo "morto: pid $pid"
            morto=1
        else
            echo "recusando matar pid $pid: exe=$(readlink "/proc/$pid/exe" 2>/dev/null)" >&2
        fi
    fi
    rm -f "$PIDFILE"
fi

# 2) sobras deste checkout
for d in /proc/[0-9]*; do
    pid=${d#/proc/}
    [ "$pid" = "$$" ] && continue
    if eh_dev "$pid"; then
        matar "$pid"
        echo "morto (sobra): pid $pid"
        morto=1
    fi
done

[ "$morto" = 1 ] || echo "nenhuma instância de dev rodando"
