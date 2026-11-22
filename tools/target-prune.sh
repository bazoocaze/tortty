#!/bin/sh
# target-prune.sh — the disk cargo never gives back.
#
# The version is part of the artifact identity hash, so the build that follows a
# release bump writes to a new path under `target/` and the previous one stays:
# nothing in cargo collects it. Measured here on 2026-11-22: a bump left a 142.7 MB
# copy of the binary plus its incremental session behind (+225 MB), and 124 of them
# reached 52 GB. The dependencies do not grow — `Cargo.lock` pins their identity, so
# their path never changes — and they are what a cold build pays for; this script
# never touches them. `-p` names our crates only, which is why the next build costs
# seconds instead of a minute.
#
#   tools/target-prune.sh              # prunes when target/ is above 4 GB
#   tools/target-prune.sh --ceiling 8  # a different ceiling, in GB
#   tools/target-prune.sh --dry-run    # says what it would do
#
# It is housekeeping, not a check: it is not a step of quality-check.sh, and it exits
# 0 whether or not it had to prune.
set -eu

HERE=$(cd "$(dirname "$0")/.." && pwd)
cd "$HERE"

CEILING_GB=4
DRY=0
while [ $# -gt 0 ]; do
    case $1 in
        --ceiling)
            CEILING_GB=${2:-}
            case $CEILING_GB in
                '' | *[!0-9]*) echo "target-prune: --ceiling needs a whole number of GB" >&2; exit 2 ;;
            esac
            shift 2
            ;;
        --dry-run) DRY=1; shift ;;
        -h | --help) sed -n '/^#   tools\/target-prune.sh/p' "$0" | sed 's/^# \{1,\}//'; exit 0 ;;
        *) echo "target-prune: unknown option: $1" >&2; exit 2 ;;
    esac
done

TARGET=${CARGO_TARGET_DIR:-$HERE/target}
if [ ! -d "$TARGET" ]; then
    echo "target-prune: no $TARGET; nothing to prune"
    exit 0
fi

# The workspace members come from their manifests: a hardcoded pair would rot the
# day a third crate is added, and `cargo clean -p` on a name that does not exist is
# an error, so the list has to be the real one. Only the `[package]` block is read:
# a `[[bin]]` further down has a `name` of its own and is cleaned with its package.
pkgs=$(for f in crates/*/Cargo.toml; do
    awk -F'"' '/^\[package\]/{p=1; next} /^\[/{p=0} p && /^name[ ]*=/{print $2; exit}' "$f"
done)
if [ -z "$pkgs" ]; then
    echo "target-prune: no crate manifest under crates/; refusing to guess" >&2
    exit 1
fi

set --
for p in $pkgs; do set -- "$@" -p "$p"; done

before=$(du -sm "$TARGET" | cut -f1)
ceiling=$((CEILING_GB * 1024))
if [ "$before" -le "$ceiling" ]; then
    echo "target-prune: ${before} MB, under the ${ceiling} MB ceiling; nothing to do"
    exit 0
fi

echo "target-prune: ${before} MB is above ${ceiling} MB; cleaning $(echo "$pkgs" | tr '\n' ' ')"
if [ "$DRY" = 1 ]; then
    echo "target-prune: would run: cargo clean $*"
    exit 0
fi

cargo clean "$@" >/dev/null
after=$(du -sm "$TARGET" | cut -f1)
echo "target-prune: ${before} MB -> ${after} MB (the next build of our crates costs seconds)"
if [ "$after" -gt "$ceiling" ]; then
    echo "target-prune: ${after} MB is still above the ceiling; the rest is the dependency tree," \
        "which does not grow. \`cargo clean\` would take it at the cost of a cold build." >&2
fi
exit 0
