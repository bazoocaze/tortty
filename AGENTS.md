# bzmux (Rust)

VT terminal in Rust: own pty + VT parser + grid, drawing on the GPU with
`wgpu`. It is the continuation of the Python/GTK bzmux
(`~/tortty-python`). The migration is **closed** (2026-11-02):
[`docs/MIGRATION.md`](docs/MIGRATION.md) is its record, one line per Python
feature with the final state and the `file:line` reference.

Coexistence rule between the two repositories: Python **was** the observable
specification while the migration ran; today it is historical — it serves to
answer "how the app behaved" and to judge whether something here is a
regression. What is still missing here **is not parity**: it is new activity, and
it lives in [`docs/BACKLOG.md`](docs/BACKLOG.md). Only what is a decision not to
do it enters [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) (what it lists
itself); pending migration work no longer exists.

## Documentation

Each fact lives in **one place only**. All of them are written for agent
reading; only README.md is for humans.

- `AGENTS.md` (this one) — conventions, architecture, tools.
- `TASK-bzmux-in-rust.md` — dated record of phase 1 (the feasibility
  experiment): questions, decisions, what was measured that day. It is not
  current reading; what still holds was promoted to the docs below.
- `docs/MIGRATION.md` — **the migration record**: complete inventory of the
  Python bzmux features, the final state of each one here
  (`done`/`partial`/`missing`/`no`) and the order in which it was done. Closed at
  195/195; **it is not a queue**. **Migration scope only**: an idea Python does
  not have does not go in here.
- `docs/BACKLOG.md` — the queue: activity Python does not have (id `B<number>`,
  the old `M-*`, each with trigger and cost) and the rules of the queue. If
  Python does not do it, it is not parity: it is backlog.
- `docs/BENCHMARKS.md` — **the only place with numbers** for throughput, parse,
  drawing and footprint (with date, governor and condition). The other docs cite
  the fact in one sentence and link.
- `docs/CONFIG.md` — the configuration file, key by key. A test enforces that
  every key of `DEFAULT_CONF` appears there (the doc does not age).
- `docs/INSTALL.md` — install/uninstall the `bzmux` command (and what lands in
  each prefix). It is the only place that describes the installer.
- `docs/DEBUG.md` — operational manual: how to start/kill without dropping the
  session, probes, the debug socket contract and the pitfalls
  (winit/wgpu/pty/parser). **Read before running any graphical tool.**
- `docs/LIMITATIONS.md` — what is not done and why (a decision or an accepted
  limit, not a queue).
- `docs/HISTORY.md` — dated record of what was done (narrative). The history of
  the Python phase lives in the other repo, in `docs/HISTORY.md` there.
- `README.md` — the human view: run, measure, structure.

### A rule is not created by itself

A fact, a measurement and code can be recorded directly. **A norm cannot**:
before adding or changing a rule in any doc — queue entry criterion, state
definition, "what goes where", record format, a new section someone has to feed
— propose the text and wait for the project owner's ok. It holds for this file
too.

## Architecture

Two-crate workspace. The boundary is rigid and it is the reason the core is
testable without a display:

- `crates/bzmux-core` — pure emulation: cell, palette, DEC charset, grid, VT
  parser, scrollback. **It knows no window, pty, GPU, clock, thread or
  configuration.** No `wgpu`, no `winit`, no system dependency. If it needs
  something from the world, the caller passes it as a parameter or reads the
  result of an event returned by `feed`.
- `crates/bzmux` — the app: `pty` (portable-pty), `app` (winit + event loop),
  `render`/`glyph`/`atlas` (wgpu + swash), `keys`, `debugsock`, `prof`, `cli`.
- `tools/` — `devrun.sh`/`devkill.sh` (PID file), `benchrun.py`, `bench.sh`,
  `doomfire.py` (byte for byte identical to the one in the Python repo: it is
  the same load).

Flow of a byte: pty reader thread → `Term::feed` (core) → marks damage on the
line → `request_redraw` → `RedrawRequested` builds the instance vector →
`wgpu` draws (background, glyph, underline, cursor: one pipeline, one draw call
per category, in order).

## Code conventions

- Version: bump the patch in `Cargo.toml` (root, `workspace.package`) when you
  **close an item** of `docs/MIGRATION.md` or `docs/BACKLOG.md` — the commit that
  closes the item carries the bump, not every commit. A docs, test or one-off
  adjustment commit does not bump. It is the single source: both crates inherit
  it (`version.workspace = true`) and `--version` shows it
  (`env!("CARGO_PKG_VERSION")`, compiled at build).
- Comment and message explain **why**; no comment narrates the obvious.
- File names and symbols (variable, field, etc) in English.
- For what is already in Portuguese, the fix is on demand, during the edit, in
  the commit that already touches the file.
- No `unsafe` in the core. In the app, only where the `wgpu`/`winit` API
  requires it.
- Measurement before optimizing: `parsebench` for the parser,
  `tools/benchrun.py` for the app. A change that touches the hot path comes with
  the before and after number in the same commit.
- Every sequence implemented in the parser lands with a test in `bzmux-core`
  (`#[test]` table at the end of the module, test name = what it guarantees).

## Quality: one command only

```sh
./tools/check.sh          # fmt --check + clippy -D warnings + test + build
./tools/check.sh --fast   # without the build (it is the pre-commit)
./tools/check.sh --fix    # applies `cargo fmt` and `clippy --fix` before checking
```

The **same** script runs in the git hook (`.githooks/pre-commit`, enabled with
`git config core.hooksPath .githooks`) and in CI (`.github/workflows/ci.yml`).
A hook and a CI that disagree are good for nothing.

- **`cargo fmt`** is the formatting: `rustfmt.toml`, width 100, stable options
  only. Do not argue style in review — `cargo fmt` decides.
- **`cargo clippy`** runs with `-D warnings` and turns on `all`, `correctness`,
  `suspicious` and `pedantic`. The lints that are off are in `Cargo.toml` with
  the reason written (cell/pixel cast, `unsafe_code = deny` with a local allow).
- **A lint `allow` is local and commented**, never at the top of the crate: if
  the rule does not hold for a function, the `#[allow]` lives in that function
  and says why.
- A commit only passes with the ruler green.

## Tools

- `./tools/devrun.sh --cmd 'mc' --debug-socket /tmp/bz.sock` — starts in
  background with a PID file.
- `./tools/devkill.sh` — kills **only** the registered PID, checking
  `/proc/<pid>/exe`. `pkill -f bzmux` kills the shell that called it (the
  pattern is in its command line) and `pkill -x bzmux` kills the pipx `bzmux`,
  which has the same name as the binary. It has happened; do not use it.
- `./tools/visual.sh` — visual verification **without showing up on the owner's
  screen**: its own Xvfb, fixed theme, capture and pixel comparison
  (`tools/pixel.py`). It exits 0/1, so it works as a gate.
  `./tools/pixel.py shot.png --diff a.png b.png` says how many pixels changed
  and where.
- `./tools/bench.sh` — Rust x Python ladder in both grids.
- `cargo run --release -p bzmux-core --bin parsebench -- --file /tmp/dmesg.txt`
  — real-load parsing, no display.
