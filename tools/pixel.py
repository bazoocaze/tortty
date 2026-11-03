#!/usr/bin/env python3
"""Lê e compara pixels de uma captura de tela — para provar o que foi desenhado.

Espelha o `tools/pixel.py` do bzmux Python, sem depender de `gi`: a captura é
decodificada pelo `convert` (ImageMagick), que já é requisito do `import` usado
para capturar. Assim o mesmo script roda aqui, no CI e por ssh.

    tools/pixel.py shot.png                 # amostra padrão
    tools/pixel.py shot.png 100,50 200,50   # pontos (x,y) específicos
    tools/pixel.py shot.png --scan 300      # coluna x=300, uma amostra por célula
    tools/pixel.py shot.png --row 515       # linha y=515 amostrada na largura
    tools/pixel.py --expect 100,50=ff0000   # confere a cor (± --tol)
    tools/pixel.py --diff a.png b.png       # quantos pixels mudaram e onde
    tools/pixel.py --diff a.png b.png --max 0   # falha (exit 1) se algo mudou

`--diff` é o que transforma captura em **teste**: páginas de conteúdo igual têm
de dar 0 pixel diferente; conteúdo novo tem de dar diferença — e é assim que se
prova que a janela está desenhando de verdade (surface morto = 0 diffs).
"""

import argparse
import subprocess
import sys
from dataclasses import dataclass


class Erro(Exception):
    pass


@dataclass
class Img:
    w: int
    h: int
    px: bytes

    def cor(self, x: int, y: int) -> tuple[int, int, int]:
        if not (0 <= x < self.w and 0 <= y < self.h):
            raise Erro(f"ponto fora da imagem: {x},{y} ({self.w}x{self.h})")
        o = (y * self.w + x) * 3
        return self.px[o], self.px[o + 1], self.px[o + 2]


def carregar(caminho: str) -> Img:
    """PNG (ou qualquer coisa que o ImageMagick leia) para RGB cru, via PPM."""
    try:
        out = subprocess.run(
            ["convert", caminho, "-depth", "8", "ppm:-"],
            capture_output=True,
            check=True,
        ).stdout
    except FileNotFoundError:
        raise Erro("convert não encontrado (ImageMagick)")
    except subprocess.CalledProcessError as e:
        raise Erro(f"não consegui ler {caminho}: {e.stderr.decode().strip()}")
    # P6 <larg> <alt> <max>\n<bytes>: o cabeçalho é ASCII com comentários (`#`)
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
        raise Erro(f"formato inesperado: {tokens[0]!r} (esperado P6)")
    w, h, maxval = (int(t) for t in tokens[1:4])
    if maxval != 255:
        raise Erro(f"profundidade {maxval}: esperado 8 bits por canal")
    dados = out[i + 1 : i + 1 + w * h * 3]
    if len(dados) != w * h * 3:
        raise Erro(f"PPM truncado: {len(dados)} bytes para {w}x{h}")
    return Img(w, h, dados)


def hexa(c: tuple[int, int, int]) -> str:
    return "%02x%02x%02x" % c


def perto(a: tuple[int, int, int], b: tuple[int, int, int], tol: int) -> bool:
    return all(abs(x - y) <= tol for x, y in zip(a, b))


def diffs(a: Img, b: Img, tol: int) -> tuple[int, tuple[int, int, int, int] | None]:
    """Pixels diferentes e o retângulo que os contém."""
    if (a.w, a.h) != (b.w, b.h):
        raise Erro(f"tamanhos diferentes: {a.w}x{a.h} e {b.w}x{b.h}")
    n = 0
    x0 = y0 = 10**9
    x1 = y1 = -1
    for y in range(a.h):
        for x in range(a.w):
            if not perto(a.cor(x, y), b.cor(x, y), tol):
                n += 1
                x0, y0, x1, y1 = min(x0, x), min(y0, y), max(x1, x), max(y1, y)
    return n, (None if n == 0 else (x0, y0, x1 - x0 + 1, y1 - y0 + 1))


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("png", nargs="?", help="captura a ler")
    ap.add_argument("pontos", nargs="*", help="x,y (repetível)")
    ap.add_argument("--scan", type=int, metavar="X", help="coluna X, uma amostra por célula")
    ap.add_argument("--row", type=int, metavar="Y", help="linha Y amostrada na largura")
    ap.add_argument("--cell", metavar="LxA", default="0x0", help="tamanho da célula (ex. 16x31)")
    ap.add_argument("--expect", action="append", default=[], metavar="X,Y=RRGGBB")
    ap.add_argument("--tol", type=int, default=8, help="tolerância por canal (padrão 8)")
    ap.add_argument("--diff", nargs=2, metavar=("A", "B"), help="compara duas capturas")
    ap.add_argument("--max", type=int, default=None, help="máximo de pixels diferentes")
    args = ap.parse_args(argv)

    try:
        cw, ch = (int(v) for v in args.cell.split("x"))

        if args.diff:
            a, b = carregar(args.diff[0]), carregar(args.diff[1])
            n, caixa = diffs(a, b, args.tol)
            pct = 100.0 * n / (a.w * a.h)
            print(f"{n} pixel(s) diferente(s) ({pct:.2f}%)", end="")
            if caixa:
                print(f" caixa {caixa[0]},{caixa[1]} {caixa[2]}x{caixa[3]}")
            else:
                print()
            if args.max is not None and n > args.max:
                print(f"FALHA: máximo era {args.max}")
                return 1
            return 0

        if not args.png:
            raise Erro("informe a captura (ou --diff A B)")
        img = carregar(args.png)
        print(f"{args.png}: {img.w}x{img.h}")

        if args.expect:
            falhas = 0
            for expr in args.expect:
                ponto, cor = expr.split("=")
                x, y = (int(v) for v in ponto.split(","))
                esperado = tuple(int(cor[i : i + 2], 16) for i in (0, 2, 4))
                obtido = img.cor(x, y)
                ok = perto(obtido, esperado, args.tol)
                falhas += 0 if ok else 1
                print(
                    f"{'ok  ' if ok else 'FALHA'} {x},{y} = {hexa(obtido)} "
                    f"(esperado {hexa(esperado)} ±{args.tol})"
                )
            return 1 if falhas else 0

        if args.scan is not None:
            passo = ch or 31
            y = passo // 2
            while y < img.h:
                print(f"  y={y:<4} {hexa(img.cor(args.scan, y))}")
                y += passo
            return 0

        if args.row is not None:
            passo = cw or 16
            x = passo // 2
            while x < img.w:
                print(f"  x={x:<4} {hexa(img.cor(x, args.row))}")
                x += passo
            return 0

        pontos = args.pontos or ["5,5"]
        for ponto in pontos:
            x, y = (int(v) for v in ponto.split(","))
            print(f"  {x},{y} = {hexa(img.cor(x, y))}")
        return 0
    except Erro as e:
        print(f"pixel: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
