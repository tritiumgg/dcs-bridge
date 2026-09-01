#!/bin/sh
# Keep STATE.md small enough to read at the start of every session.
#
# The file is loaded cold each time, so its size is a tax on every session.
# Left alone it grows: completions accumulate, carry-forward entries are added
# and never deleted, and durable facts leak in from CLAUDE.md.
#
# Budgets are per section, because a single total lets one section eat the
# others. Exceeding one is not a failure of the file; it means something in
# that section belongs somewhere else. Where, per section, is in STATE.md.
#
# POSIX sh and awk only.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
FILE=${1:-$ROOT/STATE.md}

[ -f "$FILE" ] || { printf 'no state file at %s\n' "$FILE" >&2; exit 2; }

awk '
    BEGIN {
        # section                 lines
        budget["In progress"]     = 12
        budget["Just finished"]   = 8
        budget["Next"]            = 12
        budget["After that"]      = 10
        budget["Carries forward"] = 60
        total_lines = 120
        total_bytes = 8192
    }
    { bytes += length($0) + 1; sub(/\r$/, "") }
    # The stamp is a date and nothing else. Left open it collects a status
    # clause, which then says what "Just finished" already says two lines
    # below and goes stale on the next change. Intervals such as {4} are not
    # portable across awks, so the digits are spelled out.
    /^\*\*Last updated:\*\*/ {
        stamped = 1
        if ($0 !~ /^\*\*Last updated:\*\* [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/)
            malformed = $0
    }
    /^## / {
        sec = $0; sub(/^## /, "", sec)
        if (!(sec in seen)) { seen[sec] = 1; order[++k] = sec }
        next
    }
    sec != "" { n[sec]++ }
    END {
        printf "%-18s %6s %8s\n", "SECTION", "LINES", "BUDGET"
        for (i = 1; i <= k; i++) {
            s = order[i]
            if (!(s in budget)) {
                printf "%-18s %6d %8s  unknown section\n", s, n[s], "-"
                bad = 1
                continue
            }
            over = (n[s] > budget[s])
            printf "%-18s %6d %8d%s\n", s, n[s], budget[s], (over ? "  OVER" : "")
            if (over) { bad = 1; fat = 1 }
            got[s] = 1
        }
        for (s in budget)
            if (!(s in got)) { printf "%-18s %6s %8d  MISSING\n", s, "-", budget[s]; bad = 1 }

        printf "\n%-18s %6d %8d%s\n", "whole file", NR, total_lines, (NR > total_lines ? "  OVER" : "")
        printf "%-18s %6d %8d%s\n", "bytes", bytes, total_bytes, (bytes > total_bytes ? "  OVER" : "")
        printf "%-18s %6d\n", "approx tokens", int(bytes / 4)
        if (NR > total_lines || bytes > total_bytes) { bad = 1; fat = 1 }

        if (!stamped) { printf "\nno **Last updated:** line\n"; bad = 1 }
        else if (malformed != "") {
            printf "\nthe **Last updated:** line carries more than a date:\n\n"
            printf "  %s\n\n", malformed
            printf "It reads **Last updated:** YYYY-MM-DD and stops there.\n"
            printf "What changed goes in the sections below.\n"
            bad = 1
        }

        if (bad) {
            printf "\nSTATE.md failed the check.\n"
            if (!fat) exit 1
            printf "If a section is over budget, nothing is deleted. It moves:\n"
            printf "  a completion older than the last few  -> git log\n"
            printf "  a choice with reasoning behind it     -> docs/decisions/\n"
            printf "  a durable fact about the project      -> CLAUDE.md\n"
            printf "  a resolved carry-forward              -> delete it\n"
            exit 1
        }
        printf "\nwithin budget\n"
    }
' "$FILE"
