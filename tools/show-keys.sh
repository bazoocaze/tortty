#!/bin/sh
# show-keys.sh — run the witness: what the window system hands to a key press.
#
# It is a crate of its own (tools/show-keys) on purpose, and it depends on
# `winit` and nothing else: no `tortty-core`, no config, no chord table. What it
# prints is what the terminal receives, so a disagreement between this and the
# terminal is a fact about the terminal — the tool has no opinion to defend.
#
#   tools/show-keys.sh                  # window opens, close it to stop
#   tools/show-keys.sh --quit-after 5   # automation: no window close needed
#   tools/show-keys.sh > keys.txt       # the comparison table, one line per key
#
# Debug profile, like every probe that is not about speed (AGENTS.md): seconds
# against the ~50 s of a release build.
set -eu
HERE=$(cd "$(dirname "$0")/.." && pwd)
cd "$HERE"
exec cargo run --quiet -p show-keys -- "$@"
