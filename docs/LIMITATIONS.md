# Limits

Accepted or unresolved limitations. Planned work lives in `BACKLOG.md`
(`B<n>`); here the id is `L<n>`. Overcome limitations leave this file and go to
`HISTORY.md` (probably through a `BACKLOG` activity).

## Scope

- **One user, one machine.** No session multiplexing, no server, no
  reconnection, no daemon. Closing the window ends everything.
- **One terminal, not a window environment.** No splits at this stage
  (`BACKLOG.md` M-J), no layout manager; tabs and workspaces exist, but they do
  not survive closing (`BACKLOG.md` M-I).
- **One image, drawn as it arrived.** The kitty protocol is accepted for what
  `icat` needs (direct, file, temp and SHM transmission); animating frames and
  speaking sixel/iTerm2 are not limits: they are the
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
  under an Xvfb of its own llvmpipe draws (`tools/render-check.sh` passes without
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
- **The alternate keys of the kitty protocol exist for the ASCII letters only**:
  with flag 4, Shift+A is `CSI 97:65:97;2u` (key, shifted, base). A shifted
  punctuation mark stays single-field (`CSI 33;2u`), because the base of `!` is
  `1` on a US board and another key elsewhere — `winit` delivers the key already
  applied to the layout, and undoing that would need a layout table
  (`xkb`/`keymap`) of our own. Without flag 4 the field is the post-Shift code
  (`CSI 65;2u`), which is what the protocol asks for.

## Interface

- **Fixed shortcuts up to F3.** There is no remapping; the config file (which
  comes in F3) covers font, colors, padding, bell, scrollback. Remapping is
  `M-H` in [`BACKLOG.md`](BACKLOG.md).
- **No automatic dark/light theme**: the palette comes from the config, not from
  the desktop portal.
- **The scrollbar is drawn over the last column**, not beside it: the window
  does not reserve a strip for it, and reserving one would narrow every terminal
  for a control that appears only when there is history. It is 8 px wide (a
  target that can be hit) and translucent, so what is under it stays readable —
  but on the last column the text is behind the thumb.
- **Notifications stay inside the window.** `OSC 9`/`777` toast, badge and
  ask the window manager for urgency, but there is no D-Bus, no portal and no
  `org.freedesktop.Notifications`: a desktop notice carries a sender identity,
  and anything the child sends would arrive wearing the terminal's face — a
  remote `ssh` session popping system-level notices is spoofing, not a feature.
- **No progress display.** `OSC 9` carries two protocols: the iTerm2 message
  (`9;text`, shown) and the mintty/ConEmu progress form (`9;4;state[;progress]`,
  swallowed). The progress state goes nowhere — no taskbar bar, no percentage
  — because a 1 Hz keepalive repeater (what the form is for) would pin any
  toast or badge forever. If progress UI ever pays, it is its own item, not a
  notification.
- **No context menu and no DnD** of a file onto the window. The menu is `M-V` in
  [`BACKLOG.md`](BACKLOG.md); dragging a file onto the window has no id yet.
- **An `OSC 8` link opens through `xdg-open`, and only for five schemes**
  (`http`, `https`, `file`, `ftp`, `mailto`). No portal, no D-Bus, no browser
  integration: the URI comes from the child, and anything else becomes a toast
  instead of a handler the desktop happened to register.
- **No expiry for an `OSC 0/2` title.** The tab shows it until another title
  arrives or the screen is reset (`RIS`): the terminal cannot know that the author
  of the title is gone, and the protocol's answer is the app pushing and popping
  it (`B37`). An app that sets one and does not restore it leaves the tab frozen
  ([`EXTERNALS.md`](EXTERNALS.md)).

## Development

- **`--no-vsync` is not a mode of use**: it is an instrument. In normal use
  drawing is limited by vsync, and that is how it should be.
