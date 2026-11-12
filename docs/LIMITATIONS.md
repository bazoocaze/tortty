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
- **The second window is a scratch.** One tab, the default size, and nothing
  of it is remembered on the way out — so what the second window saw is not in
  the state, and closing one window never edits another's session.
- **Decoration is not content.** The line that separates a restored picture from
  the new run is marked when it is written and skipped when the snapshot is taken
  (`B48`): it shows on the screen and leaves no trace in the file, which is the
  point — but it also means the picture never records *where* a past run ended, so
  two runs look continuous in the restored content.
- **One image, drawn as it arrived.** The kitty protocol is accepted for what
  `icat` needs (direct, file, temp and SHM transmission), and iTerm2's inline
  one (`OSC 1337;File=`, display only — `inline=0` would have the terminal write
  the child's bytes to disk, and that it does not do): one image per id, with no
  notion of frame.
- **`TERM=xterm-256color`.** We do not invent our own terminfo entry: the goal
  is that `vim`, `mc`, `less` and `dmesg` work with what already exists.

## Platform

- **No rasterizer of our own.** Drawing is `wgpu` and nothing else: there is no
  cell-by-cell CPU painter to fall back on, and writing one is an entire
  renderer, not a switch. What `wgpu` offers **is** used — `render.rs` prefers a
  hardware adapter and, when there is none, takes the CPU one it can find
  (llvmpipe/lavapipe) and says so on stderr once, because a VM or a container
  without `/dev/dri` has only that. Whether it **presents** is another matter:
  under an Xvfb of its own llvmpipe draws (`tools/render-check.sh` passes without
  `/dev/dri`), but in the owner's confined session on 2026-11-03 the window
  received no frame — Vulkan blew the present timeout and GL returned success
  without painting a pixel. Whoever runs confined uses `sandbox @gpu`, which presents
  for real ([`DEBUG.md`](DEBUG.md)).

## Emulation

- **Wide char works, and it is why the cell model has a continuation mark**
  (`WIDE_CONT`). Every new grid operation has to respect the pair.
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
- **Notifications stay inside the window.** `OSC 9`/`777` toast, badge and
  ask the window manager for urgency, but there is no D-Bus, no portal and no
  `org.freedesktop.Notifications`: a desktop notice carries a sender identity,
  and anything the child sends would arrive wearing the terminal's face — a
  remote `ssh` session popping system-level notices is spoofing, not a feature.
- **Progress stays in the window.** `OSC 9;4` (mintty/ConEmu) draws a 2 px bar
  on the bottom edge of the tab and of the workspace card: no taskbar bar, no
  percentage in the title, and no animation — the indeterminate state is a still
  translucent band, because something that moves costs a clock and a frame per
  step. The toast is never involved: a 1 Hz keepalive repeater would pin it
  forever.
- **An `OSC 8` link opens through `xdg-open`, and only for five schemes**
  (`http`, `https`, `file`, `ftp`, `mailto`). No portal, no D-Bus, no browser
  integration: the URI comes from the child, and anything else becomes a toast
  instead of a handler the desktop happened to register.
- **No expiry for an `OSC 0/2` title.** The tab shows it until another title
  arrives or the screen is reset (`RIS`): the terminal cannot know that the author
  of the title is gone, and the protocol's answer is the app pushing and popping
  it (`B37`). An app that sets one and does not restore it leaves the tab frozen
  ([`EXTERNALS.md`](EXTERNALS.md)).
