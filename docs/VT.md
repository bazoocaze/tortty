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
| `OSC 133 ; A` / `B` / `C` / `D` (with the exit status on `D`) | prompt marks: prompt start, command start, output start, command end with the exit status. They feed the prompt navigation (Ctrl+Shift+Up/Down) and the status lifetimes below. `D` also raises the notification when the command ran long (`shell_notify_secs`, default 10 s, `none`/`off`/`0` disables it) — `exit 0 · 32.0 s`, red when it failed |
| `OSC 777 ; notify ; title ; body` | urxvt notification: the same road as `OSC 9` |
| `OSC 1337 ; File=… :base64` | iTerm2 inline image: `inline=1` is required (`inline=0` asks the terminal to write a file, and that is refused with a toast), `width`/`height` in cells — PNG, or the TIFF `chafa -f iterm` sends |
| `OSC 99 ; metadata ; payload` | kitty's identified notification — see [OSC 99](#osc-99) |
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

| state | meaning | what it does |
|---|---|---|
| `0` | clear | removes the root record — the bar goes with it |
| `1` | working, normal | root record `working`, bar filled to the percentage |
| `2` | error | root record `error`: **it asks for you** (see below), no bar either way — and the percentage, when the program gave one, stays on the record |
| `3` | indeterminate | root record `working` without a measure: light cyan bar, full and still (it does not move: moving it would need a clock and a frame per step for a fact the state already tells) |
| `4` | warning / paused | root record `paused`: yellow bar **and** it asks for you |

`percentage` is 0–100 and optional (absent means "busy, no measure"). Where the
bar is drawn, and whether it is drawn at all, is `progress_show`
(`none`/`title`/`bar`) — see [`CONFIG.md`](CONFIG.md).

Four behaviours worth knowing:

- States `1` and `3` are **progress only**: a program that repeats them once a
  second changes nothing (the same state and the same percentage is the same fact
  told twice), and nothing on screen asks for you.
- States `2` and `4` are **final as far as the user is concerned**: they become a
  notification, exactly like `OSC 9` — no text to invent, so the state's own word
  (`error`, `paused`) is what the notification says.
- A digit state tortty does not know is dropped whole: it is not this form at all, and never a notification.
- `OSC 9` with anything that is not `4;…` is a **notification**, as above — the
  two forms share the number and are told apart by shape.

## OSC 99

Kitty's identified desktop notification (`B72`). The escape is
`OSC 99 ; metadata ; payload ST`, both semicolons always present, the metadata
colon-separated `key=value` pairs.

- `p=title` / `p=body` — the payload sets the title or the body; both sides are
  kept apart in the notification, and specifying one of them several times
  concatenates. **Absent, `p` means title** (the spec's quickstart,
  `99;;Hello world`). `p=icon` and `p=buttons` are **accepted and ignored**:
  this terminal draws no icons or buttons in a notification.
- `p=close` — the program retires its own notification: the entry with that
  `i` leaves the list (an unknown id is ignored, and no `i` is a no-op, by
  spec). A close also kills an assembly still held under the id.
- `p=alive` — the program asks which of its notifications live: the answer
  rides the same tab's pty, `OSC 99 ; i=<echo> : p=alive ; id1,id2 <ST>` (the
  ids still in the list; empty when none).
- `p=?` — the support query: the answer is
  `OSC 99 ; i=<echo> : p=? ; p=title,body:o=always <ST>` — the two payload
  types this terminal renders, and `o=always` because it implements no
  occasions (the spec demands the key in that case). Actions (`a`), close
  events (`c`), sounds (`s`), urgency (`u`) and expiry (`w`) are absent
  rather than lied about. This is the OSC family's first reply.
- `i=id` — the notification id: the same id **updates the notification in
  place** (the list replaces the entry and moves it to the end — how a program
  shows progress inside a notification). Without `i`, the notification is
  unique by definition and no updating ever takes place, by spec.
- `d=0` — the notification is not done: pieces accumulate under the id until a
  chunk with `d=1` arrives, and only then it displays. Without an id, `d=0`
  holds nothing (there is nothing to assemble with).
- `e=1` — the payload is Base64 (unpadded accepted); plain otherwise. A payload
  over 2048 bytes plain / 4096 encoded is refused whole, and the trace says so.
- `f`, `t`, `n`, `g`, `s`, `a`, `c`, `w`, `o`, `u` and any unknown key —
  ignored, by spec (filtering keys and the click lifecycle, which has no
  surface here).

The notification rides the same road as every other source: `OSC 9`'s surfaces
(bell on the tab label, yellow card, window urgency), the same list, the same
presence rules. What stays silent is the **click side**: the activation report
(`a=report`) and the close-event report (`c=1`) need a click surface and a way
to find the tab that owns an id — `B72` keeps that in the backlog. A `RIS`
kills the half-assembled chunks; delivered notifications survive it.

The progress form feeds the root [status record](#program-status-osc-7501), so a
program that only knows `9;4` still gets the same record, the same bar and the
same notifications. The form stays out of the root while a real `7501` report
owns it **and is still in flight with a `kind`** — exactly the `kind` and `msg`
a mapped `9;4` would wipe. Nothing richer than that is protected, and the lock
ends with the report: a finished report (or one carrying only a state) leaves
the root to the form again, so the keepalive keeps painting for the rest of the
tab's life.

## Program status: `OSC 7501`

Where `OSC 9` says "something happened", `7501` says **what** the program is
doing. The spec is v0.4 (2026-10-10) and it is implemented with its hard limits.
The body is a list of `key=value` pairs separated by `:`:

```
ESC ] 7501 ; state=blocked:kind=permission:app=terraform:msg=<base64> ST
```

| key | values | notes |
|---|---|---|
| `state` | `idle`, `working`, `done`, `blocked`, `paused`, `error` | the only required key; an unknown state discards the whole report (a state added by a later version of the spec must not read as `idle`). `paused` is tortty's name for the `9;4` *warning/paused* shade, and it is advertised in the feature-detection answer |
| `kind` | `permission`, `question`, `auth` | only means something with `blocked` |
| `id` | a `/`-separated path, up to 8 levels of 32 bytes | addresses one record; **absent** (or empty) is the *root* record |
| `app` | up to 32 bytes | who is asking; a record without it inherits its nearest ancestor's |
| `title` | up to 192 bytes | one line |
| `msg` | up to 2048 bytes when decoded | base64; what the toast shows |
| `progress` | `0`–`100` | only with `working` and `blocked` |
| `state=clear` | — | removes that record and everything below it, or every record when there is no `id` |

The text a notification carries is `msg`, else `title`, else `app`, else the
state's name — one line, already decoded.

**What appears, and when.** Every state declares, once, two fixed facts — and
nothing depends on what the record said before:

| state | still in flight (the bar) | asks for you |
|---|---|---|
| `working` | yes | no |
| `blocked` | yes | **yes** — it is waiting for an answer |
| `paused` | yes | **yes** — same |
| `done` | no | **yes** |
| `error` | no | **yes** |
| `idle` | no | **yes** |

- **The bar** follows the newest record still in flight: `working` green with
  the percentage, `blocked`/`paused` yellow with it. A program that names its
  parts reports several records — the last that reported owns the bar, and an
  older in-flight lane comes back when the newer finishes. `done`, `error` and
  `idle` draw no bar — a finished command has nothing left to show, and the news
  is the notification.
- **Asking for you** is a notification, the same one `OSC 9` raises: the bell in
  front of the tab's label, the workspace card in yellow and the window's urgency.
  Where the user is decides the lifetime, not the sender: watching the tab, the
  bell shows 800 ms and goes; elsewhere it stays until the tab is visited. The
  text is not pushed — it lands in the tab's notification list (last eight, one
  entry per `(source, id)`), which is where the message waits; the state itself
  lives in the grid.
- **A report that says exactly what the record already says is not news**: no new
  event, no notification. That is what keeps a program that reports `working`
  every second — or a shell that reports `idle` at every prompt — silent.
- **A new shell prompt (`OSC 133;A`) drops every record of that terminal.** The
  command that produced them is gone, and the notice survives as the
  notification — which is why the spec's "keep `done`/`error` for the user to
  find" moved to the list and the mark. Keeping them would also make the *next*
  command's final report read as a repetition, and it would not be announced.

**Feature detection.** `OSC 7501 ; ?` is answered with

```
ESC ] 7501 ; ? :states=idle,working,done,blocked,paused,error:kinds=permission,question,auth ST
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
| `ESC P … q … ST` | DCS: **sixel** — decoded by tortty's own decoder, cursor moves below the picture. Anything else answers `0` the standard way (see below) |
| `DCS $ q Pt ST` | **DECRQSS**: asks what a setting is. Answered from what is really kept — `m` (the pen: `0` plus the attributes set, then the foreground, then the background, e.g. `0;1;31m`), `r` (the scroll region, 1-based, e.g. `2;3r`), `SP q` (the cursor style as `DECSCUSR`, e.g. `2 q`; without one seen, the shipped default, steady block) — as `DCS 1 $ r Pt ST`. Anything else (`"p` and the rest) gets `DCS 0 $ r ST` |
| `DCS + q hex ST` | **XTGETTCAP**: asks for termcap values by hex name (`;`-separated). The table holds what is true here — `Co` = `256` (as the exported `TERM` says), `RGB` = `8` (as the exported `COLORTERM` says), `TN` = `tortty` (as the exported `TERM_PROGRAM` says; a kitty check fails here, correctly) — answered as `DCS 1 + r hex ST` with a literal `=` and `;`, unknown names skipped. Nothing known, or not hex at all: `DCS 0 + r ST`. Key names are refused on purpose: what a key sends depends on the keyboard flags in force, so a static answer would lie |
| `ESC _ G … ST` | APC: the **kitty graphics protocol** — direct and chunked transmission, placement, ids, z-order, crop, animation, Unicode placeholders |

Images have their own page: [`FEATURES.md`](FEATURES.md). The one rule that
surprises people is there too — a picture belongs to the cells it covers, so an
erase that touches any of them takes the whole picture (and the alternate screen
keeps its own set).
