#!/bin/sh
# List what README.md says is not final or not built yet.
#
# The README is written for users, and where the build has not reached a
# thing the README describes, the sentence says so. This prints those
# sentences with their line numbers, so the session closing the task that
# settles one knows which paragraph to rewrite. It fails nothing: an open
# claim is a fact about the build, not a defect in the file.
#
# POSIX sh and awk only.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
FILE=${1:-$ROOT/README.md}

[ -f "$FILE" ] || { printf 'no README at %s\n' "$FILE" >&2; exit 2; }

awk '
    { sub(/\r$/, "") }
    /^#/ { heading = $0; sub(/^#+ /, "", heading) }
    /not final|not yet|planned|does not yet|so far/ {
        line = $0; sub(/^[ \t]+/, "", line)
        printf "%4d  %-32s %s\n", NR, heading, line
        n++
    }
    END {
        if (n == 0) { printf "README.md marks nothing as open\n"; exit }
        printf "\n%d open claim%s. Each comes out with the task that settles it.\n", n, (n == 1 ? "" : "s")
    }
' "$FILE"
