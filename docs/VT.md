# VT — the sequences tortty understands

This is the user's reference: what a program may send and what tortty does with
it. Every line is a fact of `crates/core` (the parser), not an intention — if a
sequence is not in these tables, it is **not** implemented, and an unknown
sequence is discarded without touching the screen.

Two conventions used throughout:

- `ESC` is the byte `0x1b`; `ST` is `ESC \` (the 8-bit `0x9c` is accepted where
  clients send it).
- A missing parameter means **1** for counts and **0** for the selectors, as in
  xterm.

## CSI — `ESC [ … final`

| sequence | what it does |
|---|---|
| `CSI n A` / `B` / `C` / `D` | cursor up / down / forward / back, `n` cells (never past the edge) |
| `CSI n E` / `F` | next / previous line: down or up, then column 0 |
| `CSI n G` | column `n` (1-based) on the current row. The old HPA final byte (a grave accent) is accepted too |
| `CSI n d` | row `n` (1-based), same column |
| `CSI r ; c H` or `CSI r ; c f` | cursor position (row; column, 1-based) |
| `CSI s` / `CSI u` | save / restore cursor (with the pen), like `ESC 7` / `ESC 8` |
| `CSI n @` / `P` / `X` | insert `n` blanks / delete `n` chars / erase `n` chars at the cursor |
| `CSI n L` / `M` | insert / delete `n` lines inside the scroll region |
| `CSI n S` / `T` | scroll the region up / down `n` lines |
| `CSI n b` | REP: repeat the last printed character `n` times |
| `CSI n J` | ED: `0` cursor to end, `1` start to cursor, `2` whole screen, `3` whole screen **and** the scrollback (main screen only — on the alternate screen the history is not the app's to take, `B63`) |
| `CSI n K` | EL: `0` cursor to end of row, `1` start of row to cursor, `2` the whole row |
| `CSI n I` / `Z` | forward / backward `n` tab stops |
| `CSI n g` | TBC: `0` clears the tab stop at the cursor, `3` clears all |
| `CSI t ; b r` | DECSTBM: scroll region from row `t` to row `b`; the cursor goes home, `DECOM` clamps it to the region |
| `CSI n h` / `l` | set / reset modes — see [Modes](#modes) |
| `CSI … m` | SGR — see [SGR](#sgr) |
| `CSI n n` | DSR: `5` answers `CSI 0 n` (ready), `6` answers the cursor position (`CSI ? r ; c R` when private) |
| `CSI c` | DA1: answers `CSI ? 62 ; 4 c` — VT220 with **sixel** graphics |
| `CSI > c` | DA2: answers `CSI > 1 ; 100 ; 0 c` |
| `CSI ? n $ p` | DECRQM: answers `CSI ? n ; 1 $ y` (set), `2` (reset) or `0` (not recognised) |
| `CSI ! p` | DECSTR: soft reset (pen, modes and margins back to defaults; the grid stays) |
| `CSI SP q` | DECSCUSR: cursor shape `1`–`6` — block / underline / bar, blinking on odd, steady on even; anything else gives the shape back to the config |
| `CSI 14 t` | answers the text area in **pixels** (`CSI 4 ; height ; width t`) — what `kitten icat` asks before sending an image |
| `CSI 16 t` | answers the cell size in pixels (`CSI 6 ; height ; width t`) |
| `CSI 18 t` | answers the text area in characters (`CSI 8 ; rows ; cols t`) |
| `CSI 22 ; Ps t` / `CSI 23 ; Ps t` | push / pop the title (there is one title, so `Ps` 0, 1 and 2 all do the same) |
| `CSI ? u`, `CSI > u`, `CSI < u`, `CSI = u` | the kitty keyboard protocol: query, push, pop and adjust the flags the child asked for |

Everything else in `CSI` (DECSCNM `?5`, insert mode `4`, the printer, the
rectangular edits, …) is ignored: the sequence is consumed and the screen is
left alone.

### SGR

| attribute | effect |
|---|---|
| `0` | reset the pen (colours, flags, underline) |
| `1` `2` `3` | bold, faint, italic |
| `4` | single underline; `4:0` none, `4:2` double, `4:3` curly, `4:4` dotted, `4:5` dashed |
| `5` `6` | blink: the text turns on and off every 500 ms |
| `7` `9` | inverse, strike |
| `21` | double underline (xterm's meaning, not ECMA-48's "no bold") |
| `22` `23` `24` `25` `27` `29` | remove bold+faint, italic, underline, blink, inverse, strike |
| `30`–`37` | foreground, the eight low palette slots |
| `40`–`47` | background, the eight low palette slots |
| `90`–`97` / `100`–`107` | the bright half of the palette |
| `38` / `48` / `58` | extended colour, `;5;n` (256) or `;2;r;g;b` (truecolour) — the colon form `38:2::r:g:b` is read too |
| `39` `49` `59` | foreground, background and underline colour back to the default |
| `53` `55` | overline on / off |

`bold_as_bright on` (the config, off by default) draws SGR 1 on slots 0–7 with
the bright half. With a truecolour theme the eight low slots are the theme's, so
the two settings are independent.

## OSC — `ESC ] number ; payload ST`

| sequence | what it does |
|---|---|
| `OSC 0 ; title` / `OSC 2 ; title` | window title — what the tab label shows. `OSC 1` (the icon name) is ignored: there is one title, and it is the one worth showing |
| `OSC 4 ; i ; spec` | palette slot `i` (0–15): define it, or `?` to read it back |
| `OSC 7 ; file://host/path` | the shell's working directory (the host is ignored, the path is percent-decoded, an empty payload forgets it) — what the tab shows when the title is empty |
| `OSC 8 ; params ; uri` | hyperlink over the cells that follow; an empty `uri` closes it. `params` (`id=…`) are accepted and ignored |
| `OSC 9 ; text` | notification: toast, tab badge, workspace card, window urgency |
| `OSC 9 ; 4 ; state [; pct]` | progress form — see [OSC 9;4](#progress-osc-94) |
| `OSC 10` / `11` / `12 ; spec` | foreground / background / cursor colour: define, or `?` to read |
| `OSC 52 ; c ; base64` | the child sets the clipboard (`c` = CLIPBOARD, `p` = PRIMARY); the **query** form is refused, and so is a payload cut at the parser's cap |
| `OSC 104 [; i …]` | palette slots back to the palette in force — the config's, not a built-in table (all, or only the slots named) |
| `OSC 110` / `111` / `112` | foreground / background / cursor colour back to the default |
| `OSC 133 ; A` / `B` / `C` / `D` (with the exit status on `D`) | prompt marks: prompt start, command start, output start, command end with the exit status. They feed the prompt navigation (Ctrl+Shift+Up/Down) and the status lifetimes below |
| `OSC 777 ; notify ; title ; body` | urxvt notification: the same road as `OSC 9` |
| `OSC 1337 ; File=… :base64` | iTerm2 inline image: `inline=1` is required (`inline=0` asks the terminal to write a file, and that is refused with a toast), `width`/`height` in cells — PNG, or the TIFF `chafa -f iterm` sends |
| `OSC 7501 ; …` | program status — see [OSC 7501](#program-status-osc-7501) |

The whole OSC body is capped at **4096 bytes** (the image form gets its own,
much larger cap). An OSC past the cap is dropped whole, never truncated into
something that parses.

When a program does something unexpected — a tab title full of `?` and garbage is
the usual — the way to see what it actually sent is the debug socket: the `osc`
verb returns the last 8 OSC payloads, raw and as text ([`DEBUG.md`](DEBUG.md)).

## Modes

Modes are **CSI**, not OSC: `CSI ? n h` sets and `CSI ? n l` resets (the `?`
makes it a *DEC private* mode). `CSI ? n $ p` reports the state back.

| mode | name | what it does |
|---|---|---|
| `?1` | DECCKM | application cursor keys: `↑` sends `ESC O A` instead of `ESC [ A` (what `less` and `vim` ask for) |
| `?6` | DECOM | origin mode: the cursor moves inside the scroll region and `CUP 1;1` is the region's first row |
| `?7` | DECAWM | autowrap: a character past the last column goes to the next row. Off, it stays in the last column |
| `?25` | DECTCEM | the cursor is visible |
| `?47`, `?1047`, `?1049` | — | the alternate screen. `1049` also saves the cursor. The history is **kept** and the alternate screen's lines never enter it (`B63`) |
| `?1000` | — | mouse reporting: presses and releases |
| `?1002` | — | mouse reporting: presses, releases and drags |
| `?1003` | — | mouse reporting: every movement |
| `?1005` | — | UTF-8 mouse coordinates (letters above 223 cells) |
| `?1006` | — | SGR mouse encoding (`CSI < b ; x ; y M/m`), the one that survives column 223 |
| `?1016` | — | mouse coordinates in pixels instead of cells |
| `?1004` | — | report focus in/out to the child |
| `?2004` | — | bracketed paste: a paste arrives wrapped in `CSI 200 ~` … `CSI 201 ~`, so the shell can tell typing from pasting |
| `?2026` | — | synchronized output: the screen is not painted until the app says so |

One ANSI (non-`?`) mode is implemented: `CSI 20 h` / `l`, LNM — a line feed also
returns to column 0.

`ESC =` and `ESC >` (DECKPAM/DECKPNM) are tracked as state, but the bytes of the
keypad do not change: a keypad key sends what it always sends. Insert mode
(`CSI 4 h`, IRM) is deliberately not implemented — it would put a test in the
parser's hot path for a sequence nothing in the wild uses.

## Progress: `OSC 9;4`

The mintty/ConEmu form: `OSC 9 ; 4 ; state [; percentage]`, where the state is
digits, not a sentence.

| state | meaning | on screen |
|---|---|---|
| `0` | clear | the bar, the chip and the label mark go away |
| `1` | working, normal | green bar, filled to the percentage |
| `2` | error | red bar, full |
| `3` | indeterminate | translucent cyan bar, full and still (it does not move: moving it would need a clock and a frame per step for a fact the state already tells) |
| `4` | warning / paused | yellow bar, filled to the percentage |

`percentage` is 0–100 and optional (absent means "busy, no measure"). The bar is
2 px on the bottom edge of the tab and of the workspace card.

Three behaviours worth knowing:

- It is **swallowed, never toasted**. A program that repeats it once a second
  would pin a toast to the screen forever.
- A digit state tortty does not know is still this form: swallowed, no notification.
- `OSC 9` with anything that is not `4;…` is a **notification**, as above — the
  two forms share the number and are told apart by shape.

The progress form also feeds the root [status record](#program-status-osc-7501),
so a program that only knows `9;4` still gets the bar and, with `status_show`, the
chip or the label mark — but not a toast: `9;4` has no text to show, and a
repeater would keep one on screen forever. That mapping stops for good once the
program sends a real `7501` report — a mapped `9;4` would wipe the `kind` and the
`msg` that report carried.

## Program status: `OSC 7501`

Where `OSC 9` says "something happened", `7501` says **what** the program is
doing. The spec is v0.4 (2026-10-10) and it is implemented with its hard limits.
The body is a list of `key=value` pairs separated by `:`:

```
ESC ] 7501 ; state=blocked:kind=permission:app=terraform:msg=<base64> ST
```

| key | values | notes |
|---|---|---|
| `state` | `idle`, `working`, `done`, `blocked`, `error` | the only required key; an unknown state discards the whole report (a state added by a later version of the spec must not read as `idle`) |
| `kind` | `permission`, `question`, `auth` | only means something with `blocked` |
| `id` | a `/`-separated path, up to 8 levels of 32 bytes | addresses one record; **absent** (or empty) is the *root* record — the one the tab bar draws |
| `app` | up to 32 bytes | who is asking; a record without it inherits its nearest ancestor's |
| `title` | up to 192 bytes | one line |
| `msg` | up to 2048 bytes when decoded | base64; what the toast shows |
| `progress` | `0`–`100` | only with `working` and `blocked` |
| `state=clear` | — | removes that record and everything below it, or every record when there is no `id` |

The text a notification carries is `msg`, else `title`, else `app`, else the
state's name — one line, already decoded.

**What appears, and when.**

- The tab's bar follows the state root record, exactly like `9;4` does:
  `working` green with the percentage, `blocked` yellow with the percentage,
  `error` red and full. `done` and `idle` draw no bar — for `done` the news is
  the badge and the toast, not a bar that would stay on screen until the next
  command.
- `done`, `blocked` and `error` **ask for you**: toast (with the app's colour —
  `done` green, `blocked` yellow, `error` red — and the title `blocked · app`), a
  badge on the tab, an alert on the workspace card, and window urgency when the
  window is not in front.
- After the toast leaves, the state stays visible where `status_show` says
  (`chip` on the last grid row, `tab` on the tab label and the card, or `none`) —
  see [`CONFIG.md`](CONFIG.md). The chip is dismissed with a click on it: `done`
  and `error` have no lifetime of their own, so nothing else would ever end them.
  A program that is still working reports again and the chip comes back.
- A new shell prompt (`OSC 133;A`) ends the command that is gone: `working` and
  `blocked` records are dropped with it, while `done` and `error` stay for you to
  find. That is the spec's lifetime rule, and it is why a `blocked` from a
  finished command cannot linger forever.

**Feature detection.** `OSC 7501 ; ?` is answered with

```
ESC ] 7501 ; ? :states=idle,working,done,blocked,error:kinds=permission,question,auth ST
```

**Hard caps.** A report that breaks one is discarded **whole**, before any record
is touched, so a buggy or hostile program cannot grow memory:

| what | limit |
|---|---|
| the whole sequence | 4096 bytes |
| one key | 16 bytes |
| `msg` | 2732 bytes encoded / 2048 decoded |
| `title` | 256 bytes encoded / 192 decoded |
| `app` | 32 bytes |
| `id` | 128 bytes, 32 per segment, 8 levels |
| records per terminal | 256 — the least recently updated leaves to make room |

An unknown key is ignored (that is how the protocol extends), and a malformed
pair is skipped instead of killing the report.

**From a shell** — the `msg` is base64, so `\033` and `\\` above are `ESC` and
`ST`:

```sh
report() { printf '\033]7501;%s\033\\' "$1"; }
report 'state=working:app=make:progress=30'
# GNU base64; on macOS the flag is -b0
report "state=blocked:kind=permission:app=terraform:msg=$(printf 'approve the plan?' | base64 -w0)"
report 'state=done:app=make'
report 'state=clear'
```

## ESC, DCS and APC

Short, because these are the axes the tables above do not cover:

| sequence | what it does |
|---|---|
| `ESC 7` / `ESC 8` | save / restore cursor (DECSC/DECRC) |
| `ESC D` / `E` | index (down, scrolling at the bottom) / next line |
| `ESC M` | reverse index (up, scrolling at the top) |
| `ESC H` | set a tab stop at the cursor |
| `ESC c` | RIS: full reset — grid, pen, modes, margins, images, scrollback |
| `ESC ( 0` / `ESC ) 0` | the DEC line-drawing charset for G0 / G1 (`lqk` → `┌─┐`); `ESC ( B` goes back to ASCII |
| `ESC # 8` | DECALN: the screen fills with `E` (the alignment test) |
| `ESC P … q … ST` | DCS: **sixel** — decoded by tortty's own decoder, cursor moves below the picture |
| `ESC _ G … ST` | APC: the **kitty graphics protocol** — direct and chunked transmission, placement, ids, z-order, crop, animation, Unicode placeholders |

Images have their own page: [`FEATURES.md`](FEATURES.md). The one rule that
surprises people is there too — a picture belongs to the cells it covers, so an
erase that touches any of them takes the whole picture (and the alternate screen
keeps its own set).
