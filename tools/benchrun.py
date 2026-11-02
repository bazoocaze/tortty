#!/usr/bin/env python3
"""benchrun.py — uma medição do doom-fire com a condição travada pelo GRID.

Port do `tortty-python/tools/benchrun.py` para o app em Rust, com o **mesmo
protocolo**: sobe o app com o socket, usa o verbo `resize` até o grid bater no
alvo (confere com `state`), só então digita a carga no pty e lê as estatísticas.

O gerador é o mesmo arquivo (`tools/doomfire.py`, byte a byte igual ao do
tortty-python), então o que se mede é o consumidor — o terminal — e não a carga.

Uso:
  tools/benchrun.py                       # app Rust, 1200 frames, 61x18
  tools/benchrun.py --app ./run.sh        # app Python, mesma condição
  tools/benchrun.py --cols 70 --rows 21 --frames 600

Imprime `chave valor` por linha. Sai com código 3 se o grid não bater (número
de outra condição não é número).
"""

import argparse
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(HERE)
APP_PADRAO = os.path.join(RAIZ, "target", "release", "bzmux")
DOOMFIRE = os.path.join(HERE, "doomfire.py")


class Falha(Exception):
    pass


class Alvo:
    """Cliente do socket: um pedido JSON por linha (o mesmo do Python)."""

    def __init__(self, caminho):
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.settimeout(30)
        self.s.connect(caminho)
        self.buf = b""

    def pede(self, **req):
        cmd = req.pop("cmd")
        self.s.sendall((json.dumps({"cmd": cmd, **req}) + "\n").encode())
        while b"\n" not in self.buf:
            pedaco = self.s.recv(65536)
            if not pedaco:
                raise Falha("socket fechou antes da resposta")
            self.buf += pedaco
        linha, _, self.buf = self.buf.partition(b"\n")
        resp = json.loads(linha)
        if not resp.get("ok"):
            raise Falha(resp.get("err", "resposta sem ok"))
        return resp

    def fecha(self):
        try:
            self.s.close()
        except OSError:
            pass


def espera_socket(caminho, proc, limite=25.0):
    fim = time.monotonic() + limite
    while time.monotonic() < fim:
        if proc.poll() is not None:
            raise Falha("o app morreu antes de abrir o socket (código %s)" % proc.returncode)
        if os.path.exists(caminho):
            try:
                return Alvo(caminho)
            except OSError:
                pass
        time.sleep(0.1)
    raise Falha("socket %s não abriu em %.0fs" % (caminho, limite))


def trava_grid(cliente, cols, rows, tentativas=8):
    st = cliente.pede(cmd="state")
    if not cols or not rows:
        return st
    for _ in range(tentativas):
        if st["cols"] == cols and st["rows"] == rows:
            return st
        cliente.pede(cmd="resize", cols=cols, rows=rows)
        time.sleep(0.4)
        st = cliente.pede(cmd="state")
    if st["cols"] != cols or st["rows"] != rows:
        raise Falha(
            "grade ficou %dx%d, alvo %dx%d (WM recusou o resize?)"
            % (st["cols"], st["rows"], cols, rows)
        )
    return st


def amostra_frequencia(parar, guarda):
    while not parar.is_set():
        try:
            valores = []
            for cpu in os.listdir("/sys/devices/system/cpu"):
                caminho = "/sys/devices/system/cpu/%s/cpufreq/scaling_cur_freq" % cpu
                try:
                    with open(caminho) as f:
                        valores.append(int(f.read()))
                except OSError:
                    pass
            if valores:
                guarda.append(max(valores))
        except OSError:
            pass
        parar.wait(0.2)


def le_stats(caminho):
    dados = {}
    with open(caminho, encoding="utf-8") as f:
        for linha in f:
            campos = linha.split(None, 1)
            if len(campos) == 2:
                dados[campos[0].rstrip("/")] = campos[1].strip()
    return dados


def main(argv):
    ap = argparse.ArgumentParser(description="uma medição do doom-fire pelo grid")
    ap.add_argument("--frames", type=int, default=1200)
    ap.add_argument("--cols", type=int, default=61)
    ap.add_argument("--rows", type=int, default=18)
    ap.add_argument("--font-size", default="13")
    ap.add_argument("--app", default=APP_PADRAO)
    ap.add_argument("--env", action="append", default=[], metavar="K=V")
    ap.add_argument("--extra", action="append", default=[], metavar="ARG",
                    help="argumento extra para o app")
    ap.add_argument("--timeout", type=float, default=0)
    a = ap.parse_args(argv)

    tmp = tempfile.mkdtemp(prefix="bzmux-rs-bench-")
    sock = os.path.join(tmp, "bzmux.sock")
    stats = os.path.join(tmp, "stats.txt")
    log = os.path.join(tmp, "app.log")

    env = dict(os.environ)
    env["XDG_STATE_HOME"] = os.path.join(tmp, "state")

    if a.app.endswith(".py") or a.app.endswith("run.sh"):
        # app Python do tortty-python: aceita -c ARQ (config) e --debug-log
        conf = os.path.join(tmp, "bzmux.conf")
        with open(conf, "w", encoding="utf-8") as f:
            f.write("font_size %s\n" % a.font_size)
        ordem = [a.app, "-c", conf, "--debug-log", "off", "--debug-socket", sock]
        env["PYTERM_PROF"] = "1"
        env.pop("PYTERM_GEOM", None)
    else:
        ordem = [
            a.app,
            "--debug-socket",
            sock,
            "--no-vsync",
            "--prof",
            "--font-size",
            str(a.font_size),
        ]
        ordem += a.extra

    for par in a.env:
        chave, _, valor = par.partition("=")
        env[chave] = valor

    limite = a.timeout or (a.frames / 20.0 + 60)
    frequencias = []
    parar = None
    with open(log, "w", encoding="utf-8") as saida:
        proc = subprocess.Popen(ordem, env=env, stdout=saida, stderr=saida, cwd=RAIZ)
    try:
        cliente = espera_socket(sock, proc)
        st = trava_grid(cliente, a.cols, a.rows)
        if os.path.exists(stats):
            os.unlink(stats)
        parar = threading.Event()
        amostrador = threading.Thread(
            target=amostra_frequencia, args=(parar, frequencias), daemon=True
        )
        amostrador.start()
        inicio = time.monotonic()
        cliente.pede(
            cmd="write",
            data="python3 %s --fps 0 --frames %d --stats-to %s\r"
            % (DOOMFIRE, a.frames, stats),
        )
        while not os.path.exists(stats):
            if time.monotonic() - inicio > limite:
                raise Falha("o doomfire não escreveu %s em %.0fs" % (stats, limite))
            if proc.poll() is not None:
                raise Falha("o app morreu durante a carga (código %s)" % proc.returncode)
            time.sleep(0.2)
        time.sleep(0.3)
        try:
            prof = cliente.pede(cmd="prof")
        except (Falha, OSError):
            prof = {}
        cliente.pede(cmd="quit")
        cliente.fecha()
    finally:
        if parar is not None:
            parar.set()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()

    dados = le_stats(stats)
    grade = dados.get("grade", "?")
    mbs = dados.get("escrito", "")
    mbs = mbs.split("(")[-1].rstrip(")") if "(" in mbs else "-"
    pico = max(frequencias) / 1e6 if frequencias else 0.0
    media = sum(frequencias) / len(frequencias) / 1e6 if frequencias else 0.0

    print("app %s" % ("python" if ordem[0].endswith((".py", "run.sh")) else "rust"))
    print("grade %s" % grade.split()[0] if grade else "?")
    print("fps %s" % dados.get("fps", "-"))
    print("mbs %s" % mbs)
    print("bpf %s" % dados.get("bytes/frame", "-"))
    if "draw_ms_call" in prof:
        print("draw %.2f ms/call (%s chamadas)" % (prof["draw_ms_call"], prof.get("draw_calls", 0)))
        print("parse %.2f ms/call (%s chamadas)" % (prof["parse_ms_call"], prof.get("parse_calls", 0)))
        print("pty %.2f ms/call (%s chamadas)" % (prof["pty_ms_call"], prof.get("pty_calls", 0)))
    else:
        # app Python: lê a linha `prof:` do log
        try:
            with open(log, encoding="utf-8", errors="replace") as f:
                linhas = [ln for ln in f if ln.startswith("prof:")]
            if linhas:
                for campo in linhas[-1][5:].split("|"):
                    partes = campo.split()
                    if len(partes) >= 3 and partes[0] in ("draw", "pty"):
                        print("%s %s" % (partes[0], partes[2]))
        except OSError:
            pass
    print("cell %s" % (st.get("cell", "-")))
    print("ghz %.2f/%.2f" % (media, pico))
    print("log %s" % log)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except Falha as erro:
        print("erro: %s" % erro, file=sys.stderr)
        sys.exit(3)
