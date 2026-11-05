# Backlog — new activities, outside parity

The rule is a single one, and it is about **nature**, not taste:

| does Python do it? | where it lives | what it is |
|---|---|---|
| **it does** | [`MIGRATION.md`](MIGRATION.md) | parity: line with `done`/`partial`/`missing`/`no` and the `arquivo:linha` of reference |
| **it does not** | **this file** | new activity, with id `B<numero>`, a trigger and a cost |

A piece of work has **one owner**. An item Python does never appears here; an
item it does not do never appears there. It was the lack of that rule that
filled both documents with the same work on 2026-11-02 (see `HISTORY.md`).

## Queue rules (the same as tortty-python)

- **Id: `B<numero>`, exclusive.** A new item gets the next free number —
  `B1`, `B2`, … — and the number **never** goes back into use, not even after
  the item closes: an id is identity, not position in the queue. Items already
  registered as `M-*` keep the id they have: renumbering does not pay for the
  diff and breaks the references that [`CONFIG.md`](CONFIG.md),
  [`LIMITATIONS.md`](LIMITATIONS.md) and [`MIGRATION.md`](MIGRATION.md) already
  make.
- **Registering is just registering.** The investigation (reading code,
  measuring, reproducing) happens when the item is executed, not when it is
  written down. The note carries the symptom, the trigger and whatever comes for
  free along the way; no complete diagnosis ahead of time.
- **Every item has a trigger.** An improvement without a trigger becomes a wish:
  only what has a sentence of the sort "when X appears" enters the table. An item
  without a trigger is a limitation ([`LIMITATIONS.md`](LIMITATIONS.md)), not a
  queue.
- **When closing an item**: it leaves the table of this file and gets a dated
  line in [`HISTORY.md`](HISTORY.md) with what was verified. A closed item does
  not stay here: the past has a single owner.
- **A bug fixed outside an item** enters the numbered list of `HISTORY.md`; if it
  is an environment/tool trap, it enters [`DEBUG.md`](DEBUG.md).
- **A decision not to do** goes to [`LIMITATIONS.md`](LIMITATIONS.md).

## Open

| id | activity | why | trigger | cost |
|---|---|---|---|---|
| M-Q | **Chrome overlay** — tab bar, sidebar cards and toast become a list of elements (rectangle + text + click area) drawn over the grid, in the same `wgpu` | today every chrome element is written **twice**: the drawing in `render.rs` (`tabbar`/`sidebar`/`toast`) and the click test in `app.rs` (`tab_at`/`card_at`), with the same coordinates in separate formulas. Three bugs in one session were that divergence: wrong tab because `tab_at` subtracted the sidebar, card starting on the wrong line, double click on the card of line 1 becoming a tab click. A `Vec<Elemento>` with a declared hit-test kills the duplication without a toolkit and without a system dependency (the grid stays cell by cell; only the chrome becomes a scene) | when the **4th** chrome element appears — and it already has a name: `M-V`, the context menu, which is precisely the trigger of `M-Q` | low-medium: ~200 lines (element, cell layout, hit-test in reverse z order) + migrating the 3 existing elements and deleting `tab_at`/`card_at`/`bar_drag` in their current form |
| M-V | Context menu (right button on the grid): copy, paste, open link, new tab | Python does not have it; today the right button either becomes a mouse report to the app or does nothing | the first daily use where copying requires a drag or a shortcut | medium: depends on `M-Q` (the menu is the 4th chrome) + integration with `arboard` |
| M-C | OSC 8 hyperlink: store per cell, underline and open with Ctrl+click | links in `dmesg`, compilers and `ls --hyperlink`; today the escape is consumed and discarded | when a log link has to be opened by hand twice | low: one `u32` per cell + hover |
| M-R | Escapes that are discarded today: OSC 52 (remote clipboard), OSC 7 (shell cwd), OSC 133 (prompt marks) | three escapes Python ignores; OSC 7/133 unlock "new tab in the right cwd" and prompt navigation; OSC 52 is what makes a remote `ssh` copy | when `ssh` needs to paste/copy from the remote terminal (OSC 52) | low: there is already an OSC handler with a slot per index |
| M-D | Scrollback search with highlight | today you only find it by scrolling; in a big log it is the function that is missing | the first build log that has to be mined | medium: per-line index or scan on demand |
| M-E | Word/line selection (double/triple click) and rectangular | copying a log path requires a precise drag; block selection is what `ls -l` and a log table ask for | when copying a path by drag annoys twice | low |
| M-F | IME and composition (CJK, compose) | whoever types CJK cannot use it; Python does not have it either | the first user who types CJK | medium: `winit::Ime` + pre-editing in the view |
| M-G | Kitty keyboard: **flag 4 only** (alternate keys) | the rest of the protocol is parity (Python has `kitty_kbd.py`); flag 4 is the only part it does not deliver | when an app asks for `CSI > u` with flag 4 and gets an incomplete response | medium |
| M-H | Config v2: shortcut remapping (the palette is already done: `color0..15`, fg/bg/cursor) | a shortcut is the first thing someone wants to change after the font, and today it is a `match` in the code | when a WM shortcut collides with one of the terminal's | low-medium: parser ready, the action table is missing |
| M-I | Tabs/workspaces persistent across runs | today closing the window costs the layout; the socket's `state` is already JSON | when closing the window costs a day's layout | low |
| M-J | Splits (more than one terminal in the same view) | request recorded in Python's P39; here each split is a view in the same `wgpu` | when two terminals side by side become routine | high |
| M-K | OSC 9/777 notification + urgency | "the build finished" without having to look at the window | when a long build requires watching | low |
| M-L | Images: z-index, placement id, Unicode placeholder, source crop | Python always shows the image on top and with no fine control | after the basic image (F6) is working with `icat` | medium |
| M-M | Visible, draggable scrollbar | today only the wheel and `Shift+Pg` (which already work); the position indicator is missing | when scrolling a big log only with the wheel gets tiring | low: rectangle + state |
| M-N | Socket transport on Windows (named pipe) | `benchrun.py` and automation need to work there too | together with `M-X` (Windows) | low |
| M-P | Remap the selection anchor on reflow | today reflow kills the selection instead of shifting it (Python kills it too). `reflow` already computes `(paragraph, offset)` for each old line, so the new anchor is one calculation | when losing a long selection on a resize annoys | low |
| M-S | Keys Python does not deliver: F13-F24 and the numeric keypad with NumLock off (`KP_*`) | extended keyboards and laptop numeric keypads stay mute | when someone maps F13 or uses `KP_*` in an app | low: table in `keys.rs` |
| M-T | Mouse reporting in pixels (`?1016`) | a graphical app asking for a pixel coordinate gets a cell | when a graphical TUI app asks for `?1016` | low: `cell_at`/`px` already exist |
| M-U | Parser: DECALN (`ESC # 8`) and combining characters (compose into the previous cell) | DECALN is the VT100 test screen; a combining mark occupies its own cell today, and that is what breaks a dead accent in an app that sends `U+0301` separately | the first decomposed accent coming out displaced (or a VT100 screen test) | low for DECALN; medium for the combining mark (touches `wcwidth`/advance) |
| M-AA | **Running confined** (sandbox/container): no `/dev/dri`, read-only `$HOME`, no D-Bus, no compositor | the owner already ran under `sandbox` (bubblewrap) and the window opened **without presenting a frame** (`state` → `skipped (timeout/occluded)`, llvmpipe). Done on 2026-11-02: `state` separates `drawn` from `skips` and the app warns **once** on stderr when the surface refuses the frame (before, the symptom was a "dead window" with no explanation); saving state degrades with a warning; the installer explains a blocked target; the clipboard is created on demand and does not stall the start; and `tools/visual.sh` catches the case from the pixel side (new content that does not change a pixel = it did not present). Missing: **backend failover** (recreate the renderer in GL when no frame presents) — it cannot be written blind, it needs an environment that refuses the surface: here `Xvfb` does not even come up inside `sandbox` (segfault in `InitFonts`) | when someone needs to use the confined terminal for real (the window opens and does not update) | medium: the failover is recreating `Instance`/`Surface`/`Renderer` with a forced backend, and the trigger is `skips > N` with `drawn == 0` |
| M-X | **Windows 10/11 target** (D3D12 + ConPTY) | Python never ran there: it is not parity, it is a new target. Today the pty is POSIX and the debug socket is `UnixListener` | when someone needs to run tortty on a Windows (or the target list comes back into the conversation) | high: ConPTY through `portable-pty`, named pipe (`M-N`), `%APPDATA%`/`%LOCALAPPDATA%`, `$COMSPEC`, Consolas/Cascadia font, Windows runner in CI |
| M-Y | **macOS target** (Metal, arm64 and x86_64) | same case as `M-X`: Python does not run there, and it is precisely the target that the `winit`+`wgpu` route makes cheap | when there is a Mac at hand (a macOS runner in CI is paid by the minute) | medium-high: `~/Library/Application Support`, Menlo/SF Mono, universal binary with `lipo`, macOS runner |
| M-Z | Signing (Windows) and notarization (macOS) | without them SmartScreen and the Gatekeeper ask for manual confirmation on every download; Python has none of this | after `M-X`/`M-Y`: on the first public download | low in code (steps in [`PORTABILITY.md`](PORTABILITY.md)), with a money cost: Apple developer id at US$ 99/year |
| M-AB | **Exercise Linux Wayland** (verification, not implementation) | the features already compile (`winit` wayland + CSD adwaita) and have never run in a compositor, so "it works on Wayland" today is a statement about compilation. Python runs there (GTK), so exercising it is parity that compilation alone does not cover | the first session with a compositor at hand (weston, sway, GNOME) | low: one verification session (CSD, focus, `x/y` that Wayland does not deliver, clipboard through `wl-clipboard`) |
| M-AC | **Per-region drawing** (the real `queue_draw_area`) | Python does it (`view.py:330-364`); per-line damage already applies when building the instances, but the `draw` is full screen because the swapchain does not guarantee the previous frame's content. Closing it requires its own target (scene texture + stripe blit), and the cost does not pay for the current gain | when per-frame drawing starts to bother on a big screen — measure first (`draw` today at ~0.7 ms/call, [`BENCHMARKS`](BENCHMARKS.md)) | medium-high: scene texture the size of the window, recreation on resize, stripe blit and damage in pixel coordinates |
| M-AD | **1 px seam in box drawing** | at a fractional cell width (16.3 px) the horizontal bar of `─ ┬ ┴ ┼` leaves an antialias hole at the seam between cells: it shows in the `mc` frame and in `vim` with `listchars`. It already exists today (it did not come from emoji nor from the ligature); rounding the cell origin reduced it, it did not close it | already happened in the `mc` frame on a fractional cell | low: round to a whole pixel and/or stretch the bar by 1 px, with an A/B through `tools/pixel.py --diff` (the seam closed or not) |
| B1 | **Image by file/temp/SHM** (`t=f`, `t=t`, `t=s` of the kitty protocol) | today the image has to come **embedded** in the escape (base64): a 4K screenshot becomes ~30 MB of base64 crossing the pty. Python also only does embedded | when a big image stalls the pty because of the escape size, or when an app only offers `t=f`/`t=s` (`icat --transfer-mode=file`) | low-medium: open the file/SHM and read; the APC routing and `f=` already exist, the load path and the `t=` in the `a=q` response are missing |
| B2 | **Image animation** (`a=f` frames, `a=a` control) | an animated GIF drawn by `icat` would become video in the terminal; the current model stores one image per id, with no notion of frame nor of clock | the first animated GIF someone wants to see in the terminal | medium: frame list per image, clock in the **app** (the core has no time), discard and pause when the tab is not visible |
| B3 | **Sixel and iTerm2 inline** | they are the protocols the rest of the world speaks: `chafa -f sixel`, `img2sixel`, `lsix`, `imgcat` from iTerm2. Without them, a tool that only knows sixel draws nothing | the first program that only offers sixel (`chafa -f sixel`, `lsix`) | high for sixel (its own palette + RLE and another drawing pipeline); iTerm2 is low-medium (PNG base64 + positioning, almost the whole `f=100` path already exists) |
