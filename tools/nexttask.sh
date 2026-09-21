#!/bin/sh
# Name the next plan task: the first row, in the plan's order, not closed yet.
#
# The plan states build order and has no status column, and a branch that
# lands by fast-forward leaves no trace of its name in git. What records a
# closed task is its merged pull request, whose branch is task/<id>-<summary>.
# So the closed set is read from GitHub and the order from the plan, and
# neither is kept by hand.
#
# Two kinds of row say their own state, in the first words of the task cell,
# because no pull request can. "Landed ..." is closed: it shipped inside
# another task's pull request, or on main before there were any. "Waits on
# ..." is open and cannot start, so it is passed over and named after the
# row, where the caller sees it every time.
#
# What this cannot see is whether the row may start: a task in progress, a
# pull request waiting on review, and a maintainer decision a phase waits on
# are all in STATE.md and GitHub, in prose, and are for the caller to read.
#
#   tools/nexttask.sh                 the next row: id, phase, task, what closes it
#   tools/nexttask.sh --closed FILE   closed branches or ids from FILE, one per
#                                     line, in place of asking GitHub
#   tools/nexttask.sh --plan FILE     a plan other than docs/plan/plan.md
#
# Exit 0 with the row, 1 when no row is left to start, 2 on a usage error or a
# plan whose rows cannot be read, 3 when
# GitHub cannot be asked.
#
# POSIX sh and awk only.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PLAN=$ROOT/docs/plan/plan.md
CLOSED=

while [ $# -gt 0 ]; do
    case $1 in
        --closed) [ $# -ge 2 ] || { printf 'usage: --closed FILE\n' >&2; exit 2; }
                  CLOSED=$2; shift 2 ;;
        --plan)   [ $# -ge 2 ] || { printf 'usage: --plan FILE\n' >&2; exit 2; }
                  PLAN=$2; shift 2 ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done

[ -f "$PLAN" ] || { printf 'no plan at %s\n' "$PLAN" >&2; exit 2; }

TMP=
cleanup() { [ -z "$TMP" ] || rm -f "$TMP"; }
trap cleanup EXIT

if [ -z "$CLOSED" ]; then
    command -v gh >/dev/null 2>&1 || { printf 'gh is not installed, so the merged pull requests cannot be read\n' >&2; exit 3; }
    TMP=$(mktemp "${TMPDIR:-/tmp}/nexttask.XXXXXX")
    # gh carries its own jq, so --jq asks nothing of the machine.
    if ! gh pr list --state merged --limit 1000 --json headRefName --jq '.[].headRefName' >"$TMP" 2>/dev/null; then
        printf 'gh could not list the merged pull requests\n' >&2
        exit 3
    fi
    CLOSED=$TMP
fi

[ -f "$CLOSED" ] || { printf 'no closed list at %s\n' "$CLOSED" >&2; exit 2; }

awk '
    # The closed list first: a branch name or a bare id per line. Told apart
    # by name and not by FNR == NR, which an empty list makes true of the plan.
    FILENAME == ARGV[1] {
        sub(/\r$/, "")
        line = $0
        sub(/^task\//, "", line)
        if (match(line, /^[0-9]+\.C?[0-9]+/)) closed[substr(line, 1, RLENGTH)] = 1
        next
    }
    { sub(/\r$/, "") }
    /^### Phase /  { phase = $0; sub(/^### /, "", phase); in_tasks = 1; next }
    /^## /         { in_tasks = 0 }
    in_tasks && /^\| *[0-9]+\.C?[0-9]+ *\|/ {
        # A table cell writes a literal pipe as \|, which is set aside for
        # the split and put back after it. A bare one makes a fourth cell,
        # and a row read wrong is worse than a row refused.
        row = $0
        gsub(/\\\|/, "\001", row)
        n = split(row, cell, "|")
        id = cell[2];   gsub(/^ +| +$/, "", id)
        if (n != 5) {
            printf "row %s has %d cells and a task row has three; write a literal | as \\|\n", id, n - 2 | "cat 1>&2"
            bad = 1
            exit
        }
        task = cell[3]; gsub(/^ +| +$/, "", task); gsub(/\001/, "|", task)
        done = cell[4]; gsub(/^ +| +$/, "", done); gsub(/\001/, "|", done)
        rows++
        if (id in seen) next
        seen[id] = 1
        if (id in closed) next
        if (task ~ /^Landed /) next
        if (task ~ /^Waits on /) { waiting = waiting (waiting == "" ? "" : ", ") id; next }
        printf "%s\n%s\n\n%s\n\nDone when: %s\n", id, phase, task, done
        found = 1
        exit
    }
    END {
        if (bad) exit 2
        # POSIX awk promises no /dev/stderr, and every awk can pipe to cat.
        if (!rows) { print "no task row under a Phase heading in the plan" | "cat 1>&2"; exit 2 }
        if (waiting != "") printf "\nPassed over, waiting: %s\n", waiting
        if (!found) { print "every row in the plan is closed or waiting"; exit 1 }
    }
' "$CLOSED" "$PLAN"
