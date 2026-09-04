#!/bin/sh
# Stop: refuse to end a turn that changed the tree without stamping STATE.md.
#
# STATE.md is updated before a session ends, not only when a task finishes,
# and its Last updated line carries the date. Nothing else enforces that. So
# when tracked files differ from HEAD and STATE.md is not stamped today, the
# stop is refused once, with the reason, and the model either updates the
# file or says in its reply why STATE.md does not change.
#
# stop_hook_active is true when the model is already continuing because of
# this hook. The second stop passes, so the refusal cannot loop.
#
# Exit 2 refuses the stop and hands stderr to the model.

. "$(dirname -- "$0")/payload.sh"

active=$(parse stop_hook_active)
[ "$active" = "true" ] && exit 0

cd "$(project_root)" || exit 0
git rev-parse --git-dir >/dev/null 2>&1 || exit 0

changed=$(git status --porcelain 2>/dev/null)
[ -n "$changed" ] || exit 0

today=$(date +%Y-%m-%d)
grep -q "^\*\*Last updated:\*\* $today\$" STATE.md 2>/dev/null && exit 0

cat >&2 <<EOF
The working tree has changes and STATE.md is not stamped $today.

STATE.md is the handoff between sessions and is updated before a session
ends, not only when a task finishes. Fill in "In progress" with what is done,
what is not, and where to resume, or move the task to "Just finished", and
set the Last updated line to $today. Or, where this turn changed nothing
STATE.md should carry, say so in one line of the reply and stop.
EOF
exit 2
