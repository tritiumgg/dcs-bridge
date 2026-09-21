#!/bin/sh
# Run tools/nexttask.sh against a small plan and check the row it names.
#
# The script picks what a session builds next, so a wrong answer costs a
# task's worth of work on the wrong row. Each case gives it a closed list and
# says which id must come back, or which exit code.
#
# POSIX sh only. Run it by hand, or through mise run docs.
set -u

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
NEXT=$ROOT/tools/nexttask.sh

DIR=$(mktemp -d "${TMPDIR:-/tmp}/nexttasktest.XXXXXX")
trap 'rm -rf "$DIR"' EXIT

cat >"$DIR/plan.md" <<'EOF'
# A plan

### 1.1 CLI increments

| ID | Verb |
|---|---|
| 2.C1 | `tail`, which is a row of another table and not a task row |

### Phase 1 — Repository

| ID | Task | Done when |
|---|---|---|
| 1.1 | The workspace. | It builds. |
| 1.2 | The gate. | It refuses. |
| 1.3 | Waits on the maintainer: the rule. | It is set. |

### Phase 2 — The broker

| ID | Task | Done when |
|---|---|---|
| 2.1 | The carrier. | It loads. |
| 2.C1 | CLI `tail`. | It prints. |
| 2.10 | The schema. | It is served. |
| 2.19 | Landed with 2.1: counted and nothing more. | The wire carries it. |
| 2.2 | The surface. | It opens. |

| Milestone | Reached when | Rows |
|---|---|---|
| **M2.1** | A person sees it. | 2.1, 2.2 |

## 2. What ships alongside

| ID | Task | Done when |
|---|---|---|
| 9.9 | A row outside the task breakdown. | Never. |
EOF

fail=0
n=0

# expect_id <id> <closed lines...>: the first line printed is <id>
expect_id() {
    want=$1; shift
    n=$((n + 1))
    : >"$DIR/closed"
    for line in "$@"; do printf '%s\n' "$line" >>"$DIR/closed"; done
    got=$(sh "$NEXT" --plan "$DIR/plan.md" --closed "$DIR/closed" 2>/dev/null | sed -n 1p)
    if [ "$got" != "$want" ]; then
        printf 'FAIL want %s got %s  closed: %s\n' "$want" "${got:-nothing}" "$*" >&2
        fail=1
    fi
}

# expect_exit <code> <args...>
expect_exit() {
    want=$1; shift
    n=$((n + 1))
    sh "$NEXT" "$@" >/dev/null 2>&1
    got=$?
    if [ "$got" -ne "$want" ]; then
        printf 'FAIL want exit %d got %d  %s\n' "$want" "$got" "$*" >&2
        fail=1
    fi
}

# Nothing closed: the first task row, not the row of the table before Phase 1.
expect_id 1.1

# Branch names and bare ids both close a row, and order in the list is free.
expect_id 1.2 task/1.1-cargo-workspace
expect_id 2.1 1.2 task/1.1-cargo-workspace

# A branch that is not a task branch closes nothing.
expect_id 1.1 fix/miri-attach-detach-wake docs/plan-drop-branch-tables

# A task that landed as several branches is closed by any of them.
expect_id 2.C1 1.1 1.2 task/2.1-3-load-test

# 2.1 closed does not close 2.10 or 2.19, and 2.C1 is its own id.
expect_id 2.10 1.1 1.2 task/2.1-carrier task/2.C1-tail

# A row that landed with another is passed over for the row after it.
expect_id 2.2 1.1 1.2 2.1 2.C1 task/2.10-schema

# A row that waits is passed over, and named after the row that is next.
n=$((n + 1))
printf '1.1\n1.2\n' >"$DIR/closed"
if ! sh "$NEXT" --plan "$DIR/plan.md" --closed "$DIR/closed" | grep -q '^Passed over, waiting: 1\.3$'; then
    printf 'FAIL the waiting row 1.3 was not named\n' >&2
    fail=1
fi

# A row that has not been reached is not named as passed over.
n=$((n + 1))
: >"$DIR/closed"
if sh "$NEXT" --plan "$DIR/plan.md" --closed "$DIR/closed" | grep -q '^Passed over'; then
    printf 'FAIL a waiting row past the next one was named\n' >&2
    fail=1
fi

# Every row closed or waiting: exit 1, and the table under "## 2." is never
# reached.
printf '1.1\n1.2\n2.1\n2.C1\n2.10\n2.2\n' >"$DIR/all"
expect_exit 1 --plan "$DIR/plan.md" --closed "$DIR/all"

# A literal pipe written \| stays inside its cell, in both cells.
cat >"$DIR/pipes.md" <<'EOF'
### Phase 1 — Pipes

| ID | Task | Done when |
|---|---|---|
| 1.1 | Run `a\|b` in the configuration. | It prints `x\|y`. |
| 1.2 | Run `a|b` with the pipe bare. | It prints. |
EOF
n=$((n + 1))
: >"$DIR/closed"
out=$(sh "$NEXT" --plan "$DIR/pipes.md" --closed "$DIR/closed" 2>/dev/null)
case $out in
    *'Run `a|b` in the configuration.'*'Done when: It prints `x|y`.'*) ;;
    *) printf 'FAIL an escaped pipe moved a cell: %s\n' "$out" >&2; fail=1 ;;
esac

# A bare one makes a fourth cell, and the row is refused and not misread.
printf '1.1\n' >"$DIR/one"
expect_exit 2 --plan "$DIR/pipes.md" --closed "$DIR/one"

# A plan with no task row under a Phase heading is not a plan all closed.
printf '# A plan\n\n| ID | Task | Done when |\n|---|---|---|\n| 1.1 | A row. | Never. |\n' >"$DIR/nophase.md"
expect_exit 2 --plan "$DIR/nophase.md" --closed "$DIR/closed"

# Usage errors.
expect_exit 2 --plan "$DIR/missing.md" --closed "$DIR/all"
expect_exit 2 --plan "$DIR/plan.md" --closed "$DIR/missing"
expect_exit 2 --closed
expect_exit 2 --what

if [ "$fail" -eq 0 ]; then
    printf '%d nexttask cases pass\n' "$n"
fi
exit "$fail"
