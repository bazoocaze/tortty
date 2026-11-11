# History

Concise history of past activities. A maximum of 10 lines per activity.

## 2026-11-01 — phase 1: the feasibility experiment

Question: does rewriting in Rust solve distribution and performance? The
decision to abort was "if it does not do ≥3× the Python's MB/s and does not open
vim/mc, it does not pay".

- **Core + app from scratch** (`tortty-core` without GUI/pty/GPU, `tortty` with
  `portable-pty` + `winit` + `wgpu` + `swash`). Verified with 23 core tests,
  `vim`, `mc` and `dmesg` on screen.
- **Measurement**: 8–9.5× the Python in fps and MB/s on doom-fire, 29–46× in
  drawing, `dmesg` of 1397 lines in 2.1 ms against 42.6 ms. Numbers and
  condition in [`BENCHMARKS.md`](BENCHMARKS.md).
- **Debug socket** with the same JSON protocol as the Python, so the same
  `benchrun.py` measures both.
- **Pitfall recorded**: `pkill -x bzmux` killed the pipx `bzmux` (then the same
  name as this binary) and `pkill -f bzmux` kills the shell that called it. Hence
  `tools/devrun.sh` + `tools/devkill.sh` by PID file with a `/proc/<pid>/exe`
  check.
- **Pitfall recorded**: `request_inner_size` is a suggestion to the WM; without
  correcting in cell steps the benchmark measures 60x18 thinking it measured
  61x18 (it is the `want`/`adjust_left` in `app.rs`).

## 2026-11-01 — decision to migrate, inventory and docs

- **Decision**: migrate (the 3× criterion was beaten with room to spare;
  distribution too). Single route `winit`+`wgpu`, without keeping the GTK stack
  as an alternative.
- **Parity inventory**: 4 independent surveys of the Python code
  (core/parser, render/colors/images, input/socket/infra, config/tabs/CLI)
  became the 250 lines of [`MIGRATION.md`](MIGRATION.md), each with the Python
  `arquivo:linha` and its state here.
- **Docs restructured** in the format of the Python project: `MIGRATION`,
  `BENCHMARKS`, `DEBUG`, `PORTABILITY`, `LIMITATIONS`, `BACKLOG`, `HISTORY` +
  `AGENTS.md`.

## 2026-11-01 — hygiene: fmt, clippy, one command

- `rustfmt.toml` and workspace lints (`all`, `correctness`, `suspicious`,
  `pedantic`); 113 warnings zeroed. `unsafe_code = deny` with a local `#[allow]`
  on the single block (`SO_PEERCRED`).
- `tools/check.sh` runs on pre-commit (`.githooks`) and in CI — same script, so
  hook and CI do not disagree. Verified: `./tools/check.sh` green.
- **Bug**: the debug socket's `keys` verb had a second key table (F5 came out
  with 1 byte, `alt+left` lost the modifier). It now calls the real encoder
  (`keys::encode_parts`). Verified: `tab`=1 B, `shift+tab`=3 B, `f5`=5 B,
  `alt+left`=6 B.

## 2026-11-02 — full SGR (C5, C6, C7)

- `Cell` gained `ul` (style 1–5) and `ul_color`; `flags` now has italic, faint,
  blink, strikeout and overline. `SGR 22` turns off bold **and** faint, like
  xterm; `21` is double underline (which is what `bat`/`delta` want).
- Drawing: italic uses the italic face from `fontdb`; bold-italic got its own
  lookup (`bold_italic`), because the old candidate order made "bold+italic"
  come out upright. Curly/dotted/dashed are made of rectangles — there is no
  curved geometry in the pipeline.
- Verified on screen: 14 SGR combinations in a single session (`bold`, `faint`,
  `italic`, strikeout, overline, the 5 underlines, `58` in 256 and in truecolor,
  reverse) + 4 new core tests (26 in total).

## 2026-11-02 — mouse reporting (F1-F6, F19, F20)

- `mouse.rs` ports `mouse.py`: button/modifier/motion, SGR (`?1006`) and the old
  protocol (release = button 3, coordinate saturating at 222).
- `?1005` (UTF-8 coordinate) implemented — the Python ignores it; without it, a
  column above 222 in the old protocol reports wrong by saturation.
- `app.rs` gained `on_cursor_moved`/`on_mouse_input`/`on_wheel` (`window_event`
  was over 100 lines). With the mouse enabled, a click goes back to live before
  reporting (F19); the wheel while reading the history keeps scrolling the view,
  it does not go to the app.
- Verified with a probe that reads the *received* bytes (not the drawing): click
  1 and 3 and the wheel under `?1000/?1006` → `ESC[<0;11;4M`, `ESC[<0;11;4m`,
  `ESC[<2;11;4M`, `ESC[<64;…M` + `m`. And with `vim -c 'set mouse=a'`: a click on
  line 6 moved the cursor to 7.7 (read in `state` and in the status bar).

## 2026-11-02 — selection, clipboard and paste (F8, F9, F12-F16, G14, D5-D7)

- `crates/core/src/select.rs`: absolute anchor (`(line, column, generation)`) and text
  by `span`, like the Python — the selection survives scrolling and incoming
  output, and dies when the history is rewritten (`CSI 3J`/RIS/reflow), instead
  of copying the wrong text.
- Drawing: real inversion, including for a cell without color (the default
  resolves through the other extreme); the same rule holds for `SGR 7`.
- `clip.rs` with `arboard` without the image feature (the binary stays at 10 MB,
  no `libX11`: arboard speaks the protocol directly).
- Shortcuts: Ctrl+Shift+C/V before the encoder (otherwise Ctrl+Shift+C becomes
  `^C`), the middle button pastes PRIMARY, Shift+PgUp/PgDn/Home/End navigate the
  history, the wheel is worth 3 lines per notch and a trackpad fraction
  accumulates.
- **Pitfall measured**: `xdotool` coordinates are not `winit`'s (window frame).
  The drag test calibrates the offset by moving the mouse to two known points
  and reading `cursor_px`/`cell_at` from `state`.
- Verified end to end: drag over "CCC DDD" → inversion on screen, `state` with
  anchors `a=(1,0) b=(1,6)`, Ctrl+Shift+V and the middle button delivered
  "CCC DDD" to `cat` (checked in the file and on screen).

## 2026-11-02 — F1 closed on the keyboard, scroll and indicator items

- **D8**: `↑N` indicator in the top right corner (inverted badge, with the atlas
  glyph). Without it, scrolling back is indistinguishable from an app that is
  stuck.
- **D6/D7**: Shift+PgUp/PgDn/Home/End navigate; a real key goes back to live (a
  modifier does not, otherwise the Shift of Shift+PgUp would already reset
  before scrolling).
- **E5**: Ctrl+Space is now NUL; the generic table produced `CSI 1;5 `.
  Checked byte by byte with the `keys` verb, which now returns the hex.
- **E9**: dead keys already work without our code (winit composes and delivers
  text); verified with `xdotool key dead_tilde a` → `ãéù`.
- **Tooling**: `tools/devkill.sh` now sweeps the leftovers of this checkout (the
  PID file only keeps the last one). `cargo build` replaces the file, so the old
  process shows up as `tortty (deleted)` — the suffix must be stripped before
  comparing, otherwise the leftover survives and answers in place of the new
  build.

## 2026-11-02 — initial grid: a lost column and the child born too early

Symptom: `bat`, `dmesg` and `seq` "shuffled" in the first KB. It was not the
parser (the same output read from a file reproduced identically): it was two
things.

- `round(cols*cw)` asked for one pixel too few (1304.4 → 1304 → 99 columns). It
  became `ceil`.
- the child was born before the final size, so the WM's first resize arrived
  while it was writing. Now `apply_size` runs first and `spawn_shell` only
  happens on the first applied size.

Verified: `--cols 100 --rows 10` → `stty size` = `10 100` and a 250-character
line breaks at 100/100/50. (My initial suspicion of "interleaved text" was a
misreading on my part: lines 17 and 18 of `keys.rs` broke in the right places.)

## 2026-11-02 — installer: `tortty` is the Rust one, `tortty-python` is the Python one

The Python was already renamed in pipx (`~/.local/bin/tortty-python`), so the short
name was free and this repo's `install.sh` takes it over.

The installer is the opposite of the Python one in size: there it is distro
packages (`python3-gi`, `gir1.2-gtk-3.0`, cairo, pipx) and a pipx step; here it
is **copying one file**. What it actually does is what breaks when it is
missing: it warns if the command is already in `PATH` and is not this one,
installs with `install` instead of a
symlink to `target/` (otherwise `cargo clean` leaves the command broken), writes
the `.desktop` + SVG icon, and checks `--version` + `ldd` without "not found"
before saying it worked.

A detail that only showed up later: the first version of the Python installer
(`tortty-python/shortcut.py`) deleted `bzmux.desktop`/`bzmux.svg` as "old name" — and
those were the Rust ones, so running that installer deleted this repo's menu
entry. Proven in a temporary `XDG_DATA_HOME` (`install()` removed both files),
reported and fixed in `tortty-python`; revalidated here with
`--install-shortcut`/`--remove-shortcut` over the real files (same md5), and the
case became a test in `bzmux/checks.py`.

Two bugs of mine along the way: the mode function was called `install`, so the
`install -m 755` inside it called itself (silent recursion); and the write test
used `-w` on a directory that did not exist yet, which asked for `sudo` to
install in `/tmp`. Functions renamed (`do_install`) and the test walks up to the
existing ancestor.

## 2026-11-02 — F2: reflow on resize (A22, A23)

Before: `resize` only stretched/cut each line. Narrowing the window **ate** the
text on the right; widening left the text chopped into 80-column pieces. Now the
resize of the main buffer joins the "soft" (`wrapped`) lines into paragraphs and
re-breaks everything at the new width.

Decisions worth recording:

- **cut** only of whitespace with default background (`Cell::is_blank`). A space
  with a painted background is content — `vim`'s status bar, `git diff --color`,
  `bat --style=full` are full-width painted lines;
- **cursor** does not come from `cy`: it comes from the *offset inside the
  paragraph*, so 15 cells in 5 columns end on the 3rd line, column 5, with a
  pending wrap (it is the VT's `wrap_pending`, not an occupied column);
- **wide pair** is not split: if the cut would fall between the character and
  its continuation, the line breaks one column earlier;
- `scrolled` stays monotonic and `abs_gen` goes up: absolute numbering changes,
  so the selection dies (same as the Python) instead of copying the wrong text.

Proven with 6 new tests (`reflow_*`, 38 in total) and in the app: 30 paragraphs
of 152 columns, `100x20 → 50x20` re-breaks without losing anything, `→ 120x20`
joins it back at the full width.

## 2026-11-02 — F2: the view is not dragged by output (D10)

Whoever scrolled into the history and stayed reading was pushed: a line arrived
and the view went down with the content. Now `drain` saves `scrolled` and the
history size before the `feed` and asks `Term::view_after_output` for the
correction — `off + lines_that_left - lines_that_fell_off_the_ceiling`.

The second term matters: with the history full (ceiling of 1000), each new line
pushes `d` and discards `d` of the old ones — if the discard were not in the
count, the view would walk into the void.

Verified in the app: `view_off` 27 → 29 → 31 with two `echo` arriving, and the
first visible line stayed "echo linha-12". Two core tests (including with
`scrollback_limit = 3`).

### F2 closed

One detail was missing: `CSI 3J` emptied the history but `view_off` stayed at
the old value, so the screen showed a line that does not exist (or the wrong
line). Now `drain` clamps `view_off` to the history size after every `feed`.
Verified with the app: `view_off` 29 → 0 and empty history.

The deviation stays on record: reflow kills the selection instead of remapping
the anchor (same behaviour as the Python). Remapping is improvement M-P, because
`reflow` already knows the `(paragraph, offset)` of each old line.

## 2026-11-02 — `tortty --selftest` (N6)

A command that answers "is the emulator still whole?" without display, without
GPU and without pty: 14 checks in the core (`checks.rs`), each one building the
scenario and comparing against the expected — parser/cursor, autowrap and `?7l`,
SGR and ED, truecolor and theme palette, DEC special graphics + DA1 + CPR, alt
screen and DECSTBM, history and ED3, reflow, selection over history, bracketed
paste, CSI with intermediate (DECSCUSR and soft reset), config, color OSC and
damage.

It went into `tools/check.sh` (which runs on pre-commit) and into CI. Three of my
expectations were wrong and the check caught them: backspace after `z	w`
overwrites the `w`, `DECSCUSR 4` stores `(shape 1, blink false)` and `ECH 2`
leaves `"  c"`.

## 2026-11-02 — release by tag (M8)

`.github/workflows/release.yml` (tag `v*`) builds `x86_64` and `aarch64`, and the
packaging is `tools/release.sh` — the same command as local, not a parallel CI
path. It:

- checks `--check-version vX.Y.Z` against `Cargo.toml` (a tag pointing at other
  code is the easiest mistake to make and the nastiest to discover);
- runs `tortty --version` **on the packaged binary** — on cross, via
  `qemu-user-static`; without qemu, it says so in the log instead of pretending
  it verified;
- builds `tortty-<version>-<triple>.tar.gz` with binary + LICENSE + README and
  prints the `sha256`, which is the format `install.sh --from-release` expects.

Tested locally: 3.9 MB package, extracts, runs and answers `tortty 0.1.0`; a
wrong tag (`v0.2.0` with `Cargo.toml` at `0.1.0`) exits with code 1.

Still missing to really close M8: `windows-latest` and `macos-14`/`macos-13` in
the matrix — they depend on M6 (ConPTY) and M7 (Metal), which are in
`PORTABILITY.md`. The MIT license came along (the package carries the file).

## 2026-11-02 — cursor blink and toast (G12, I12, K13)

Two things that were missing so the terminal would not "look broken":

**Blink (G12).** The phase is not accumulated state: it comes from `blink_epoch`
(`Instant`) divided by 500 ms, so there is no timer that can lag or drift.
`cursor_blink on` blinks, a key or a paste **relights** it immediately (a cursor
that vanishes exactly when you look at it is worse than a fixed cursor) and
`CSI 6 q` with an even shape (4/6) asks for a fixed cursor and wins. The text's
`SGR 5` uses the same phase — the attribute existed since F1 but did not
animate.

The loop only wakes mid-period when there **is** something blinking
(`blink_active`), and the "is there a cell with SGR 5?" test is recomputed in
`feed`, not on every tick. Verified by A/B of captures: with `on`, 432 px
alternate between frames at 260 ms; with `off` (hot-reloaded), the captures are
identical.

**Toast (I12/K13).** The config file is edited in an editor — which does not
show our log. Now the error shows on screen: `config kept — line 3:
'afffont_size': chave desconhecida (+1)`, an inverted band on the first line
(the prompt is at the bottom, and the warning is the terminal's, not the
program's), 3.5 s and it goes away by itself. The same message in a row becomes
`(×N)` instead of a queue. Saving correctly after an error shows
`config recarregada`.

Two details that only appeared while testing: the first tick re-parsed the file
and the error came out twice (now the startup `mtime` already counts) and the
"reloaded" condition looked for `config:` with a colon, which the new message
did not have.

Benchmark after all of it: 950–985 fps / 40.8–42.3 MB/s at 61x18 — inside the
documented range, no regression.

## 2026-11-02 — tabs (F4a: I1-I4, I13, I15-I17, I20)

Rust stopped being "a terminal" and became tortty: `Tab` holds `term` + `pty` +
what is about looking (view, selection, drag), and `App` has `tabs: Vec<Tab>`
with one active.

What came with it:

| item | decision |
|---|---|
| draining **all** tabs | the channel is not unbounded: a `yes` in a hidden tab would fill memory. What is reserved to the active one is only the blink and the view |
| cwd of the new tab | `/proc/<pid>/cwd` of the **deepest descendant** (via `task/<pid>/children`), not of the shell: whoever is in front, `vim`/`mc`, decides. Verified: `cd /tmp` → new tab born in `/tmp` |
| title | OSC 0/2 → `basename(cwd)` → `tortty`; the window follows the active tab (`rust_term — tortty`), only when it changes |
| tab bar | one line above the grid (the shader `origin` is `pad + bar`), and the grid of **all** tabs shrinks by one line; with a single tab there is no bar |
| BEL | the tab label keeps `color3` for 800 ms — including on a background tab, which is the point |
| shortcut digit | **physical** key (`Digit1..0`): `Ctrl+Shift+1` arrives as `!` and on ABNT2 every shifted digit is another symbol |
| `--cmd` | only applies to the first tab: repeating the debug command on the second would not open a shell |

Shortcuts: `Ctrl+Shift+T` (new), `Ctrl+Shift+W` (close), `Alt+1..9` (`0` = 10th).
The tab whose shell dies closes itself; closing the last one closes the app.

Verified over the socket (`tab op=new/select/close`, independent draining per
tab, `rows` from 10 → 9 with the bar) and with XTest (`ctrl+shift+t`, `alt+1`,
`alt+3`, `ctrl+shift+w` checked via `state`), plus captures for the BEL flash and
the inversion of the active tab. Benchmark with one tab: 993 fps / 42.7 MB/s — no
regression.

Still missing to close F4: workspaces and sidebar (F4b: I5-I11), drag to reorder
(I10) and the wheel on the bar (I11).

## 2026-11-02 — F4 closed: workspaces and the rest of the chrome (I5-I11, I15, I20)

**Workspaces**: `parked` keeps the other whole sets (live pty, own grid) and
switching is a swap of contents — there is no term copy and no dead process. The
logical order is rebuilt on switch: swapping `parked[slot]` in place scrambled
the cards when the current one went to a higher position (seen in `state`, with
the names in each other's place). The sidebar appears with 2+ workspaces and eats
18 columns from the grid; the tab bar still runs edge to edge, on top of it.

Shortcuts: Ctrl+Shift+T/W (tab), N/Q (workspace), Ctrl+Shift+F2 (config in the
editor, in a new tab — P33), Ctrl+Shift+1..9, Alt+1..9 and Super+N/Q/1..9. Left
click on the card switches, **middle closes** (in the Python only the card had
that), double click renames in place (`|` is the cursor, the 1st key replaces the
old name like a selected `Gtk.Entry`, Enter saves, Esc discards).

Three pitfalls only appeared while testing:

| symptom | cause | fix |
|---|---|---|
| tab bar on line 2 | drawn at `y=0` with the origin already at `pad+bar` | draws at `y=-bar`, the reserved band |
| click on the bar hit the wrong tab | `tab_at` subtracted the sidebar, which pushes the **grid**, not the bar | the bar starts at the window edge |
| wheel on the bar switched nothing | X11 sends **two** events per notch (press and release) with the same direction, and the second undid the switch | pair deduplicated in 10 ms |

Verification: `state` with `tabs/ws/ws_names` checked at every step (workspace
order stable when switching and when closing), capture of the sidebar with and
without the bar, and the wheel/drag checked with a real mouse.

## 2026-11-02 — sidebar and tab bar share line 1

The sidebar started on line 2 when the tab bar appeared: the bar ran edge to edge
and the left column had a hole on the first line. Now the bar starts where the
grid starts (`x0 = 0`, to the right of the sidebar) and the first card starts at
`y = -bar`, on line 1, beside it. `card_at` came along: the top of the card is
`pad`, not `pad + bar`. The order of the queries matters — `card_at` before
`tab_at`, otherwise the click on the card of line 1 became a click on the tab.

## 2026-11-02 — the backlog gained what is not migration

The two documents did the same job halfway: `MIGRATION.md` had a "proposed
improvements" section and `BACKLOG.md` said the queue was `MIGRATION.md`. Now
each has a single subject — `MIGRATION.md` answers "how much is left for Rust to
do what Python does"; `BACKLOG.md` keeps what migration does not ask for, with a
**trigger** per item ("only what has a 'when X shows up' enters"), because an
improvement without a trigger is a wish.

First item of the new list: **M-Q, chrome overlay**. The tab bar, the sidebar
cards and the toast are drawn with rectangle + glyph (they inherit the font, the
palette and the cell alignment for free, and pull no toolkit — that is the reason
the project exists), but each one is written twice: the drawing in `render.rs`
and the click test in `app.rs`, with the same coordinates in separate formulas.
Three bugs this week were that divergence. The item is to turn the chrome into a
list of elements with a declared hit-test, in the same `wgpu`, when the fourth
element shows up — before writing the draw/click pair for the fourth time.

## 2026-11-02 — the boundary: panel = parity, backlog = new activity

A rule of nature, now written in both files: **if Python does it, it is parity
and lives in `MIGRATION.md`; if it does not, it is new activity and lives in
`BACKLOG.md`**. A piece of work has one owner only.

What the audit found: **44 of the 252 panel lines were not parity** — 25 of them
still in the queue (`missing`, counting as work to do) and 19 already done but
marked as improvement. And 13 of the 15 backlog items already had a line in the
panel, with the same work in two states. Worse: two duplications inside
`MIGRATION.md` itself — section 15 was a second copy of the panel, frozen 9 items
behind, and section 16 was the improvements queue.

| before | after |
|---|---|
| 252 lines, 25 of them improvement in the queue + 19 improvement done | **208 lines, parity only** (145 done / 14 partial / 46 missing / 3 no) |
| 15 backlog items, 13 duplicating the panel | **19 new activities** + 17 closed, none with a line in the panel |
| 67.9% "equivalent ready" counting extras | **73.1% parity** (what is new activity does not enter the count) |

Two backlog lines were disguised parity and went back to the panel: `M-A`
(per-line damage — Python does it, `view.py:330-364`) and `M-G` (kitty keyboard
— Python does it in `kitty_kbd.py`; only flag 4 remained as new activity).

Six queue lines were orphaned when they left the panel and got their own item:
`M-R` (OSC 52/7/133), `M-S` (F13-F24 and `KP_*`), `M-T` (`?1016`), `M-U` (DECALN
and combining) and `M-V` (context menu, which is the declared trigger of `M-Q`).

## 2026-11-02 — new activity that was already in the program

Consolidation, not new work: what Python does **not** do and was already done
here leaves the parity panel and is recorded only in this line. The verification
is what ran on top of them — `cargo test` (52 in `tortty-core`), `--selftest`
14/14, `mc`/`vim`/`dmesg` in the manual script, plus the specific proofs cited
(item by item, when there was one).

**Parser and grid:** G1 charset with SO/SI; wide char (`wcwidth`) with
continuation cell; configurable tab stops (HTS/TBC/CHT/CBT); origin mode `?6` and
switchable autowrap `?7`; DCS/APC/PM/SOS consumed up to ST (in Python they leak
on screen); DECSC saving SGR/origin/autowrap and not just the cursor; `ESC D`/
`ESC E` (IND/NEL, no-op in Python); clamp of `n` in IL/DL/ICH/DCH/ECH; SU/SD; REP
(`CSI b`); length ceiling in the `csi`/`osc` states.

**Colors and drawing:** full SGR (italic, faint, blink, strikeout, overline,
color and underline styles) — proven with `bat` and `git diff`; OSC
4/104/110/111/112 as *set* at runtime; configurable palette/theme, with
`color0..15`/fg/bg/cursor in the file — checked by pixel sampling against
`tortty-python` (`mc` and `htop` identical).

**Input:** wheel with fractional delta (trackpad) accumulating; `?1005`
coordinate in UTF-8 above the 222 ceiling (unit tests in `mouse.rs`).

**Window and CLI:** grid pinned by the socket survives WM rounding (correction
loop `dw/dh`); `--cols`/`--rows`/`--dpi`/`--no-vsync`/`--prof` (checked with
`stty`: 100x10 requested, 10 100 on output).

**Tooling:** `--selftest` in the binary (14 checks without display or GPU), in
`check.sh` and in CI.

## 2026-11-02 — kitty keyboard (E12-E15, E17-E20, B13)

The child turns flags on with `CSI > u` and the keys change shape. The flags live
in `Term` (one per tab, stackable) and the encoder only consults: **state in the
core, encoding in `keys.rs`** — the same division as `kitty_kbd.py`.

| flag | what changes |
|---|---|
| 1 (disambiguate) | ESC becomes `CSI 27u`, Ctrl+a becomes `CSI 97;5u`, arrow with modifier becomes `CSI 1;5A`; arrow without modifier stays on the legacy path (and respects DECCKM) |
| 2 (event types) | `:1`/`:2`/`:3` — a held key gave `CSI 97;1:2u` five times and `CSI 97;1;3u` on release |
| 8 (all keys) | `a` becomes `CSI 97u`; Shift+letter stops being plain text |
| 16 (text) | `CSI 97;97u`, with the text after the code |

Two things that only appeared with a real keyboard:

1. **The release has to reach the encoder.** winit delivers `Released` and the
   app discarded it in `if event.state == Pressed`. With flag 2, the release is
   information. But a local shortcut cannot run twice (Ctrl+Shift+N would open
   two tabs) — hence the shortcut staying on the press only, and the release
   going on its own to the encoder.
2. **A swallowed shortcut swallows the release.** `Ctrl+Shift+1` switched
   workspace and left `CSI 49;1:3u` in the child: the press was the app's, the
   release leaked. And X11 releases **Shift and Ctrl before** the key, so the
   consumed-press marker cannot be cleared by them (it was the blind `take()`
   that caused the leak on the first attempt).

Known deviation, and it is the Python's too: Shift+letter comes out as
`CSI 65;2u` (the character already comes after the shift) and not `CSI 97;2u` as
in kitty — the base key would require a layout table. F3 came out as `CSI 1;2R`
(kitty's form) and **not** `CSI 13;2~`: the Python's `kitty_kbd.py` has F3 as
`13 ~`, which is xterm's Shift+F3. Both deviations are recorded in
`LIMITATIONS.md` §Keyboard, with the why — that is where the information stays,
so it is not lost in the history.

Verification: `state`/`text` over the socket with a child in raw mode printing
the hex of what it receives — the query responses (`?0u`, `?3u`, `?11u`) and the
keys (`97u`, `97;5u`, `97;1:2u`, `97;1:3u`, `27u`, `1;5A`, `1;2R`) read from the
screen, and the local shortcut leaving no trace in the child.

## 2026-11-02 — automation without `xdotool` (L2, L3, L5, L7, L8, L12, L15, J7-J9, K15)

What was missing so that `benchrun.py` and CI would not depend on a mouse and on
screen capture. No GPU here: it is instrumentation.

**Full `state`.** It gained `xid` (from `RawWindowHandle`, Xlib/Xcb; 0 on
Wayland — whoever is going to capture pixels asks the app, not X), `config`,
`errors` (the config error is a fact even when the previous config is kept),
`dpi`, `advance`, `padding` and `pin`. `text` already had `screen`; it was
checked.

**`keys` through the real keyboard path.** The verb now goes through the local
shortcut **before** the encoder, like a physical key: `ctrl+shift+n` answers
`handled:true` and sends no byte at all (`ctrl+a` is still `01`). Before, it
talked straight to the encoder, and the answer lied about what the app would do.

**`prof` with writes.** `writes`, `write_kb` and `eagain`, counted at the single
pty write door — which became a `pty_write` in the app (before it was six
scattered `pty.write`). `log` turns on, off (`off`) and switches file at runtime.

**Dump and lifecycle.** `--dump-to`, `--quit-after SEG`, `SIGUSR1` (dump right
away) and `SIGTERM` (dump and clean exit). The signal handler only raises an
`AtomicBool`: the loop acts, and it already wakes every 250 ms. The dump has a
header per workspace and per tab, scrollback (the last 40 lines) and the
viewport, marking `[background]` on what is not the active tab — checked with two
workspaces and two tabs: 78 lines, three headers, process gone after `SIGTERM`.

**Orphan socket (L12).** Before, any occupied path was deleted on startup; now a
file that is not a socket is never touched (`path exists and is not a socket`) and
the orphan only goes away when `connect` refuses. Verified with a text file in
place of the socket: it survived with the contents intact.

## 2026-11-02 — Wave 0 + Wave 1: reconciliation and the check battery (B3, F12, K18, K19, N2-N5)

Four items that sat in the panel as pending **were not pending code**, and four
missing checks became code.

**Reconciliation (Wave 0).** `B3` (C1 0x80-0x9F) and `F12` (PRIMARY/CLIPBOARD)
were already in the Python's behaviour — proven by running, not by reading: C1 in
the middle of `abc` gives 5 cells (`a\u{80}b\u{9b}c`), and the release copies
PRIMARY always with CLIPBOARD only under `copy_on_select`. `K18` revealed that
the `.desktop` and the icon already come out of `install.sh` with
`StartupWMClass`: `--install-shortcut` as a binary flag **does not** exist and
should not — two owners for the same artifact is the defect we just fixed in
Python. `K19` was real code: `WM_CLASS` is now explicit (`with_name`), checked by
running the renamed binary — without it the `.desktop`'s `StartupWMClass` would
only match by luck.

**Checks (Wave 1).** `--selftest` went from 14 to 24 and `cargo test` from 52 on
the board to 93 tests. Each area got its own module (`checks_n2.rs`,
`checks_n5.rs`) so it would not become a three-thousand-line file.

- **N2** — config: kitty format, key-by-key merge with the defaults, whole-line
  and end-of-line comment, repeated key (the last one wins), error **per line**
  with the reason; history ceiling (cuts by the oldest, not by the arriving
  one), RIS does not own the configuration, BEL only counts (who decides
  visual/audible is the view). Plus 7 argv tests: `--help`/`--version` exit
  through the same error path, an unknown option names the option, and the merge
  is CLI > file > default.
- **N3** — kitty keyboard end to end (`Term` parser + encoder):
  `push/pop/set/query` with modes 1/2/3, flags 1/2/8/16, `:2` and `:3`, DECCKM
  deciding the legacy path, Super entering the count only with flag 8. Mouse: the
  five modes, SGR beating `?1005`, X10 with release always 3, wheel with and
  without modifier.
- **N4** — the socket server got the seam it was missing: a `Sink` trait (the
  `EventLoopProxy` in the app, a channel in the test). This is not a test
  ornament — it was what prevented exercising the socket without a window. 8
  tests: framing of one JSON line per request and per response, several requests
  on the same connection, two in a single `write`, `0600`, and the three
  conflicting decisions of `clear_stale` (orphan goes, live socket is not
  stolen, a file that is not a socket is never deleted).
- **N5** — history as a window (never more than `rows`, clamps at the oldest),
  absolute anchor against drift when the history hits the ceiling (13 lines of
  drift proven), `?2026` + `2J`/`3J` killing the absolute anchor but not the
  distance pin, per-line damage (1 dirty cell dirties 1 line; ED2/scroll/resize/
  RIS dirty the grid), selection (reversed order, inclusive ends, no space on the
  right), truecolor in both syntaxes (`;` and `:`) surviving the resize, and WCAG
  contrast of the theme.

**A seam that was missing in the infrastructure:** the `--selftest` checks did
not run under `cargo test` — only in the binary. A green
`cargo test -p tortty-core` said nothing about them. Now there is a
`selftest_completo` that runs `run_all()` and shows the failures, and the
verification was done by mutation: breaking `view_after_output` makes
`scroll_anchor` fail with "expected 7, got 2"; removing the "not a socket" guard
from `clear_stale` makes the L12 test fail.

## 2026-11-02 — five bugs the checks found before they became routine

The agents that wrote the Wave 1 battery reported suspicions at the end of the
report. None was invented: the five below were reproduced and now have a test.

**The D10 drift at the history ceiling** (the serious one). `view_after_output`
subtracted the lines the scrollback discarded. Reproduced: history at the
ceiling, `off=3`, 10 new lines → the count returned 3 (showing line `43`) when
the absolute anchor asked for 13 (line `33`, which the user was reading). That
is: in the case D10 exists to serve — a large log pouring while you read the
past — the view walked **one line per new line**, and the reader lost the place.
The right count is `(off + d).min(len)`: whoever handles "the line being read
died" is the clamp, not the subtraction. The `scroll_anchor` check now demands
the two counts agree at the ceiling too, and the verification was by mutation:
restoring the subtraction breaks the check.

**Ctrl+Space under the kitty flags.** The NUL was returned before the kitty form,
so with flag 1 (disambiguation) or 8 (all keys) the app received `0x00` instead
of `CSI 32;5u`, and the modifier was lost. Now `Space` enters the kitty table,
but **only** when the app asked for disambiguation or all keys: with flag 2 alone
(the `pi` case) the NUL stays, because that is what readline/vim expect.

**`?1005` outside DECRQM.** `CSI ? 1005 $ p` answered 0 with the mode on: the
mode existed in the parser and not in the query table.

**Usage error exit code.** `tortty --cols abc` exited with **0**: `main` only
mapped two message prefixes to 2 and the rest fell to 0, so an automation script
walked over an invalid command. The rule went to `cli.rs` (owner of the
messages) in a function with a test: only `--help`/`--version` exit with 0,
everything else with 2. The distinction is by **prefix** ("tortty …"), not by
content — the usage error message brings the help glued at the end, and looking
for "USAGE:" in the text classified an error as help.

**Invalid UTF-8 on the socket.** `lines()` returned `Err` and the connection died
**without a response**: whoever sent a crooked byte got EOF instead of JSON. Now
reading is by `read_until` with `from_utf8_lossy` (the same
`decode("utf-8","replace")` as the Python): the byte becomes U+FFFD, the request
goes on to the app and the response is a normal `err` — checked with `0xFF` in
the middle of a line: `{"err":"cmd desconhecido: …","ok":false}` and the
connection alive for the next request.

**Also fixed, minor:** `--debug-socket ""` stopped trying `bind("")` (an empty
path is "I did not ask for a socket", like an empty `debug_log`), and the
`--selftest` report had `config_file` **twice** (the old one from `checks.rs`
became `config_defaults`).

## 2026-11-02 — M-W: child death enters the `close_tab_at` path

`Ctrl+D`/`exit` on the visible tab killed the whole app even with another parked
workspace. The culprit was a shortcut in `about_to_wait`: when the dead tab was
the only one in the workspace, the code went straight to `event_loop.exit()` and
skipped `drop_empty_workspace` — which is what promotes the parked one. That is:
the shortcut (`Ctrl+Shift+W`) had the right behaviour and the **shell's natural
end** did not.

Now child death calls `close_tab_at(dead)` like any other close, and the app only
exits when `drop_empty_workspace` has no workspace left to promote
(`exiting = true` + `redraw()`, and `RedrawRequested` closes the loop). A drawing
decision comes with it: `exiting` does not close the loop by itself — the frame
does, so the app does not exit in the middle of a drawing.

Verified live, both ways: **2 workspaces × 1 tab**, `exit` on the visible one →
the parked one is promoted, the socket connection stays and the content of the
promoted workspace appears; **1 workspace × 1 tab**, `exit` → the app closes
(socket drops, process exits). `I20` in `MIGRATION.md` went from `partial` to
`done` and `M-W` left the `BACKLOG.md` queue.

## 2026-11-02 — Wave 2: the drawing gate (G16, B28, G15)

The app drew once per pty read. Under a continuous dump that was one drawing per
read chunk — and since drawing competes with reading, the total cost went up
without the screen getting any faster.

Now there is `frame::Gate` (`crates/app/src/frame.rs`), a pure type, without
`winit`, with four tests:

- **rhythm**: whoever already has a scheduled time does not reschedule → at most
  one frame every 16 ms;
- **`?2026` open**: nothing is drawn until the close, because drawing now would
  show the **middle** of each repaint (which is what makes the screen look like it
  jumps several screens);
- **close**: the final state goes out immediately, without waiting for the
  rhythm;
- **200 ms ceiling**: an app that opens the transaction and forgets to close it
  does not freeze the screen — and after the ceiling the gate opens **another**
  200 ms window, otherwise it would be back to drawing on every read, which is
  exactly the defect it exists to avoid.

One order matters: the pty `Wake` now does `drain()` **before** asking about
`?2026`. Asking first, the state read was the previous read's and the first frame
of every transaction came out in the middle of it.

Measurement (900 doom-fire frames, 61x18, 5 shots, numbers in `BENCHMARKS.md`):
draw calls drop from **1499–1585** to **63–74** with the same fps (1012–1032),
which is ~10x less CPU drawing on the same load. Live, with the transaction
coming from the pty (20 lines with a pause): 4 drawings in 0.4 s — one per
ceiling, as designed — and the whole final state on the grid after the close.

## 2026-11-02 — Wave 3: the window remembers where it was (J2–J6)

`crates/app/src/state.rs`, as pure as the `state.py` over there: JSON file in
`$XDG_STATE_HOME/tortty/window.json` (with `~/.local/state` as fallback), atomic
write by tmp + rename, and no state failure disturbs the close. Nine tests,
without display: `fit` accepts a sane rectangle, rejects garbage size
(`< 320×200`), rejects a corner outside every monitor (disconnected monitor),
accepts when there is no known monitor, and handles the absence of position
(Wayland); the file comes back identical, leaves no `.tmp` and a dozen pieces of
junk (`""`, `{`, `[]`, `null`, `"texto"`, `{"x":1}`) fall into "open at the
default" instead of breaking.

**Order of precedence**, which is the decision of this wave: a grid requested on
the command line (`--cols`/`--rows`) **turns off** remembering — it is a
measurement, and it is what automation and the bench need (the role of
`PYTERM_GEOM` in Python). Without an explicit grid, the remembered state decides,
if it passes `fit`; and the last resort is the Python's 980×640, not the grid
size — whoever wants an exact grid asks for the grid.

**The pitfall the Python had already documented** (`window.py:1009`): saving the
**client** position and restoring it on the frame drops the window by one title
bar per run. Here the saved position is the one from the `Moved` event, which is
the outer one, the same one `with_position` consumes. Verified in three runs in a
row: `120,90` requested, `120,90` saved, no pixel of drift.

**Maximized is separate state** (J5): `Resized` only updates the normal size when
the window is not maximized, and `max` lives next to `w/h` in the file. Live:
maximized at 1920×985, the file keeps `{w:900, h:600, max:true}`, reopens
maximized, and unmaximizing gives the 900×600 back.

**Wayland** (J6): `backend_x11()` decides by `WAYLAND_DISPLAY`/`DISPLAY`, because
`winit` does not tell which backend it chose; without X11 the position is neither
requested nor saved. Caveat recorded in the panel line: there is no Wayland
session on this machine, so it is code + test, not live verification.

Live verification (X11 + `xdotool`): no state → `980x640`; state `900×600` at
`120,90` → restores both; `100×80` → `980x640`; corner at `9000,9000` →
`980x640`; crooked JSON → `980x640` with the file rewritten sane.

The file schema is **the same** as the Python's (`x`, `y`, `w`, `h`, `max`), in
the same path, on purpose: the two terminals read each other's state — what
changes is only the formatting of whoever wrote last (the Python writes with
spaces, Rust in compact JSON). This machine's `~/.local/state/tortty/window.json`,
for example, was written by tortty-python and is read by Rust without conversion.

## 2026-11-02 — Wave 5: the kitty images (H1–H13)

The last big block of visible functionality. `crates/core/src/graphics.rs`
is the analogue of `graphics.py` over there: APC, base64, zlib, PNG (only the
IHDR, the decoder stays in the app), `a=t/T/p/d/q`, `m=1`, `C=1`, `q=`, `c/r/X/Y`,
placements on the screen line and the lifecycle (scroll, reflow, `ED 3`). Twelve
tests in the module + six in `Term` + one check in `--selftest`, all without
display.

In the app: one texture per image (cache by `versao`, which changes on every
transmission — the app may reuse the `i=`), its own pipeline sampling RGB instead
of the atlas mask, and the drawing **between** the grid text and the chrome/cursor,
which is kitty's order. The images are drawn with `linha + view_off`, so they
follow the history.

**Two defects that only appeared when I tried to use it for real.** Implementing
the protocol and watching `kitten icat` work required fixing two things that were
not the protocol:

- the pty sent `ws_xpixel`/`ws_ypixel` = **0**. That is where (via `TIOCGWINSZ`,
  not from `CSI`) `icat` gets the resolution from: with zero it gives up with
  *"terminal does not support reporting screen sizes in pixels"*. Now `PtySize`
  carries `cols*cw` × `rows*ch`;
- the parser kept the **default `cell_size` of 10×19** — the app never passed the
  real cell size. That got the default `c=`/`r=` wrong (a 48 px image came out
  5×3 cells instead of 4×2) and the `CSI 14/16 t` responses. As a bonus, `CSI
  14 t` started being answered: the Python does not answer it either, and without
  it the `icat` otherwise gives up. Both are noted in the H9 and H2 lines.

**Live verification** (X11 + `xdotool` + screenshot): `kitten icat` of a PNG
draws; two `icat` on the same screen use two textures; an image pushed into the
history (31 lines) appears in the right position when going up with `Shift+PgUp`
(MARK_BEFORE / image / MARK_AFTER, with `view_off=7`); `printf '\e[3J'` clears
images and history, `ED 2` does not.

I also instrumented the socket's `state` with `images` and `placements`: without
that the only way to check the placement was to look at the screen.

## 2026-11-02 — Windows and macOS leave the panel (they become queue)

The panel had been carrying `M6` (Windows), `M7` (macOS) and `M9` (signing) as
`missing` since the first day. They are not: **Python never ran outside Linux**.
By the single-owner rule (parity = what Python does), a new target is queue, not
migration. They left section 13 and went to [`BACKLOG`](BACKLOG.md) as:

| before (`MIGRATION` §13) | now (`BACKLOG`) |
|---|---|
| `M6` Windows 10/11 (D3D12 + ConPTY) | `M-X` |
| `M7` macOS arm64/x86_64 (Metal) | `M-Y` |
| `M9` notarization/signing | `M-Z` (behind `M-X`/`M-Y`: without a compiled target there is nothing to sign) |

What **stays** in the panel is what Python actually delivers: `M4` (Wayland) and
`M5` (GL backend — GTK runs without Vulkan, so "runs on Linux" is parity). Along
the way, `M2` stopped promising ConPTY in the same API (that is a promise of
`M-X`) and `M8` (release matrix) is now stated as Linux x86_64/aarch64.

Panel: 194 done / 3 partial / **5** missing / 3 no = **205** lines, **95.4%**.
Missing only: `A17`/`G17` (per-line/region damage in drawing), `G8`
(shaping/ligatures) and `M4`/`M5`.

Pointers fixed in `LIMITATIONS.md` (platform and signing), `INSTALL.md` (release
matrix) and `PORTABILITY.md` (which now says explicitly that it is analysis, not
queue). `PORTABILITY.md` remains the place for the **how** of each target; the
**when** is the backlog.

## 2026-11-02 — Damage in drawing (A17/G17) and the GL backend (M5)

Two fronts, both with the same problem: **this sandbox stopped drawing**. I
record first what was verified, then what was left unverified.

**A17 — drawing uses the core's damage.** `crates/app/src/damage.rs` (new, 11
tests without display) decides, per frame, between "everything" and "these
lines". The rule lives in one place only, and it is the `Assinatura`: tab
(monotonic identity — the index does not work, closing and opening brings the
index back), columns, visible lines, history scroll, visible selection, blink
phase **when there is an SGR 5 cell in view**, theme generation and fg/bg. Equal
signature ⇒ the line-by-line `dirty` holds; different ⇒ whole frame. The
`Renderer` keeps the instances **per line** (`por_linha`) and only concatenates;
the final list is still a single one, in the blend order from before (grid →
images → chrome → cursor → badge → toast).

Two things the change required and that are easy to forget: damage is consumed
**only when the frame was presented** (a skipped frame cannot clear the mark,
otherwise the dirty line stays old forever), and the cursor blink does not enter
the signature — otherwise the whole grid would be rebuilt twice a second because
of a cursor that does not even live in it.

**G17 — damage by region.** It stays `partial` on purpose: damage already works
per line when building, but the *draw* is still the whole screen. The swapchain
does not guarantee preserving the previous frame's content, so drawing only the
band requires its own target (scene texture + blit) — that is another
architectural drawing, not an `if`. Writing it as "done" would be a lie.

**M5 — GL backend.** Three real defects, all with the same symptom ("GL is no
good") and different causes:

| symptom | cause | fix |
|---|---|---|
| `gl not compatible with provided surface` | instance created without a display handle; EGL needs it to match the surface | `new_with_display_handle_from_env(window)` |
| translucent window (the terminal behind showed through) | `alpha_mode: caps.alpha_modes[0]` picked `PreMultiplied` on GL | explicit `CompositeAlphaMode::Opaque` |
| adapter chosen "by accident" | `request_adapter` returned the first one | `choose_adapter`: hardware beats CPU; a tie follows `wgpu`'s order |

The line `tortty: GPU: <backend> <board>` also went into stderr (the first
question when the window does not draw on another machine) and the `last_frame`
field in `state`.

**What was not verified, and why.** In this sandbox (container without
`/dev/dri`, without a compositor and with the `cargo` cache read-only) the window
even answered `skipped (timeout/occluded)` on *every* frame — no pixel comes out,
on any backend; and a new crate does not compile. Therefore: A17/G17 have the
logic covered by tests and the counters exposed in `state`, but the
**screenshot script is missing**; `M4` (Wayland) has no compositor to exercise;
`G8` (shaping) would need `rustybuzz`, which cannot be downloaded here. All of
that is in `DEBUG.md` ("degraded sandbox"), which is where the next session will
look first.

## 2026-11-02 — Confinement: what tortty has to withstand

We ran a whole session inside `sandbox` (bubblewrap, `~/sandbox`). Three
things appeared, and the heaviest one became queue (`M-AA`), not a last-minute
fix:

- **Drawing without GPU**: with `$HOME` read-only and no `/dev/dri`, the app comes
  up, the pty works, `state` answers — and **no frame comes out** (`skipped
  (timeout/occluded)` on every frame, llvmpipe). A terminal that is left with a
  dead window in a container is useless. Diagnosis and the fallback to software
  go into `M-AA`; that is where it gets measured, not here.
- **State without writes**: `state::save_to` already failed without blowing up
  (the terminal has to run confined), but it failed **silently** — nobody would
  understand why the window is not remembered. Now a line goes to stderr, and
  there is a test (`save_in_impossible_place_does_not_blow_up`).
- **Install into a blocked target**: `install.sh` already exited with code 1, but
  the message was `sudo`'s dump ("If sudo is running in a container..."). Two
  fixes: a target inside `$HOME` without write permission does not try to
  escalate privilege (it says "home read-only? use `--prefix <writable dir>`"),
  and a system target warns beforehand that `sudo` does not run without a
  password there.

A note on least privilege: the natural request ("free up `~/.cargo`") is too
broad. What is missing is `~/.cargo/registry` — and `sandbox` already knows how to
grant exactly that (`-w ~/.cargo/registry`, or `write("~/.cargo/registry")` in a
profile). In this session not even that went through, because the `ro` mount of
`$HOME` on the host makes the bind inherit `ro`; it is a mounting problem, not a
sandbox policy one. Until then, `CARGO_HOME` in a writable directory (`/tmp`)
solves it without granting anything.

## 2026-11-02 — A17 closed: damage became real partial drawing

`A17` was `partial` because the logic existed and nothing was saved: `state`
showed **20 lines rebuilt per written line** on a grid of 20. Instrumenting
answered it on the spot — `frames_all` and `global_why` (new `state` fields) said
the culprit was the core, not the renderer:

| symptom | cause | fix |
|---|---|---|
| every output rebuilt the grid | `scroll_up` marked `dirty_all` — and in a terminal **every new line scrolls** | full-screen scroll became `scroll_shift`: drawing shifts the finished lines and rebuilds only the blank one coming in |
| `zle` redraws the prompt and rebuilds everything | `IL`/`DL` marked the whole screen | they mark from the cursor to the end of the region |
| `ED 0`/`ED 1` rebuilt everything | same | they mark only the side they erase (`ED 2`/`ED 3` stay global) |
| scroll in both directions | a net displacement of zero would lie (the line that leaves becomes blank) | `scroll_conflict` forces a rebuild |

The instance is now built **relative to the line** (y = 0) and the displacement
enters at concatenation — that is what allows running the finished line without
rewriting coordinates or rebuilding glyphs.

Measured on this machine (60x12 grid, socket `state`):

| operation | before | after |
|---|---|---|
| `echo UMA` | 20 lines (grid) + 1 global frame | **1 line** + 0 global |
| `seq 1 100` | 20 per burst | 12 once (whole screen changed), then 1 per line |
| `mc` (repaints everything on purpose) | 20/frame | 64 lines in 13 frames, 3 global |

Checked on screen: `seq 1 30` aligned, `mc` with panels, colors, DEC frame and
key bar. Two new core tests pin the contract of the scroll and the precision of
`IL`/`DL`/`ED`; the suite went from 74 to 79.

Only `G17` stays `partial`: the *draw* is still the whole screen (the swapchain
does not preserve the previous frame; drawing a band requires scene texture +
blit).

## 2026-11-02 — G7 closed: color emoji and CJK (a font that does not exist and a font that does not draw)

Emoji and CJK had been blank forever: the text arrived on the grid (`state`
showed the right characters) and the glyph did not exist. There were **two**
causes, and neither was "missing font":

| symptom | cause | fix |
|---|---|---|
| CJK blank | a single fallback face, which did not cover CJK | fallback chain (`sans-serif`, `DejaVu Sans`, CJK, emoji) |
| 🔥 came out as a gray stroke | `query()` fell back to the generic family when the **name** asked for a missing font: "Noto Sans Symbols 2" became FreeMono, which maps the emoji and draws an outline | a missing font returns `None`; the chain skips it |
| 😀 blank even with `Noto Color Emoji` installed | `DejaVu Sans` maps U+1F600 and produces no bitmap; the first cmap with `gid != 0` won and the glyph came out empty | resolution in two passes: **color** bitmap over the whole chain, then mask |
| emoji without color | there was no RGBA atlas and no pipeline for it | `Atlas::new_rgba` + draw call in the image pipeline (bind group with the image layout, not the mask atlas one) |
| `write_texture` overflowed the buffer | the mask atlas' pending white block (1 byte/pixel) went along into the RGBA one | the RGBA constructor rebuilds the pending block at 4 bytes/pixel |

Checked on screen (Vulkan/RADV, 80x20 grid): `emoji 😀 🔥 🚀 👍 🥰 🌈` in color,
`CJK 日本語 汉字`, arrows, Greek, and emoji inside bold. Two new tests pin the
contract: an emoji has to come out **in color** and a missing font cannot become
another font.

No regression in performance on doom-fire (no emoji in the load): 872–963 fps /
37.5–41.4 MB/s at 61x18, `draw` 0.69–0.72 ms/call.

## 2026-11-02 — M5 closed: the GL backend draws with a real GPU

`M5` was `partial` because GL only initialized; the visual verification had died
along with the container's `/dev/dri`. Running under `sandbox @gpu,dev`, with the
device nodes visible, `WGPU_BACKEND=gl` comes up on `Gl AMD Radeon Graphics
(radeonsi, renoir, ACO)` and draws: `last_frame: drawn`, text, colors, cursor and
`mc -V` in the screenshot. Without the `TORTTY_NO_TITLE` probe — the pending X
error panic was llvmpipe's, not GL's.

It is recorded in [`DEBUG.md`](DEBUG.md) that the first question in the face of
"the window does not paint" is `ls -la /dev/dri`: without the nodes, both
backends stay on llvmpipe and nothing reaches the screen.

## 2026-11-02 — G8 closed: ligatures (shaping) and the `ligatures` key

`G8` was the last `missing` in the panel: `swash` rasterizes glyphs, it does not
shape, and Python shaped through Pango. What decided the design was measuring the
font, not its documentation:

| font | features in GSUB | `->`/`fi` with `calt` on | with `calt=0` |
|---|---|---|---|
| DejaVu Sans Mono (app default) | `liga`, `dlig` | **2 glyphs, no ligature** | same |
| Fira Code | only `calt` | `->` → glyphs `[1186,1458]` | `[1221,1580]` (pure) |
| JetBrains Mono | only `calt` | `->` → `[1742,881]` | `[855,1051]` (pure) |
| DejaVu Sans (proportional, control) | `liga` | `fi` → **1 glyph, cluster `[0,0]`** | 2 glyphs |

That is: **a mono font does not merge cells**. `calt` swaps each cell's glyph for
a piece that joins the neighbor's, and the advance is still one cell per cell.
That simplified everything: the cell is still one character, drawing only asks
HarfBuzz for the right glyph **with the run's context**. The "one glyph covers N
cells" path only shows up in a font that merges (the DejaVu Sans case above) and
is guarded by an advance rule: if the glyph does not fit N cells (±10%), the run
goes back to cell by cell. A cluster out of order (RTL) also goes back —
reordering bidi is not done here.

How it turned out:

- `crates/app/src/shape.rs` — shaping per run of the same face and style,
  cluster → cell map, cache by `(face, mode, text)`. The plan speaks position
  **inside the run** (it was a bug: with an absolute column, every run outside
  column 0 drew the wrong glyph — caught in the capture, fixed with a test).
- a **single-cell** run is not shaped: without context there is no ligature, and
  that is the case of whoever paints cell by cell. Doom-fire (2400 runs of 1)
  stays free.
- **cursor and selection break the ligature**: the cursor cell (and its neighbor)
  become a run on their own. Without that the cursor paints in the middle of a
  glyph.
- key `ligatures` (`off`/`standard`/`all`, default `standard`) in
  [`CONFIG.md`](CONFIG.md). `off` turns off the **ligatures**, not shaping: Arabic
  and Devanagari depend on it and keep being shaped.
- test font **JetBrains Mono** (SIL OFL 1.1) embedded in `assets/fonts/`, with the
  license: `cargo test` proves the ligature without depending on an installed
  font.

Measured cost (`cargo test --release -- --ignored --nocapture`, 64-cell line):
**88.5 µs** per new line — 32.4 µs parsing the font and 56.4 µs shaping — and
**57 ns** on the cache. Since damage is already per line (A17), only the line
that changed pays it.

Visual verification **without a window in the owner's session**: `Xvfb :77` +
`DISPLAY=:77` + `import -window <id>` (recipe in [`DEBUG.md`](DEBUG.md)).
JetBrains Mono on screen: `->` becomes an arrow, `!=` becomes ≠, `fi`/`ffi`/`fl`
ligated, and the cursor in the middle of `fi` undoes the pair.

## 2026-11-02 — N1, N7 and M-AA: a test that does not open a window, a test that opens one by itself

Three pending items that seemed to depend on a display. None did.

**N1 — the app-side checks.** `--selftest` only ran the core, which knows nothing
about `cli.rs`, `keys.rs`, `mouse.rs`, `state.rs` or `debugsock.rs`; six
`checks.py` checks were out of reach (`argv_parsing`, `kitty_keyboard`,
`mouse_modes_and_sgr`, `window_state`, `debug_socket`, `desktop_shortcut`). Now
there is `crates/app/src/checks_app.rs` with the six, and `--selftest` runs
**31** (25 + 6). `cargo test` runs the same list (`app_checks_pass`), so a
new check does not stay in the binary only.

Two of them cost an expectation fix, which is the point of the check existing:

- `?1005` adds 33 and encodes in UTF-8: column 300 becomes `c5 8d` (U+014D), and
  the old protocol saturates at 222 (`ff`). My first assertion looked at the
  wrong byte;
- `desktop_shortcut` demands the `install.sh` contract
  (Exec/TryExec/Icon/Categories/terminal and the removal on `--uninstall`) and
  the embedded SVG — `install.sh` and the icon enter the binary through
  `include_str!`, so the check holds in the installed binary, far from the
  repository.

**N7 — automated visual verification.** Capture with `import` and comparison were
manual. Now:

- `tools/pixel.py` reads the capture through `convert` (PPM), without `gi`, and
  **compares**: `--diff` says how many pixels changed and the box, `--max` turns
  it into a test, `--expect` demands color per point, `--scan`/`--row` sample per
  cell;
- `tools/visual.sh` brings up its own Xvfb (if there is none), a fake `HOME` with
  a fixed theme, waits for the first frame through `state`, captures by the `xid`
  that **our** `state` reports, and demands four things: new content has to change
  a pixel, SGR 41/42 has to paint `color1`/`color2` in the cell, text has to fall
  inside the cell and two captures of the same screen cannot differ. Exits 0/1.

The `xid` coming from the search by class was the test's own bug: on a display
with more than one instance the capture took the wrong window and came out black
(it happened twice before I looked at the image instead of the code).

**M-AA — running confined.** What can be done without reproducing the owner's
environment: `state` now separates **`drawn`** from **`skips`** (before only
`last_frame`, which did not distinguish "did not draw" from "drew and the surface
refused"), and the app warns **once** on stderr when the surface refuses the
frame, pointing at `DEBUG.md`. The pixel gate now catches the case from the
outside: new content that does not change a pixel = did not present.

Left out, on purpose: **backend failover** (recreating the renderer in GL when no
frame presents). It cannot be written blind — `Xvfb` does not even come up inside
`sandbox` (segfault in `InitFonts`), so there is no way to reproduce the surface
refusing here. The item stays in the `BACKLOG` with the failover trigger written
(`skips > N` with `drawn == 0`) and the rest of what it asked for (degraded state
with a warning, installer explaining a blocked target, clipboard without stalling
startup, path without pixel) already done.

**Doc debt paid in the same step:** `M4` and `G17` **left the panel** at the
owner's request (2026-11-02) and now exist only in the `BACKLOG`, as `M-AB` and
`M-AC` — an item without an environment and an expensive item do not lock the
migration count, and moving is removing from the origin: the panel ended with 203
lines (200 done / 3 no). `§16.2` stopped listing 27 checks "still without an
equivalent" that already existed under another name (it now says where the checks
live: 25 in the core, 6 in the app) and the 1 px seam of box drawing entered the
`BACKLOG` as `M-AD` — it had never been recorded, despite yesterday's `HISTORY`
saying it was.

## 2026-11-02 — migration closed (and the closure paperwork)

The panel reached zero parity pending. What was left was not migration work, it
was tidying — and that is what this day closed.

**The three image lines were not parity.** `H17` (`t=f`/`t=t`/`t=s`), `H18`
(animation `a=f`/`a=a`) and `H19` (sixel/iTerm2) were marked `no`, with the `ref`
column saying "they do not exist" — that is, Python does not have the three
either. By the panel's own rule (if Python does not do it, it is not parity) they
could not be there. They left: today they are `B1`, `B2` and `B3` in the
`BACKLOG`, with trigger and cost written, because the decision stopped being "we
will never do it" and became "when someone needs it". `LIMITATIONS.md` stopped
claiming them: only the fact that still holds stayed (image embedded in the
escape, base64) plus the sentence that carrying it over file/SHM, animating and
speaking sixel are queue, not a limit.

**Panel: 200 of 200 = 100%** at that moment (195/195 after the five improvements
left, at the end of the day). Area 8 closed at 13, nobody stayed `partial`.

**The closure paperwork**, which is what keeps the document from going on talking
in the future tense:

- `§16` stopped being a plan ("ready when…") and became a record of the order the
  migration followed, with the criterion that closed each slice; the queue became
  the `BACKLOG`, stated in the section. `§16.1` became "rules that held along the
  way" — they are what explains the shape of the lines.
- `§16.2` gained the map of the three checks that changed name (`graphics_kitty`
  covers `graphics_image_and_offset` + `graphics_follow_history`;
  `desktop_shortcut` covers `shortcut_remove_only_own_name`), so nobody
  "discovers" missing coverage where there is none.
- `AGENTS.md`: the rule of coexistence with Python changed tense — it **was** the
  observable specification and today it is history; what is missing here is new
  activity, not parity. The queue with id `B<numero>` also came in.
- `README.md` and the `MIGRATION` header stopped saying "in progress", and the
  README's "not yet" list was days out of date (it said tabs, images, config and
  reflow were missing).
- version **1.0.0**: closing the migration is the only milestone of this project
  that is not "one more item", and the patch convention had no way to express it.

**Still open, and it is not parity:** `M-AB` (exercising Wayland on a compositor)
and `M-AC` (drawing by region), both in the `BACKLOG` by a 2026-11-02 decision.

### The five `done` lines that were not parity

The same ruler of the `no` ones, applied to what was on the other side: five lines
were `done` with the `ref` column saying that Python **does not deliver that**.
Work really done, but our improvement, not parity — they left the panel and are
recorded here, with the day each one closed:

| line | what it was | closed on | verified by |
|---|---|---|---|
| `C5` | SGR 3 italic, 2 faint, 5/6 blink, 9/29 strikeout | 2026-11-02 | `full SGR (C5, C6, C7)` — `bat`/`git diff` on screen; SGR 5 animates with the cursor's phase |
| `C6` | SGR 53/55 overline, 58/59 underline color | 2026-11-02 | same: colored underline and overline drawn in `render.rs` |
| `M1` | a single binary, no GTK/Pango/fontconfig/Python | 2026-11-01 | 10.1 MB, deps `libc`/`libm`/`libgcc_s`, startup in 0.18 s (`BENCHMARKS.md`) |
| `M5` | GL backend (`WGPU_BACKEND=gl`) | 2026-11-02 | `M5 closed: the GL backend draws with a real GPU` — `Gl AMD Radeon Graphics (radeonsi)` |
| `M8` | CI build matrix + release by tag | 2026-11-02 | `release by tag (M8)` — Linux x86_64/aarch64; macOS/Windows stay in the `BACKLOG` |

Panel: **195/195 = 100%**. What Python does not have stopped inflating the
migration count; what it has is there, all `done`.

## 2026-11-03 — B4: drawing painted every line at `2r`

The owner's symptom was about use, not about tests: in the real tortty every
`Enter` "jumped two lines" and the cursor appeared far from the prompt. The first
reproduction attempt failed because it used `bash` on a roomy grid — and the
report was about `zsh` with the screen already full of history.

The grid was **right**: `state` showed columns/rows and cursor where they should
be, and `stty size` answered the same size as the app. The error was in drawing,
and there were **two**, both introduced by the per-line drawing of `A17`:

1. `paint_line`/`paint_glyphs` positioned each quad at `y * ch` (absolute) and
   `concat_lines` added `y * ch` **again**: line `r` came out at `2r`. The cursor
   is drawn once only, hence the "right" cursor and the text far from it.
2. On scroll, `rebuild_rows` rebuilt only `len - 1` (a constant index inside the
   loop) instead of the `n` lines that came in: the others stayed **empty** —
   blank bands in the middle of the screen after `seq`.

Measured, no guessing: each grid line received a distinct `48;5;N` background and
the capture said which line fell in each 27.3 px band. Before: band `r` showed
line `r/2` (1 hit in 15). After: identity 15/15, in **`bash` and in `zsh`**. The
gate became step 5 of `tools/visual.sh` ("line r is painted in band r"), and it
goes red on the old binary.

A **third** error appeared later, and only when the owner complained again: the
content started in the middle of the screen. Grid and pixel agreed (line 12 on
both sides), so the drawing was right — the wrong one was `resize`. The saved
state is `max: true`: the window is born with 36 lines, the shell writes the
prompt on line 0 and only then does the WM maximize to 48. Shrinking/growing
pushed the 12 **empty lines at the bottom** into the history and gave them back
as 12 lines above the prompt (the cursor went from 0 to 12). Python already had
the rule in `_reflow` — "what is after the cursor is blank area, it is not worth
pushing an empty line into the history" — and Rust did not. Rule brought over,
with a unit test, and step 6 of `tools/visual.sh` (shrink/grow cycle in its own
instance) catches the return: red on the previous binary, green on the fixed one.

As a bonus, the same day recorded two operational facts: capturing an
**obscured** window returns what is on the screen at those coordinates (not its
content), and `import`/`xwd` grab the pointer with a crosshair — screen capture
only on its own virtual display.

## 2026-11-03 — B6: write and scroll in the same chunk, damage did not scroll along

The owner's symptom in real use: `time sleep 1` sometimes left the timeline line
**blank**, and dragging the mouse made the text appear. That dragging fixed it
already said where to look: the text was on the grid, what went stale was the
drawing's **per-line cache** — dragging changes the signature (selection) and
forces a full rebuild.

Deterministic reproduction: 20 attempts of `time sleep 1` on a 40-line grid. The
first 19 pass; on the **20th**, with the screen full and the scroll starting, two
lines with text on the grid had no pixel. Instrumenting the frame (`shift`,
damage, lines with an empty cache) and the pty bytes interleaved, the mechanism
became explicit:

```
TRACE in  cy=39 sleep 1  0.00s user ... total<0d><0a>   <- text + LF in the same chunk
TRACE frame shift=1 dano=rows:1 cy=39 vazias=[38]
```

`time` writes the timeline on the last line and the `\r\n` scrolls the screen **in
the same chunk**. The renderer rotates the cache (`cache[38] ← cache[39]`), but
the damage **mask did not rotate with it**: line 38 received the content that had
not been drawn yet, with the cache of line 39 (the command echo, or empty), and
nothing rebuilt it — blank until the next full rebuild.

Fix in the core, both directions: when scrolling with a usable displacement,
`dirty` **rotates with the content** (`rotate_left`/`rotate_right`, the new line
marked). And `scroll_conflict` (went up and down in the same frame) now forces
whole-screen damage — before it only turned off the cache rotation, which left
the rotated lines stale all the same.

Measured after: 40 attempts (twice what it took to blow up) with no divergence,
and `last_damage=rows: 2` per frame — the `A17` optimization still holds (2 lines
per frame, not the whole screen). New gate: step 7 of `tools/visual.sh` writes
**one line at a time** until it passes two screens and compares grid × pixel line
by line; red on the previous binary, green here.

## 2026-11-03 — B7: `StartupNotify=false` in the shortcut

Opening from the system shortcut showed **two** entries in the taskbar — `tortty`
and a **"Starting tortty"** — with the pointer in busy mode for many seconds,
until the ghost disappeared by itself. The menu entry asked for
`StartupNotify=true`: whoever launches opens the startup notification and waits
for the app to say it finished (`DESKTOP_STARTUP_ID` + `_NET_STARTUP_INFO`).
tortty never said it, so the panel kept the entry and the busy cursor until the
timeout — which explains exactly the "many seconds" and the fact that the
symptom does not exist when running from the terminal.

Owner's decision: the simplest way out. The `.desktop` now carries
`StartupNotify=false`, so nothing is opened waiting for an answer. The "opening"
notice that would be lost with that depended on that same answer, that is, it
never worked. If one day the app completes the notification (`winit` has
`read_token_from_env` + `with_activation_token`, and its X11 sends the `remove`
when creating the window), the key goes back to `true` along with the code.

## 2026-11-03 — B5: an X error from the WSI no longer kills the app

`sandbox tortty` exited with **134** in `set_window_title` and, past that, the window
stayed black. B5 was noted with two stages and a cost; today both were measured
and one of them closed.

**Where the process died.** `winit` keeps the last X error of the connection and
`expect()`s it on the next call that flushes (`x11/window.rs:1028`, inside
`set_window_title` itself). With `TORTTY_XERR=1` (new probe) the hook printed what
Mesa's WSI leaves pending in the sandbox without a GPU:

```
XERR codigo=9  requisicao=149 (Present)     -> BadDrawable
XERR codigo=10 requisicao=130 (DRI3Open)    -> BadAccess
```

The `DRI3` `BadAccess` was **the** one that killed it — the first version of the
policy swallowed only the drawing codes when the request was an extension and the
process still aborted. `crates/app/src/xguard.rs` registers winit's public hook
(`register_xlib_error_hook`) and swallows `BadDrawable`/`BadWindow`/`BadMatch` in
any request and `BadAccess` only in an extension; everything goes to stderr (the
first three occurrences and then sparsely). Measured: `sandbox tortty` without GPU
**stays alive** 12 s, answers on the socket and keeps drawing.

**Stage 2 is environment, not code.** Measured on the same day, with the test that
depends on no counter at all — `xwd` of the window itself before and after telling
the app to paint, compared byte by byte:

| environment | backend | result |
|---|---|---|
| `sandbox @gpu` (RADV) | Vulkan hardware | pixmap changes: **presents** |
| `sandbox` without GPU | Vulkan lavapipe | `skips` grow, black window |
| `sandbox` without GPU | GL llvmpipe | `drawn` goes up, `skips 0` and the pixmap **does not change** (silent no-op) |

That is: without `/dev/dri` this X server presents a GPU surface on no backend at
all, and `drawn > 0` does not prove a drawn screen. The app's message now says
that in one line, with the way out (`/dev/dri`, `sandbox @gpu`), and `DEBUG.md` gained
the table, the probe and the `xwd` method. Decision recorded in
[`LIMITATIONS.md`](LIMITATIONS.md): there is no software backend of our own.

## 2026-11-04 — B4: garbage before the tab title (two bugs, not one)

Symptom reported by the owner: the tab bar labels appeared with "textual garbage"
before the tab text. There were **two** independent causes, and the second was in
every tab, not only in those carrying OSC.

**1. OSC 0/2 did not close on the 8-bit ST.** The parser only recognized `BEL`
and `ESC \` as terminators. A client sending `0x9c` (or the `c2 9c` of U+009C in
UTF-8) left the OSC **open**, and everything that came after entered the payload
until the next `BEL`. Worse: an `ESC` in the body was pushed back into the
buffer, so the `ESC ]` of the **following** title was glued inside this title.
Reproduced with the child sending `OSC 0;GARBAGE\x9c` and then `OSC 0;CLEAN\x07`:
`state.tab_titles` returned `"GARBAGE\u{9c}\x1b]0;CLEAN"` — the garbage before the
text, exactly as reported (capture in Xvfb, bar readable).

**2. The tab number was drawn by Noto Color Emoji.** On the path without shaping
(`glyph_bitmap`, which draws the bar, sidebar card, badge and toast) the **color**
pass swept the whole fallback chain before any mask. Noto Color Emoji maps ASCII
digits and returns a 19x18 px color bitmap (two cells): the `1` of the label was a
big rectangle, and the whole label came out as `b$Tb$` instead of ` 1 T1 `.
Measured with a temporary probe in `tabbar` (`ch='1' cache=19x18 cor=true face=5`)
and in a test that asks, face by face, who has a color bitmap for `'1'`: only
Noto Color Emoji. The grid was not affected because it draws through shaping,
which picks the face by cmap.

**Fixes.**

- `tortty-core`: `0x9c` closes OSC and also DCS/APC/PM/SOS, with a guard against a
  UTF-8 continuation byte (`М` is `d0 9c`; cutting there would lose the rest of
  the title); an `ESC` that is not ST **ends** the sequence instead of going back
  into the body.
- The title now has a single rule, `sanitize_title` (core): control out
  (C0/C1/DEL), zero-width, direction marks (`U+202A..202E`, `U+2066..2069`), BOM
  and soft hyphen. It is applied at **two levels**, not in each caller: in
  drawing — `Renderer::chrome_text`, created here as the only point that draws
  text outside the grid (bar, card, badge, toast now use the same function), so
  new chrome inherits the guarantee — and in the producers of the text: OSC
  payload, cwd basename and workspace name (`Tab::cwd_name`, `finish_rename` and
  the rename typing), because those strings also go to the window title, to
  `state` and to the `dump` header.
  Measured live: OSC with `A U+202E B \x01 C` gives `ABC`; a tab with cwd
  `dir\u{202e}XYZ` shows `dirXYZ`; a rename typed as `U+202E x U+200B y` becomes
  `xy`.
- `glyph.rs`: the cell font and the style variants resolve first (color and then
  mask); the fallback chain keeps color before mask, which is what keeps emoji in
  color.
- `app.rs`: `osc` verb on the socket — the last 8 raw payloads (hex, text,
  truncated, resulting title), because the question "what bytes did the child
  send" only the parser can answer.

**Verified.** 8 new tests in `tortty-core` (8-bit ST pure and in UTF-8, `ESC` in
the body, control and direction mark in the title, `sanitize_title` over a
directory name, DCS with `0x9c`, `М` not cut, probe ring) + 1 in `glyph.rs`
(digit comes from the primary in mask). Live in Xvfb, with three tabs: the 12 OSC
cases of the battery gave the expected title and the bar capture shows
`1 CLEAN │ 2 b4 │ 3 b4`. `tools/check.sh` green.

## 2026-11-04 — code, docs and scripts in English

Owner's request: translate everything — file names, symbols, comments, docs.

- **Symbols**: 322 identifiers (`tortty-core` and `tortty`) renamed by a curated
  part map (`linha`→`row`, `tela`→`screen`, `desenha`→`paint`…), then the
  leftovers the dictionaries could not see were hunted by name.
- **Prose**: comments, doc comments, user-facing strings, test names (now
  sentences: `reflow_narrows_and_widens_preserving_the_text`), the 12 docs,
  `README`, `AGENTS.md`, `TASK-tortty-in-rust.md`, the scripts, the CI workflows,
  the hook, the shaders, both `Cargo.toml` and `rustfmt.toml`.
- **What was not touched**, on purpose: the `DEFAULT_CONF` keys (a test reads the
  name of each one in `CONFIG.md`, so key and doc have to keep the spelling),
  CLI flags, debug socket verbs and their JSON, protocol notation (`CSI`, `OSC`,
  `SGR`, `0x9c`) and the historical `arquivo:linha` of the Python repository.
  Test data that is Portuguese on purpose also stays (`keys.rs` checks the
  literal `é` = U+00E9; `HISTORY` quotes the real `xdotool` output `ãéù`).
- **Stale references fixed**: the docs cited test and function names that the
  rename had already changed (`pisca_ativo`→`blink_active`,
  `gravar_em_lugar_impossivel_nao_estoura`→`save_in_impossible_place_does_not_blow_up`,
  `digito_vem_da_primaria_em_mascara`→`digit_comes_from_primary_as_mask`); the
  187 `arquivo:linha` of the docs were checked to still land inside their file.

**Verified.** 83 core + 90 app tests, `tools/check.sh` green,
`tools/visual.sh` green, `--help`/`--version`/`--selftest` in English,
`install.sh` installing **and** uninstalling on a temporary prefix,
`python3 -m py_compile` and `sh -n` on every script, `devrun`/`devkill` exercised
in Xvfb (spawn, refuse a second instance, kill, refuse a recycled PID).

**Pitfalls recorded** — the mechanical pass created four regressions, and only
one of them the compiler saw:

- `SO_PEERCRED` became `ONLY_PEERCRED` (the map had `so`→`only`) — caught by the
  build.
- `starts_with`/`ends_with` became `start_with`/`end_with` (the third-person
  verb pass ate the `s`) — caught by the build.
- `json.loads` became `json.load` inside the Python heredocs of the scripts:
  `tools/check.sh` stayed green and **only `tools/visual.sh` broke**. A gate that
  is not run does not exist; the capture gate is what found it.
- `matar()` became `kill()` in `devkill.sh`: in POSIX shell a function shadows
  the builtin, so `kill "$pid"` inside `kill()` is infinite recursion, and the
  `|| true` swallowed it — `devkill.sh` printed nothing and killed nothing.
  Rule taken: a shell function is never named after a builtin or a command
  (`cleanup` instead of `clear` in `visual.sh` too).
- Hygiene: `.gitignore` ignored `pi/` while the session directory is `.pi/`, so
  two session files had been committed. The pattern is now `.pi/` and the two
  files left the index (they stay on disk).

## 2026-11-04 — rebrand: `bzmux` → `tortty` (TorTTY — Torbie TTY)

Symbolic name `tortty`, friendly name **TorTTY — Torbie TTY**, headline *Fast,
GPU based terminal*, repository `bazoocaze/tortty`. The name research (why this
one, and what it beat) is in `docs/NAMING.md`, which is not versioned.

- **Directories carry no project name**: `crates/bzmux-core` → `crates/core`
  (package `tortty-core`) and `crates/bzmux` → `crates/app` (package `tortty`,
  binary `tortty`). That is the vocabulary the architecture already used.
- 449 occurrences renamed: crates and `bzmux_core::` → `tortty_core::`, binary,
  `BZMUX_*` probes → `TORTTY_*`, config `~/.config/tortty/tortty.conf`, state
  `~/.local/state/tortty/window.json`, socket and dump paths, `WM_CLASS`, wgpu
  labels, the desktop entry (`Name=TorTTY`, `Comment=Fast, GPU based terminal`),
  the icon (`assets/tortty.svg`), `TASK-tortty-in-rust.md`, the installer, the
  tools, the CI and the release packaging.
- **Not renamed, on purpose**: `tortty-python` (the Python project), `Python bzmux` in
  prose, and the citations `bzmux/term.py`, `bzmux/keys.py`, `bzmux/checks.py`…
  Renaming them would falsify the history and break the `arquivo:linha` of the
  other repository.
- **Transition left out of the installer** (owner's call: only he had it
  installed). The pipx warnings and the "the installer takes the name over"
  block are gone; the old `bzmux.desktop`/`bzmux.svg`/binary and the old
  `~/.config/bzmux` are removed by hand, once.

**Pitfall recorded** — the rename script protected `bzmux/<file>.py` with a
pattern that swallowed the slash (`bzmuxterms.py`), and it also rewrote the
places where `bzmux` was the *subject* rather than the app: the entry where
`tortty-python/shortcut.py` deleted `bzmux.desktop` as an "old name", and the
`pkill -x` warning (with the name unique, `pkill -x tortty` no longer hits the
Python app). Both classes only show up on a read of the diff, not on a grep.

**Verified.** 83 + 90 tests, `tools/check.sh` green, `tools/visual.sh` green,
`install.sh` installing and uninstalling on a temporary prefix (binary, entry,
icon), `devrun`/`devkill` in Xvfb, `--version` printing `tortty <version>`.

## 2026-11-04 — B36: dead key followed by space lost the accent

`~` then space came out as a bare space, while `~`+`a` correctly produced `ã`.
The two halves of the pair do not arrive the same way: the composition is done by
`xkbcommon` inside winit, the dead key is `Key::Dead` (no byte) and the result
lands in `event.text` — but `~`+space arrives with `logical_key` still
`Named(Space)`. The encoder read the named table (`" "`) and threw the composed
text away. The `Character` path already preferred `text`, which is why only space
(and the other dead keys whose second half is space) was affected.

Fix in `keys.rs`: on the named path, a `text` that differs from the key's own
base and carries a printable character wins, keeping the ESC prefix for Alt.
Control characters are not composition — `Backspace` brings `\x08` and still
goes out as `DEL`.

**Verified.** Xvfb + `xdotool` (XTest: the real path through winit) with the
child recording its own stdin (`--cmd 'cat > file'`) and a `Return` to flush:
before, `~`+space, `´`+space and `` ` ``+space reached the child as
`c3 a3 20 20 20 5a 0a` (three spaces replacing the three accents); after,
`c3 a3 7e 27 60 5a 0a` (`ã` `~` `'` `` ` ``). The `'` for `dead_acute`+space is
the system Compose table (`<dead_acute> <space> : "'"`), not a second bug. Test
`dead_key_composed_over_space_keeps_the_dead_character` in `keys.rs`;
`tools/check.sh` green.

## 2026-11-04 — B37: the tab title went back to following the cwd

The title followed the cwd you `cd` into until the first program that set its
own title: after leaving `mc` (or `vim` with `titlestring`) the tab stayed on the
last OSC 0/2 for good, though a new program's title still came through.

**Cause, measured rather than guessed.** Apps do not restore the title with an
OSC — they use xterm's title stack. A pty harness recording the child's bytes
shows `mc` sending `CSI 22;0t`, `CSI 23;0t`, `CSI 22;0t`, `OSC 0 mc [...]` and a
`CSI 23;0t` on exit; vim 9 pushes `22;0`, `22;2`, `22;1` and pops them in the
same order. Both `22` and `23` fell into the parser's `_ => {}`, so the pop never
happened. There is one title here (`OSC 1`, the icon title, is ignored since
`C10`), so any `Ps` of 0/1/2 saves and restores the same thing and the sequence
stays symmetric; the stack is capped at 16 (`TITLE_STACK_MAX`).

No app-side change: the pop restores the initial `tortty`, and the app's rule
"OSC wins, else the cwd" already treats `tortty` as "no request" — the cwd
fallback comes back by itself.

**Verified.** Debug binary + Xvfb + `xdotool`, reading the **window** title
(`xdotool getwindowname`; `state.title` is `Term::title`, a different thing —
recorded in [`DEBUG.md`](DEBUG.md)). Before: `cd /etc` → `etc — tortty`, `mc` →
`mc [user@host]:/etc — tortty`, and after leaving `mc` and `cd /tmp` it
stayed on the `mc` title. After: the exit brought `etc — tortty` back and the
next `cd` gave `tmp — tortty`. Three tests in `tortty-core`
(`title_stack_gives_the_title_back_when_the_app_leaves`,
`title_stack_nests_like_vim_pushes_it`,
`title_stack_does_not_grow_without_bound`), `tools/check.sh` green,
`tools/visual.sh` green.

## 2026-11-04 — B38: closing by the shell lost the window state

Symptom: maximize, quit, open again via the `.desktop` shortcut and the window
came back normal, as if the change was not remembered.

**Cause: a panic, not a missing save.** The window state is written only in
`exiting()`. When the terminal is closed the way a terminal is usually closed —
typing `exit` or Ctrl+D, so the *child* ends and the last tab goes away — the app
kept working with `tabs` empty, and the paths of the way out indexed
`tabs[active]`: `wscards()` (from `redraw()`, which `drop_empty_workspace`
called) panicked first, and after that was fixed `exiting()` itself panicked while
killing the pty of a tab that no longer existed. Closing through the WM (the X
button) never hit it, which is why the bug looked like "it does not save".

Fix: once `exiting` is set nothing else is handled (`window_event` returns on
anything but the two events that leave the loop), `drop_empty_workspace` only
sets the flag instead of drawing with no tab, and `exiting()` reads the tab with
`get_mut`. Measured, not guessed: under Xvfb + **Marco** (the owner's WM) the
three close paths were told apart — WM button wrote the file, `exit` and Ctrl+D
wrote nothing at all, and stderr showed the panic at `app.rs:561`.

**Verified.** Xvfb + Marco with its own `XDG_STATE_HOME`: the three paths now
write `{"max":true,...}` and the next launch is born maximized; new step 8 of
[`tools/visual.sh`](../tools/visual.sh) ("the shell ending the session exits
cleanly and saves the window state") fails on the old code with `rc=101`.
`tools/check.sh` green, `tools/visual.sh` green.

## 2026-11-04 — touchpad scroll: the view kept moving after the finger left

Reported as "smooth scroll is weird" and then as inertia. Two defects, found
with a probe that logged every wheel event and, in parallel, the driver's side
(`xinput test-xi2 --root`):

- `scroll_view()` drew a frame **inline** (`redraw()` → `renderer.render`) for
  every wheel event, and a touchpad delivers up to ~125 events/s. The app
  consumed ~37/s, so the rest queued in the X server and kept draining after the
  finger stopped: the "inertia" was backlog. The frame is now requested
  (`request_redraw`), and one draw per frame comes from `RedrawRequested`.
- The wheel's 3-line notch factor (D4) was applied to the touchpad too. The X11
  smooth valuators arrive as `LineDelta(delta / scroll increment)`, a *fraction*
  of a line, so every ~15 px of finger scrolled 3 lines. The factor now applies
  only to a notch (exactly ±1.0); the fraction is kept as sent and accumulates
  in `scroll_frac` (`mouse::wheel_lines`, with a test).

Measured in one 9 s gesture: 337 events, ~37/s, delta 0.58–2.06 lines per event
(22–132 lines/s). The event rate stays put and the magnitude carries the finger
speed — which is why a fixed step per event was tried and rejected: it pins the
speed at ~37 lines/s and loses the control.

## 2026-11-05 — B4: the drag paid one present per mouse event

`fn redraw()` (`app.rs`) drenches the pty, builds the frame and presents
synchronously, so a handler that called it from a **motion** path paid one
present per event: `drag_select` (selection) and `bar_drag` (tab reorder, which
drew twice — `move_tab` also redrew). A touchpad delivers up to ~125 events/s, so
the X queue grew and the drag kept going after the hand stopped: the wheel's trap
(`6499f09`) in another path, and the reason the lag showed up "in several
points".

Measured with `PROF` under Xvfb (120 motion events per drag, `--prof`, socket
`prof` reset before each one): selection **57 presents → 3**, tab reorder
**9 → 3**. The selection ended on the same cells before and after, and the tab
drag still reorders (`['tortty','tmp','usr']` → `['tmp','usr','tortty']`).

The third number is the socket: `UserEvent::Debug` asked for a redraw on **every**
verb, and `state` is what `benchrun.py` polls in a loop — 60 polls were 61
presents, now **0** (`debug_read_only` skips the verbs that only read).

The keystroke and action paths (`rename_key`, `write_paste`, new/close tab, new
workspace, config reload) keep the immediate draw on purpose: one present per
action is the latency they want; a queue is what they must not have.

## 2026-11-06 — M-S: F13–F24 and the numeric keypad

F13..F24 have no sequence of their own: on a PC keyboard they are F1..F12 with
Shift, and that is what xterm's terminfo says (`kf13=\E[1;2P`). The base is the
F1..F12 sequence and the Shift is part of the key, so a modifier on top adds to it
(Ctrl+F13 = `CSI 1;6P`). Under the kitty protocol they only exist in the `CSI <n>u`
form, with their own numbers (57376..57387). The `keys` verb of the socket learned
the same twelve names, so the encoder can be exercised without a keyboard.

The keypad was **mute**, and not only with NumLock off: winit leaves `KP_0`..
`KP_9`, `KP_Decimal` and the operators out of its keysym table (`keymap.rs` has
them commented out), so with NumLock on they arrive as `Key::Unidentified` and
were dropped. `resolve_key` now falls back to the physical key for exactly that
case. With NumLock off it already worked: winit maps `KP_Home`/`KP_Up` to the
navigation keys, the logical key wins and the keypad keeps acting as the arrow
cluster (`KP_Enter` too, as `Enter`).

Verified with `keys::tests::f13_to_f24_are_f1_to_f12_with_shift` and
`keys::tests::the_keypad_sends_its_character_even_with_num_lock_on`. What stays
out: `DECKPAM` (`ESC =` / `ESC >`) is still "no effect" in the parser, so the
keypad always sends the character instead of the `\EOw` of the application
keypad — the `xterm-256color` installed here has no `kp*` to honor
([`LIMITATIONS.md`](LIMITATIONS.md)).

## 2026-11-06 — M-E: double click selects the word, triple the line

The click count lives in the tab (`Tab::click`: anchor, count, instant) — 2 takes
the word, 3 the line, 4 starts over, and a click on another cell resets it. The
rule itself is in the core (`Term::word_span`, `Term::line_span`), so it is
testable without a display: a word ends **only at whitespace**, which is what
makes a path, a `-rw-r--r--` or an `--option=value` come out whole (the case the
item exists for, and the reason a drag was needed), and a soft-wrapped line is
**one** line — the span of a `wrapped` row goes on into the row below.

That last part forced a fix in the copy: `span_text` put a `\n` between every pair
of rows, so copying over a wrapped line invented a line break, and the trailing
spaces of a `wrapped` row (content, because the row is full) were trimmed. Now the
break only exists where the row is not `wrapped` and `Row::text_cols_full` keeps
them; `selection_text` (the viewport copy of Ctrl+Shift+C) follows the same rule.

Verified on a real window (Xvfb + `xdotool`, `--cmd` with `ls -l /tmp/a.txt
fim`): double click on the path gave `sel a=[0,6] b=[0,15]` (`/tmp/a.txt`),
triple gave `a=[0,0] b=[0,21]` (the whole line, without the padding), and a drag
after the word click extends from the start of the word. Unit tests:
`double_click_takes_the_whole_word`,
`triple_click_takes_the_whole_line_without_the_padding` and
`triple_click_follows_the_soft_wrap`.

The rectangular selection, the fourth part of the item, is registered as `B5`: it
is a different copy (a column range) and does not fit in the same span.

## 2026-11-06 — M-AD: the 1 px seam of box drawing

Measured before touching anything, with a private Xvfb and a capture of three rows
of box drawing (`─`, `┼`, `│`) at the default cell of 9.63 px: on the scanline of
the bar **25 of the 386 columns** came out at 83% of the ink (0.83 against 1.00),
a dim column at every other cell boundary. That is the seam.

The cause is the antialias: the glyph is drawn at the fractional advance and its
mask ends in a partially covered column, so two neighbours meet with less than
full ink. Box drawing (`U+2500`–`U+257F`) and block elements (`U+2580`–`U+259F`)
are exactly the glyphs that exist to touch the neighbour, so `draw_cell` draws
them half a pixel wider on each side **when the ink spans the cell**
(`gw >= cw - 1`); a stroke like `│` does not span it and stays where the advance
puts it. Rounding the origin — what the item suggested — was measured first and
changes **nothing**: the masks are placed 1:1, and a sub-pixel shift moves no
sample (moving a quad by 2 px moves 5370 pixels, by 0.37 px moves none).

After: **0** dim columns, 385 of 386 at full ink (the last one is the window
edge), and `tools/visual.sh` still passes — text, colour, scrolling and the saved
state are untouched.

## 2026-11-06 — `OSC 7`, `?1016` and `LNM`: three cheap facts from the parser

Three escapes/modes that were consumed and thrown away. None of them touches the
hot path: `OSC 7` is one arm in a `match` that already existed, `LNM` is one flag
read where `LF` already goes, and `?1016` reuses the SGR encoder and the cell
geometry the app already has.

- **`OSC 7` (shell cwd).** The payload is `file://host/path`: the authority is
  dropped and the path is percent-decoded (`a%20dir` → `a dir`), because a
  directory with a space arrives escaped. An empty payload clears the cwd (the
  shell saying it does not know) and a payload that is not `file://` does not
  overwrite a known one. The app keeps `/proc/<pid>/cwd` as the first answer — it
  is where the program **in front** is, not where the shell thinks it is — and
  the shell's word is the last resort when `/proc` has none, and only when the
  path is still a directory.
- **`?1016` (mouse in pixels).** The shape stays SGR; what changes is the number
  in it. `Proto::pixels` makes the encoder emit SGR, and `App::grid_px` gives the
  offset from the same origin `cell_at` divides by the cell size.
- **`LNM` (`CSI 20 h`).** `LF` returns to column 0 as well. `IRM` stays out on
  purpose: inserting a cell means a branch in `put_ascii_run`.

Verified: 4 new core tests and 2 in the app (`mouse.rs`, `app.rs`), all green
under `tools/check.sh --fast`; and by hand, with a private Xvfb and
`xdotool mousemove --window $XID 55 47 click 1` on cell `10x19`, the child
received `ESC[<0;6;3M` with `?1006` and `ESC[<0;56;48M` with `?1016` — the same
click, `5*10+5` and `2*19+9` in pixels. A child that printed
`OSC 7;file://host/home/user/a%20dir` arrived in `state` as
`cwd=/home/user/a dir`. The socket `state` learned the two fields (`cwd`,
`mouse_px`), which is how the two were checked without guessing.

Found on the way: **DECALN (`ESC # 8`) was already implemented** (`term.rs`,
screen full of `E`, confirmed by test) while `M-U` listed it as missing; the row
became combining-characters-only.

## 2026-11-06 — `M-G`: kitty flag 4, the alternate keys

The protocol has three slots in the first field: the key as it is on the board,
the same key with Shift, and the key in the base layout — `CSI 97:65:97;2u` for
Shift+A. Only the first was sent, and it was the **post**-Shift code (`CSI 65;2u`),
which is what `winit` delivers: the logical key already has the layout applied.

With flag 4 the letters are the case that can be undone without a layout table of
our own: `A` in `0x41..=0x5a` is `a` shifted, so the pair is written out. A
shifted punctuation mark stays single-field on purpose — the base of `!` is `1` on
a US board and another key on ABNT2 — and an absent field is honest where a wrong
one is not. Ctrl+Shift+a already travels as the base key (`97;6u`), so flag 4 adds
nothing there.

Verified: one new test in `keys.rs` covering the four cases (no flag 4, with flag
4, a punctuation mark, Ctrl+Shift), and through the socket on the real binary — a
child that prints `ESC[>12u` (flags 4|8) and then `{"cmd":"keys","data":"shift+A"}`
answers `1b5b39373a36353a39373b3275` (`ESC[97:65:97;2u`); with `ESC[>8u` it
answers `ESC[65;2u`, which is the difference the flag makes.

## 2026-11-06 — `M-C`: OSC 8, hyperlink on the cell

The escape was consumed and thrown away; now it is state. `Cell` carries a
`link: u16` — an index into `Term::links`, `0` = none — and the span is the pair
of `OSC 8` (`8;params;uri` opens, an empty URI closes). The index and not the URI
because the URI repeats in every cell of the span, and a `String` per cell would
cost more than the row. Repeated URIs share their index (`ls --hyperlink` re-links
every name), the params (`id=`) are ignored, and the table stops at 512: past
that the text is still text, without a link.

The width is the interesting part. The first version used `u32` and the cell went
from 20 to 24 bytes: `parsebench` on the `dmesg` load fell from **12,26–12,43** to
**10,51–10,57 MB/s** (-14%), which is far outside the spread. With `u16` the field
lands in the padding the struct already had, the cell stays at **20 bytes**
(asserted by a test) and the same parsebench reads **12,20–12,29 MB/s** — inside
the run-to-run spread of the measurement. That is the whole reason for the type.

On screen: Ctrl underlines every link at once (the modifier that opens them, so
nothing has to be discovered with the mouse), Ctrl+click hands the URI to
`xdg-open`, and only `http`/`https`/`file`/`ftp`/`mailto` are opened — the URI
comes from the child, and `xdg-open` would run whatever handler the desktop
registered for anything else. The underline enters the per-line damage signature
(`Signature::links`), otherwise the toggle would never repaint.

Verified: 3 new core tests (open/close, index reuse, wide char carrying the link,
control characters dropped, the cap), 1 test pinning the cell size, and by hand
on an Xvfb: a child that prints `OSC 8;;https://example.com/x` around `LINK`, a
capture before and after holding Ctrl (**39 pixels** changing, in a `39x1` box at
y=16 — one row, four cells), and a Ctrl+click with a stub `xdg-open` in `PATH`
that received exactly `https://example.com/x`. The same probe found the trap that
`xdotool key` under `Xvfb` does nothing without `windowfocus` (no WM to set
focus), now recorded in [`DEBUG.md`](DEBUG.md).

## 2026-11-06 — `M-M`: the scrollbar

An 8 px strip on the grid's right edge, drawn only when `scrollback` is not empty:
a translucent track (10% of the foreground) and a thumb (55%) whose height is the
share of the buffer the view shows, at the bottom while the view follows the
output and at the top at the oldest line. A press on the strip jumps to that
position, and the drag walks it — the motion belongs to the view, not to the text
selection (`scroll_drag` in the app).

It is drawn **over** the last column instead of reserving a strip: reserving would
narrow every terminal for a control that only exists when there is history, and
3 px of translucent ink over the last column costs less than the column would.

The first version was 3 px and did not work in practice: clicks near the right
edge started a selection instead. Two reasons, both now fixed — 3 px is a hard
target, and the hit test ended at the grid's edge (`cols * cw`, rounded down),
so the window's right margin was dead and a click there fell through to
`cell_at`, which clamps to the last column and starts a selection. The strip is
now 8 px (`render::SCROLLBAR_W`, the same number on both sides) and the target
runs from the strip all the way to the window's edge.

Nothing here touches the core: the numbers were already there (`scrollback.len()`,
`view_off`, `border` of the grid in `pad`/`bar`/`side`). The thumb enters the frame
after the grid and before the chrome, so the bar, the sidebar and the toast stay
on top.

Verified by hand on an Xvfb, with `seq 1 200` as the load (history 193, window
386x149, strip at x=384): at `view_off=0` the pixel at the bottom of the strip is
`7f7f7f` (thumb) and at the top `161616` (track); after a click at the top of the
strip `state` says `view_off=193` and the two colours swap; and a drag from the top
to the middle lands on `view_off=97` (half of 193). `tools/render-check.sh` still
passes — with no history, nothing is drawn.

## 2026-11-06 — build: `codegen-units` back to the cargo default

`[profile.release]` carried `codegen-units = 1` (one thread, best optimization
scope). Trying `4` cut the app-only rebuild from 51 s to 31 s, but the full
rebuilds that followed looked alarming (102 s, 115 s) until the cause was clear:
any profile change rebuilds all 191 dependencies, and with `lto = "thin"` every
extra unit becomes a module in the serial thin-link — so those numbers measured
"a whole world", never the tuning itself. The rematch under `schedutil` (the
machine had been on `powersave`, which alone cost ~30 s on every build) settled
it: incremental 18.6 s (default) vs 19.3 s (split 4/16) vs 22.4 s (all at 1),
with a ~19 s link floor no tuning breaks. Runtime, default vs all-at-1 on the
same battery (`bench.sh`, 600 frames, 2 shots): 61x18 772.8 vs 850.0/797.6 fps,
70x21 755.1 vs 755.2 fps on the converging shots, `draw` 0.87–0.93 vs 0.87–1.07
ms/call — inside the run-to-run spread (one binary alone swung 638–755 fps
between shots), and `parsebench` 38.06 vs 38.28 MB/s best. So the two lines left:
default everywhere, nothing to explain. Price, declared: the binary grows
12.09 to 14.07 MB.

## 2026-11-07 — `M-K`: OSC 9/777 notification with tab badge and card alert

The bell could flash but carried no text; now the child can say what finished.
`Term` counts `notifs` and keeps `notif_text` (capped at 512 chars), `OSC 9`
takes the whole payload as the message and `OSC 777` only the `notify`
subcommand (`title: body`). The app mirrors the bell drain through one `pump`:
the foreground tab toasts, every tab fires window urgency (a notification is an
explicit request — `bell` governs only `BEL`), and the old `ring()` is gone,
its three arms covered identically.

Two watermarks because delivery is not acknowledgement: the drain advances
`notifs_seen` (toast + urgency, exactly once), while the badge lives on
`notifs_acked` — the tab label while in front, the sidebar card (new
`WsCard.alert`) while parked. The toast and the badges ignore keys for 3 s
(`NOTIF_FLOOR`; the toast already lives 3.5 s), then the first delivered key or
the visit acknowledges; a parked card survives keys typed elsewhere and clears
on entry. The drain also feeds parked workspaces now (feed only, no title or
draw) — before, a bell in workspace 3 waited for the return; the view still
follows the output so the switch back lands where it would have.

The probe tripped on the design once: the first version marked seen at drain
and no badge ever stuck. Verified on an Xvfb: foreground toast (1724 px diff on
the bottom row, `state` `notif='build ok'`), early key keeps (`notifs=1`, toast
still diffs), late key clears (`notifs=0`), background tab (no toast, 1824 px
diff on the bar, visit clears), parked workspace (1003 px diff on the sidebar,
switch clears), `777` joining (`notif='make: all done'`), urgency in `WM_HINTS`,
plus 3 core tests. `tools/render-check.sh` still passes.

## 2026-11-07 — `B1`: kitty image by file, temp file and shared memory

`icat --transfer-mode=file` used to stop at the capability query: answering
`t=f` with `EINVAL` made it report "terminal does not support the graphics
protocol". Now the payload of `t=f`/`t=t`/`t=s` is read for what it is — a
path, base64'd — and the protocol answers `OK` for all three media.

The core still does no I/O: `Graphics::handle` returns a `FileRequest` (path,
whether `t=t` means delete-after-read, `a=T` or `a=t`, plus the request's
context), `Term` queues it, and the app drains it with `serve_images` before
collecting `child_out`, so the reply rides the same frame. The answer is
`Graphics::provide` (the bytes go through the same direct-transmission path, so
`f=`, `s=`/`v=` and the 64 MiB cap apply unchanged) or `Graphics::missing`
(`ENOENT` — the child learns the image is not coming, never which of missing /
unreadable / directory it was). The app expands a leading `~` and reads at most
one byte over the cap, so an oversized file is refused instead of buffered.

Verified on an Xvfb with a 40×38 red PNG: `t=f` loads and draws (`ff0000` at the
placement, `images: 1`); `t=t` draws and the file is gone afterwards; `t=s`
draws and the shm file stays; a missing path leaves `images: 0`; and with the
image covering the right edge the scrollbar thumb still wins the last pixels
(`e67f7f` = 55% foreground over `ff0000`). Six new core tests cover the request
per medium, `provide`, `missing` and the oversize verdict; 115 core tests pass.

## 2026-11-07 — the toast becomes a box (`B8` follow-up)

The toast was a strip across the last row with the colours inverted: it erased
whatever was on that line and did not read as an aviso. The owner asked for the
box, and the box cannot have the theme background as its own surface — a black
rectangle on a black screen only shows the border. The surface is therefore the
background veiled 14% towards the foreground, which is also what keeps it honest
in a light theme (it darkens instead of lightening) and needs no key in
`CONFIG.md`. Two rects, not one: the opaque background goes first, or the text
underneath shows through the veil.

Geometry: 1.5-cell gutter on each side of the text (the marker lives in the left
gutter), 8 px above and below, 1 cell from the grid's right edge and 8 px from
the bottom, so the box spans two rows — the chrome is drawn after the grid, so
covering text costs nothing. The right edge is pinned: a long text grows to the
left instead of pushing the margin out of the window. The marker is a 6 px
`push_rect` rather than a glyph, because the atlas is `Format::Alpha` and a
coloured bell emoji would draw as a silhouette.

Verified on an Xvfb 90×12 (cell 11.2 × 21.7 px): box 438 × 38 px, surface
`#1e1e1e` = 14% of `#d6d6d6` over `#000000`, border `#cdcd00` on the four 1 px
edges, marker at x+5, 12 px and 8 px from the window edges. `quality-check.sh
--fast` and `tools/render-check.sh` still pass.

## 2026-11-07 — `B40`: the wheel to a reporting child carries the finger's travel

Reported as "the touchpad scroll inside `opencode 2` is hyper-sensitive: many
lines with a minimal movement", while the same app under kitty is fine. The
branch of `on_wheel` that hands the wheel to a child that reports mouse used the
delta only as a **sign and a non-zero test**: every `MouseWheel` event became one
whole notch. A touchpad delivers *fractions* of a detent at a high rate (X
sends up to ~125 events/s), so a whisper of the finger was worth dozens of
notches — the very defect that was fixed for the history on 2026-11-04
(`wheel_lines` + `scroll_frac`) and never applied to the child's branch. Two
more things in the same path: the SGR wheel **release** went out (`<64;x;ym`),
which no xterm sends and which a TUI that counts reports rolls twice; and X11
delivers one detent as button press *and* release — two `MouseWheel` events,
both reported.

The fix is the same rule, with the child's own units: `mouse::child_notches`
accumulates the travel in a per-tab `child_frac` (one per axis) and emits one
notch per detent, carrying the remainder and dropping it on a direction change
(kitty's V120 rule, so a reversal does not eat the first event); a detent —
exactly ±1.0 — is one notch and never enters the accumulator, with the
press/release pair deduplicated in 10 ms (the guard the tab bar already had);
`mouse::encode` drops **every** wheel release; and the units are the raw ones (a
detent, or a cell height for a pixel delta), never the 3-line factor, which is
ours and belongs to the history. The horizontal pixel delta, discarded before,
now goes out too.

Numbers: Xvfb + a child that enables `?1000/?1006` and logs what arrives, with
`xdotool click 4` three times — **6 reports before** (`<64;…M` plus `<64;…m` per
click), **3 after** (`<64;…M` only). Unit: three touchpad events of 0.4, 0.3 and
0.4 detent were three notches and are now one notch plus 0.1 carried.

What was **not** measured: the per-event delta of this machine's touchpad (the
passive `xinput` capture did not land), so "one detent per notch" is the anchor —
the same order as kitty, which spends one notch per cell of finger travel. If
the feel still asks for less, the next step is a multiplier knob
(`touch_scroll_multiplier`), and it belongs to `M-H`.

Verified: `tools/quality-check.sh` (fmt, clippy, test, release build, selftest,
parsebench all green), the Xvfb wheel probe above, and
`tools/render-check.sh`.

## 2026-11-07 — the toast lives 8 s and no key takes it out

The toast was 3.5 s and a notification toast could be dismissed at 3 s by the
first key delivered to the child. Both were short of what the notice needs: the
toast carries the *content* of a notification whose *signal* (the badge) already
lives longer, so the text disappeared before the reader finished the line it was
sitting under.

One lifetime now: `TOAST_DUR` is 8 s and every toast — config notice, `link
refused`, notification — gets it. `NOTIF_FLOOR` became the same constant, so a
key never acknowledges a notification before the toast itself would have left;
what the floor still guards is the badge, which has no other expiry (it stays
until the tab is visited or a key lands after it). With the two equal, the early
dismissal path became dead code and went: `Toast` lost its `notif` flag,
`toast_notif` and `toast_impl` collapsed into `toast`, and `ack_notifs` now
answers for the badges only. A repeated identical message still counts (`×N`)
and still restarts the clock.

Numbers: Xvfb, a child that prints `OSC 9` two seconds after starting, and
captures at 2.0/4.0/6.0/7.5/8.6/9.5 s of age — pixels differ from the no-toast
baseline at every step up to **7.5 s** (3710 px, box `270,106 106x35`) and are
**0 at 8.6 s**. The same probe on the previous build (1.0.29) was already empty
at 4.5 s.

Verified: `tools/quality-check.sh`, the capture probe above.

## 2026-11-07 — the `?2026` window gets a log

The synchronized-output window was invisible: the core flipped
`sync_output`, the gate held the frame, and nothing said what the child had done
inside it. Debugging "the view runs to the top while an app repaints" needed the
window's **content**, not the flag.

Split on purpose. `SyncTrace` (`crates/core/src/term.rs`) is pure counters — it
has no clock, so it can say what the window carried but not how long it lasted:
`ed[0..3]`, `el`, `home`, `stbm`, `alt`, `ris`, `decstr`, the lines it pushed to
the scrollback, the bytes of the reads that found it open, and how many reads
carried it. The counting is one `Option` test on escapes that are already rare
(and one per read, not per byte, in `feed`); a `RIS` closes the open trace with
`ris×1`, because a full reset ends the DEC modes that keep it open. Closed traces
wait for the app in a capped list, so a build that never logs does not grow it.

`crates/app/src/app.rs` drains them after each read and writes one line per
window into the `--debug-log`, with the only thing the core cannot know: the
**duration**, taken between the read that opened the window and the one that
closed it, plus the view offset before and after. Log format in `DEBUG.md`.

Numbers from a real `pi` turn in 120×34 (the case that motivated it): every
window came **atomic** — `feeds 1, 0 B`, `0.0 ms` — so the parser never held the
screen for the dialect's own sync updates; the repaint is `EL` per line
(`el×1` to `el×18`), never `2J`/`3J`, no `RIS`, no `DECSTR`, no scroll region, no
alt screen. What they do carry is `hist+N` (1..7 lines pushed to the history per
window): with the view scrolled up, each of those lines moves the anchor by one
(`view_after_output`), which is the mechanism to look at when the view appears to
run away — and the reason the log prints `view A→B`.

Verified: `cargo test -p tortty-core` (three new tests: counters inside a window,
`RIS` closing it, lines pushed to the history), `tools/quality-check.sh`, and the
Xvfb run above against both a synthetic child and `pi`.

## 2026-11-07 — a repaint does not move the reader (`B41`)

The log of a real `pi` session (5685 `?2026` windows, `--debug-log`) separated the
two kinds of synchronized update the dialect sends: **5673** carry only `EL`
(incremental repaint) and are atomic — one read, `0.0 ms`. **10** carry
`ED 2` + `ED 3` + `CSI H` with **no** `EL`: the transcript is wiped and reprinted,
25 KB to 1.1 MB inside one window, 1.1 to 33.7 ms of held screen and up to 534
reads. Those ten are the ones that moved the view, and only when the reader was
scrolled up: `view 59 → 207` with `hist 60 → 207` and `view 108 → 387` with
`hist 19 → 392`.

The arithmetic gave it away: the closing `hist` equals the window's own `hist+`
(207 = 207, 392 = 392), so `CSI 3J` had emptied the scrollback inside the window
(60 + 207 ≠ 207). What happened then was ours: `view_after_output` added the
reprinted lines to the offset and clamped at `scrollback.len()` — and the ceiling
of a brand-new history is its **top**. The reader was not following output; he was
teleported to the oldest line of a history he had never read.

The rule now has two parts, and the difference is who asked for the wipe:

- rows a `?2026` window scrolled count as a **repaint**, not as output — the offset
  survives (`scrolled_sync` counts them, `view_after_output` subtracts them);
- a history erased **inside** a window is a repaint too: the offset stays, clamped
  to the lines the reader could already see (`FeedMark` carries them) — never
  carried to the top by the reprinted lines;
- a history erased **outside** a window is a `clear`: the saved lines are gone and
  the view returns to the live screen, as before.

`view_after_output` also lost its dead `prev_hist` parameter for a `FeedMark`
snapshot (`scrolled`, `redraw`, `history`, `hist_gen`), so the app no longer keeps
two loose numbers and a clamp of its own in each drain path.

Numbers: Xvfb, child scrolls the screen inside one window with `2J`+`3J`+300
lines, reader at `view_off 42` — after the window **42**, and the next three lines
of real output take it to **45**. Before, the same case ended at the top of the
new history.

Verified: `cargo test -p tortty-core` (two new tests: a window does not move the
reader, an erased history does not reach the top), `tools/quality-check.sh`,
`tools/render-check.sh`, and the Xvfb run above.

## 2026-11-07 — the touchpad audit: the bar's wheel and the bar's drag

Reported as three mouse defects after `B40`, with a fourth sentence that was the
real question: "every point where the touchpad is handled is super-sensitive
from the start, except the ones already adjusted". The audit found **two** left,
both on the tab bar:

- **the wheel cycled one tab per event.** The bar's branch used the delta as a
  sign — and, for a pixel delta, `signum() * 3.0`, so *any* fraction of a detent
  was worth a whole tab. A touchpad sends fractions at ~37 events/s, so a
  whisper of the finger became a storm of tabs. The `10 ms` dedupe that was there
  only killed the press/release pair of one X11 notch; it could not help a
  gesture. The bar now uses the same rule the child's wheel got in `B40`
  (`mouse::notches`, renamed from `child_notches` because it is no longer the
  child's): a detent is one notch, a touchpad fraction accumulates in
  `App::bar_frac` until it completes one, and the direction change drops the
  remainder.
- **the reorder drag ran away from a resting hand.** `bar_drag` measured the
  travel against the *press* point and never moved the press point, so after one
  step the same travel was still there and every following motion event spent it
  again: a tab moved one label, then walked to the end of the bar with the hand
  standing still. The anchor now travels with the tab (`mouse::drag_crossed`
  returns the destination **and** the travel consumed, and the anchor carries
  what the tab actually moved — never what the finger asked, so a clamped move at
  the end of the bar leaves no debt).

The other paths were checked and hold: the history's wheel (`scroll_frac`, 2026-11-04),
the child's wheel (`child_frac`, `B40`), the scrollbar drag (absolute `y`, not a
delta) and the selection drag (3 px threshold). The remaining report — no
scrollbar in the workspace list — is not a defect but a missing feature, and it
went into the queue as `B42`.

Verified: `mouse::notches` and `mouse::drag_crossed` unit tests (a shake does not
reorder, half a label moves one, the anchor moves with it, the end of the bar
leaves no debt); and an Xvfb probe with three tabs through the debug socket
(`tab_titles` + `active`, `xdotool` for the mouse): one `click 4` on the bar from
tab 1 lands on tab 3 and not on tab 2 (one detent, not two); a 2 px drag leaves
the order alone; a drag of half a label plus **20 more 1 px motions** ends with
the tab in slot 2 — with the old binary the same probe ends with it in slot 3.
`tools/quality-check.sh` green.

## 2026-11-08 — `M-P`: the reflow carries the selection

`resize` bumped `abs_gen` and every anchor died: a resize in the middle of a copy
lost the selection (the Python's behaviour, and what `M-P` was about).

- `Term::resize_keep` takes the span by parameter — the core keeps no selection —
  and returns it remapped: the reflow records `(paragraph, cell offset)` of the
  points asked about before the rewrap and `(row, col)` after it, the arithmetic
  the cursor already did. A row the ceiling ate returns `None`; `apply_size`
  passes `tab.sel`, so the app no longer gives up the span.

Verified: three new `term.rs` tests (carried, spans a wrap, dies at the ceiling)
plus `tools/quality-check.sh --fast` green.

## 2026-11-08 — `DECKPAM`/`DECKPNM`: the keypad application mode

The parser took `ESC =`/`ESC >` and threw them away, so a keypad key always sent
`7` or `+` and a program could not tell `KP_7` from the digit, nor `KP_Enter`
from `Return`. `Term::keypad_app` is that mode now — cleared by `DECSTR`/`RIS`,
answered by `DECRQM` as 66 — and `keys::encode` sends the VT100 `\EO` table for
the bare key (`KP_7` → `\x1bOw`, `KP_Enter` → `\x1bOM`): the numeric mode cannot
express it, since winit hands over only the character.

Verified: three tests in the two crates, and an Xvfb probe with `cat -v` — `^[Ow`
with the mode, `7` without.

Correction, same day: the `\EO` table fired for *any* keypad key, and the
programs asking for the mode cannot read it (`xterm-256color` declares no
`k0`..`k9`: `zsh`'s line editor beeped and `vim` turned `ESC O w` into `7`). The
first attempt was to let a navigation key win over the mode — still wrong with
NumLock on, where winit hands over no keysym and the keypad answered `ESC O w` to
a shell that then swallowed it. Measured, at last, what the system terminal does
(ghostty 1.3.1 in the same `cat -v` scene under `DECKPAM`): NumLock off → `^[OH`
/ `^[OD` (the navigation key, application cursor form), NumLock on → `7`, `4`,
`+`. So the table went out: `DECKPAM` is state only (`DECRQM` 66) and the keypad
sends the digit or the navigation key, which is what every program reads.
Verified through the socket with `zsh -i` on Xvfb: NumLock on types `hello781+-`,
NumLock off takes the cursor from 33 to 28 with keypad Home and back to 33 with
keypad End; `vim` still moves with keypad 4 and 7.

## 2026-11-08 — `B42`: the workspace list scrolls

The cards were drawn from the top at a fixed 2.4 rows and nothing moved them: a
workspace past the bottom of the window was reachable only by `Ctrl+Shift+1..9`,
and the wheel over the column fell through to the grid's history.

- `App::card_off` is the first card drawn; the wheel over the column walks it,
  hatched with the same rule as the bar and the history (a detent is one card, a
  touchpad fraction accumulates in `card_frac`). `card_at` adds the offset, so a
  click lands on the card the eye sees.
- `sidebar` cuts the list at the bottom of the grid and draws a text row only
  when the whole cell fits; `reveal_card` (in `after_ws_change`) brings the card
  of the workspace entering from the keyboard into sight, and `clamp_cards` keeps
  the offset inside the list. No track is drawn — the item's cost line is the
  offset, not a scrollbar.

Verified: an Xvfb probe with 8 workspaces in a 5-row window through the debug
socket (`cards_off`, `cards_visible` are in `state`): 7 × `Ctrl+Shift+N` leaves
`off` at 6 (the maximum) with `ws` 7; six wheel-downs hold at 6; a click on the
top visible card picks workspace 7; `Ctrl+Shift+1` takes `off` back to 0. The
captures show cards 7/8 with the active one highlighted, and no half glyph at the
cut. `tools/render-check.sh` and `quality-check.sh` green.

## 2026-11-08 — `M-I`: the workspaces and tabs come back

Closing the window cost the layout: every run started with one tab, whatever the
day had been. The file that remembered the geometry (`J2`–`J6`) already existed.

- `state::State` now carries `geometry` and an optional `layout`: one entry per
  workspace with its name and the cwd of each tab, plus the workspace and tab in
  front. A file without a layout (the old format) still opens, with one tab.
- `session_layout` writes it on the way out; `restore_layout` rebuilds it before
  the first frame, in the order the `parked` invariant wants; a directory that is
  gone opens the tab on the shell's default, and the workspaces with no tab and
  the out-of-range indices in a hand-edited file are dropped. `--cmd` and
  `--cols/--rows` do not restore: a probe and a measurement want a clean terminal.

Verified: `the_layout_survives_a_round_trip_and_loses_only_what_it_cannot_open`,
the extended `window_state` check, and three Xvfb probes through the socket: two
workspaces with two tabs come back as two workspaces with two tabs and `ws=1`;
a layout pointing at `/nao/existe` opens both workspaces without an error; the
shell of a restored tab really starts in the saved `/tmp/mi-cwd` (the app saves
that same path again on the way out).

## 2026-11-09 — the reader stays on its text inside a `?2026` window (`B53`)

With `pi` writing in the tab, scrolling back made the text slide: every line the
program wrote moved it one row, though the offset on screen did not change.

- Measured from the owner's `--debug-log`: 574 of 691 windows closed with the
  reader scrolled back (up to 78 rows), **none** moved the view, and 104 lines
  went into the history under it. All 53 windows that pushed lines carried `el`
  and no `ed2`/`ed3`/`home` — they were appends, not repaints.
- The cause was the rule of `B41`: any scroll inside a `?2026` window was counted
  as redrawing (`scrolled_sync`), so `view_after_output` computed no output and
  left the offset frozen while the buffer grew. For `vim` the window is a frame
  of what is on screen; for `pi` it wraps new lines.
- The discriminator is now the window's content, `SyncTrace::repaints`: `ED 2`,
  `ED 3`, cursor home or `RIS` mean the screen is being drawn again, and only
  those scrolls are ignored. `el` is not a mark — a line-by-line append clears
  lines too, and that is the shape the creep came in.
- Measured with a child appending inside the windows: before, 25 rows pushed and
  the view moved 0; after, the same 25 rows and the view moved 25.

## 2026-11-09 — a workspace card is dragged to its place (`B49`)

The order of the workspaces was the order of creation: clicking a card changed
*which* one was in front, never *where* it sat, so a workspace opened by accident
stayed in its place forever — and the layout is saved as a list (`B44`), so the
wrong order came back on the next open.

- The gesture is the tab bar's (`I10`): press, 3 px of travel before anything
  moves — those 3 px are what keeps the rename's double click from being a drag —
  and the card under the pointer is the place the dragged one takes, so the list
  moves as the hand passes and the drop is seen before the button is released.
- The front is a storage detail (`tabs`/`active`), not a place in the order: the
  list is gathered in logical order, moved as one vector by `reorder` and
  scattered back, so `parked.len() + 1 == ws_count` holds through the move and
  the number renumbers itself, because it is positional.
- Measured on Xvfb with three workspaces restored from a `window.json` of
  `/tmp/wsa,/tmp/wsb,/tmp/wsc`: a plain click left the saved order untouched; a
  drag of the first card onto the last saved `wsa, wsc, wsb`; a drag of the last
  card onto the first saved `wsc, wsa, wsb` — in both, the dragged one in front,
  because the press selects the card (what the tab bar does).

## 2026-11-09 — a new workspace leaves the others in place (`B54`)

`Super+N` (and `Ctrl+Shift+N`) sometimes looked like it shuffled the workspaces.
It did: the workspace being left was pushed to the **end** of the parked list
instead of the place it occupied, so every workspace after it walked one step to
the left and the new one was appended after them.

- Measured on the installed `1.0.53`: `/tmp`, `/var`, `/usr` created in that
  order, the middle one brought to the front, then a new workspace — the saved
  order came out `/tmp, /usr, /var, /var`. After the fix the same steps give
  `/tmp, /var, /usr, /var`, which is the only order that keeps what was there.
- The fix is `park_front`: the front goes back into `parked` at `ws_idx` (its
  logical slot), and the answer it returns is the new front's index, which stays
  the last one because the new workspace is appended.
- The helper is a free function with a unit test over names (`0,1,2` with `1` in
  front), because the trap is the index arithmetic, not the spawn.

## 2026-11-09 — the block selection (`B5`)

`ls -l` and a log table have their columns in the middle of the line, and a drag
between two anchors takes every character between them. **Alt+drag** is the
rectangle now.

- The shape travels in `Span` (`Shape::Flow` | `Shape::Block`): a block keeps the
  two corners' columns unsorted and sorts only the lines, so a drag from the
  bottom right is the same rectangle. `visible` returns a `Rect` with the flag,
  and `is_swap` cuts a block by **columns on every row** where a flow selection
  cuts only its first and last.
- `span_text` has a block half: the same column range per row, one line each, no
  soft-wrap joining — the wrap continues on the screen, and that is where the
  block reads it. Trailing blanks are dropped per line.
- The app decides at the press: Alt sets `tab.sel_block` for the whole drag (the
  shape cannot change under the hand), and a block has no word and no line, so
  the click count is not consulted.
- Verified through the real mouse path on Xvfb: the same drag over
  `aaa bbb ccc / ddd eee fff / ggg hhh iii` from col 4 to 6, rows 1–3, copies
  `"eee fff\nggg hhh iii\njjj kkk"` without Alt and `"eee\nhhh\nkkk"` with it;
  bottom-up with Alt gave `"d eee\ng hhh\nj kkk"`. A captured screen has the
  background inverted inside the column and untouched beside it.
- A `sel` verb on the debug socket returns the text and whether the shape is a
  block — the only way to see a selection without a clipboard.
- Limitation recorded: across a resize the rectangle is carried by its two
  corners, and with the child asking for the mouse the drag is reported to it.

## 2026-11-09 — the layout is taken before the farewell (`B51`)

With `restore_state = content` the content came back and every tab came back in
the wrong directory. Reproduced, not guessed: `cd /tmp` + `quit` wrote
`{"cwds":[null]}` with `content` and `{"cwds":["/tmp"]}` with `layout`.

- **Cause**: the farewell of 1.0.47 hangs every pty up **before** `save_geometry`,
  and a tab's directory is `/proc/<pid>/cwd` (with `OSC 7` only as the fallback,
  and few prompts send it). By the time the layout was written the children were
  gone, `prefer_cwd` got `None`, and `null` in the file is as legitimate as a real
  path — nothing complained. Only `content` suffered, because `last_words` does
  not run when nothing is written.
- **Fix**: `exiting` reads the layout first and `save_geometry` now **requires** it
  as an argument, so the order is the compiler's business instead of a comment's.
  No caching: a cached cwd would be a second truth with its own staleness.
- The trap is pinned by a test with a real pty (`/proc` answers while the child
  lives, and stops answering after `hangup`), because an `App` cannot be built
  without a window.
- Verified on the debug build: two tabs in `/tmp` and `/usr` saved as
  `["/tmp","/usr"]` with snapshots on disk, and the reopened session put the
  front tab back in `/usr` above the restored content.

## 2026-11-09 — a file dropped on the window (`B50`)

Dragging a file from the file manager onto the terminal did nothing. Now the
**path** goes into the child, quoted for the shell and written like a paste (with
bracketed paste when the child asked for it, `?2004`), which is what the owner
means by dropping a file on a terminal — and the terminal does not guess further:
no new tab, no `cd`, because it does not know what is running there.

- `shell_quoted`: a path made of characters the shell does not touch goes out as
  it is; anything else is wrapped in single quotes with the embedded `'` closed
  and escaped (`'\''`) — the one form that survives `$`, spaces, quotes and
  newlines. Its unit test is the whole table.
- The XDND event is one arm (`WindowEvent::DroppedFile` → `drop_path`); `winit`
  0.30.13 already speaks it on X11, Windows and macOS.
- The socket grows a `drop` verb that calls the same `drop_path`, so the path is
  verifiable without a drag source: `{"cmd":"drop","path":"/tmp/a file $HOME's.txt"}`
  in a real shell wrote `'/tmp/a file $HOME'\''s.txt'` and `printf` printed the
  file back byte for byte.

Honest gap: the winit event arm itself has no automated test — it cannot be
constructed from a test and Xvfb has no drag source; everything below it does.

## 2026-11-09 — the marker is decoration, not content (`B48`)

`── content restored from the previous run ──` is fed into the grid so the owner
can see where the past ends — and it was then carried by the snapshot: every
restart saved the marker of the previous one, so a tab reopened five times came
back with five markers and no content (measured in the owner's own state: six tabs
of 963–1012 bytes holding nothing but prompts and stacked markers).

- `Row` gains `chrome`: a row the terminal itself wrote to say something about the
  session. It is drawn like any other row and `snapshot` skips it, so the
  photograph leaves with zero markers and the screen keeps exactly one.
- `Term::feed_chrome` sets the mark while a feed runs. The first attempt missed
  because the ASCII fast path (`put_ascii_run`) writes the row without passing
  through `put_char`; the mark is set there too — once per run, not per char, so
  the parser's hot path pays one branch.
- A mark is a property of the **row**, not a position: it survives the scrollback
  and the reflow, which a row index or a hash of the text would not (a resize
  rewraps the marker and the text stops matching). It is carried by paragraph in
  `reflow`.

Verified in a real run under Xvfb with `restore_state = content`: three restarts,
`content restored` count in `session-0-0.ans` = **0** every time, the content
(`echo HELLO-B48`) still there and one row longer per run, and the marker visible
on the screen through the debug socket.

## 2026-11-09 — one owner of the state (`B46`)

Two windows open at once read the same `window.json`: the second one restored the
same tabs and directories and, on the way out, decided what the next run got — the
last to close won, and the session that was open lost its layout. The owner's own
debug runs did exactly that.

- `state::lock` takes a **non-blocking `flock`** on `state.lock`, beside the
  state file, and returns an `Owner` held for the life of the app: the owner is
  asked of the kernel, so there is no stale lock and no recycled PID to defend
  against (`devkill.sh` learned the same lesson with `/proc/<pid>/exe`). The lock
  is a property of the *file*, so an instance pointed at another state (an
  `XDG_STATE_HOME` of its own) never disputes this one.
- The window that gets it restores the layout and the content, applies the
  remembered geometry and is the **only** writer: `save_geometry` starts with
  `owner.is_none() → return`.
- A window that finds it taken is a **scratch** window: one tab in the directory
  it was called from, the 980×640 default instead of the remembered rectangle (a
  window exactly over the first one is the one case where restoring the position
  hurts), no restore, no write. `restore_state = none`, `--cmd`, `--cols` and
  `--rows` do not even try the lock — a probe leaves the hot session untouched.
- `tools/devrun.sh` stops taking the hot session as its state: it exports
  `XDG_STATE_HOME` to `${TORTTY_RS_STATE:-<pidfile minus .pid>-state}`, so the
  window under test has a `window.json` and snapshots of its own by default.

Verified with two instances on one state dir from a hand-written `window.json`
(2 workspaces, `ws = 1`, 700×500): the owner came back at 72×26 with both
workspaces and wrote them on the way out; the scratch came back at 101×34 with
one workspace and left the file **byte for byte** the same. A `restore_state =
none` instance opened beside a default one does not hold the lock: the default one
restored as the owner.

## 2026-11-09 — the child's last words are in the snapshot

`restore_state = content` kept the screen but never the last line a program
writes on the way out. The cause was the **order** in `exiting`: `save_geometry`
(the snapshot) ran *before* the child had been told anything, so the bytes of a
farewell could not be in it — not "sometimes": always.

- `Pty::hangup` sends `SIGHUP` and returns without waiting (a bare `kill(2)`;
  `portable-pty`'s `kill` would send it too, but it waits 200 ms **per child**
  while the app has to be draining).
- `App::last_words` runs first when the window closes: hangup every pty of the
  session, feed the readers for up to 250 ms (or until every child is gone), and
  only then write the snapshots. With `layout`/`none`, or a `--cmd` probe, it
  does nothing and costs nothing.

Verified with the installed 1.0.46 and the new build side by side: a child that
`trap 'echo FAREWELL-HUP' HUP` is absent from the old snapshot and present in the
new one, and a TUI that leaves the alt screen and prints its farewell there
(`printf '\033[?1049l'; echo 'To resume this session: …'`) comes back with the
line and **without** the alt screen. The real `pi` frame is still captured
(3220 bytes): the window did not shrink, it grew by the farewell.

## 2026-11-09 — `B45`: `restore_state`, and what comes back

`M-I` gave back the arrangement and nothing else: the same tabs in the same
directories, every screen empty. That is one of three reasonable answers, so it
became a config key with a ladder — `none`, `layout` (the default, today's
behaviour) and `content`.

- The snapshot is **ANSI**, written by `tortty_core::snapshot`: text, SGR and
  `OSC 8` links, from the primary screen (a photograph of `vim` is not what
  anyone wants back) with the alt screen dropped. Soft-wrapped rows come back
  without a hard newline, so reopening in another width reflows the paragraph by
  the same code that reflows a live resize. Nothing that acts goes in: no image,
  no `OSC 52`, no notification, no title; a control character inside a link URI
  is dropped instead of ending the sequence early.
- Each tab's snapshot is a file of its own (`session-<ws>-<tab>.ans`, tmp +
  rename) so `window.json` keeps its schema and a failed snapshot cannot take the
  geometry with it; the files of closed tabs are dropped on the next save.
- The replay happens **before** the shell exists, so the child's output lands
  below a marker line: above it is the previous run, below it is this one.
- `none` writes no layout; `--cmd` hands the real session back untouched instead
  of overwriting it with the probe's single tab; the geometry is written whatever
  the key says, because that is what anyone expects of a window.

Verified: five `tortty-core` tests (same screen, cursor, soft wrap reopened
wider, alt screen excluded, link with a control character), the `state` round
trip with a closed tab, the config test for the three degrees, and Xvfb probes of
the real binary: `content` reopens the previous screen of every tab (two
workspaces, three tabs, each with its own line) with the marker and a live prompt
below it, `layout` opens the same tabs empty, `none` opens one tab and writes no
layout, and a `--cmd` probe leaves `window.json` and the snapshots byte for byte
intact.

## 2026-11-10 — the toast says what it is about

The notice was one line cut at the grid width, pinned to the bottom-right corner
with a single (yellow) border and no way out before the clock. Everything the
owner reads in one glance — what happened, on which line, which key — was
flattened into the same run of text, and two different notices in a row
overwrote each other.

- Two levels: a **title** in the foreground colour and a **detail** pulled 22%
  towards the surface, wrapped at the box margin (a word is not cut while it
  fits; a path with no space is cut where it reaches the margin), four lines at
  most.
- Three shades by the palette, so a custom theme moves the toast with it and no
  key was added: `Idx(2)` receipt, `Idx(3)` warning, `Idx(1)` error. The 6 px
  geometric marker became a 4 px accent bar on the left, and the box is
  **centred** at the bottom, 28 px up from the edge.
- The time on screen follows the shade: 3 s for a receipt, 10 s for a warning or
  an error, 8 s for a notification (`OSC 9`/`777`, the same time the badge floor
  assumes). No key takes it out; a **click** does, and the press goes no further
  — the box is drawn last and is above the grid, so the click neither starts a
  selection nor reaches the child. The renderer reports the rect in window
  pixels, otherwise the hit test would repeat the geometry that drew the box.
- A different notice **waits in the queue** instead of overwriting the one being
  read; equal words only raise the count (`×3`) and no longer restart the clock,
  which used to pin the box to the screen (that was `B8`, item (d)).
- The middle shade gained a source: `--debug-socket` announces the surface it
  opened, as the python did.
- A clean reload answers on screen (`Configuration applied`, with the family and
  the size in force) instead of only when the previous save had been refused: the
  python always says it, and the owner editing the file is the one who needs to
  hear it.
- A refused reload says what it did to the running configuration —
  `kept the previous one — line N: …` — which is the first thing the owner wants
  to know after saving a broken file; the python shows the error alone. At
  startup the detail is the error alone, because there is no previous
  configuration to keep.

No rounded corner and no shadow yet: the pipeline paints everything through one
texture sample, so a corner needs a mask baked into the atlas (nine patch). The
idea stays for later — the shape did not justify a second pipeline today.

Verified under Xvfb with the real binary, 80x20, fixed palette: the notification
box is centred (239..531 in 771 px), the accent is green; a second notice arriving
while the first is on screen changes **0 pixels** (it queued); after the first
expired the second took the screen (9438 pixels inside the box); a click at the
box centre left **0** accent pixels; a config error drew 1028 red pixels and the
debug socket 1028 yellow. A save of the file with the window open drew
`Configuration applied / Monospace 12 pt` in green (778 accent pixels), and a
save carrying two unknown keys drew `Configuration error / kept the previous one
— line 6: 'no_such_key': unknown key (+1 more)` in red. Tests: `a_notice_detail_wraps_at_the_space`,
`a_word_longer_than_the_line_is_cut`, `the_detail_stops_at_the_line_cap`,
`an_error_stays_longer_than_a_receipt`, `the_same_notice_needs_both_lines`,
`a_click_lands_only_inside_the_box`.

## 2026-11-10 — the fallback chain was one font short

`pi` draws its spinner with braille (`U+280B`, `⠋`). Inside tortty the cell came
out blank — no tofu box, nothing — while the same `pi` showed it in another
terminal. Not pi's: the cell existed, had width 1 and held the character.

The face table numbered `FACE_FALLBACK = 3` with `FACE_BOLD_ITALIC = 4`, so index
4 was two things at once: `face(4)` answered with the bold-italic, and the second
entry of the chain — DejaVu Sans, the only font here with braille — was never
asked. `glyph_bitmap` then discarded the `gid == 0` and drew nothing at all.

- the styles end at 3 and the chain starts at 4; `is_from_fallback` is a plain
  comparison again, without the exception that hid the collision.
- `each_face_index_reaches_its_own_font` (every index answers with its own font)
  and `braille_comes_from_the_chain` (braille has a bitmap, and from the chain):
  with the old numbering the second one fails on face 0 and `gid == 0`. The
  numbering itself is now a compile-time `assert!`, so it cannot come back
  silently.

Verified with the real binary under Xvfb: a child printing `⠋⠙⠹` leaves the
characters in the grid and 76 lit pixels in those three cells.

## 2026-11-10 — one fact in one place

The same absence was told twice: `LIMITATIONS.md` carried the queued activities
beside the accepted limits, and `FEATURES.md` had a "Not implemented" line
naming them again.

- `LIMITATIONS.md` keeps what is accepted, `BACKLOG.md` the queue, `FEATURES.md`
  what exists — the id stays where it is the border of a real feature.
- `B8` had six cells and `B55` no trigger; `M-N` was the named pipe piece of
  `M-X`, and `M-Q`'s trigger pointed at itself through `M-V`.
- `LIMITATIONS.md`'s header lost the `L<n>` id that nothing carries.

## 2026-11-08 — `B43`: the card says what is in it

Every card said `workspace {n}` over `{title} ({k} tabs)`: the first row repeated
the word the whole column already is, and the tab counter answered a question
nobody asks — what decides a visit is *which* program is there.

- The card is now `N <name>` over the title of the selected tab, and `N <title>`
  alone when the workspace was never renamed (the second row stays empty; the
  card keeps its 2.4 rows, so `B42`'s offset arithmetic did not move).
- `ws_name` is born **empty**, and empty means "never renamed": `finish_rename`
  accepts it instead of refusing the key, and clearing the field in the rename
  goes back to the automatic title. `n_tabs` left `WsCard` with the counter.
- `restore_layout` drops the `workspace {n}` of the runs before this one (for the
  workspace it numbered only), so an old state file does not paint the word back.
  The `dump` header says `=== workspace N ===` when there is no name.

Verified: `the_card_never_stands_on_the_number_alone` and
`a_legacy_workspace_name_is_dropped_on_restore`, plus an Xvfb probe with three
workspaces — `1 vim ~/.zshrc` / `2 tortty` / `3 tortty`, the double click on the
third becoming `3 build` over `tortty` and going back to one row when the name was
erased; a state file with `workspace 1`/`build`/`workspace 3` reopened with the two
defaults dropped and `build` kept. `render-check.sh` and `quality-check.sh` green.

## 2026-11-08 — `I8`: the rename answers while the child holds the mouse

`mc` enables `?1002`, and the press on chrome had two implementations: the one
used with the mouse reported only switched workspace and tab, so the double click
that starts the rename (`I8`) and the drag that reorders tabs (`I10`) did not
exist inside `mc`.

- `press_card`/`press_tab` are the one copy now, called by both paths: the
  sidebar and the bar answer first, with or without the mouse reported.

Verified: an Xvfb probe with `mc` in front (`mouse_mode` 2) — the same double
click on the card of the front workspace names it; with the code before this, the
typing went to `mc` and the card kept `workspace 1`.

## 2026-11-08 — `B44`: the restored session keeps the order

`restore_layout` rebuilt the workspaces with the one in front **last** and left
`ws_idx` there: that is the `parked` invariant (the front is not in the list, it
*is* `tabs`/`active`), but read as an order it renumbered the session — a front in
position 1 reopened in the last one and every other workspace shifted left, so
`Ctrl+Shift+1..9`, the card and the `dump` header pointed at a different workspace
after a restart.

- `restore_order(total, ws)` returns the build order (the others in file order,
  the front last) **and** the number the front keeps; `restore_layout` puts that
  number back in `ws_idx` once the rebuild is done.

Verified: `a_restored_session_keeps_the_order_of_the_file` (the mapping the cards
use reads back `0, 1, 2` for every position of the front), and an Xvfb
round trip — three workspaces, the middle one named `MID`, the front in the middle,
quit and reopened: `ws: 1` and `['', 'MID', '']` both times. With the code before
this, the same file reopened as `ws: 2` and `['', '', 'MID']`.
