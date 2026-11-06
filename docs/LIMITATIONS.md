# Limits

The limitations of the project.

## Scope

- **One user, one machine.** No session multiplexing, no server, no
  reconnection, no daemon. Closing the window ends everything.
- **One terminal, not a window environment.** No splits at this stage
  (`BACKLOG.md` M-J), no layout manager; tabs and workspaces exist, but they do
  not survive closing (`BACKLOG.md` M-I).
- **Image embedded in the escape (base64), kitty protocol.** The image has to
  arrive inside the escape itself; carrying it over file/temp/SHM, animating
  frames and speaking sixel/iTerm2 are not accepted limits: they are the
  [`BACKLOG.md`](BACKLOG.md) queue.
- **`TERM=xterm-256color`.** We do not invent our own terminfo entry: the goal
  is that `vim`, `mc`, `less` and `dmesg` work with what already exists.

## Platform

- **Linux X11 is the tested target.** Wayland is not exercised here (`M-AB` in
  BACKLOG); Windows and macOS are untested and are in the queue as `M-X`/`M-Y`
  of [`BACKLOG`](BACKLOG.md), with what each one
  requires in [`PORTABILITY.md`](PORTABILITY.md). Until they go through CI, "it
  works there" is an expectation, not a fact.
- **Window position on Wayland**: the client does not choose where the window
  goes. When geometry is persisted (J2/J3), on Wayland only size and maximized
  are stored. It is not missing code, it is the protocol.
- **No rasterizer of our own.** Drawing is `wgpu` and nothing else: there is no
  cell-by-cell CPU painter to fall back on, and writing one is an entire
  renderer, not a switch. What `wgpu` offers **is** used — `render.rs` prefers a
  hardware adapter and, when there is none, takes the CPU one it can find
  (llvmpipe/lavapipe) and says so on stderr once, because a VM or a container
  without `/dev/dri` has only that. Whether it **presents** is another matter:
  under an Xvfb of its own llvmpipe draws (`tools/visual.sh` passes without
  `/dev/dri`), but in the owner's confined session on 2026-11-03 the window
  received no frame — Vulkan blew the present timeout and GL returned success
  without painting a pixel. Whoever runs confined uses `sandbox @gpu`, which presents
  for real ([`DEBUG.md`](DEBUG.md)).
- **No signing/notarization for now.** macOS 15+ requires "Open Anyway" for a
  binary downloaded from the network. It is in `M-Z` of
  [`BACKLOG`](BACKLOG.md) (the cost is the developer account, US$ 99/year), behind
  `M-X`/`M-Y`: without a compiled target there is nothing to sign.

## Emulation

- **Wide char works, and it is why the cell model has a continuation mark**
  (`WIDE_CONT`). Every new grid operation has to respect the pair.
- **Grapheme combination**: each cell carries a `char`; a combining mark that did
  not come together in the same commit as the text becomes its own cell. Composing
  the two is queued (`BACKLOG.md` M-U).
- **Shaping works; three consequences are accepted** (`G8` closed on 2026-11-02
  with `rustybuzz`, see [`HISTORY.md`](HISTORY.md)): a **single-cell** run is not
  shaped (no context, no ligature — this is the doom-fire case, 2400 runs of 1);
  the **cursor and the selection break the ligature** around them, because those
  cells become runs of their own; and **bidi reordering is not done** — a cluster
  that comes out of order falls back to cell by cell. A font that merges N cells
  is only used when the advance fits, ±10%.
- **No `IRM`** (insert mode) and no `CSI ? 3` (80/132): rare to the point of not
  paying for the test. `IRM` costs more than it looks — inserting a cell means a
  branch in `put_ascii_run`, which is the parser's hot path.
- **No DECDL/DECDHL** and no double-height: nothing draws them, so there is no
  state to keep.

### Keyboard

- **The numeric keypad has no application mode** (`DECKPAM`/`DECKPNM`, `ESC =` /
  `ESC >`): the parser takes both escapes and does nothing with them, so a keypad
  key always sends its character (`7`, `+`) and never the `\EOw` form of a
  terminal in that mode. The keypad itself works; what is missing is the mode. The
  `xterm-256color` installed here declares no `kp*` capability, so a program that
  set the mode would have nothing to read back.
- **Shift+letter in the kitty protocol comes out with the *post*-Shift code**:
  Shift+A becomes `CSI 65;2u`, not `CSI 97;2u`. `winit` delivers the key already
  applied to the layout and the **base** key would require its own layout table
  (`xkb`/`keymap`) only to undo Shift. Whoever needs the base key for a shortcut
  (`ctrl+shift+a` distinct from `ctrl+a`) only has the shifted code.

## Interface

- **Fixed shortcuts up to F3.** There is no remapping; the config file (which
  comes in F3) covers font, colors, padding, bell, scrollback. Remapping is
  `M-H` in [`BACKLOG.md`](BACKLOG.md).
- **No automatic dark/light theme**: the palette comes from the config, not from
  the desktop portal.
- **No context menu and no DnD** of a file onto the window. The menu is `M-V` in
  [`BACKLOG.md`](BACKLOG.md); dragging a file onto the window has no id yet.

## Development

- **`--no-vsync` is not a mode of use**: it is an instrument. In normal use
  drawing is limited by vsync, and that is how it should be.
