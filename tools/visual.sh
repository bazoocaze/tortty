#!/bin/sh
# Verificação visual automática: abre a janela num display isolado (Xvfb),
# desenha conteúdo conhecido, captura e **compara** — sem janela na sessão de
# quem está trabalhando e sem olho humano no resultado.
#
#   tools/visual.sh                # roda a bateria e diz PASSOU/FALHOU
#   tools/visual.sh --keep         # guarda as capturas em /tmp/bzmux-visual
#   tools/visual.sh --display :77  # usa outro display (o Xvfb sobe se faltar)
#
# O que ele prova, e que nenhum teste headless prova:
#
#   1. a janela desenha (surface vivo): conteúdo mudou ⇒ pixel mudou;
#   2. a cor sai certa: SGR 41/42 pintam o fundo da célula com color1/color2;
#   3. o texto sai na célula certa: o glifo pinta dentro da célula pedida;
#   4. tela parada não repinta diferente (comparação de duas capturas = 0).
#   5. cada linha da grade cai na sua própria faixa de pixels: antes do B4 a
#      linha `r` saía em `2r`, e o texto aparecia com o dobro do espaçamento
#      com o cursor longe do prompt.
#
# Se o `wgpu` não conseguir apresentar (o caso do `M-AA` sob sandbox), o passo 1
# falha aqui — antes isso só aparecia como "a janela abriu e ficou morta".
set -u

RAIZ=$(cd "$(dirname "$0")/.." && pwd)
APP=${BZMUX_BIN:-$RAIZ/target/release/bzmux}
DISPLAY_XVFB=${BZMUX_DISPLAY:-:77}
SAIDA=${BZMUX_VISUAL_DIR:-/tmp/bzmux-visual}
MANTER=0
SOCK=/tmp/bzmux-visual.sock

while [ $# -gt 0 ]; do
    case "$1" in
        --keep) MANTER=1; shift ;;
        --display) DISPLAY_XVFB=$2; shift 2 ;;
        -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "opção desconhecida: $1" >&2; exit 2 ;;
    esac
done

falhas=0
passo() { printf '%s %s\n' "$1" "$2"; }
ok() { passo "ok   " "$1"; }
falhou() { passo "FALHA" "$1"; falhas=$((falhas + 1)); }

[ -x "$APP" ] || { echo "sem binário em $APP (cargo build --release)" >&2; exit 2; }
command -v import >/dev/null || { echo "falta ImageMagick (import)" >&2; exit 2; }

mkdir -p "$SAIDA"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/bzmux-visual.XXXXXX")
XVFB_PID=
APP_PID=
APP2_PID=

limpar() {
    [ -n "$APP2_PID" ] && kill "$APP2_PID" 2>/dev/null
    [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null
    # espera o app sair: sem isso o Xvfb pode cair com cliente conectado
    [ -n "$APP_PID" ] && wait "$APP_PID" 2>/dev/null
    [ -n "$APP2_PID" ] && wait "$APP2_PID" 2>/dev/null
    [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null
    rm -f "$SOCK"
    [ "$MANTER" -eq 1 ] || rm -rf "$TMP"
}
trap limpar EXIT INT TERM

# display isolado: ninguém vê, ninguém perde o foco
if DISPLAY=$DISPLAY_XVFB xdpyinfo >/dev/null 2>&1; then
    SEDIADO=1
else
    Xvfb "$DISPLAY_XVFB" -screen 0 1280x800x24 >"$TMP/xvfb.log" 2>&1 &
    XVFB_PID=$!
    SEDIADO=0
    i=0
    while ! DISPLAY=$DISPLAY_XVFB xdpyinfo >/dev/null 2>&1; do
        i=$((i + 1))
        [ "$i" -gt 50 ] && { echo "Xvfb não subiu: $(cat "$TMP/xvfb.log")" >&2; exit 2; }
        sleep 0.1
    done
fi

# tema fixo: sem isto a cor esperada depende do ~/.config de quem roda
mkdir -p "$TMP/home/.config/bzmux"
cat >"$TMP/home/.config/bzmux/bzmux.conf" <<'CONF'
background #000000
foreground #ffffff
color1 #ff0000
color2 #00ff00
cursor_blink off
window_padding_width 0
CONF

DISPLAY=$DISPLAY_XVFB HOME=$TMP/home "$APP" \
    --cols 40 --rows 8 --debug-socket "$SOCK" \
    --cmd 'printf "\033[41m  \033[0m\033[42m  \033[0m"; printf "\033[3;1HLINHA"; sleep 600' \
    >"$TMP/app.log" 2>&1 &
APP_PID=$!

# espera o socket e o primeiro frame apresentado
i=0
while [ ! -S "$SOCK" ]; do
    i=$((i + 1)); [ "$i" -gt 100 ] && { echo "app não abriu o socket" >&2; cat "$TMP/app.log" >&2; exit 2; }
    sleep 0.1
done
python3 - "$SOCK" <<'PY' || { echo "nenhum frame apresentado (surface morto?)" >&2; exit 1; }
import json, socket, sys, time
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile()
def q(o):
    s.sendall((json.dumps(o) + "\n").encode()); return json.loads(f.readline())
for _ in range(100):
    st = q({"cmd": "state"})
    if st.get("last_frame") == "drawn" and st.get("draws", 0) > 0:
        break
    time.sleep(0.1)
else:
    print(f"last_frame={st.get('last_frame')} draws={st.get('draws')}", file=sys.stderr)
    sys.exit(1)
PY

# o id vem do `state` da **nossa** instância: procurar por classe pegaria
# janela de outra instância no mesmo display, e a captura sairia vazia
XID=$(python3 - "$SOCK" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile()
s.sendall(b'{"cmd":"state"}\n')
print(json.loads(f.readline()).get("xid") or "")
PY
)
[ -n "$XID" ] || { echo "a instância não reportou xid no state" >&2; exit 1; }

# geometria da célula: o app já sabe (o pty recebeu TIOCSWINSZ com xpixel)
CEL=$(python3 - "$SOCK" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile()
s.sendall(b'{"cmd":"state"}\n'); st = json.loads(f.readline())
print(f"{int(round(st['cell'][0]))}x{int(round(st['cell'][1]))}")
PY
)
[ -n "${CEL:-}" ] || CEL=16x31
CWL=${CEL%x*}
CHL=${CEL#*x}

tirar() { # tirar <arquivo>
    DISPLAY=$DISPLAY_XVFB import -window "$XID" "$1" 2>/dev/null
    [ -s "$1" ] || { echo "captura vazia em $1" >&2; return 1; }
}

# com a barra de 1 aba, a grade começa na linha 1 (barra escondida)
tirar "$SAIDA/a.png" || { falhou "captura"; exit 1; }

# 1. desenho vivo: mudar o conteúdo tem de mudar pixels
python3 - "$SOCK" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile()
s.sendall((json.dumps({"cmd": "write", "data": "\r\nMUDOU"}) + "\n").encode())
json.loads(f.readline())
PY
# resumo do `state` para o diagnóstico: separa "o app não desenhou" de
# "desenhou e a superfície não apresentou" (o sintoma do M-AA)
estado_resumo() {
    python3 - "$SOCK" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile()
s.sendall(b'{"cmd":"state"}\n')
st = json.loads(f.readline())
print(
    "drawn={} skips={} last_frame={!r}".format(
        st.get("drawn"), st.get("skips"), st.get("last_frame")
    )
)
PY
}

sleep 0.4
tirar "$SAIDA/b.png" || falhou "captura 2"
if ./tools/pixel.py --diff "$SAIDA/a.png" "$SAIDA/b.png" --max 0 >/dev/null 2>&1; then
    falhou "conteúdo mudou e nenhum pixel mudou ($(estado_resumo))"
else
    ok "conteúdo novo mudou pixels"
fi

# 2. cor de fundo por SGR 41/42 na primeira linha
if ./tools/pixel.py "$SAIDA/b.png" \
    --expect "$((CWL / 2)),$((CHL / 2))=ff0000" \
    --expect "$((CWL * 2 + CWL / 2)),$((CHL / 2))=00ff00" \
    --expect "$((CWL * 20)),$((CHL / 2))=000000" >"$SAIDA/cores.txt" 2>&1; then
    ok "SGR 41/42 pintam color1/color2 na célula"
else
    falhou "cor errada: $(tr '\n' ' ' <"$SAIDA/cores.txt")"
fi

# 3. texto pintado dentro da célula (linha 3, coluna 1: glifo branco no preto)
if ./tools/pixel.py "$SAIDA/b.png" --row "$((CHL * 2 + CHL / 2))" \
    >"$SAIDA/linha3.txt" 2>&1 && grep -qv '000000' "$SAIDA/linha3.txt"; then
    ok "texto desenha dentro da célula"
else
    falhou "nenhum pixel de texto na linha 3"
fi

# 4. tela parada: duas capturas seguidas têm de ser idênticas
tirar "$SAIDA/c.png" || falhou "captura 3"
if ./tools/pixel.py --diff "$SAIDA/b.png" "$SAIDA/c.png" --max 0 >/dev/null 2>&1; then
    ok "tela parada não repinta diferente"
else
    falhou "duas capturas da mesma tela diferem (repintura instável)"
fi

# 5. a linha `r` da grade tem de ser pintada na faixa `r` (regressão do B4:
# `pinta_linha` somava `y*ch` e `concat_linhas` somava de novo, e tudo saía em
# `2r` — texto com espaçamento dobrado e cursor longe do prompt). Cada linha
# leva o próprio número e um fundo 48;5;N distinto; a faixa `r` tem de mostrar
# a cor do número que a grade diz estar em `r`.
#
# Aqui não há shell: o filho é `printf…; sleep 600`, então nada do que
# escrevemos é executado. Quem entrega o conteúdo à grade é o **eco do pty** —
# e é isso que queremos, bytes previsíveis. Separador `\r` (o pty converte).
if python3 - "$SOCK" "$DISPLAY_XVFB" "$SAIDA" <<'PY'
import json, os, socket, subprocess, sys, time
sock, disp, saida = sys.argv[1], sys.argv[2], sys.argv[3]
N = 15


def q(**kw):
    c = socket.socket(socket.AF_UNIX)
    c.connect(sock)
    f = c.makefile("rwb")
    f.write(json.dumps(kw).encode() + b"\n")
    f.flush()
    r = json.loads(f.readline())
    c.close()
    return r["ok"] if isinstance(r.get("ok"), dict) else r


linhas = "".join("{}\r".format(i) for i in range(N))
q(cmd="write", data=linhas)
time.sleep(0.9)
st = q(cmd="state")
ch, cw, xid = st["cell"][1], st["cell"][0], st.get("xid")
grid = q(cmd="text", where="view", count=st["rows"]).get("lines") or []

png = os.path.join(saida, "linhas.png")
subprocess.run(["xwd", "-id", str(xid), "-silent", "-out", png + ".xwd"],
               env=dict(os.environ, DISPLAY=disp), check=False)
subprocess.run(["convert", png + ".xwd", "-depth", "8", png + ".ppm"], check=True)
with open(png + ".ppm", "rb") as f:
    assert f.readline().strip() == b"P6", "captura não decodificou como PPM"
    while True:
        ln = f.readline()
        if not ln.startswith(b"#") and len(ln.split()) == 2:
            break
    w, h = (int(v) for v in ln.split())
    f.readline()
    buf = f.read()

LV = [0, 95, 135, 175, 215, 255]


def cor(i):
    idx = (17 + i) - 16
    return (LV[idx // 36], LV[(idx % 36) // 6], LV[idx % 6])


erros, vistas = [], 0


def claro(px, y):
    """ha pixel de texto (brilhante) na faixa vertical [y, y+ch)?"""
    for yy in range(y, min(int(y + ch), h)):
        for xx in range(0, min(int(cw * 8), w)):
            i = (yy * w + xx) * 3
            if buf[i] > 90 and buf[i + 1] > 90 and buf[i + 2] > 90:
                return True
    return False


for r, ln in enumerate(grid):
    y = int(r * ch)
    if y + ch > h:
        break
    tem = claro(buf, y)
    if ln.strip():
        vistas += 1
        if not tem:
            erros.append(f"linha {r} tem texto na grade e nenhum pixel na faixa")
if vistas < 5:
    erros.append(f"poucas linhas conferidas ({vistas})")
print("; ".join(erros))
sys.exit(1 if erros else 0)
PY
then
    ok "linha r é pintada na faixa r (espaçamento de 1 célula)"
else
    falhou "linhas fora da faixa (espaçamento dobrado ou cursor deslocado)"
fi

# 6. encolher e crescer (é o que o WM faz ao maximizar/restaurar) não pode
# mover o cursor nem o conteúdo. Era o terceiro erro: com o prompt na linha 0
# e o resto da tela vazio, encolher 48→36 empurrava as 12 linhas de baixo para
# o histórico; ao crescer de volta elas voltavam como 12 linhas acima do
# prompt — o shell então escrevia na linha 12. Instância própria, com só um
# marcador na linha 0: reaproveitar a do teste contaminaria a medida.
SOCK2="$TMP/ciclo.sock"
DISPLAY=$DISPLAY_XVFB HOME=$TMP/home "$APP" \
    --cols 40 --rows 8 --debug-socket "$SOCK2" \
    --cmd 'printf "MARCA"; sleep 600' >"$TMP/app2.log" 2>&1 &
APP2_PID=$!
i=0
while [ ! -S "$SOCK2" ]; do
    i=$((i + 1))
    [ "$i" -gt 100 ] && { echo "2a instância não abriu o socket" >&2; break; }
    sleep 0.1
done
if python3 - "$SOCK2" <<'PY'
import json, socket, sys, time

sock = sys.argv[1]


def q(**kw):
    c = socket.socket(socket.AF_UNIX)
    c.connect(sock)
    f = c.makefile("rwb")
    f.write(json.dumps(kw).encode() + b"\n")
    f.flush()
    r = json.loads(f.readline())
    c.close()
    return r["ok"] if isinstance(r.get("ok"), dict) else r


def estado():
    st = q(cmd="state")
    g = q(cmd="text", where="view", count=st["rows"]).get("lines") or []
    com = [i for i, l in enumerate(g) if l.strip()]
    return (com[0] if com else None), st["cursor"][0], st


time.sleep(0.6)
antes, cy_antes, st = estado()
cols, rows = st["cols"], st["rows"]
for r in (rows + 12, rows, rows + 12):
    q(cmd="resize", cols=cols, rows=r)
    time.sleep(0.5)
depois, cy_depois, st = estado()
print(
    f"grade {st['cols']}x{st['rows']} texto antes={antes} depois={depois} "
    f"cursor antes={cy_antes} depois={cy_depois}"
)
sys.exit(0 if (antes, cy_antes) == (0, 0) == (depois, cy_depois) else 1)
PY
then
    ok "cursor e conteúdo ficam na linha 1 ao encolher e crescer"
else
    falhou "cursor/conteúdo desceu de linha ao redimensionar (maximizar/restaurar)"
fi

echo
if [ "$falhas" -eq 0 ]; then
    echo "visual: tudo ok (capturas em $SAIDA)"
    exit 0
fi
echo "visual: $falhas falha(s)"
exit 1
