# Configuration

One file, kitty format: **one option per line**, `key value`, leading and
trailing space ignored and `#` commenting to the end of the line (as long as it
is at the start of the line or after a space — that is what makes
`foreground #e6e6e6` a colour and not a comment). Repeated key: the last one
wins.

```sh
~/.config/tortty/tortty.conf          # or $XDG_CONFIG_HOME/tortty/tortty.conf
```

You do not need to create it: **the defaults are built in**, and your file goes
on top, key by key. The order is: the common base → the template of this system
→ your file → the command line.

The base is
[`crates/core/defaults/tortty.conf`](../crates/core/defaults/tortty.conf), and
beside it one template per system,
[`tortty-linux.conf`](../crates/core/defaults/tortty-linux.conf),
[`tortty-macos.conf`](../crates/core/defaults/tortty-macos.conf) and
[`tortty-windows.conf`](../crates/core/defaults/tortty-windows.conf). A line is
in the base only when the same chord is correct, primary and free of a system
collision on all three — everything else is in the template of the system that
needs it (the font family, `Ctrl+Shift+C` here against `⌘C` on macOS, `Alt+1`
against `⌘1` for the tabs, nothing at all under `Super` on Windows). What
follows is what *your* binary carries; the other two templates are in the repo
to be read, not loaded here.

They are taken into the binary at build time (`include_str!`, so there is still
one file to install), which makes them readable, diffable and usable as a reset:
`tortty --config crates/core/defaults/tortty.conf` (the base alone). A test
guarantees the four files parse clean, that a template never repeats a line of
the base, that every system answers for every action, and that every key appears
on this page — the documentation does not age silently.

The command line options (`--font-size`, `--scrollback`, `--cmd`…) override the
file.

## Keys

| key | default | what it does |
|---|---|---|
| `shell` | empty | initial program of the tabs; empty = `$SHELL` (`--cmd` wins) |
| `font_family` | `Monospace` | font family; monospaced only (the cell is the advance of "M"). The default is the template's: `Menlo` on macOS, `Cascadia Mono` on Windows |
| `font_size` | `12` | size in **points** (4..96); the DPI converts to pixels |
| `ligatures` | `standard` | font ligatures: `off`, `standard` (`liga`/`calt`: `fi`, `->`, `!=`) or `all` (`standard` + discretionary). Only shows up in a font that has them (Fira Code, JetBrains Mono); it does not merge cells — the cell is still one character |
| `cursor_shape` | `block` | `block`, `beam` or `underline`; a `CSI 6 q` from an app wins while it lasts |
| `cursor_blink` | `off` | blinking cursor (500 ms per phase); a key or a paste lights the cursor immediately. `CSI 6 q` with an even shape (4/6) asks for a steady cursor and wins while the program wants it |
| `restore_state` | `layout` | what comes back on the next run: `none` (one tab, a clean screen — nothing of the session is written, and since only the owner of the state writes, the geometry is not remembered either), `layout` (the workspaces and tabs of the last run, in their order, the front where it was) or `content` (the layout **and** what the tabs had on screen and in the history — a photograph, not the process: the shell starts again in the same directory). The window position and size are always remembered, whatever the value |
| `scrollback` | `10000` | history lines above the screen (cap 100000); with `restore_state = content` it is also the size of the snapshot written on the way out |
| `window_padding_width` | `0` | margin in px around the grid (cap 200) |
| `bell` | `none` | BEL: `none`, `visual` (marks the window) or `audible` |
| `copy_on_select` | `off` | the selection goes to CLIPBOARD as well (PRIMARY is always) |
| `bold_as_bright` | `off` | SGR 1 on the eight low colours (`0`–`7`) draws the bright half (`8`–`15`) — xterm's `boldColors`; off by default, because the palette is the owner's and a theme may give the low half its own meaning |
| `debug_log` | `off` | diagnostic log; `off` turns it off, a path turns it on |
| `map` | the table below | `map CHORD ACTION [N]`: binds a chord to an app action (and re-binds one) |
| `unmap` | — | `unmap CHORD`: frees the chord, so the program sees the key again |
| `foreground` | `#d6d6d6` | text colour (`#rrggbb` or `#rgb`) |
| `background` | `#000000` | background colour |
| `cursor_color` | `#d6d6d6` | cursor colour |
| `color0` … `color15` | see file | ANSI palette; the rest of the 256 palette (cube and greys) is computed |

### Shortcuts (`M-H`)

A shortcut is a **chord** — modifiers joined with `+` (`ctrl`, `alt`, `shift`,
`super`) plus one key — and an **action**. The key is a single character or one
of `enter esc tab backspace space insert delete home end pgup pgdn up down left
right f1`…`f24`.

| action | what it does |
|---|---|
| `copy` / `paste` | CLIPBOARD |
| `new_tab` / `close_tab` | tab |
| `new_workspace` / `close_workspace` | workspace |
| `prev_prompt` / `next_prompt` | walk the `OSC 133` marks (`M-R`) |
| `open_config` | the config file in `$EDITOR`, in a new tab |
| `scroll_page_up` / `scroll_page_down` / `scroll_top` / `scroll_bottom` | the history |
| `search` | the search bar over the scrollback (`M-D`), `Ctrl+Shift+F` |
| `select_tab N` / `select_workspace N` | `N` is 1..10, where 10 is key `0` |
| `ws_prev` / `ws_next` / `ws_move_up` / `ws_move_down` | walk and carry the workspace |

The defaults are `map` lines in the defaults file itself — `Ctrl+Shift+C/V/T/W/N/Q`,
`Ctrl+Shift+Up/Down`, `Ctrl+Shift+F/F2`, `Shift+PgUp/PgDn/Home/End`, `Alt+1`…`0`
(tab), `Ctrl+Shift+1`…`0` and `Super+1`…`0` (workspace), `Super+Up/Down` and
`Super+Shift+Up/Down`, one line per chord. That text is the single source: copying
the block into your file reproduces the app's table, changing an action changes it
for you, and **deleting a line frees the key** as surely as `unmap`. A digit under
`Ctrl`/`Alt`/`Super` is read from the **physical** key, so `Ctrl+Shift+2` still
means 2 on a layout where it produces `@`. An unbound key goes to the program.

```conf
map ctrl+shift+y new_tab
unmap ctrl+shift+w
```

The colour default is the **classic xterm palette** (black background, blue
`#0000ee`). It is not laziness: `mc`, `htop`, `ls
--color` and `git diff` were calibrated on it, and swapping it for a "pretty"
theme changes the meaning of the colours the program picked — `mc` blue becomes
something else.

A boolean accepts `on/yes/true/1` and `off/no/false/0`.

`ligatures off` turns off the **ligatures**, not shaping: Arabic and Devanagari
are still shaped (otherwise the text stops being readable). Cursor and selection
always break the ligature of the cell they are on — without that the cursor
would paint in the middle of a glyph that spans more than one cell.

## An error is per line

- **initial load**: what validated holds; the invalid key keeps the default and
  the error comes out with the line number (`line 7: 'font_size': "abc" is not a number`);
- **reload** (editing the file with the app open): **any** error keeps the
  configuration that was already in effect. Half a configuration is worse than
  none.

An unknown key is an error, it is not silently ignored.

## Colours change at run time

A program can query (`OSC 10;?`, `OSC 11;?`, `OSC 4;N;?`) and **change**
(`OSC 4;N;colour`, `OSC 10`, `OSC 11`, `OSC 12`, and the resets
`104`/`110`/`111`/`112`) the colours. The change holds for the session and goes
straight to the renderer; the configuration file is only re-read when you edit
it (and then it wins: the colours from the file replace what the program had
asked for).

## Not yet

The palette as a full theme (16 colours + the 240 of the cube), key remapping
and `include` of another file are in the queue (`docs/BACKLOG.md`, item M-H).
The palette in the file (`color0..15`, fg/bg/cursor) already works — what is
missing is key remapping.
