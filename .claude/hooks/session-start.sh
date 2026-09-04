#!/bin/sh
# SessionStart: put the handoff into context before the first prompt.
#
# STATE.md is the handoff between sessions and CLAUDE.md says to read it
# before anything else. Printing it here makes that automatic. The README's
# open claims and the working tree come with it, because a session that
# starts on a dirty tree or a topic branch should know at once.
#
# Whatever a SessionStart hook prints to stdout is added to the context.
# A compaction keeps its own summary, so nothing is printed then.

. "$(dirname -- "$0")/payload.sh"

source=$(parse source)
[ "$source" = "compact" ] && exit 0

cd "$(project_root)" || exit 0

if [ -f STATE.md ]; then
    printf '## STATE.md\n\n'
    cat STATE.md
    printf '\n'
fi

if [ -f tools/readmeopen.sh ]; then
    printf '## README open claims\n\n'
    sh tools/readmeopen.sh 2>/dev/null
    printf '\n'
fi

if git rev-parse --git-dir >/dev/null 2>&1; then
    printf '## Working tree\n\n'
    printf 'branch: %s\n' "$(git symbolic-ref --short -q HEAD 2>/dev/null || git rev-parse --short HEAD)"
    status=$(git status --short 2>/dev/null | head -20)
    if [ -n "$status" ]; then
        printf '%s\n' "$status"
    else
        printf 'clean\n'
    fi
fi
exit 0
