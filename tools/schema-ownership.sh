#!/bin/sh
# The ownership check: the bridge's own packages hold the bridge's own records
# and nothing else, and nothing else lives under dcsbridge.
#
# A topic id is its payload's fully-qualified type name, so the package name
# partitions the topic space and nothing else has to. Each package names the
# component that produces its records: the bridge's own go in dcsbridge.broker,
# dcsbridge.hook and dcsbridge.sim, the built-in sets' in dcsbridge.builtin.hook
# and dcsbridge.builtin.sim, and an adopter's in a package they own. ADR 0021.
# This script polices the dcsbridge side of that line, which is the only side
# this repository can police.
#
# For each of the bridge's three packages the permitted set is the records that
# component produces -- the ones the broker frames, answers or knows by name,
# the lifecycle topics the hook driver emits, the resync pair the sim driver
# emits, and the bridge's own commands by which driver handles them -- together
# with the nested types those records carry. It is a naming check rather than
# a numbering one, because the Envelope names no payload type and so no shared
# file exists for two owners to contend over.
#
# The lists below are that set, and they are the whole of it. Adding a name is
# a change to what the bridge owns, so it does not happen because a record was
# convenient to put here. A record the bridge does not own belongs in another
# package. Moving a name between lists renames a topic, which no released
# consumer survives; a record that changes producer is a new record.
#
# Only top-level messages are checked. A message nested inside a permitted
# record is a nested type that record carries, and is permitted with it.
#
# POSIX sh and awk only. Needs no buf: the check reads the .proto sources, so
# it runs on a checkout with no toolchain.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TREE=${1:-$ROOT/proto}
PREFIX=dcsbridge

[ -d "$TREE" ] || { printf 'no .proto tree at %s\n' "$TREE" >&2; exit 2; }

# One record per line: package, a tab, message name. A leading # is a comment
# and blank lines are skipped.
owned() {
    cat <<'EOF'
# dcsbridge.broker: the frame, the handshake, the request and reply pairs the
# broker answers itself, and what it consumes or knows by name
dcsbridge.broker	Envelope
dcsbridge.broker	Handshake
dcsbridge.broker	Ping
dcsbridge.broker	Pong
dcsbridge.broker	Auth
dcsbridge.broker	AuthResult
dcsbridge.broker	GetSchema
dcsbridge.broker	Schema
dcsbridge.broker	GetTopics
dcsbridge.broker	Topics
dcsbridge.broker	TopicEntry
dcsbridge.broker	SetTopicFilter
dcsbridge.broker	TopicFilterResult
dcsbridge.broker	Rejected
dcsbridge.broker	SeqAck
dcsbridge.broker	SetEnabled

# The acknowledgement record. Both drivers emit it and the broker is the one
# component that knows it by name, so neither driver's package owns it.
dcsbridge.broker	CommandAck

# dcsbridge.hook: the lifecycle topics the hook driver emits
dcsbridge.hook	MissionLoadBegan
dcsbridge.hook	MissionLoaded
dcsbridge.hook	MissionStopped
dcsbridge.hook	EpochOpened
dcsbridge.hook	EpochClosed
dcsbridge.hook	CoordinateCalibration
dcsbridge.hook	SimulationPaused
dcsbridge.hook	SimulationResumed
dcsbridge.hook	CallbackHz
dcsbridge.hook	SimDriverLoaded
dcsbridge.hook	SimDriverReloaded

# What CoordinateCalibration carries. These three are named separately because
# they are not records in their own right.
dcsbridge.hook	MissionDate
dcsbridge.hook	Projection
dcsbridge.hook	Verification

# The bridge's own commands the hook driver handles, and the operator-eval
# audit record it emits
dcsbridge.hook	ReloadSimDriver
dcsbridge.hook	ReloadConfig
dcsbridge.hook	EvalExecuted

# dcsbridge.sim: the resync command and the pair of records that bracket it
dcsbridge.sim	Resync
dcsbridge.sim	ResyncBegan
dcsbridge.sim	ResyncEnded
EOF
}

# The packages under the prefix that may exist at all. The built-in packages
# are open: any record may live there, and the generator refuses a name
# collision between the two.
packages() {
    cat <<'EOF'
dcsbridge.broker
dcsbridge.hook
dcsbridge.sim
dcsbridge.builtin.hook
dcsbridge.builtin.sim
EOF
}

# Every package declaration and every top-level message, as
# package<tab>name<tab>file:line. A package declaration reports itself with an
# empty name, so a file under the prefix that declares no message is checked
# too.
#
# Depth comes from counting braces, so an extend block, an enum and a nested
# message all close themselves and only a message at depth zero is reported.
# Line comments are stripped first; the tree uses no block comments and no
# brace inside a string.
declared() {
    awk '
        FNR == 1 { pkg = ""; depth = 0 }
        { line = $0; sub(/\/\/.*$/, "", line) }
        line ~ /^[ \t]*package[ \t]+[A-Za-z0-9_.]+[ \t]*;/ {
            pkg = line
            sub(/^[ \t]*package[ \t]+/, "", pkg)
            sub(/[ \t]*;.*$/, "", pkg)
            print pkg "\t\t" FILENAME ":" FNR
        }
        {
            if (depth == 0 &&
                line ~ /^[ \t]*message[ \t]+[A-Za-z_][A-Za-z0-9_]*/) {
                name = line
                sub(/^[ \t]*message[ \t]+/, "", name)
                sub(/[^A-Za-z0-9_].*$/, "", name)
                print pkg "\t" name "\t" FILENAME ":" FNR
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

# Space-separated, because BSD awk refuses a -v value carrying a newline. An
# owned record is spelled package/name so one word carries both.
OWNED=$(owned | grep -v '^#' | grep -v '^[[:space:]]*$' | tr '\t' '/' | tr '\n' ' ')
PACKAGES=$(packages | tr '\n' ' ')

# Each finding is kind<tab>package<tab>name<tab>where. Kinds: "package" for a
# package under the prefix that is not one of the five, and "stray" for a
# message in one of the bridge's own packages that its list does not name.
# shellcheck disable=SC2086
FOUND=$(declared $FILES | awk -F '\t' -v prefix="$PREFIX" \
    -v owned="$OWNED" -v packages="$PACKAGES" '
    BEGIN {
        n = split(owned, a, " ")
        for (i = 1; i <= n; i++) { ok[a[i]] = 1; split(a[i], p, "/"); bounded[p[1]] = 1 }
        n = split(packages, a, " ")
        for (i = 1; i <= n; i++) known[a[i]] = 1
    }
    {
        pkg = $1; name = $2; where = $3
        under = (pkg == prefix || index(pkg, prefix ".") == 1)
        if (under && !(pkg in known)) {
            if (name == "") print "package\t" pkg "\t-\t" where
            next
        }
        if (name != "" && (pkg in bounded) && !((pkg "/" name) in ok))
            print "stray\t" pkg "\t" name "\t" where
    }
')

COUNT=$(printf '%s' "$FILES" | grep -c . || true)

if [ -n "$FOUND" ]; then
    printf 'the schema holds something the bridge'"'"'s packages do not own.\n\n' >&2
    printf '%s\n' "$FOUND" | while IFS='	' read -r kind pkg name where; do
        case $kind in
        package) printf '  package %s\t%s\n' "$pkg" "${where#"$ROOT"/}" >&2 ;;
        stray) printf '  %s.%s\t%s\n' "$pkg" "$name" "${where#"$ROOT"/}" >&2 ;;
        esac
    done
    printf '\n' >&2
    printf 'Under %s there are five packages: broker, hook and sim hold the\n' \
        "$PREFIX" >&2
    printf 'bridge'"'"'s own records, builtin.hook and builtin.sim the built-in sets'"'"'.\n' >&2
    printf 'An adopter'"'"'s record belongs in a package they own. If the bridge\n' >&2
    printf 'really does own this record, add it to the list in %s.\n' \
        "tools/schema-ownership.sh" >&2
    exit 1
fi

printf '%s owns nothing it should not, across %s file(s)\n' "$PREFIX" "$COUNT"
