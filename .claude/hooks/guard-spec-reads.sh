#!/bin/sh
# PreToolUse guard: refuse an unbounded Read of a specification.
#
# The specifications are large and frozen. Loading one whole costs most of a
# context window and buys nothing the ledger cannot locate more precisely. This hook
# makes that constraint hold rather than hoping the instruction is followed.
#
# A bounded Read still passes: pass offset and limit, with limit at or under
# the cap. Everything else routes through tools/ledger.sh, which runs in bash
# and is not affected by this hook.
#
# Exit 2 blocks the tool call and hands stderr to the model as the reason.
# Exit 0 renders no decision and the normal permission flow continues.
#
# No jq. It is not installed by default on macOS or Windows, so the payload
# is parsed with awk instead.

CAP=${SPEC_READ_MAX_LINES:-400}

payload=$(cat)

parse() {
    printf '%s' "$payload" | awk -v key="$1" '
        {
            # String form: "key": "value", with escaped quotes tolerated.
            pat = "\"" key "\"[ \t]*:[ \t]*\""
            if (match($0, pat)) {
                rest = substr($0, RSTART + RLENGTH)
                out = ""
                for (i = 1; i <= length(rest); i++) {
                    ch = substr(rest, i, 1)
                    if (ch == "\\") { out = out substr(rest, i + 1, 1); i++; continue }
                    if (ch == "\"") break
                    out = out ch
                }
                print out
                exit
            }
            # Number form: "key": 123
            pat = "\"" key "\"[ \t]*:[ \t]*[0-9]+"
            if (match($0, pat)) {
                s = substr($0, RSTART, RLENGTH)
                sub(/^.*:[ \t]*/, "", s)
                print s
                exit
            }
        }
    '
}

tool=$(parse tool_name)
[ "$tool" = "Read" ] || exit 0

path=$(parse file_path)
[ -n "$path" ] || exit 0

# Normalize separators so one pattern covers both platforms, and squeeze
# repeats so a doubled separator cannot slip a path past the patterns below.
norm=$(printf '%s' "$path" | tr '\\' '/' | tr -s '/')

# Only the frozen specifications. The plan is 428 lines, changes weekly, and
# its ledger drifts between regenerations, so routing reads of it through a
# possibly stale index would be worse than reading it whole.
guarded=no
case "$norm" in
    */docs/specs/*/spec.md|docs/specs/*/spec.md) guarded=yes ;;
esac
[ "$guarded" = yes ] || exit 0

limit=$(parse limit)

if [ -n "$limit" ] && [ "$limit" -le "$CAP" ] 2>/dev/null; then
    exit 0
fi

name=${norm##*/}
dir=${norm%/*}
code=${dir##*/}

cat >&2 <<EOF
Blocked: an unbounded Read of $norm.

This document is too large to load whole, and the ledger beside it exists so
you do not have to. Locate first, then retrieve:

    tools/ledger.sh subjects <CODE>              every subject in the ledger
    tools/ledger.sh find <CODE> <subject>        the rows, with their anchors
    tools/ledger.sh show <CODE> "<anchor>"       the prose around one anchor
    tools/ledger.sh sections <CODE>              the heading tree with sizes
    tools/ledger.sh read <CODE> "<section>"      one whole section

Run tools/ledger.sh codes for the CODE that names this file ($code/$name).

If you truly need the Read tool here, bound it: pass offset and limit with
limit at or under $CAP lines.
EOF
exit 2
