#!/usr/bin/env bash
# Point apt at the fastest mirror, so a run does not pay minutes for megabytes.
#
# Why this exists: the runner ships with `azure.archive.ubuntu.com`, and that host
# collapses often enough that two attempts of the same commit paid 55 s and 114 s
# in `system deps`. Measured on the runner, on the *same* host and run:
#
#   Get: ... resolute-updates/main amd64v3 Packages [901 kB]    ← the index
#   Fetched 4284 kB in 0s (11.4 MB/s)
#   Get: ... libx11-dev ... [743 kB]                            ← the packages
#   Fetched 2137 kB in 1min 47s (20.0 kB/s)
#
# so the mirror serves indexes fast and the package pool at a crawl. That is why
# the probe here is a *pool* file and not `dists/<suite>/InRelease`: an index-only
# probe calls this mirror healthy and would have kept it. The path comes from apt
# itself (`apt-get --print-uris`), so nothing about a suite or a version is guessed.
#
# Why here and not in `tools/`: this is CI-only. It is not installed, not shipped
# in the release and not a dev command; it exists to make `ci.yml` cheap, so it
# lives beside `ci.yml`.
#
# Why not a third-party action: it needs sudo and it rewrites apt's sources, so the
# trust surface should be this file, in our diff, reviewed as code.
#
# Where the candidates come from, in this order:
#   1. when `URIs:` is `mirror+file:PATH`, PATH — apt's own list, written by the
#      runner image (azure priority:1, archive priority:2, security priority:3).
#      apt tries them in that order and fails over on *errors*, never on slowness,
#      so azure's 20 kB/s pool is used every time it answers;
#   2. `mirrors.ubuntu.com/mirrors.txt` — Ubuntu's own list, picked by requester IP
#      (Launchpad `getBestMirrorsForCountry`). It answers with real mirrors in the
#      country of a residential IP, and with a single line pointing at the main
#      archive when the country has none — which is what a datacenter IP gets;
#   3. `archive.ubuntu.com`, so there is always something to compare against.
# The list is HTTP-only (port 443 has no listener — verified) and unsigned, which
# costs little: it is a *hint* about where to look, and apt still verifies every
# index against the Ubuntu keyring, so a poisoned list can slow us down but cannot
# change what installs. Entries are validated against a strict scheme/host pattern
# anyway, because the winner is interpolated into a `sed` that rewrites apt sources.
#
# Verbose by default under GitHub Actions: when a run is slow or a mirror is chosen
# for a bad reason, the log has to say which URL answered what, in how long, and by
# how much the winner won.
set -euo pipefail
export LC_ALL=C   # one decimal separator in the log, whichever locale the runner has

LIST_URL=${LIST_URL:-http://mirrors.ubuntu.com/mirrors.txt}
COUNTRY=""
PACKAGES=${APT_PACKAGES:-}
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
    --packages) PACKAGES=$2; shift ;;
    --max-mirrors) MAX_MIRRORS=$2; shift ;;
    --min-gain) MIN_GAIN=$2; shift ;;
    --sources) FILES+=("$2"); shift ;;     # a fixture instead of /etc, for testing
    *) echo "usage: $0 [--dry-run] [-v|--verbose] [-q|--quiet] [--country CC]" >&2
       echo "          [--packages \"pkg ...\"] [--max-mirrors N] [--min-gain R]" >&2
       echo "          [--sources FILE]" >&2
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
# a literal substitution that leaves a `security.ubuntu.com` stanza untouched. It may
# also be apt's own `mirror+file:PATH` method, which is not a URL at all.
current=$(awk '/^URIs:/ { print $2; exit }' "${EXISTING[@]}" 2>/dev/null || true)
[ -n "${current:-}" ] || current=$(awk '/^deb[[:space:]]/ { print $2; exit }' "${EXISTING[@]}" 2>/dev/null || true)
current=${current%/}
if [ -z "${current:-}" ]; then
  say "could not read the current mirror, leaving apt alone"
  exit 0
fi
mirror_method_file=""
case $current in
  mirror+file:*) mirror_method_file=${current#mirror+file:} ;;
  mirror://*)    say "current is $current (apt's remote mirror method); cannot read it, leaving apt alone"; exit 0 ;;
esac

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

# What the mirrors are actually asked for: the biggest .deb of the packages the job
# installs, as a path relative to the mirror root. `--print-uris` resolves versions
# and pockets from the lists apt already has, so nothing here can go stale.
probe_path=""
if [ -n "$PACKAGES" ]; then
  # shellcheck disable=SC2086  # the package list is word-split on purpose
  probe_path=$(apt-get --print-uris -y install $PACKAGES 2>/dev/null \
    | awk -F"'" '/^.(http|https)/ { n = split($3, f, " "); print f[2], $2 }' \
    | sort -rn | head -1 | cut -d' ' -f2- \
    | sed 's|.*\(pool/.*\)|\1|' || true)
  [ -n "$probe_path" ] && log "probe file: /$probe_path (biggest .deb of: $PACKAGES)"
fi
if [ -z "$probe_path" ]; then
  # No lists yet, or everything already installed: fall back to the index. Weaker on
  # purpose-known weakness (it does not see a slow pool), but it still proves the
  # mirror carries this suite at all.
  probe_path="dists/$suite/InRelease"
  log "probe file: /$probe_path (fallback: no .deb to resolve)"
fi

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
  local base=$1 tls code bytes secs speed tls_code=-
  tls=${1/http:/https:}
  if [ "$tls" != "$base" ]; then
    read -r code bytes secs speed <<< "$(measure "$tls/$probe_path")"
    if ok_code "$code"; then
      printf '%s %s %s %s %s -\n' "$tls" "$speed" "$code" "$bytes" "$secs"
      return
    fi
    tls_code=$code
    read -r code bytes secs speed <<< "$(measure "$base/$probe_path")"
    printf '%s %s %s %s %s %s\n' "$base" "$speed" "$code" "$bytes" "$secs" "$tls_code"
    return
  fi
  read -r code bytes secs speed <<< "$(measure "$base/$probe_path")"
  printf '%s %s %s %s %s -\n' "$base" "$speed" "$code" "$bytes" "$secs"
}

# 1. apt's own list, in apt's own order — the entry it would pick first is the
#    baseline the winner has to beat, not some host we assume is current.
FROM_APT=()
if [ -n "$mirror_method_file" ] && [ -r "$mirror_method_file" ]; then
  group "apt-mirror: apt's own list ($mirror_method_file)"
  while IFS= read -r line; do
    entry=${line%%[[:space:]]*}          # drop the "\tpriority:N" suffix
    entry=${entry%/}
    [ -n "$entry" ] || continue
    log "listed: $entry"
    FROM_APT+=("$entry")
  done < "$mirror_method_file"
  endgroup
elif [ -n "$mirror_method_file" ]; then
  say "cannot read $mirror_method_file, leaving apt alone"
  exit 0
fi

# 2. Ubuntu's list by requester IP (or a country, when asked).
group "apt-mirror: Ubuntu mirror list"
url=$LIST_URL
[ -n "$COUNTRY" ] && url="http://mirrors.ubuntu.com/$COUNTRY.txt"
list=$(curl -fsS -m 8 "$url" 2>/dev/null || true)
FROM_LIST=$(printf '%s\n' "$list" | tr -d '\r' | sed 's|/$||' | grep -E '^https?://' | head -n "$MAX_MIRRORS" || true)
say "$url answered $(printf '%s' "$FROM_LIST" | grep -c . ) candidate(s), capped at $MAX_MIRRORS"
log "raw answer: $(printf '%s' "$list" | tr '\n' ' ')"
endgroup

RAW=("${FROM_APT[@]}")
while IFS= read -r m; do [ -n "$m" ] && RAW+=("$m"); done <<< "$FROM_LIST"
RAW+=(http://archive.ubuntu.com/ubuntu)

# One probe per host: a list that repeats an entry (or the archive we append) would
# otherwise be measured twice and fill the log with the same line. The key drops the
# scheme, so `http://host/ubuntu` and `https://host/ubuntu` are one candidate — the
# probe tries TLS first anyway, so they would be the same measurement.
CANDIDATES=()
declare -A seen=()
for m in "${RAW[@]}"; do
  m=${m%/}
  [ -n "$m" ] || continue
  valid_url "$m" || { say "ignoring unusable entry: $m"; continue; }
  key=${m#http://}; key=${key#https://}
  [ -n "${seen[$key]:-}" ] && { log "duplicate skipped: $m"; continue; }
  seen[$key]=1
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

mb() { awk -v s="${1:-0}" 'BEGIN { printf "%.2f", (s + 0) / 1048576 }'; }
best=""
best_speed=0
probe_of() {  # prints the measured speed for a candidate, 0 when it did not answer
  local m=$1 k=0
  for c in "${CANDIDATES[@]}"; do
    if [ "$c" = "$m" ]; then
      awk -v s="$(awk '{ print $2 }' "$work/$k")" 'BEGIN { printf "%d", s + 0 }'
      return
    fi
    k=$((k + 1))
  done
  printf '0'
}

i=0
for m in "${CANDIDATES[@]}"; do
  read -r chosen speed code bytes secs tls_code < "$work/$i" || { i=$((i + 1)); continue; }
  [ "$tls_code" != "-" ] && log "https form of ${m%/} answered $tls_code, measured http instead"
  if ok_code "$code"; then
    log "$(printf '%8s MB/s  %s  %sB  %.2fs  %s' "$(mb "$speed")" "$code" "$bytes" "$secs" "$chosen")"
  else
    log "$(printf '%8s        %s  %sB  %.2fs  %s' '—' "$code" "$bytes" "$secs" "$chosen")"
  fi
  if ok_code "$code" && awk -v a="${speed:-0}" -v b="${best_speed:-0}" 'BEGIN { exit !(a + 0 > b + 0) }'; then
    best=${chosen%/}
    best_speed=$speed
  fi
  i=$((i + 1))
done

# What apt would use today, and therefore what the winner must beat:
#   - a concrete mirror: itself;
#   - `mirror+file:`: the first entry of that file that answers at all, because apt
#     tries them in order and never skips one for being slow.
baseline=$current
baseline_speed=0
if [ -n "$mirror_method_file" ]; then
  baseline=""
  for m in "${FROM_APT[@]}"; do
    s=$(probe_of "$m")
    if [ "${s:-0}" -gt 0 ]; then baseline=$m; baseline_speed=$s; break; fi
  done
  [ -n "$baseline" ] || say "none of apt's own entries answered"
  say "apt would use ${baseline:-nothing} at $(mb "$baseline_speed") MB/s (first entry that answers)"
else
  baseline_speed=$(probe_of "$current")
fi

ratio=$(awk -v a="${best_speed:-0}" -v b="${baseline_speed:-0}" \
  'BEGIN { if (b + 0 > 0) printf "%.2f", (a + 0) / (b + 0); else print "inf" }')
say "fastest $best at $(mb "$best_speed") MB/s vs $(mb "$baseline_speed") MB/s, ratio ${ratio}x, need ${MIN_GAIN}x"

apply() {  # $1 the URL to write in place of `current`
  local to=$1
  for f in "${EXISTING[@]}"; do
    grep -qF "$current" "$f" || continue
    run_root cp "$f" "$f.save"
    log "sed -i s|$current|$to|g $f"
    run_root sed -i "s|$current|$to|g" "$f"
    [ "$DRY_RUN" = 1 ] || say "rewrote $f (backup $f.save)"
  done
}

if [ -z "$best" ]; then
  say "no mirror answered, leaving apt alone"
elif [ "$best" = "$baseline" ]; then
  say "keeping $current (nothing beat it)"
elif [ -n "$mirror_method_file" ] && [ "$best" = "$current" ]; then
  say "keeping $current (nothing beat apt's own list)"
elif [ -n "$mirror_method_file" ]; then
  # Replacing apt's method with a concrete URL gives up its failover, so it only
  # happens on a measured gain over the entry apt would have used.
  if awk -v a="${best_speed:-0}" -v b="${baseline_speed:-0}" -v g="$MIN_GAIN" \
       'BEGIN { exit !((a + 0) > 0 && ((b + 0) == 0 || (a + 0) > (b + 0) * (g + 0))) }'; then
    say "overriding apt's list: $best is over ${MIN_GAIN}x its first working entry ($baseline)"
    apply "$best"
  else
    say "keeping $current (winner does not beat apt's own list by ${MIN_GAIN}x)"
  fi
elif [ "${best#https://}" = "${current#http://}" ]; then
  # Same host, TLS form: no new party is trusted and no speed is being guessed at,
  # so the gain rule does not apply — the probe already proved the TLS listener works.
  say "upgrading $current to https (same host, no churn)"
  apply "$best"
elif awk -v a="${best_speed:-0}" -v b="${baseline_speed:-0}" -v g="$MIN_GAIN" \
     'BEGIN { exit !((a + 0) > 0 && ((b + 0) == 0 || (a + 0) > (b + 0) * (g + 0))) }'; then
  say "switching to $best"
  apply "$best"
else
  say "keeping $current (winner is under ${MIN_GAIN}x faster, not worth the churn)"
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
printf '%s\n' "${best:-$current}"   # stdout: the mirror apt will use, for scripts and tests
