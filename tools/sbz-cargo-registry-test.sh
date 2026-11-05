#!/bin/sh
# sandbox + cargo: the Rust workflow needs to write into ~/.cargo/registry.
#
# Run it ON THE HOST (this script calls `sandbox`; it does not need to be inside it).
#   sh tools/sandbox-cargo-registry-test.sh          # tests
#   sh tools/sandbox-cargo-registry-test.sh -v       # shows the bwrap arguments
#   sh tools/sandbox-cargo-registry-test.sh --clean  # deletes the scratch
#
# The scratch lives next to the script (persistent), not in /tmp: cargo's `target/`
# is expensive to rebuild and /tmp is usually tmpfs. Change it with SCRATCH_DIR=...
#
# Exit: 0 = sandbox ok for cargo   1 = a fix is missing (the reason is printed)

SANDBOX=${SANDBOX:-sandbox}
HERE=$(cd "$(dirname "$0")" && pwd)
SCRATCH=${SCRATCH_DIR:-$HERE/.sandbox-cargo-scratch}
VERB=""
case "$1" in
    -v) VERB="-v" ;;
    --clean) rm -rf "$SCRATCH" && echo "scratch deleted: $SCRATCH" && exit 0 ;;
esac

failures=0
ok() { printf '  ok      %s\n' "$1"; }
not() { printf '  FAILURE   %s\n' "$1"; [ -n "$2" ] && printf '          %s\n' "$2"; failures=$((failures + 1)); }
# cargo prints a cache warning and only then the fatal error: take the fatal one
reason() {
    printf '%s\n' "$1" | grep -m1 'error: failed to open' ||
        printf '%s\n' "$1" | grep -m1 'Read-only file system'
}

# the test is only worth anything with the crate outside the local cache: if it is there,
# "no grant" passes for free and the probe lies
crate_not_cached() {
    for c in tinyjson humansize textwrap difference heck arrayvec bit-set colored; do
        if ! ls "$HOME"/.cargo/registry/cache/*/"$c"-*.crate >/dev/null 2>&1; then
            printf '%s' "$c"
            return 0
        fi
    done
    printf 'rustybuzz' # everything cached: falls back to the old behavior
}
CRATE=$(crate_not_cached)

echo "== A. how \$HOME is mounted on the host (the cause when --bind does not stick)"
mount | grep -E " on $HOME " || echo "  (no mount at $HOME: not this case)"

# hand-written crate, with an empty [workspace]: `cargo new` inside the repo
# would add the member to the root Cargo.toml and contaminate the workspace
crate() {
    mkdir -p "$SCRATCH/$1/src"
    printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n' "$1" \
        >"$SCRATCH/$1/Cargo.toml"
    echo 'fn main() {}' >"$SCRATCH/$1/src/main.rs"
}

echo "== B. no grant: reproduces the defect"
rm -rf "$SCRATCH"
mkdir -p "$SCRATCH" || { echo "no write access to $SCRATCH"; exit 1; }
crate app
out=$($SANDBOX $VERB -w "$SCRATCH" -- sh -c \
    "cd $SCRATCH/app && cargo add -q $CRATE && cargo build" 2>&1)
if [ $? -eq 0 ]; then
    printf '  notice   %s\n' "build passed WITHOUT grant: warm cache or global grant (inconclusive reproduction)"
else
    ok "no grant fails, as expected ($(reason "$out" | cut -c1-60)...)"
fi

echo "== C. with -w ~/.cargo/registry: the minimum the Rust workflow asks for"
if $SANDBOX -w "$SCRATCH" -w ~/.cargo/registry -- \
    sh -c 'touch ~/.cargo/registry/.probe && rm -f ~/.cargo/registry/.probe' 2>/dev/null; then
    ok "~/.cargo/registry writable inside the sandbox with -w"
else
    not "~/.cargo/registry is still read-only with -w" \
        "if \$HOME is mounted ro on the host, the bind inherits ro: no sandbox policy fixes it"
fi

echo "== D. the cargo lock (~/.cargo/.package-cache) lives OUTSIDE registry/"
if $SANDBOX -w "$SCRATCH" -w ~/.cargo/registry -- \
    sh -c 'touch ~/.cargo/.package-cache.probe && rm -f ~/.cargo/.package-cache.probe' 2>/dev/null; then
    ok "~/.cargo/.package-cache writable"
else
    printf '  notice   %s\n' "~/.cargo/.package-cache not writable (cargo lock; only matters if the build complains)"
fi

echo "== E. is the bind emitted by sandbox? (policy ok, mount not)"
if printf '%s' "$($SANDBOX -v -w ~/.cargo/registry -- true 2>&1)" | grep -q "$HOME/.cargo/registry"; then
    ok "sandbox emits --bind $HOME/.cargo/registry"
else
    not "sandbox does not emit the ~/.cargo/registry bind" "check the cap write(\"~/.cargo/registry\")"
fi

echo "== F. real build with the grant (the test that has to pass)"
crate app2
out=$($SANDBOX $VERB -w "$SCRATCH" -w ~/.cargo/registry -- sh -c \
    "cd $SCRATCH/app2 && cargo add -q $CRATE && cargo build" 2>&1)
if [ $? -eq 0 ]; then
    ok "cargo build with a new dependency passed"
else
    not "cargo build with a new dependency failed" "$(reason "$out")"
    # the scratch stays for inspection: Cargo.toml, build target, etc.
    echo "          scratch preserved at $SCRATCH"
fi

echo "== G. @dev profile (where the sandbox fix usually lives)"
crate app3
out=$($SANDBOX $VERB @dev -w "$SCRATCH" -- sh -c \
    "cd $SCRATCH/app3 && cargo add -q $CRATE && cargo build" 2>&1)
if [ $? -eq 0 ]; then
    ok "cargo build passed with @dev (profile resolved)"
else
    not "cargo build failed with @dev" "$(reason "$out")"
fi

echo
if [ "$failures" -eq 0 ]; then
    echo "SANDBOX OK FOR CARGO"
    echo "suggested cap in the profile:  write(\"~/.cargo/registry\")"
    exit 0
fi
echo "NEEDS A FIX ($failures point(s))"
exit 1
