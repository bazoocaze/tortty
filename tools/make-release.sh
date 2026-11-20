#!/usr/bin/env bash
# make-release.sh — publishes the tag that makes a release.
#
# The tag is the whole input: `release.yml` builds both Linux packages for `v<version>`
# and publishes them with `SHA256SUMS`. So this script builds nothing — its job is to
# make the tag *safe to push*. Each check below is a way a release goes wrong quietly:
#
#   dirty tree        the tag names a commit, so anything uncommitted ships excluded
#                     while the release notes describe it
#   HEAD not pushed   nobody can see the commit the tag names, and CI never ran on it
#   CI red or running `release.yml` does not depend on `ci.yml`: without this check a
#                     red commit gets a green tag and a published broken package
#   tag already there force-updating a published tag swaps the package under whoever
#                     already downloaded it, and the `SHA256SUMS` stop matching
#   version           the tag is built from `Cargo.toml`, so a release of code whose
#                     bump was forgotten is refused here instead of in the middle of
#                     CI, after the tag is public
#
# Usage:
#   ./tools/make-release.sh            checks, tags v<version>, pushes it
#   ./tools/make-release.sh --dry-run  runs every check, stops before the tag
#   ./tools/make-release.sh --skip-ci  trusts the commit without asking GitHub
#
# What it does not do, on purpose: it does not bump the version (that rides in the
# commit that changes the code — AGENTS.md) and it does not install anything.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)
cd "$ROOT"

DRY=0
CI_CHECK=1
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1; shift ;;
        --skip-ci) CI_CHECK=0; shift ;;
        -h|--help) awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "$0"; exit 0 ;;
        *) printf 'invalid option: %s (use -h)\n' "$1" >&2; exit 2 ;;
    esac
done

say() { printf '%s\n' "$*"; }

VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
[ -n "$VERSION" ] || { say "could not find the version in Cargo.toml" >&2; exit 1; }
TAG="v$VERSION"
HEAD=$(git rev-parse HEAD)
SHORT=$(git rev-parse --short HEAD)

fail=0
ok()  { printf '  %-7s %s\n' ok "$*"; }
bad() { printf '  %-7s %s\n' FAILED "$*"; fail=1; }

say "== releasing $TAG from $SHORT"

# -- the tag has to name a commit that is already published and already green ----
if [ -z "$(git status --porcelain)" ]; then
    ok "the working tree is clean"
else
    bad "the working tree has uncommitted changes: a tag names a commit, not a directory"
fi

branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$branch" = main ]; then
    ok "on main"
else
    bad "on '$branch', not main"
fi

git fetch --quiet origin main 2>/dev/null || true
remote=$(git rev-parse origin/main 2>/dev/null || true)
if [ -n "$remote" ] && [ "$remote" = "$HEAD" ]; then
    ok "HEAD is origin/main"
else
    bad "HEAD ($SHORT) is not origin/main (${remote:0:7}): push first, or the tag names a commit nobody can see"
fi

if [ -f .github/workflows/release.yml ]; then
    ok "the tag triggers .github/workflows/release.yml"
else
    bad "no .github/workflows/release.yml: the tag would publish nothing"
fi

# -- the tag must not exist, here or on the remote ------------------------------
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
    bad "tag $TAG already exists here (bump the version in Cargo.toml for a new release)"
else
    ok "tag $TAG is free here"
fi

if [ -n "$(git ls-remote --tags origin "refs/tags/$TAG" 2>/dev/null)" ]; then
    bad "tag $TAG already exists on origin (a published tag is not moved)"
else
    ok "tag $TAG is free on origin"
fi

# -- the CI verdict for exactly this commit -------------------------------------
if [ "$CI_CHECK" = 0 ]; then
    say "  skipped the CI verdict (--skip-ci)"
elif ! command -v gh >/dev/null 2>&1; then
    bad "gh is not installed, so the CI verdict cannot be read (--skip-ci to release blind)"
else
    verdict=$(gh run list --commit "$HEAD" --workflow=ci.yml --limit 1 \
        --json status,conclusion,url --jq '.[0] | "\(.status) \(.conclusion) \(.url)"' 2>/dev/null || true)
    case "$verdict" in
        "completed success "*)
            ok "CI is green for $SHORT (${verdict##* })"
            ;;
        "")
            bad "no ci.yml run for $SHORT: push it and wait for CI, or --skip-ci to release blind"
            ;;
        *)
            bad "CI for $SHORT is not green (${verdict% *})"
            ;;
    esac
fi

if [ "$fail" != 0 ]; then
    say ""
    say "not releasing: everything above has to pass first"
    exit 1
fi

say ""
if [ "$DRY" = 1 ]; then
    say "would create: git tag -a $TAG -m \"tortty $VERSION\""
    say "would push:   git push origin $TAG"
    exit 0
fi

git tag -a "$TAG" -m "tortty $VERSION"
say "== tag $TAG created on $SHORT"
if ! git push origin "$TAG"; then
    say "" >&2
    say "the push failed; the tag exists locally only. Remove it with: git tag -d $TAG" >&2
    exit 1
fi

say "== pushed: release.yml is building x86_64 and aarch64"
say "   watch:  gh run list --workflow=release.yml --limit 1"
say "   verify: gh release view $TAG --json assets --jq '.assets[].name'"
