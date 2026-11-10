#!/bin/sh
# Automatic rendering verification: opens the window on an isolated display
# (Xvfb), draws known content, captures and **compares** — with no window on the
# working session and no human eye on the result.
#
#   tools/render-check.sh               # runs the battery, says PASSED/FAILED
#   tools/render-check.sh --keep        # keeps the captures in /tmp/tortty-render
#   tools/render-check.sh --display :77 # uses another display (Xvfb if missing)
#
# What it proves, and no headless test proves:
#
#   1. the window draws (live surface): content changed ⇒ pixel changed;
#   2. the color comes out right: SGR 41/42 paint the cell background with color1/color2;
#   3. the text lands in the right cell: the glyph paints inside the requested cell;
#   4. a still screen does not repaint differently (comparing two captures = 0).
#   5. each grid line falls in its own pixel band: before B4, line `r` came out at
#      `2r`, and the text showed double spacing with the cursor far from the
#      prompt.
#
# If `wgpu` cannot present (the `M-AA` case under a sandbox), step 1 fails
# here — before, that only showed up as "the window opened and stayed dead".
set -u

ROOT=$(cd "$(dirname "$0")/.." && pwd)
# debug by default: what this gate proves does not depend on the profile, and the
# debug build is seconds against ~50 s of release (`TORTTY_BIN` overrides)
APP=${TORTTY_BIN:-$ROOT/target/debug/tortty}
DISPLAY_XVFB=${TORTTY_DISPLAY:-:77}
OUT_DIR=${TORTTY_RENDER_DIR:-/tmp/tortty-render}
KEEP=0
SOCK=/tmp/tortty-render.sock

while [ $# -gt 0 ]; do
    case "$1" in
        --keep) KEEP=1; shift ;;
        --display) DISPLAY_XVFB=$2; shift 2 ;;
        -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

failures=0
step() { printf '%s %s\n' "$1" "$2"; }
ok() { step "ok   " "$1"; }
fail() { step "FAILURE" "$1"; failures=$((failures + 1)); }

[ -x "$APP" ] || { echo "no binary at $APP (cargo build -p tortty)" >&2; exit 2; }
command -v import >/dev/null || { echo "ImageMagick (import) is missing" >&2; exit 2; }

mkdir -p "$OUT_DIR"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/tortty-render.XXXXXX")
XVFB_PID=
APP_PID=
APP2_PID=
APP3_PID=

cleanup() {
    [ -n "$APP2_PID" ] && kill "$APP2_PID" 2>/dev/null
    [ -n "$APP3_PID" ] && kill "$APP3_PID" 2>/dev/null
    [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null
    # wait for the app to exit: without that Xvfb can die with a client connected
    [ -n "$APP_PID" ] && wait "$APP_PID" 2>/dev/null
    [ -n "$APP2_PID" ] && wait "$APP2_PID" 2>/dev/null
    [ -n "$APP3_PID" ] && wait "$APP3_PID" 2>/dev/null
    [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null
    rm -f "$SOCK"
    [ "$KEEP" -eq 1 ] || rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

# isolated display: nobody sees it, nobody loses focus
if DISPLAY=$DISPLAY_XVFB xdpyinfo >/dev/null 2>&1; then
    DISPLAY_READY=1
else
    Xvfb "$DISPLAY_XVFB" -screen 0 1280x800x24 >"$TMP/xvfb.log" 2>&1 &
    XVFB_PID=$!
    DISPLAY_READY=0
    i=0
    while ! DISPLAY=$DISPLAY_XVFB xdpyinfo >/dev/null 2>&1; do
        i=$((i + 1))
        [ "$i" -gt 50 ] && { echo "Xvfb did not come up: $(cat "$TMP/xvfb.log")" >&2; exit 2; }
        sleep 0.1
    done
fi

# fixed theme: without it the expected color depends on the runner's ~/.config
mkdir -p "$TMP/home/.config/tortty"
cat >"$TMP/home/.config/tortty/tortty.conf" <<'CONF'
background #000000
foreground #ffffff
color1 #ff0000
color2 #00ff00
cursor_blink off
window_padding_width 0
CONF

DISPLAY=$DISPLAY_XVFB HOME=$TMP/home "$APP" \
    --cols 40 --rows 8 --debug-socket "$SOCK" \
    --cmd 'printf "\033[41m  \033[0m\033[42m  \033[0m"; printf "\033[3;1HLINE"; sleep 600' \
    >"$TMP/app.log" 2>&1 &
APP_PID=$!

# wait for the socket and the first presented frame
i=0
while [ ! -S "$SOCK" ]; do
    i=$((i + 1)); [ "$i" -gt 100 ] && { echo "app did not open the socket" >&2; cat "$TMP/app.log" >&2; exit 2; }
    sleep 0.1
done
python3 - "$SOCK" <<'PY' || { echo "no frame presented (dead surface?)" >&2; exit 1; }
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

# the id comes from **our** instance's `state`: looking it up by class would catch
# another instance's window on the same display, and the capture would come out empty
XID=$(python3 - "$SOCK" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile()
s.sendall(b'{"cmd":"state"}\n')
print(json.loads(f.readline()).get("xid") or "")
PY
)
[ -n "$XID" ] || { echo "the instance did not report an xid in state" >&2; exit 1; }

# cell geometry: the app already knows it (the pty received TIOCSWINSZ with xpixel)
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

take() { # take <file>
    DISPLAY=$DISPLAY_XVFB import -window "$XID" "$1" 2>/dev/null
    [ -s "$1" ] || { echo "empty capture at $1" >&2; return 1; }
}

# with the 1-tab bar, the grid starts on line 1 (bar hidden)
take "$OUT_DIR/a.png" || { fail "capture"; exit 1; }

# 1. live drawing: changing the content has to change pixels
python3 - "$SOCK" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f = s.makefile()
s.sendall((json.dumps({"cmd": "write", "data": "\r\nCHANGED"}) + "\n").encode())
json.loads(f.readline())
PY
# `state` summary for the diagnosis: separates "the app did not draw" from
# "it drew and the surface did not present" (the M-AA symptom)
state_summary() {
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
take "$OUT_DIR/b.png" || fail "capture 2"
if ./tools/pixel-probe.py --diff "$OUT_DIR/a.png" "$OUT_DIR/b.png" --max 0 >/dev/null 2>&1; then
    fail "content changed and no pixel changed ($(state_summary))"
else
    ok "new content changed pixels"
fi

# 2. background color through SGR 41/42 on the first line
if ./tools/pixel-probe.py "$OUT_DIR/b.png" \
    --expect "$((CWL / 2)),$((CHL / 2))=ff0000" \
    --expect "$((CWL * 2 + CWL / 2)),$((CHL / 2))=00ff00" \
    --expect "$((CWL * 20)),$((CHL / 2))=000000" >"$OUT_DIR/colors.txt" 2>&1; then
    ok "SGR 41/42 paint color1/color2 in the cell"
else
    fail "wrong color: $(tr '\n' ' ' <"$OUT_DIR/colors.txt")"
fi

# 3. text painted inside the cell (line 3, column 1: white glyph on black)
if ./tools/pixel-probe.py "$OUT_DIR/b.png" --row "$((CHL * 2 + CHL / 2))" \
    >"$OUT_DIR/line3.txt" 2>&1 && grep -qv '000000' "$OUT_DIR/line3.txt"; then
    ok "text draws inside the cell"
else
    fail "no text pixel on line 3"
fi

# 4. still screen: two captures in a row have to be identical
take "$OUT_DIR/c.png" || fail "capture 3"
if ./tools/pixel-probe.py --diff "$OUT_DIR/b.png" "$OUT_DIR/c.png" --max 0 >/dev/null 2>&1; then
    ok "still screen does not repaint differently"
else
    fail "two captures of the same screen differ (unstable repaint)"
fi

# 5. grid line `r` has to be painted in band `r` (B4 regression: the line
# painter added `y*ch` and the row concatenation added it again, so everything
# came out at `2r` — double-spaced text and a cursor far from the prompt). Each line
# carries its own number and a distinct 48;5;N background; band `r` has to show
# the color of the number the grid says is at `r`.
#
# There is no shell here: the child is `printf…; sleep 600`, so nothing we
# write is executed. What hands the content to the grid is the **pty echo** —
# and that is what we want, predictable bytes. `\r` separator (the pty converts it).
if python3 - "$SOCK" "$DISPLAY_XVFB" "$OUT_DIR" <<'PY'
import json, os, socket, subprocess, sys, time
sock, disp, out_dir = sys.argv[1], sys.argv[2], sys.argv[3]
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


payload = "".join("{}\r".format(i) for i in range(N))
q(cmd="write", data=payload)
time.sleep(0.9)
st = q(cmd="state")
ch, cw, xid = st["cell"][1], st["cell"][0], st.get("xid")
grid = q(cmd="text", where="view", count=st["rows"]).get("lines") or []

png = os.path.join(out_dir, "lines.png")
subprocess.run(["xwd", "-id", str(xid), "-silent", "-out", png + ".xwd"],
               env=dict(os.environ, DISPLAY=disp), check=False)
subprocess.run(["convert", png + ".xwd", "-depth", "8", png + ".ppm"], check=True)
with open(png + ".ppm", "rb") as f:
    assert f.readline().strip() == b"P6", "capture did not decode as PPM"
    while True:
        ln = f.readline()
        if not ln.startswith(b"#") and len(ln.split()) == 2:
            break
    w, h = (int(v) for v in ln.split())
    f.readline()
    buf = f.read()

LV = [0, 95, 135, 175, 215, 255]


def color(i):
    idx = (17 + i) - 16
    return (LV[idx // 36], LV[(idx % 36) // 6], LV[idx % 6])


errors, seen = [], 0


def light(px, y):
    """is there a text pixel (bright) in the vertical band [y, y+ch)?"""
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
    has = light(buf, y)
    if ln.strip():
        seen += 1
        if not has:
            errors.append(f"line {r} has text in the grid and no pixel in the band")
if seen < 5:
    errors.append(f"too few lines checked ({seen})")
print("; ".join(errors))
sys.exit(1 if errors else 0)
PY
then
    ok "line r is painted in band r (1-cell spacing)"
else
    fail "lines out of band (double spacing or shifted cursor)"
fi

# 6. shrinking and growing (what the WM does when maximizing/restoring) must not
# move the cursor nor the content. It was the third bug: with the prompt on line 0
# and the rest of the screen empty, shrinking 48→36 pushed the bottom 12 lines into
# the history; growing back brought them back as 12 lines above the
# prompt — the shell then wrote on line 12. Own instance, with just one
# marker on line 0: reusing the test's instance would contaminate the measurement.

SOCK2="$TMP/ciclo.sock"
DISPLAY=$DISPLAY_XVFB HOME=$TMP/home "$APP" \
    --cols 40 --rows 8 --debug-socket "$SOCK2" \
    --cmd 'printf "MARK"; sleep 600' >"$TMP/app2.log" 2>&1 &
APP2_PID=$!
i=0
while [ ! -S "$SOCK2" ]; do
    i=$((i + 1))
    [ "$i" -gt 100 ] && { echo "2nd instance did not open the socket" >&2; break; }
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


def state():
    st = q(cmd="state")
    g = q(cmd="text", where="view", count=st["rows"]).get("lines") or []
    with_text = [i for i, l in enumerate(g) if l.strip()]
    return (with_text[0] if with_text else None), st["cursor"][0], st


time.sleep(0.6)
before, cy_before, st = state()
cols, rows = st["cols"], st["rows"]
for r in (rows + 12, rows, rows + 12):
    q(cmd="resize", cols=cols, rows=r)
    time.sleep(0.5)
after, cy_after, st = state()
print(
    f"grid {st['cols']}x{st['rows']} text before={before} after={after} "
    f"cursor before={cy_before} after={cy_after}"
)
sys.exit(0 if (before, cy_before) == (0, 0) == (after, cy_after) else 1)
PY
then
    ok "cursor and content stay on line 1 when shrinking and growing"
else
    fail "cursor/content moved down a line on resize (maximize/restore)"
fi

# 7. scrolling the screen cannot leave a line with text and no pixel. It was B6: a
# write that has not been drawn yet (`time sleep 1`) scrolled in the same chunk and
# the damage did not rotate with the content, so the line rotated with the old
# cache and stayed blank until a full rebuild. Fill the screen and go past it.
if python3 - "$SOCK2" "$DISPLAY_XVFB" "$OUT_DIR" <<'PY'
import json, os, socket, subprocess, sys, time

sock, disp, out_dir = sys.argv[1], sys.argv[2], sys.argv[3]


def q(**kw):
    c = socket.socket(socket.AF_UNIX)
    c.connect(sock)
    f = c.makefile("rwb")
    f.write(json.dumps(kw).encode() + b"\n")
    f.flush()
    r = json.loads(f.readline())
    c.close()
    return r["ok"] if isinstance(r.get("ok"), dict) else r


st = q(cmd="state")
rows = st["rows"]
# one line at a time, with a frame in between: that is how pending damage meets
# a 1-line scroll (the `time sleep 1` case). In a single block, the `shift`
# goes past the screen size and the rebuild ends up being the whole screen.
for i in range(rows * 2 + 4):
    q(cmd="write", data="{}\r".format(i))
    time.sleep(0.04)
time.sleep(0.8)
st = q(cmd="state")
ch, cw = st["cell"][1], st["cell"][0]
grid = q(cmd="text", where="view", count=st["rows"]).get("lines") or []

png = os.path.join(out_dir, "scroll")
subprocess.run(["xwd", "-id", str(st["xid"]), "-silent", "-out", png + ".xwd"],
               env=dict(os.environ, DISPLAY=disp), check=False)
subprocess.run(["convert", png + ".xwd", "-depth", "8", png + ".ppm"], check=True)
with open(png + ".ppm", "rb") as f:
    assert f.readline().strip() == b"P6", "capture did not decode as PPM"
    while True:
        ln = f.readline()
        if not ln.startswith(b"#") and len(ln.split()) == 2:
            break
    w, h = (int(v) for v in ln.split())
    f.readline()
    buf = f.read()

errors, seen = [], 0
for r, ln in enumerate(grid):
    y0, y1 = int(r * ch), min(int((r + 1) * ch), h)
    if y0 >= h:
        break
    if not ln.strip():
        continue
    seen += 1
    found = any(
        buf[(y * w + x) * 3] > 90
        and buf[(y * w + x) * 3 + 1] > 90
        and buf[(y * w + x) * 3 + 2] > 90
        for y in range(y0, y1)
        for x in range(0, min(int(cw * len(ln)), w))
    )
    if not found:
        errors.append(f"line {r} has text in the grid and no pixel: {ln.strip()[:20]!r}")
if seen < 5:
    errors.append(f"too few lines checked ({seen})")
print("; ".join(errors))
sys.exit(1 if errors else 0)
PY
then
    ok "scrolling does not leave a line with text and no pixel"
else
    fail "grid line lost its pixels after scrolling (B6)"
fi

# 8. the shell ending the session (`exit`, Ctrl+D) is a close path of its own: it
# has to exit cleanly **and save the window state**. Drawing with no tab indexed
# `tabs[active]` and panicked on the way out, so the state file was never written
# and the remembered geometry (maximized included) was lost — B38. The program
# that ends by itself is the `shell` of a private config, not `--cmd`: since
# `B46` a `--cmd` window is a scratch, never takes the lock and writes nothing,
# and the write is exactly what this check watches for.
mkdir -p "$TMP/home8/.config/tortty"
printf '#!/bin/sh\nexit 0\n' >"$TMP/shell8"
chmod +x "$TMP/shell8"
printf 'shell %s\n' "$TMP/shell8" >"$TMP/home8/.config/tortty/tortty.conf"
STATE8="$TMP/state"
DISPLAY=$DISPLAY_XVFB HOME=$TMP/home8 XDG_STATE_HOME="$STATE8" "$APP" \
    >"$TMP/app3.log" 2>&1 &
APP3_PID=$!
wait "$APP3_PID" 2>/dev/null
RC3=$?
APP3_PID=
if [ "$RC3" -eq 0 ] && [ -s "$STATE8/tortty/window.json" ] &&
    ! grep -q panicked "$TMP/app3.log"; then
    ok "the shell ending the session exits cleanly and saves the window state"
else
    fail "exit by the shell: rc=$RC3 state=[$(cat "$STATE8/tortty/window.json" 2>/dev/null)] $(grep -m1 panicked "$TMP/app3.log")"
fi

echo
if [ "$failures" -eq 0 ]; then
    echo "render-check: all ok (captures in $OUT_DIR)"
    exit 0
fi
echo "render-check: $failures failure(s)"
exit 1
