# Development: tools, verification and pitfalls

Operational manual. **Read before running any graphical tool**: the first item
below already cost someone their work terminal.

## Never `pkill` — PID file, always

`pkill -f tortty` kills the **shell that called** the command, because the
pattern appears in its command line. `pkill -x` was the one that hurt: while two
builds shared the same binary name, it killed the installed app and not the dev
binary — that is how a `pkill` here brought down the user's terminal session. The
rename removed the collision, but the rule stands: nothing here kills by name.

The tools only kill a registered PID, checking `/proc/<pid>/exe` first:

```sh
./tools/devrun.sh --cmd 'mc' --cols 100 --rows 30 --debug-socket /tmp/tt.sock
./tools/devkill.sh            # kills only the PID from /tmp/tortty-rs.pid
./tools/devkill.sh --status   # shows life/exe without killing
```

`devrun.sh` refuses to start if a live process with that PID file already
exists (it avoids two instances fighting over the same socket).

## Probes (what you can turn on without recompiling)

| probe | effect |
|---|---|
| `--cmd 'CMD'` | runs `sh -c CMD` instead of `$SHELL` |
| `--cols N --rows N` | initial size in cells (the WM may adjust it) |
| `--font-family`, `--font-size PT`, `--dpi N` | font in points; `--dpi` is `Xft.dpi` |
| `--scrollback N` | cap on history lines |
| `--no-vsync` | draws as fast as possible (measure throughput) |
| `--prof` | turns on the `draw`/`parse`/`pty` counters (queried by `prof`) |
| `--debug-socket PATH\|auto` | JSON introspection socket (`auto` = `$XDG_RUNTIME_DIR/tortty-<pid>.sock`) |
| `--debug-log FILE` | event log (`logln!`): state dumps, toasts, config reloads and every `?2026` transaction |
| `WGPU_BACKEND=gl\|vulkan` | forces the `wgpu` backend (escape valve for a problematic GPU) |
| `WGPU_ADAPTER_NAME=…` | picks the adapter (useful to test llvmpipe) |
| `SHELL=…` | default shell when there is no `--cmd` |

### The `?2026` log: what a synchronized update did

`--debug-log` records every `?2026` (synchronized output) window the child
opens, one line per window, with the counters of what happened inside it:

```
log start: +0.0ms epoch 1794081658402 ms
tab 1 sync closed +4865.4ms after 0.0 ms — feeds 1, 0 B, view 0→0, hist 0 — ed0×0 ... el×18 home×0 stbm×0 alt×0 ris×0 decstr×0 hist+0
```

- `feeds 1, 0 B` — the window opened **and** closed inside one pty read, so the
  parser never held the screen and the duration is `0.0 ms`. `feeds > 1` means
  the app took several reads to finish the frame, and then the duration is the
  time it kept us waiting.
- `ed0..3`, `el`, `home`, `stbm`, `alt`, `ris`, `decstr` — the clears and resets
  the window carried: `ed2` erases the screen, `ed3` erases the scrollback too,
  `el` is the per-line erase a repaint uses, `home` is `CSI H` to `1;1`, `stbm`
  is a scroll region, `alt` an alt-screen switch, `ris`/`decstr` the two resets.
- `hist+N` — lines the window pushed into the scrollback. Each of them moves a
  scrolled-up view by one line (`view_after_output`), so this is the number to
  look at when the view seems to run away while an app repaints.
- `view A→B` — our view offset before and after the read.

The core counts and the app times: `SyncTrace` (`crates/core/src/term.rs`) has no
clock, and the app stamps the window when it first sees it open. A window that
never closes is logged once as `sync open` and then only at `state` (`"sync":
true`); a `RIS` inside it closes the trace with `ris×1` because a full reset ends
the DEC modes.

## Visual verification without stealing the owner's screen

Opening the window in the graphical session of whoever is working gets in the
way: it comes up on top and, with `xdotool`, steals focus. The visual
verification here runs on a display of its own — the window exists, but not on
anyone's screen:

```sh
./tools/render-check.sh            # Xvfb + fixed theme + capture + comparison; exits 0/1
./tools/render-check.sh --keep     # keeps the captures in /tmp/tortty-render
```

`tools/render-check.sh` owns that recipe (display, fake `HOME` with a
`tortty.conf` of the fixed theme, wait for the first frame, capture through
`state`/`xid`, and the four pixel checks via
[`tools/pixel-probe.py`](../tools/pixel-probe.py)). To probe a capture by hand:
`tools/pixel-probe.py shot.png 100,50` (colour of one point),
`--scan X`/`--row Y` (sample per cell) and `--diff a.png b.png` (how many pixels
changed and where).

Details that save time:

- **The `xid` comes from `state`**, not from `xdotool search --class tortty`: on
  a display with more than one instance the search picks the wrong window and
  the capture comes out empty.
- **A fake `HOME`** is what installs a test font or configuration without
  touching the real `~/.config`; a separate PID file keeps `devkill` from
  bringing down the instance the owner is using.
- **`import -window`/`xwd` do not need focus** — `xdotool` is the one that needs
  it, and on Xvfb it sends it to the window of the isolated display.
- **Without `/dev/dri` `wgpu` falls back to llvmpipe, and llvmpipe draws** under
  Xvfb (that is how emoji, ligature and damage were checked on 2026-11-02). It
  serves for content and geometry; a performance number is only valid on the
  GPU.

## Verification with a window (without relying on an eye)

```sh
./tools/devrun.sh --cmd 'mc' --cols 90 --rows 24 --debug-socket /tmp/tt.sock
PID=$(cat /tmp/tortty-rs.pid)
WID=$(wmctrl -lp | awk -v p=$PID '$3==p {print $1; exit}')
xwd -id "$WID" -out /tmp/x.xwd && convert /tmp/x.xwd /tmp/x.png   # look
xdotool windowactivate "$WID"; xdotool mousemove --window "$WID" 200 200
for i in $(seq 6); do xdotool click 4; done                        # mouse wheel
```

Then check **by state**, not by pixel: `state.scrollback`, `state.view_off` and
`text where=view` say whether the scroll happened. Pixel says something was
drawn; state says what.

For the keyboard, the path is the real one (`xdotool key` → XTest → winit →
encoder → pty), not the socket `keys`: `keys` tests the encoder, `xdotool` tests
the boundary.

## Introspection socket

Unix socket, one JSON request per line, one JSON response per line. `ok: true`
or `ok: false` with `err`. The protocol is stable on purpose:
`tools/benchrun.py` runs on both.

| verb | input | response |
|---|---|---|
| `state` | — | `ok` + `cols`, `rows`, `cursor` `[row,col]`, `title`, `view_off`, `scrolled`, `scrollback`, `alt`, `sync`, `cell` `[w,h]`, `font`, `font_px`, `tabs`, `workspaces`, `errors`, `draws`, **`drawn`**, **`skips`**, **`last_frame`**, `last_damage`, `frames_all`, `rows_max`, `all_reason`, `xid` |
| `text` | `where` (`view`\|`history`\|`screen`), `start`, `count` | `lines` (strings) |
| `write` | `data` (string) | `ok` (writes to the pty) |
| `keys` | `data` (spec like `ctrl+shift+t`, `f5`, `alt+left`) | `keys: [n]` bytes written to the pty |
| `resize` | `cols`, `rows` | `requested` (the grid **asked for**; check it in `state`) |
| `prof` | `on` (bool, optional) | counters: `draw_ms_call`, `draw_calls`, `parse_*`, `pty_*` |
| `osc` | `tab` (optional, default the active one) | the last 8 OSC the parser saw, newest to oldest: `hex`, `text` (lossy), `truncated`, `title` (the resulting one). It is the probe for "why is the tab title full of garbage" |
| `log` | `data` | writes to the debug log |
| `quit` | — | closes the window |

`state` is the source of truth for the harness: the response has the grid
**after** the WM, which is what matters. `resize` returns the request, not the
result — the client is the one that checks, with `state`, and that is why
`benchrun.py` insists until it matches.

**`state.title` is not the title on screen.** It is `Term::title`, what the child
asked for with `OSC 0/2` (default `tortty`); the tab bar and the window show
`Tab::title()`, which is the OSC when there is one and `basename(cwd)` otherwise.
A probe that reads `state.title` sees `tortty` and concludes the cwd fallback is
broken when it is working (found investigating B37). For the title the user
sees: `xdotool getwindowname <xid>` for the window, or `text where=view` for the
bar.

## Pitfalls

### winit

- **`request_inner_size` is a suggestion.** Asking for `cols * cw` pixels can
  come back 1 cell short (the WM rounds and `floor` cuts). The app keeps the
  requested grid (`want`) and corrects it in cell steps until it matches, up to
  4 attempts; without that the benchmark would measure 60x18 thinking it
  measured 61x18.
- **`RedrawRequested` is not "every frame"**: it is on demand. With `--no-vsync`
  what pulls the drawing is the `request_redraw` after each pty read; in normal
  use it is vsync.
- **`Resized` arrives with the physical size**; the grid has to be recomputed
  with the current `cw`/`ch`, and the `wgpu` `surface` reconfigured in the same
  event.
- **Keyboard**: `logical_key` matters for text; `physical_key` for modifiers and
  function keys. Using only one of the two gets accents or shortcuts wrong.
- **A composed dead key keeps the base key as `logical_key`**: `xkbcommon` (in
  winit) does the composition, the dead key itself is `Key::Dead` (no byte) and
  the result lands in `event.text` — but `~`+space arrives as `Named(Space)`, so
  whoever reads only the named table throws the `~` away (B36: the child got a
  bare space). The composed text wins over the named table whenever it differs
  and is printable; a control character (`Backspace` brings `\x08`) is not
  composition.

### wgpu

- **`InstanceDescriptor`, `RequestAdapterOptions` and `DeviceDescriptor` changed
  shape** between versions: new mandatory fields, no default. Compile against
  the `Cargo.lock` version, not against the web documentation.
- **The `surface` must be reconfigured on every resize** (format, size),
  otherwise a frame with garbage or a `Validation Error` in the log.
- **`Usage::COPY_DST` on the atlas** and `write_texture` in the same command
  that draws needs care: upload the atlas *before* the frame's render pass.
- **The atlas format is `R8`** (alpha): the shader colors by the instance
  colour. Using `Rgba8` quadruples the memory and improves nothing.

### font (fontdb + swash)

- **A colour bitmap steals a glyph from the primary**: `glyph_bitmap` exists for
  text outside the grid (bar, sidebar, badge, toast) and resolves colour →
  mask. If the colour pass walks the entire fallback chain before any mask, a
  font that maps ASCII wins: **Noto Color Emoji** maps digits and returns a
  19x18 px bitmap, so the tab number became a two-cell rectangle. The right
  order is the cell font first (colour and mask in the same face) and only then
  the chain; the grid does not suffer from this because it picks the face by
  the cmap (`face_of`). Test: `digit_comes_from_primary_as_mask`.
- **There is no fontconfig**: the choice is a fixed preference list + a system
  scan. A machine without a monospaced font → the app exits with a message, not
  with `.notdef`.
- **pt→px**: `px = pt * dpi / 72 * scale_factor`. It is what makes the cell
  match Pango/GDK (`Xft.dpi`); getting it wrong misaligns the grid by 8%.
- **Metrics** come from the `M` glyph, not from `font_extents`: it is what the
  terminal needs (advance width), not the typographic box.

### pty

- **`CommandBuilder` does not inherit the `cwd`** on its own: without `cwd()`,
  `mc` opens in `$HOME` even if the app was called from another directory.
- **The exported `SHELL`** is what actually runs (`--cmd` swaps the command, not
  `SHELL`): the child inherits the whole environment and starts lying about
  itself.
- **`TERM=xterm-256color`** and **`COLORTERM=truecolor`** must be exported;
  without them `mc` draws in 8 colours and `vim` does not turn on truecolor.

### parser / grid

- **Pending wrap**: after writing to the last column, the next character wraps
  the line — and a `\r\n` right after it **must not** count as a second wrap. TUI
  patterns that draw up to the edge depend on it.
- **Damage is the only source of repaint.** A new path that writes a cell
  without marking `dirty[y]` does not show up on screen until something scrolls,
  and the bug looks like a parse bug.
- **Wide char at the right edge**: it does not fit, so the cell stays empty and
  the glyph goes to the next line. Writing in the middle of a wide pair (or
  erasing half of it) has to clear both cells, otherwise a ghost remains.
- **A history anchor is a distance from live, not an absolute line**: `CSI 3J`
  erases the history and an absolute anchor makes the view jump to the top.
- **`CSI 3J`/`RIS`/alt-screen switch** invalidate cursor, margins and charset:
  clearing only the grid leaves dirty state between applications.
- **`feed` with partial UTF-8**: the buffer can end in the middle of a sequence;
  keep the bytes and continue on the next call (the `dmesg` cut in the middle of
  an accented character is the test case).
- **8-bit `ST`**: `0x9c` closes OSC/DCS/APC/PM/SOS like `ESC \` (many clients
  send that instead of the 7-bit ST). It only holds when it is **not** a UTF-8
  continuation byte — `М` is `d0 9c`, and cutting there loses the rest of the
  title. An `ESC` that is not ST (inside the body) **ends** the sequence: pushing
  it back into the payload glued the `ESC ]` of the following title inside the
  title and the tab bar showed garbage before the text (B4, 2026-11-04). The
  probe is the socket `osc` verb, which returns the raw payloads.
- **Chrome text is not a terminal sequence**: the rule is a single one,
  `tortty_core::term::sanitize_title` (control, zero-width, direction marks and
  BOM out), and it holds at **two levels**:
  - *drawing*: `Renderer::chrome_text` is the only place that draws text outside
    the grid (tab bar, sidebar card, history badge, toast) and sanitizes there —
    a new chrome element inherits the guarantee without calling anything;
  - *producers*: OSC payload (`osc_dispatch`), cwd basename (`Tab::cwd_name`)
    and workspace name (rename typing and the `Enter` that confirms it) come
    out clean, because besides the screen they go to the window title, to the
    socket `state` and to the `dump` header — a `\n` in a directory name breaks
    the dump format.

## Quality (linter and formatting)

```sh
./tools/quality-check.sh          # the full ruler: fmt, clippy, test, build, parsebench
./tools/quality-check.sh --fix    # fixes what is mechanical (fmt + clippy --fix)
```

`pre-commit` runs `--fast` on its own (installed with
`git config core.hooksPath .githooks`); CI runs the whole script. If `clippy`
complains about something the code **wants** to do, the `#[allow]` goes in the
function with the reason — not at the top of the crate, and not in a global
`#![allow]`.

Before running the GUI, remember the rest of this file: killing a process is by
PID file, and `pkill` by name brings a session down.

### Mouse

`mouse.rs` is pure: it translates `(button, col, row)` into protocol bytes. What
decides whether the event goes to the child is the `Term` (`mouse_mode`
0/1/2/3, `mouse_sgr`, `mouse_utf8`, `mouse_px`). The socket `state` exposes all
four, which is how you find out, without guessing, whether the app turned the
mouse on and in which dialect.

`?1016` is the SGR shape with the numbers in pixels instead of cells: the
encoder keeps the shape and the caller fills `col`/`row` with pixel offsets from
the grid origin (`App::grid_px`, the same origin `cell_at` divides by the cell
size).

**Pitfall**: `xdotool key`/`keydown` under `Xvfb` goes **nowhere** without
`xdotool windowfocus $XID` first — there is no window manager to give the window
focus, so the app never sees `ModifiersChanged` and a probe of anything that
depends on a modifier (the `OSC 8` underline, Ctrl+click) reports "nothing
happened" for the wrong reason.

Probe that closes the loop (what the app *received*, not what it did):

```sh
./tools/devrun.sh --cmd 'python3 /tmp/mtest.py' --cols 60 --rows 12 --debug-socket /tmp/tt.sock
# mtest.py does tty.setraw(0), turns on ?1000/?1006 and prints the hex of what arrives
xdotool mousemove $((X+120)) $((Y+60)) click 1
# read the screen: {"cmd":"text","where":"view"} -> 1b5b3c303b31313b344d = ESC[<0;11;4M
```

### Chrome: line 1 and the left column

Two bands of the window are not grid, and both live in the same space as the
shader `origin` (`pad + bar`, `pad + side`), which is the source of almost every
coordinate bug:

| band | drawing | click test |
|---|---|---|
| tab bar (line 1, to the right of the sidebar) | `y0 = -bar`, `x0 = 0` | `tab_at`: `py` in `[pad, pad+bar)`, `x` from `pad+side` |
| sidebar (left column, from line 1) | `y0 = -bar` on the 1st card | `card_at`: `px < pad+side`, `i = (py - pad) / (2.4*ch)` |

The bar **disappears** with a single tab (`bar_h = 0`), so everything that
depends on it has to ask `bar_h(ch)` at the time — and the grid gets the line
back when it goes away. The card is 2.4 lines tall: to click the second one, the
`y` is ~1.5 lines below the first, not half a line.

Origin correction, when the click lands on the wrong band: move the mouse and
read `cursor_px`/`cell_at` from `state` (it is exact, no frame offset) and derive
`side = px - col*cw`.

### Initial grid size

Two pitfalls together, and both look like "scrambled text in the first second":

1. Asking for the window at `round(cols*cw)` gives **fewer** pixels than the grid
   needs (100 cells of 13.044 px = 1304.4 → 1304 → 99 columns). That is why the
   request is `ceil`.
2. The child used to be born before knowing the final size; the first WM
   `resize` arrived with the child already writing, and what was on screen kept
   the old width. Now `apply_size` runs first and `spawn_shell` only happens at
   the first applied size.

Probe: `tortty --cmd 'stty size'` has to print exactly `<rows> <cols>` of the
request, with no resize afterwards.

### Keyboard

The socket `keys` verb goes through the **real encoder** and returns the bytes
in hex, which is how you check `Ctrl+Space` (NUL), `Alt+←` (`CSI 1;3D`) and `F5`
(`CSI 15~`) without guessing:

```sh
python3 -c 'import socket,json; s=socket.socket(socket.AF_UNIX); s.connect("/tmp/tt.sock");
s.sendall(b"{\"cmd\":\"keys\",\"data\":\"alt+left\"}\n"); print(s.recv(999))'
```

Careful when testing: `ctrl+c`/`ctrl+\\` kill the child (`SIGINT`/`SIGQUIT`), the
app closes with it and the client is left reading from a dead socket. Use a child
that ignores the signals (`sh -c 'trap "" INT QUIT; sleep 200'`).

Dead keys have no code of ours: `winit` delivers `Key::Dead` (which generates no
byte) and then the composed text in `event.text`. Test with
`xdotool key dead_tilde a` and read the screen through `text`. The second key
still travels as the base key: `dead_tilde space` arrives as `Named(Space)` with
text `~`, which is why a child that records its stdin (`--cmd 'cat > file'`) and
a `Return` at the end is the probe that shows the bytes and not just the screen
(B36).

## Measurement

See [`BENCHMARKS.md`](BENCHMARKS.md) — it is the only place with numbers. Flow
summary: `parsebench` for the parser, `tools/benchrun.py` for the app,
`tools/bench.sh` for the ladder.

For a probe of the socket or of the keyboard (`xdotool`, `state`, `text`) the
**debug** binary is enough: `cargo build -p tortty` takes seconds against the
~50 s of `cargo build --release`, and nothing there is about throughput.
`bench.sh` is the one that needs `release`; [`render-check.sh`](../tools/render-check.sh)
runs the **debug** binary by default (`TORTTY_BIN` overrides).

## Pitfall: drawing inside an input handler turns the event queue into inertia

`scroll_view()` drew a frame inline (`redraw()` → `renderer.render`) for every
wheel event. One line of code, and it does not look expensive — but a touchpad
delivers up to ~125 events/s, so the app consumed slower than X delivered, the
server queue grew, and the view kept scrolling after the finger left the pad.
That reads as smooth-scroll inertia and is not: it is backlog. Any path that
mutates the view must `request_redraw()` and let `RedrawRequested` draw, which
is what the pty path and the frame gate already do. `ask_frame()` (`app.rs`) is
that ask; `redraw()` is only for the action paths, where one present per action
is the latency wanted. The socket verbs that only read (`state`, `text`, `prof`,
`log`, `osc`) must not draw either: `debug_read_only` keeps a poll loop from
buying a frame per request, which is what `benchrun.py` does with `state`.

## Pitfall: one input event is not one notch

`on_wheel`'s branch for a child that reports mouse used the delta only as a sign:
every `MouseWheel` became a full notch, so a touchpad — fractions of a detent at
up to ~125 events/s from X — scrolled dozens of screens on a whisper of the
finger (`B40`, 2026-11-07). The count has to follow the **travel**
(`mouse::child_notches` + `Tab::child_frac`), one detent per notch. Two traps sit
beside it: X11 turns one detent into button press *and* release, two
`MouseWheel` events with the same direction (dedupe like `bar_wheel`), and no
protocol has a wheel release — `mouse::encode` drops it, because a TUI that
counts every report rolls twice. Probe without the owner's display: Xvfb plus a
child that writes `?1000h`/`?1006h` and logs what arrives, then `xdotool click
4` three times — three `<64;…M` and nothing else.

## Pitfall: a pending X error kills the app in `set_title`

Symptom, on X11:

```
thread 'main' panicked at winit-0.30/src/platform_impl/linux/x11/window.rs:1028
Failed to set window title: XError { description: "BadDrawable (invalid Pixmap or
Window parameter)", error_code: 9, request_code: 149, minor_code: 4 }
```

`winit` treats **any** X error received during a `flush` as fatal and attributes
it to the call that was being made. When the Mesa WSI (llvmpipe, without
`/dev/dri`) creates and destroys pixmaps on our connection, a pending error is
left over and the next `ChangeProperty` (title) takes the blame — the window dies
even though it is healthy, and the error is not its fault.

How to confirm: `RUST_BACKTRACE=1` shows `sync_title` in the stack, and the app
**runs normally** with the probe below.

Probe: `TORTTY_NO_TITLE=1 tortty` (does not touch the window title). It is a
debugging probe, not configuration — it only exists so you can use the terminal
while investigating.

## Pitfall: a panic on the way out is invisible from the `.desktop` shortcut

Launched from the menu, tortty has **no stderr**: the entry is `Terminal=false`
and nothing captures the output, so a panic during the shutdown looks exactly
like a normal close — the window disappears and the only trace is an exit code
nobody reads. That is how B38 read as "the window state is not saved": the app
was dying on the way out, before `window.json` was written. The same silence
covers any failure that only shows on the way in (font, GPU, config).

How to see it: run the installed binary from a shell (`~/.local/bin/tortty`) and
close it the same way, or append `2>/tmp/tortty.err` to the `Exec=` line while
investigating. The gate is step 8 of [`render-check.sh`](../tools/render-check.sh): it closes
by the shell ending and fails on `rc=101`.

## Pitfall: GL without a display handle, and `alpha_mode`

Two things that only show up in the GL backend:

- `InstanceDescriptor::new_without_display_handle_from_env()` makes EGL reply
  `gl not compatible with provided surface`. The display handle is not optional
  for EGL; the instance is created with
  `new_with_display_handle_from_env(Box::new(window))`.
- `alpha_mode: caps.alpha_modes[0]` picked `PreMultiplied` on GL and the window
  came out translucent. A terminal is opaque: explicit
  `CompositeAlphaMode::Opaque`.

The backend and the board chosen appear in the first line of the log:

```
tortty: GPU: Vulkan llvmpipe (LLVM 20.1.2, 256 bits) (driver llvmpipe)
```

## Degraded sandbox: a window that paints nothing

**Before investigating drawing, check `/dev/dri` with `ls -la /dev/dri`.** Without
the device nodes `wgpu` only has llvmpipe. Under a display of its own (Xvfb)
llvmpipe **draws** — `tools/render-check.sh` passes without `/dev/dri`. In the owner's
real session, on 2026-11-02, the container without `/dev/dri` made the pty and
the socket work with the window **receiving no frame**.

The first field to look at is the pair that separates the two causes:

| `drawn` | `skips` | `last_frame` | reading |
|---|---|---|---|
| > 0 | 0 | `drawn` | it drew and presented |
| 0 | > 0 | `skipped (timeout/occluded)` | drawing runs, the **surface** refuses: environment (compositor/WM/sandbox), not the app |
| 0 | 0 | `none` | the app did not even get to draw: see the log (`tortty: GPU:`), pty and config |

**`drawn > 0` does not prove the screen drew.** Measured on 2026-11-03 on `sandbox`
without a GPU: with `WGPU_BACKEND=gl` the counter went up (`drawn 5`, `skips 0`)
and the window's own pixmap **did not change a byte** — a present that returns
success and does not paint. The test that does not lie is comparing the window
with itself:

```sh
xwd -id <xid> -silent -out /tmp/a.xwd      # xid comes from the socket `state`
# ... make the app paint something (the `write` verb) ...
xwd -id <xid> -silent -out /tmp/b.xwd
cmp -s /tmp/a.xwd /tmp/b.xwd && echo "does not paint" || echo "paints"
```

When the surface refuses the frame, the app writes **once** to stderr:

```
tortty: the surface is not presenting (timeout/occluded) — the app draws, but the
frame does not reach the screen. A confined environment without a visible GPU
usually looks like this: check /dev/dri and, on sandbox, use `sandbox @gpu`; see docs/DEBUG.md
```

Signals in the log, in this order, when it is the sandbox:

```
_amdgpu_device_initialize: amdgpu_get_auth (1) failed (-1)
libEGL warning: egl: failed to create dri2 screen
MESA: error: Failed to attach to x11 shm
```

Practical consequence: **visual verification is not trustworthy in that state**.
In that case the gate is another one: `tools/render-check.sh` on an Xvfb (pixel proof
without depending on the owner's display), plus what does not depend on pixel —
`state` (grid, cursor, images, `drawn`/`skips`/`rows_max`/`last_damage`),
`--selftest`, `cargo test` and `parsebench`.

In the same sandbox, the **`cargo` cache has been read-only**
(`/home/user/.cargo/registry`): a new dependency failed with `Read-only file
system (os error 30)`. Today the `@dev` profile of the sandbox frees writing in
that directory (`sandbox @dev -- cargo build`), and without it the workaround is
`CARGO_HOME=/tmp/cargohome` (a copy of `~/.cargo` on tmpfs).

### The three terminals inside `sandbox` (measured on 2026-11-02)

Same sandbox, no `/dev/dri`, everything on llvmpipe. The difference between them
is error policy, not the capacity of the environment:

| terminal | what happens | how it was found out |
|---|---|---|
| `kitty 0.48.2` | **opens and draws**; it only complains about DBus | `xdotool` finds the window; the log only has `glfw error 65544` (DBus) |
| `ghostty 1.3.1` | opens the window, but GL degrades: `MESA: error: Failed to attach to x11 shm` in a loop | it does not exit; the window exists and the log repeats the SHM error |
| `tortty` (Vulkan/lavapipe), up to 1.0.4 | **aborted** (134) in `set_window_title` | winit panic at `x11/window.rs:1028` |
| `tortty` since 1.0.5 | **does not abort**: the X error that is not ours is recorded and the app goes on (hook below) | `sandbox tortty` alive for 12 s, answering on the socket |
| `tortty` with `WGPU_BACKEND=gl` | also does not abort, but **does not paint** (silent no-op present) | `xwd` of the window identical before/after |
| `sandbox @gpu tortty` | **really paints** (`RADV RENOIR`, pixmap changes) | `xwd` before/after different |

That is: without `/dev/dri` this X server does not present a GPU surface —
neither through Vulkan nor through GL. The way out is the sandbox `@gpu`
profile, not a different backend; the app warns about it in one line on stderr.

### An X error from the WSI must not kill the app

`winit` keeps the last X error of the connection and `expect()`s it on the next
call that flushes (`x11/window.rs:1028`, in `set_window_title`). An error that
**is not ours** — the Mesa WSI leaves it pending while creating/destroying a
pixmap — killed the process. `crates/app/src/xguard.rs` registers winit's
public hook (`platform::x11::register_xlib_error_hook`) and swallows it,
recording it on stderr:

| code | where | why |
|---|---|---|
| `BadDrawable` (9), `BadWindow` (3), `BadMatch` (8) | any request | drawing: the WSI touches pixmap/window, including on request from the core |
| `BadAccess` (10) | extension only (`request_code >= 128`) | it is `DRI3Open` denied in the sandbox — **this was the one that killed it** |

With `TORTTY_XERR=1` the hook prints **every** X error it sees, with code,
request and whether it swallowed it — that is how the `BadAccess` on `DRI3`
showed up:

```
XERR code=10 request=130 minor=1 resource=0x4000002 swallow=false
```

`TORTTY_NO_TITLE=1` still exists as a probe (it does not touch the title), but
since the hook it is no longer needed for the app to survive.

### Screen capture is not neutral

- **Obscured window**: `xwd`/`import` of a covered window return what is **on
  screen** at those coordinates — the content of another window. If the capture
  "does not match the app", first ask whether it is visible.
- **Pointer grab**: `import`/`xwd` grab the pointer and show a crosshair (the
  "aim mode"); if the process dies halfway, the pointer can stay stuck until the
  X client goes away.
- Consequence: capture only on a virtual display of your own
  (`tools/render-check.sh`); never on the display of whoever is working.
