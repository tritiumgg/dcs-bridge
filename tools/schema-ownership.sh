#!/bin/sh
# SPEC 8.4's ownership check: only the bridge's own records may live in the
# dcs.bridge package.
#
# A topic id is its payload's fully-qualified type name (SPEC 5.2), so the
# package name partitions the topic space and nothing else has to. SPEC 8.2
# assigns the bridge's own records to dcs.bridge, both built-in sets' to
# dcs.builtin, and an adopter's to a package they own. This script polices the
# first of those three, which is the only one this repository can police.
#
# SPEC 8.4 defines the permitted set as the records SPEC 1.2 enumerates -- from
# SPEC 5.2 (broker-answered), SPEC 9 (lifecycle), SPEC 9.5 (the bridge's own
# commands), SPEC 7.6 (the operator-eval audit record) and SPEC 8.5.3 (the
# acknowledgement record) -- together with the nested types those records
# carry. It is a naming check rather than a numbering one, because the Envelope
# names no payload type and so no shared file exists for two owners to contend
# over.
#
# The list below is that set, and it is the whole of it. Adding a name is a
# change to what the bridge owns, so it needs a SPEC section beside it. A
# message the specification does not put in dcs.bridge belongs in another
# package instead.
#
# Only top-level messages are checked. A message nested inside a permitted
# record is a nested type that record carries, which SPEC 8.4 permits by name.
#
# POSIX sh and awk only. Needs no buf: the check reads the .proto sources, so
# it runs on a checkout with no toolchain.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TREE=${1:-$ROOT/proto}
PACKAGE=dcs.bridge

[ -d "$TREE" ] || { printf 'no .proto tree at %s\n' "$TREE" >&2; exit 2; }

# One name per line. A leading # is a comment and blank lines are skipped.
owned() {
    cat <<'EOF'
# SPEC 5.2 -- the frame, and the five request/reply pairs the broker answers
Envelope
Ping
Pong
Auth
AuthResult
GetSchema
Schema
GetTopics
Topics
TopicEntry
SetTopicFilter
TopicFilterResult
Rejected

# SPEC 9 -- the thirteen lifecycle topics
MissionLoadBegan
MissionLoaded
MissionStopped
EpochOpened
EpochClosed
CoordinateCalibration
SimulationPaused
SimulationResumed
ResyncBegan
ResyncEnded
CallbackHz
SimDriverLoaded
SimDriverReloaded

# SPEC 6.3 -- what CoordinateCalibration carries. SPEC 8.4 names these three
# because no enumeration of records covers them.
MissionDate
Projection
Verification

# SPEC 9.5 -- the bridge's own commands
Resync
SeqAck
ReloadSimDriver
SetEnabled
ReloadConfig

# SPEC 7.6 -- the operator-eval audit record
EvalExecuted

# SPEC 8.5.3 -- the acknowledgement record
CommandAck
EOF
}

# Every top-level message in a file whose package is the one under check.
#
# Depth comes from counting braces, so an extend block, an enum and a nested
# message all close themselves and only a message at depth zero is reported.
# Line comments are stripped first; the tree uses no block comments and no
# brace inside a string.
declared() {
    awk -v want="$PACKAGE" '
        FNR == 1 { pkg = ""; depth = 0 }
        { line = $0; sub(/\/\/.*$/, "", line) }
        line ~ /^[ \t]*package[ \t]+[A-Za-z0-9_.]+[ \t]*;/ {
            pkg = line
            sub(/^[ \t]*package[ \t]+/, "", pkg)
            sub(/[ \t]*;.*$/, "", pkg)
        }
        {
            if (pkg == want && depth == 0 &&
                line ~ /^[ \t]*message[ \t]+[A-Za-z_][A-Za-z0-9_]*/) {
                name = line
                sub(/^[ \t]*message[ \t]+/, "", name)
                sub(/[^A-Za-z0-9_].*$/, "", name)
                print name "\t" FILENAME ":" FNR
            }
            n = gsub(/\{/, "{", line)
            m = gsub(/\}/, "}", line)
            depth += n - m
            if (depth < 0) depth = 0
        }
    ' "$@"
}

FILES=$(find "$TREE" -name '*.proto' -type f | sort)
[ -n "$FILES" ] || { printf 'no .proto files under %s\n' "$TREE" >&2; exit 2; }

# One space-separated line, because BSD awk refuses a -v value carrying a
# newline.
ALLOWED=$(owned | grep -v '^#' | grep -v '^[[:space:]]*$' | tr '\n' ' ')

# shellcheck disable=SC2086
STRAY=$(declared $FILES | awk -v allowed="$ALLOWED" '
    BEGIN { n = split(allowed, a, " "); for (i = 1; i <= n; i++) ok[a[i]] = 1 }
    !($1 in ok) { print }
')

COUNT=$(printf '%s' "$FILES" | grep -c . || true)

if [ -n "$STRAY" ]; then
    printf 'SPEC 8.4: the %s package holds a message the bridge does not own.\n\n' \
        "$PACKAGE" >&2
    printf '%s\n' "$STRAY" | while IFS='	' read -r name where; do
        printf '  %s\t%s\n' "$name" "${where#"$ROOT"/}" >&2
    done
    printf '\n' >&2
    printf 'Only the records SPEC 1.2 enumerates may live in %s. A built-in\n' \
        "$PACKAGE" >&2
    printf 'set'"'"'s record belongs in dcs.builtin and an adopter'"'"'s in a package\n' >&2
    printf 'they own (SPEC 8.2). If the specification does put this record in\n' >&2
    printf '%s, add it to the list in %s with its\n' \
        "$PACKAGE" "tools/schema-ownership.sh" >&2
    printf 'SPEC section beside it.\n' >&2
    exit 1
fi

printf '%s owns nothing it should not, across %s file(s)\n' "$PACKAGE" "$COUNT"
