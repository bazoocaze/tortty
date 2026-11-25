# Limits

Accepted or unresolved limitations. Planned work lives in `BACKLOG.md`
(`B<n>`). Overcome limitations leave this file and go to `HISTORY.md`
(probably through a `BACKLOG` activity).

## Scope

- **One user, one machine.** No session multiplexing, no server, no
  reconnection, no daemon. Closing the window ends everything.
- **One terminal, not a window environment.** No layout manager; tabs and
  workspaces are reopened by the next run, but the running processes are not — a
  new shell starts in the directory the old one was in. The content of the
  screen comes back only with `restore_state = content`, and it is a picture:
  text, SGR and `OSC 8` links, never the process that wrote them, and it stops
  at the `scrollback` cap.
- **The second window is a scratch.** One tab, nothing of it remembered on the
  way out — so what the second window saw is not in the state, and closing one
  window never edits another's session. It does open at the remembered size and
  place: the geometry is the window's, the session is the owner's.
- **Decoration is not content.** The line that separates a restored picture from
  the new run is marked when it is written and skipped when the snapshot is taken
  (`B48`): it shows on the screen and leaves no trace in the file, which is the
  point — but it also means the picture never records *where* a past run ended, so
  two runs look continuous in the restored content.
- **Images are stored as they arrived, not as they are drawn.** The kitty
  protocol is accepted for what `icat` needs (direct, file, temp and SHM
  transmission plus animation frames), and iTerm2's inline one (`OSC
  1337;File=`, display only — `inline=0` would have the terminal write the
  child's bytes to disk, and that it does not do). Two consequences are
  accepted: the **raw bytes are kept for the life of the image** — a frame is a
  recipe composed into a texture when it is drawn, so an id that is never
  deleted costs its PNG plus its rectangles — and **composition happens at draw
  time**: `a=c` (a rectangle of one frame onto another) and `r=1` (editing the
  root frame, which is the image's own bytes) answer `EINVAL` instead of keeping
  a second copy of what the child already sent.
- **A picture bigger than the grid is cut, and a full-screen one is not stable.**
  A placement that asks for more cells than the screen has is clipped to the grid
  and its source rectangle is cut with it, so what is shown is the part that fits
  — the rest is never drawn, because the cells to draw it on do not exist. And
  the erase stays all or nothing per picture: one cell repainted by the child
  takes the picture whole. `chafa --size 200x120` on a 60x45 grid now shows a
  corner of the image (60x45 cells) and survives the prompt, but a picture that
  reaches the row the prompt is drawn on does not. Sizing to the grid, which
  `chafa` does by itself, is the way to keep a picture.
- **`TERM=xterm-256color`.** We do not invent our own terminfo entry: the goal
  is that `vim`, `mc`, `less` and `dmesg` work with what already exists.

## Platform

- **On Windows the port is minimal: the window, the GPU and ConPTY, nothing else.**
  Measured on 2026-11-22 in the Windows 10 21H2 guest: `--selftest` 33/33, the
  grid drawing the `cmd.exe` prompt through ConPTY, WARP (`Dx12 Microsoft Basic
  Render Driver`) presenting the frame. What it does **not** have, each answered
  by the seam with the reason: the debug socket (a named pipe is the real answer,
  so `--debug-socket` refuses to bind), `SIGHUP` and a program's last words
  (`B48`) — closing the pty is `ClosePseudoConsole`, and a Unix signal has no
  equivalent; the cwd of the program in front of the pty (no `/proc`), so the tab
  title falls back to the program name; `SIGUSR1`/`SIGTERM` triggers; the X11
  window id and `WM_CLASS`. The log stamps `tz +00:00` there: the standard library
  hands out UTC only and the civil offset would cost `windows-sys` for one field.
  `shell_quote` wraps a path for `CreateProcess` but does **not** escape `cmd.exe`
  metacharacters (`%`, `^`, `&`, `|`) — a filename carrying one is out of reach.
  None of this is a regression on Linux; it is the honest size of the port so far.

- **On Windows the executable is console-subsystem, so a console window opens with
  the terminal.** The attribute is chosen at compile time and only one of the two
  answers everything: `windows` hides the console and takes `--selftest`,
  `--version` and the `GPU:` line with it — the very output used to validate the
  port — while `console` (today) leaves a second window that covers part of the
  grid (`start /min "" tortty.exe` minimizes it; the shortcut on a test desktop
  does that). Choosing per-run instead of per-build is the open question, and it is
  in `M-X`.

- **The Windows defaults are measured; the macOS ones are not.** On a Windows 10
  21H2 guest (build 19044) driven headless, the Super chords were seen by a
  low-level keyboard hook and by **no window at all** — `Win+1` went to the
  taskbar, `Win+↑` snapped, `Win+Shift+S` opened the snipping overlay and `Win+R`
  the Run dialog — while `Alt+1`, `Alt+↑`, `Alt+Shift+↑`, `Ctrl+Shift+1` and
  `Ctrl+Shift+T` arrived as `WM_SYSKEYDOWN`/`WM_KEYDOWN` in the focused window.
  That is why the Windows template has no `super` family and why the walk is on
  `Alt`. The same run listed the installed monospaced families — `Consolas`,
  `Courier New`, `Lucida Console` — and **no Cascadia Mono**, which is why the
  template names both (`Cascadia Mono, Consolas`: the value is a fallback list).
  Neither Cascadia nor the preference chain (DejaVu, Liberation, Noto, Menlo) is
  installed on a plain Windows 10, so the one name that always answers there is
  `Consolas`. The macOS template follows the ⌘ conventions and nobody has
  measured them: there is no macOS host yet (`M-Y`).

- **No rasterizer of our own.** Drawing is `wgpu` and nothing else: there is no
  cell-by-cell CPU painter to fall back on, and writing one is an entire
  renderer, not a switch. What `wgpu` offers **is** used — `render.rs` prefers a
  hardware adapter and, when there is none, takes the CPU one it can find
  (llvmpipe/lavapipe) and says so on stderr once, because a VM or a container
  without `/dev/dri` has only that. Whether it **presents** is another matter:
  under an Xvfb of its own llvmpipe draws (`tools/render-check.sh` passes without
  `/dev/dri`), but in the owner's confined session on 2026-11-03 the window
  received no frame — Vulkan blew the present timeout and GL returned success
  without painting a pixel. Whoever runs confined binds `/dev/dri`, which presents
  for real ([`DEBUG.md`](DEBUG.md)).

## Emulation

- **Wide char works, and it is why the cell model has a continuation mark**
  (`WIDE_CONT`). Every new grid operation has to respect the pair.
- **An emoji sequence is one cell, and a few things around it stay as they are**
  (`B66`). The cluster lives in `Row.tail`, beside the cells, so the 20-byte cell
  does not grow; the cap there is 16 codepoints (a family of four is seven).
  Two consequences are accepted: a face **without the ZWJ ligature** draws the
  base character and its combining marks (the width is still right, the picture
  is just the first emoji); and a cluster never crosses a row, so a sequence
  broken across a wrap is two cells. The reflow carries the tail with its cell
  (`B68`), so a resize and a restore into another width keep the picture.
- **Shaping works; three consequences are accepted** (`G8` closed on 2026-11-02
  with `rustybuzz`, see [`HISTORY.md`](HISTORY.md)): a **single-cell** run is not
  shaped (no context, no ligature — this is the doom-fire case, 2400 runs of 1);
  the **cursor and the selection break the ligature** around them, because those
  cells become runs of their own; and **bidi reordering is not done** — a cluster
  that comes out of order falls back to cell by cell. A font that merges N cells
  is only used when the advance fits, ±10%.
- **A block selection is a rectangle of the screen, not of the text**: a resize
  carries it by its two corners through the reflow, so a block dragged across
  rows that rewrap comes back near — not equal — to what it was. Re-deriving it
  would mean re-finding each row's text, which is the cost the shape does not
  pay for.
- **With the child asking for the mouse, a plain drag is the child's** —
  Alt+Left drags a local block selection instead (the meaning Alt already has
  without the mouse, `B5`). The press decides the gesture: it starts one only
  when the child holds no button, and its release ends it locally even with
  Alt already up.
- **No `IRM`** (insert mode) and no `CSI ? 3` (80/132): rare to the point of not
  paying for the test. `IRM` costs more than it looks — inserting a cell means a
  branch in `put_ascii_run`, which is the parser's hot path.
- **No DECDL/DECDHL** and no double-height: nothing draws them, so there is no
  state to keep.

### Keyboard

- **`DECKPAM` is tracked, not honored**: `ESC =`/`ESC >` set and clear the mode
  (`DECRQM` 66 answers it, `DECSTR`/`RIS` clear it) and the keypad keeps sending
  what the key is — a digit with NumLock on, the navigation key with it off. The
  programs that ask for the mode cannot read the `ESC O` form: `xterm-256color`
  declares no `k0`..`k9`, so `zsh`'s line editor beeps at `ESC O w` and `vim`
  turns it into `7`. The system terminal this replaces (ghostty 1.3.1, measured
  with `cat -v` under `DECKPAM`) sends the digit and the navigation keysym too,
  so we do the same and accept the divergence from xterm.
- **The alternate keys of the kitty protocol exist for the ASCII letters only**:
  with flag 4, Shift+A is `CSI 97:65:97;2u` (key, shifted, base). A shifted
  punctuation mark stays single-field (`CSI 33;2u`), because the base of `!` is
  `1` on a US board and another key elsewhere — `winit` delivers the key already
  applied to the layout, and undoing that would need a layout table
  (`xkb`/`keymap`) of our own. Without flag 4 the field is the post-Shift code
  (`CSI 65;2u`), which is what the protocol asks for.

## Interface

- **No automatic dark/light theme**: the palette comes from the config, not from
  the desktop portal.
- **Notifications stay inside the window.** Every source (`OSC 9`, `777`, the
  final states of `9;4` and of `7501`, and the BEL under `bell visual`) rings the
  bell on the tab label, turns the workspace card yellow and asks the window
  manager for urgency, but there is no D-Bus, no portal and no
  `org.freedesktop.Notifications`: a desktop notice carries a sender identity,
  and anything the child sends would arrive wearing the terminal's face — a
  remote `ssh` session popping system-level notices is spoofing, not a feature.
  The notification list (the last eight, one per `(source, id)`) has no screen of
  its own yet: the debug socket's `notifications` verb is how it is read.
- **Progress stays in the window.** `OSC 9;4` (mintty/ConEmu) draws a 2 px bar
  on the bottom edge of the tab and of the workspace card (or the percentage on
  the label, under `progress_show = title`): no taskbar bar and no animation —
  the indeterminate state is a still band of light cyan, because something
  that moves costs a clock and a frame per step. The toast is never involved: a 1 Hz
  keepalive repeater would pin it forever. With a single tab there is no tab bar
  and `bar` draws nothing — `title` is the face that shows in every window.
- **An `OSC 8` link opens through `xdg-open`, and only for five schemes**
  (`http`, `https`, `file`, `ftp`, `mailto`). No portal, no D-Bus, no browser
  integration: the URI comes from the child, and anything else becomes a toast
  instead of a handler the desktop happened to register.
- **No expiry for an `OSC 0/2` title.** The tab shows it until another title
  arrives or the screen is reset (`RIS`): the terminal cannot know that the author
  of the title is gone, and the protocol's answer is the app pushing and popping
  it (`B37`). An app that sets one and does not restore it leaves the tab frozen
  ([`EXTERNALS.md`](EXTERNALS.md)).
