#!/usr/bin/env bash
# Point apt at the fastest mirror, so a run does not pay minutes for megabytes.
#
# Why this exists: the runner ships with `azure.archive.ubuntu.com`, and that host
# collapses often enough that two attempts of the same commit paid 55 s and 114 s
# for 2.1 MB of dev headers — 30 kB/s, a quarter of the job, for seven packages
# whose names never change. The mirror, not the work, is what makes the job's cost
# unstable (306 s vs 455 s for the same tree).
#
# Why here and not in `tools/`: this is CI-only. It is not installed, not shipped
# in the release and not a dev command; it exists to make `ci.yml` cheap, so it
# lives beside `ci.yml`.
#
# Why not a third-party action: that one needs sudo and rewrites apt's sources, so
# the trust surface should be this file, in our diff, reviewed as code.
#
# The candidate list is Ubuntu's own: `mirrors.ubuntu.com/mirrors.txt` is picked by
# the requester's IP (Launchpad's `getBestMirrorsForCountry`), and answers in the
# country of a CI runner with real mirrors, or with a single line pointing at the
# main archive when the country has none. It is served over HTTP only (port 443 has
# no listener — verified), which costs little: what we take from it is a *hint*
# about where to look, and apt still verifies every index against the Ubuntu
# keyring, so a poisoned list can slow us down but cannot change what installs.
# Entries are still validated against a strict scheme/host pattern, because the
# winner is interpolated into a `sed` that rewrites apt's sources.
#
# The probe is `dists/<suite>/InRelease`: it is the signed file apt itself fetches
# first, so a mirror that answers it fast is a mirror that will actually serve us,
# and the sample stays smaller than the payload it is buying a fast path for.
#
# Verbose by default under GitHub Actions: when a run is slow or a mirror is chosen
# for a bad reason, the log has to say which URL answered what, in how long, and by
# how much the winner won.
set -euo pipefail
export LC_ALL=C   # one decimal separator in the log, whichever locale the runner has

LIST_URL=${LIST_URL:-http://mirrors.ubuntu.com/mirrors.txt}
COUNTRY=""
MAX_MIRRORS=10
SAMPLE_KB=128
TIMEOUT=4        # a 30 kB/s mirror is cut at ~120 kB and still measured correctly
MIN_GAIN=1.5     # only rewrite the sources when the winner is clearly better
DRY_RUN=0
VERBOSE=${VERBOSE:-}
[ "${GITHUB_ACTIONS:-}" = true ] && VERBOSE=1
FILES=()

while [ $# -gt 0 ]; do
  case $1 in
    --dry-run) DRY_RUN=1 ;;
    --verbose|-v) VERBOSE=1 ;;
    --quiet|-q) VERBOSE= ;;
    --country) COUNTRY=$2; shift ;;
    --max-mirrors) MAX_MIRRORS=$2; shift ;;
    --min-gain) MIN_GAIN=$2; shift ;;
    --sources) FILES+=("$2"); shift ;;     # a fixture instead of /etc, for testing
    *) echo "usage: $0 [--dry-run] [-v|--verbose] [-q|--quiet] [--country CC]" >&2
       echo "          [--max-mirrors N] [--min-gain R] [--sources FILE]" >&2
       exit 2 ;;
  esac
  shift
done

SECONDS=0
# Diagnostics go to stderr and *only* there: a probe runs in a background subshell
# whose stdout is the pipe carrying its result back, so one log line on stdout is
# read as the measurement itself. stdout is kept for the answer — the mirror this
# script settled on — which is also what a test can assert on.
# Every line carries `apt-mirror:` so a grep over the job log finds the story.
say() { printf 'apt-mirror: %s\n' "$*" >&2; }
log() { [ -n "$VERBOSE" ] && printf 'apt-mirror:   %s\n' "$*" >&2; return 0; }
# Collapsible sections in the GitHub UI (the runner parses commands on stderr too);
# no-op anywhere else.
group() { [ "${GITHUB_ACTIONS:-}" = true ] && printf '::group::%s\n' "$*" >&2; return 0; }
endgroup() { [ "${GITHUB_ACTIONS:-}" = true ] && printf '::endgroup::\n' >&2; return 0; }

if [ ${#FILES[@]} -eq 0 ]; then
  FILES=(/etc/apt/sources.list.d/ubuntu.sources /etc/apt/sources.list.d/system.sources /etc/apt/sources.list)
fi
EXISTING=()
for f in "${FILES[@]}"; do [ -e "$f" ] && EXISTING+=("$f"); done
if [ ${#EXISTING[@]} -eq 0 ]; then
  say "no apt sources found, leaving apt alone"
  exit 0
fi
log "sources: ${EXISTING[*]}"

# In a dry run nothing is executed: the point is to see the choice, not to make it.
run_root() {
  if [ "$DRY_RUN" = 1 ]; then say "would run: $*"; return; fi
  command sudo "$@"
}

# The mirror exactly as written in the file (no trailing slash), so replacing it is
# a literal substitution that leaves a `security.ubuntu.com` stanza untouched.
current=$(awk '/^URIs:/ { print $2; exit }' "${EXISTING[@]}" 2>/dev/null || true)
[ -n "${current:-}" ] || current=$(awk '/^deb[[:space:]]/ { print $2; exit }' "${EXISTING[@]}" 2>/dev/null || true)
current=${current%/}
if [ -z "${current:-}" ]; then
  say "could not read the current mirror, leaving apt alone"
  exit 0
fi

# The suite comes from the sources file first: that is literally what apt will ask
# for, and it is right on derivatives (a Mint box says VERSION_CODENAME=zena and
# UBUNTU_CODENAME=noble), where guessing from os-release probes a suite the mirrors
# answer 404 for.
suite=$(awk '/^Suites:/ { print $2; exit }' "${EXISTING[@]}" 2>/dev/null || true)
if [ -z "$suite" ]; then
  suite=$(. /etc/os-release 2>/dev/null && printf '%s' "${UBUNTU_CODENAME:-${VERSION_CODENAME:-}}")
fi
[ -n "$suite" ] || suite=$(lsb_release -sc 2>/dev/null || true)
if [ -z "$suite" ]; then
  say "could not determine the suite, leaving apt alone"
  exit 0
fi
say "current $current (suite $suite)"

# Only http(s), only host/path characters: this string ends up in a sed expression.
valid_url() { [[ $1 =~ ^https?://[A-Za-z0-9._~:/@%-]+$ ]]; }

# curl without -f: a 404 is a diagnosis, not a silent zero, and `code` is what says
# whether the mirror answered at all.
measure() {  # $1 url → "<code> <bytes> <secs> <bytes/s>"
  local out rc=0
  out=$(curl -sS -m "$TIMEOUT" -r "0-$((SAMPLE_KB * 1024 - 1))" -o /dev/null \
              -w '%{http_code} %{size_download} %{time_total} %{speed_download}' "$1" 2>/dev/null) || rc=$?
  if [ $rc -ne 0 ]; then printf '000 0 0 0'; else printf '%s' "$out"; fi
}

ok_code() { [ "$1" = 200 ] || [ "$1" = 206 ]; }

# A mirror can be listed over http and still serve https; probing the TLS form first
# upgrades the transport for the real download when it costs nothing to find out.
# Nothing here logs: this runs in a background subshell whose stdout *is* the result
# file, so a diagnostic line would be read back as data. The failed TLS code comes
# out as a sixth field and the caller reports it.
probe() {  # $1 base url → "<url> <bytes/s> <code> <bytes> <secs> <tls code|->"
  local base=$1 tls code bytes secs speed path
  path="/dists/$suite/InRelease"
  tls=${1/http:/https:}
  if [ "$tls" != "$base" ]; then
    read -r code bytes secs speed <<< "$(measure "$tls$path")"
    if ok_code "$code"; then
      printf '%s %s %s %s %s -\n' "$tls" "$speed" "$code" "$bytes" "$secs"
      return
    fi
    read -r code bytes secs speed <<< "$(measure "$base$path")"
    printf '%s %s %s %s %s %s\n' "$base" "$speed" "$code" "$bytes" "$secs" "$code"
    return
  fi
  read -r code bytes secs speed <<< "$(measure "$base$path")"
  printf '%s %s %s %s %s -\n' "$base" "$speed" "$code" "$bytes" "$secs"
}

group "apt-mirror: candidate list"
url=$LIST_URL
[ -n "$COUNTRY" ] && url="http://mirrors.ubuntu.com/$COUNTRY.txt"
list=$(curl -fsS -m 8 "$url" 2>/dev/null || true)
listed=$(printf '%s\n' "$list" | tr -d '\r' | sed 's|/$||' | grep -E '^https?://' | head -n "$MAX_MIRRORS" || true)
say "$url answered $(printf '%s' "$listed" | grep -c .) candidate(s), capped at $MAX_MIRRORS"
log "raw answer: $(printf '%s' "$list" | tr '\n' ' ')"
endgroup

# The list, the current mirror and the main archive: the list may answer with a
# single line (that is what Launchpad does for a country it has no mirrors in), and
# then the only comparison left is against the mirror the runner already has.
RAW=("$current")
while IFS= read -r m; do [ -n "$m" ] && RAW+=("$m"); done <<< "$listed"
RAW+=(http://archive.ubuntu.com/ubuntu)

# One probe per host: a list that repeats an entry (or the archive we append) would
# otherwise be measured twice and fill the log with the same line.
CANDIDATES=()
declare -A seen=()
for m in "${RAW[@]}"; do
  m=${m%/}
  [ -n "$m" ] || continue
  valid_url "$m" || { say "ignoring unusable entry: $m"; continue; }
  [ -n "${seen[$m]:-}" ] && { log "duplicate skipped: $m"; continue; }
  seen[$m]=1
  CANDIDATES+=("$m")
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
group "apt-mirror: probing ${#CANDIDATES[@]} mirror(s), $SAMPLE_KB kB each, ${TIMEOUT}s cap"
i=0
for m in "${CANDIDATES[@]}"; do
  ( probe "$m" > "$work/$i" ) &
  i=$((i + 1))
done
wait || true
endgroup

best=$current
best_speed=0
current_speed=0
mb() { awk -v s="${1:-0}" 'BEGIN { printf "%.2f", (s + 0) / 1048576 }'; }
i=0
for m in "${CANDIDATES[@]}"; do
  read -r chosen speed code bytes secs tls_code < "$work/$i" || { i=$((i + 1)); continue; }
  [ "$tls_code" != "-" ] && log "https form of ${m%/} answered $tls_code, measured http instead"
  tag=""
  [ "${m%/}" = "$current" ] && { current_speed=$speed; tag=" (current)"; }
  if ok_code "$code"; then
    log "$(printf '%8s MB/s  %s  %sB  %.2fs  %s%s' "$(mb "$speed")" \
        "$code" "$bytes" "$secs" "$chosen" "$tag")"
  else
    log "$(printf '%8s        %s  %sB  %.2fs  %s%s' '—' "$code" "$bytes" "$secs" "$chosen" "$tag")"
  fi
  if ok_code "$code" && awk -v a="${speed:-0}" -v b="${best_speed:-0}" 'BEGIN { exit !(a + 0 > b + 0) }'; then
    best=${chosen%/}
    best_speed=$speed
  fi
  i=$((i + 1))
done

ratio=$(awk -v a="${best_speed:-0}" -v b="${current_speed:-0}" \
  'BEGIN { if (b + 0 > 0) printf "%.2f", (a + 0) / (b + 0); else print "inf" }')
say "fastest usable $best at $(mb "$best_speed") MB/s (current $(mb "$current_speed") MB/s, ratio ${ratio}x, need ${MIN_GAIN}x)"

if [ "$best" = "$current" ]; then
  say "keeping $current (nothing faster answered)"
elif [ "${best#https://}" = "${current#http://}" ]; then
  # Same host, TLS form: no new party is trusted and no speed is being guessed at,
  # so the gain rule does not apply — the probe already proved the TLS listener works.
  say "upgrading $current to https (same host, no churn)"
  for f in "${EXISTING[@]}"; do
    grep -qF "$current" "$f" || continue
    run_root cp "$f" "$f.save"
    log "sed -i s|$current|$best|g $f"
    run_root sed -i "s|$current|$best|g" "$f"
    say "rewrote $f (backup $f.save)"
  done
elif ! awk -v a="${best_speed:-0}" -v b="${current_speed:-0}" -v g="$MIN_GAIN" \
     'BEGIN { exit !((a + 0) > 0 && ((b + 0) == 0 || (a + 0) > (b + 0) * (g + 0))) }'; then
  say "keeping $current (winner is under ${MIN_GAIN}x faster, not worth the churn)"
else
  say "switching to $best"
  for f in "${EXISTING[@]}"; do
    grep -qF "$current" "$f" || continue
    run_root cp "$f" "$f.save"
    log "sed -i s|$current|$best|g $f"
    run_root sed -i "s|$current|$best|g" "$f"
    say "rewrote $f (backup $f.save)"
  done
fi

group "apt-mirror: apt-get update"
# Refresh the lists here, so the install step does not pay for `apt-get update`
# again; retries answer the 404 a just-switched mirror occasionally serves. A failure
# is reported and not fatal: the install step refreshes again on its own retry.
if ! run_root apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=15 \
     -o Acquire::https::Timeout=15 update; then
  say "apt-get update failed here; the install step will retry it"
fi
endgroup

log "apt now reads: $(grep -h -m1 '^URIs:' "${EXISTING[@]}" 2>/dev/null | head -1)"
say "done in ${SECONDS}s"
printf '%s\n' "$best"   # stdout: the mirror apt will use, for scripts and tests
