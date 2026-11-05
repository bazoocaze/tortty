#!/usr/bin/env python3
"""Doom fire in the terminal: maximum write load to measure the hot path.

The PSX port effect from Doom: intensity buffer 0..36 propagated from the bottom
up with random decay and a palette of 37 colors. Drawn in a half block (`▀`, two
pixels per cell) with truecolor fg and bg per cell, grouping nothing — it is the
worst case for the parser and for drawing.

Measurement, not a test: run it by hand inside the terminal you want to measure.

    python3 tools/doomfire.py                 # 30 fps until Ctrl+C
    python3 tools/doomfire.py --fps 0 --frames 600
    ./debug.sh --cmd 'python3 tools/doomfire.py --fps 0 --frames 600 \
        --stats-to /tmp/fire-bzmux.txt'      # statistics into the file

`--fps 0` lets go: the terminal becomes the limiter, and the fps reached (and
the MB/s of escapes) is its throughput measure. The statistics come out on exit, on
stderr or in the requested file, so that two terminals can be compared.

MIT (see LICENSE).
"""

import argparse
import os
import shutil
import signal
import sys
import time

BLOCK = "▀".encode()

# doom fire palette (37 shades, from near-black to white)
PALETTE = [
    (0x07, 0x07, 0x07),
    (0x1F, 0x07, 0x07),
    (0x2F, 0x0F, 0x07),
    (0x47, 0x0F, 0x07),
    (0x57, 0x17, 0x07),
    (0x67, 0x1F, 0x07),
    (0x77, 0x1F, 0x07),
    (0x8F, 0x27, 0x07),
    (0x9F, 0x2F, 0x07),
    (0xAF, 0x3F, 0x07),
    (0xBF, 0x47, 0x07),
    (0xC7, 0x47, 0x07),
    (0xDF, 0x4F, 0x07),
    (0xDF, 0x57, 0x07),
    (0xDF, 0x57, 0x07),
    (0xD7, 0x5F, 0x07),
    (0xD7, 0x5F, 0x07),
    (0xD7, 0x67, 0x0F),
    (0xCF, 0x6F, 0x0F),
    (0xCF, 0x77, 0x0F),
    (0xCF, 0x7F, 0x0F),
    (0xCF, 0x87, 0x17),
    (0xC7, 0x87, 0x17),
    (0xC7, 0x8F, 0x17),
    (0xC7, 0x97, 0x1F),
    (0xBF, 0x9F, 0x1F),
    (0xBF, 0x9F, 0x1F),
    (0xBF, 0xA7, 0x27),
    (0xBF, 0xA7, 0x27),
    (0xBF, 0xAF, 0x2F),
    (0xB7, 0xAF, 0x2F),
    (0xB7, 0xB7, 0x2F),
    (0xB7, 0xB7, 0x37),
    (0xCF, 0xCF, 0x6F),
    (0xDF, 0xDF, 0x9F),
    (0xEF, 0xEF, 0xC7),
    (0xFF, 0xFF, 0xFF),
]

# ready-made escapes per shade: building the string per cell would measure str,
# not the terminal, and the format is the same on every frame
FG = [b"\033[38;2;%d;%d;%dm" % c for c in PALETTE]
BG = [b"\033[48;2;%d;%d;%dm" % c for c in PALETTE]
LAST = len(PALETTE) - 1


def update(buf, w):
    """One fire step: each pixel inherits from the one below, with decay."""
    rand = int.from_bytes(os.urandom(1), "big")  # 0..255, cheap enough
    n = len(buf)
    seed = rand | 0x100  # guarantees an odd multiplier in the xorshift
    for src in range(w, n):
        if seed & 1:
            seed = (seed >> 1) ^ 0x8F3F73B5
        else:
            seed >>= 1
        pixel = buf[src]
        if not pixel:
            buf[src - w] = 0
            continue
        # sideways shift of -1..+1 and decay of 0..1
        dst = src - w + (seed % 3) - 1
        if dst < 0:
            continue
        v = pixel - (seed & 1)
        buf[dst] = v if v > 0 else 0


def render(buf, w, h):
    """Whole frame in a half block: fg = upper pixel, bg = the lower one."""
    out = bytearray(b"\033[H")
    fg, bg, block = FG, BG, BLOCK
    for row in range(0, h, 2):
        top = row * w
        bottom = top + w
        for x in range(w):
            out += fg[buf[top + x]]
            out += bg[buf[bottom + x]]
            out += block
    return out


def main(argv):
    ap = argparse.ArgumentParser(description="doom fire in the terminal (measurement)")
    ap.add_argument(
        "--fps",
        type=float,
        default=30.0,
        help="frames per second; 0 = no limit (default 30)",
    )
    ap.add_argument(
        "--frames",
        type=int,
        default=0,
        help="stop after N frames; 0 = until Ctrl+C",
    )
    ap.add_argument("--stats-to", default="-", help="statistics file; - = stderr")
    a = ap.parse_args(argv)

    cols, lines = shutil.get_terminal_size((80, 24))
    w, h = cols, lines * 2
    buf = bytearray(w * h)
    buf[-w:] = bytes([LAST]) * w  # last row at maximum fire

    # SIGTERM/SIGINT stop the loop (and do not kill before the statistics): you can
    # measure with `timeout -s TERM 20 python3 tools/doomfire.py --fps 0`
    stop = []
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, lambda *_: stop.append(True))

    out = sys.stdout.buffer
    out.write(b"\033[?1049h\033[0m\033[2J\033[?25l")
    out.flush()

    frames = 0
    sent = 0
    t0 = time.monotonic()
    deadline = t0
    try:
        while not stop and (not a.frames or frames < a.frames):
            update(buf, w)
            frame = render(buf, w, h)
            out.write(frame)
            out.flush()  # without a flush there is no throughput to measure
            frames += 1
            sent += len(frame)
            if a.fps > 0:
                deadline += 1.0 / a.fps
                delay = deadline - time.monotonic()
                if delay > 0:
                    time.sleep(delay)
                else:
                    deadline = time.monotonic()
    except KeyboardInterrupt:
        pass
    finally:
        # without the SGR reset the statistics text inherits the last fire color
        out.write(b"\033[0m\033[?25h\033[?1049l\033[0m")
        out.flush()

    dt = time.monotonic() - t0
    lines_out = [
        f"grade       {cols}x{lines} ({w}x{h} pixels)",
        f"frames      {frames} in {dt:.2f}s",
        f"fps         {frames / dt:.1f}",
        f"written     {sent / 1e6:.1f} MB ({sent / dt / 1e6:.1f} MB/s)",
        f"bytes/frame {sent / frames if frames else 0:.0f}",
    ]
    txt = "\n".join(lines_out) + "\n"
    if a.stats_to == "-":
        sys.stderr.write(txt)
    else:
        with open(a.stats_to, "w", encoding="utf-8") as f:
            f.write(txt)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
