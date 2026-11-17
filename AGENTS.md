# tortty (TorTTY — Torbie TTY)

Fast, GPU based terminal. VT terminal in Rust: own pty + VT parser + grid,
drawing on the GPU with `wgpu`.

## Documentation

Each fact lives in **one place only**. All of them are written for agent
reading; only README.md is for humans.

- `AGENTS.md` (this one) — conventions, architecture, tools.
- `docs/MIGRATION.md` — **the record of the 2026-11 migration**: the inventory it
  came from and the order it was done in, closed at 195/195. A photograph of the
  past: it is not read as state, not a queue, and it is not edited.
- `docs/FEATURES.md` — **the index of what exists today**, by area: one line per
  capability, and in the same entry the limit of that capability — a `BACKLOG`
  id when it is queued, a `LIMITATIONS` reason when it is accepted. It owns no
  fact: it points, it does not restate.
- `docs/BACKLOG.md` — the queue: the work not started or in progress (id
  `B<number>`, the old `M-*`, each with trigger and cost) and the rules of the
  queue.
- `docs/BENCHMARKS.md` — **the only place with numbers** for throughput, parse,
  drawing and footprint (with date, governor and condition). The other docs cite
  the fact in one sentence and link.
- `docs/CONFIG.md` — the configuration file, key by key. A test enforces that
  every key of `DEFAULT_CONF` appears there (the doc does not age).
- `docs/INSTALL.md` — install/uninstall the `tortty` command (and what lands in
  each prefix). It is the only place that describes the installer.
- `docs/DEBUG.md` — operational manual: how to start/kill without dropping the
  session, probes, the debug socket contract and the pitfalls
  (winit/wgpu/pty/parser). **Read before running any graphical tool.**
- `docs/LIMITATIONS.md` — the limitations of the project.
- `docs/EXTERNALS.md` — bugs of other software that were measured and are not
  ours, with the proof and the workaround.
- `docs/HISTORY.md` — dated record of what was done (narrative).
- `README.md` — the human view: run, measure, structure. The project owner must
  approve changes before committing.

### A rule is not created by itself

A fact, a measurement and code can be recorded directly. **A norm cannot**:
before adding or changing a rule in any doc — queue entry criterion, state
definition, "what goes where", record format, a new section someone has to feed
— propose the text and wait for the project owner's ok. It holds for this file
too.

## Architecture

Two-crate workspace. The boundary is rigid and it is the reason the core is
testable without a display:

- `crates/core` — pure emulation: cell, palette, DEC charset, grid, VT
  parser, scrollback. **It knows no window, pty, GPU, clock, thread or
  configuration.** No `wgpu`, no `winit`, no system dependency. If it needs
  something from the world, the caller passes it as a parameter or reads the
  result of an event returned by `feed`.
- `crates/app` — the app: `pty` (portable-pty), `app` (winit + event loop),
  `render`/`glyph`/`atlas` (wgpu + swash), `keys`, `debugsock`, `prof`, `cli`.
- `tools/` — `devrun.sh`/`devkill.sh` (PID file), `benchrun.py`, `bench.sh`,
  `doomfire.py` (the fixed load, byte for byte, so two runs are comparable).

Flow of a byte: pty reader thread → `Term::feed` (core) → marks damage on the
line → `request_redraw` → `RedrawRequested` builds the instance vector →
`wgpu` draws (background, glyph, underline, cursor: one pipeline, one draw call
per category, in order).

## Code conventions

- `platform` names the operating system; the rest of the code names the
  **capability**. `crates/app/src/platform/` is the only module allowed to name a
  system — syscalls, paths, signals, shell quoting, windowing details — and the app
  asks it for `try_lock`, `hangup`, `shell_quote`. What the implementation does not
  have comes back as `Option`/`Err` with the reason, never as a silent `#[cfg]` in
  the feature code. Policy per system (chords, default font, shell name as data) is
  **not** there: it lives in `crates/core/defaults/`. The ruler is
  `tools/platform-gate.sh`, a step of `tools/quality-check.sh`.

- Version: bump the patch in `Cargo.toml` (root, `workspace.package`) in **every
  commit that changes code** (`crates/`, `tools/`, `install.sh`) — closing an item
  of `docs/BACKLOG.md` is one of those cases, not a separate one. The bump rides
  in the commit that earns it. A commit that only touches docs or tests does not
  bump. It is the single source: both crates
  inherit it (`version.workspace = true`) and `--version` shows it
  (`env!("CARGO_PKG_VERSION")`, compiled at build).
- The defaults are four files in `crates/core/defaults/`: `tortty.conf` (the
  common base) and `tortty-{linux,macos,windows}.conf`. A line lives in the base
  **only if the same chord is correct, primary and free of a system collision on
  all three**; the rest goes into the OS template, which only adds and never
  frees a line of the base. A test parses all four and requires every `Action`
  to be reachable on each system.
- Comment and message explain **why**; no comment narrates the obvious.
- When a decision has no precedent in the code, think of what the user of a
  terminal expects.
- Use Conventional Commits (`<type>(<scope>): <description>`; types: feat, fix, chore, docs, style,
  refactor, perf, test, build, ci, revert.
- Commit message in English, concise, three lines maximum.
- File names and symbols (variable, field, etc) in English.
- For what is already in Portuguese, the fix is on demand, during the edit, in
  the commit that already touches the file.
- No `unsafe` in the core. In the app, only where the `wgpu`/`winit` API
  requires it.
- Measurement before optimizing: `parsebench` for the parser,
  `tools/benchrun.py` for the app. A change that touches the hot path comes with
  the before and after number in the same commit.
- Every sequence implemented in the parser lands with a test in `tortty-core`
  (`#[test]` table at the end of the module, test name = what it guarantees).

## Quality: one command only

```sh
./tools/quality-check.sh              # every step, in order
./tools/quality-check.sh --fast       # without the build (it is the pre-commit)
./tools/quality-check.sh --only test  # named steps: CI runs them in parallel jobs
./tools/quality-check.sh --fix        # applies `cargo fmt` and `clippy --fix` before checking
```

The **same** script runs in the git hook (`.githooks/pre-commit`, enabled with
`git config core.hooksPath .githooks`) and in CI (`.github/workflows/ci.yml`).
A hook and a CI that disagree are good for nothing — and the compiler is part of
the ruler: `rust-toolchain.toml` pins it, so a newer `stable` cannot turn the hook
green over a red CI. `--only` picks steps; it never defines them.

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

- `./tools/devrun.sh --cmd 'mc' --debug-socket /tmp/tt.sock` — starts in
  background with a PID file.
- **`target/debug/tortty` for any test that is not about speed.**
  `cargo build -p tortty` takes seconds against ~50 s of `cargo build
  --release`, and nothing in the socket/`xdotool` probes depends on the profile.
  `bench.sh` is the one that needs `release`; `render-check.sh` uses debug by
  default (`TORTTY_BIN` overrides).
- `./tools/devkill.sh` — kills **only** the registered PID, checking
  `/proc/<pid>/exe`. `pkill -f tortty` kills the shell that called it (the
  pattern is in its command line); `pkill -x` was worse still when two builds
  shared the same binary name (it brought down the owner's session). It has
  happened; do not use it.
- `./tools/render-check.sh` — rendering verification **without showing up on the
  owner's screen**: its own Xvfb, fixed theme, capture and pixel comparison
  (`tools/pixel-probe.py`). It exits 0/1, so it works as a gate.
  `./tools/pixel-probe.py shot.png --diff a.png b.png` says how many pixels
  changed, and where.
- `./tools/bench.sh` — the ladder in both grids, against the reference build.
- `cargo run --release -p tortty-core --bin parsebench -- --file /tmp/dmesg.txt`
  — real-load parsing, no display.
