#!/bin/sh
# PostToolUse on Bash: after a git commit, check the message at HEAD.
#
# Conventional Commits: type(scope): summary, imperative, under 72 characters.
# The message is read back from git rather than out of the command, because a
# commit message arrives by -m, by heredoc or by file and git sees them all
# the same way.
#
# Exit 2 hands stderr to the model. The fix is git commit --amend, which is
# safe here: the commit is seconds old and unpushed.

. "$(dirname -- "$0")/payload.sh"

tool=$(parse tool_name)
[ "$tool" = "Bash" ] || exit 0

cmd=$(parse command)
printf '%s\n' "$cmd" | grep -Eq 'git[[:space:]]+commit([[:space:]]|$)' || exit 0

cd "$(project_root)" || exit 0
subject=$(git log -1 --format=%s 2>/dev/null) || exit 0
[ -n "$subject" ] || exit 0

TYPES='build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test|bench'
why=""
if [ "${#subject}" -gt 72 ]; then
    why="the subject is ${#subject} characters and the limit is 72"
elif ! printf '%s\n' "$subject" | grep -Eq "^($TYPES)(\([A-Za-z0-9._/-]+\))?!?: [^[:space:]]"; then
    why="the subject does not read type(scope): summary, with type one of $TYPES"
fi
[ -n "$why" ] || exit 0

cat >&2 <<EOF
The commit at HEAD does not follow the project's commit format: $why.

    $subject

Rewrite it with git commit --amend. Conventional Commits: type(scope):
summary, imperative, under 72 characters, with a body when the change needs
explaining.
EOF
exit 2
