#!/bin/sh
# check.sh — a régua de qualidade: formatação, lints e testes.
#
# É o mesmo comando no pre-commit e no CI, de propósito: se o hook e o CI
# discordam, o hook não serve para nada.
#
#   tools/check.sh            # fmt --check, clippy, testes, build release
#   tools/check.sh --fast     # sem o build release (o que o pre-commit roda)
#   tools/check.sh --fix      # aplica fmt e clippy --fix antes de checar
#
# Sai com 0 só se tudo passa. Nada aqui roda GUI: `mc`/`vim` continuam manuais
# (docs/DEBUG.md).
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
cd "$HERE"

FAST=0
FIX=0
for arg in "$@"; do
    case "$arg" in
        --fast) FAST=1 ;;
        --fix) FIX=1 ;;
        -h|--help) sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "opção desconhecida: $arg" >&2; exit 2 ;;
    esac
done

falhou=0
passo() {
    nome=$1
    shift
    printf '%-22s' "$nome"
    if saida=$("$@" 2>&1); then
        echo "ok"
    else
        echo "FALHOU"
        echo "$saida" | tail -30 | sed 's/^/    /'
        falhou=1
    fi
}

if [ "$FIX" = 1 ]; then
    cargo fmt --all
    # o `--fix` do clippy recusa árvore suja por padrão; aqui é intencional
    cargo clippy --fix --allow-dirty --allow-staged --all-targets \
        --workspace --release >/dev/null 2>&1 || true
fi

passo "cargo fmt --check" cargo fmt --all -- --check
passo "clippy -D warnings" cargo clippy --workspace --all-targets --release -- -D warnings
passo "cargo test" cargo test --workspace --quiet

if [ "$FAST" = 0 ]; then
    passo "build --release" cargo build --release --workspace
    # parsebench é o benchmark sem display: se ele quebra, o número do
    # docs/BENCHMARKS.md morre junto
    passo "parsebench" ./target/release/parsebench --frames 100 --cols 40 --rows 12
fi

if [ "$falhou" = 0 ]; then
    echo "tudo verde"
else
    echo "tem coisa vermelha acima" >&2
fi
exit "$falhou"
