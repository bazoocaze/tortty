# Siblings

Three implementations of the same terminal live side by side: **Rust** (this
repo, the reference), **Python** (`tortty-python`) and **Java**
(`tortty-java`). Each one owns its own docs and its own code; this file owns
one fact only — **where their configuration agrees and where it does not**.

Audited on **2026-11-12**, against `tortty` `1658c16`, `tortty-python` `68c07a4`,
`tortty-java` `2d408da`. This table is an audit, not a gate: nothing fails when
the three drift, which is why it carries the date.

## Config Parity

33 key names in the union: **31 in all three**, **2 only in Python**
(`bold_as_bright`, `letter_spacing`). A cell holds that version's default; `—`
means the parser refuses the key as unknown.

| key | Rust (main version) | Python | Java |
|---|---|---|---|
| `shell` | empty = `$SHELL` | empty = `$SHELL` | empty = `$SHELL` |
| `font_family` | `Monospace` | `Monospace` | **`DejaVu Sans Mono`** |
| `font_size` | `12` (4..96 pt) | `12` (**4..30** pt) | **`17`** (4..96 pt) |
| `ligatures` | `standard` | `standard` | **`off`** |
| `cursor_shape` | `block` | `block` | `block` |
| `cursor_blink` | `off` | `off` | `off` |
| `restore_state` | `layout` (`none`/`layout`/**`content`**) | `layout` (`none`/`layout`) | `layout` (`none`/`layout`/**`content`**) |
| `scrollback` | `10000` (cap 100000) | **`1000`** (**1..10000**) | `10000` (cap 100000) |
| `window_padding_width` | `0` (cap 200) | `0` (cap 200) | `0` (cap 200) |
| `bell` | `none` | `none` | `none` |
| `copy_on_select` | `off` | `off` | **`on`** |
| `debug_log` | `off` | **`/tmp/tortty-python.debug.log`** | off (empty) |
| `bold_as_bright` | — | `off` | — |
| `letter_spacing` | — | `0` (0..20 px) | — |
| `foreground` | `#d6d6d6` | `#d6d6d6` | `#d6d6d6` |
| `background` | `#000000` | `#000000` | `#000000` |
| `cursor_color` | `#d6d6d6` | `#d6d6d6` | `#d6d6d6` |
| `color0` … `color15` | classic xterm | identical | identical |

The 16 ANSI colours are the **same values byte for byte** in the three
(`#000000`, `#cd0000`, `#00cd00`, `#cdcd00`, `#0000ee`, `#cd00cd`, `#00cdcd`,
`#e5e5e5`, `#7f7f7f`, `#ff0000`, `#00ff00`, `#ffff00`, `#5c5cff`, `#ff00ff`,
`#00ffff`, `#ffffff`); `CONFIG.md` carries the table, and the other two copied it
because `mc`, `htop` and `ls --color` were calibrated on it.

### The nine divergences

1. **`font_family`** — Java names the face it ships with (`DejaVu Sans Mono`);
   Rust and Python open with the generic `Monospace` alias.
2. **`font_size`** — Java opens at 17 pt, twice the other two, and Python stops
   at **30 pt** where Rust and Java go to 96.
3. **`ligatures`** — Java defaults to `off`; the other two to `standard`.
4. **`restore_state`** — Python has two degrees (`content` — the ANSI snapshot —
   does not exist there); Rust and Java have the three.
5. **`scrollback`** — Python keeps 1000 lines with a ceiling of 10000 (its cell
   is heavier: the file declares ~50 B per cell); Rust and Java keep 10000 up to
   100000.
6. **`copy_on_select`** — Java's default is `on`, keeping its copy-on-release;
   Rust and Python leave it `off`.
7. **`debug_log`** — Python logs to `/tmp/tortty-python.debug.log` **by default**
   (a path means on); Rust and Java are off until a path is given.
8. **`bold_as_bright`** (`off`) — Python only: `SGR 1` over `color0..7` picks the
   bright variant (xterm's `boldColors`). Absent from Rust and Java.
9. **`letter_spacing`** (`0`, 0..20 px) — Python only, the cell grows with it.
   Absent from Rust and Java.

One smaller one, in accepted **values** rather than keys: Java also takes `off`
for `bell` and `none`/`on` for `ligatures`, where Rust and Python accept only the
canonical three names. `debug_log` is read the same way in the three — an empty
value and the literal `off` both mean "no log" (`Config.java`, `debug_log_path`)
— and only its default differs (divergence 7).
