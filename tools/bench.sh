#!/bin/sh
# bench.sh — escada de medição do doom-fire: Rust x Python, nas duas grades.
#
# Mede o MESMO gerador (`tools/doomfire.py`, byte a byte igual ao do tortty-python)
# rodando dentro de cada terminal, na mesma grade travada em CÉLULAS pelo verbo
# `resize` do socket (`tools/benchrun.py`). Número de outra condição não entra:
# se o grid não bater, o benchrun sai com código 3 e esta tabela falha.
#
# Uso:
#   tools/bench.sh                    # 3 tiros por linha, 1200 frames
#   tools/bench.sh --frames 300 --shots 1
#   tools/bench.sh --so-rust          # sem a referência Python
#
# O app Python sai de $PYBZMUZ (default: o repo irmão em ~).
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
FRAMES=1200
SHOTS=3
COLORS="61x18 70x21"
ONLY_RUST=0
PYBZMUZ=${PYBZMUZ:-$HOME/data/desenv/gh/tortty-python}

while [ $# -gt 0 ]; do
    case "$1" in
        --frames) FRAMES=$2; shift 2 ;;
        --shots) SHOTS=$2; shift 2 ;;
        --grades) COLORS=$2; shift 2 ;;
        --only-rust) ONLY_RUST=1; shift ;;
        -h|--help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "opção desconhecida: $1" >&2; exit 2 ;;
    esac
done

[ -x "$HERE/target/release/bzmux" ] || { echo "compila antes: cargo build --release" >&2; exit 1; }

GOV=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo '?')
echo "# frames=$FRAMES tiros=$SHOTS governor=$GOV display=${DISPLAY:-?}"
echo "# app | grade | fps | MB/s | draw | parse"

run() {
    app=$1; grade=$2; rotulo=$3
    cols=${grade%x*}; rows=${grade#*x}
    i=1
    while [ "$i" -le "$SHOTS" ]; do
        if [ -n "$app" ]; then
            out=$(python3 "$HERE/tools/benchrun.py" --app "$app" \
                --frames "$FRAMES" --cols "$cols" --rows "$rows" 2>&1) || {
                echo "$rotulo | $grade | FALHOU: $(echo "$out" | tail -1)"; i=$((i+1)); continue; }
        else
            out=$(python3 "$HERE/tools/benchrun.py" \
                --frames "$FRAMES" --cols "$cols" --rows "$rows" 2>&1) || {
                echo "$rotulo | $grade | FALHOU: $(echo "$out" | tail -1)"; i=$((i+1)); continue; }
        fi
        fps=$(echo "$out" | awk '/^fps /{print $2}')
        mbs=$(echo "$out" | awk '/^mbs /{print $2" "$3}')
        draw=$(echo "$out" | awk '/^draw /{print $2" "$3}')
        if echo "$out" | grep -q '^parse '; then
            parse=$(echo "$out" | awk '/^parse /{print $2" "$3}')
        else
            parse=$(echo "$out" | awk '/^pty /{print $2" "$3}')  # app Python
        fi
        echo "$rotulo | $grade | $fps | $mbs | $draw | $parse"
        i=$((i+1))
    done
}

for g in $COLORS; do
    run "" "$g" "rust"
done
if [ "$SO_RUST" = 0 ]; then
    [ -x "$PYBZMUZ/run.sh" ] || { echo "# sem $PYBZMUZ/run.sh: pulando o Python" >&2; exit 0; }
    for g in $COLORS; do
        run "$PYBZMUZ/run.sh" "$g" "python"
    done
fi
