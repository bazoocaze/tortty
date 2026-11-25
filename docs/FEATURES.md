# Features

What tortty does today, by area. One line per capability; where a capability has
a known limit, the **limit** is named in the same entry. A limit in brackets with
an id is in the [`BACKLOG.md`](BACKLOG.md) queue; without one it is an accepted
consequence, explained in [`LIMITATIONS.md`](LIMITATIONS.md).

Numbers are [`BENCHMARKS.md`](BENCHMARKS.md); configuration keys are
[`CONFIG.md`](CONFIG.md).

## Terminal core

- VT parser: C0/C1, CSI, OSC, and DCS/APC/PM/SOS routing, incremental UTF-8 with a
  buffered partial and invalid input as U+FFFD. **limit:** a DCS is routed so
  that sixel reaches the decoder and every other kind is dropped — `DECRQSS` and
  `XTGETTCAP`, the two that have a standard "not supported" to give, among them
  (`B73`).
- Combining marks (`M-U`) compose into the character before them — up to three
  per cell, drawn over it at the font's own bearing — so `e` + `U+0301` is one
  cell with an accent and not two cells. **limit:** more than three are dropped,
  a mark at the very start of the grid has nothing to attach to, and a reflow
  drops them (a mark is not addressed by a column).
- Cursor: CUU/CUD/CUF/CUB/CNL/CPL/CHA/VPA/CUP/HVP, DECSC/DECRC and `CSI s`/`u`,
  DECSTBM scroll region, RI at the margin. With `?6` (DECOM) on, the absolute
  moves count from the region top and relative walks stop at its margins; the
  mode homes the cursor as it flips, and only the region bounds an absolute one.
- Editing: IL/DL/ICH/DCH/ECH, EL 0/1/2, ED 0/1/2/3 — `ED 3` clears the scrollback
  and kills the selection anchors; **limit:** only from the main screen — up in
  the alternate screen it clears nothing of what lies under the app (`B63`).
- Modes: `?1` DECCKM, `?25`, `?47`/`?1047`/`?1049` alt screen, `?1048`,
  `?1000`/`?1002`/`?1003`/`?1006`/`?1016`, `?1004` focus, `?2004` paste, `?2026`
  sync, and `20` LNM (LF returns the column too).
- SGR: bold, italic, underline (styles `4:2`…`4:5`), reverse, 16 colours, 256,
  truecolor `38;2`/`48;2`, and `:` subparameters.
- OSC: 0/2 window title, 1 icon title, 4 palette query, 7 shell cwd, 8 hyperlink,
  10/11 default colour query, 52 remote clipboard. **limit:** everything else
  is discarded — OSC 133 (prompt marks) among them (`M-R`).
- Hyperlinks, two ways. **Declared:** `OSC 8` puts the URI in the cell (an index
  into `Term::links`, repeated URIs share it). **Detected:** a URL the child only
  printed is found by `links::scan` when Ctrl is held or the menu opens, never
  stored per cell (`B69`); a URL cut by the right edge is joined through
  `Row.wrapped`. Ctrl underlines every link on screen and Ctrl+click hands it to
  `xdg-open`; `OSC 8` wins where both exist. **limit:** only `http`, `https`,
  `file`, `ftp` and `mailto` are links, and the scanner and the opener share that
  one list, so an underline never offers a refused click — the rest of the URI
  space (`javascript:`, `data:`, an application's SSH handler) is not a terminal's
  to launch. A scheme alone (`http://`), a single-label host that is not
  `localhost`, and a URL split by a hard newline are not links.
- DEC Special Graphics charset (G0).
- Alt screen: separate grid, swap preserving the cursor, crop/pad on resize (no
  reflow there). The switch lends the screen and takes it back: the history
  survives `mc`/`less`/`vim` whole, what they write up there never enters it, and
  a `clear` up there takes neither the history nor the pictures it covers
  (`B63`).
- Reflow on resize: joins and splits soft lines, keeps the coloured background,
  carries the selection to the same characters and the **grapheme cluster** to the
  same cell (`B68`: the tail is addressed by column, so a cut remaps it with the
  cell it belongs to).
- Scrollback with a configurable cap ([`CONFIG.md`](CONFIG.md)) and an 8 px
  scrollbar on the grid's right edge: it appears only when there is history, the
  thumb is the share of the buffer the view shows, a click jumps and a drag walks
  it — strip, thumb and the press target are one geometry (`chrome::Scrollbar`). Output moves the view so the line being read stays put — inside a `?2026`
  window too, when all the window does is write; a window that redraws the
  screen (`ED 2`/`ED 3`/cursor home) and a `clear` (`ED 3`) do not drag the
  reader. **limit:** no search (`M-D`).
- Wide cells carry a continuation mark. **limit:** a combining mark arriving in a
  later commit becomes its own cell (`M-U`).
- `TERM=xterm-256color` with no terminfo of our own.

## Input

- Keyboard: arrows, Home/End, Ins/Del, PgUp/PgDn, F1–F24 (F13–F24 as F1–F12
  with Shift, which is what xterm's terminfo says) and the numeric keypad (its
  digits and operators in every NumLock state, the navigation cluster with
  NumLock off; `DECKPAM` (`ESC =`) is tracked and answered by `DECRQM` as 66, and
  `DECKPNM`/`DECSTR`/`RIS` take it back); modifiers as a parameter; DECCKM;
  Shift+Tab; Ctrl+letter; Alt as an ESC prefix; dead keys and composition (ABNT2
  `~ ^ ´`); printable text from the event on non-US layouts. **limit:** `DECKPAM`
  does not change the bytes of the keypad ([`LIMITATIONS.md`](LIMITATIONS.md)).
- kitty keyboard protocol: push/pop/set/query, flag 1 (disambiguate), flag 2
  (event types, release `:3`), flag 4 (alternate keys: `CSI 97:65:97;2u`), flag 8
  (all keys as CSI u), flag 16 (associated text). **limit:** the alternate pair
  exists only for the ASCII letters — the digit row would need the layout (`!` is
  `1` on a US board and something else elsewhere), so it goes out with a single
  field (`CSI 33;2u`).
- Mouse: X10 and SGR encodings, modifiers in the button code, wheel, buttons 8/9,
  motion with and without a button, and `?1016` (the SGR shape with the
  coordinates in pixels from the grid origin). The wheel handed to the child is
  press only — no protocol has a wheel release — and its **count follows the
  finger**: a detent is one notch, a touchpad fraction accumulates until it
  completes one, and a direction change drops the remainder.
- Selection: drag with a 3 px threshold, double click for the word (a word ends
  only at whitespace, so a path comes out whole), triple click for the whole
  line — a soft-wrapped line included —, absolute anchor that survives scrolling
  and output, drawn as inversion. **Alt+drag** takes a **block**: the same
  columns on every row, dragged from any corner, copied one line each — the
  column of a table, not the ends of its lines. **limit:** a block is carried
  across a resize by its two corners (the reflow does not re-derive the
  rectangle), and with the child asking for the mouse a plain drag is reported
  to it instead — Alt+drag still selects a block.
- Context menu (`M-V`, `B64`): **one menu, four contexts**, chosen by where the
  pointer is — on the grid (**Copy**, **Paste**, **Open link**, **New tab**), on a
  workspace card (**New workspace**, **Rename**, **Move up**, **Move down**,
  **Close workspace**), on a tab (**New tab**, **Close tab**) and on the bar's
  empty strip right of the last tab (**New tab** alone). It is chrome from the
  `M-Q` layout: a chip like the search bar's, hanging from the pointer's **cell**
  on the grid (so a row lines up with the text it covers) and from the **pointer**
  on the sidebar and the bar, which are not made of cells, then pulled inside the
  window. One box, one hit test, one key path: the arrows walk the usable rows,
  `Enter` runs one, `Escape` or any other key closes it, and a row that cannot be
  used is drawn dim and does nothing — no selection to copy, no link under the
  pointer, only one workspace left to close, `Move up` on the first card.
  **limit:** with the child asking for the mouse the grid's menu is the child's —
  `Shift`+right opens it then; the sidebar and the bar are always ours. The
  sidebar's free row and the window's margin act on nothing.
- Clipboard: PRIMARY on release, CLIPBOARD with `copy_on_select`,
  Ctrl+Shift+C/V, button 2 pastes PRIMARY, bracketed paste, `OSC 52` from the
  child (`c` sets CLIPBOARD, `p` PRIMARY — the `ssh` copy). **limit:** PRIMARY
  dies with the app — it owns the selection; CLIPBOARD survives. A remote can
  write but never read: the `?` query is refused, and so are non-text payloads
  and bodies cut at the OSC cap.

## Drawing

- `wgpu`, one pipeline and one draw call per category (background, glyph,
  underline, cursor); the adapter is chosen by rule, hardware before CPU.
  **limit:** no rasterizer of our own, and the CPU adapter serves content, not a
  confined session ([`LIMITATIONS.md`](LIMITATIONS.md)).
- Font in **points** converted by DPI, cell metric from the advance of `M`,
  baseline layout, whole cells.
- Bold and italic from a real face, no synthesis: without the face it falls back
  to regular.
- `font_family` takes a fallback list (`Cascadia Mono, Consolas`): the first name
  installed on the machine wins and the bold and italic faces follow it, so one
  template can name what two versions of a system ship.
- Per-glyph fallback chain by name, so a missing font does not become another
  one; colour emoji (CBDT) in their own RGBA atlas; CJK, arrows, Greek. The
  emoji and image textures are `Rgba8UnormSrgb` while the render target is not
  (the cell colours are already sRGB), so `image.wgsl` encodes linear → sRGB on
  the way out: without that, every emoji and every protocol picture was drawn one
  gamma too dark (the heart's `(244, 67, 54)` reached the screen as `(231, 14, 9)`).
- Ligatures and shaping with `rustybuzz` (`off`/`standard`/`all`). **limit:**
  single-cell runs are not shaped, the cursor and the selection break the
  ligature around them, and bidi is not reordered.
- **A grapheme cluster is one cell** (`B66`): `👨‍👩‍👧`, `🇧🇷`, `👍🏽`, `1️⃣` and
  `❤️` occupy the two columns of one character, and the codepoints that live
  beside the base sit in the row's tail (`Row::tail`) — that is what makes the
  width right, so a box drawn around an emoji lines up. Drawing shapes the whole
  cluster: `shape::cluster_gid` asks each face of the chain for **one** glyph for
  the whole string, and only a face with the ligature answers (Noto Color Emoji
  draws the family, the flag, the tinted hand and the keycap), so a family is one
  picture and not three faces. `VS16` makes the cell emoji presentation (two
  columns) and `VS15` keeps the text glyph (one). Copy takes the whole cluster,
  joiners included. **limit:** the tail keeps 16 codepoints; a face without the
  ligature draws the base and its combining marks only; and a cluster never
  crosses a row, so a joiner at the end of a line joins nothing.
- Cursor block/beam/underline with blink; `CSI 6 q` from an application wins.
- Underline at the width of the run, selection as inversion, padding in px,
  background painted to the border.
- Damage per line: only dirty lines are rebuilt, and a full-screen scroll shifts
  the lines that are ready. **limit:** per-region drawing is not done (`M-AC`).
- Box drawing and block elements are drawn half a pixel wider than their ink, so
  that two neighbours meet at a fractional cell width.
- At most one frame every 16 ms, and no drawing while `?2026` is open — the
  window is held until it closes, with a 200 ms cap for an app that forgets.

## Images (kitty protocol)

- Transmit, display, place by id, delete by id or everything, capability query
  (`a=q`; with a file medium and a name it reads the file and answers `OK` or
  `ENOENT`, storing nothing — the spec's "will try to load the image and
  respond with either OK or an error"), chunking, `t=d/f/t/s` (direct, file,
  temp file deleted after reading *under
  the protocol's rule*: temp dir plus `tty-graphics-protocol` in the path,
  otherwise read but kept — the path comes from the child; POSIX shared
  memory under `/dev/shm` — the app loads it, the core never touches
  the filesystem), `f=24/32/100` (PNG decoded in the app), `o=z`, `s=`/`v=`,
  `c=`/`r=`, `X=`/`Y=`, `C=1`, `q=`, and the **source crop** `x=`/`y=`/`w=`/`h=`
  (the rectangle of the picture to show, which is the texture `uv`; the cells
  follow it when `c=`/`r=` are absent, and a rectangle that leaves the image
  answers `EINVAL:crop outside the image` instead of being clamped), the
  **placement id** `p=` (the same `(image, p)` moves that placement instead of
  adding another one, and `d=p`/`d=P` delete one placement or every placement of
  an image without deleting the image) and the **depth** `z=` (negative draws
  *behind* the text — the span goes out before the grid's first draw — positive
  in front, ties in placement order); the image follows the scroll and is pruned
  with the history; `ED 3` clears images.
- **Unicode placeholders** (`U=1`, `M-L`): a virtual placement (`a=p,U=1,i=…,
  c=<cols>,r=<rows>`) is the prototype for cells holding `U+10EEEE` — the picture
  is drawn wherever those cells are, so any program that moves text moves the
  image. The cell's foreground colour carries the image id, its underline colour
  the placement id, and the combining diacritics the row and column, with the
  protocol's inheritance from the cell to the left (the table is the one from
  `rowcolumn-diacritics.txt`, in its order). **limit:** `r=1` edits of an
  animation frame and `a=c` composition stay refused, and a mark is placed by the
  font's own bearing — base and mark are not re-shaped together, so an accent can
  sit a little off on a glyph wider than a cell.
- **Sixel** (`B3`): the bitmap protocol `chafa -f sixel`, `img2sixel` and
  `lsix` speak, inside a DCS (`ESC P <p1>;<p2>;<p3> q … ST`) — bands six pixels
  tall, `!n` runs, `$` to repaint a band, `-` for the next one, and both colour
  spaces the protocol defines (`#n;2;<h>;<l>;<s>` percentages and `#n;1` HLS).
  `P2=0` fills the cell with the terminal background and `P2=1` leaves it
  transparent, so text behind shows through. `DA1` advertises it
  (`ESC[?62;4c`), which is what makes the producers try. The picture is placed at
  the cursor and **the cursor ends below it**, scrolling the content up when the
  room is not there — `chafa` sends no newline of its own, so a terminal that
  keeps the cursor in place prints the prompt over the picture (`B3`, reported
  against `1.0.83`). The stream is kept as it arrived and decoded in the app like
  a PNG (see
  [`BENCHMARKS`](BENCHMARKS.md#images-what-one-picture-costs-b3-sizing-2026-11-13)).
  **limit:** the aspect hint `P1` is parsed and ignored (pixels are square), `P2=2`
  behaves as `P2=0` (nothing reads a framebuffer back), a picture above
  8.5 M px is refused, one taller than the whole grid is clipped at the top (it
  could not fit either way), and since sixel has no id and no `d=` at most 64
  pictures are kept — the oldest goes with its placements.
- **The kitty protocol takes the chunks the producers really send** (`1.0.86`):
  the payload of a transmission may be cut in pieces, and `chafa` base64-encodes
  **each piece on its own** — the padding therefore lands in the middle and a
  decoder that joins first refuses everything (`EINVAL:base64` on every third
  chunk, and no picture). The pieces are decoded as they arrive, so a piece that
  pads itself and a piece that does not are both taken; the spare bits of the
  last symbol are not required to be zero, which is how `chafa` writes them.
  The **last** piece may also come without its padding: `kitten icat` cuts at
  4096 characters and drops the trailing `=` (the length is in `S=`, so the `=`
  says nothing), which used to be refused as `EINVAL:base64` for every image
  whose payload needs padding — a 55 KB PNG ends in a 78-character piece.
  **limit:** the whole transmission is capped at 64 MiB, like a single one.
- **A picture lives in its cells** (`B3`, reported against `1.0.84`): `ED`/`EL`/
  `ECH` erase the cells, so they take the picture covering them — `clear` no
  longer leaves the last `chafa` behind, and a full-screen app that paints over
  without the alternate screen (`mc`) no longer keeps it in every cell it leaves
  blank. A picture in the scrollback is untouched by a screen clear (the screen
  is what was erased), and the **alternate screen is its own place**: what
  `vim`/`less`/`top` draw and clear does not reach the pictures of the screen
  they cover, and their own pictures do not follow them out.
  **limit:** the erase is all or nothing per picture — a picture that loses one
  cell loses the placement (the bytes stay until the `d=` or the keep cap).
  A picture that asks for **more cells than the grid has** (`c`/`r` bigger than
  the screen, what `chafa --size` can be forced to do) is cut to the cells there
  are, and its source rectangle with it: the part that fits is drawn, in
  proportion, and the placement never claims rows or columns beyond the grid.
- iTerm2's inline image (`OSC 1337;File=…:base64`, what `imgcat` sends): the
  bytes are placed on the cursor cell through the same direct transmission, with
  `width`/`height` in cells, in `px`, in `%` of the grid or `auto`, and **the
  cursor ends below the picture** (it takes the cells it covers). Two payload
  shapes are read: a **PNG** (`imgcat`) and an **uncompressed TIFF**, which is
  what `chafa -f iterm` writes instead (`1.0.86`). **limit:** `inline=1` only —
  `inline=0` (the protocol's default) asks the terminal to write the file into the
  cwd, and the bytes come from the child, so it is refused with a notice; a JPEG
  or a compressed TIFF is refused with a notice as well (one decoder, no zlib
  inside a container that is not a PNG); no `SetMark`/`CurrentDir` either.
- **Animation** (`B2`): `a=f` sends a rectangle of a frame — `x=`/`y=` where it
  goes, `X=1` overwrite or alpha-blend, `Y=` the canvas colour, `c=` the frame it
  is drawn over, `r=` paints into a frame that already exists, `z=` the gap before
  the next frame (negative = *gapless*, base data that is never shown) — and
  `a=a` controls it: `s=1` stop, `s=2` run and wait for more frames, `s=3` run and
  loop, `c=` jump to a frame, `z=` set a frame's gap, `v=` how many loops (`1` is
  forever). The frames are recipes in the core and the app composes them into the
  texture, so the core still has no clock and no PNG decoder; the clock is the
  app's, one wake-up per frame, and only for the tab on screen — a parked tab is
  paused, not fast-forwarded. `d=f` drops the frames and keeps the image.
  **limit:** `a=c` (composing a rectangle from one frame onto another) answers
  `EINVAL:a=c not implemented`, and `r=1` is refused because the root frame is the
  image's own bytes.
- **limit:** `pi` does not
  list the name `tortty`, so it falls back to text unless
  `PI_IMAGE_PROTOCOL=kitty` ([`EXTERNALS.md`](EXTERNALS.md)).

## Window

- Tabs: own pty and term, new tab inherits the cwd of the deepest descendant
  (else the shell's `OSC 7` when `/proc` has no answer), close with Ctrl+Shift+W
  or middle click, a dead shell closes itself, drag to reorder (half of the
  label is the threshold, and the tab stays where the hand left it), wheel
  cycles one tab per detent (the touchpad's travel accumulates, as in the
  child's wheel).
- Workspaces: a set of tabs, sidebar cards (hidden when there is only one),
  inline rename, stable numbering; closing the one in front (Super+Q) leaves
  selected the workspace that took its place — the next, or the previous when it
  was the last; the card is the number plus the name the owner
  typed over the title of the selected tab (the same `Rename` the card menu
  offers), and a workspace never renamed
  shows that title on the first row with the second one empty — clearing the name
  in the field goes back to it; the card list scrolls — the wheel over the column
  walks it one card per detent (the touchpad's travel accumulates) and entering a
  workspace by keyboard brings its card into sight; a card is dragged to another
  place in the list, and the cards move under the pointer as it passes,
  so the drop is seen before the button is released. The column shows **whole
  cards only** and keeps its last row free: a drag pushed into that strip walks
  the list down by itself, one card every 500 ms, and the rows above the first
  card walk it back up, so a place that is off the column can be reached either
  way. **limit:** the drag moves the cards, not the tabs inside them; being on a
  card is always a drop and the strip is always a scroll, so the walk up takes
  the first card's top row, and the wheel over the column keeps working with the
  button held.
- Title: OSC 0/2 as it arrives, else the cwd — `~` when the shell is at the home
  directory, `/` at the root, the last component anywhere else — else `tortty`;
  the window title mirrors the active tab. **limit:** an app that sets a title
  without pushing it (`CSI 22/23 t`) keeps the tab on it after leaving
  ([`EXTERNALS.md`](EXTERNALS.md)).
- Chrome: toast as a floating box over the bottom of the grid, centred — two
  levels (title in the foreground colour, detail pulled towards the surface and
  wrapped in up to four lines), an accent bar and border in the shade of the
  notice — the wrap and the box are `chrome::Toast`, so the box a click tests is
  the box that was drawn, **rounded corners** (one anti-aliased tile in the mask atlas, mirrored
  into the four) with a **shadow** of two veils, theme background veiled 14%
  towards the foreground, 1.5-cell gutter.
  Three shades: a receipt lives 3 s, a warning and an error 10 s, a notification
  8 s; no key takes it out, a click does; a different notice waits in a queue and
  the same words only raise the count (`×3`) without restarting the clock.
  Visual bell, audible bell. **limit:** the shadow is two flat veils, not a blur,
  and truncation counts characters, not cell width, so a wide glyph (emoji, CJK)
  can overflow the box, and the detail is cut
  at four lines.
- Notifications (every source, one drawing): `OSC 9`, `777;notify`, the final
  states of `9;4` (`error`, `paused`) and of `7501` (`done`, `blocked`, `paused`,
  `error`, `idle`), the shell's `133;D` when the command ran long
  (`shell_notify_secs`), and the BEL under `bell visual`. The foreground tab wears the
  **bell** in front of its label (two cells, from the emoji atlas when the font
  has a colour one, tinted with the label otherwise); the workspace card turns
  yellow when the tab is in another workspace, and the window asks for urgency
  only when it is not focused. The lifetime comes from **where the user is**, not
  from who sent it: watching the tab, the bell shows 800 ms and goes; elsewhere it
  stays until the tab is visited (and a visit is the only acknowledgement — a key
  is not, and there is no floor to protect). What was said lives in the tab's
  **notification list**: the last eight, newest first, one entry per
  `(source, id)` — an update replaces its own entry, which is exactly what a
  program that identifies its notifications wants — with `title` and `body` kept
  apart and `text` derived. The socket's `notifications` verb prints every tab's
  list — that is how the list is read, since it has no screen of its own.
  **limit:** inside the window + urgency only — no desktop notification daemon;
  `OSC 99`, the identified and updatable notification, is not implemented
  (`B72`); a BEL under `bell audible` is a beep and leaves no trace, and under
  `bell none` it is discarded whole.
- Progress (`OSC 9;4;state[;pct]`, the mintty/ConEmu form): `progress_show`
  decides the face — `bar` (the 2 px bar on the bottom edge of the tab label and
  of the workspace card), `title` (the percentage on the tab label **and on the
  window title**, the state word when the program gave no measure) or `none`. The
  shade is the state's, one light tone each, all opaque: light green normal,
  light yellow while the program waits for you (`blocked`/`paused`), light red
  on error, light cyan for the indeterminate band of the full width. The fill is in pixels, so 37% of a 20-cell tab is 7.4 cells
  of ink; `0` removes the record, and the bar with it. Progress never asks for
  you, so it never marks a tab. **limit:** the window only (no taskbar); the bar
  needs the tab bar, so with a single tab and `progress_show = bar` nothing is
  drawn — `title` is the face that shows in every window shape; only the root
  record drives it (a named `id` reports without a bar).
- Scrollback search (`M-D`, `Ctrl+Shift+F` or the `search` action): two chips on
  the grid's bottom row take the query — `[find: query_ ]`, with an amber `_`
  where the next character goes, and `[ 1/3 ]` beside it (red `[ no match ]`
  when the query finds nothing; the chip is absent while the query is empty).
  The surface is opaque and a hair lighter than the grid, so the bar does not
  blend into what is behind it. every match in the history and on the
  screen is highlighted (dim amber) with the current one in bright amber, and
  `Enter`/`Shift+Enter` walk them, wrapping at both ends. The view jumps so the
  match lands on the first row. **limit:** plain text, ASCII case folding, and the
  query is not kept after `Esc` closes the bar beyond that run's last one; no
  regex, no whole-word mode; matches stop at 5000 (a one-letter query in a huge
  history keeps the first 5000).
- Program status (`OSC 7501`, the Program Status Protocol v0.4): the program says
  what it is doing — `idle`/`working`/`done`/`blocked`/`paused`/`error`, with
  `kind`, `progress`, `app`, `title` and a base64 `msg` — and the terminal keeps a
  set of records per tab, one per `id` path, replaced whole by each report. A
  record without `app` inherits it from its nearest ancestor; `clear` removes the
  record and everything beneath it. Each state is one row of a fixed table — still
  in flight (`working`, `blocked`, `paused`: the bar) and asks for you (`blocked`,
  `paused`, `done`, `error`, `idle`: the notification) — and nothing depends on
  the state before, which is what makes `idle` after a long job the notice it
  should be. A report that repeats the record is not news (no event, no notice),
  so a program reporting `working` every second and a shell reporting `idle` at
  every prompt stay silent; the notice wears the state's shade (`done` green,
  `blocked`/`paused` yellow, `error` red) and names the state and the app
  (`blocked · test`). `7501;?` is answered with the states and kinds this terminal
  accepts; `OSC 9;4` feeds the root record until a real report arrives. A new
  prompt (`OSC 133;A`) or the process leaving drops **every** record of the
  terminal — deliberate deviation from the spec's "keep `done`/`error`": who
  persists now is the notification list plus the mark, and keeping the record
  would make the next command's final report read as a repetition and go
  unannounced. **limit:** every hard cap of the spec is enforced by discarding
  the whole report (4096 bytes, `msg` 2732/2048, `title` 256/192, `app` 32, `id`
  128 in 8 levels of 32, 256 records, control characters in the decoded text);
  nothing on screen shows the records themselves (the bar and the notification
  read the root), the feature-detection reply lands on the grid when the program
  is a shell that is not waiting for it, and a named `id` gets no bar.
- Prompt marks (`OSC 133`): the shell says where the prompt starts, where the
  command starts, where the output starts and how the command ended (with the
  status); the marks live on absolute lines, so scrolling and output do not move
  them, and `Ctrl+Shift+Up/Down` puts the previous/next prompt on the first row
  (at the ends the key does nothing). The list is capped at 512 marks per tab and
  a mark whose line fell out of the scrollback is dropped. **limit:** nothing
  else consumes them yet — the command's output as a range, the command in the
  tab label and the redraw are `B39`, and no shell ships a script that emits them
  (the snippet is the reader's).
- Shortcuts are a **table** (`map`/`unmap` in the config, `M-H`): `map
  ctrl+shift+y new_tab` re-binds, `unmap ctrl+shift+w` frees the chord so the
  program sees the key again. Twenty actions (copy, paste, tab and workspace
  open/close, prompt walk, config, the four scroll moves, search,
  `select_tab`/`select_workspace` with a number, the workspace walk and carry).
  **The defaults are `map` lines in four files**, the same syntax you write: the
  common base holds the chords that mean the same thing everywhere
  (`Ctrl+Shift+F`, `Ctrl+Shift+Up/Down`, `Ctrl+Shift+F2`,
  `Shift+PgUp/PgDn/Home/End`, `Ctrl+Shift+1`…`0` for the workspace), and the
  template of your system holds the rest — `Ctrl+Shift+C/V/T/W/N/Q` here against
  `⌘C/V/T/W/N/Q` on macOS, `Alt+1`…`0` against `⌘1`…`0` for the tabs, the walk on
  `Super+Up/Down`, `Alt+Up/Down` or `⌘⌥↑/↓`, and the font family. Deleting a line
  frees the key (no `unmap` needed), and the auto-reload reparses from that base,
  so an edit takes effect without a restart. **limit:** chord + action only — no
  sequences, no per-program tables — and a digit under `Ctrl`/`Alt`/`Super` is
  read from the physical key, so a non-QWERTY layout reaches the same number.
- Geometry: 980×640 without state, size restored, maximized tracked separately
  from the normal size, invalid geometry rejected; the workspaces and tabs of the
  last session come back with the workspace and tab that were in front, each
  shell in the directory it had. **limit:** on Wayland the position is not
  stored, because the client does not choose it.
- `restore_state`: `none` (one tab, nothing of the session written — not even
  the geometry, since only the owner writes and `none` never owns), `layout`
  (the default: what the bullet above describes) or `content` (the layout **and**
  the screen and the history of every tab, fed back before the shell exists,
  closed by a marker line that separates the photograph from this run — a
  decoration, drawn on the screen and **never** saved, so a tab reopened five
  times does not come back with five markers; the
  window closes with a hangup and the ptys are drained for a quarter of a second
  before the snapshot, so the child's last words travel with it; the snapshot
  carries the **whole cell**, the codepoints of a grapheme cluster included, so a
  family emoji comes back composed instead of as its first face).
  **limit:** a photograph, not the process — a new shell starts in the same
  directory and the previous programs are gone.
- One owner of the state: the window that opens the session holds a `flock` on
  `state.lock`, and that is the one that restores, applies the remembered
  geometry and writes. A window that finds the lock taken is a **scratch**
  window: one tab in the directory it was called from, the 980×640 default (a
  window exactly over the first one is the one case where restoring the position
  hurts) and it never writes. `restore_state = none`, `--cmd`, `--cols` and
  `--rows` never even try the lock, so a probe or `none` leaves the state of the
  hot session untouched.
- Lifecycle: the window closes when the shell ends; `SIGUSR1` dumps,
  `SIGTERM` dumps and exits cleanly, `--quit-after` for automation.
- A file dropped on the window becomes its **path**, quoted for the shell and
  written into the child like a paste (bracketed when the child asked for it).
  **limit:** nothing else happens — no new tab, no `cd`: the terminal does not know
  what is running, so a program that is not a shell still gets a path it can read.

## Configuration

- A kitty-format file on top of the built-in defaults, one option per line, last
  key wins, an error is reported per line and does not take the file down.
- The keys are the ones in [`CONFIG.md`](CONFIG.md) — the shell, the font, the
  cursor, scrollback, padding, bell, clipboard, `restore_state`, the diagnostic
  log and the colours.
- Hot reload with a debounce; **any** error on reload keeps the configuration
  already in effect, and the toast says both things: `Configuration error` over
  `kept the previous one — line N: …` (with the count of the others). A clean
  reload answers on screen with `Configuration applied` and the family and size
  that are in force — proof that this file is the one being read, not only when
  the previous save was refused. At startup there is nothing to keep, so the
  detail is the error alone.
- Ctrl+Shift+F2 opens that file in the editor, in a tab **next to the current
  one**, which comes back to it when the editor exits: `VISUAL`, then `EDITOR`,
  then `vim`. The directory is created first, because the editor only writes the
  file on save; a session with no path (no `HOME`) answers with a toast instead
  of a tab.
- CLI: `-c/--config`, `--cmd`, `--cols`/`--rows`, `--font-family`, `--font-size`,
  `--scrollback`, `--dpi`, `--no-vsync`, `--debug-socket`, `--debug-log`, `--prof`,
  `--quit-after`, `--dump-to`, `--selftest`, `--version`.

## Debug socket and instrumentation

- Unix socket, JSON per line, several requests per connection: `state`, `text`,
  `write`, `drop`, `sel`, `resize`, `tab`, `keys`, `prof`, `osc`, `log`,
  `notifications`, `quit`
  ([`DEBUG.md`](DEBUG.md) has the shape of each).
- `--debug-socket auto` opens `$XDG_RUNTIME_DIR/tortty-<pid>.sock`; the socket is
  `0600` and checks `SO_PEERCRED`, and it never deletes a file that is not its
  own. The socket itself is the `platform` seam (one capability, not a `#[cfg]`
  in the protocol code); **limit:** only the Linux implementation exists (a named
  pipe is `M-X`), and the self test asks the seam before checking a socket's
  permissions, so a system without one reports no failure.
- `--dump-to` and `SIGUSR1` give the same view with no socket.
- One instance per PID file: `tools/devkill.sh` kills that PID, checking
  `/proc/<pid>/exe`.

## Platform and distribution

- Linux X11 with Vulkan or GL, plus the CPU adapter fallback.
  **limit:** Wayland compiles and is not exercised (`M-AB`).
- Windows 10/11 opens a window, draws and runs ConPTY, measured on 2026-11-22 in
  the Windows 10 21H2 guest (`M-X`). **limit:** the port is minimal — no debug
  socket, no last words on close, no child cwd for the title, no macOS build
  (`M-Y`), and nothing ships for Windows yet: no package, no runner, no signing
  (`M-Z`). Details in [`PORTABILITY.md`](PORTABILITY.md).
- Installer: the `tortty` command, menu entry, icon, `--prefix`, `--check`,
  `--uninstall`. **limit:** `--from-release` needs an asset published in GitHub
  Releases; there is none yet, so `./install.sh` is the path today.

## Tests and verification

- Headless core check suite, run by `--selftest` without a window, covering
  parser, config/CLI, keyboard/mouse, socket, reflow/selection/anchor.
- Visual gate on its own Xvfb: draws, captures and compares pixels
  (`tools/render-check.sh`), so a drawing regression fails without an eye.
- Reproducible benchmark with the grid locked by the socket
  (`tools/benchrun.py`) and the ladder of `tools/bench.sh`.
