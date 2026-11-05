# Configuration

One file, kitty format: **one option per line**, `key value`, leading and
trailing space ignored and `#` commenting to the end of the line (as long as it
is at the start of the line or after a space — that is what makes
`foreground #e6e6e6` a colour and not a comment). Repeated key: the last one
wins.

```sh
~/.config/bzmux/bzmux.conf          # or $XDG_CONFIG_HOME/bzmux/bzmux.conf
```

You do not need to create it: **the defaults are built in** in
`crates/bzmux-core/src/config.rs` (`DEFAULT_CONF`) and your file goes on top,
key by key. A test guarantees the defaults parse clean and that every key of
them appears on this page — the documentation does not age silently.

The command line options (`--font-size`, `--scrollback`, `--cmd`…) override the
file.

## Keys

| key | default | what it does |
|---|---|---|
| `shell` | empty | initial program of the tabs; empty = `$SHELL` (`--cmd` wins) |
| `font_family` | `Monospace` | font family; monospaced only (the cell is the advance of "M") |
| `font_size` | `12` | size in **points** (4..96); the DPI converts to pixels |
| `ligatures` | `standard` | font ligatures: `off`, `standard` (`liga`/`calt`: `fi`, `->`, `!=`) or `all` (`standard` + discretionary). Only shows up in a font that has them (Fira Code, JetBrains Mono); it does not merge cells — the cell is still one character |
| `cursor_shape` | `block` | `block`, `beam` or `underline`; a `CSI 6 q` from an app wins while it lasts |
| `cursor_blink` | `off` | blinking cursor (500 ms per phase); a key or a paste lights the cursor immediately. `CSI 6 q` with an even shape (4/6) asks for a steady cursor and wins while the program wants it |
| `scrollback` | `10000` | history lines above the screen (cap 100000) |
| `window_padding_width` | `0` | margin in px around the grid (cap 200) |
| `bell` | `none` | BEL: `none`, `visual` (marks the window) or `audible` |
| `copy_on_select` | `off` | the selection goes to CLIPBOARD as well (PRIMARY is always) |
| `debug_log` | `off` | diagnostic log; `off` turns it off, a path turns it on |
| `foreground` | `#d6d6d6` | text colour (`#rrggbb` or `#rgb`) |
| `background` | `#000000` | background colour |
| `cursor_color` | `#d6d6d6` | cursor colour |
| `color0` … `color15` | see file | ANSI palette; the rest of the 256 palette (cube and greys) is computed |

The colour default is the **classic xterm palette** (black background, blue
`#0000ee`), the same as the Python bzmux. It is not laziness: `mc`, `htop`, `ls
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
