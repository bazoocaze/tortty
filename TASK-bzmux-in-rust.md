# TASK — bzmux in Rust (feasibility experiment)

Progress document for this experiment. Source analyzed:
`/home/user/tortty-python` (pure Python, GTK3 + cairo + Pango).
Target here: `rust_term/` (Cargo workspace).

Status: **in progress**. Every section marked `[x]` was measured, not assumed.

---

## 1. Objective

Answer two questions with numbers, not opinions:

1. Does win/lin/mac **distribution** get easy in Rust?
2. Does **performance** get better than Python (which today does ~114 fps / 4.9 MB/s
   on doom-fire at 61×18 on the real display, and ~700 fps in kitty)?

Success condition of the experiment (the "go/no-go"):

- [ ] Rust project that opens a window, runs a shell on a pty and draws the grid;
- [ ] runs doom-fire in a comparable way (same grid, same generator);
- [ ] headless benchmark of the parser (µs/cell) and end to end (fps, MB/s);
- [ ] works in `vim`, `mc` and console flow (`dmesg`) — a usefulness requirement,
      not a performance one;
- [ ] debug socket with the same one-JSON-line-per-request protocol (instrumentation/tests);
- [ ] side-by-side comparison with the Python bzmux on the same machine and display.

Out of scope for this phase: tabs/workspaces, kitty protocol images, config file,
clipboard/selection, mouse reporting, reflow, kitty keyboard protocol, real
Windows/macOS (only the architectural sketch).

---

## 2. Analysis of the source project

### 2.1 What was measured there (docs/BENCHMARKS.md)

- Main load: `tools/doomfire.py`. Intensity buffer 0..36 propagated from the
  bottom up; drawing in a half block `▀` (two pixels per cell), with
  `38;2;r;g;b` + `48;2;r;g;b` truecolor **per cell**, without grouping. It is the
  worst case for the parser and for drawing: ~34–38 escape bytes per cell.
- End-to-end number for Python (2026-11-01, `:0` with glamor, 61×18, 1200 frames):
  **113.9 fps / 4.9 MB/s / 42.8 KB per frame / draw 13.9 ms**. On Xvfb: 129 fps and
  draw 4.2 ms (the display changes the number by 3.4×).
- External reference: kitty 0.48.2 at 70×21 does 676–714 fps / 38–41 MB/s. Python
  does 5.2 µs per cell; kitty 1 µs.
- Decomposition of the parse in Python: **35 µs/cell**, of which **91% is truecolor
  SGR** (closing the CSI, `split(';')`, `int()`, building lists). P20 brought in a
  fast path with regex + `escape → color` cache and it dropped to 5.5 µs/cell (5.8×).
- Decomposition of drawing: ~5.7 µs per color run; Pango `show_layout` 43%,
  background+state (4 cairo context calls per run) 27%.
- Central discovery of the last adjustment: **the expensive part is alternating
  background and glyph** (13.0 ms → 7.9 ms just by grouping into two passes), because
  each operation changes state on the X server. A renderer per instance (GPU)
  eliminates the alternation by construction.
- Isolated parse (no GTK) of `dmesg` (1319 lines): 42.6 ms without color / 78.6 ms with
  color. That is what freezes the UI when a log is dumped on the GTK thread.

### 2.2 What is indispensable for the benchmark and for vim/mc/dmesg

From the Python code (`bzmux/term.py` 1126 lines, `view.py`, `palette.py`):

| area | what has to exist | why |
|---|---|---|
| grid | `Cell {ch, fg, bg, flags}`, cursor, autowrap, `row_wrapped` | the basis of everything |
| SGR | 16/256/truecolor, `38:2::r:g:b` subparameter, bold/underline/inverse | the whole doom-fire; vim with `termguicolors` |
| editing CSI | ED/EL, IL/DL, ICH/DCH/ECH, DECSTBM (scroll region), CUP/CUU/CUD/CUF/CUB/CHA/VPA | vim, mc redraw with these |
| modes | `?1` DECCKM (arrows `\EOA`), `?25` cursor, `?7` autowrap, `?47/1047/1049` alt-screen (+ clearing scrollback on 1049), `?1000/1002/1003/1006` mouse, `?1004`, `?2004`, `?2026` | mc and vim live on alt-screen + arrows in application mode |
| DEC charset | `ESC ( 0` → line graphics (`lqk x m j`) | **that is how mc draws its frames**; without it you get `lqqqqk` |
| queries | DA1 (`\e[?62c`), DA2 (`\e[>1;100;0c`), DSR/CPR (`\e[6n`), DECRQM (`$p`), DECSCUSR (`6 q`), window ops 16/18 (cell in px) | TUI apps wait for an answer and hang at startup without it |
| OSC | title (0/2), consume 4/8/10/11/52 without breaking | shell, vim, `dmesg` |
| scrollback | history with a cap, `view_off` (mouse wheel) | console flow |
| pty | `openpty` + `fork/exec` + `TIOCSWINSZ` + non-blocking read | — |
| keyboard | keyval → bytes, DECCKM, Alt as ESC prefix, Shift+Tab `\e[Z`, Ctrl+letter | vim/mc |

What is **not** needed in this phase: kitty images (`graphics.py`), tabs/workspaces
(`window.py`), reflow (`_reflow`), selection/clipboard, config file
(`config.py`), kitty keyboard protocol (`kitty_kbd.py`), shortcuts and chrome.

### 2.3 Debug socket protocol (replicate exactly)

`bzmux/debugsock.py`: Unix socket, **one JSON line per request and per response**,
`{"ok":true,...}` or `{"ok":false,"err":...}`, same uid only
(`SO_PEERCRED`), `0600`. Verbs: `state`, `text`, `write`, `keys`, `resize`,
`prof`, `log`, `quit`. `--debug-socket auto` = `$XDG_RUNTIME_DIR/bzmux-<pid>.sock`.

`tools/benchrun.py` depends on that: it starts the app, asks for the grid, uses
`resize` until it hits the target (61×18), and **only then** types the load into the
pty with `write`. Keeping the protocol the same lets the same harness be reused on
both terminals — which is what makes the comparison honest.

---

## 3. Answers to the user's questions

### 3.1 Does win/lin/mac distribution get easy? — **Yes, substantially.**

What Python carries today (`docs/PORTABILITY.md`): PyGObject **has no wheel
anywhere** (only sdist → compile against headers), pycairo has no macOS wheel,
Pango will never be pip-installable, and Qt has a system dependency on Linux
(`libxcb-cursor0`). Result: installing = conda (~350–400 MB), or praying the
desktop has GTK.

In Rust the chosen stack is all native and free of system dependencies:

- window/events: `winit` → native X11 and Wayland, Win32, Cocoa;
- GPU: `wgpu` → Vulkan/DX12/Metal (and GL as fallback) with no per-platform code;
- font: `fontdb` + `swash` — we rasterize the glyph in process. **No freetype,
  no fontconfig, no Pango**;
- pty: `portable-pty` → Unix pty and **ConPTY** on Windows (that is why this crate
  was chosen instead of raw `openpty`);
- text: `unicode-width`; no ICU.

Concrete consequence: a single binary of ~5–15 MB per target, produced by CI
(`cargo build --target x86_64-pc-windows-msvc`, `aarch64-apple-darwin`, etc.).

Caveats that do **not** go away (that is why the answer is "yes, but"):

1. **macOS signing/notarization**: an unsigned binary opens with a Gatekeeper
   warning; clean distribution requires an Apple account (US$ 99/year) + `codesign` +
   `notarytool`. Rust does not change that — it is OS policy.
2. **Windows SmartScreen** likewise for a new binary with no reputation.
3. **Linux**: X11/Wayland ok, but the "runs on any distro" claim calls for a build
   against an old glibc (or musl) and a graphics driver dependency; Vulkan does not
   exist on an old machine → the GL fallback is needed (`wgpu` covers it, but it is a
   path to be tested).
4. Fonts: since we do not use fontconfig, on Windows/macOS we read the system fonts;
   an embedded fallback solves a machine with no decent monospace font.

Honest comparison: Python→Rust trades "carrying 350 MB of GTK stack" for "carrying a
binary", and trades three hand-written window backends for one crate. What
**remains** work: signing, CI matrix, and testing each platform.

### 3.2 Does performance get better than Python? — **Yes, and the difference is structural.**

It is not generic "Rust is faster": both bottlenecks measured in Python
disappear by construction.

| bottleneck measured in Python | cost today | why it drops in Rust |
|---|---|---|
| closing and parsing truecolor CSI | 32 of 35 µs/cell | parsing is O(bytes) with no allocation: `split` over a slice + `u32` on the stack. Order of magnitude: tens of **ns** per escape |
| alternating background `fill` and `show_layout` per run (state change on the X server) | 13.0 ms/frame; drops to 7.9 just by grouping | a **single** instanced draw call: background and glyph become the same pipeline, same pass, order guaranteed per instance |
| per color-run shaping/layout | 43% of draw | no shaping per frame: the glyph is already in the atlas, the frame just builds vertices |
| Python cell (list of 4 objects) + `Vec` of lists | ~100 ns per access, GC | `#[derive(Copy)] Cell` in a contiguous `Vec<Row>` |

Estimate (marked as an estimate): parse of 35 µs/cell (Python without P20) →
~0.02–0.05 µs/cell; with P20 (5.5 µs) → ~0.02–0.05 µs as well, because in Rust
the fast path becomes the normal path. Drawing from 13.9 ms/frame → building ~2200
instances (~30 µs) + GPU. That is **>10× on the parse and >50× on drawing**, which
moves the system limit from "Python drawing" to "monitor vsync".

Two caveats worth more than the fps:

1. In real use the fps is limited by vsync (60–144 Hz). The visible gain is
   **latency** and **CPU headroom** (dumping `dmesg` no longer freezes the UI),
   not the doom-fire number.
2. `wgpu`/GPU pays a fixed cost per frame. On a small grid the bottleneck becomes
   the generator; that is why the comparison has to be on the same grid and with the
   same `tools/doomfire.py` — otherwise you measure the generator, not the terminal.

### 3.3 Verdict

Both answers are "yes". **The experiment continues**, with one honesty condition:
the numbers in section 7 are measured here and now, on the same machine, same
display and same grid as Python; if Rust does not win clearly, the document says so.

### 3.4 Decisions taken (user, 2026-11-01)

1. **Route C — full Rust.** Not the PyO3 hybrid, not optimizing Python.
2. **New repo** (not inside `tortty-python`).
3. **Linux only in this phase**; `portable-pty` already chosen for the Windows day
   (ConPTY), but no CI for other platforms now.
4. **Vsync on by default**, `--no-vsync` for benchmarking.
5. **Font in points**, a metric equivalent to Pango — but no hack: if the
   pt→px conversion of `swash` diverges from Pango, the difference is recorded and
   the comparison stays locked to cells.
6. **No reflow in phase 1** (vim/mc redraw on SIGWINCH; a history that breaks when
   narrowing is an accepted and documented effect).
7. **Abort criterion:** if Rust does not reach **≥3× the MB/s of Python on the same
   grid** and does not open `vim`/`mc` correctly, the experiment does not pay the
   migration cost and the recommendation becomes route B (Rust core + Python UI).

## 3.5 Distribution: what "easy" means in practice

User's question: can you copy the executable and run it on the Mac? Is it a paid
store? Does a GitHub Release work?

- **Copying between systems does not**: a Linux binary is ELF; macOS is Mach-O; Windows
  is PE. "Copy and run" holds within the same system (and that is where Rust beats
  Python: a self-contained binary, with no GTK/Python/fontconfig). For the Mac you have to
  compile **for** the Mac.
- **How to compile for the Mac**: runners `macos-14`/`macos-15` (arm64) and `macos-13`
  (x86_64) on GitHub Actions; `lipo` joins them into a universal binary. Cross-compiling
  from Linux with `osxcross` is possible and fragile (Apple SDK) — not the path.
- **Gatekeeper**: the block comes from the `com.apple.quarantine` attribute, which the
  **browser** sets on download. `curl`, `git clone` and `brew` do not set it → it runs
  directly. Downloading a non-notarized zip through Safari/Chrome is blocked, and on
  macOS 15+ the old "right click → Open" was removed (the user has to go to
  Settings → Privacy and Security → "Open Anyway").

- **What is paid**: it is not a store, it is the **Apple Developer Program (US$ 99/year)**,
  which issues the *Developer ID Application* certificate needed to **notarize**.
  Notarization itself is free; the account is not. That alone removes the warning on a
  browser download. App Store is irrelevant (the sandbox breaks pty).
- **Homebrew tap** is the friendly route on the Mac: free, installs through `curl` (no
  quarantine), gives you `brew upgrade`.
- **Windows**: an unsigned `.exe` triggers the SmartScreen warning ("More information
  → Run anyway"); an OV/EV certificate today requires a token/HSM.
- **Conclusion**: a GitHub Release works on all three. Linux perfect; macOS clean
  via `curl`/Homebrew (US$ 99/year only to open without a warning coming from the browser);
  Windows with the SmartScreen warning. No per-download fee, no store.
- **Packaging plan** (future phase, not this one): CI matrix with
  `bzmux-linux-x86_64`, `bzmux-macos-arm64`, `bzmux-macos-x86_64`, universal via
  `lipo`, `bzmux-windows-x86_64.exe` + `SHA256SUMS` in the Release.

---

## 4. Chosen architecture

```
rust_term/
├── TASK-bzmux-in-rust.md      (this file)
├── Cargo.toml                 workspace
├── crates/bzmux-core/         pure emulation: cell, palette, parser, grid
│   ├── src/{lib,color,cell,grid,charset,term}.rs
│   └── src/bin/parsebench.rs  headless parser benchmark (µs/cell)
├── crates/bzmux/              app: window, GPU, pty, keyboard, debug socket
│   └── src/{main,cli,app,keys,pty,prof,debugsock,render,glyph,atlas}.rs
└── tools/                     doomfire.py, benchrun.py, bench.sh (comparison)
```

A rule inherited from Python and kept: **`bzmux-core` does not know about the GUI** (not
winit, not wgpu, not pty). That is what lets `cargo test -p bzmux-core` run without a
display — and what guarantees the parser benchmark measures the parser.

Decisions with reasons:

- **Our own parser**, not `vte`/`alacritty_terminal`: bzmux was born from its own VT
  parser and compatibility (kitty/ghostty) is the product. A third-party dependency here
  would freeze exactly what we want to evolve.
- **GPU (`wgpu`) instead of cairo/softbuffer**: the measured bottleneck in Python is
  state change per draw operation. Instancing quads is the opposite of that.
- **A single pipeline** for background, glyph, underline and cursor: the atlas keeps one
  white pixel; the background is a "solid" quad and the glyph is a quad with UV into the
  coverage mask. One draw call per frame, blending order guaranteed by instance order.
- **On-demand glyph atlas** (`swash`): rasterizes once per (face, glyph, size), never
  per frame.
- **`portable-pty`**: it is what gives ConPTY for free; raw `openpty` would cost a whole
  implementation on the day the Windows target comes in.

---

## 5. Phases

- [x] 1. Analysis of the Python project (term/view/palette/debugsock/BENCHMARKS/PORTABILITY)
- [x] 2. Workspace + `bzmux-core`: cell, 256 palette, DEC charset, grid
- [x] 3. VT parser: ground/CSI/OSC/ESC, truecolor SGR, modes, queries
- [x] 4. Core tests (23 tests) + headless `parsebench`
- [x] 5. App: pty (`portable-pty`), window (`winit`), keyboard, resize
- [x] 6. GPU renderer (`wgpu`): glyph atlas (`swash`/`fontdb`) + instances
- [x] 7. Debug socket (same JSON protocol; verbs state/text/write/keys/resize/prof/log/quit)
- [x] 8. Run doom-fire and measure; compare with Python (section 7)
- [x] 9. Functional test vim / mc / dmesg
- [ ] 10. Conclusion and recommendation (keep migrating / rethink)

## 6. Measurement protocol (so the number is worth something)

- Same machine, same display (`:0`), one terminal at a time, governor
  recorded (`schedutil`).
- Grid **locked in cells** (61×18 and 70×21) via the debug socket `resize` verb,
  checked before typing the load — like the Python `tools/benchrun.py` does.
- Same generator (`tools/doomfire.py`, `--fps 0`) running **inside** each
  terminal: what is measured is the consumer, not the generator.
- Three numbers per run: fps (from the generator), MB/s, and the app's own internal
  cost (parse/draw per call).
- Isolated parser: `parsebench` with the exact doomfire frame, reporting µs/cell
  and MB/s, repeating until it stabilizes (best of N).
- Old numbers from another day/governor/grid do not go into the table.

---

## 7. Results

Measured on this machine (Ryzen 7 4800H, Radeon Renoir/RADV, X11 `:0`), governor
`schedutil` at 4.1–4.3 GHz, one terminal at a time, 2026-11-01. Measured cell:
Python 14x28 px, Rust 14.13x27.32 px (same 13 pt font, same effective DPI —
the pt→px conversion of `swash` matches Pango's within 2.4% in height).

### 7.1 Headless parser (`parsebench`, no GUI, no display)

Exact doom-fire frame (61x18, 1098 cells, 35 B/cell = 2 truecolor SGR +
`▀` per cell), best of 800 runs:

| parser | µs/cell | MB/s | ns/escape |
|---|---|---|---|
| Rust (this one) | **0.183** (mean 0.199) | 180–195 | 99 |
| Python post-P20 (the production one) | 5.49 | ~7 | — |
| Python pre-P20 | 35.0 | ~1 | — |

Gain: **30x** over production Python, **191x** over Python without the fast
path. The kitty reference (measured in the end-to-end benchmark, with drawing) is ~1.0
µs/cell.

Real load, the system's `dmesg` (1397 lines, 109 KB without color / 134 KB with color),
100x40 grid, best of 20 passes:

| payload | Rust | Python | gain |
|---|---|---|---|
| `dmesg` without color | **2.11 ms** (51.7 MB/s) | 42.6 ms | 20x |
| `dmesg --color=always` | **2.22 ms** (60.2 MB/s) | 78.6 ms | 35x |

It is this number that explains the Python UI "freezing" when a log is dumped: 42–79 ms
of work on the UI thread versus 2.2 ms.

### 7.2 End to end (doom-fire inside the pty, 1200 frames)

Command: `tools/benchrun.py` (same protocol: socket, `resize` until the grid
matches, check, then type the load). Rust with `--no-vsync` — with vsync the
number would be the monitor's, not the terminal's.

| grid | terminal | fps | MB/s | `draw` per call |
|---|---|---|---|---|
| 61x18 | Rust | **916.6–968.2** | **39.4–41.6** | 0.25–0.28 ms |
| 61x18 | Python | 113.2–120.9 | 4.9–5.2 | 7.77–8.11 ms |
| 70x21 | Rust | **628.6–666.9** | **35.8–38.0** | 0.28–0.30 ms |
| 70x21 | Python | 62.3–70.2 | 3.6–4.0 | 13.58–13.73 ms |

- **8.0–9.0x in fps and MB/s at 61x18**; **9.5x at 70x21**. The abort criterion
  (≥3x) was beaten with room to spare.
- Drawing dropped **29x** (61x18) and **46x** (70x21): 7.8–13.7 ms → 0.25–0.30 ms.
  The cause is the one the Python BENCHMARKS already pointed at: there is no longer any
  "alternating background `fill` and `show_layout` per run" — background, glyph, underline and
  cursor are instances of the same pipeline, in one draw call per frame.
- **The limit is now the generator, not the terminal**: doom-fire alone does ~55
  MB/s (756 fps at 80x24, measured in the Python project). At 39–42 MB/s Rust is at
  ~75% of the load's ceiling. fps grows 9x but MB/s does not grow 9x for the same reason —
  the 61x18 screen has no more cells to consume.
- `parse` came out at 0.13–0.15 ms per call and `pty` (just the read) at 0.26–0.39
  ms; in Python `on_pty` (read + parse) was 0.28 ms. That is: the parse
  practically disappeared from the frame budget.

### 7.3 Functional (what was opened and checked on screen)

- **`mc`**: panels, frames in DEC Special Graphics (`lqk x m j` → `┌┐└┘│─`),
  colors, F-keys bar, alt-screen and arrows in application mode (DECCKM).
- **`vim`**: real file with `-u NONE`, `~` titles, status line, block
  cursor, navigation with arrows/Home/End through the real keyboard path
  (XTest → winit → encoder → pty).
- **`dmesg --color=always`**: colors, scrolling, 287 lines of history, mouse wheel
  going up in the scrollback (`view_off` = 12 lines after 6 clicks).
- **Truecolor per cell**, bold and SGR 256 checked by screen capture.
- **Debug socket**: `state`, `text`, `write`, `keys`, `resize`, `prof`, `log`,
  `quit` — the same measurement harness from Python runs on both.

### 7.4 Footprint

- Release binary: **10.1 MB**, linked only against `libc`, `libm`, `libgcc_s` (+ the
  graphics driver at runtime). No GTK, no Pango, no freetype, no fontconfig,
  no Python.
- Startup until the window opens and the command runs: **0.18 s**.
- `cargo build --release` from scratch: ~2 min (most of it is `wgpu`).

## 8. Risks and what is deliberately left pending

Pending in this phase (none of it is a requirement of the experiment):

- no reflow on resize (decision 6); the history "breaks" when narrowing, and
  vim/mc redraw on SIGWINCH;
- no tabs/workspaces, kitty protocol images, selection/clipboard, mouse
  reporting (`?1000/1002/1003/1006` are already read and stored, but do not emit),
  OSC 8 links, config file, kitty keyboard protocol (`CSI u`);
- `IM` (insert mode) and `LNM` ignored; combining marks are not composed (same
  limitation as Python);
- `ScaleFactorChanged` at runtime does not recreate the font (only at startup).

Measured or known risks:

- Vulkan missing on an old machine → the `wgpu` GL backend remains to be exercised
  (`WGPU_BACKEND=gl` exists as an escape valve, it was not tested);
- the environment variable `WGPU_BACKEND`/`WGPU_ADAPTER_NAME` lets you force
  llvmpipe; a test on a software renderer is the next portability step;
- font: since there is no fontconfig, the choice is a preference list + generic
  fallback. On a machine with no monospace font at all the app exits with a message;
- `--no-vsync` is mandatory to measure throughput: with vsync the number is the
  monitor's (60–144), which is the correct behavior in use.

Phase conclusion (to be filled in item 10 after the user's review).
