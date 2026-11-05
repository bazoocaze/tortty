# Migration: bzmux Python → bzmux Rust

Record of the migration from bzmux Python to Rust: one line per Python feature,
with its state here and the `arquivo:linha` of origin. While it was running,
this file was how "how much is left" was answered — not by estimate. Today it
answers "what Python did, and where that is here".

- **Python was the observable specification** (`~/tortty-python`):
  every line keeps the `arquivo:linha` the feature came from.
- **The migration is closed** (2026-11-02): **195 lines, all `done`** — the
  closure is recorded right below the panel. What is still missing here is not
  parity: it is **new activity**, and it lives in [`BACKLOG.md`](BACKLOG.md), with
  id `B<numero>` (the old ones stay `M-*`). What is a decision to **not** do lives
  in [`LIMITATIONS.md`](LIMITATIONS.md) and would appear here as `no`.
- Performance numbers do not go here: [`BENCHMARKS.md`](BENCHMARKS.md).
- Dated record of what has already been closed: [`HISTORY.md`](HISTORY.md).

## Panel
| area | done | partial | missing | no | blocked | total |
|---|---|---|---|---|---|---|
| 1. Core: cell, colors, grid | 18 | 0 | 0 | 0 | 0 | 18 |
| 2. Parser: controls, CSI, modes | 28 | 0 | 0 | 0 | 0 | 28 |
| 3. Parser: SGR and OSC | 11 | 0 | 0 | 0 | 0 | 11 |
| 4. Scrollback and history view | 8 | 0 | 0 | 0 | 0 | 8 |
| 5. Input: keyboard and the kitty protocol | 16 | 0 | 0 | 0 | 0 | 16 |
| 6. Input: mouse, selection, clipboard, paste | 14 | 0 | 0 | 0 | 0 | 14 |
| 7. Drawing: font, geometry, colors | 15 | 0 | 0 | 0 | 0 | 15 |
| 8. kitty protocol images | 13 | 0 | 0 | 0 | 0 | 13 |
| 9. Window: tabs, workspaces, chrome | 18 | 0 | 0 | 0 | 0 | 18 |
| 10. Window: lifecycle, geometry, state | 10 | 0 | 0 | 0 | 0 | 10 |
| 11. Configuration and CLI | 18 | 0 | 0 | 0 | 0 | 18 |
| 12. Debug socket and instrumentation | 14 | 0 | 0 | 0 | 0 | 14 |
| 13. Platform and distribution | 3 | 0 | 0 | 0 | 0 | 3 |
| 14. Tests and verification | 9 | 0 | 0 | 0 | 0 | 9 |
| **total** | **195** | **0** | **0** | **0** | **0** | **195** |

Ready equivalent: **100%** (195 of 195 parity lines `done`; `partial` counts 0.5
and `blocked` counts as `missing` — neither of the two exists any more). The
total here is parity only: an improvement Python does not have lives in
[`BACKLOG.md`](BACKLOG.md) and does not enter this count.

## Closing of the migration (2026-11-02)

**The migration from bzmux Python to bzmux Rust is finished.** The panel above is
the proof: 195 parity lines, no `missing`, no `partial`, no `blocked`. What
Python does, this repository does.

What closed, in the order it closed (section 16 has the criterion of each slice):

- the last parity lines were per-line damage in drawing (`A17`), emoji/CJK
  (`G7`) and ligatures with `rustybuzz` (`G8`);
- then came the checks without display (`N1` — 31 in `--selftest`), the automated
  visual verification (`N7` — `tools/visual.sh`) and the rejected-frame
  diagnostic (`M-AA`, closed as an improvement and therefore in `BACKLOG`);
- and then the tidying: `M4` and `G17` left for `BACKLOG` (2026-11-02) and the
  lines that **were not parity** — the three image refusals (`B1`–`B3` today) and
  five improvements of ours that were counted as if they were Python's — left
  the panel, with a dated record in [`HISTORY.md`](HISTORY.md).

## Closure: what this file becomes

- **It is not a queue.** A new item is born in [`BACKLOG.md`](BACKLOG.md), with id
  `B<numero>`. Only a line Python delivers enters here.
- **It is not pending work.** "Missing" stopped being the project's vocabulary:
  what does not exist here is new activity.
- **Python became history.** It serves to answer "how the app behaved" and to
  judge regression — not to say what to do next.
- **The closure verification**: `./tools/check.sh` green (fmt, `clippy -D warnings`,
  tests, `build --release`, `--selftest` 31/31, `parsebench`), `tools/visual.sh`
  green, and version **1.0.0** installed and exercised.

What came after the migration — and the order in which it was done — is in
section 16.

## Rules of this document

1. **One line, one feature.** If the line needs "and" to describe it, it is two
   lines.
2. **State always filled** with one of these: `done`, `partial`, `missing`,
   `no`, `blocked`. `partial` requires saying **what is missing** in the line
   itself; `blocked` requires saying **what blocks it** (an environment that does
   not exist here), and it is not the same as `no`: `no` is a decision, `blocked`
   is a missing condition. **An improvement has no state here**: if Python does
   not do it, it is not parity — it becomes an item of [`BACKLOG.md`](BACKLOG.md)
   and gets no line in this file.
3. **Closing an item** = changing the state, running the verification of the
   "verification" column and recording a line in `HISTORY.md` with the date. The
   item does not leave here: the panel counts what is done, not what is left.
4. **New item** (discovered along the way) enters as a new line in the area, not
   as a paragraph. If Python does not have it, it is an **improvement**
   (`BACKLOG.md`), not migration.
5. **Execution order** does not live in the tables: it lives in section 16.
   Marking `done` out of order is normal; changing the order is a recorded
   decision.
6. **Verification** is what entitles `done`: an automated test when an
   equivalent exists in Python's `checks.py`; otherwise, the manual script
   written in the line itself (`vim`, `mc`, `dmesg`, screenshot).

Legend of the `ref` columns: `arquivo:linha` of the Python repo → `arquivo` of
the Rust repo (no line, so it does not age with every commit).

---

## 1. Core: cell, colors, grid

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| A1 | Cell `{ch, fg, bg, flags}` | `palette.py:1-6` → `cell.rs` | done |
| A2 | 16 ANSI colors + 8 brights palette | `palette.py:38-55` → `color.rs` | done |
| A3 | 6×6×6 cube + 24 grays (xterm-256) | `palette.py:58-70` → `color.rs` | done |
| A4 | Default fg `#d6d6d6` / bg `#000000` | `palette.py:71-76` → `color.rs` | done |
| A5 | Truecolor in the cell (no allocated table) | `palette.py:24-32` → `color.rs` | done |
| A6 | Default-color sentinel distinct from an index | `palette.py:15-38` → `color.rs` | done |
| A7 | Reverse video per cell (inversion after resolving color) | `view.py:963-973` → `render.rs` | done |
| A8 | BCE: erasing inherits the current bg | `term.py:155-157` → `term.rs` | done |
| A9 | DEC Special Graphics charset (G0, `ESC ( 0`) | `palette.py:110-142` → `charset.rs` | done |
| A16 | Per-line damage (`dirty`/`dirty_all`) | `term.py:355-362` → `term.rs` | done |
| A17 | Render uses the damage (only rebuilds dirty lines) | `view.py:920-935` → `damage.rs` + `render.rs` | done: `damage.rs` decides by `Signature` (tab, geometry, scroll, selection, blink, theme) and the renderer keeps one instance per line. Full-screen scrolling now **shifts** the ready lines (the core reports `scroll_shift`), and `IL`/`DL`/`ED0`/`ED1` mark only the affected range. Measured on a 60x12 grid: `seq 1 100` rebuilds 12 lines once, `echo` rebuilds 1 (`state` → `rows_built`, `frames_all`, `global_why`); before it was the whole grid on every written line. Checked on screen (`seq`, `mc`) |
| A18 | Separate alt-screen grid, with a swap that preserves the cursor | `term.py:602-629` → `term.rs` | done |
| A19 | Alt-screen on resize: crop/pad without reflow | `term.py:184-217` → `term.rs` | done |
| A20 | A line leaving through the top goes to scrollback (only `top==0`) | `term.py:384-400` → `term.rs` | done |
| A21 | Absolute anchor (`scrolled` + map/clear generation) | `term.py:99-108.1026-1078` → `select.rs` (`gen`) | done: reflow/ED3/RIS bump `abs_gen` and kill the anchor |
| A22 | Reflow on resize (main buffer, joins/splits soft line) | `term.py:219-332` → `term.rs` `reflow()` | done |
| A23 | Reflow preserves colored bg (cuts only blanks with default bg) | `term.py:270-280` → `Cell::is_blank` in the cut | done |
| A24 | Core without I/O, without GUI, without clock (headless testable) | Python's decision (`AGENTS.md`) → `bzmux-core` | done |

## 2. Parser: controls, CSI, modes

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| B1 | C0: BEL (counter), BS, HT, LF, CR | `term.py:408-441` → `term.rs` | done |
| B2 | C0: CAN/SUB cancel the sequence | `term.py:858-861` → `term.rs` | done |
| B3 | C1 (0x80-0x9F) | draws as text (`term.py:862-866`) | done: C1 (0x80-0x9F) draws as text, same as Python — checked `0xC2 0x80`/`0xC2 0x9B` in the middle of "abc": 5 cells, `a\u{80}b\u{9b}c`. 8-bit `CSI` is not interpreted in either of them |
| B4 | `ESC [ ` CSI, `ESC ]` OSC, `ESC P/_/^/X` DCS/APC/PM/SOS | `term.py:871-876` → `term.rs` | done |
| B6 | `ESC 7`/`ESC 8` DECSC/DECRC | `term.py:882-888` → `term.rs` | done |
| B9 | `ESC M` RI (reverse scroll at the margin) | only moves the cursor (`term.py:879-881`) → `term.rs` | done |
| B10 | `ESC H` HTS, `ESC c` RIS | RIS `term.py:889-890` → `term.rs` | done |
| B12 | CSI cursor: CUU/CUD/CUF/CUB/CNL/CPL/CHA/VPA/CUP/HVP | `term.py:688-703` → `term.rs` | done |
| B13 | CSI `s`/`u` (SCOSC/SCORC) and `u` with a prefix = kitty kbd | `term.py:734-740` → `term.rs` | done: classic `CSI s`/`u` intact and prefixed `u` is the kitty protocol (`?`/`>`/`<`/`=`) |
| B14 | IL/DL/ICH/DCH/ECH | `term.py:708-728` → `term.rs` | done |
| B16 | EL 0/1/2 and ED 0/1/2 | `term.py:704-707` → `term.rs` | done |
| B17 | ED 3 clears scrollback + invalidates anchors | `term.py:456-465` → `term.rs` | done |
| B19 | DECSTBM (scroll region) | `term.py:729-733` → `term.rs` | done |
| B21 | Private modes `?1` DECCKM, `?25` DECTCEM | `term.py:594-601` → `term.rs` | done |
| B22 | `?47`/`?1047`/`?1049` alt screen (+ history clearing on 1049) | `term.py:602-629` → `term.rs` | done |
| B23 | `?1048` save/restore cursor | `term.py:630-634` → `term.rs` | done |
| B24 | `?1000`/`?1002`/`?1003` (mouse state) | `term.py:635-639` → `term.rs` | done (state; the encoding is missing — E) |
| B25 | `?1006` SGR mouse (state) | `term.py:640-641` → `term.rs` | done (state; encoding missing) |
| B26 | `?1004` focus report (state + send in the app) | `term.py:642-643` → `term.rs`, `app.rs` | done |
| B27 | `?2004` bracketed paste + payload `200~`/`201~` | `term.py:648-651.1082-1091` → `select.rs` | done |
| B28 | `?2026` synchronized output | `term.py:652-655` | done: state in the parser and the frame is held until the close (`frame::Gate`), with a 200 ms cap. Verified on the pty: 20 lines inside the transaction → 4 draws in 0.4 s (one per cap), and the final state comes out at the close |
| B29 | DSR/CPR `CSI 6 n`, status `CSI 5 n` | `term.py:772-773` → `term.rs` | done |
| B30 | DA1 `CSI c`, DA2 `CSI > c` | `term.py:743-749` → `term.rs` | done |
| B31 | DECRQM `CSI ? … $ p` | `term.py:750-754` → `term.rs` | done |
| B32 | `CSI 16 t` (cell) and `CSI 18 t` (grid) | `term.py:755-762` → `term.rs` | done |
| B33 | DECSCUSR `CSI Ps SP q` (+ back to the config on 0) | `term.py:764-771` → `term.rs`, `render.rs` | done |
| B34 | Response buffer to the child (`child_out`) | `term.py:140` → `term.rs` | done |
| B35 | Incremental UTF-8 with a buffered partial and invalid → U+FFFD | `view.py:227.404-406` → `term.rs` | done |

## 3. Parser: SGR and OSC

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| C1 | SGR 0, 1, 4, 7, 22, 24, 27 | `term.py:514-527` → `term.rs` | done |
| C2 | SGR 30-37, 39, 40-47, 49, 90-97, 100-107 | `term.py:528-539` → `term.rs` | done |
| C3 | SGR 38;5 / 48;5 (256) and 38;2 / 48;2 (truecolor) | `term.py:540-576` → `term.rs` | done |
| C4 | Subparameters `:` (`38:2::r:g:b`) | `term.py:500-512.579-591` → `term.rs` | done |
| C7 | SGR 4:2/4:3/4:4/4:5 (underline styles) | `4:x` collapses into simple (`term.py:518-519`) → `render.rs` | done (curly is an approximation with 4 rectangles) |
| C8 | Bold without "bold as bright" | `view.py:992` → `render.rs` | done (same semantics) |
| C9 | OSC 0/2 title | `term.py:959-960` → `term.rs` | done |
| C10 | OSC 1 (icon title) | ignored in Python | done (ignored, same) |
| C11 | OSC 10/11 default color query | `term.py:961-965` → `Term::osc_color` | done |
| C12 | OSC 4 palette query | `term.py:966-980` → `Term::osc_palette` | done |
| C19 | OSC with BEL **and** ST terminator, a lone ESC goes back to reprocessing | `term.py:914-926` → `term.rs` | done |

## 4. Scrollback and history view

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| D1 | Scrollback with a configurable cap (default 1000) | `term.py:109`, `config.py:150` → `term.rs` | done |
| D2 | Soft-break flag parallel to the history | `term.py:106-118` → `grid.rs` | done |
| D3 | `visible_rows(off)` always returns `rows` lines | `term.py:1006-1024` → `term.rs` | done |
| D4 | Mouse wheel scrolls 3 lines | `view.py:1208-1226` → `app.rs` | done |
| D6 | Shift+PgUp/PgDn/Home/End | `view.py:1285-1292` → `app.rs` | done |
| D7 | Any key returns to live (a modifier does not count) | `view.py:1290-1292` → `app.rs` | done |
| D8 | `↑N` indicator in the corner when in history | `view.py:1082-1091` → `render.rs` | done |
| D10 | View pinned while reading as output arrives | `view.py` + `term.py:1026-1078` → `Term::view_after_output` | done |

## 5. Input: keyboard and the kitty protocol

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| E1 | Simple key table (arrows, Home/End, Ins/Del, PgUp/PgDn, F1-F12) | `keys.py:46-74` → `keys.rs` | done |
| E2 | Modifier as a parameter (`CSI 1;mA`) | `keys.py:116-138` → `keys.rs` | done |
| E3 | DECCKM: arrows/Home/End in application mode (`ESC O A`) | `keys.py:78-84` → `keys.rs` | done |
| E4 | Shift+Tab (`ISO_Left_Tab` → `CSI Z`) | `keys.py:180-181` → `keys.rs` | done |
| E5 | Ctrl+letter (`chr(kv&0x1f)`), Ctrl+Space, Ctrl+minus | `keys.py:192-199` → `keys.rs` | done (Ctrl+Space = NUL, Ctrl+\ = 0x1c) |
| E6 | Alt as ESC prefix on a special key and on text | `keys.py:121-138.215-216` → `keys.rs` | done (special key becomes `CSI 1;3x`, text becomes `ESC` + letter) |
| E9 | Dead keys / composition (ABNT2 `~ ^ ´`) | `keys.py:13-44.219-227` → winit | done (winit delivers the composed text; `Key::Dead` generates no byte) |
| E10 | Printable text coming from the event (non-US layout) | `view.py:1295-1300` → `app.rs` | done (uses `event.text` and falls back to `acquired` when empty) |
| E12 | Kitty kbd: `CSI > u` push, `CSI < n u` pop, `CSI = f;m u` set | `term.py:982-1001` | done: `CSI > u` pushes (cap of 64, like kitty), `CSI < n u` pops; `Term::kbd_flags` stores per tab |
| E13 | Kitty kbd: `CSI ? u` query | `term.py:986` | done: `CSI ? u` answers `x1b[?<flags>u`; checked live (`?0u`, `?3u`, `?11u` after each push) |
| E14 | Kitty kbd flag 1 (disambiguate) | `kitty_kbd.py:25-52.115-119` | done: flag 1 — ESC becomes `CSI 27u`, Ctrl+a `CSI 97;5u`, arrow with a modifier `CSI 1;5A`; an arrow without a modifier stays in the legacy form (respects DECCKM) |
| E15 | Kitty kbd flag 2 (event types, release `:3`) | `kitty_kbd.py:90-109` | done: flag 2 — `:1`/`:2`/`:3`; a held key gave `CSI 97;1:2u` ×5 and release `:3` on the pty (hex read from the screen) |
| E17 | Kitty kbd flag 8 (all keys as CSI u) | `kitty_kbd.py:127-135` | done: flag 8 — `a` becomes `CSI 97u`, Shift+letter stops being plain text |
| E18 | Kitty kbd flag 16 (associated text) | `kitty_kbd.py:62-79` | done: flag 16 — `CSI 97;97u` (text field after the code) |
| E19 | Release without the kitty form sends no text (bug in `pi`) | `kitty_kbd.py:90-99` | done: a release without flag 2 sends nothing; the release only reaches the encoder when the flag exists (it was the doubled text in `pi`) |
| E20 | Local shortcuts take precedence over the encoder | `view.py:1263-1267` | done: the local shortcut runs before the encoder **and** eats the key release (otherwise `Ctrl+Shift+1` left `CSI 49;1:3u` in the child; X11 releases Shift/Ctrl before the key) |

## 6. Input: mouse, selection, clipboard, paste

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| F1 | Legacy X10 encoding (`CSI M` + 3 bytes, release=3, coord ≤222) | `mouse.py:57-64` → `mouse.rs` | done |
| F2 | SGR encoding (`CSI < b;x;y M/m`) | `mouse.py:51-64` → `mouse.rs` | done |
| F3 | Modifiers in the button code (+4/+8/+16) | `mouse.py:26.34-36` → `mouse.rs` | done |
| F4 | Wheel as buttons 64-67 (press and release in SGR) | `mouse.py:21-23.46-47` → `mouse.rs` | done |
| F5 | Buttons 8/9 (back/forward) | `mouse.py:22,30` → `mouse.rs` | done |
| F6 | Motion with button (`?1002`) and without button (`?1003`) | `view.py:1199-1205` → `app.rs` | done |
| F8 | Drag selection with a 3 px threshold | `view.py:1145-1200` → `app.rs` | done |
| F9 | Selection on an absolute anchor (survives scrolling and output) | `view.py:854-881`, `term.py:1056-1075` → `select.rs` | done (A21's reflow still kills the anchor, same as Python) |
| F12 | Selection: copy on release to PRIMARY; `copy_on_select` copies CLIPBOARD | `view.py:1168-1176` → `app.rs` | done: on release it copies PRIMARY always and CLIPBOARD only with `copy_on_select`; without a real drag (`a == b`) it copies nothing and the user's PRIMARY stays intact — same condition as `view.py:1168-1176` |
| F13 | Ctrl+Shift+C copies (selection or viewport), Ctrl+Shift+V pastes | `view.py:1271-1277.1239-1254` → `app.rs` | done |
| F14 | Button 2 pastes PRIMARY | `view.py:1152-1154` → `clip.rs` | done |
| F15 | Bracketed paste wraps the payload in `200~`/`201~` | `term.py:1082-1091` → `select.rs` | done |
| F16 | Clipboard survives exit / system integration | `Gtk.Clipboard` → `clip.rs` (`arboard`) | done in CLIPBOARD; PRIMARY dies with the app (it is the owner of the selection) |
| F19 | Click with mouse mode active returns to live before reporting | `view.py:1136-1141` → `app.rs` | done |

## 7. Drawing: font, geometry, colors

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| G1 | Font in **points** converted by DPI (Pango equivalent) | `view.py:640-659` → `glyph.rs` | done |
| G2 | Cell metric from the advance of `M` (not from the typographic box) | `view.py:661-680` → `glyph.rs` | done |
| G3 | Layout baseline, not a fixed ascent | `view.py:1011-1021` → `glyph.rs` | done |
| G4 | Whole cell (fractional advance does not "slide" the grid) | `view.py:689-704` → `render.rs` | done |
| G5 | Bold and italic (real face; its own bold-italic) | `view.py:689-704,992` → `glyph.rs` | done (no synthesis: without the face, falls back to regular) |
| G6 | Glyph fallback by preference list | Pango/fontconfig automatic → `glyph.rs` | done |
| G7 | Fallback for emoji/CJK | `HISTORY.md` P10 → `glyph.rs`, `atlas.rs`, `render.rs` | done: rescue chain by name (a missing font does not become another one) and two passes per glyph (color before mask); CBDT emoji goes to its own RGBA atlas, drawn by the image pipeline. On screen: 😀🔥🚀👍 and CJK, arrows, Greek; `emoji_comes_out_colored` and `font_missing_not_becomes_other` in `cargo test` |
| G8 | Ligatures/shaping | Pango (P9) → `shape.rs` | done: `rustybuzz` per segment of the same face/style, with a cluster → cell map and a cache per (face, mode, text). A ligature in a mono font does not merge cells (the `calt` swaps each one's glyph), so the cell keeps being one character. Key `ligatures` (`off`/`standard`/`all`); cursor and selection break the ligature. On screen: JetBrains Mono with `->`, `!=`, `fi`, `ffi`, `fl` (Xvfb) |
| G9 | Window padding in px | `config.py` `window_padding_width` → shader (`origin`) | done |
| G10 | Cell-surplus background up to the border (no black stripe) | `view.py:926-934` | done (background painted by a whole-grid instance) |
| G12 | Cursor: block/beam/underline + blink | `view.py:1058-1079`, `828-838` → `render.rs`, `app.rs` | done: `cursor_blink on` blinks at 500 ms, a key/paste relights it, `CSI 6 q` pair wins; SGR 5 uses the same phase |
| G13 | Underline at the width of the run | `view.py:1022-1025` → `render.rs` | done |
| G14 | Selection drawn as inversion | `view.py:943.953-973` → `render.rs` | done (the default resolves through the other end) |
| G15 | `?2026` holds the frame (200 ms cap) | `view.py:296-324` | done: `frame::Gate` (`SYNC_MS = 200`), tested without display (transaction open, immediate close, an app that never closes does not draw on every read) |
| G16 | Coalesced frame (~16 ms), not one draw per read | `view.py:286-320` | done: `frame::Gate` (`FRAME_MS = 16`), fed on `Wake` **after** the `drain` (the `?2026` is only visible after parsing). 900 frames: 1499–1585 draws → **63–74**, same fps (`BENCHMARKS.md`) |

## 8. kitty protocol images

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| H1 | APC `ESC _ G … ST` routed to the subsystem | `term.py:944-953` | done: `State::Str` gained the APC buffer (APC only: DCS/PM/SOS remain discarded) and ST hands the body to `graphics::handle` |
| H2 | `a=t` transmit / `a=T` transmit+display | `graphics.py:188-212` | done: `a=t` stores the image, `a=T` stores and places it. Verified with a real `kitten icat` (`blueman.png` and two `icat` on the same screen) |
| H3 | `a=p` display an already transmitted image | `graphics.py:184-192` | done: places the image by id; a nonexistent id answers `ENOENT` (like Python) |
| H4 | `a=d` delete (by id and everything) | `graphics.py:175-183` | done: `d=i` removes the image and its placements, `d=a` clears everything (and the app releases the texture on the next frame) |
| H5 | `a=q` capability query | `graphics.py:173-181` | done: answers `OK` for `t=d` with `f=24/32/100` and `EINVAL` for transmission by file/memory — that answer is what makes `icat` go on instead of giving up |
| H6 | Chunking `m=1` | `graphics.py:146-155` | done: the control data of the **first** chunk applies to the transmission and `m` does not accumulate (it was the Python bug documented there); 64 MiB cap for a chunk that never closes |
| H7 | `f=24/32` (raw RGB/RGBA) and `f=100` (PNG) | `graphics.py:190-196` | done: all three in the core; the PNG is decoded in the app (`png` crate) and the file magic beats the lying `f=` |
| H8 | `o=z` (zlib) | `graphics.py:161-168` | done: `flate2` (zlib, not gzip); a crooked zlib answers `EINVAL:zlib` instead of taking the parser down |
| H9 | `s=`/`v=`, automatic `i=`, default `c=`/`r=`, `X=`/`Y=` px offset | `graphics.py:100-106.193-207` | done: default `c`/`r` = `ceil(width/cell)`; without `i=` the id comes from a counter. **Fixed on the way:** the app never passed the real cell size to the parser (it stayed at the 10×19 default), so the image came out 5×3 where 4×2 fit |
| H10 | `C=1` does not move the cursor; otherwise it goes down `rows` | `graphics.py:184-187` | done: `Output.move_cursor`/`lines` and the `apc_done` moves the cursor down by the image lines; `C=1` leaves it where it is |
| H11 | `q=` silences the response | `graphics.py:170` | done: any `q` other than 0 silences the response (`icat` sends `q=2`) |
| H12 | Image follows the scroll and is pruned from the history | `graphics.py:74-96` | done: the placement lives in a screen line (negative = history); `scroll_up` moves everyone down, region scrolling discards, the scrollback cap and reflow prune the unreachable. In drawing the screen line is `linha + view_off` — verified with a screenshot: the image comes back in the right place when going up in the history |
| H13 | `ED 3` clears images | `term.py:481-485` | done: `CSI 3 J` clears images and placements along with the history; `ED 2` does **not** clear (verified in both directions) |

## 9. Window: tabs, workspaces, chrome

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| I1 | Tab = term + pty with its own lifecycle | `window.py:140-211` → `app.rs:Tab` | done: `Tab` has term/pty/rx/view/selection; `drain` reads them all |
| I2 | New tab inherits the cwd of the current tab (last descendant of the pty) | `window.py:140-153.904-919` | done: `/proc/<pid>/cwd` of the deepest descendant; `cd /tmp` → new tab in `/tmp` |
| I3 | Close tab (closes pty, removes the page, picks the next) | `window.py:184-201` | done: `Ctrl+Shift+W`; a dead-shell tab closes by itself |
| I4 | Tab title by OSC 0/2, else `basename(cwd)`, else `bzmux` | `window.py:277-286` | done: `Tab::title` (OSC → cwd → `bzmux`) |
| I5 | Workspace = set of tabs, with name and renumbering | `window.py:798-848` → `app.rs:Workspace` | done: `parked` keeps the other integers (pty alive); switching is a swap; logical order is stable |
| I6 | Sidebar with cards (`n`, name, title, k tabs) | `window.py:289-301` → `render.rs:sidebar` | done: 18-cell column; active card inverted; title truncates with `…` |
| I7 | Sidebar hidden with 1 workspace | `window.py:396-400` | done: `side_w` = 0 with one workspace; the grid gets the columns back |
| I8 | Rename workspace inline (double click, Enter/Esc) | `window.py:315-356` | done: double click on the card; the 1st key replaces (like the selected `Gtk.Entry`), Enter commits, Esc discards, `\|` is the cursor |
| I9 | Middle click closes tab (on the label) and workspace (on the card) | `window.py:208-212.307-309` | done in Rust (Python **did not** have it: only the card); the left click switches |
| I10 | Drag tab to reorder | `window.py:173` → `app.rs:bar_drag` | done: moves one tab width with the button held on the bar and the tab changes place |
| I11 | Mouse wheel on the tab bar | `window.py:100` | done: cycles between the tabs; X11 sends press+release per notch, and the pair is deduplicated (10 ms) |
| I12 | Toast (error/warning) with queue and timeout | `window.py:411-465.677-740` → `render.rs:toast` | done: inverted stripe in the **last** line (the top belongs to the bar), 3.5 s, repeated becomes `(×N)` |
| I13 | Visual bell: label flash for 800 ms (800 ms, `BELL_BG`) | `window.py:255-275` → `render.rs:tabbar` | done: `color3` for 800 ms, including in a background tab |
| I14 | Audible bell (`beep`) | `window.py:666-669` | done (`bell audible` writes BEL to stdout; no toolkit `beep`) |
| I15 | Tab/workspace shortcuts (Ctrl+Shift+T/W/N/Q/F2, Alt+N, Super+N/Q) | `window.py:854-898` | done: Ctrl+Shift+T/W/N/Q/F2, Ctrl+Shift+1..9, Alt+1..9, Super+N/Q/1..9 |
| I16 | Shortcut digit by raw keycode (survives ABNT2) | `window.py:70-84` → `digit_of` (physical key) | done |
| I17 | Window title = active tab title | `window.py:375.921-926` | done: `<title> — bzmux`, only when it changes |
| I20 | Closing the last tab closes the workspace; the last workspace closes the app | `window.py:196-197.836-845` | done: the shortcut and the **child exit** enter the same path (`close_tab_at` → `drop_empty_workspace`). Verified live: 2 workspaces × 1 tab, `exit` in the visible one → parked one promoted and app alive; 1 workspace × 1 tab → app closes |

## 10. Window: lifecycle, geometry, state

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| J1 | Closes the window when the shell ends | `window.py:250-253` → `app.rs` | done |
| J2 | Initial geometry 980×640 and size restoration | `window.py:24-31.467-478` | done: without state it opens 980×640 (checked with `xdotool`: `Geometry: 980x640`); with state it restores the size (900×600 requested → 900×600). Whoever requests a grid (`--cols/--rows`) does **not** restore or save — that is the role of `PYTERM_GEOM` there, and it is what automation and bench need |
| J3 | Geometry persistence (`$XDG_STATE_HOME/bzmux/window.json`) | `state.py:16-63` | done: `state.rs` (pure), atomic write (tmp + rename, no tmp left over — tested), `XDG_STATE_HOME` with `~/.local/state` as fallback and I/O error swallowed on close |
| J4 | Reject invalid geometry (disconnected monitor, garbage) | `state.py:46-63` | done: `state::fit` requires `w/h` ≥ 320×200 and the corner **inside some monitor**. Live: 100×80 → 980×640, corner at 9000,9000 → 980×640, crooked JSON → 980×640, and the file is rewritten sane |
| J5 | Maximized is state separate from the normal size | `window.py:1009-1012` | done: `Resized` only updates the normal size when the window is **not** maximized; the file stores `max` next to `w/h`. Live: maximized 1920×985 → file `{w:900,h:600,max:true}` → reopens maximized → unmaximizing goes back to 900×600 |
| J6 | Wayland: do not save x/y | `window.py:975-978` | done: `backend_x11()` decides by `WAYLAND_DISPLAY`/`DISPLAY`; without X11 the position is neither requested nor written, and `state::save` simply omits `x`/`y` (tested). Caveat: there is no Wayland session on this machine, so this line is code + test, not live verification |
| J7 | `SIGUSR1` = immediate dump; `SIGTERM` = clean exit | `window.py:503-508` | done: `SIGUSR1` = dump right away, `SIGTERM` = dump and clean exit (the handler only raises an `AtomicBool`; the one who acts is the loop) |
| J8 | `--quit-after SEG` (used by automation) | `cli.py:35` | done: `--quit-after SEG` closes by itself and leaves the dump (checked: dump written and process gone) |
| J9 | Dump of the text of all tabs to a file | `window.py:929-945` | done: dump with a header per workspace and per tab, scrollback (last 40) and viewport, marking the tab `[fundo]` |
| J11 | One instance per PID file, without killing someone else's process | `debug.sh` (Python) → `tools/devkill.sh` | done |

## 11. Configuration and CLI

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| K1 | kitty format (`key value`, `#` comments, repeated wins the last) | `config.py:66-85.264-287` → `config.rs` | done |
| K2 | Merge with the embedded default config | `config.py:289-305` → `DEFAULT_CONF` | done |
| K3 | Per-line error does not take the file down; unknown key is an error | `config.py:264-287` → `Config::Error` | done |
| K4 | `shell` (empty = `$SHELL`; exports the real shell to the child) | `config.py:87-97` → `pty.rs` | done |
| K5 | `font_family`, `font_size` | `config.py:99-125` → `cli.rs` + renderer | done (CLI beats the file) |
| K6 | `cursor_shape`, `cursor_blink` | `config.py:127-143` → `render.rs` | done (shape and blink; hot reload) |
| K7 | `scrollback` | `config.py:149-151` → `cli.rs` | done |
| K8 | `window_padding_width` | `config.py:153-155` → shader + `cell_at` | done |
| K9 | `bell` (`none`/`visual`/`audible`) | `config.py:131-133` → `App::ring` | done |
| K10 | `copy_on_select` | `config.py:145-147` → `app.rs` | done |
| K11 | `debug_log` (path or `off`) | `config.py:157-167` → `cli.rs` | done |
| K12 | Auto-reload with a 250 ms debounce | `config.py:307-329` → `App::check_config` | done |
| K13 | Config errors become a toast (1st + count) | `config.py:331-345` | done: startup and reload show "config kept — line N: 'key': reason (+K)"; saving it right shows "config reloaded" |
| K14 | `-c/--config`, `--cmd`, `--debug-socket`, `--debug-log` | `cli.py:31-41` → `cli.rs` | done |
| K15 | `--quit-after`, `--dump-to`, `--selftest`, `--install-shortcut` | `cli.py:35-96` | done: `--selftest`, `--quit-after`, `--dump-to` (the desktop shortcut belongs to `install.sh`) |
| K16 | `--version`, `--help`, code 2 for invalid usage | `cli.py:85-101` → `cli.rs` | done |
| K18 | `.desktop` + icon (`--install-shortcut`) | `shortcut.py:34-136` | done: `.desktop` + SVG come out of `install.sh` (with `StartupWMClass`); there is no `--install-shortcut` as a binary flag — the installer owns it, having both would be a second source |
| K19 | Stable WM_CLASS (`StartupWMClass`) | `cli.py:130-131` | done: explicit `WM_CLASS` (`with_name("bzmux","bzmux")`), not inherited from the executable — checked by running the renamed binary (`/tmp/fz…`): `WM_CLASS(STRING) = "bzmux", "bzmux"` |

## 12. Debug socket and instrumentation

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| L1 | Unix socket transport, JSON per line, several requests per connection | `debugsock.py:217-240` → `debugsock.rs` | done |
| L2 | `state` with grid, cursor, title, history, cell, font, dpi | `window.py:531-546` → `app.rs` | done: `state` brings grid, cursor, title, history, cell, font, `pid`, `xid` (Xlib/Xcb; 0 on Wayland), `config`, `errors`, `dpi`, `advance`, `padding`, `pin`, selection and metrics |
| L3 | `text` (`view`/`history`/`screen`, `start`, `count`) | `debugsock.py:86-93` | done: `view`/`history`/`screen` with `start`/`count` (`screen` is the live grid, without the view offset) |
| L4 | `write` (raw bytes to the pty) | `debugsock.py:106-111` | done |
| L5 | `keys` (spec → keys, going through the encoder and shortcuts) | `debugsock.py:94-99` → `app.rs` | done: `keys` goes through the **same** path as the keyboard (local shortcut before the encoder, `handled` in the response); `ctrl+shift+n` returns `handled:true` with no byte |
| L6 | `resize` | `debugsock.py:100-103` | done |
| L7 | `prof` (counters per stage) | `debugsock.py:104-105` | done: `draw`/`parse`/`pty` + `writes`/`write_kb`/`eagain` (counter on the pty write) |
| L8 | `log` (turn logging on/off at runtime) | `debugsock.py:112-113` | done: `log` turns on, turns off (`off`) and swaps the path at runtime, also counting as the current config |
| L9 | `quit` (the response leaves before closing) | `debugsock.py:114-115` | done |
| L10 | `--debug-socket auto` (`$XDG_RUNTIME_DIR/bzmux-<pid>.sock`) | `debugsock.py:33-43` → `debugsock.rs` | done |
| L11 | Socket `0600` + `SO_PEERCRED` (same uid) | `debugsock.py:146-157,180` → `debugsock.rs` | done |
| L12 | Do not delete someone else's file; an orphan socket only goes away with `ECONNREFUSED` | `debugsock.py:122-144` | done: a file that is not a socket is never deleted (error `path exists and is not a socket`) and the orphan only goes away when `connect` refuses |
| L13 | Error always `{"ok":false,"err":...}`, never takes the app down | `debugsock.py:70-119` → `debugsock.rs` | done |
| L15 | `--dump-to`/`SIGUSR1` for inspection without a socket | `window.py:947-949` | done: `--dump-to` and `SIGUSR1` write the dump without a socket |

## 13. Platform and distribution

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| M2 | POSIX pty through a single API (`PtySize`, `spawn`, `read`, `write`) | `pty.fork` (POSIX only) → `pty.rs` | done |
| M3 | Linux X11 + Vulkan | dev environment | done |
| M10 | Installer: `bzmux` command, menu, icon, `--uninstall`, `--check` | `install.sh` (pipx + system deps) → `install.sh` (copies 1 binary) | done |

## 14. Tests and verification

| # | feature | ref (Python → Rust) | state |
|---|---|---|---|
| N1 | Headless core check suite (`checks.py`, 27 checks) | `checks.py:820-838` → `checks.rs`/`checks_n2.rs`/`checks_n5.rs` + `checks_app.rs` | done: **31 checks** in `--selftest` (25 core + 6 on the app side: `argv_parsing`, `kitty_keyboard`, `mouse_modes_and_sgr`, `window_state`, `debug_socket`, `desktop_shortcut`), and `cargo test` runs the same list (`selftest_complete`/`app_checks_pass`) |
| N2 | Config/CLI checks | `checks.py:125-287` | done: `config_file`, `config_options` and `scrollback_and_bell` in `--selftest` (kitty format, merge with defaults, per-line error with reason, history cap, RIS does not own the config, BEL only counts) + 7 tests of argv and of the CLI > file > default merge in `cli.rs` |
| N3 | Keyboard/mouse checks | `checks.py:325-343.554-562` | done: 6 kitty keyboard tests (push/pop/set/query, flags 1/2/8/16, `:2`/`:3`, DECCKM, flag 16 text) and 3 mouse ones (modes 1000/1002/1003/1005/1006, SGR vs X10, wheel and modifiers) |
| N4 | Socket checks | `checks.py:702-810` | done: `DebugServer` accepts a `Sink` (the `EventLoopProxy` in the app, a channel in the test), so 8 tests run without a window: one-line JSON framing, `ok`/`err`, several requests on the same connection and in a single `write`, `0600`, orphan vs live socket vs a file that is not a socket, `resolve("auto")` |
| N5 | Reflow/selection/anchor checks | `checks.py:380-520` | done: `history`, `scroll_anchor`, `redraw_keeps_history_position`, `draw_damage`, `selection`, `truecolor` and `style_contrast` in `--selftest` (`paste_markers` and `selection_follows_history` already existed) |
| N6 | `--selftest` in the binary (runs N1..N5 without display) | `cli.py:92` → `checks.rs` + `checks_app.rs` | done: `bzmux --selftest` (**31 checks**), in `check.sh` and in CI |
| N7 | Automated visual verification (capture + comparison) | `tools/pixel.py`, `xtest.py` → `tools/pixel.py`, `tools/visual.sh` | done: `tools/visual.sh` opens the window in its own `Xvfb`, waits for the first frame, captures and **compares** — new content has to change a pixel, SGR 41/42 has to paint `color1`/`color2`, text has to fall inside the cell and a still screen cannot repaint differently. Exits 0/1, so it works as a gate |
| N8 | Reproducible benchmark with a locked grid | `tools/benchrun.py` → `tools/benchrun.py` | done |
| N9 | Rust x Python ladder | `bench.sh` → `tools/bench.sh` | done |

---

## 15. Count per area

There is no table here: the panel at the top of this file is the **only** count.
A second copy ages silently (this one existed and fell 9 items behind).

## 16. The order the migration followed

The order was not the one of the tables: it was by **what unlocks daily use** and
by dependency. It closed like this, each slice with the criterion that closed it:

| slice | what it was | the criterion that applied |
|---|---|---|
| F0 | hygiene: `fmt`, `clippy`, a single command in the hook and in CI | `./tools/check.sh` green from scratch |
| F1 | usable terminal before any chrome | `vim` and `mc` with mouse, copy between two instances, `bat` with italic/bold, `dmesg` navigable by keyboard |
| F2 | reflow and anchors | narrowing the window with a long history preserves the paragraph and the selection; `CSI 3J` does not skip the view |
| F3 | configuration in a file, with hot reload | font/color/padding/bell/scrollback change live; an invalid line **keeps** the previous config |
| F4 | tabs, workspaces and chrome | shortcuts, cards, rename, drag to reorder, wheel on the bar, title by OSC, bell flash |
| F5 | input protocols (kitty keyboard) | flags 1/2/8/16 checked on the pty, not on paper |
| F6 | kitty images | `icat` draws, scrolls along and is pruned from the history |
| F7 | cursor blink (the rest became new activity) | frame A/B with the cursor blinking and with SGR 5 |
| F8 | Linux ports | X11 + Vulkan draws with a real GPU; for free, the GL backend too (`WGPU_BACKEND=gl`) |
| F9 | splits | it is not parity: it went to `BACKLOG` before starting |

**From here on the queue is [`BACKLOG.md`](BACKLOG.md)**, and it is of new
activity: pending parity no longer exists.

### 16.1 Rules that applied along the way

They stay recorded because they explain the line format of this file: a slice
only started with `cargo test` green and its manual scripts passing; a refactor
that was not a requirement became `BACKLOG`, it did not enter the slice; and
every migrated feature brought the equivalent test from `checks.py` — when there
was no equivalent, the manual script was written in the line itself.

### 16.2 Checks: where they live

The smoke checks run in `--selftest` (and in `cargo test`, from the same list):
**25 in the core** (`checks.rs`, `checks_n2.rs`, `checks_n5.rs` — parser, grid,
scrollback, selection, damage, config, graphics) and **6 on the app side**
(`checks_app.rs` — `argv_parsing`, `kitty_keyboard`, `mouse_modes_and_sgr`,
`window_state`, `debug_socket`, `desktop_shortcut`). The core does not know
`cli.rs`/`keys.rs`/`mouse.rs`/`state.rs`/`debugsock.rs`, and that is why there
are two owners: Python's `checks.py` covered the 31 in a single file.

Three Python checks do not have the same name here, because the coverage is the
same with another cut: `graphics_image_and_offset` + `graphics_follow_history`
became `graphics_kitty` (offset and history in a single test, over the same
images), and `shortcut_remove_only_own_name` became `desktop_shortcut` (installs,
checks the `.desktop`/icon and removes it on `--uninstall` — Python's "only
deletes what is its own" is the removal half).

A new check enters the list and **compares with the expected** — "it did not
panic" is not a check. What requires a window is not a check: it is
[`tools/visual.sh`](../tools/visual.sh).
