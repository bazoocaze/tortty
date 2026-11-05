#!/bin/sh
# sandbox + cargo: o fluxo Rust precisa gravar em ~/.cargo/registry.
#
# Rode NO HOST (este script chama o `sandbox`; não precisa estar dentro dele).
#   sh tools/sandbox-cargo-registry-test.sh          # testa
#   sh tools/sandbox-cargo-registry-test.sh -v       # mostra os argumentos do bwrap
#   sh tools/sandbox-cargo-registry-test.sh --clean  # apaga o scratch
#
# O scratch fica ao lado do script (persistente), não em /tmp: o `target/` do
# cargo é caro de refazer e /tmp costuma ser tmpfs. Trocar com SCRATCH_DIR=...
#
# Saida: 0 = sandbox ok para cargo   1 = falta correção (o motivo é impresso)

SANDBOX=${SANDBOX:-sandbox}
HERE=$(cd "$(dirname "$0")" && pwd)
SCRATCH=${SCRATCH_DIR:-$HERE/.sandbox-cargo-scratch}
VERB=""
case "$1" in
    -v) VERB="-v" ;;
    --clean) rm -rf "$SCRATCH" && echo "scratch apagado: $SCRATCH" && exit 0 ;;
esac

falhas=0
ok() { printf '  ok      %s\n' "$1"; }
not() { printf '  FAILURE   %s\n' "$1"; [ -n "$2" ] && printf '          %s\n' "$2"; falhas=$((falhas + 1)); }
# o cargo imprime um warning de cache e só depois o erro fatal: pega o fatal
reason() {
    printf '%s\n' "$1" | grep -m1 'error: failed to open' ||
        printf '%s\n' "$1" | grep -m1 'Read-only file system'
}

# o teste só vale com crate fora do cache local: se estiver lá, "sem grant"
# passa de graça e a sonda mente
crate_fora_do_cache() {
    for c in tinyjson humansize textwrap difference heck arrayvec bit-set colored; do
        if ! ls "$HOME"/.cargo/registry/cache/*/"$c"-*.crate >/dev/null 2>&1; then
            printf '%s' "$c"
            return 0
        fi
    done
    printf 'rustybuzz' # tudo em cache: cai no comportamento antigo
}
CRATE=$(crate_fora_do_cache)

echo "== A. montagem do \$HOME no host (a causa quando o --bind não pega)"
mount | grep -E " on $HOME " || echo "  (nenhuma montagem em $HOME: não é esse o caso)"

# crate escrito à mão, com [workspace] vazio: `cargo new` dentro do repo
# adicionaria o membro no Cargo.toml da raiz e contaminaria o workspace
crate() {
    mkdir -p "$SCRATCH/$1/src"
    printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n' "$1" \
        >"$SCRATCH/$1/Cargo.toml"
    echo 'fn main() {}' >"$SCRATCH/$1/src/main.rs"
}

echo "== B. sem grant: reproduz o defeito"
rm -rf "$SCRATCH"
mkdir -p "$SCRATCH" || { echo "sem escrita em $SCRATCH"; exit 1; }
crate app
out=$($SANDBOX $VERB -w "$SCRATCH" -- sh -c \
    "cd $SCRATCH/app && cargo add -q $CRATE && cargo build" 2>&1)
if [ $? -eq 0 ]; then
    printf '  notice   %s\n' "build passou SEM grant: cache quente ou grant global (reprodução inconclusiva)"
else
    ok "sem grant falha, como esperado ($(motivo "$out" | cut -c1-60)...)"
fi

echo "== C. com -w ~/.cargo/registry: o mínimo que o fluxo Rust pede"
if $SANDBOX -w "$SCRATCH" -w ~/.cargo/registry -- \
    sh -c 'touch ~/.cargo/registry/.probe && rm -f ~/.cargo/registry/.probe' 2>/dev/null; then
    ok "~/.cargo/registry gravável dentro do sandbox com -w"
else
    not "~/.cargo/registry continua somente leitura com -w" \
        "se \$HOME está montado ro no host, o bind herda ro: nenhuma política do sandbox resolve"
fi

echo "== D. o lock do cargo (~/.cargo/.package-cache) fica FORA de registry/"
if $SANDBOX -w "$SCRATCH" -w ~/.cargo/registry -- \
    sh -c 'touch ~/.cargo/.package-cache.probe && rm -f ~/.cargo/.package-cache.probe' 2>/dev/null; then
    ok "~/.cargo/.package-cache gravável"
else
    printf '  notice   %s\n' "~/.cargo/.package-cache não gravável (lock do cargo; só importa se o build reclamar)"
fi

echo "== E. o bind é emitido pelo sandbox? (política ok, montagem não)"
if printf '%s' "$($SANDBOX -v -w ~/.cargo/registry -- true 2>&1)" | grep -q "$HOME/.cargo/registry"; then
    ok "sandbox emite --bind $HOME/.cargo/registry"
else
    not "sandbox não emite o bind de ~/.cargo/registry" "verifique o cap write(\"~/.cargo/registry\")"
fi

echo "== F. build de verdade com o grant (o teste que tem de passar)"
crate app2
out=$($SANDBOX $VERB -w "$SCRATCH" -w ~/.cargo/registry -- sh -c \
    "cd $SCRATCH/app2 && cargo add -q $CRATE && cargo build" 2>&1)
if [ $? -eq 0 ]; then
    ok "cargo build com dependência nova passou"
else
    not "cargo build com dependência nova falhou" "$(motivo "$out")"
    # o scratch fica para inspeção: Cargo.toml, alvo de build, etc.
    echo "          scratch preservado em $SCRATCH"
fi

echo "== G. perfil @dev (onde o fix do sandbox costuma morar)"
crate app3
out=$($SANDBOX $VERB @dev -w "$SCRATCH" -- sh -c \
    "cd $SCRATCH/app3 && cargo add -q $CRATE && cargo build" 2>&1)
if [ $? -eq 0 ]; then
    ok "cargo build passou com @dev (perfil resolvido)"
else
    not "cargo build falhou com @dev" "$(motivo "$out")"
fi

echo
if [ "$falhas" -eq 0 ]; then
    echo "SANDBOX OK PARA CARGO"
    echo "sugestão de cap no perfil:  write(\"~/.cargo/registry\")"
    exit 0
fi
echo "FALTA CORREÇÃO ($falhas ponto(s))"
exit 1
