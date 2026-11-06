# Features

What tortty does today, by area. One line per capability; where a capability has
a known limit, the **limit** is named in the same entry. A limit in brackets with
an id is in the [`BACKLOG.md`](BACKLOG.md) queue; without one it is an accepted
consequence, explained in [`LIMITATIONS.md`](LIMITATIONS.md).

Where each thing came from (the Python original, `file:line`) is
[`MIGRATION.md`](MIGRATION.md); numbers are [`BENCHMARKS.md`](BENCHMARKS.md);
configuration keys are [`CONFIG.md`](CONFIG.md).

## Terminal core

- VT parser: C0/C1, CSI, OSC, and DCS/APC/PM/SOS routing, incremental UTF-8 with a
  buffered partial and invalid input as U+FFFD.
- Cursor: CUU/CUD/CUF/CUB/CNL/CPL/CHA/VPA/CUP/HVP, DECSC/DECRC and `CSI s`/`u`,
  DECSTBM scroll region, RI at the margin.
- Editing: IL/DL/ICH/DCH/ECH, EL 0/1/2, ED 0/1/2/3 — `ED 3` clears the scrollback
  and kills the selection anchors.
- Modes: `?1` DECCKM, `?25`, `?47`/`?1047`/`?1049` alt screen, `?1048`,
  `?1000`/`?1002`/`?1003`/`?1006`, `?1004` focus, `?2004` paste, `?2026` sync.
- SGR: bold, italic, underline (styles `4:2`…`4:5`), reverse, 16 colours, 256,
  truecolor `38;2`/`48;2`, and `:` subparameters.
- OSC: 0/2 window title, 1 icon title, 4 palette query, 10/11 default colour
  query. **limit:** everything else is discarded — OSC 52 (remote clipboard) and
  OSC 7 (shell cwd) are examples (`M-R`).
- DEC Special Graphics charset (G0).
- Alt screen: separate grid, swap preserving the cursor, crop/pad on resize (no
  reflow there).
- Reflow on resize: joins and splits soft lines and keeps the coloured
  background. **limit:** it kills the selection anchor, as in Python (`M-P`).
- Scrollback with a configurable cap ([`CONFIG.md`](CONFIG.md)).
  **limit:** no search (`M-D`) and no visible scrollbar (`M-M`).
- Wide cells carry a continuation mark — an improvement over Python, where CJK
  occupied one cell. **limit:** a combining mark arriving in a later commit
  becomes its own cell (`M-U`).
- `TERM=xterm-256color` with no terminfo of our own.
- **Not implemented:** `IRM`/`LNM`, `CSI ? 3` (80/132), DECDL/DECDHL and
  double-height — none of which Python has either.

## Input

- Keyboard: arrows, Home/End, Ins/Del, PgUp/PgDn, F1–F12; modifiers as a
  parameter; DECCKM; Shift+Tab; Ctrl+letter; Alt as an ESC prefix; dead keys and
  composition (ABNT2 `~ ^ ´`); printable text from the event on non-US layouts.
- kitty keyboard protocol: push/pop/set/query, flag 1 (disambiguate), flag 2
  (event types, release `:3`), flag 8 (all keys as CSI u), flag 16 (associated
  text). **limit:** flag 4, the alternate keys, is not done (`M-G`), and
  Shift+letter reports the post-Shift code (`CSI 65;2u`), as Python does.
- Mouse: X10 and SGR encodings, modifiers in the button code, wheel, buttons 8/9,
  motion with and without a button. **limit:** no pixel-precision reporting
  `?1016` (`M-T`).
- Selection: drag with a 3 px threshold, absolute anchor that survives scrolling
  and output, drawn as inversion. **limit:** no double/triple click word or line
  and no rectangular selection (`M-E`).
- Clipboard: PRIMARY on release, CLIPBOARD with `copy_on_select`,
  Ctrl+Shift+C/V, button 2 pastes PRIMARY, bracketed paste. **limit:** PRIMARY
  dies with the app — it owns the selection; CLIPBOARD survives.

## Drawing

- `wgpu`, one pipeline and one draw call per category (background, glyph,
  underline, cursor); the adapter is chosen by rule, hardware before CPU.
  **limit:** no rasterizer of our own, and the CPU adapter serves content, not a
  confined session ([`LIMITATIONS.md`](LIMITATIONS.md)).
- Font in **points** converted by DPI, cell metric from the advance of `M`,
  baseline layout, whole cells.
- Bold and italic from a real face, no synthesis: without the face it falls back
  to regular.
- Per-glyph fallback chain by name, so a missing font does not become another
  one; colour emoji (CBDT) in their own RGBA atlas; CJK, arrows, Greek.
- Ligatures and shaping with `rustybuzz` (`off`/`standard`/`all`). **limit:**
  single-cell runs are not shaped, the cursor and the selection break the
  ligature around them, and bidi is not reordered.
- Cursor block/beam/underline with blink; `CSI 6 q` from an application wins.
- Underline at the width of the run, selection as inversion, padding in px,
  background painted to the border.
- Damage per line: only dirty lines are rebuilt, and a full-screen scroll shifts
  the lines that are ready. **limit:** per-region drawing is not done (`M-AC`),
  and box drawing can show a 1 px seam at a fractional cell width (`M-AD`).
- Frame coalescing and `?2026` held output, so one frame is drawn per frame.

## Images (kitty protocol)

- Transmit, display, place by id, delete by id or everything, capability query,
  chunking, `f=24/32/100` (PNG decoded in the app), `o=z`, `s=`/`v=`, `c=`/`r=`,
  `X=`/`Y=`, `C=1`, `q=`; the image follows the scroll and is pruned with the
  history; `ED 3` clears images.
- **limit:** the image must arrive **embedded** in the escape (base64) — file and
  SHM transfer (`t=f`, `t=s`) are not there yet (`B1`), there is no animation
  (`B2`), and no z-index, placement id, Unicode placeholder or source crop
  (`M-L`).

## Window

- Tabs: own pty and term, new tab inherits the cwd of the deepest descendant,
  close with Ctrl+Shift+W or middle click, a dead shell closes itself, drag to
  reorder, wheel cycles.
- Workspaces: a set of tabs with a name, sidebar cards (hidden when there is
  only one), inline rename, stable numbering.
- Title: OSC 0/2, else `basename(cwd)`, else `tortty`; the window title mirrors
  the active tab.
- Chrome: toast with a queue and timeout, visual bell, audible bell.
- Shortcuts: Ctrl+Shift+T/W/N/Q/F2, Ctrl+Shift+1…9, Alt+1…9, Super+N/Q/1…9, with
  the digit taken from the physical key so ABNT2 layouts still work.
  **limit:** they are fixed, no remapping yet (`M-H`), and keys Python does not
  deliver (F13–F24, keypad with NumLock off) are not handled (`M-S`).
- Geometry: 980×640 without state, size restored, maximized tracked separately
  from the normal size, invalid geometry rejected. **limit:** tabs and workspaces
  do not survive closing (`M-I`); on Wayland the position is not stored, because
  the client does not choose it.
- Lifecycle: the window closes when the shell ends; `SIGUSR1` dumps,
  `SIGTERM` dumps and exits cleanly, `--quit-after` for automation.
- **Not implemented:** context menu (`M-V`), drag-and-drop of a file onto the
  window, and an automatic dark/light theme — the palette comes from the config.

## Configuration

- A kitty-format file on top of the built-in defaults, one option per line, last
  key wins, an error is reported per line and does not take the file down.
- Keys: `shell`, `font_family`, `font_size`, `ligatures`, `cursor_shape`,
  `cursor_blink`, `scrollback`, `window_padding_width`, `bell`, `copy_on_select`,
  `debug_log`, `foreground`, `background`, `cursor_color`, `color0..15`.
- Hot reload with a debounce; **any** error on reload keeps the configuration
  already in effect, and the error shows as a toast.
- CLI: `-c/--config`, `--cmd`, `--cols`/`--rows`, `--font-family`, `--font-size`,
  `--scrollback`, `--dpi`, `--no-vsync`, `--debug-socket`, `--debug-log`, `--prof`,
  `--quit-after`, `--dump-to`, `--selftest`, `--version`.

## Debug socket and instrumentation

- Unix socket, JSON per line, several requests per connection: `state`, `text`,
  `write`, `keys`, `resize`, `prof`, `log`, `quit`.
- `--debug-socket auto` opens `$XDG_RUNTIME_DIR/tortty-<pid>.sock`; the socket is
  `0600` and checks `SO_PEERCRED`, and it never deletes a file that is not its
  own.
- `--dump-to` and `SIGUSR1` give the same view with no socket.
- One instance per PID file: `tools/devkill.sh` kills that PID, checking
  `/proc/<pid>/exe`.

## Platform and distribution

- Linux X11 with Vulkan or GL, plus the CPU adapter fallback.
  **limit:** Wayland compiles and is not exercised (`M-AB`); Windows and macOS
  are not targets yet (`M-X`, `M-Y`), so "it works there" is not a fact.
- Installer: the `tortty` command, menu entry, icon, `--prefix`, `--check`,
  `--uninstall`. **limit:** `--from-release` needs an asset published in GitHub
  Releases; there is none yet, so `./install.sh` is the path today.

## Tests and verification

- Headless core check suite, run by `--selftest` without a window, covering
  parser, config/CLI, keyboard/mouse, socket, reflow/selection/anchor.
- Visual gate on its own Xvfb: draws, captures and compares pixels
  (`tools/visual.sh`), so a drawing regression fails without an eye.
- Reproducible benchmark with the grid locked by the socket
  (`tools/benchrun.py`) and the Rust × Python ladder (`tools/bench.sh`).
