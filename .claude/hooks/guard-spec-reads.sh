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

. "$(dirname -- "$0")/payload.sh"

CAP=${SPEC_READ_MAX_LINES:-400}

tool=$(parse tool_name)
[ "$tool" = "Read" ] || exit 0

path=$(parse file_path)
[ -n "$path" ] || exit 0

norm=$(normalize_path "$path")

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
