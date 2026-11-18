# Test fonts

Four subsets, 52 KB together, that the glyph tests of `crates/app/src/glyph.rs`
load **from this directory** instead of from the machine. `include_bytes!` sits
inside `#[cfg(test)]`, so nothing here reaches a release binary.

Why: the tests are assertions about the fallback chain, and a runner that has no
CJK face fails them for being empty and not for being wrong. Installing the real
ones costs `fonts-noto-cjk` = **61.2 MB**, which the Ubuntu mirror served at
59 kB/s on 2026-11-17 and held the `test` CI job for **18 minutes** — the whole
run, since the other jobs take 30-80 s. A fixture removes the download and, at
the same time, the machine from the measurement.

| file | family | carries | size |
|---|---|---|---|
| `DejaVuSansMono-subset.ttf` | DejaVu Sans Mono | ASCII, box drawing U+2500..257F | 14 KB |
| `DejaVuSans-subset.ttf` | DejaVu Sans | braille U+2800..28FF, U+1F600 mapping with no outline | 14 KB |
| `NotoSansCJKsc-subset.otf` | Noto Sans CJK SC | 日本語 | 1.9 KB |
| `NotoColorEmoji-subset.ttf` | Noto Color Emoji | 😀 🔥 🚀 👍 as CBDT bitmaps | 12 KB |

The two DejaVu faces are two different jobs on purpose: the mono one is the
primary the tests draw with, the proportional one is the earlier face in the
chain that **maps** an emoji and draws nothing — the bug that made emoji blank
on screen.

## Regenerating

`fonttools` (`pyftsubset`) is the only tool. The sizes above are what these
commands produce on Noto 2.004/Droid-era packages; a newer upstream font moves
them by a few KB, nothing else.

```sh
pyftsubset DejaVuSansMono.ttf --unicodes="U+0020-007E,U+2500-257F" \
  --no-hinting --layout-features='' --output-file=DejaVuSansMono-subset.ttf
pyftsubset DejaVuSans.ttf --unicodes="U+2800-28FF,U+1F600" \
  --no-hinting --layout-features='' --output-file=DejaVuSans-subset.ttf
pyftsubset NotoSansCJK-Regular.ttc --font-number=2 \
  --unicodes="U+65E5,U+672C,U+8A9E" --no-hinting --desubroutinize \
  --drop-tables+=GSUB,GPOS,GDEF --output-file=NotoSansCJKsc-subset.otf
pyftsubset NotoColorEmoji.ttf --unicodes="U+1F600,U+1F525,U+1F680,U+1F44D" \
  --no-hinting --drop-tables+=GSUB,GPOS,GDEF --output-file=NotoColorEmoji-subset.ttf
```

`--font-number=2` is the `Noto Sans CJK SC` face inside the TTC (0 is JP). The
name tables are left alone: see `LICENSE` for why that is allowed.
