# History

Concise history of past activities. A maximum of 5 lines per activity.

## 2026-11-01 — phase 1: the feasibility experiment

Question: does rewriting in Rust solve distribution and performance? The abort criterion was "if it does not do ≥3× the Python's MB/s and does not open `vim`/`mc`, it does not pay".
- Core + app from scratch (`tortty-core` without GUI/pty/GPU, `tortty` with `portable-pty` + `winit` + `wgpu` + `swash`); 23 core tests, `vim`, `mc` and `dmesg` on screen.
- Measurement: 8–9.5× the Python in fps and MB/s on doom-fire, 29–46× in drawing, `dmesg` of 1397 lines in 2.1 ms against 42.6 ms — numbers in [`BENCHMARKS.md`](BENCHMARKS.md).
- Debug socket with the Python's JSON protocol, so the same `benchrun.py` measures both.
- Pitfalls recorded: `pkill -x bzmux` killed the pipx one (hence `devrun.sh`/`devkill.sh` by PID file with a `/proc/<pid>/exe` check), and `request_inner_size` is a suggestion to the WM (hence the cell-step `want`/`adjust_left`).

## 2026-11-01 — decision to migrate, inventory and docs

- **Decision**: migrate — the 3× criterion was beaten with room to spare, distribution too; single route `winit`+`wgpu`, without keeping the GTK stack as an alternative.
- **Parity inventory**: 4 independent surveys of the Python code (core/parser, render/colors/images, input/socket/infra, config/tabs/CLI) became the 250 lines of [`MIGRATION.md`](MIGRATION.md), each with the Python `arquivo:linha` and its state here.
- **Docs restructured** in the format of the Python project: `MIGRATION`, `BENCHMARKS`, `DEBUG`, `PORTABILITY`, `LIMITATIONS`, `BACKLOG`, `HISTORY` + `AGENTS.md`.

## 2026-11-01 — hygiene: fmt, clippy, one command

- `rustfmt.toml` and workspace lints (`all`, `correctness`, `suspicious`, `pedantic`); 113 warnings zeroed; `unsafe_code = deny` with one local `#[allow]` (`SO_PEERCRED`).
- `tools/check.sh` runs on pre-commit (`.githooks`) and in CI — the same script, so hook and CI cannot disagree.
- **Bug**: the socket's `keys` verb had a second key table (F5 came out with 1 byte, `alt+left` lost the modifier); it now calls `keys::encode_parts` — verified `tab`=1 B, `shift+tab`=3 B, `f5`=5 B, `alt+left`=6 B.

## 2026-11-02 — full SGR (C5, C6, C7)

- `Cell` gained `ul` (style 1–5) and `ul_color`; `flags` has italic, faint, blink, strikeout and overline; `SGR 22` turns off bold **and** faint like xterm, and `21` is double underline (what `bat`/`delta` want).
- Italic uses the italic face from `fontdb` and bold-italic got its own lookup (the old candidate order made "bold+italic" upright); curly/dotted/dashed are made of rectangles, since the pipeline has no curved geometry.
- Verified on screen with 14 SGR combinations in one session plus 4 new core tests (26 in total).

## 2026-11-02 — mouse reporting (F1-F6, F19, F20)

- `mouse.rs` ports `mouse.py`: button/modifier/motion, SGR (`?1006`) and the old protocol (release = button 3, coordinate saturating at 222).
- `?1005` (UTF-8 coordinate) implemented though the Python ignores it: above column 222 the old protocol reports wrong by saturation.
- `app.rs` gained `on_cursor_moved`/`on_mouse_input`/`on_wheel`; with the mouse enabled a click goes back to live before reporting (F19), and the wheel while reading history scrolls the view instead of going to the app.
- Verified by a probe that reads the **received** bytes (click 1/3 and the wheel under `?1000/?1006` → `ESC[<0;11;4M`, `ESC[<0;11;4m`, `ESC[<2;11;4M`, `ESC[<64;…M`+`m`) and with `vim -c 'set mouse=a'` (a click on line 6 moved to 7.7).

## 2026-11-02 — selection, clipboard and paste (F8, F9, F12-F16, G14, D5-D7)

- `core/src/select.rs`: absolute anchor `(line, column, generation)` and text by `span` — the selection survives scrolling and output, and dies when the history is rewritten (`CSI 3J`/RIS/reflow) instead of copying the wrong text.
- Drawing is real inversion, including for a cell without colour (the default resolves through the other extreme), and `SGR 7` follows the same rule.
- `clip.rs` uses `arboard` without the image feature (binary stays ~10 MB, no `libX11`: arboard speaks the protocol); Ctrl+Shift+C/V run before the encoder (else it becomes `^C`), the middle button pastes PRIMARY, Shift+PgUp/PgDn/Home/End navigate, the wheel is 3 lines per notch with fractions accumulating.
- **Pitfall measured**: `xdotool` coordinates are not `winit`'s (window frame) — the drag test calibrates the offset against two known points read from `state`.
- Verified end to end: drag over "CCC DDD" → inversion, `state` with `a=(1,0) b=(1,6)`, and Ctrl+Shift+V/middle button delivering "CCC DDD" to `cat`.

## 2026-11-02 — F1 closed on the keyboard, scroll and indicator items

- **D8**: `↑N` indicator in the top right (inverted badge, atlas glyph) — without it, scrolling back is indistinguishable from a stuck app.
- **D6/D7**: Shift+PgUp/PgDn/Home/End navigate; a real key goes back to live but a modifier does not (otherwise the Shift would reset before scrolling).
- **E5**: Ctrl+Space is NUL, not `CSI 1;5 ` from the generic table; checked byte by byte with the `keys` verb, which now returns hex.
- **E9**: dead keys already work (winit composes and delivers text) — `xdotool key dead_tilde a` → `ãéù`.
- **Tooling**: `devkill.sh` sweeps this checkout's leftovers (the PID file keeps only the last), and the `tortty (deleted)` suffix must be stripped before comparing, or a leftover answers in place of the new build.

## 2026-11-02 — initial grid: a lost column and the child born too early

- Symptom: `bat`, `dmesg` and `seq` "shuffled" in the first KB, and it was not the parser (the same bytes read from a file reproduced identically).
- `round(cols*cw)` asked one pixel too few (1304.4 → 1304 → 99 columns) and became `ceil`.
- The child was born before the final size, so the WM's first resize arrived while it was writing; now `apply_size` runs first and `spawn_shell` waits for the first applied size.
- Verified: `--cols 100 --rows 10` → `stty size` = `10 100`, and a 250-character line breaks at 100/100/50.

## 2026-11-02 — installer: `tortty` is the Rust one, `tortty-python` is the Python one

- The Python was already renamed in pipx, so the short name was free and this repo's `install.sh` takes it over.
- It copies one file, and does exactly what breaks when missing: warns if the command in `PATH` is another, installs with `install` instead of a symlink to `target/` (`cargo clean` would break it), writes the `.desktop` + SVG icon, and checks `--version` and `ldd` without "not found".
- The Python installer deleted `bzmux.desktop`/`bzmux.svg` as "old name" — those were the Rust ones; proven in a temporary `XDG_DATA_HOME`, fixed in `tortty-python`, revalidated here with `--install-shortcut`/`--remove-shortcut` (same md5), and it became a test in `bzmux/checks.py`.
- Two bugs of mine: the mode function was called `install`, so the `install -m 755` inside it recursed (→ `do_install`); and the write test used `-w` on a directory that did not exist yet, asking for `sudo` in `/tmp`.

## 2026-11-02 — F2: reflow on resize (A22, A23)

- Before, `resize` stretched or cut each line (narrowing **ate** text on the right, widening left 80-column pieces); now the main buffer joins the "soft" (`wrapped`) lines into paragraphs and re-breaks them at the new width.
- **Cut** only whitespace with default background (`Cell::is_blank`): a space with a painted background is content — `vim`'s status bar, `git diff --color`, `bat --style=full`.
- **Cursor** comes from the offset inside the paragraph, not from `cy`, with a pending wrap; a **wide pair** is not split (the break goes one column earlier); `abs_gen` goes up so the selection dies instead of copying the wrong text.
- Proven with 6 new `reflow_*` tests (38 in total) and in the app: 30 paragraphs of 152 columns, `100x20 → 50x20` re-breaks without losing anything and `→ 120x20` joins it back.

## 2026-11-02 — F2: the view is not dragged by output (D10)

- Whoever scrolled into the history was pushed by arriving output; now `drain` saves `scrolled` and the history size before the `feed` and asks `Term::view_after_output` for the correction `off + lines_that_left - lines_that_fell_off_the_ceiling`.
- The second term matters: with the history full every new line discards an old one, and without it the view walks into the void.
- Verified: `view_off` 27 → 29 → 31 with two `echo` arriving and the first visible line still "echo linha-12"; two core tests, one with `scrollback_limit = 3`.
- **F2 closed**: `CSI 3J` emptied the history but left `view_off` pointing at a line that does not exist; `drain` now clamps it to the history size after every `feed` (29 → 0).
- Deviation on record: reflow kills the selection instead of remapping the anchor (the Python does the same); remapping is `M-P`, since `reflow` already knows each old line's `(paragraph, offset)`.

## 2026-11-02 — `tortty --selftest` (N6)

- A command that answers "is the emulator still whole?" without display, GPU or pty: 14 checks in `checks.rs` covering parser/cursor, autowrap and `?7l`, SGR and ED, truecolor and theme palette, DEC graphics + DA1 + CPR, alt screen and DECSTBM, history and ED3, reflow, selection over history, bracketed paste, CSI with intermediate, config, colour OSC and damage.
- It went into `tools/check.sh` (pre-commit) and into CI.
- Three of my expectations were wrong and the check caught them: backspace after `z<TAB>w` overwrites the `w`, `DECSCUSR 4` stores `(shape 1, blink false)`, and `ECH 2` leaves `"  c"`.

## 2026-11-02 — release by tag (M8)

- `.github/workflows/release.yml` (tag `v*`) builds `x86_64` and `aarch64`, and the packaging is `tools/release.sh` — the same command as local, not a parallel CI path.
- It checks `--check-version vX.Y.Z` against `Cargo.toml` (a tag pointing at other code is the nastiest mistake to discover later), runs `tortty --version` **on the packaged binary** (on cross via `qemu-user-static`, saying so in the log when qemu is absent), and prints the `sha256` of `tortty-<version>-<triple>.tar.gz` — the format `install.sh --from-release` expects.
- Tested locally: a 3.9 MB package extracts, runs and answers `tortty 0.1.0`; a wrong tag (`v0.2.0` with `Cargo.toml` at `0.1.0`) exits 1.
- Still missing to close M8: `windows-latest` and `macos-14`/`macos-13`, which depend on M6 (ConPTY) and M7 (Metal), both in `PORTABILITY.md`. The MIT license came along (the package carries the file).

## 2026-11-02 — cursor blink and toast (G12, I12, K13)

- **Blink (G12)**: the phase comes from `blink_epoch` (`Instant`) divided by 500 ms, so no timer can lag or drift; `cursor_blink on` blinks, a key or a paste **relights** it, and `CSI 6 q` with an even shape (4/6) asks for a fixed cursor and wins; `SGR 5` text uses the same phase. The loop only wakes mid-period when something blinks (`blink_active`, recomputed in `feed`, not per tick).
- A/B of captures: with `on`, 432 px alternate at 260 ms; with `off` (hot-reloaded), the captures are identical.
- **Toast (I12/K13)**: the config file is edited in an editor that does not show our log, so errors now appear on screen — `config kept — line 3: 'afffont_size': chave desconhecida (+1)`, an inverted band on line 1, 3.5 s; the same message in a row becomes `(×N)`; a good save after an error shows `config recarregada`.
- Two details found while testing: the first tick re-parsed the file and the error came out twice (the startup `mtime` now counts), and the "reloaded" condition looked for `config:` with a colon.
- Benchmark after all of it: 950–985 fps / 40.8–42.3 MB/s at 61x18 — inside the documented range, no regression.

## 2026-11-02 — tabs (F4a: I1-I4, I13, I15-I17, I20)

- Rust became tortty: `Tab` holds `term` + `pty` + what looks (view, selection, drag) and `App` has `tabs` with one active; `Ctrl+Shift+T`/`W` and `Alt+1..9` (`0` = 10th), and the tab whose shell dies closes itself (closing the last one closes the app).
- **All** tabs are drained (a bounded channel: `yes` in a hidden tab would fill memory); only blink and view are reserved to the active one.
- A new tab's cwd is `/proc/<pid>/cwd` of the **deepest descendant** (via `task/<pid>/children`), so whoever is in front decides — verified `cd /tmp` → new tab born in `/tmp`.
- Title OSC 0/2 → `basename(cwd)` → `tortty`, followed only when it changes; the bar is one line above the grid (shader `origin` is `pad + bar`) and every grid shrinks by one, with no bar for a single tab; BEL keeps `color3` for 800 ms even on a background tab.
- The digit is **physical** (`Digit1..0`: `Ctrl+Shift+1` arrives as `!` and ABNT2 shifted digits differ) and `--cmd` applies only to the first tab; verified over the socket (`tab op=new/select/close`, rows 10 → 9) and XTest, plus captures for the BEL flash and the active inversion — 993 fps / 42.7 MB/s with one tab.

## 2026-11-02 — F4 closed: workspaces and the rest of the chrome (I5-I11, I15, I20)

- **Workspaces**: `parked` keeps the other whole sets (live pty, own grid) and switching swaps contents — no term copy, no dead process; the logical order is rebuilt on switch, because swapping `parked[slot]` in place scrambled the cards (seen in `state`).
- The sidebar shows with 2+ workspaces and eats 18 columns from the grid; the tab bar still runs edge to edge on top of it.
- Shortcuts `Ctrl+Shift+T/W`, `N/Q` (workspace), `F2` (config in the editor, in a new tab — P33), `Ctrl+Shift+1..9`, `Alt+1..9`, `Super+N/Q/1..9`; left click on a card switches, **middle closes**, and double click renames in place (`|` is the cursor, the 1st key replaces the name, Enter saves, Esc discards).
- Three pitfalls: the bar was drawn at `y=0` with the origin already at `pad+bar` (→ `y=-bar`); `tab_at` subtracted the sidebar, which pushes the **grid** and not the bar (→ the bar starts at the window edge); X11 sends **two** events per notch and the second undid the switch (→ paired within 10 ms).
- Verified with `state` (`tabs/ws/ws_names` at every step), captures with and without the bar, and a real mouse for wheel and drag.

## 2026-11-02 — sidebar and tab bar share line 1

- The sidebar started on line 2 when the bar appeared (the bar ran edge to edge and the left column had a hole on line 1); now the bar starts where the grid starts (`x0 = 0`, to the right of the sidebar) and the first card starts at `y = -bar`.
- `card_at` came along (the top of the card is `pad`, not `pad + bar`), and the order of the queries matters: `card_at` before `tab_at`, or a click on the card of line 1 became a click on the tab.

## 2026-11-02 — the backlog gained what is not migration

- The two documents did half the job each — `MIGRATION.md` had a "proposed improvements" section and `BACKLOG.md` pointed at it; now each has one subject.
- `MIGRATION.md` answers "how much is left for Rust to do what Python does"; `BACKLOG.md` keeps what migration does not ask for, with a **trigger** per item, because an improvement without a trigger is a wish.
- First item: **M-Q, chrome overlay** — bar, cards and toast are rectangle + glyph (they inherit font, palette and cell alignment and pull no toolkit, which is why the project exists), but each is written twice: drawing in `render.rs`, hit test in `app.rs`, the same coordinates in two formulas.
- Three bugs that week were that divergence; the item is to make the chrome a list of elements with a declared hit test, in the same `wgpu`, when the fourth element shows up — before writing the draw/click pair a fourth time.

## 2026-11-02 — the boundary: panel = parity, backlog = new activity

- Rule now written in both files: if Python does it, it is parity and lives in `MIGRATION.md`; if not, it is new activity and lives in `BACKLOG.md` — one owner per piece of work.
- The audit found **44 of the 252 panel lines were not parity** (25 still queued, 19 done but marked as improvement) and 13 of the 15 backlog items duplicated the panel; worse, section 15 was a frozen second copy of the panel and 16 the improvements queue.
- After: 208 lines of parity only (145 done / 14 partial / 46 missing / 3 no) and **73.1%** parity — what is new activity no longer inflates the count — with 19 new activities, none with a line in the panel.
- `M-A` (per-line damage, `view.py:330-364`) and `M-G` (kitty keyboard, `kitty_kbd.py`) were disguised parity and went back to the panel, with only flag 4 left as new activity.
- Six orphaned queue lines got their own item: `M-R` (OSC 52/7/133), `M-S` (F13-F24 and `KP_*`), `M-T` (`?1016`), `M-U` (DECALN and combining) and `M-V` (context menu, the declared trigger of `M-Q`).

## 2026-11-02 — new activity that was already in the program

- Consolidation, not new work: what Python does **not** do and was already here leaves the parity panel and is recorded only in this line; on top of it ran `cargo test` (52 in core), `--selftest` 14/14 and `mc`/`vim`/`dmesg` in the manual script.
- Parser and grid: G1 charset with SO/SI, wide char with continuation cell, configurable tab stops, `?6`/`?7`, DCS/APC/PM/SOS consumed up to ST (they leak on screen in Python), DECSC saving SGR/origin/autowrap, `ESC D`/`ESC E`, clamp of `n` in IL/DL/ICH/DCH/ECH, SU/SD, REP and a ceiling in the `csi`/`osc` states.
- Colours and drawing: full SGR (proven with `bat` and `git diff`), OSC 4/104/110/111/112 as *set* at runtime, and a configurable palette/theme checked by pixel sampling against `tortty-python` (`mc` and `htop` identical).
- Input: wheel with fractional delta (trackpad) accumulating, and `?1005` above the 222 ceiling (unit tests in `mouse.rs`).
- Window, CLI and tooling: grid pinned by the socket surviving WM rounding (`dw/dh` loop), `--cols`/`--rows`/`--dpi`/`--no-vsync`/`--prof` (100x10 requested, `10 100` from `stty`), and `--selftest` in the binary, in `check.sh` and in CI.

## 2026-11-02 — kitty keyboard (E12-E15, E17-E20, B13)

- The child turns flags on with `CSI > u`; the flags live in `Term` (per tab, stackable) and the encoder only consults — **state in the core, encoding in `keys.rs`**, the `kitty_kbd.py` division.
- Flag 1 (disambiguate): ESC → `CSI 27u`, Ctrl+a → `CSI 97;5u`, a modified arrow → `CSI 1;5A`, a plain arrow stays legacy (respecting DECCKM); flag 2 adds `:1`/`:2`/`:3`; flag 8 makes `a` → `CSI 97u`; flag 16 appends the text (`CSI 97;97u`).
- Two findings with a real keyboard: the release has to reach the encoder (winit delivered `Released` and the app dropped it), but a local shortcut cannot run twice, so the shortcut stays on the press and the release goes to the encoder; and X11 releases Shift/Ctrl **before** the key, so they cannot clear the consumed-press marker — the blind `take()` leaked `CSI 49;1:3u`.
- Known deviations, the Python's too and recorded in `LIMITATIONS.md` §Keyboard: Shift+letter is `CSI 65;2u` (not `97;2u`, which would need a layout table) and Shift+F3 is `CSI 1;2R` (not `13;2~`).
- Verified over the socket with a child in raw mode printing hex: queries `?0u`/`?3u`/`?11u`, keys `97u`, `97;5u`, `97;1:2u`, `97;1:3u`, `27u`, `1;5A`, `1;2R`, and the local shortcut leaving no trace in the child.

## 2026-11-02 — automation without `xdotool` (L2, L3, L5, L7, L8, L12, L15, J7-J9, K15)

- Not a feature but instrumentation: what was missing so `benchrun.py` and CI would not depend on a mouse and on screen capture.
- `state` gained `xid` (from `RawWindowHandle`; 0 on Wayland — whoever captures pixels asks the app, not X), `config`, `errors` (a config error is a fact even when the previous config is kept), `dpi`, `advance`, `padding` and `pin`.
- `keys` now goes through the local shortcut **before** the encoder like a physical key (`ctrl+shift+n` → `handled:true` and no byte, `ctrl+a` still `01`), instead of answering what the app would not do.
- `prof` counts `writes`/`write_kb`/`eagain` at the single pty write door (now `pty_write`, before six scattered `pty.write`) and `log` switches file at runtime; `--dump-to`, `--quit-after`, `SIGUSR1` (dump now) and `SIGTERM` (dump and clean exit) run off an `AtomicBool` read by the 250 ms loop — 78 lines, three headers, process gone after `SIGTERM`.
- Orphan socket (L12): a file that is not a socket is never touched and the orphan only goes when `connect` refuses — verified with a text file in its place that survived with the contents intact.

## 2026-11-02 — Wave 0 + Wave 1: reconciliation and the check battery (B3, F12, K18, K19, N2-N5)

- Four panel items marked pending **were not pending code**: `B3` (C1 0x80-0x9F → 5 cells for `a\u{80}b\u{9b}c`) and `F12` (PRIMARY always, CLIPBOARD only under `copy_on_select`) proven by running; `K18` showed `--install-shortcut` as a binary flag does not exist and should not (two owners for one artifact) and `K19` was real code (`WM_CLASS` explicit with `with_name`).
- `--selftest` went 14 → 24 checks and `cargo test` 52 → 93, each area in its own module (`checks_n2.rs`, `checks_n5.rs`).
- N2 config (kitty format, merge with the defaults, comments, the last repeated key winning, error per line with the reason, ceiling cutting the oldest, RIS not owning the config, BEL only counting) plus 7 argv tests; N3 kitty keyboard end to end and the five mouse modes with SGR beating `?1005`.
- N4 gave the socket server a `Sink` seam (`EventLoopProxy` in the app, a channel in the test) — what prevented exercising the socket without a window — with 8 tests (framing, two requests in one `write`, `0600`, the three `clear_stale` decisions); N5 history as a window, anchor against drift, `?2026` + `2J`/`3J`, per-line damage, selection, truecolor in both syntaxes and WCAG contrast.
- A seam was missing: the `--selftest` checks did not run under `cargo test`, which said nothing about them; now `selftest_completo` runs `run_all()`, and the proof was by mutation — breaking `view_after_output` fails `scroll_anchor`, removing the guard fails the L12 test.

## 2026-11-02 — five bugs the checks found before they became routine

- None was invented: all five were reproduced and now have a test.
- **The D10 drift at the history ceiling** (the serious one): `view_after_output` subtracted the discarded lines, so with the history full the view walked one line per new line (it showed line 43 where the absolute anchor asked 33); the right count is `(off + d).min(len)` — the clamp, not the subtraction, handles "the line being read died" — and `scroll_anchor` now demands the two agree at the ceiling, proven by mutation.
- **Ctrl+Space under the kitty flags** returned the NUL before the kitty form and lost the modifier; `Space` now enters the kitty table only with flags 1 or 8, since with flag 2 alone (`pi`) the NUL is what readline/vim expect — and **`?1005` was missing from DECRQM**.
- **Usage error exit code**: `tortty --cols abc` exited **0** because `main` mapped only two prefixes to 2; the rule moved to `cli.rs` (only `--help`/`--version` exit 0), decided by **prefix** and not by content, since the usage message carries the help glued at the end.
- **Invalid UTF-8 on the socket** gave EOF with no response; now `read_until` + `from_utf8_lossy` (the Python's `decode("utf-8","replace")`) makes it U+FFFD and answers a normal `err` with the connection alive. Also: `--debug-socket ""` no longer `bind("")`, and the selftest report had `config_file` twice.

## 2026-11-02 — M-W: child death enters the `close_tab_at` path

- `Ctrl+D`/`exit` on the visible tab killed the whole app even with another parked workspace: `about_to_wait` went straight to `event_loop.exit()` and skipped `drop_empty_workspace`, which is what promotes the parked one — the shortcut had the right behaviour and the shell's natural end did not.
- Now child death calls `close_tab_at(dead)` like any close and the app exits only when nothing is left to promote (`exiting = true` + `redraw()`, with `RedrawRequested` closing the loop so it never exits mid-drawing).
- Verified both ways: 2 workspaces × 1 tab → the parked one is promoted and the socket stays; 1 × 1 → the app closes. `I20` went `partial` → `done` and `M-W` left the queue.

## 2026-11-02 — Wave 2: the drawing gate (G16, B28, G15)

- The app drew once per pty read, so under a continuous dump the total cost went up without the screen getting faster.
- `frame::Gate` (`crates/app/src/frame.rs`) is a pure type with four tests: at most one frame each 16 ms; nothing is drawn while `?2026` is open (drawing then shows the **middle** of the repaint); the close draws at once; and a 200 ms ceiling — an app that opens and forgets does not freeze the screen, and the gate opens **another** 200 ms window instead of going back to drawing per read.
- One order matters: the pty `Wake` now `drain()`s **before** asking about `?2026`, or the first frame of every transaction came out in the middle of it.
- On 900 doom-fire frames at 61x18 draw calls fell from 1499–1585 to 63–74 with the same fps (1012–1032), ~10× less CPU drawing on the same load; live with a pty transaction, 4 drawings in 0.4 s and the whole final state after the close.

## 2026-11-02 — Wave 3: the window remembers where it was (J2–J6)

- `crates/app/src/state.rs`, as pure as the `state.py`: JSON in `$XDG_STATE_HOME/tortty/window.json` (`~/.local/state` as fallback), atomic tmp + rename, and no state failure disturbs the close; nine tests without display (garbage size, corner outside every monitor, no position on Wayland, junk JSON falling to the default).
- Precedence: an explicit grid (`--cols`/`--rows`) **turns off** remembering (it is a measurement — the `PYTERM_GEOM` role), else the remembered state if it passes `fit`, else the Python's 980×640, not the grid.
- The pitfall the Python had documented (`window.py:1009`): saving the **client** position drops the window one title bar per run; here the saved position is the outer one from the `Moved` event, the same one `with_position` consumes — three runs gave `120,90` with no pixel of drift.
- Maximized is separate state (J5): `Resized` only updates the normal size when not maximized, so the file keeps `{w:900,h:600,max:true}`, reopens maximized and unmaximizing gives 900×600 back.
- Wayland (J6): `backend_x11()` decides by `WAYLAND_DISPLAY`/`DISPLAY` because `winit` does not say which backend it chose, and without X11 the position is neither requested nor saved (code + test, no Wayland session here). Live: `900×600` at `120,90` restores, `100×80` and a corner at `9000,9000` fall to `980×640`, crooked JSON → default with the file rewritten sane — and the schema is the Python's (`x`,`y`,`w`,`h`,`max`) on purpose, so the two terminals read each other's state.

## 2026-11-02 — Wave 5: the kitty images (H1–H13)

- `crates/core/src/graphics.rs` is the analogue of `graphics.py`: APC, base64, zlib, PNG (only the IHDR, the decoder stays in the app), `a=t/T/p/d/q`, `m=1`, `C=1`, `q=`, `c/r/X/Y`, placements on the screen line and the lifecycle (scroll, reflow, `ED 3`) — 12 tests in the module, 6 in `Term` and one check in `--selftest`, all without display.
- In the app: one texture per image (cached by `versao`, which changes on every transmission, since the app may reuse the `i=`), its own pipeline sampling RGB instead of the atlas mask, drawn **between** grid text and chrome/cursor (kitty's order) with `linha + view_off`, so images follow the history.
- Two defects that only appeared when using it for real: the pty sent `ws_xpixel`/`ws_ypixel` = 0 (where `icat` gets the resolution — now `PtySize` carries `cols*cw` × `rows*ch`), and the parser kept the default 10×19 cell size, wrong for the default `c=`/`r=` and for the `CSI 14/16 t` responses (14 t is answered now, as the Python does not).
- Live: `kitten icat` draws, two `icat` on one screen use two textures, an image pushed 31 lines into the history appears in place with `Shift+PgUp` (`view_off=7`), and `ED 3` clears images and history while `ED 2` does not.
- `state` gained `images` and `placements`, without which the only way to check a placement was to look at the screen.

## 2026-11-02 — Windows and macOS leave the panel (they become queue)

- `M6` (Windows), `M7` (macOS) and `M9` (signing) had been `missing` since the first day, but Python never ran outside Linux — by the single-owner rule a new target is queue, not migration, so they became `M-X`, `M-Y` and `M-Z` (signing behind the other two: without a compiled target there is nothing to sign).
- What stays is what Python delivers: `M4` (Wayland) and `M5` (GL backend, since GTK runs without Vulkan); `M2` stopped promising ConPTY in the same API and `M8` is stated as Linux x86_64/aarch64.
- Panel: 194 done / 3 partial / **5** missing / 3 no = 205 lines, **95.4%** — missing only `A17`/`G17`, `G8` and `M4`/`M5`.
- Pointers fixed in `LIMITATIONS.md`, `INSTALL.md` and `PORTABILITY.md`, which now says explicitly that it is analysis, not queue: the **how** of each target lives there and the **when** in the backlog.

## 2026-11-02 — Damage in drawing (A17/G17) and the GL backend (M5)

- `crates/app/src/damage.rs` (new, 11 tests without display) decides per frame between "everything" and "these lines" from an `Assinatura`: tab (monotonic identity — the index comes back after close/open), columns, visible lines, history scroll, visible selection, blink phase **when there is an SGR 5 cell in view**, theme generation and fg/bg.
- Equal signature ⇒ the line-by-line `dirty` holds; different ⇒ whole frame. The `Renderer` keeps the instances **per line** and only concatenates, in the same blend order (grid → images → chrome → cursor → badge → toast).
- Two things easy to forget: damage is consumed **only when the frame was presented** (a skipped frame cannot clear the mark, or the dirty line stays old forever), and the cursor blink does not enter the signature, or the whole grid would be rebuilt twice a second.
- `G17` (damage by region) stays `partial` on purpose: building is per line, but the draw is still the whole screen — the swapchain does not guarantee the previous frame, so drawing a band needs its own target (scene texture + blit), another architecture and not an `if`.
- `M5` had three real defects: the instance was created without a display handle (→ `new_with_display_handle_from_env`), `alpha_mode: caps.alpha_modes[0]` picked `PreMultiplied` on GL (→ explicit `Opaque`), and `request_adapter` took the first one (→ `choose_adapter`, hardware beats CPU); `tortty: GPU: <backend> <board>` went to stderr and `last_frame` to `state`. Unverified in this sandbox (no `/dev/dri`, read-only cache — every frame `skipped`), which is what `DEBUG.md` "degraded sandbox" is for.

## 2026-11-02 — Confinement: what tortty has to withstand

- A whole session inside `sandbox` (bubblewrap) showed three things, the heaviest becoming queue (`M-AA`) instead of a last-minute fix.
- **Drawing without GPU**: with `$HOME` read-only and no `/dev/dri` the app comes up, the pty works and `state` answers, but **no frame comes out** (`skipped` on every frame, llvmpipe) — diagnosis and the software fallback go into `M-AA`.
- **State without writes**: `state::save_to` already failed without blowing up, but **silently**; now a line goes to stderr and there is a test (`save_in_impossible_place_does_not_blow_up`).
- **Install into a blocked target**: `install.sh` already exited 1 with `sudo`'s dump; now a target inside `$HOME` without write permission does not try to escalate (it says "home read-only? use `--prefix <writable dir>`") and a system target warns that `sudo` needs a password there.
- Least privilege: what is missing is `~/.cargo/registry`, not all of `~/.cargo`; the `ro` mount of `$HOME` makes the bind inherit `ro` (a mounting problem, not sandbox policy), and `CARGO_HOME` in `/tmp` solves it without granting anything.

## 2026-11-02 — A17 closed: damage became real partial drawing

- `A17` was `partial` because the logic existed and nothing was saved: `state` showed **20 lines rebuilt per written line** on a 20-line grid, and the new `frames_all`/`global_why` fields put the culprit in the core, not the renderer.
- Four core fixes: `scroll_up` marked `dirty_all` and in a terminal **every new line scrolls** (→ full-screen scroll became `scroll_shift`, which shifts the finished lines and rebuilds only the blank coming in); `IL`/`DL` marked the whole screen (→ from the cursor to the end of the region); `ED 0`/`ED 1` rebuilt everything (→ only the side they erase, `ED 2`/`ED 3` stay global); a net displacement of zero would lie (→ `scroll_conflict` forces a rebuild).
- The instance is built **relative to the line** (`y = 0`) and the displacement enters at concatenation — that is what allows running a finished line without rewriting coordinates or rebuilding glyphs.
- Measured at 60x12: `echo UMA` 20 lines + 1 global frame → **1 line** + 0; `seq 1 100` 20 per burst → 12 once then 1 per line; `mc` 20/frame → 64 lines in 13 frames with 3 global. On screen `seq 1 30` and `mc` (panels, colours, DEC frame, key bar) are aligned, two new tests pin the scroll contract and the `IL`/`DL`/`ED` precision, and the suite went 74 → 79.
- Only `G17` stays `partial`: the draw is still the whole screen.

## 2026-11-02 — G7 closed: color emoji and CJK (a font that does not exist and a font that does not draw)

- Emoji and CJK were blank because the glyph did not exist, and there were **two** causes, neither "missing font": a single fallback face that did not cover CJK (→ chain of `sans-serif`, `DejaVu Sans`, CJK, emoji) and `query()` falling back to the generic family when the **name** was missing ("Noto Sans Symbols 2" became FreeMono and drew an outline — a missing font now returns `None`).
- 😀 stayed blank even with `Noto Color Emoji` installed because `DejaVu Sans` maps U+1F600 and produces no bitmap, and the first cmap with `gid != 0` won; resolution is now two passes — **color** bitmap over the whole chain, then mask.
- Colour needed what did not exist: `Atlas::new_rgba` plus a draw call in the image pipeline (image bind group, not the mask atlas one), and the RGBA constructor rebuilds the pending white block at 4 bytes/pixel, since the 1 byte/pixel one overflowed `write_texture`.
- Checked on screen (Vulkan/RADV, 80x20): colour emoji, CJK, arrows, Greek and emoji inside bold; two new tests pin that an emoji comes out **in color** and that a missing font cannot become another font. No regression on doom-fire: 872–963 fps / 37.5–41.4 MB/s at 61x18.

## 2026-11-02 — M5 closed: the GL backend draws with a real GPU

- `M5` was `partial` because GL only initialized and the visual verification had died with the container's `/dev/dri`; under `sandbox @gpu,dev`, `WGPU_BACKEND=gl` comes up on `Gl AMD Radeon Graphics (radeonsi, renoir, ACO)` and draws — `last_frame: drawn`, text, colours, cursor and `mc -V` in the screenshot (the pending X error panic was llvmpipe's, not GL's).
- It is recorded in `DEBUG.md` that the first question facing "the window does not paint" is `ls -la /dev/dri`: without the nodes both backends stay on llvmpipe.

## 2026-11-02 — G8 closed: ligatures (shaping) and the `ligatures` key

- `G8` was the last `missing`: `swash` rasterizes and does not shape, and Python shaped through Pango; measuring the fonts decided the design — DejaVu Sans Mono declares `liga`/`dlig` and **does not ligate**, Fira Code and JetBrains Mono carry only `calt` and swap each cell's glyph for a piece that joins the neighbour, and only a proportional font merges cells (`fi` → 1 glyph).
- So **a mono font does not merge cells**: the cell stays one character and drawing only asks HarfBuzz for the right glyph **with the run's context**; the "one glyph covers N cells" path is guarded by an advance rule (the glyph must fit N cells ±10%) and by a cluster-order rule (RTL goes back to cell by cell; bidi is not done here).
- `crates/app/src/shape.rs` shapes per run of the same face and style, with a cluster → cell map and a cache by `(face, mode, text)`; the plan speaks position **inside the run** (an absolute column drew the wrong glyph — caught in the capture, fixed with a test).
- A **single-cell** run is not shaped (no context, no ligature), which keeps doom-fire's 2400 runs of 1 free; the **cursor and selection break the ligature** (the cursor cell and its neighbour become a run), and the `ligatures` key (`off`/`standard`/`all`, default `standard`) turns off ligatures, not shaping, since Arabic and Devanagari need shaping; test font JetBrains Mono (SIL OFL 1.1) lives in `assets/fonts/`.
- Cost: **88.5 µs** per new line (32.4 µs parsing the font, 56.4 µs shaping) and **57 ns** on the cache, paid only by the line that changed (A17). Verified in `Xvfb :77` + `import -window <id>`: `->` becomes an arrow, `!=` becomes ≠, `fi`/`ffi`/`fl` ligated, and the cursor in the middle of `fi` undoes the pair.

## 2026-11-02 — N1, N7 and M-AA: a test that does not open a window, a test that opens one by itself

- `N1`: `--selftest` only ran the core, leaving `cli.rs`, `keys.rs`, `mouse.rs`, `state.rs` and `debugsock.rs` out of reach (six `checks.py` checks); `crates/app/src/checks_app.rs` has the six, `--selftest` runs **31** (25 + 6) and `cargo test` runs the same list (`app_checks_pass`), so a new check does not stay in the binary only.
- Two expectations came out wrong, which is why the check exists: `?1005` column 300 is `c5 8d` (U+014D) while the old protocol saturates at 222, and `desktop_shortcut` demands the `install.sh` contract (Exec/TryExec/Icon/Categories/terminal, removal on `--uninstall`) plus the SVG embedded with `include_str!`, so it holds in the installed binary, far from the repository.
- `N7`: `tools/pixel.py` reads the capture as PPM without `gi` and **compares** (`--diff`, `--max`, `--expect`, `--scan`/`--row`), and `tools/visual.sh` brings up its own Xvfb with a fake `HOME` and fixed theme, waits for the first frame through `state`, captures by the `xid` that **our** `state` reports, and demands four things (new content changes a pixel, SGR 41/42 paints `color1`/`color2`, text falls inside the cell, two captures of the same screen match) — exits 0/1. Searching the window by class was the test's own bug: with two instances it captured the wrong one and came out black, twice.
- `M-AA`: `state` now separates **`drawn`** from **`skips`** and the app warns **once** on stderr when the surface refuses the frame, pointing at `DEBUG.md`; backend failover was left out on purpose — it cannot be written blind, since `Xvfb` does not even come up inside `sandbox` (segfault in `InitFonts`), so the trigger (`skips > N` with `drawn == 0`) stays written in the backlog.
- Doc debt paid in the same step: `M4` and `G17` left the panel at the owner's request and became `M-AB`/`M-AC` (the panel ended at 203 lines, 200 done / 3 no), `§16.2` stopped listing 27 checks that already existed under another name, and the 1 px seam of box drawing entered as `M-AD`.

## 2026-11-02 — migration closed (and the closure paperwork)

- The panel reached zero parity pending: `H17` (`t=f`/`t=t`/`t=s`), `H18` (animation) and `H19` (sixel/iTerm2) were marked `no` with the `ref` column saying "they do not exist" — that is, Python does not have them either, so by the panel's own rule they could not be there.
- They left as `B1`, `B2` and `B3`, with trigger and cost written, because the decision stopped being "we will never do it" and became "when someone needs it"; `LIMITATIONS.md` stopped claiming them — only the fact that still holds stayed (image embedded in the escape, base64) plus the sentence that file/SHM, animation and sixel are queue, not a limit.
- **Panel: 200 of 200 = 100%** at that moment (195/195 after the five `done`-but-not-parity lines left), area 8 closed at 13 and nobody stayed `partial`.
- `§16` stopped being a plan and became the record of the order the migration followed, with the criterion that closed each slice (`§16.1` = "rules that held along the way"), and `§16.2` gained the map of the three checks that changed name (`graphics_kitty`, `desktop_shortcut`), so nobody "discovers" missing coverage where there is none; `AGENTS.md` changed the Python rule to past tense, `README.md`/`MIGRATION` stopped saying "in progress", and the version became **1.0.0** — closing the migration is the only milestone the patch convention cannot express.
- Five `done` lines were not parity either and left the panel with the day each closed (`C5`, `C6`, `M1`, `M5`, `M8`), so the count stopped inflating: **195/195 = 100%**, and what is still open is not parity — `M-AB` (Wayland on a compositor) and `M-AC` (drawing by region).

## 2026-11-03 — B4: drawing painted every line at `2r`

- Owner's symptom: every `Enter` "jumped two lines" and the cursor appeared far from the prompt — and the grid was **right** (`state` and `stty size` agreed), so the error was drawing, in two places both introduced by the per-line drawing of `A17`.
- `paint_line`/`paint_glyphs` positioned each quad at `y * ch` and `concat_lines` added `y * ch` **again** (line `r` came out at `2r`), and on scroll `rebuild_rows` rebuilt only `len - 1` instead of the `n` lines that came in, leaving blank bands.
- Measured instead of guessed: a distinct `48;5;N` background per line said which line fell in each 27.3 px band — before, band `r` showed line `r/2` (1 hit in 15); after, identity 15/15 in `bash` and `zsh`. It became step 5 of `tools/visual.sh`.
- A **third** error only appeared when the owner complained again: content started in the middle of the screen because `resize` pushed the blank lines below the cursor into the history and gave them back above the prompt (the cursor went 0 → 12) — the rule already existed in the Python's `_reflow`, brought over with a unit test and step 6 of `visual.sh`.
- Two operational facts recorded: capturing an **obscured** window returns what is on screen at those coordinates, and `import`/`xwd` grab the pointer with a crosshair — screen capture only on its own virtual display.

## 2026-11-03 — B6: write and scroll in the same chunk, damage did not scroll along

- `time sleep 1` sometimes left the timeline line **blank** and dragging the mouse made the text appear — which already said where to look: the text was on the grid and what went stale was the drawing's per-line cache.
- Deterministic reproduction: 20 attempts on a 40-line grid, the 20th with the screen full; `time` writes the timeline on the last line and the `\r\n` scrolls **in the same chunk**, so the renderer rotated the cache (`cache[38] ← cache[39]`) but the damage mask did not rotate with it.
- Fix in the core, both directions: when scrolling with a usable displacement `dirty` **rotates with the content** (`rotate_left`/`rotate_right`, the new line marked), and `scroll_conflict` now forces whole-screen damage instead of only turning the rotation off.
- Measured after: 40 attempts with no divergence and `last_damage=rows: 2` per frame, so the `A17` optimization holds; new gate is step 7 of `tools/visual.sh`, writing one line at a time past two screens and comparing grid × pixel line by line.

## 2026-11-03 — B7: `StartupNotify=false` in the shortcut

- Opening from the system shortcut showed **two** taskbar entries (`tortty` and a "Starting tortty") with the pointer busy for seconds: the menu entry asked for `StartupNotify=true`, so the launcher waited for the app to say it finished (`DESKTOP_STARTUP_ID` + `_NET_STARTUP_INFO`) and tortty never said it.
- Owner's decision: the simplest way out — `StartupNotify=false` in the `.desktop`, and the "opening" notice that would be lost depended on that same answer, that is, it never worked.
- If the app one day completes the notification (`winit` has `read_token_from_env` + `with_activation_token`, and its X11 sends the `remove`), the key goes back to `true` along with the code.

## 2026-11-03 — B5: an X error from the WSI no longer kills the app

- `sandbox tortty` exited with **134** in `set_window_title`, because `winit` keeps the last X error and `expect()`s it on the next flush; the new `TORTTY_XERR=1` probe showed Mesa's WSI leaving `BadDrawable` (Present) and **`BadAccess` (DRI3Open)** pending without a GPU.
- `crates/app/src/xguard.rs` registers winit's public hook and swallows `BadDrawable`/`BadWindow`/`BadMatch` in any request and `BadAccess` only in an extension, with everything going to stderr (the first three occurrences, then sparsely); measured, `sandbox tortty` without GPU stays alive 12 s, answers on the socket and keeps drawing.
- Stage 2 is environment, not code: with `xwd` of the window compared byte by byte, `sandbox @gpu` (RADV) presents, Vulkan lavapipe grows `skips` with a black window, and GL llvmpipe grows `drawn` with `skips 0` and a **pixmap that does not change** — so `drawn > 0` does not prove a drawn screen.
- The app's message says that in one line with the way out (`/dev/dri`, `sandbox @gpu`), `DEBUG.md` gained the table and the `xwd` method, and `LIMITATIONS.md` records that there is no software backend of our own.

## 2026-11-04 — B4: garbage before the tab title (two bugs, not one)

- Two independent causes. First, OSC 0/2 did not close on the 8-bit ST: `0x9c` (or `c2 9c`) left the OSC **open** and everything after entered the payload until the next `BEL`, and an `ESC` in the body was pushed back, gluing the next title inside this one — reproduced as `"GARBAGE\u{9c}\x1b]0;CLEAN"`.
- Second, and in every tab: the tab number was drawn by **Noto Color Emoji**, because the path without shaping swept the whole chain in the color pass first, and Noto maps ASCII digits to a 19x18 px bitmapped rectangle (the label came out as `b$Tb$`); measured face by face, and the grid was spared because it draws through shaping.
- Fixes: `0x9c` closes OSC and DCS/APC/PM/SOS with a guard against a UTF-8 continuation byte (`М` is `d0 9c`) and an `ESC` that is not ST ends the sequence; `sanitize_title` (core) drops control, zero-width, direction marks and BOM **at two levels** — `Renderer::chrome_text`, the only place drawing outside the grid, and the producers (OSC payload, `Tab::cwd_name`, rename typing) — and `glyph.rs` resolves the cell font first, keeping the color-before-mask order for emoji.
- `app.rs` gained the `osc` socket verb with the last 8 raw payloads (hex, text, truncated, resulting title), because "what bytes the child sent" only the parser can answer.
- Verified: 8 new core tests (8-bit ST pure and in UTF-8, `ESC` in the body, direction mark, DCS with `0x9c`, `М` not cut, the probe ring) plus 1 in `glyph.rs`; live with three tabs, the 12 OSC cases gave the expected title and the bar capture shows `1 CLEAN │ 2 b4 │ 3 b4`.

## 2026-11-04 — code, docs and scripts in English

- Owner's request: translate everything — 322 identifiers renamed by a curated part map (`linha`→`row`, `desenha`→`paint`…), then the leftovers hunted by name.
- Prose translated as well: comments, user-facing strings, test names (now sentences: `reflow_narrows_and_widens_preserving_the_text`), the 12 docs, `README`, `AGENTS.md`, the scripts, the workflows, the shaders and both `Cargo.toml`/`rustfmt.toml`.
- **Not touched** on purpose: `DEFAULT_CONF` keys (a test reads each name in `CONFIG.md`), CLI flags, socket verbs and their JSON, protocol notation and the historical `arquivo:linha` of the Python repo, plus test data that is Portuguese on purpose (`é` = U+00E9, the real `xdotool` output `ãéù`).
- 187 stale `arquivo:linha` references in the docs were checked to still land inside their file.
- Four regressions from the mechanical pass, only one seen by the compiler: `SO_PEERCRED` → `ONLY_PEERCRED`, `starts_with`/`ends_with` → `start_with`/`end_with`, `json.loads` → `json.load` in a script that only `visual.sh` exercises ("a gate that is not run does not exist"), and `matar()` → `kill()` in `devkill.sh`, where a POSIX shell function shadows the builtin and recursed with the `|| true` swallowing it — hence the rule that a function is never named after a builtin. Also `.gitignore` had `pi/` while the session directory is `.pi/`, and two session files had been committed.

## 2026-11-04 — rebrand: `bzmux` → `tortty` (TorTTY — Torbie TTY)

- Symbolic name `tortty`, friendly name **TorTTY — Torbie TTY**, headline *Fast, GPU based terminal*, repository `bazoocaze/tortty`; the name research is in the unversioned `docs/NAMING.md`.
- Directories stopped carrying the project name — `crates/bzmux-core` → `crates/core` (package `tortty-core`) and `crates/bzmux` → `crates/app` (package `tortty`) — which is the vocabulary the architecture already used.
- 449 occurrences: `tortty_core::`, binary, `BZMUX_*` → `TORTTY_*`, config and state paths, socket/dump paths, `WM_CLASS`, wgpu labels, desktop entry, icon, installer, tools, CI and release packaging.
- **Not renamed** on purpose: `tortty-python`, `the Python sibling` in prose and the `bzmux/*.py` citations, since renaming them would falsify the history and break the other repo's `arquivo:linha`; and the installer transition was left out by the owner's call (only he had it installed).
- Pitfall: the rename script's protection for `bzmux/<file>.py` swallowed the slash (`bzmuxterms.py`) and it rewrote places where `bzmux` was the *subject* — the entry about `tortty-python/shortcut.py` deleting `bzmux.desktop` and the `pkill -x` warning — which only a read of the diff catches.

## 2026-11-04 — B36: dead key followed by space lost the accent

- `~` then space came out as a bare space while `~`+`a` gave `ã`: the composition is done by `xkbcommon` inside winit and the result lands in `event.text`, but `~`+space arrives with `logical_key` still `Named(Space)` and the encoder read the named table (`" "`) and threw the composed text away.
- Fix in `keys.rs`: on the named path a `text` that differs from the key's own base and carries a printable character wins, keeping the ESC prefix for Alt; control characters are not composition, so `Backspace`'s `\x08` still goes out as `DEL`.
- Verified in Xvfb with `xdotool` (the real path through winit) and a child recording its stdin: before, `~`+space, `´`+space and `` ` ``+space reached it as three spaces; after, as `ã` `~` `'` `` ` `` — the `'` for `dead_acute`+space is the system Compose table, not a second bug. Test `dead_key_composed_over_space_keeps_the_dead_character`.

## 2026-11-04 — B37: the tab title went back to following the cwd

- The title followed the cwd until the first program that set its own title, and after leaving `mc` (or `vim` with `titlestring`) it stayed on the last OSC 0/2 for good, though a new program's title still came through.
- Cause, measured rather than guessed: apps restore the title with xterm's title stack, not an OSC — a pty harness shows `mc` sending `CSI 22;0t`, `CSI 23;0t`, `CSI 22;0t`, `OSC 0 mc [...]` and a `CSI 23;0t` on exit, and both `22` and `23` fell into `_ => {}`.
- There is one title here (`OSC 1` is ignored since `C10`), so any `Ps` of 0/1/2 saves and restores the same thing and the sequence stays symmetric; the stack is capped at 16 (`TITLE_STACK_MAX`).
- No app-side change was needed: the pop restores the initial `tortty` and the "OSC wins, else the cwd" rule already treats `tortty` as "no request".
- Verified by reading the **window** title (`xdotool getwindowname`; `state.title` is `Term::title`, a different thing, recorded in `DEBUG.md`): after leaving `mc`, `etc — tortty` came back and the next `cd` gave `tmp — tortty`; three core tests and both gates green.

## 2026-11-04 — B38: closing by the shell lost the window state

- Maximize, `exit`, reopen via the shortcut and the window came back normal — and the cause was a **panic**, not a missing save: once the last tab was gone, `wscards()` (from `redraw()`, called by `drop_empty_workspace`) indexed `tabs[active]` and panicked, and after that was fixed `exiting()` panicked killing the pty of a tab that no longer existed.
- Closing through the WM never hit it, which is why it looked like "it does not save".
- Fix: once `exiting` is set `window_event` returns on anything but the two events that leave the loop, `drop_empty_workspace` only sets the flag instead of drawing with no tab, and `exiting()` reads the tab with `get_mut`.
- Measured under Xvfb + **Marco** (the owner's WM) with its own `XDG_STATE_HOME`: the WM button wrote the file, `exit` and Ctrl+D wrote nothing, and stderr showed the panic at `app.rs:561`; now all three write `{"max":true,...}` and the next launch is born maximized, with step 8 of `tools/visual.sh` failing on the old code with `rc=101`.

## 2026-11-04 — touchpad scroll: the view kept moving after the finger left

- Reported as "smooth scroll is weird" and then as inertia. The probe logged every wheel event next to the driver's side (`xinput test-xi2 --root`).
- `scroll_view()` drew a frame **inline** for every wheel event, and a touchpad delivers up to ~125 events/s while the app consumed ~37/s — the rest queued in the X server and kept draining after the finger stopped, so the "inertia" was backlog; the frame is now requested and one draw per frame comes from `RedrawRequested`.
- The wheel's 3-line notch factor (D4) was applied to the touchpad too, but X11 smooth valuators arrive as a *fraction* of a line, so every ~15 px scrolled 3 lines; the factor now applies only to a notch (exactly ±1.0) and the fraction accumulates in `scroll_frac` (`mouse::wheel_lines`, with a test).
- Measured in one 9 s gesture: 337 events, ~37/s, 0.58–2.06 lines per event (22–132 lines/s) — the magnitude carries the finger speed, which is why a fixed step per event was tried and rejected.

## 2026-11-05 — B4: the drag paid one present per mouse event

- `fn redraw()` drenches the pty, builds the frame and presents **synchronously**, so a handler calling it from a motion path paid one present per event: `drag_select` and `bar_drag` (which even drew twice, since `move_tab` also redrew).
- A touchpad's ~125 events/s grew the X queue and the drag kept going after the hand stopped — the wheel's trap in another path, and the reason the lag appeared "in several points".
- Measured with `PROF` under Xvfb, 120 motion events per drag: selection **57 presents → 3**, tab reorder **9 → 3**, with the selection ending on the same cells and the drag still reordering.
- The socket was the third number: `UserEvent::Debug` asked for a redraw on **every** verb and `benchrun.py` polls `state` in a loop, so 60 polls were 61 presents — now **0**, since `debug_read_only` skips the verbs that only read.
- The keystroke and action paths (rename, paste, new/close tab, new workspace, config reload) keep the immediate draw on purpose: one present per action is the latency they want, a queue is what they must not have.

## 2026-11-06 — M-S: F13–F24 and the numeric keypad

- F13..F24 have no sequence of their own: on a PC keyboard they are F1..F12 with Shift (`kf13=\E[1;2P` in terminfo), so the base is the F1..F12 sequence, the Shift is part of the key, and a modifier on top adds to it (Ctrl+F13 = `CSI 1;6P`); under the kitty protocol they exist as `CSI <n>u` with their own numbers (57376..57387), and the socket's `keys` verb learned the same twelve names.
- The keypad was **mute** and not only with NumLock off: winit leaves `KP_0`..`KP_9`, `KP_Decimal` and the operators out of its keysym table, so with NumLock on they arrived as `Key::Unidentified` and were dropped — `resolve_key` now falls back to the physical key for exactly that case.
- With NumLock off it already worked: winit maps `KP_Home`/`KP_Up` to the navigation keys, the logical key wins and the keypad keeps acting as the arrow cluster (`KP_Enter` as `Enter`).
- Verified with `f13_to_f24_are_f1_to_f12_with_shift` and `the_keypad_sends_its_character_even_with_num_lock_on`. What stays out: `DECKPAM` is still a no-op in the parser, so the keypad always sends the character and never the `\EOw` of the application keypad.

## 2026-11-06 — M-E: double click selects the word, triple the line

- The click count lives in the tab (`Tab::click`: anchor, count, instant) — 2 takes the word, 3 the line, 4 starts over and a click on another cell resets it — while the rule itself is in the core (`Term::word_span`, `Term::line_span`), testable without a display.
- A word ends **only at whitespace**, which is what makes a path, a `-rw-r--r--` or an `--option=value` come out whole (the case the item exists for), and a soft-wrapped line is **one** line, since the span of a `wrapped` row goes on into the row below.
- That forced a copy fix: `span_text` put a `\n` between every pair of rows (inventing a break over a wrapped line) and trimmed the trailing spaces of a `wrapped` row, which are content; now the break only exists where the row is not `wrapped`, `Row::text_cols_full` keeps them and `selection_text` follows the same rule.
- Verified on a real window with `ls -l /tmp/a.txt fim`: double click on the path gave `a=[0,6] b=[0,15]`, triple gave `a=[0,0] b=[0,21]` without the padding, and a drag after the word click extends from the start of the word; three unit tests cover it.
- The rectangular selection (the fourth part of the item) became `B5`: it is a different copy (a column range) and does not fit in the same span.

## 2026-11-06 — M-AD: the 1 px seam of box drawing

- Measured before touching anything, with a private Xvfb: three rows of box drawing (`─`, `┼`, `│`) at a 9.63 px cell left **25 of 386 columns** at 83% of the ink — a dim column at every other cell boundary.
- The cause is the antialias: the glyph is drawn at the fractional advance and its mask ends in a partially covered column, so neighbours meet with less than full ink; box drawing and block elements exist to touch the neighbour, so `draw_cell` draws them half a pixel wider on each side **when the ink spans the cell** (`gw >= cw - 1`), while a stroke like `│` stays where the advance puts it.
- Rounding the origin — what the item suggested — was measured first and changes **nothing** (a sub-pixel shift moves no sample: 2 px moves 5370 pixels, 0.37 px moves none).
- After: **0** dim columns, 385 of 386 at full ink (the last is the window edge), and `tools/visual.sh` still passes.

## 2026-11-06 — `OSC 7`, `?1016` and `LNM`: three cheap facts from the parser

- Three escapes/modes that were consumed and thrown away, none of them touching the hot path: `OSC 7` is one arm in an existing `match`, `LNM` one flag read where `LF` already goes, `?1016` a reuse of the SGR encoder and the cell geometry.
- **`OSC 7` (shell cwd)**: the payload is `file://host/path`, the authority is dropped and the path percent-decoded (`a%20dir` → `a dir`); an empty payload clears the cwd and a non-`file://` payload does not overwrite a known one. `/proc/<pid>/cwd` stays the first answer (it is where the program in front is) and the shell's word the last resort, only when the path is still a directory.
- **`?1016` (mouse in pixels)**: the shape stays SGR and only the number changes — `Proto::pixels` makes the encoder emit SGR and `App::grid_px` gives the offset from the same origin `cell_at` divides by; **`LNM`** (`CSI 20 h`) makes `LF` return to column 0, while `IRM` stays out because inserting a cell means a branch in `put_ascii_run`.
- Verified by 4 new core tests and 2 in the app, and by hand: with `?1006` the child got `ESC[<0;6;3M` for a click on cell `10x19` and `ESC[<0;56;48M` with `?1016` (`5*10+5`, `2*19+9`), and a child printing `OSC 7;file://host/home/user/a%20dir` arrived as `cwd=/home/user/a dir`; `state` learned the two fields. Found on the way: **DECALN was already implemented**, so `M-U` became combining-characters-only.

## 2026-11-06 — `M-G`: kitty flag 4, the alternate keys

- The protocol has three slots in the first field (the key as it is on the board, the same with Shift, and the key in the base layout — `CSI 97:65:97;2u` for Shift+A) and only the first was sent, with the **post**-Shift code `CSI 65;2u`, which is what `winit` delivers.
- With flag 4 the letters are the case that can be undone without a layout table: `A` in `0x41..=0x5a` is `a` shifted, so the pair is written out; a shifted punctuation mark stays single-field on purpose (the base of `!` is `1` on a US board and another key on ABNT2) and an absent field is honest where a wrong one is not, while Ctrl+Shift+a already travels as the base key (`97;6u`).
- Verified with one test in `keys.rs` (no flag 4, with flag 4, punctuation, Ctrl+Shift) and through the socket on the real binary: with `ESC[>12u` (flags 4|8) the answer is `ESC[97:65:97;2u`, with `ESC[>8u` it is `ESC[65;2u`.

## 2026-11-06 — `M-C`: OSC 8, hyperlink on the cell

- The escape is state now: `Cell` carries `link: u16`, an index into `Term::links` (`0` = none), with the span being the pair of `OSC 8` (an empty URI closes); the index and not the URI because the URI repeats in every cell, and repeated URIs share their index (`ls --hyperlink` re-links every name) while the params (`id=`) are ignored and the table stops at 512.
- The width was the interesting part: with `u32` the cell went 20 → 24 bytes and `parsebench` fell from **12.26–12.43** to **10.51–10.57 MB/s** (-14%, far outside the spread); with `u16` the field lands in existing padding, the cell stays at **20 bytes** (asserted by a test) and the same bench reads 12.20–12.29 MB/s — the whole reason for the type.
- On screen Ctrl underlines every link at once and Ctrl+click hands the URI to `xdg-open`, and only `http`/`https`/`file`/`ftp`/`mailto` are opened, since the URI comes from the child and `xdg-open` runs whatever handler the desktop registered; the underline enters the damage signature (`Signature::links`), or the toggle would never repaint.
- Verified with 3 core tests (open/close, index reuse, wide char carrying the link, control characters dropped, the cap), 1 pinning the cell size, and by hand: holding Ctrl changed **39 pixels** in a `39x1` box at y=16 and a Ctrl+click reached a stub `xdg-open` with exactly `https://example.com/x` — the same probe found that `xdotool key` under Xvfb does nothing without `windowfocus`, now in `DEBUG.md`.

## 2026-11-06 — `M-M`: the scrollbar

- An 8 px strip on the grid's right edge, drawn only when there is history: a translucent track (10% of the foreground) and a thumb (55%) whose height is the share of the buffer the view shows, at the bottom while following the output and at the top at the oldest line; a press jumps and the drag walks it (`scroll_drag`, the motion belongs to the view and not to the selection).
- It is drawn **over** the last column instead of reserving a strip: reserving would narrow every terminal for a control that only exists with history.
- The first version was 3 px and did not work — clicks near the edge started a selection, because 3 px is a hard target and the hit test ended at the grid's edge (`cols * cw`, rounded down), leaving the window's right margin dead; the strip is now 8 px (`render::SCROLLBAR_W`, the same number on both sides) and the target runs to the window edge.
- Nothing touches the core (the numbers were already there: `scrollback.len()`, `view_off`, the grid border in `pad`/`bar`/`side`), and the thumb enters the frame after the grid and before the chrome.
- Verified with `seq 1 200` (history 193, 386x149, strip at x=384): at `view_off=0` the bottom pixel is `7f7f7f` and the top `161616`, a click at the top gives `view_off=193` with the colours swapped, and a drag to the middle lands on `view_off=97`.

## 2026-11-06 — build: `codegen-units` back to the cargo default

- `[profile.release]` carried `codegen-units = 1`; trying `4` cut the app-only rebuild from 51 s to 31 s, but the full rebuilds that followed (102 s, 115 s) measured "a whole world", because any profile change rebuilds all 191 dependencies and with `lto = "thin"` every extra unit becomes a module in the serial thin-link.
- The rematch under `schedutil` (the machine had been on `powersave`, which alone cost ~30 s per build) settled it: incremental 18.6 s (default) vs 19.3 s (4/16) vs 22.4 s (all at 1), with a ~19 s link floor.
- Runtime, default vs all-at-1 over the same battery (600 frames, 2 shots): 772.8 vs 850.0/797.6 fps at 61x18, 755.1 vs 755.2 at 70x21, `draw` 0.87–0.93 vs 0.87–1.07 ms/call and `parsebench` 38.06 vs 38.28 MB/s — inside the run-to-run spread (one binary alone swung 638–755 fps).
- So the two lines left: default everywhere. Price declared: the binary grows 12.09 → 14.07 MB.

## 2026-11-07 — `M-K`: OSC 9/777 notification with tab badge and card alert

- The bell could flash but carried no text; now `Term` counts `notifs` and keeps `notif_text` (capped at 512), `OSC 9` takes the whole payload and `OSC 777` only the `notify` subcommand (`title: body`), and the app mirrors the bell drain through one `pump`: the foreground tab toasts, every tab fires window urgency, and `ring()` is gone.
- Two watermarks because delivery is not acknowledgement: the drain advances `notifs_seen` (toast + urgency, exactly once) while the badge lives on `notifs_acked` — the tab label while in front, the sidebar card (`WsCard.alert`) while parked.
- The toast and badges ignore keys for 3 s (`NOTIF_FLOOR`), then the first delivered key or the visit acknowledges; a parked card survives keys typed elsewhere and clears on entry. The drain feeds parked workspaces now too (feed only, no title or draw), which before waited for the return, and the view still follows the output.
- The probe tripped on the design once: the first version marked seen at drain and no badge ever stuck.
- Verified on Xvfb: foreground toast (1724 px), an early key keeps it, a late key clears (`notifs=0`), a background tab draws no toast but 1824 px on the bar, a parked workspace 1003 px on the sidebar, `777` joins, urgency in `WM_HINTS`, plus 3 core tests; `render-check.sh` still passes.

## 2026-11-07 — `B1`: kitty image by file, temp file and shared memory

- `icat --transfer-mode=file` stopped at the capability query because answering `t=f` with `EINVAL` made it report "terminal does not support the graphics protocol"; now the payload of `t=f`/`t=t`/`t=s` is read as a path and the protocol answers `OK` for all three media.
- The core still does no I/O: `Graphics::handle` returns a `FileRequest` (path, whether `t=t` means delete-after-read, `a=T` or `a=t`, plus the request's context), `Term` queues it and the app drains it with `serve_images` before collecting `child_out`, so the reply rides the same frame.
- The answer is `Graphics::provide` (the bytes go through the same direct path, so `f=`, `s=`/`v=` and the 64 MiB cap apply) or `Graphics::missing` (`ENOENT` — the child learns the image is not coming, never which of missing/unreadable/directory it was); the app expands a leading `~` and reads at most one byte over the cap, so an oversized file is refused instead of buffered.
- Verified on Xvfb with a 40×38 red PNG: `t=f` loads and draws (`images: 1`), `t=t` draws and the file is gone, `t=s` draws and the shm file stays, a missing path leaves `images: 0`, and the scrollbar thumb still wins the last pixels (`e67f7f` = 55% over `ff0000`); six new core tests cover the request per medium, `provide`, `missing` and the oversize verdict.

## 2026-11-07 — the toast becomes a box (`B8` follow-up)

- The toast was a strip across the last row with inverted colours: it erased the line and did not read as an aviso; the box cannot use the theme background as its own surface, since a black rectangle on black only shows the border, so the surface is the background veiled 14% towards the foreground — honest in a light theme too and with no key in `CONFIG.md`. Two rects, not one: the opaque background first, or the text shows through the veil.
- Geometry: 1.5-cell gutter on each side of the text (the marker lives in the left one), 8 px above and below, 1 cell from the grid's right edge and 8 px from the bottom, so the box spans two rows and the chrome drawn after the grid costs nothing; the right edge is pinned, so a long text grows to the left.
- The marker is a 6 px `push_rect`, not a glyph, because the atlas is `Format::Alpha` and a coloured emoji would draw as a silhouette.
- Verified on an Xvfb 90×12 (cell 11.2 × 21.7 px): box 438 × 38 px, surface `#1e1e1e` (14% of `#d6d6d6` over `#000000`), border `#cdcd00` on the four 1 px edges, marker at x+5, 12 px and 8 px from the window edges.

## 2026-11-07 — `B40`: the wheel to a reporting child carries the finger's travel

- The branch of `on_wheel` that hands the wheel to a child reporting mouse used the delta only as a **sign and a non-zero test**, so every `MouseWheel` became one whole notch — and a touchpad sends fractions at up to ~125 events/s, so a whisper of the finger was worth dozens of notches (the defect already fixed for the history on 2026-11-04, never applied here).
- Two more things in the same path: the SGR wheel **release** went out (`<64;x;ym`), which no xterm sends and which a TUI counting reports rolls twice, and X11 delivers one detent as press *and* release — two events, both reported.
- The fix is the same rule with the child's own units: `mouse::child_notches` accumulates the travel in a per-tab `child_frac` and emits one notch per detent, dropping the remainder on a direction change (kitty's V120 rule); a detent is one notch and never enters the accumulator, with the press/release pair deduplicated in 10 ms; `mouse::encode` drops **every** wheel release; and the units are raw, never the 3-line factor that belongs to the history. The horizontal pixel delta, discarded before, now goes out too.
- Numbers: three `xdotool click 4` gave **6 reports before** (`<64;…M` plus `<64;…m` per click) and **3 after**; a unit case of 0.4 + 0.3 + 0.4 detent was three notches and is now one plus 0.1 carried. What was not measured is this machine's per-event touchpad delta, so "one detent per notch" is the anchor, with a multiplier knob left to `M-H` if the feel asks for it.

## 2026-11-07 — the toast lives 8 s and no key takes it out

- The toast was 3.5 s and a notification toast could be dismissed at 3 s by the first key delivered to the child — both short of what the notice needs, since the toast carries the *content* of a notification whose *signal* (the badge) already lives longer.
- One lifetime now: `TOAST_DUR` is 8 s for every toast (config notice, `link refused`, notification), and `NOTIF_FLOOR` became the same constant, so a key never acknowledges before the toast would have left; what the floor still guards is the badge, which has no other expiry.
- With the two equal the early-dismissal path became dead code and went: `Toast` lost its `notif` flag, `toast_notif`/`toast_impl` collapsed into `toast`, and `ack_notifs` answers for the badges only; a repeated identical message still counts (`×N`) and restarts the clock.
- Numbers: a child printing `OSC 9` two seconds in, captured at 2.0/4.0/6.0/7.5/8.6/9.5 s of age — pixels differ from the baseline at every step up to **7.5 s** (3710 px, box `270,106 106x35`) and are **0 at 8.6 s**; the previous build was already empty at 4.5 s.

## 2026-11-07 — the `?2026` window gets a log

- The synchronized-output window was invisible — the core flipped `sync_output`, the gate held the frame and nothing said what the child had done inside it; debugging "the view runs to the top while an app repaints" needed the window's **content**.
- `SyncTrace` (`crates/core/src/term.rs`) is pure counters with no clock (`ed[0..3]`, `el`, `home`, `stbm`, `alt`, `ris`, `decstr`, the lines pushed to the scrollback, the bytes and reads that found it open), costing one `Option` test on escapes that are already rare and one per read in `feed`; a `RIS` closes the open trace with `ris×1`, and closed traces wait in a capped list.
- The app drains them into the `--debug-log` after each read, adding the only thing the core cannot know: the **duration** between the read that opened the window and the one that closed it, plus the view offset before and after (`DEBUG.md` has the format).
- A real `pi` turn in 120×34 came fully atomic (`feeds 1, 0 B`, `0.0 ms`), with the repaint done by `EL` per line (`el×1` to `el×18`), never `2J`/`3J`, no `RIS`, no `DECSTR`, no scroll region, no alt screen — and `hist+N` (1..7 lines per window), which is the mechanism that moves the anchor when the view appears to run away, hence `view A→B` in the log.
- Verified with three new core tests (counters inside a window, `RIS` closing it, lines pushed to the history) and the Xvfb run against a synthetic child and `pi`.

## 2026-11-07 — a repaint does not move the reader (`B41`)

- A real `pi` session (5685 windows) separated the two kinds of `?2026` update: **5673** carry only `EL` and are atomic (one read, `0.0 ms`), and **10** carry `ED 2` + `ED 3` + `CSI H` with no `EL` — the transcript wiped and reprinted, 25 KB to 1.1 MB in one window, up to 534 reads — and those are the ones that moved the view (`view 59 → 207`, `view 108 → 387`).
- The arithmetic gave it away: the closing `hist` equals the window's own `hist+`, so `CSI 3J` had emptied the scrollback inside the window; `view_after_output` then added the reprinted lines to the offset and clamped at `scrollback.len()`, and the ceiling of a brand-new history is its **top** — the reader was teleported to the oldest line of a history he never read.
- The rule now has two parts, differing by who asked for the wipe: rows a `?2026` window scrolled count as **repaint** (`scrolled_sync`, subtracted in `view_after_output`), and a history erased **inside** a window keeps the offset clamped to the lines the reader could already see (`FeedMark` carries them); a history erased **outside** a window is a `clear` and returns the view to the live screen.
- `view_after_output` also lost its dead `prev_hist` parameter for a `FeedMark` snapshot (`scrolled`, `redraw`, `history`, `hist_gen`), so the app no longer keeps two loose numbers and a clamp per drain path.
- Verified: a child scrolling inside one window with `2J`+`3J`+300 lines and a reader at `view_off 42` ends at **42** and the next three real lines take it to **45** (before, it ended at the top of the new history); two new core tests plus both gates.

## 2026-11-07 — the touchpad audit: the bar's wheel and the bar's drag

- The audit behind `B40` found **two** left, both on the tab bar: the wheel's branch used the delta as a sign (and `signum() * 3.0` for a pixel delta), so any fraction of a detent was a whole tab, and the 10 ms dedupe only killed the press/release pair of one notch, not a gesture.
- The bar now uses the same rule as the child's wheel (`mouse::notches`, renamed from `child_notches` because it is no longer the child's): a detent is one notch, a touchpad fraction accumulates in `App::bar_frac` and a direction change drops the remainder.
- The reorder drag ran away from a resting hand because `bar_drag` measured travel against the **press** point and never moved it, so every following motion spent the same travel again; the anchor now travels with the tab (`mouse::drag_crossed` returns the destination **and** the travel consumed, and the anchor carries what the tab actually moved, so a clamped move at the end leaves no debt).
- The other paths were checked and hold: the history's wheel (`scroll_frac`), the child's wheel (`child_frac`), the scrollbar drag (absolute `y`) and the selection drag (3 px threshold); the missing scrollbar in the workspace list is not a defect and became `B42`.
- Verified with unit tests (a shake does not reorder, half a label moves one, the anchor moves with it, the end leaves no debt) and an Xvfb probe with three tabs: one `click 4` from tab 1 lands on tab 3 (one detent, not two), a 2 px drag leaves the order alone, and half a label plus 20 more 1 px motions ends in slot 2 — the old binary ends in slot 3.

## 2026-11-08 — `M-P`: the reflow carries the selection

- `resize` bumped `abs_gen` and every anchor died, so a resize in the middle of a copy lost the selection — the Python's behaviour, and what `M-P` was about.
- `Term::resize_keep` takes the span by parameter (the core keeps no selection) and returns it remapped: the reflow records `(paragraph, cell offset)` of the points before the rewrap and `(row, col)` after it, the arithmetic the cursor already did; a row the ceiling ate returns `None`, and `apply_size` passes `tab.sel`.
- Verified with three new `term.rs` tests (carried, spans a wrap, dies at the ceiling) and `quality-check.sh --fast` green.

## 2026-11-08 — `DECKPAM`/`DECKPNM`: the keypad application mode

- The parser took `ESC =`/`ESC >` and threw them away, so a keypad key always sent `7` or `+` and a program could not tell `KP_7` from the digit nor `KP_Enter` from `Return`; `Term::keypad_app` is that mode now (cleared by `DECSTR`/`RIS`, answered by `DECRQM` as 66) and `keys::encode` sent the VT100 `\EO` table (`KP_7` → `\x1bOw`), since winit hands over only the character.
- Same-day correction: the `\EO` table fired for **any** keypad key and the programs asking for the mode cannot read it (`xterm-256color` declares no `k0`..`k9`: `zsh` beeped and `vim` turned `ESC O w` into `7`), and letting a navigation key win was still wrong with NumLock on.
- What settled it was measuring the system terminal (ghostty 1.3.1 in the same `cat -v` scene under `DECKPAM`): NumLock off → `^[OH`/`^[OD` (the navigation key, application cursor form), NumLock on → `7`, `4`, `+`.
- So the table went out: `DECKPAM` is state only and the keypad sends the digit or the navigation key, which is what every program reads. Verified through the socket with `zsh -i` on Xvfb (NumLock on types `hello781+-`, NumLock off moves the cursor with keypad Home/End) and with `vim` moving with keypad 4 and 7.

## 2026-11-08 — `B42`: the workspace list scrolls

- The cards were drawn from the top at a fixed 2.4 rows and nothing moved them: a workspace past the bottom of the window was reachable only by `Ctrl+Shift+1..9`, and the wheel over the column fell through to the grid's history.
- `App::card_off` is the first card drawn; the wheel walks it with the same hatch as the bar and the history (a detent is one card, a touchpad fraction accumulates in `card_frac`), and `card_at` adds the offset so a click lands on the card the eye sees.
- `sidebar` cuts the list at the bottom of the grid and draws a text row only when the whole cell fits; `reveal_card` brings the card entering from the keyboard into sight and `clamp_cards` keeps the offset inside the list — no track is drawn, since the item's cost was the offset, not a scrollbar.
- Verified with 8 workspaces in a 5-row window through the socket (`cards_off`, `cards_visible`): 7 × `Ctrl+Shift+N` leaves `off` at 6 with `ws` 7, six wheel-downs hold at 6, a click on the top visible card picks workspace 7 and `Ctrl+Shift+1` takes `off` back to 0, with no half glyph at the cut.

## 2026-11-08 — `M-I`: the workspaces and tabs come back

- Closing the window cost the layout — every run started with one tab — and the file that remembered the geometry (`J2`–`J6`) already existed.
- `state::State` now carries `geometry` and an optional `layout`: one entry per workspace with its name and the cwd of each tab, plus the workspace and tab in front; a file without a layout (the old format) still opens, with one tab.
- `session_layout` writes it on the way out and `restore_layout` rebuilds it before the first frame, in the order the `parked` invariant wants; a directory that is gone opens on the shell's default, and empty workspaces or out-of-range indices in a hand-edited file are dropped. `--cmd` and `--cols/--rows` do not restore: a probe and a measurement want a clean terminal.
- Verified with `the_layout_survives_a_round_trip_and_loses_only_what_it_cannot_open`, the extended `window_state` check and three probes: two workspaces with two tabs come back as such with `ws=1`, a layout pointing at `/nao/existe` opens both, and the restored tab really starts in the saved `/tmp/mi-cwd`.

## 2026-11-09 — the reader stays on its text inside a `?2026` window (`B53`)

- With `pi` writing in the tab, scrolling back made the text slide: every line the program wrote moved it one row, though the offset on screen did not change.
- Measured from the owner's `--debug-log`: 574 of 691 windows closed with the reader scrolled back (up to 78 rows), **none** moved the view, and 104 lines went into the history under it; all 53 windows that pushed lines carried `el` and no `ed2`/`ed3`/`home`, so they were appends.
- The cause was the `B41` rule: any scroll inside a `?2026` window counted as redrawing (`scrolled_sync`), so `view_after_output` computed no output and left the offset frozen while the buffer grew — for `vim` a window is a frame of what is on screen, for `pi` it wraps new lines.
- The discriminator is now the window's content, `SyncTrace::repaints`: `ED 2`, `ED 3`, cursor home or `RIS` mean the screen is being drawn again and only those scrolls are ignored; `el` is not a mark, since a line-by-line append clears lines too and that is the shape the creep came in. Before, 25 rows pushed moved the view 0; after, 25.

## 2026-11-09 — a workspace card is dragged to its place (`B49`)

- The order of the workspaces was the order of creation: clicking a card changed *which* one was in front, never *where* it sat, and since the layout is saved as a list (`B44`) the wrong order came back on the next open.
- The gesture is the tab bar's (`I10`): press, 3 px of travel before anything moves (what keeps the rename's double click from being a drag), and the card under the pointer is the place the dragged one takes, so the list moves as the hand passes.
- The front is a storage detail (`tabs`/`active`), not a place in the order: the list is gathered in logical order, moved as one vector by `reorder` and scattered back, so `parked.len() + 1 == ws_count` holds and the number renumbers itself.
- Measured on Xvfb with three workspaces restored from a `window.json` of `/tmp/wsa,/tmp/wsb,/tmp/wsc`: a plain click left the saved order untouched, a drag of the first card onto the last saved `wsa, wsc, wsb` and of the last onto the first `wsc, wsa, wsb`, with the dragged one in front because the press selects the card.

## 2026-11-09 — a new workspace leaves the others in place (`B54`)

- `Super+N` (and `Ctrl+Shift+N`) sometimes looked like it shuffled the workspaces, and it did: the workspace being left was pushed to the **end** of the parked list instead of the place it occupied, so every workspace after it walked one step left.
- Measured on the installed `1.0.53`: `/tmp`, `/var`, `/usr`, the middle one brought to the front and then a new workspace saved the order `/tmp, /usr, /var, /var`; after the fix, `/tmp, /var, /usr, /var`.
- The fix is `park_front`: the front goes back into `parked` at `ws_idx` (its logical slot) and the answer it returns is the new front's index, still the last one because the new workspace is appended; the helper is a free function with a unit test over names (`0,1,2` with `1` in front), because the trap is the index arithmetic, not the spawn.

## 2026-11-09 — the block selection (`B5`)

- `ls -l` and a log table have their columns in the middle of the line and a drag between two anchors takes every character between them; **Alt+drag** is the rectangle now.
- The shape travels in `Span` (`Shape::Flow` | `Shape::Block`): a block keeps the two corners' columns unsorted and sorts only the lines, `visible` returns a `Rect` with the flag, and `is_swap` cuts a block by **columns on every row** where a flow selection cuts only its first and last.
- `span_text` has a block half — the same column range per row, one line each, no soft-wrap joining (the wrap continues on screen, which is where the block reads it) — and trailing blanks are dropped per line; the app decides at the press (Alt sets `tab.sel_block` for the whole drag) and a block has no word or line, so the click count is not consulted.
- Verified through the real mouse on Xvfb: the same drag over `aaa bbb ccc / ddd eee fff / ggg hhh iii` from col 4 to 6 copies `"eee fff\nggg hhh iii\njjj kkk"` without Alt and `"eee\nhhh\nkkk"` with it (bottom-up with Alt: `"d eee\ng hhh\nj kkk"`), and a `sel` verb on the socket returns the text and the shape.
- Limitations recorded: across a resize the rectangle is carried by its two corners, and with the child asking for the mouse the drag is reported to it.

## 2026-11-09 — the layout is taken before the farewell (`B51`)

- With `restore_state = content` the content came back and every tab came back in the wrong directory — reproduced, not guessed: `cd /tmp` + `quit` wrote `{"cwds":[null]}` with `content` and `{"cwds":["/tmp"]}` with `layout`.
- **Cause**: the farewell of 1.0.47 hangs every pty up **before** `save_geometry`, and a tab's directory is `/proc/<pid>/cwd` (with `OSC 7` only as fallback); by the time the layout was written the children were gone, `prefer_cwd` got `None`, and `null` in the file is as legitimate as a real path, so nothing complained. Only `content` suffered, because `last_words` does not run when nothing is written.
- **Fix**: `exiting` reads the layout first and `save_geometry` now **requires** it as an argument, so the order is the compiler's business instead of a comment's; no caching, since a cached cwd would be a second truth with its own staleness.
- Pinned by a test with a real pty (`/proc` answers while the child lives and stops answering after `hangup`), because an `App` cannot be built without a window; on the debug build, two tabs in `/tmp` and `/usr` were saved as `["/tmp","/usr"]` and the reopened session put the front tab back in `/usr` above the restored content.

## 2026-11-09 — a file dropped on the window (`B50`)

- Dragging a file from the file manager onto the terminal did nothing; now the **path** goes into the child, quoted for the shell and written like a paste (bracketed when the child asked for `?2004`), which is what dropping a file on a terminal means — and the terminal does not guess further: no new tab, no `cd`, because it does not know what is running there.
- `shell_quoted`: a path made of characters the shell does not touch goes out as it is, anything else is wrapped in single quotes with the embedded `'` closed and escaped (`'\''`) — the one form that survives `$`, spaces, quotes and newlines.
- The XDND event is one arm (`WindowEvent::DroppedFile` → `drop_path`), which `winit` 0.30.13 already speaks on X11, Windows and macOS, and the socket grew a `drop` verb that calls the same `drop_path`, so the path is verifiable without a drag source: `{"cmd":"drop","path":"/tmp/a file $HOME's.txt"}` wrote `'/tmp/a file $HOME'\''s.txt'` and `printf` printed the file back byte for byte.
- Honest gap: the winit event arm itself has no automated test — it cannot be constructed from a test and Xvfb has no drag source; everything below it does.

## 2026-11-09 — the marker is decoration, not content (`B48`)

- `── content restored from the previous run ──` is fed into the grid so the owner sees where the past ends, and it was then carried by the snapshot: every restart saved the previous marker, so a tab reopened five times came back with five markers and no content (six tabs of 963–1012 bytes holding only prompts and stacked markers in the owner's own state).
- `Row` gains `chrome`: a row the terminal itself wrote to say something about the session, drawn like any other and skipped by `snapshot`, so the photograph leaves with zero markers and the screen keeps one.
- `Term::feed_chrome` sets the mark while a feed runs; the first attempt missed because the ASCII fast path (`put_ascii_run`) writes the row without passing through `put_char`, so the mark is set there too — once per run, not per char.
- A mark is a property of the **row**, not a position: it survives the scrollback and the reflow (a row index or a hash of the text would not, since a resize rewraps the marker) and is carried by paragraph in `reflow`. Verified over three real restarts with `restore_state = content`: `content restored` count **0** every time, the content still there and one row longer per run.

## 2026-11-09 — one owner of the state (`B46`)

- Two windows on the same `window.json` restored the same tabs and, on the way out, the last to close decided the next run — the session that was open lost its layout, and the owner's own debug runs did exactly that.
- `state::lock` takes a **non-blocking `flock`** on `state.lock` beside the state file and returns an `Owner` held for the app's life: the owner is asked of the kernel, so there is no stale lock and no recycled PID to defend, and the lock being a property of the *file* means an instance pointed at another state (`XDG_STATE_HOME` of its own) never disputes this one.
- The owner restores layout and content, applies the remembered geometry and is the **only** writer (`save_geometry` starts with `owner.is_none() → return`); a window that finds it taken is a **scratch** one — one tab in the directory it was called from, 980×640 instead of the remembered rectangle, no restore and no write.
- `restore_state = none`, `--cmd`, `--cols` and `--rows` do not even try the lock, and `tools/devrun.sh` exports `XDG_STATE_HOME` to `${TORTTY_RS_STATE:-<pidfile minus .pid>-state}` so the window under test has its own state.
- Verified with two instances on one state dir (2 workspaces, `ws = 1`, 700×500): the owner came back 72×26 with both workspaces and wrote them, the scratch came back 101×34 with one and left the file byte for byte; a `restore_state = none` instance beside a default one does not hold the lock.

## 2026-11-09 — the child's last words are in the snapshot

- `restore_state = content` kept the screen but never the last line a program writes on the way out — and the cause was the **order** in `exiting`: `save_geometry` ran *before* the child had been told anything, so the farewell could not be in it, not "sometimes" but always.
- `Pty::hangup` sends `SIGHUP` and returns without waiting (a bare `kill(2)`; `portable-pty`'s `kill` waits 200 ms **per child**, and the app has to be draining).
- `App::last_words` runs first when the window closes: hangup every pty, feed the readers for up to 250 ms (or until every child is gone) and only then write the snapshots; with `layout`/`none` or a `--cmd` probe it does nothing and costs nothing.
- A/B with the installed 1.0.46: a child that `trap 'echo FAREWELL-HUP' HUP` is absent from the old snapshot and present in the new one, and a TUI that leaves the alt screen and prints its farewell there comes back with the line and **without** the alt screen; the real `pi` frame is still captured (3220 bytes), so the window grew by the farewell instead of shrinking.

## 2026-11-09 — `B45`: `restore_state`, and what comes back

- `M-I` gave back the arrangement and nothing else (same tabs, same directories, empty screens) — one of three reasonable answers, so it became a config key with a ladder: `none`, `layout` (the default, then current behaviour) and `content`.
- The snapshot is **ANSI**, written by `tortty_core::snapshot`: text, SGR and `OSC 8` links from the primary screen (a photograph of `vim` is not what anyone wants back) with the alt screen dropped; soft-wrapped rows come back without a hard newline, so reopening in another width reflows by the same code that reflows a live resize; nothing that acts goes in (no image, no `OSC 52`, no notification, no title) and a control character inside a link URI is dropped instead of ending the sequence.
- Each tab's snapshot is its own file (`session-<ws>-<tab>.ans`, tmp + rename), so `window.json` keeps its schema and a failed snapshot cannot take the geometry with it, and the files of closed tabs are dropped on the next save.
- The replay happens **before** the shell exists, so the child's output lands below a marker line; `none` writes no layout, `--cmd` hands the real session back untouched, and the geometry is written whatever the key says.
- Verified with five core tests, the `state` round trip with a closed tab, the config test for the three degrees, and probes: `content` reopens every tab's previous screen with the marker and a live prompt below it, `layout` opens the same tabs empty, `none` opens one tab and a `--cmd` probe leaves everything byte for byte.

## 2026-11-10 — the toast says what it is about

- The notice was one line cut at the grid width, pinned to the bottom-right corner, with everything the owner reads in one glance (what happened, on which line, which key) flattened into one run of text — and two notices in a row overwrote each other.
- Two levels now: a **title** in the foreground colour and a **detail** pulled 22% towards the surface, wrapped at the box margin (a word is not cut while it fits) and capped at four lines; three shades by the palette so a custom theme moves the toast with it and no key was added (`Idx(2)` receipt, `Idx(3)` warning, `Idx(1)` error), an accent bar on the left and the box **centred**, 28 px up from the edge.
- Time follows the shade (3 s receipt, 10 s warning/error, 8 s notification), no key takes it out and a **click** does, with the press going no further (the box is drawn last and above the grid); the renderer reports the rect in window pixels, otherwise the hit test would repeat the geometry that drew the box.
- A different notice **waits in the queue**, equal words only raise the count (`×3`) without restarting the clock (the `B8` (d) trap), and the middle shade gained a source: `--debug-socket` announces the surface it opened.
- A clean reload answers on screen (`Configuration applied`, family and size in force) and a refused one says what it did to the running configuration (`kept the previous one — line N: …`), with the error alone at startup since there is no previous configuration; no rounded corner yet, since a corner needs a mask baked into the atlas (nine patch). Verified on Xvfb: second notice = **0 pixels**, click at the box centre = **0** accent pixels, config error 1028 red and the socket 1028 yellow; six tests.

## 2026-11-10 — the fallback chain was one font short

- `pi` draws its spinner with braille (`U+280B`) and the cell came out blank — no tofu, nothing — while the same `pi` showed it elsewhere; the cell existed, had width 1 and held the character.
- The face table numbered `FACE_FALLBACK = 3` with `FACE_BOLD_ITALIC = 4`, so index 4 was two things at once: `face(4)` answered with the bold-italic and the second entry of the chain — DejaVu Sans, the only font here with braille — was never asked, leaving `glyph_bitmap` to discard the `gid == 0` and draw nothing.
- The styles end at 3 and the chain starts at 4, `is_from_fallback` is a plain comparison again, and the numbering is a compile-time `assert!` so it cannot come back silently.
- Verified with `each_face_index_reaches_its_own_font` and `braille_comes_from_the_chain` (which fails on face 0 with the old numbering) and with the real binary: a child printing `⠋⠙⠹` leaves the characters in the grid and 76 lit pixels in those three cells.

## 2026-11-10 — one fact in one place

- The same absence was told twice: `LIMITATIONS.md` carried the queued activities beside the accepted limits and `FEATURES.md` had a "Not implemented" line naming them again.
- Now `LIMITATIONS.md` keeps what is accepted, `BACKLOG.md` the queue and `FEATURES.md` what exists, with the id staying where it is the border of a real feature.
- Along the way: `B8` had six cells and `B55` no trigger, `M-N` was the named pipe piece of `M-X`, `M-Q`'s trigger pointed at itself through `M-V`, and `LIMITATIONS.md`'s header lost the `L<n>` id that nothing carries.

## 2026-11-08 — `B43`: the card says what is in it

- Every card said `workspace {n}` over `{title} ({k} tabs)`: the first row repeated the word the whole column already is, and the tab counter answered a question nobody asks — what decides a visit is *which* program is there.
- The card is now `N <name>` over the title of the selected tab, and `N <title>` alone when the workspace was never renamed (second row empty, so the 2.4 rows and `B42`'s offset arithmetic did not move).
- `ws_name` is born **empty** and empty means "never renamed": `finish_rename` accepts it instead of refusing the key and clearing the field goes back to the automatic title; `n_tabs` left `WsCard`, and `restore_layout` drops the `workspace {n}` of older runs, with the `dump` header saying `=== workspace N ===` when there is no name.
- Verified with `the_card_never_stands_on_the_number_alone` and `a_legacy_workspace_name_is_dropped_on_restore`, plus a probe with three workspaces (`1 vim ~/.zshrc` / `2 tortty` / `3 tortty`, the double click on the third giving `3 build` and going back to one row when erased) and a state file with `workspace 1`/`build`/`workspace 3` reopening with the two defaults dropped.

## 2026-11-08 — `I8`: the rename answers while the child holds the mouse

- `mc` enables `?1002` and the press on chrome had two implementations: the one used with the mouse reported only switched workspace and tab, so the double click that starts the rename (`I8`) and the drag that reorders tabs (`I10`) did not exist inside `mc`.
- `press_card`/`press_tab` are the one copy now, called by both paths: the sidebar and the bar answer first, with or without the mouse reported.
- Verified with an Xvfb probe with `mc` in front (`mouse_mode` 2): the same double click on the front workspace's card names it, where before the typing went to `mc` and the card kept `workspace 1`.

## 2026-11-08 — `B44`: the restored session keeps the order

- `restore_layout` rebuilt the workspaces with the one in front **last** and left `ws_idx` there — the `parked` invariant, but read as an order it renumbered the session: a front in position 1 reopened in the last one and every other workspace shifted left, so `Ctrl+Shift+1..9`, the card and the `dump` header pointed elsewhere after a restart.
- `restore_order(total, ws)` returns the build order (the others in file order, the front last) **and** the number the front keeps, and `restore_layout` puts that number back in `ws_idx` once the rebuild is done.
- Verified with `a_restored_session_keeps_the_order_of_the_file` (the mapping the cards use reads back `0, 1, 2` for every position of the front) and an Xvfb round trip: three workspaces, the middle one named `MID` and in front, reopened as `ws: 1` and `['', 'MID', '']` — where the old code gave `ws: 2` and `['', '', 'MID']`.

## 2026-11-11 — `pi` image detection is upstream (`B52`)

- `pi` drew no image inside tortty because `pi-tui` picks the image protocol only by terminal name (`TERM_PROGRAM`, `KITTY_WINDOW_ID`, …) and `tortty` is not in the list (`detectCapabilities()` answers `images: null, hyperlinks: false`); our half already worked — the `f=100` path draws and `a=q` is answered.
- Verified end to end with `PI_IMAGE_PROTOCOL=kitty` (`PI_HYPERLINKS=1`): the owner read `brazil_flag.png` in `pi` and the picture drew on screen.
- Closed as an `EXTERNALS.md` row (proof + workaround) plus one line in `FEATURES.md`, no code of ours; if upstream lists the name, the row leaves.

## 2026-11-11 — the keyboard walks and moves workspaces (`B49`)

- `Super+Up/Down` selects the previous/next workspace and `Super+Shift+Up/Down` moves the front one a place up/down the sidebar, using the same `reorder` as the card drag so the front follows itself; walk and carry both clamp at the ends.
- Both live in `shortcut_ws`, before the encoder, so the child never sees the keys — the same stance as `Super+N/Q`, which the WM may also swallow.
- Verified with `a_workspace_walk_clamps_at_both_ends`, the carry riding on `move_workspace`, covered by the `reorder`/`moved_front` tests.

## 2026-11-11 — Alt+drag selects even with the mouse reported

- With `?1000/1002/1003` every drag went to the child, Alt included; now Alt+Left starts a local block selection (`select_input`, the `B5` Alt) and its release ends it, so Alt may go up mid-drag.
- The **press** decides the gesture and a press the child already holds is never stolen (`held`), and the motion only extends a selection `select_input` started (`dragging`).
- Verified with an Xvfb probe: a plain drag delivers `ESC[<0;3;1M ESC[<32;10;2M ESC[<0;10;2m` with `sel` null, while Alt+drag leaves the screen untouched and sets a block `sel` of `pha beta`/`lta epsi`.

## 2026-11-11 — `OSC 52` sets the clipboard (`M-R`, half)

- The parser takes `52;c;text` (CLIPBOARD) and `52;p;text` (PRIMARY); the app drains each tab after the feed, front and parked, and an empty body clears, as in xterm.
- Never the reverse: the `?` query is refused with no reply, and so are non-UTF-8, control characters and bodies cut at `MAX_OSC` (cut base64 still decodes); payloads ride the 4096-byte OSC cap (~3 KB decoded).
- Verified with seven `osc52_*` core tests plus an Xvfb probe where the child's escapes landed on the real selections (`CLIPBOARD: 'hello remote'`, `PRIMARY: 'prim'`, read back with python-xlib).

## 2026-11-12 — kitty `t=s` resolves under `/dev/shm` (image ENOENT)

- `kitten icat` probes `d`/`t`/`s`, we answer `OK` to all three, and it picks shared memory: the payload is a NAME (`icat-XXX`), not a path, and opened as-is it resolved against the app cwd, so every shm transmission answered `Gi=0;ENOENT` with the bytes sitting in `/dev/shm`.
- The core now resolves `t=s` to `/dev/shm/<name>`, a slash in the name is not a name and is refused (`EINVAL:shm name`), and `t=f`/`t=t` are untouched.
- The visible garbage (`^[_Gi=0;ENOENT^[[`, `brazil_flag.pngGi=0;ENOENT` on the prompt line) is zsh eating the protocol reply as input (`ESC _` yanks the last argument, the rest self-inserts): without kitty shell integration no shell consumes these replies, and kitty itself does the same.
- Verified with three new graphics tests and an Xvfb probe: `icat brazil_flag.png` draws the flag (294381 px changed, 85% of the window) and the grid shows no reply.

## 2026-11-12 — `t=t` deletes only under the protocol's rule

- The payload path comes from the child (over `ssh`, from a remote) and `t=t` meant delete-after-read with no questions; per the spec, deletion now needs both a known temp dir (`temp_dir`/`TMPDIR`, `/tmp`, `/dev/shm`) and `tty-graphics-protocol` in the path — anything else is read but kept.
- Canonicalized before deciding, so `..` and symlinks cannot walk the check out of the temp dir, and decided after the read, on a path that exists.
- Verified with four app tests (deletes with the marker, keeps without it, keeps outside temp, keeps through `..`).

## 2026-11-12 — `?6` (DECOM) offsets the absolute moves only

- `cursor_to` adds the region top for whoever asks, which is right for `CUP`/`HVP`/`VPA` and the `DECSTBM` home, but the relative and column moves passed `self.cy`, already absolute: with `?6h` and a region, `CUU 1` walked two rows **down**, `CHA 5` changed the row and the cursor parked near the region bottom.
- Split by kind, measured against `xterm/cursor.c`: `CursorUp`/`CursorDown` stop at the margin but only from inside (a cursor above the region reaches the screen top, one below the last row) and the column moves pass the row through `CursorRow`, which undoes the offset before `CursorSet` puts it back; `?6` also homes the cursor as it flips.
- Verified with two tests, the absolute moves with the offset and the relative ones without it, both with the conditional margin rule; `tortty-java` `B39` keeps the conditional clamp to be ported and homes nothing on the flip.

## 2026-11-12 — `a=q` over a file loads it and answers

- The spec's query action "will try to load the image and respond with either OK or an error", and `graphics.rs` resolved `t=f`/`t=t`/`t=s` only for `a=t`/`a=T`, so a query carrying a name fell through to the capability probe: `OK` for a file never opened, the name neither read nor removed — and `kitten icat` probes exactly that way before a real transmission.
- Now `q` goes through the file branch too (`FileRequest::usage`), and `provide` answers the verdict instead of storing: `OK`, `ENOENT`, or `EINVAL:unsupported transmission` when the bytes are not the size `s=`/`v=`/`f=` claim; a query with no payload at all is still the probe.
- Verified end to end, with the child writing the APC itself: the marked temp file is read and deleted, the unmarked one is read and kept, a missing one answers `ENOENT`; the same probe against `1.0.63` left **both** files, so the leak was real (this `icat` build happens to unlink its own probes, which makes it answer by the spec rather than by that accident).

## 2026-11-12 — a drag walks the workspace column (`B55`)

- A place off the column could not be reached: `card_drag` took the pointer as a
  place only over a card, and the cards fill the column from the top, so the card
  at the end is cut by the bottom edge — there was no empty band to aim at.
- The band is the **last text row of the column**, and a drag resting there walks
  the list by itself: one card per `CARD_WALK` (500 ms), the order untouched,
  because a place is still a card under the pointer. Back up mid-drag is the
  wheel, which already works with the button held.
- Measured on Xvfb with eight workspaces in a six-row window: resting in the
  band, `1.0.65` moved the card **one** slot (`ws` 0→2) and stopped there
  (`cards_off` 1, forever); now `cards_off` walks 0→2→4→6 with `ws` on 0, and
  the drop on the card that came into sight lands the workspace at the end.

## 2026-11-12 — a notification out of sight keeps its content (`B8` a+b)

- The common case — a command in tab 2 while you read tab 1 — ended with a badge
  and no way to learn what it said: `pump` toasted only the tab in front, and
  entering a workspace acknowledged **every** tab of it, against the rule.
- The toast now follows visibility, not arrival: `visit_active` shows the text of
  whatever is pending when the tab comes to the front, and acknowledges it then.
- The card and the label became two signals: a notification out of sight raises
  the **card** of its workspace (`Workspace::card_alert`, `PumpOut::card_alert`)
  and entering clears only that, leaving each tab's badge for its own visit.
- Measured with the new `state` fields `tabs_alert`/`cards_alert`: arriving out of
  sight leaves `[false, true]` with `cards_alert=[true, …]` and no toast;
  leaving and re-entering the workspace gives `cards_alert=[false, false]` with
  the label badge still on; visiting it toasts `chegou` and clears. `1.0.66` on
  the same scene: the toast after the visit is **null** — the content was lost.

## 2026-11-12 — `OSC 9;4` progress on the tab and the card

- The progress form was **swallowed** on purpose (`osc_progress`) because a 1 Hz
  keepalive would pin a toast; the cost of that was a whole class of programs
  reporting nothing. Now `Term.progress` is `(state, pct)` — `0` clears it, `RIS`
  clears it, a digit state we do not know stays swallowed — and the parser gained
  no branch: the shape was already recognised.
- The app draws it as a **2 px bar on the bottom edge** of the tab label and of
  the workspace card, in the toast's three shades (`state` 2 → `Idx(1)`, 4 →
  `Idx(3)`, 1 → `Idx(2)`), the indeterminate one a still band in `Idx(6)` at 35%;
  on an alert tab the bar takes the label's text colour, or the yellow of the
  badge would swallow it. The fill is in pixels, not cells.
- The toast path is untouched, and that is why the limit could go: the bar is a
  state, not an event — a repeater just redraws the same 2 px.
- Measured on Xvfb, 60x9 with two tabs: 37% green up to `x=131` of a 96 px label,
  100% full, error `cd0000`, indeterminate `8bd3d3` over the light label, warning
  `cdcd00`, `0` back to the plain label; on the sidebar, the parked card shows
  green up to its 37% and the front one the dim cyan band. Three core tests and
  one for `progress_ink`.

## 2026-11-12 — iTerm2's inline image (`B3`, the cheap half)

- `imgcat` and everything else that speaks `OSC 1337;File=` drew nothing: the
  escape fell into the catch-all. Now the payload is base64 → the **same**
  `transmit`/`place` the kitty path uses, with `a=T`, `C=1` (the program's own
  newline decides where the text goes on) and the cursor cell as the anchor — so
  the image scrolls and is pruned with the history like any other.
- `width`/`height` are read in the protocol's four forms: bare (cells), `px`,
  `%` of the grid, `auto` (the image's own size in cells). This is where the
  parser had to change: one OSC form carries megabytes, so the 4 KiB cap is
  raised to 16 MiB **only** when the body is known to be `1337;File=` — decided
  at the tenth byte, once per OSC, not per byte.
- `inline=0` — the protocol's *default* — asks the terminal to write the child's
  bytes into the cwd. That it does not do: `Output.refused` carries the reason to
  the app, which toasts `Image refused / inline=0 asks for a file on disk`, the
  only channel there is (the protocol has no error reply).
- Measured on Xvfb with a 40x38 red PNG: `images` 0 → 1, the red at the cursor
  cell (4x2 cells, `ff0000` inside, black beside it), 11704 pixels changed; with
  `inline=0` the count stays 1 and the toast shows the refusal. Four tests: the
  placement, the four unit forms, both refusals.

## 2026-11-12 — `B55`, the walk goes both ways

- The row above the cards walks the list **up** as the last row walks it down —
  symmetric, same 500 ms cadence (crossing from one row to the other keeps the
  timer running, so the reversal is not a wait of a full step).
- Both rows exist only while there is more list on that side; with the offset at
  the top the first row still drops on the first card, and vice versa.
- Measured on Xvfb, 8 workspaces in 6 rows: down `0→6` in 3.0 s, up `6→0` in
  2.5 s, gaps 0.46–0.51 s, order untouched (`ws` 0); the drop after the walk
  lands the workspace at 7, and the row over card 1 still lands at 1.

## 2026-11-12 — `B55`, the strip below the cards is not a card

- The walk was tied to the last text row of the column, which is **on** the last
  card (the cards are drawn from the top and the last one is cut by the edge):
  pushing a drag one millimetre past that card turned the walk on with the hand
  still over a card, so instead of dropping there the list rode to the end at
  500 ms a card — five cards in 2.5 s, measured.
- Now the column draws **whole cards only** and leaves its last row free: the
  strip is a place of its own, `card_at` never names a card there, and only the
  strip walks. The row that used to be the 14th card's is the strip (13 cards in
  a 34-row window), which is the price: one card less in sight.
- The walk up no longer requires the exact first row of the column: the strip
  is everything above the first card, the bar's row included, so a hand that
  drags to the top of the window finds it.
- Measured on Xvfb, 19 workspaces and 34 rows: drag to the 13th card reorders and
  `off` stays 0; into the strip, `off` 0→6 one card per 500 ms and stops at the
  end; the drag back to the top walks 6→0 in 2.5 s; the drop lands the workspace
  where the pointer is.

## 2026-11-12 — `B61`, the geometry is the window's, not the lock's

- One line tied two questions together: `remember_geom = self.owner.is_some()`
  (`B46`). Who may **restore and write the session** is one decision — two
  windows saving one tab list would erase each other's work — and **the size and
  place a window opens at** is another, which every window has. So a second
  instance, which cannot take the `flock`, came up at the 980x640 default.
- Now the geometry is read whenever the command line did not ask for a grid, and
  the session layout is restored only by the owner. The write side does not move:
  `save_geometry` still returns early for a window that owns nothing, so the
  second window reads and never writes.
- `restore_state = none` keeps meaning "this terminal has no state" (no geometry
  either, as documented), and `--cols/--rows` keeps meaning "this exact grid".
  `--cmd` gets the remembered geometry but no layout.
- Measured on Xvfb with `window.json` at 700x500: first window 700x500, second
  window **700x500** (1.0.68: 980x640), the saved workspace name and cwd intact
  after both closed, and the second window's own tab absent from the file. New
  `--selftest` case `geometry_is_not_the_lock` (five combinations).

## 2026-11-12 — `B62`, the workspace list closes around the hole

- `Super+Q` on the workspace in front promoted `parked.remove(0)` and set
  `ws_idx = 0`, so closing the 9th of 19 put you on the **1st**. The tabs never
  did that (`close_tab_at`: `active.min(len - 1)`), and neither should the list.
- The rule is now one named function, `successor_slot(ws_idx, parked)`: the
  logical order is `parked[0..ws_idx]`, the front, `parked[ws_idx..]`, so the
  successor is `parked[ws_idx]` and the predecessor `parked[ws_idx - 1]` when the
  front was the last. `None` still means "the app closes".
- Measured on Xvfb with each workspace carrying its own `OSC 0` title, 19
  workspaces: select `ws9`, `Super+Q` → **`ws10` selected as the new 9** (before:
  `ws=0`); walk to the end (`ws19`), `Super+Q` → **`ws18` selected** (before:
  `ws=0` again). New `--selftest` case `workspace_close_keeps_the_place`, five
  positions including the only-one case.

## 2026-11-12 — the image source crop (`x`/`y`/`w`/`h`)

- The renderer already sampled with a `uv` rectangle, hardcoded to `[0,0,1,1]`:
  the kitty crop is that rectangle as a fraction of the picture, so the whole
  feature is the four keys plus `Placement::uv(source)`, which the core can test
  without a GPU.
- The cells follow the crop when `c`/`r` are absent (a 20x20 crop of a 40x20
  image is 3 cells, not 5), and a rectangle that leaves the picture is refused
  with `EINVAL:crop outside the image` — clamping would hide the caller's
  geometry mistake. The bytes are still stored: the transmission happened.
- Measured on Xvfb with a 40x20 PNG, red on the left, blue on the right: whole
  picture → `ff0000` at the left, `0000ff` at the right; `x=20,w=20` → `0000ff`
  across the whole 3-cell box; `x=30,w=20` → no placement at all. Two core tests
  (the fraction and the four refusals).

## 2026-11-12 — `bold_as_bright`, parity with the Python version

- The key existed only in `tortty-python`: `SGR 1` over `color0..7` drawing the
  bright variant (`8..15`), which is xterm's `boldColors` and what most themes
  assume. Rust and Java refused the key as unknown, so a shared config file lost
  a line on each.
- The rule is in the core (`color::bright`), one function, tested on the ends:
  `Idx(0..7)` gains eight, everything else is itself. Default **off**, as in
  Python: the palette is the owner's, and a theme that gives the low half its own
  meaning keeps it.
- The renderer applies it to the glyph colour of a bold cell, and reverse video
  is left out of it on purpose — there the glyph colour is the background.
- Measured on Xvfb with `\e[1;31mbold\e[0m \e[31mplain\e[0m`: `off` → only
  `cd0000` in the histogram; `on` → `ff0000` for the bold half and `cd0000` still
  for the plain one. `CONFIG.md` gained the key and `SIBLINGS.md` moved it out of
  the Python-only column (32 keys in all three, 8 divergences).

## 2026-11-12 — `B8(c)`, the bell instead of a painted label

- The badge was the whole label painted yellow (`Idx(3)`), which shouted louder
  than the news and cost the tab its colours. It is now the **bell** `U+1F514` in
  front of the label, in two reserved cells; the label keeps its own background.
- The bell comes from the RGBA atlas when the font has a colour emoji and from
  the mask atlas (tinted with the label colour) when it does not — one code path,
  because the sink decides: the frame carries the grid's colour glyphs in one
  span and the chrome's in another, and the chrome's is drawn **after** the
  chrome, which is where a bell over a label has to be.
- `tab_at` grew the same two cells the renderer reserves, so a click on an alert
  tab still lands on that tab.
- Measured on Xvfb, two tabs, `OSC 9` from the one in the back: before (1.0.69)
  the whole ` 2 two ` label in `cdcd00`; after, the label dark with a gold bell
  in front of it. `render-check.sh` green.

## 2026-11-12 — `B57`, the toast gets its corner (and a shadow)

- The box was a hard rectangle and the pipeline paints everything through one
  texture sample (`color.a * atlas.r`), so a corner could not come from geometry
  alone. The way out without a second pipeline: **one** anti-aliased quarter disc
  (8×8, 4×4 samples per pixel) baked into the mask atlas once, and a nine-patch
  that samples it with `u`/`v` mirrored — one tile rounds every corner of every
  box, and the atlas never grows with the toast's width.
- The border became the ring between two rounded rects (it was four 1 px lines),
  and the shadow two stacked veils, the lighter one a step further out: nested
  rects cannot be soft, so the softness is the sum of two edges.
- Measured on Xvfb, same notice in both builds, accent-colour pixels at the box
  bounds: **1.0.69** `cdcd00` at all four corners; **now** `000000` at the three
  corners and `b3b300` six pixels in (the anti-aliased arc). `render-check.sh`
  green.

## 2026-11-13 — the flow selection paints from its own first column

- Reported by the owner: press at (line 20, column 30), drag to a lower line at
  column 5 — the copy was right, but the highlight on line 20 walked from column
  30 towards column 5 as the pointer moved.
- `Span::visible` took the start column as `min(a.col, b.col)`, which is the rule
  of a **block** (a rectangle is dragged from any corner) and not of a flow: the
  first row starts where the anchor is. `span_text` reads the two anchors
  directly, which is why only the paint was wrong.
- Now the block keeps the low/high pair and the flow takes `a.col` on its first
  row (`0` when that row is above the view) and `b.col` on its last one.
- Verified with `a_flow_selection_dragged_backwards_still_starts_at_the_anchor`
  (copy and rect, plus the block case of the same drag) and an Xvfb probe of the
  exact report: 1.0.70 painted row 2 from column 5, now columns 5/10/29 stay
  `000000` and 30.. the selection colour, rows 3 and 4 unchanged.

## 2026-11-13 — the chrome is one scene (`M-Q`)

- The bar, the cards and the toast were each written twice — the drawing in
  `render.rs` and the click test in `app.rs`, with the same coordinates in two
  formulas — and three bugs in one session were that divergence.
- `crates/app/src/chrome.rs` owns the geometry now: `Metrics` (cell, padding, bar,
  sidebar, grid), `Layout::build` (one slot per tab, and the whole cards that
  fit), and the hit test that converts window px to origin space once. The label
  text and its width — the two cells of the bell included — come from there too.
- `render.rs` draws the slots and `app.rs` asks the same layout; `tab_at`,
  `card_at` and the tab drag became three-line readers, and `Frame` no longer
  carries `cards_visible` (the layout knows).
- Verified with five `chrome` tests (bell width, tab hit by x, whole cards only,
  the walk strip naming no card, the scrolled column) and an Xvfb probe: a click
  16 cells in lands on tab 2 with the bell in front of tab 1, card 2 selects
  workspace 2, and the strip below the last card changes nothing. `render-check`
  green.

## 2026-11-13 — `OSC 133`, the shell says where the prompt is (`M-R`)

- The core kept nothing of `133;A/B/C/D`, so the scrollback was a wall of lines:
  no prompt to jump to, no command range, no status. `crates/core/src/prompt.rs`
  holds the marks — `PromptStart`, `CommandStart`, `OutputStart`, `CommandEnd`
  with the exit status — on **absolute** lines, capped at 512 and pruned when the
  history ceiling eats the line they point at.
- `Ctrl+Shift+Up/Down` walks them: the prompt lands on the first row, and at the
  ends the key does nothing (a key that moves nothing visible reads as a key that
  did nothing). The socket's `state` reports the last eight marks.
- Verified with five `prompt` tests, four `Term` tests (the four letters, the
  status, an unknown letter ignored, `RIS` forgetting them) and an Xvfb probe:
  four prompts at lines 6/11/16/21, `pin` 0 → 6 → 11 up and 11 → 6 → 0 down.

## 2026-11-13 — `OSC 7501`, the program says what it is doing (`B47`)

- The catch-all `_ => {}` threw the whole class away, and with it the one thing a
  terminal cannot infer: `terraform` is *blocked on a permission*, not "something
  happened". `crates/core/src/status.rs` implements v0.4 (2026-10-10) with the
  spec's own hard caps, and every one of them discards the report **before** a
  record is touched.
- Records live on the `Term` (state of the emulation, not of the app): one per
  `id` path, replaced whole by each report, `app` inherited from the nearest
  ancestor, `clear` taking the subtree, 256 of them with the least recently
  updated evicted. `7501;?` answers with the states and kinds accepted, from
  `child_out` like `DA`/`CPR`.
- The presentation reuses what exists: the root record drives the tab's progress
  bar (`working` 1, `blocked` 4, `error` 2), `OSC 9;4` feeds the root until a real
  report arrives, and `done`/`blocked`/`error` raise the badge/card/toast of `B8`;
  `OSC 133;A` and process exit drop `working`/`blocked`, never `done`/`error`.
- Verified with twelve `status` tests (the spec's examples, root inheritance, the
  subtree clear, one per limit, a newline in `msg`, the cap eviction) and an Xvfb
  probe: `working` → record + bar state 1; `blocked kind=auth` → state 4 and the
  tab's badge; `done` → record with the bar cleared; a prompt after `working` →
  the record gone.

## 2026-11-13 — shortcuts are a table, remappable from the config (`M-H`)

- `crates/core/src/keymap.rs`: `Chord` (key + modifiers), `Action` (nineteen,
  two of them taking a number) and `Bindings`, with `map CHORD ACTION [N]` and
  `unmap CHORD` in the config file's own syntax. The defaults live in
  `Bindings::base()` and reproduce every chord the app had, which is what
  `the_default_table_keeps_every_shortcut_the_app_had` proves.
- `App::press_local_keys` lost the four blocks that matched keys by hand
  (`shortcut_ws`, `alt_digit_only`, the `ctrl+shift+c/v` pair and the paging
  `match`) and gained one lookup: a bound chord becomes `run_action`, an unbound
  one reaches the encoder and the program. A digit under `Ctrl`/`Alt`/`Super` is
  taken from the **physical** key, so `Ctrl+Shift+2` still means 2 on ABNT2.
- Verified with an Xvfb probe: `ctrl+shift+t` opens a tab; `map ctrl+shift+y
  new_tab` opens one on the new chord and `unmap ctrl+shift+t` stops the old one;
  two bad `map` lines come out as two config errors with the rest of the file
  holding.

## 2026-11-13 — search the scrollback (`M-D`)

- `crates/core/src/search.rs`: `find(term, query)` walks the history and the grid
  once, in absolute lines (the space the prompt marks already use), returning
  `Hit { line, col, len }`; `Search` holds the query, the matches and the current
  one, and `step` wraps at both ends. Folding is ASCII only, documented: a full
  Unicode fold would change how many cells a match covers.
- The app got a modal bar (`Ctrl+Shift+F`, the new `search` action of the `M-H`
  table): the keyboard belongs to the query, `Backspace` on an empty query closes
  it, `Enter`/`Shift+Enter` walk, `Esc` closes. The view is sent to the match's
  line on the first grid row, so a hit is never half-hidden.
- The renderer keeps a per-cell mark mask (0/1/2) built with the frame: the
  background pass groups runs by mark, so a match overrides the cell's own colour
  (dim amber `#4a3d00`, current `#d7a500`) and the glyph of the current one goes
  dark. The rows are repainted when the mask has anything, because the per-row
  cache was painted without it.
- Verified: five `search` tests (history + grid, ASCII folding, touching matches,
  wrap, keeping the current match across a keystroke) and an Xvfb probe — typing
  `needle` over 40 lines gives 6 hits, `view_off` 37, `Enter` → 32, `Shift+Enter`
  back, `Esc` closes; the capture holds both highlight colours (775 px of
  `#d7a500`, 477 px of `#4a3d00`).

## 2026-11-13 — images: depth and placement id (`M-L`, part)

- `Placement` gained `id` (`p=`) and `z` (`z=`). A placement id unique per image
  **moves** that placement instead of adding another, and the delete set grew to
  the spec's kinds: `d=i` (image), `d=p` (one placement), `d=P` (every placement
  of the image), anything else everything.
- The image pass is one buffer in two spans: the placements with `z < 0` are
  sorted first and their span is drawn **before** the grid's first draw, so a
  picture can sit behind the text. `img_draw` is the span's one draw call per
  image (each has its own texture).
- Verified: a `graphics` test (`p=3` twice moves, `p=4` is a second placement,
  `d=p` and `d=P` keep the image) and an Xvfb probe placing a red 2-cell image
  over the text "AB" — `z=1` red 361 px, glyph 0 (the image hides the text);
  `z=-1` red 279 px, glyph 17 (the text is on top). The Unicode placeholder
  (`U=1`) stays in the queue, and `M-L` with it.

## 2026-11-13 — the search bar became two chips (`M-D`, visual)

- The first bar was one translucent strip with `find: query  1/3` inside it: it
  read as part of the grid, and the count disappeared into the same run of text.
  Now it is `[find: query_ ]` and `[ 1/3 ]` — the brackets are the text, as the
  owner sketched them, so the shape holds at any cell width, and each chip is
  exactly as wide as its text.
- The surface is the grid's background mixed 14 % towards the text colour, drawn
  opaque with a 1 px edge: nothing behind it shows through. `[ no match ]` comes
  in red; the chip is not drawn while the query is empty, and the cursor is the
  amber of the current match. A query longer than the room keeps its tail.

## 2026-11-13 — the search query takes spaces (`M-D`)

- The space bar arrives as a named key, never as text, so the bar's catch-all
  swallowed it: `error: ` and `foo bar` could not be typed at all. In the bar a
  space is now a character like any other; a chord that starts with Ctrl/Alt/Super
  still goes to the child.
- `find` now stops at the last non-blank cell of a line. The comment already said
  trailing blanks are not text, but nothing implemented it, so a bare space
  matched the padding to the right of every line — 94 hits where 4 lines had it.
  A space now finds the gaps inside a line, and no match starts in padding.

## 2026-11-13 — image animation (`B2`)

- The core learned `a=f` (a rectangle of a frame: where it goes, its blend mode,
  the canvas colour, the frame it is drawn over, the frame it edits, its gap) and
  `a=a` (stop, run, loading, jump, set a gap, loop count). It stores recipes, not
  pictures: composing is pixel work and the PNG decoder lives in the app, so the
  core still has no clock and no decoder — `Graphics::advance` is what the app's
  clock calls, and `gap_ms` is what it schedules.
- The app gained the composition (`compose`/`paint`/`blit`: base frame, `Y=`
  colour, `X=1` replace or source-over, clipped to the image), a texture cache
  keyed by `(version, frame)` and a per-tab clock in `about_to_wait` — one wake-up
  per frame, and only for the tab on screen.
- Verified on Xvfb: red/green/blue frames of 150 ms each came back as `ff0000`,
  `00ff00`, `0000ff` in turn at the image's own cells, and `s=1` froze it on one
  frame and one colour. `a=c` and `r=1` answer with a reason instead of nothing.

## 2026-11-13 — the grid's context menu (`M-V`)

- Right button on the grid: Copy, Paste, Open link, New tab. The geometry is the
  `M-Q` chrome's — `chrome::Menu` hangs the box from the pointer's cell and flips
  it up when the bottom would cut it, and the same numbers are used to draw and to
  hit-test, so the two cannot disagree.
- What is usable is decided when it opens and not per frame: Copy needs a
  selection, Open link needs an `OSC 8` target under the pointer. Unusable rows are
  dim and do nothing. Arrows walk the usable rows, Enter runs one, Escape and any
  other key close it — a menu that swallows the keyboard would be a dialog.
- With a child asking for the mouse the right button stays the child's; Shift
  asks for the menu. Verified on Xvfb: the box opens at 106x75 px (11 cells by 4
  rows), clicking its 4th row opened a second tab, Escape put back exactly the
  same pixels, and the label of the widest row was being cut by a rounded-down
  width limit (`Open lin`) until the cell-to-pixel remainder got its pixel.

## 2026-11-13 — Unicode placeholders and combining marks (`M-L`, `M-U`)

- The parser learned to compose: a zero-width character no longer takes a cell,
  it lands on the character before the cursor (up to three per cell) and is drawn
  over it at the font's own bearing. The marks live beside the cells
  (`Row::marks`) and not inside `Cell`, so the whole scrollback does not pay for a
  feature almost no row uses. Writing a character clears the marks of its cell,
  in the slow path and in the ASCII run fast path — the fast path is what writes
  an ordinary `x`, and it was the one that kept a stale accent.
- With that substrate, `U+10EEEE` cells work: `a=p,U=1,i=…,c=…,r=…` creates a
  **virtual** placement (no position, no cursor move), the cell's foreground
  colour carries the image id, its underline colour the placement id, the
  diacritics the row and column, and `placeholder::decode` applies the spec's three
  inheritance rules from the cell to the left. The renderer walks the visible rows
  once and draws each placeholder cell's slice of the picture — so an image rides
  along with text a program moves.
- Verified on Xvfb with a 2x2 image in four colours and a 2x2 grid of
  placeholders: the four cells came out red, green, blue and yellow. The
  diacritic table is the protocol's own list (297 entries), not a formula.

## 2026-11-13 — Sixel (`B3`)

- `ESC P <p1>;<p2>;<p3> q … ST` now becomes a picture. The decoder
  (`crates/core/src/sixel.rs`) is pure arithmetic — palette, `!n` runs, `$`, `-`,
  and both colour spaces (`;2` percentages, `;1` HLS) — so it has no dependency,
  no clock and no allocation beyond the raster: `size` walks the stream to answer
  the geometry the core needs to place the picture, and `decode` paints once.
- The body is kept as it arrived and decoded in the app, exactly like a PNG:
  `Format::Sixel` and `Format::SixelAlpha` (the transparent background of `P2=1`)
  go through `decode_pixels`, with the theme's background passed down for the
  opaque case. That is why nothing in the renderer's placement, crop, depth or
  pruning code had to learn a new trick.
- `DA1` gained the `4`: a producer checks that before it sends sixel at all, so
  without it the feature would exist and never be used. The cursor stays put (a
  sixel is drawn *over* the cell) — the child's newline is what carries text on.
- Bounded on both ends: the DCS body shares the image cap, a raster above 8.5 M px
  is refused with a notice, and because sixel has no id and no `d=`, the oldest
  of the kept pictures goes once there are more than 64 — with its placements, so
  no orphan is left.
- Verified on Xvfb with our own streams: a 40×36 red picture at a known cell
  (`48..95 × 56..92`, 1596 px, the cell geometry exactly); a 64×64 four-quadrant
  stream with per-band palettes and RLE came out red/green/blue/yellow in the four
  quadrants; `P2=1` left the glyph strokes of "ABAB…" visible under the picture
  where `P2=0` covered them. `chafa`/`lsix` are not installed here, so the
  real-producer pass is the owner's to run.

## 2026-11-13 — Sixel leaves the cursor under the picture (`B3`, 1.0.84)

- Reported against 1.0.83: `chafa -f sixel` drew the picture and the prompt never
  appeared — the image started on the prompt line and ran past the bottom of the
  window. The DEC VT340 leaves the cursor in place after a sixel; `chafa` sends no
  newline of its own, so "in place" means the shell prints over the picture. The
  cursor now ends below it (`Output.lines` + `move_cursor` on the same path the
  kitty `C=1` uses), which also means the content scrolls up when the room is not
  there — the picture gets the rows it took.
- The `apc_output` tail now clamps those rows to the grid height and returns the
  column to 0; the cursor moving down is what makes `Graphics::scroll` move the
  placement, so a tall picture rides up with the text instead of being pinned.
- Verified on Xvfb: a picture ending past the bottom of a 10-row window now leaves
  `PROMPT-BELOW` on the row below it; a picture drawn mid-screen, pushed out by 30
  lines (`placement.line = -24`) came back intact after `shift+pgup`
  (`view_off = 25`, 3552 red pixels in the same box) — the scrollback carries it.
- `docs/FEATURES.md` says both: the cursor ends below, and a picture taller than
  the whole grid is clipped at the top.

## 2026-11-13 — A picture lives in its cells (`B3`, 1.0.85)

- Reported against 1.0.84: `chafa` drew the flag, then `mc` opened and the flag
  stayed visible in every cell `mc` left blank — `mc` does not take the alternate
  screen, it only clears what it paints. The cause was ours: `ED`/`EL`/`ECH`
  erased the cells and left the placements, so `clear` after a picture also left
  it behind.
- `Graphics::erase()` drops the placements that intersect the erased span, and
  `erase_display`/`erase_line`/`erase_chars` call it with the span they touch. A
  picture in the history is untouched (the screen is what was erased), so
  scrolling up to it still shows it.
- Because that erase would have killed the pictures of the main screen the first
  time `vim`/`less` cleared the alternate one, the alternate screen now has its
  own placements: `Graphics::enter_alt`/`leave_alt` hold the main screen's set and
  give it back on the way out.
- Verified on Xvfb: `clear` after a picture leaves zero placements; `mc` over the
  flag shows a clean screen; `less` shows no picture while it is up, and the
  picture is back — same line, same pixels — after quitting it. Three core tests:
  erase by row/chars, history survives a clear, alt screen isolation.

## 2026-11-14 — `chafa` in the other two formats (`B3`, 1.0.86)

- Reported against 1.0.85: `chafa -f kitty` and `chafa -f iterm` drew nothing —
  sixel and `cat`ing the raw sequence worked, so the picture path was fine; both
  producers were refused by two separate defects.
- **Kitty**: `chafa` base64-encodes each chunk of a transmission **on its own**, so
  the padding sits in the middle of the joined string and the strict decoder
  answered `EINVAL:base64` to every third chunk (137 refusals over one 384×136
  picture, zero images). The chunks are now decoded as they arrive, a padded chunk
  taken whole and an unpadded one handing its leftover characters to the next; the
  base64 engine allows the spare bits of the last symbol to be non-zero, which is
  how `chafa` writes them (proven on a real capture: 408 chunks, 208 896 bytes,
  exactly `384×136×4`).
- **iTerm2**: `chafa -f iterm` sends an **uncompressed TIFF** (`II*\0`, pixels
  first, IFD at `0x1808`), not the PNG `imgcat` sends, and the PNG reader refused
  it as "not the size it claims". New `crates/core/src/tiff.rs` reads what that
  writer produces — strips, eight bits per sample, no compression, either byte
  order — and hands the pixels to the existing `f=32` RGBA path, so placement,
  crop, `z=` and the rest did not learn a new format. A compressed TIFF is refused
  with a reason instead of guessed at.
- The iTerm2 cursor no longer stays inside the picture: `inline=1` takes the cells
  it covers, the same rule sixel got in 1.0.84 — `chafa` sends one `\r\n` and
  expects the terminal to have moved.
- Verified on Xvfb with the owner's own `brazil_flag.png`: both formats draw
  (1 image, 1 placement, 23×8 cells) and the prompt lands under the picture.
  `chafa` never reads its own stdin (`strace`: zero `read(0)`), so the correct
  `Gi=…;OK` reply is echoed by the shell — its bug, now in `docs/EXTERNALS.md`.

## 2026-11-14 — the alternate screen keeps the history (`B63`, 1.0.87)

- Reported: after running `mc` the scrollback that had gone off screen was gone.
  Measured on 1.0.86 — 40 lines in a 12-row grid leave `scrollback 29`, starting
  `mc` gives `0`, quitting gives `1`; `less` does the same.
- Cause: `CSI ? 1049 h` arrived at `enter_alt()` as the *clear the history* flag.
  `1049` asks for the **alternate** screen to be cleared, not the main one's saved
  lines; and nothing refilled the hole, since alt-screen scrolling already refuses
  to enter the history (`scroll_up`: `keep = full && alt.is_none()`).
- `enter_alt()` stopped clearing it, and `ED 3` in the alternate screen became a
  no-op for the history and for the pictures under the app (it used to wipe both
  through `graphics.clear()`).
- Proof: core test `the_alternate_screen_keeps_the_history`; probe on Xvfb with
  `mc` (29 → 30 → 31, never 0) and `less` (32 in, 32 out), plus `?1049h` + `ED 3`
  changing nothing (33 → 33).

## 2026-11-14 — the shortcuts are `map` lines in the defaults (`M-H`, 1.0.88)

- The default chord table was the one family of defaults still hardcoded
  (`keymap::Bindings::base()`), duplicated as a comment in `DEFAULT_CONF` with
  nothing keeping the two in sync.
- The table now *is* the defaults text: 50 `map` lines in `DEFAULT_CONF`, an empty
  `Bindings` in the fallback, and `Bindings::base()` is gone. `Config::base()` is
  again "the parse of `DEFAULT_CONF`", as `config.rs` has always claimed.
- Deleting a default line frees the key, like `unmap`; the app's table and a user's
  copy of it are the same syntax. Proof: the keymap test `the_default_table_keeps
  …` reads `Config::base().bindings`, so a dropped chord fails the suite.
- Probe (Xvfb): plain → `Ctrl+Shift+T` opens a tab (2 tabs) and `Alt+2`/`Alt+1`
  walk them; `map ctrl+shift+t new_workspace` → 2 workspaces, no tab; `unmap` →
  the chord does nothing.

## 2026-11-14 — the defaults are a file (`1.0.90`)

- `DEFAULT_CONF` was a 131-line, 4 431-byte string literal inside `config.rs`,
  the one place where git saw configuration as code — `\"pretty theme\"` in the
  middle of the palette comment.
- It is now `crates/core/defaults/tortty.conf`, taken in at build time with
  `include_str!`: still one binary, still no disk read at runtime, still "the
  parse of `DEFAULT_CONF`" as `Config::base()`. Cargo tracks the file, so an edit
  rebuilds.
- Gain that is not cosmetic: the defaults can be pointed at
  (`tortty --config crates/core/defaults/tortty.conf` is a factory reset) and
  copied, which no one could do while they were a string.
- Proof of no drift: `Config::base()` dumped before and after the move is
  identical, line for line (626 lines of `Debug`, the 50 chords included).

## 2026-11-14 — a context menu per place (`B64`, 1.0.91)

- Reported: with the pointer on the sidebar the menu did not come out under the
  mouse. Cause: the box was built from `cell_at`, which clamps to column 0, so a
  click 164 px into the sidebar put the grid's menu at the grid's left edge; and
  `link_at` was read from that same clamped cell, enabling "Open link" for a link
  the pointer was not on.
- The menu became one type with four lists (`chrome::Item`, `GRID_MENU`,
  `CARD_MENU`, `TAB_MENU`, `BAR_MENU`) and two anchors: the cell on the grid, the
  pointer on the sidebar and the bar, then clamped to the window. `MenuState`
  carries only the list, the anchor and the subject, so drawing, hit test,
  keyboard and click are one path each — a new context is an array, not a menu.
- Cards got their own rows (new/rename/move up/move down/close workspace) and a
  tab its two; `Close tab`/`Close workspace` are dim on the last one, because
  closing the window is a keyboard decision (`I20`), and `Move up`/`Move down` are
  dim at the ends. The press decides: the sidebar and the bar are always ours, the
  grid's menu stays the child's when it asked for the mouse.
- Probe on Xvfb: card menu at the pointer with `Move up` dim on the first card;
  `Rename` named three workspaces, `Move down`/`Move up`/`Close workspace` acted;
  tab menu closed a tab (2→1); the strip's single `New tab` opened one (2→3) with
  the box pulled inside the window; the grid's menu still snapped to the cell and
  enabled `Open link` on the line holding the `OSC 8` link.

## 2026-11-14 — the damage counters speak English (1.0.92)

- `Renderer::linhas_remontadas`/`celulas_remontadas` were the last Portuguese
  symbols in `render.rs`, against the convention; they are now `rows_built`/
  `cells_built`, which is also what the socket has been publishing (`"rows_built"`,
  `"cells_built"`) since the counters were added — the field and the JSON key now
  carry the same name.
- Probe (Xvfb, 40×12): the first frame built 12 rows and 480 cells (one screen,
  40 per row); after `seq 200` across 4 draws, 36 rows and 1440 cells, ratio
  exactly `cols` — the counter still counts what it counted.

## 2026-11-14 — a picture cannot claim cells the grid has not (1.0.93)

- Found while measuring `chafa`'s four formats: `chafa --size 200x120 -f kitty`
  on a 60x45 grid left a placement of **200x100 cells at row -2** — rows
  that do not exist — and the first `EL` of the shell's prompt then took the
  whole picture away, because `ED`/`EL`/`ECH` erase a picture whole (`B3`) and
  that phantom rectangle covered every cell there is. The same request through
  `-f sixels` stayed half on screen and through `-f iterm` was refused with a
  toast (chafa switches to a compressed TIFF at that size).
- `Graphics::place` now takes the grid and cuts `cols`/`rows` to it, cutting the
  source rectangle in the same proportion: what a big picture shows is the part
  that fits, in proportion, and the placement claims no cell beyond the screen.
  `handle`, `sixel` and `FileRequest` carry the grid the way `iterm` already did.
  A placeholder (`U=1`) keeps its grid, since its cells are the position.
- Measured: the four `chafa` formats on a normal size are unchanged (59x30,
  60x30, 60x30 cells; same correlations 0.977 / 0.991 / 0.992); the oversized
  kitty placement is now 60x45 at row -2 and **survives** the prompt redraw.

## 2026-11-14 — an emoji sequence is one cell (1.0.94, `B66`)

- The cell held a `char`, and what had no width went to the marks table capped at
  three: `👨‍👩‍👧` took **six columns** and drew three faces, `👍🏽` took four and drew a
  hand **and** a box, `1️⃣` showed `1▯`, `❤️` kept the width of `❤︎`, `🇧🇷` was the
  letters `BR`. Measured in the core and on screen before the change.
- Core: the row's `marks` became `tail` — the codepoints of the cluster beside the
  base, `MAX_TAIL` 16 — and `put_char` first asks `try_cluster`: a combining mark,
  a joiner, a selector, a keycap, a modifier, the base a joiner is waiting for or
  the second half of a flag belong to the cell behind the cursor, take no cell
  and move it nowhere. `emoji_presentation` decides the width and `promote_wide`
  gives the cell the column to its right (`❤️` 2, `❤︎` 1). The ASCII fast path
  gave up the one character after a joiner so `a\u{200D}b` still joins. `text`/
  `text_cols_full` now speak the cluster, so copy keeps the joiners and accents.
- Drawing: `shape::cluster_gid` returns the **one** glyph a face gives the whole
  cluster, and `Fonts::cluster_bitmap` walks the chain for the first face that
  composes it and can rasterize it; `Renderer::cluster_quad` caches by style and
  cluster. Only plain marks are painted over the base now — a joiner, a member or
  a modifier is not a mark, and painting them was the square beside the hand.
- Measured on `1.0.94`, Xvfb, `emoji-check.txt` of the siblings repo: the family
  is Noto's composed icon, the flags are flags, the five hands are five tints,
  the keycaps are keycaps, `VS15`/`VS16` differ, and the five alignment boxes
  `│a😀b│ │🇧🇷x│ │👍🏽│ │你好│ │abc│` line up. Parser: 39.8 → 40.1 MB/s (median of
  three) on the same 1358-line file, so the extra check per run costs nothing.

## 2026-11-14 — emoji and images were one gamma dark (1.0.95)

- Reported as "os emojis parecem escuros": the colour of the heart in the font is
  `(244, 67, 54)` and the screen showed `(231, 14, 9)` — which is exactly that
  colour taken through sRGB → linear, i.e. the decode with no encode back. The
  banana's `(255, 228, 180)` came out as `(255, 197, 115)`, same rule. Not a
  judgement call: the numbers are the definition of the curve.
- Cause: `render.rs` picks a **non-srgb** target on purpose (the cell colours are
  already sRGB and must not be linearized again), while the emoji atlas and the
  image textures are `Rgba8UnormSrgb`, so sampling decodes them and nothing
  encoded them back. The protocol pictures had the same defect — a solid
  `(128, 128, 128)` was drawn as `(55, 55, 55)`.
- Fix: `image.wgsl` gained `encode_srgb` and `fs_img` returns the encoded colour.
  The textures stay sRGB, so minification still filters in linear space; only the
  output is encoded, and blending stays in the same space as the rest of the
  screen.
- Verified: the heart is `(244, 67, 54)` and the banana `(255, 227, 179)` (one
  LSB of rounding), the grin's outline `(66, 43, 13)` is exact, and an iTerm2
  inline image of `(128, 128, 128)` reads back `(128, 128, 128)`.

## 2026-11-15 — the restored screen keeps its clusters (1.0.96)

- Reported: `restore_state content` brought `emoji-check.txt` back with the
  picture from before `B66` — the family as one face, the flag as one letter.
  `B67` in the queue, confirmed and closed in the same session.
- Cause: `snapshot()` writes one character per cell (`out.push(c.ch)`) and the
  `Row.tail` that carries the codepoints of the cluster never reached the bytes.
  Measured in the core: `👨‍👩‍👧` is a 2-column cell with a tail of 4 and its
  snapshot was the three bytes of `👨`; `🇧🇷` → `🇧`; `👍🏽` → `👍`; `❤️` → `❤`,
  the `VS16` gone and the width back to 1.
- Fix: `snapshot()` writes `row.tail_at(x)` after the base character — the same
  order the bytes arrived in, so the restore path composes it again through
  `try_cluster`. A trailing space that carries the rest of a cluster is now
  content, not padding. Test `a_cluster_survives_the_snapshot` covers the five
  shapes of the file (family, flag, skin tone, heart with and without `VS16`),
  comparing base, tail and width, and it fails without the fix.
- Verified end to end (release build, `restore_state content`): `session-0-0.ans`
  carries `1f468 200d 1f469 200d 1f467`, `1f1e7 1f1f7`, `1f44d 1f3fd`,
  `2764 fe0f`, and the line in the restored window is pixel-identical to the one
  before the restart — family icon, flag, tinted thumb, red heart.

## 2026-11-15 — a resize carries the cluster (`B68`, 1.0.97)

- Found while explaining a report ("the flag did not come back"): restoring a
  tab whose snapshot had been written **before** 1.0.96 cannot invent the second
  regional indicator — the byte is not in the file. But a resize of the same
  session produced the same broken picture, and that one is ours.
- Measured on the installed 1.0.96: `cat emoji-check.txt` at 101x34, one
  `resize` to 88x33, and the line on screen became `BR 🇧   PT 🇵 ...`; the
  snapshot saved on exit had `0x1f1e7` and no `0x1f1f7`.
- Cause: `reflow()` flattens `row.cells` into paragraphs and rewraps them; the
  `Row.tail` — the codepoints beside the row, addressed **by column** — was left
  behind, so every cluster fell back to its base. `B66` declared that as a limit;
  it was not one, the table is addressed by column and the cut is by column.
- Fix: the tails travel beside the cells through the flatten, the trimming and
  the chunks, and are re-attached with `add_tail` on the destination column. The
  same resize now keeps `BR 🇧🇷 PT 🇵🇹 ...` on screen and `1f1f7` in the file.
  Test `a_resize_carries_the_cluster` walks 40 → 8 → 40 → 25 → 5 columns and
  compares the clusters in reading order; it fails without the fix.

## 2026-11-15 — the defaults split by system

- One defaults file carried the chords of one system, and chords are not portable: `Ctrl+Shift+C` is not the mac gesture, `Alt+1` under Option types `¡`, `Win+1`…`0` launches the taskbar and `Win` + arrows snap the window. `Super` turned out to be *correct* on macOS (`super` = ⌘) and dead on Windows.
- Split into a common base (`crates/core/defaults/tortty.conf`, only chords that are correct, primary and collision-free on all three) plus `tortty-linux.conf`, `tortty-macos.conf` and `tortty-windows.conf`, parsed on top of it by `Config::base_for(os)`; `Os::current()` is a compile-time choice, so the binary carries one template and nothing is sniffed.
- macOS: `⌘C/V/T/W/N/Q`, `⌘1`…`0` for the tabs, the workspaces on `Ctrl+Shift` (from the base) and the walk on `⌘⌥↑/↓`; Option+digit is left to the child. Windows: no `super` family at all, tabs on `Alt`, walk on `Alt`.
- Three tests: every system has a chord for every action, a template never repeats a base line nor binds a chord twice, and the four files parse clean; the `state` verb reports `config_os`.
- Refused on the way: a `mod` modifier (a name whose key changes per system — and on Linux the "main modifier" is `Ctrl+Shift`, so `mod+c` would take SIGINT) and a per-OS overlay for the user.

## 2026-11-15 — the Windows template measured in a guest

- The template rested on "the shell eats Win+1, Win+arrow and Win+letter"; that was convention, not measurement, so it went to a Windows 10 21H2 guest (build 19044) and was driven headless: QEMU's monitor socket for `sendkey` (a hardware-level keystroke, so the hotkey layer sees it) and `screendump`, a WinForms probe logging both a `WH_KEYBOARD_LL` hook and its own `WM_KEYDOWN`/`WM_SYSKEYDOWN`, and an HTTP server on the host (the guest reaches it at `10.0.2.2`) to hand the script in and post the log out.
- Super: `Win+1`, `Win+↑`, `Win+Shift+S` and `Win+R` were seen by the hook and by **no window at all** — the taskbar, the snap, the snipping overlay and the Run dialog took them, exactly as the template assumed.
- The chords chosen instead do arrive: `Alt+1`, `Alt+↑`, `Alt+Shift+↑`, `Ctrl+Shift+1` and `Ctrl+Shift+T` came in as `WM_SYSKEYDOWN`/`WM_KEYDOWN` in the focused window.
- Font: the default became **`Consolas`** — the same run listed 103 families and no Cascadia Mono (that one belongs to Windows 11 and to Windows Terminal, absent on a plain 10), so the template named a family that could only fall back.
- The macOS template stays unmeasured (`M-Y`); the guest was left as it was, and the probe scripts live beside the VM, outside this repo.

## 2026-11-16 — `font_family` is a list

- The value is now a fallback order — `Cascadia Mono, Consolas` — and the first **installed** name wins, so one template serves Windows 11 (Cascadia ships with Windows Terminal) and the measured Windows 10 21H2, which has neither Cascadia nor anything in the built-in chain.
- The defect behind it: when the configured family was absent the code still asked for bold and italic **by that absent name**, so the text silently lost its weight and slant. The styles now come from the family that was really found.
- A generic name inside the list (`Monospace`, the file default) ends the walk on the preference chain; no comma is the old single name; empty entries are skipped; a list of nothing falls back to the chain.
- Measured end to end on Xvfb: `Tortty Nao Existe, Liberation Mono` lands on Liberation (cell 11.202×21.146) and not on the chain's DejaVu (11.238×21.729).
- The Windows template now says `font_family Cascadia Mono, Consolas`; `CONFIG.md`, `PORTABILITY.md` and the `--font-family` help follow.

## 2026-11-16 — the `platform` seam

- The OS-naming code left `debugsock`, `state`, `pty`, `main`, `app` and `xguard` for `crates/app/src/platform/`: the app now asks for a **capability** (`try_lock`, `hangup`, `shell_quote`, `open_uri`, `child_cwd`, `install_triggers`, `name_window`, `runtime_dir`) and the system is named in one module.
- Measured before: 35 compile errors for `x86_64-pc-windows-gnu` and 4 for `x86_64-apple-darwin`. After: **1 and 1** — the seam's `compile_error!`, which says only Linux is measured and points at `M-X`/`M-Y`.
- The 4 macOS errors are the reason the implementation is named `linux` and not `unix`: `libc::ucred`, `SO_PEERCRED` and `winit::platform::x11` do not exist there. Being a unix buys nothing.
- The ruler is `tools/platform-gate.sh`, a step of `tools/quality-check.sh`: it fails when `std::os::unix`, `libc::`, `windows_sys::`, `winapi::` or `winit::platform::` appears outside the seam (proven by adding a bare `libc::getuid()` to `prof.rs`: the gate named the line).
- Drive-by: the doc comment of `restore_order` had been left orphaned above `shell_quoted` (two doc blocks in a row); it went back onto its function, and the two shell-quoting copies in `app.rs` became one `platform::shell_quote`.

## 2026-11-16 — the tree was cleaned for a public reading

- Audited the repo before opening it: **no credential, key or token was ever committed** (checked by `git log --diff-filter=A`, not only by the tree), so nothing had to be rotated.
- Three **vim swap files were tracked** (`.swm`, `.swn`, `.swo`): their header reads `user`/`host`/pid, the only place the machine name existed, and invisible to `git grep` for being binary. Removed, and `.gitignore` now has `*.sw?` — it had `*.swp`/`*.swo` and never covered `.swm`/`.swn`.
- The username left the code and the docs (`shell_quote` test, the `mc [user@host]` title test, the egui mockups' prompt, four absolute paths), and the private sibling got its public name: `tortty-python`.
- `OUT` (442 KB of stray ImageMagick PostScript) untracked. The `torbie.png` lost its C2PA block (the `caBX` chunk naming the generator model): pixels byte-identical, file 21 KB lighter.
- What removal at HEAD cannot fix is the history, which carries the same names in 98 old versions of this file, in 20 `.swp` blobs and in `.pi/tasks` logs: that is a history rewrite, done in the same session (`git filter-repo`), with the owner's approval.

## 2026-11-16 — CI watches the seam, and the actions are pinned

- A third CI job (`seam`) builds `tortty-core` for `x86_64-pc-windows-gnu` and `x86_64-apple-darwin`, which must pass, and then asserts that the **app** stops on the seam's own `compile_error!` and for no other reason — 35 and 4 errors before the seam, 1 and 1 after. When `M-X`/`M-Y` land, that step becomes a build.
- `tools/platform-gate.sh` gained the second half: it also fails on `cfg(target_os|target_family|target_arch)` outside the seam, because that compiles everywhere and no cross-target build reports it. It bit at once — `clip.rs` branched on `cfg(target_os = "linux")` for the X11 PRIMARY — which is now `platform::has_primary`/`primary_get`/`primary_set`.
- Every action is pinned by commit SHA (`checkout`, `rust-cache`, `upload/download-artifact`, `action-gh-release`), and `dtolnay/rust-toolchain` was replaced by plain `rustup`: one less tag to pin. `ci.yml` got `permissions: contents: read` instead of inheriting the default.

## 2026-11-17 — CI verde, e o relógio de 5m27 para ~3m

Três runs vermelhos seguidos, cada um por um motivo diferente, e a causa comum
era o CI não medir o mesmo compilador que o hook: sem `rust-toolchain.toml`, o
runner instalava `stable` (1.100.0) enquanto o laptop rodava 1.99.1, e a 1.100
trouxe lints novos (`map_unwrap_or` em `render.rs` e `app.rs`,
`needless_range_loop` em `term.rs`). O compilador agora está pinado em
`rust-toolchain.toml` e o CI só faz `rustup show active-toolchain`.

Os testes que mediam a máquina em vez do código caíram junto: o runner não tem
fonte monoespaçada nem CJK (`ci.yml` instala `fonts-dejavu-core`,
`fonts-noto-cjk`, `fonts-noto-color-emoji`), `glyph::font_missing_not_becomes_other`
perguntava `Family::Monospace` ao fontdb — que exige panose "monoespaçada", coisa
que um runner pelado não tem — e agora pergunta a cadeia que o app monta, e
`child_cwd` lia só a folha do `/proc`, o que virou corrida quando o filho morre
entre a listagem e a leitura; agora responde subindo pela cadeia de ancestrais.

Com isso o CI ficou verde (run 44004430763: `check` 5m27, `seam` 1m52) e o
relógio virou o problema. `tools/quality-check.sh` ganhou `--only <steps>` e os
passos (`seam`, `fmt`, `clippy`, `test`, `build`, `selftest`, `parsebench`)
passaram a rodar em três jobs paralelos (`lint`, `test`, `build`) mais o `seam`,
com chave de cache por job e `concurrency` cancelando run superado. Medido num
target vazio: `clippy --release` 46 s, `cargo test` 49 s, `build --release` 60 s —
em série, com duas cópias da árvore de dependências, viravam os 5m27 do CI.

Medido depois, no run 44005324723: o run inteiro caiu de **5m27** para **2m51** —
`lint` 2m46, `build` 2m47, `test` 2m37, `seam` 2m00, os quatro verdes. O relógio
passou a ser o job mais lento, não a soma deles.

Com o cache quente (run seguinte, só docs): **2m12**, e o perfil mudou de lugar —
`lint` 38 s, `test` 51 s, `build` 1m41, `seam` 2m08. No `seam` o log é claro:
`Checking tortty-core … 2.43s` e `1.64s`, contra dois `downloading` do rustup. O
gargalo não era compilar, era baixar ~500 MB de toolchain e de std por run, então
o `~/.rustup` passou a ser cacheado nesse job.
