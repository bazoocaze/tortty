#!/usr/bin/env python3
"""benchrun.py — one doom-fire measurement with the condition locked by the GRID.

Port of `tortty-python/tools/benchrun.py` to the Rust app, with the **same
protocol**: starts the app with the socket, uses the `resize` verb until the grid
matches the target (checked with `state`), and only then types the load into the pty
and reads the statistics.

The generator is the same file (`tools/doomfire.py`, byte for byte the one from
tortty-python), so what is measured is the consumer — the terminal — and not the load.

Usage:
  tools/benchrun.py                       # Rust app, 1200 frames, 61x18
  tools/benchrun.py --app ./run.sh        # Python app, same condition
  tools/benchrun.py --cols 70 --rows 21 --frames 600

Prints `key value` per line. Exits with code 3 if the grid does not match (a number
from another condition is not a number).
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
ROOT_DIR = os.path.dirname(HERE)
DEFAULT_APP = os.path.join(ROOT_DIR, "target", "release", "tortty")
DOOMFIRE = os.path.join(HERE, "doomfire.py")


class Failure(Exception):
    pass


class Client:
    """Socket client: one JSON request per line (the same one as Python)."""

    def __init__(self, path):
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.settimeout(30)
        self.s.connect(path)
        self.buf = b""

    def ask(self, **req):
        cmd = req.pop("cmd")
        self.s.sendall((json.dumps({"cmd": cmd, **req}) + "\n").encode())
        while b"\n" not in self.buf:
            chunk = self.s.recv(65536)
            if not chunk:
                raise Failure("socket closed before the response")
            self.buf += chunk
        line, _, self.buf = self.buf.partition(b"\n")
        resp = json.loads(line)
        if not resp.get("ok"):
            raise Failure(resp.get("err", "response without ok"))
        return resp

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass


def wait_socket(path, proc, limit=25.0):
    end = time.monotonic() + limit
    while time.monotonic() < end:
        if proc.poll() is not None:
            raise Failure("the app died before opening the socket (code %s)" % proc.returncode)
        if os.path.exists(path):
            try:
                return Client(path)
            except OSError:
                pass
        time.sleep(0.1)
    raise Failure("socket %s did not open within %.0fs" % (path, limit))


def lock_grid(client, cols, rows, attempts=8):
    st = client.ask(cmd="state")
    if not cols or not rows:
        return st
    for _ in range(attempts):
        if st["cols"] == cols and st["rows"] == rows:
            return st
        client.ask(cmd="resize", cols=cols, rows=rows)
        time.sleep(0.4)
        st = client.ask(cmd="state")
    if st["cols"] != cols or st["rows"] != rows:
        raise Failure(
            "grid ended up %dx%d, target %dx%d (did the WM refuse the resize?)"
            % (st["cols"], st["rows"], cols, rows)
        )
    return st


def sample_frequency(stop, sink):
    while not stop.is_set():
        try:
            values = []
            for cpu in os.listdir("/sys/devices/system/cpu"):
                path = "/sys/devices/system/cpu/%s/cpufreq/scaling_cur_freq" % cpu
                try:
                    with open(path) as f:
                        values.append(int(f.read()))
                except OSError:
                    pass
            if values:
                sink.append(max(values))
        except OSError:
            pass
        stop.wait(0.2)


def read_stats(path):
    data = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            fields = line.split(None, 1)
            if len(fields) == 2:
                data[fields[0].rstrip("/")] = fields[1].strip()
    return data


def main(argv):
    ap = argparse.ArgumentParser(description="one doom-fire measurement locked by the grid")
    ap.add_argument("--frames", type=int, default=1200)
    ap.add_argument("--cols", type=int, default=61)
    ap.add_argument("--rows", type=int, default=18)
    ap.add_argument("--font-size", default="13")
    ap.add_argument("--app", default=DEFAULT_APP)
    ap.add_argument("--env", action="append", default=[], metavar="K=V")
    ap.add_argument("--extra", action="append", default=[], metavar="ARG",
                    help="extra argument for the app")
    ap.add_argument("--timeout", type=float, default=0)
    a = ap.parse_args(argv)

    tmp = tempfile.mkdtemp(prefix="tortty-rs-bench-")
    sock = os.path.join(tmp, "tortty.sock")
    stats = os.path.join(tmp, "stats.txt")
    log = os.path.join(tmp, "app.log")

    env = dict(os.environ)
    env["XDG_STATE_HOME"] = os.path.join(tmp, "state")

    if a.app.endswith(".py") or a.app.endswith("run.sh"):
        # tortty-python Python app: accepts -c FILE (config) and --debug-log
        conf = os.path.join(tmp, "tortty.conf")
        with open(conf, "w", encoding="utf-8") as f:
            f.write("font_size %s\n" % a.font_size)
        cmdline = [a.app, "-c", conf, "--debug-log", "off", "--debug-socket", sock]
        env["PYTERM_PROF"] = "1"
        env.pop("PYTERM_GEOM", None)
    else:
        cmdline = [
            a.app,
            "--debug-socket",
            sock,
            "--no-vsync",
            "--prof",
            "--font-size",
            str(a.font_size),
        ]
        cmdline += a.extra

    for pair in a.env:
        key, _, value = pair.partition("=")
        env[key] = value

    limit = a.timeout or (a.frames / 20.0 + 60)
    frequencies = []
    stop = None
    with open(log, "w", encoding="utf-8") as out:
        proc = subprocess.Popen(cmdline, env=env, stdout=out, stderr=out, cwd=ROOT_DIR)
    try:
        client = wait_socket(sock, proc)
        st = lock_grid(client, a.cols, a.rows)
        if os.path.exists(stats):
            os.unlink(stats)
        stop = threading.Event()
        sampler = threading.Thread(
            target=sample_frequency, args=(stop, frequencies), daemon=True
        )
        sampler.start()
        start = time.monotonic()
        client.ask(
            cmd="write",
            data="python3 %s --fps 0 --frames %d --stats-to %s\r"
            % (DOOMFIRE, a.frames, stats),
        )
        while not os.path.exists(stats):
            if time.monotonic() - start > limit:
                raise Failure("doomfire did not write %s within %.0fs" % (stats, limit))
            if proc.poll() is not None:
                raise Failure("the app died during the load (code %s)" % proc.returncode)
            time.sleep(0.2)
        time.sleep(0.3)
        try:
            prof = client.ask(cmd="prof")
        except (Failure, OSError):
            prof = {}
        client.ask(cmd="quit")
        client.close()
    finally:
        if stop is not None:
            stop.set()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()

    data = read_stats(stats)
    grid = data.get("grade", "?")
    mbs = data.get("written", "")
    mbs = mbs.split("(")[-1].rstrip(")") if "(" in mbs else "-"
    peak = max(frequencies) / 1e6 if frequencies else 0.0
    mean = sum(frequencies) / len(frequencies) / 1e6 if frequencies else 0.0

    print("app %s" % ("python" if cmdline[0].endswith((".py", "run.sh")) else "rust"))
    print("grade %s" % grid.split()[0] if grid else "?")
    print("fps %s" % data.get("fps", "-"))
    print("mbs %s" % mbs)
    print("bpf %s" % data.get("bytes/frame", "-"))
    if "draw_ms_call" in prof:
        print("draw %.2f ms/call (%s calls)" % (prof["draw_ms_call"], prof.get("draw_calls", 0)))
        print("parse %.2f ms/call (%s calls)" % (prof["parse_ms_call"], prof.get("parse_calls", 0)))
        print("pty %.2f ms/call (%s calls)" % (prof["pty_ms_call"], prof.get("pty_calls", 0)))
    else:
        # Python app: read the `prof:` line from the log
        try:
            with open(log, encoding="utf-8", errors="replace") as f:
                lines = [ln for ln in f if ln.startswith("prof:")]
            if lines:
                for field in lines[-1][5:].split("|"):
                    parts = field.split()
                    if len(parts) >= 3 and parts[0] in ("draw", "pty"):
                        print("%s %s" % (parts[0], parts[2]))
        except OSError:
            pass
    print("cell %s" % (st.get("cell", "-")))
    print("ghz %.2f/%.2f" % (mean, peak))
    print("log %s" % log)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except Failure as error:
        print("error: %s" % error, file=sys.stderr)
        sys.exit(3)
