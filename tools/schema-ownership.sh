#!/bin/sh
# The ownership check: only the bridge's own records may live in the dcs.bridge
# package.
#
# A topic id is its payload's fully-qualified type name, so the package name
# partitions the topic space and nothing else has to. The bridge's own records
# go in dcs.bridge, both built-in sets' in dcs.builtin, and an adopter's in a
# package they own. This script polices the first of those three, which is the
# only one this repository can police.
#
# The permitted set is the bridge's own records -- the ones the broker answers
# itself, the lifecycle topics, the bridge's own commands, the operator-eval
# audit record and the acknowledgement record -- together with the nested types
# those records carry. It is a naming check rather than a numbering one,
# because the Envelope names no payload type and so no shared file exists for
# two owners to contend over.
#
# The list below is that set, and it is the whole of it. Adding a name is a
# change to what the bridge owns, so it does not happen because a record was
# convenient to put here. A record the bridge does not own belongs in another
# package.
#
# Only top-level messages are checked. A message nested inside a permitted
# record is a nested type that record carries, and is permitted with it.
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
# The frame, and the request and reply pairs the broker answers itself
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

# The lifecycle topics
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

# What CoordinateCalibration carries. These three are named separately because
# they are not records in their own right.
MissionDate
Projection
Verification

# The bridge's own commands
Resync
SeqAck
ReloadSimDriver
SetEnabled
ReloadConfig

# The operator-eval audit record
EvalExecuted

# The acknowledgement record
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
    printf 'the %s package holds a message the bridge does not own.\n\n' \
        "$PACKAGE" >&2
    printf '%s\n' "$STRAY" | while IFS='	' read -r name where; do
        printf '  %s\t%s\n' "$name" "${where#"$ROOT"/}" >&2
    done
    printf '\n' >&2
    printf 'Only the bridge'"'"'s own records may live in %s. A built-in set'"'"'s\n' \
        "$PACKAGE" >&2
    printf 'record belongs in dcs.builtin and an adopter'"'"'s in a package they own.\n' >&2
    printf 'If the bridge really does own this record, add it to the list in %s.\n' \
        "tools/schema-ownership.sh" >&2
    exit 1
fi

printf '%s owns nothing it should not, across %s file(s)\n' "$PACKAGE" "$COUNT"
