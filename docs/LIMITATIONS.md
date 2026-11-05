# Limits

What tortty **does not** do and why. Only a **decision** or an accepted
consequence of scope enters here. Functionality of tortty-python that has not
been migrated yet is `missing` in [`MIGRATION.md`](MIGRATION.md) — it is not a
limit. If a limit stops existing, it leaves here and becomes a completed item in
[`HISTORY.md`](HISTORY.md).

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
  BACKLOG); Windows and macOS **are not parity** (Python never ran there) and are
  in the queue as `M-X`/`M-Y` of [`BACKLOG`](BACKLOG.md), with what each one
  requires in [`PORTABILITY.md`](PORTABILITY.md). Until they go through CI, "it
  works there" is an expectation, not a fact.
- **Window position on Wayland**: the client does not choose where the window
  goes. When geometry is persisted (J2/J3), on Wayland only size and maximized
  are stored. It is not missing code, it is the protocol.
- **No software backend of our own.** If there is no usable Vulkan/Metal/D3D12/GL,
  the app exits with a message. Rasterizing on the CPU is an entire renderer.
  Worse: **llvmpipe is not an emergency exit** — measured on 2026-11-03 inside
  `sandbox` without `/dev/dri`, the present either blows the timeout (Vulkan) or
  returns success without painting a pixel (GL). Whoever runs confined uses
  `sandbox @gpu`, which presents for real; the app warns this on stderr, once (see
  [`DEBUG.md`](DEBUG.md)).
- **No signing/notarization for now.** macOS 15+ requires "Open Anyway" for a
  binary downloaded from the network. It is in `M-Z` of
  [`BACKLOG`](BACKLOG.md) (the cost is the developer account, US$ 99/year), behind
  `M-X`/`M-Y`: without a compiled target there is nothing to sign.

## Emulation

- **Wide char works, it is a conscious improvement** — and that is why the cell
  model has a continuation mark (`WIDE_CONT`). tortty-python does not have it:
  there CJK occupies one cell. Every new grid operation has to respect the pair.
- **Grapheme combination**: each cell carries a `char`; a combining mark that did
  not come together in the same commit as the text becomes its own cell. Really
  composing is new activity (`BACKLOG.md` M-U), not a limit accepted forever.
- **No ligatures/complex shaping** (G8): `swash` rasterizes a glyph, it does not
  shape like HarfBuzz. There is no plan to switch to `harfbuzz` at this stage.
- **No `IRM`/`LNM`** (insert mode, newline mode) and no `CSI ? 3` (80/132):
  rare to the point of not paying for the test. They stay registered in the
  parser as a known pending item.
- **No DECDL/DECDHL** and no double-height: Python does not have them either.

### Keyboard

- **Shift+letter in the kitty protocol comes out with the *post*-Shift code**:
  Shift+A becomes `CSI 65;2u`, not `CSI 97;2u`. `winit` delivers the key already
  applied to the layout and the **base** key would require its own layout table
  (`xkb`/`keymap`) only to undo Shift. Python, via Gdk, has the same limitation:
  it is parity, not regression. Whoever needs the base key for a shortcut
  (`ctrl+shift+a` distinct from `ctrl+a`) only has the shifted code.
- **F3 comes out `CSI 1;2R`, diverging from Python, on purpose.** `kitty_kbd.py`
  maps `GDK_KEY_F3` to `(13, "~")` — `13` is the number of **Enter** and
  `CSI 13~` is xterm's Shift+F3, not the kitty form (`F1`..`F4` are `1P`..`1S`).
  The right thing is what is in the code; do not "fix" it back for parity.

## Interface

- **Fixed shortcuts up to F3.** There is no remapping; the config file (which
  comes in F3) covers font, colors, padding, bell, scrollback.
- **No automatic dark/light theme**: the palette comes from the config, not from
  the desktop portal.
- **No context menu and no DnD** of a file onto the window.

## Development

- **A performance number lives in one place** ([`BENCHMARKS.md`](BENCHMARKS.md))
  and not in README, TASK or a code comment. Repeating a number produces
  divergence and every re-measurement becomes a hunt.
- **`--no-vsync` is not a mode of use**: it is an instrument. In normal use
  drawing is limited by vsync, and that is how it should be.
