#!/bin/sh
# bench.sh — doom-fire measurement ladder: Rust x Python, on both grids.
#
# Measures the SAME generator (`tools/doomfire.py`, byte for byte the one from
# tortty-python) running inside each terminal, on the same grid locked in CELLS by the
# socket `resize` verb (`tools/benchrun.py`). A number from another condition does
# not go in: if the grid does not match, benchrun exits with code 3 and this table fails.
#
# Usage:
#   tools/bench.sh                    # 3 shots per row, 1200 frames
#   tools/bench.sh --frames 300 --shots 1
#   tools/bench.sh --only-rust        # without the Python reference
#
# The Python app comes from $PYBZMUZ (default: the sibling repo in ~).
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
FRAMES=1200
SHOTS=3
GRIDS="61x18 70x21"
ONLY_RUST=0
PYBZMUZ=${PYBZMUZ:-$HOME/data/desenv/gh/tortty-python}

while [ $# -gt 0 ]; do
    case "$1" in
        --frames) FRAMES=$2; shift 2 ;;
        --shots) SHOTS=$2; shift 2 ;;
        --grids) GRIDS=$2; shift 2 ;;
        --only-rust) ONLY_RUST=1; shift ;;
        -h|--help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

[ -x "$HERE/target/release/bzmux" ] || { echo "build first: cargo build --release" >&2; exit 1; }

GOV=$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo '?')
echo "# frames=$FRAMES shots=$SHOTS governor=$GOV display=${DISPLAY:-?}"
echo "# app | grid | fps | MB/s | draw | parse"

run() {
    app=$1; grid=$2; label=$3
    cols=${grid%x*}; rows=${grid#*x}
    i=1
    while [ "$i" -le "$SHOTS" ]; do
        if [ -n "$app" ]; then
            out=$(python3 "$HERE/tools/benchrun.py" --app "$app" \
                --frames "$FRAMES" --cols "$cols" --rows "$rows" 2>&1) || {
                echo "$label | $grid | FAILED: $(echo "$out" | tail -1)"; i=$((i+1)); continue; }
        else
            out=$(python3 "$HERE/tools/benchrun.py" \
                --frames "$FRAMES" --cols "$cols" --rows "$rows" 2>&1) || {
                echo "$label | $grid | FAILED: $(echo "$out" | tail -1)"; i=$((i+1)); continue; }
        fi
        fps=$(echo "$out" | awk '/^fps /{print $2}')
        mbs=$(echo "$out" | awk '/^mbs /{print $2" "$3}')
        draw=$(echo "$out" | awk '/^draw /{print $2" "$3}')
        if echo "$out" | grep -q '^parse '; then
            parse=$(echo "$out" | awk '/^parse /{print $2" "$3}')
        else
            parse=$(echo "$out" | awk '/^pty /{print $2" "$3}')  # Python app
        fi
        echo "$label | $grid | $fps | $mbs | $draw | $parse"
        i=$((i+1))
    done
}

for g in $GRIDS; do
    run "" "$g" "rust"
done
if [ "$ONLY_RUST" = 0 ]; then
    [ -x "$PYBZMUZ/run.sh" ] || { echo "# no $PYBZMUZ/run.sh: skipping Python" >&2; exit 0; }
    for g in $GRIDS; do
        run "$PYBZMUZ/run.sh" "$g" "python"
    done
fi
