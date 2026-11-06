#!/usr/bin/env python3
"""Reads and compares pixels from a screen capture — to prove what was drawn.

The capture needs no toolkit: the window is grabbed by ImageMagick's `import` and
the PNG is decoded by `convert`, which `import` already requires. That way the
same script runs here, in CI and over ssh.

    tools/pixel.py shot.png                 # default sample
    tools/pixel.py shot.png 100,50 200,50   # specific (x,y) points
    tools/pixel.py shot.png --scan 300      # column x=300, one sample per cell
    tools/pixel.py shot.png --row 515       # line y=515 sampled along the width
    tools/pixel.py --expect 100,50=ff0000   # checks the color (± --tol)
    tools/pixel.py --diff a.png b.png       # how many pixels changed and where
    tools/pixel.py --diff a.png b.png --max 0   # fails (exit 1) if anything changed

`--diff` is what turns a capture into a **test**: pages with the same content have
to give 0 differing pixels; new content has to give a difference — and that is how you
prove the window is really drawing (dead surface = 0 diffs).
"""

import argparse
import subprocess
import sys
from dataclasses import dataclass


class Error(Exception):
    pass


@dataclass
class Img:
    w: int
    h: int
    px: bytes

    def color(self, x: int, y: int) -> tuple[int, int, int]:
        if not (0 <= x < self.w and 0 <= y < self.h):
            raise Error(f"point outside the image: {x},{y} ({self.w}x{self.h})")
        o = (y * self.w + x) * 3
        return self.px[o], self.px[o + 1], self.px[o + 2]


def load(path: str) -> Img:
    """PNG (or anything ImageMagick reads) to raw RGB, through PPM."""
    try:
        out = subprocess.run(
            ["convert", path, "-depth", "8", "ppm:-"],
            capture_output=True,
            check=True,
        ).stdout
    except FileNotFoundError:
        raise Error("convert not found (ImageMagick)")
    except subprocess.CalledProcessError as e:
        raise Error(f"could not read {path}: {e.stderr.decode().strip()}")
    # P6 <width> <height> <max>\n<bytes>: the header is ASCII with comments (`#`)
    tokens, i = [], 0
    while len(tokens) < 4:
        while i < len(out) and out[i : i + 1].isspace():
            i += 1
        if out[i : i + 1] == b"#":
            while i < len(out) and out[i : i + 1] != b"\n":
                i += 1
            continue
        j = i
        while j < len(out) and not out[j : j + 1].isspace():
            j += 1
        tokens.append(out[i:j])
        i = j
    if tokens[0] != b"P6":
        raise Error(f"unexpected format: {tokens[0]!r} (expected P6)")
    w, h, maxval = (int(t) for t in tokens[1:4])
    if maxval != 255:
        raise Error(f"depth {maxval}: expected 8 bits per channel")
    data = out[i + 1 : i + 1 + w * h * 3]
    if len(data) != w * h * 3:
        raise Error(f"truncated PPM: {len(data)} bytes for {w}x{h}")
    return Img(w, h, data)


def to_hex(c: tuple[int, int, int]) -> str:
    return "%02x%02x%02x" % c


def close(a: tuple[int, int, int], b: tuple[int, int, int], tol: int) -> bool:
    return all(abs(x - y) <= tol for x, y in zip(a, b))


def diffs(a: Img, b: Img, tol: int) -> tuple[int, tuple[int, int, int, int] | None]:
    """Differing pixels and the rectangle that contains them."""
    if (a.w, a.h) != (b.w, b.h):
        raise Error(f"different sizes: {a.w}x{a.h} and {b.w}x{b.h}")
    n = 0
    x0 = y0 = 10**9
    x1 = y1 = -1
    for y in range(a.h):
        for x in range(a.w):
            if not close(a.color(x, y), b.color(x, y), tol):
                n += 1
                x0, y0, x1, y1 = min(x0, x), min(y0, y), max(x1, x), max(y1, y)
    return n, (None if n == 0 else (x0, y0, x1 - x0 + 1, y1 - y0 + 1))


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("png", nargs="?", help="capture to read")
    ap.add_argument("points", nargs="*", help="x,y (repeatable)")
    ap.add_argument("--scan", type=int, metavar="X", help="column X, one sample per cell")
    ap.add_argument("--row", type=int, metavar="Y", help="line Y sampled along the width")
    ap.add_argument("--cell", metavar="WxH", default="0x0", help="cell size (e.g. 16x31)")
    ap.add_argument("--expect", action="append", default=[], metavar="X,Y=RRGGBB")
    ap.add_argument("--tol", type=int, default=8, help="per-channel tolerance (default 8)")
    ap.add_argument("--diff", nargs=2, metavar=("A", "B"), help="compares two captures")
    ap.add_argument("--max", type=int, default=None, help="maximum differing pixels")
    args = ap.parse_args(argv)

    try:
        cw, ch = (int(v) for v in args.cell.split("x"))

        if args.diff:
            a, b = load(args.diff[0]), load(args.diff[1])
            n, box = diffs(a, b, args.tol)
            pct = 100.0 * n / (a.w * a.h)
            print(f"{n} differing pixel(s) ({pct:.2f}%)", end="")
            if box:
                print(f" box {box[0]},{box[1]} {box[2]}x{box[3]}")
            else:
                print()
            if args.max is not None and n > args.max:
                print(f"FAILURE: maximum was {args.max}")
                return 1
            return 0

        if not args.png:
            raise Error("give the capture (or --diff A B)")
        img = load(args.png)
        print(f"{args.png}: {img.w}x{img.h}")

        if args.expect:
            failures = 0
            for expr in args.expect:
                point, color = expr.split("=")
                x, y = (int(v) for v in point.split(","))
                expected = tuple(int(color[i : i + 2], 16) for i in (0, 2, 4))
                got = img.color(x, y)
                ok = close(got, expected, args.tol)
                failures += 0 if ok else 1
                print(
                    f"{'ok  ' if ok else 'FAILURE'} {x},{y} = {to_hex(got)} "
                    f"(expected {to_hex(expected)} ±{args.tol})"
                )
            return 1 if failures else 0

        if args.scan is not None:
            step = ch or 31
            y = step // 2
            while y < img.h:
                print(f"  y={y:<4} {to_hex(img.color(args.scan, y))}")
                y += step
            return 0

        if args.row is not None:
            step = cw or 16
            x = step // 2
            while x < img.w:
                print(f"  x={x:<4} {to_hex(img.color(x, args.row))}")
                x += step
            return 0

        points = args.points or ["5,5"]
        for point in points:
            x, y = (int(v) for v in point.split(","))
            print(f"  {x},{y} = {to_hex(img.color(x, y))}")
        return 0
    except Error as e:
        print(f"pixel: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
