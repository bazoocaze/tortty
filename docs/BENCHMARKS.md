# Benchmarks and measurement

**This is the only file with throughput, parse, drawing and footprint numbers.**
The other docs state the fact in one sentence and link here. Every number below
has a date, a condition and a governor: a number without all three is not
comparable, and that is how two different terminals become an honest table.

## Conventions

- **Condition pinned in cells, not pixels.** `tools/benchrun.py` starts the app
  with the debug socket, asks for `resize` until `state` confirms the target
  grid, and only then types the load. If the grid does not match, it exits with
  code 3 and prints no number: a measurement from another condition is noise
  dressed up as a result.
- **The load is the same file in both terminals.** `tools/doomfire.py` is a
  byte-for-byte copy of the generator from the Python repo (same `sha1`), so
  what is measured is the consumer.
- **No vsync to measure throughput.** The app accepts `--no-vsync`; with vsync
  the number is the monitor's (60–144 Hz), which is the correct behaviour in
  use. This is recorded in the result, not hidden.
- **One terminal at a time**, no other window animating, `schedutil`.
- **Median of 3 shots** of 1200 frames (parsebench's `best`/`average` is best of
  N passes, and it says so on the line).

## Machine

| | |
|---|---|
| CPU | Ryzen 7 4800H, 16 threads, governor `schedutil` |
| GPU | Radeon Renoir integrated, RADV (Vulkan 1.4) |
| Session | X11 `:0`, no compositor redirecting |
| RAM | 22 GB, more than 10 GB free |
| Date | 2026-11-01 |

Frequency sampled during the measurement (every 200 ms, peak across all cores):
Rust 3.66–3.95 GHz average / 4.29 GHz peak; Python 4.11–4.21 GHz average / 4.29
GHz peak. That is: **Rust measured at a lower clock and won anyway.**

`scaling_governor` matters: under `powersave` the number halves and the
comparison lies. Check
`cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor` before believing
any line below.

## Main load: doom-fire

`doomfire.py` paints the whole screen per frame by writing, per cell,
`ESC[38;2;r;g;bm ESC[48;2;r;g;bm ▀` — two truecolor and one half block per
cell. It is a worst-case load on purpose: truecolor SGR is the most expensive
escape, and `▀` makes every cell need two colors.

### How to run

```sh
tools/benchrun.py                                   # Rust, 1200 frames, 61x18
tools/benchrun.py --app ~/tortty-python/run.sh   # Python reference
tools/bench.sh                                      # the whole ladder, both grids
tools/bench.sh --frames 300 --shots 1 --so-rust     # smoke
```

One line of `benchrun.py`:

```
app rust
grade 61x18
fps 968.2
mbs 41.6 MB/s
bpf 42286
draw 0.28 ms/call (2339 chamadas)
parse 0.13 ms/call (2325 chamadas)
pty 0.31 ms/call (5367 chamadas)
cell [14.131470680236816, 27.323135375976562]
ghz 3.66/4.29
```

`draw` is the time inside `RedrawRequested` (building instances + submitting);
`parse` is `Term::feed`; `pty` is only reading the fd (parsing is on the main
thread, on purpose: one core does both and the frame does not wait for a lock).

### The number (doom-fire, end to end, 1200 frames)

Measurement of 2026-11-02, after F1 (same machine, same protocol, same cell
width). Today's `draw` column includes the new decoration (per-style
underline/sub-underline, strikeout, overline) and did not change.

| grid | terminal | fps | MB/s | `draw` per call |
|---|---|---|---|---|
| 61x18 | **Rust** | **970–1005** | **41.7–43.2** | 0.29–0.33 ms |
| 61x18 | Python (post-P20) | 112–143 | 4.8–6.1 | 8.0–8.2 ms |
| 70x21 | **Rust** | **658–732** | **37.5–41.7** | 0.32–0.34 ms |
| 70x21 | Python (post-P20) | 63–72 | 3.6–4.1 | 13.1–13.4 ms |

Gain: **7–9x (61x18) and 9–11x (70x21)** in fps and MB/s; **25–46x** in drawing.

Previous measurement (2026-11-01, before F1), as a drift reference: Rust
917–968 fps / 39.4–41.6 MB/s at 61x18 and 629–667 / 35.8–38.0 at 70x21. The
difference is machine noise, not a drawing change.

Cell measured in both: 13.9–14.1 x 27.3–28.0 px (13 pt font, same effective
DPI). The pt→px conversion in `swash` matches Pango's within 2.4% in height —
which is why the comparison in cells is fair, and the 2% difference in pixels
per cell does not explain 9x.

### Drawing gate: draw calls (G16/B28/G15, 2026-11-02)

Before the gate the app drew once per pty read; now at most once every 16 ms
(and none with `?2026` open, 200 ms ceiling). Same load (900 doom-fire frames,
61x18, `--no-vsync`), 5 shots:

| | fps | MB/s | `draw` per call | draw calls |
|---|---|---|---|---|
| before | 965–1014 | 41.4–43.5 | 0.28–0.29 ms | **1499–1585** |
| after | 1012–1032 | 43.4–44.3 | 0.63–0.70 ms | **63–74** |

fps does not change (the bottleneck is the generator/pty); what changes is the
**total cost** of drawing on the same load: before `1580 x 0.28 = 442 ms`, after
`64 x 0.68 = 44 ms` — **10x less CPU drawing**, because each frame carries more
damage. The lower fps seen in one shot (847) was machine load (`load 2.4`), not
a regression.

### The ceiling is now the generator

Doom-fire alone does **~55 MB/s** (756 fps at 80x24, measured in the Python
repo). At 42–43 MB/s Rust is at ~78% of the load's ceiling. That is why fps
grows 9x but MB/s does not grow 9x: the 61x18 screen has no more cells to
consume; what is left is the Python in the generator. Measuring faster than that
requires a generator in C or reusing a buffer — and then you are measuring the
generator, not the terminal.

## Parser in isolation (`parsebench`, no display)

Same doom-fire frame, best of 800 runs, 35 B cell:

| parser | µs/cell | MB/s | ns/escape |
|---|---|---|---|
| **Rust** | **0.183** (average 0.199) | 180–195 | 99 |
| Python post-P20 (the production one) | 5.49 | ~7 | — |
| Python pre-P20 | 35.0 | ~1 | — |
| kitty (reference of the Python benchmark) | ~1.0 | ~10 | — |

**30x** over the production Python; **191x** over Python without the fast path.
kitty is in the table as a reference of magnitude — its number was measured in
the Python repo, with drawing included, so it is not the same ruler.

Why Python cost 35 µs: 91% was truecolor SGR (closing the CSI, `split(';')`,
`int()`, building lists). In Rust truecolor SGR is a function that walks the
buffer and writes three `u8` — no allocation, no `String`, no `dict`.

## Real load: `dmesg`

1397 lines (the system's `dmesg`), 100x40 grid, best of 20 passes:

| payload | Rust | Python | gain |
|---|---|---|---|
| no color (109 KB) | **2.11 ms** (51.7 MB/s, 1.5 µs/line) | 42.6 ms | 20x |
| `--color=always` (134 KB) | **2.22 ms** (60.2 MB/s, 1.6 µs/line) | 78.6 ms | 35x |

It is this pair that explains the Python UI "freezing" while a log is dumped:
42–79 ms of work on the UI thread, several times a second, against 2.2 ms. In
Rust the whole `dmesg` costs less than one 60 Hz frame.

## Footprint

| | Rust | Python |
|---|---|---|
| binary / installation | **10.1 MB**, one file | ~6.2 thousand lines + GTK3, Pango, cairo, PyGObject, fontconfig |
| system dependencies | `libc`, `libm`, `libgcc_s` (+ graphics driver at runtime) | the full GTK stack |
| startup until the window opens and runs the command | **0.18 s** | ~0.6–1.2 s (imports `gi`, registers Pango) |
| `cargo build --release` from scratch | ~2 min (most of it is `wgpu`) | — |

## Protocol (to compare two terminals)

1. `cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor` — write it down.
2. Close whatever animates (browser, video, another terminal running `yes`).
3. `tools/benchrun.py --frames 1200` and the same with `--app <other>`.
4. If the grid does not match, **do not compare**: `benchrun` exits with code 3.
5. Run 3 times and compare medians, not the best shot.
6. If comparing against Python, measure the same `--font-size` and the same
   grid.

## Pitfalls

- **Governor**: `powersave` halves the number. It was the "anomaly" that cost a
  day in the Python project.
- **Vsync**: measuring without `--no-vsync` measures the monitor. The app
  default is vsync (the right thing in use); the bench passes `--no-vsync` and
  says so in the output.
- **Grid**: without pinning by `resize` + checking in `state`, the WM hands out
  another grid and the comparison becomes apples and oranges.
- **Generator**: above ~55 MB/s you are measuring `doomfire.py`.
- **Frequency**: `schedutil` gives 3.6–4.3 GHz depending on heat. That is why
  `benchrun` samples and prints `ghz average/peak`; a shot at half clock is not
  a shot.
- **Compositor**: with a redirecting compositor, `draw` also measures buffer
  copy. Note whether the session has a compositor.
- **First frame**: `benchrun` waits for the socket and the grid before typing,
  but the first second still has shader/font coming up. The 1200 frames cover
  that; 100 frames do not.
