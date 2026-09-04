#!/bin/sh
# Refuse a specification citation in the code.
#
# A comment, a doc comment or an error message says why the code is the way it
# is, in its own words. A document code and a section number send the reader
# somewhere else to find that out, and the frozen documents are the worst
# destination for it: they are not maintained, the build drifts from them, and
# a section that moves leaves the citation pointing at nothing.
#
# Where a choice needs an argument longer than a comment, cite the decision
# record that holds it. Those are numbered, never renumbered, and written to be
# read years later.
#
# A task ID and the plan's word "done-when" are refused for the same reason.
# Tasks are ephemeral: the plan retires when the build ships, and a comment
# reading "task 1.3" or "the done-when" then points at nothing. The comment
# says what the test proves in its own words.
#
# The documents themselves are exempt, and so is anything under docs/. They
# cite each other by design.
#
# POSIX sh only. Run it by hand, or through mise run check.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

# Document codes followed by a number, in any of the spellings the documents
# use, plus the bare "Section N" form a document uses to cite itself.
PATTERN='(SPEC|SIM|HOOK|PLAN)[[:space:]]*§?[[:space:]]*[0-9]|§[0-9]|Section[[:space:]]+[0-9]|[Dd]one-when|[Tt]ask [0-9]+\.[0-9]+'

# git ls-files rather than find, so an untracked scratch file is not the thing
# that fails somebody's build.
# This script names the patterns it refuses, so it skips itself.
FILES=$(git ls-files | grep -vE '^(docs/|README\.md$|STATE\.md$|CLAUDE\.md$|tools/nospecrefs\.sh$)')
[ -n "$FILES" ] || { printf 'no files to check. Is this a checkout?\n' >&2; exit 2; }

# grep exits 1 when it matches nothing, which is the passing case here.
HITS=$(printf '%s\n' "$FILES" | xargs grep -nE "$PATTERN" 2>/dev/null || true)

if [ -z "$HITS" ]; then
    printf 'no specification citation in the code\n'
    exit 0
fi

printf '%s\n' "$HITS" >&2
printf '\n' >&2
printf 'The code cites a specification, a task or a done-when. Say why the\n' >&2
printf 'code is the way it is instead, in the comment itself, and cite a\n' >&2
printf 'decision record where the reasoning is too long to sit in a comment.\n\n' >&2
printf 'Documents may cite documents: docs/, README.md, STATE.md and CLAUDE.md\n' >&2
printf 'are exempt.\n' >&2
exit 1
