---
name: task
description: Work one plan task from brief to pull request. Takes a plan task id, or none to take the next row that is ready, or a branch name to review a branch that is no plan task. Runs the task-brief and task-review workflows.
disable-model-invocation: true
arguments: [id]
---

# Work a plan task

The maintainer invoked this with: `$ARGUMENTS`

The argument is a plan task id, nothing, or a branch name; the last section
covers a branch name. Invoking this skill is the
maintainer's consent to run the `task-brief` and `task-review` workflows for
this one task. It is not consent to merge; `CLAUDE.md` holds that rule.

## 1. Settle which task

With no id, run `sh tools/nexttask.sh`. Its first line is the id. With an id,
run it anyway: when the id given is past the row it names, say so in one line
and go on, because the order is the maintainer's to override.

When the script exits 3, GitHub could not be asked. Take the id from
`STATE.md` "Next" and say the choice was not checked against the merged pull
requests.

Then check that the task may start, and stop with a plain report when it may
not:

- `STATE.md` "In progress" has content: resume that task and pick no other.
- `gh pr list --state open` shows a `task/` branch: report it as waiting. One
  task runs at a time.
- A "Carries forward" entry marked for the maintainer says it is needed before
  this task's phase: name the decision and stop. Only the maintainer settles it.
- The script printed "Passed over, waiting": repeat that line to the maintainer.

Where `STATE.md` "Next" names another task than the one settled here, say so,
follow the plan's order, and correct "Next" in the task's `STATE.md` commit.

## 2. Brief

Run the `task-brief` workflow with `{task: "<id>"}`.

Show the maintainer what it returns, in this order: the ordered commit list
with each estimate, who verifies each clause, the live-install steps, the
README paragraphs and decision records the task owes, the risks, the critic's
problems that still stand, and the questions. Say when the plan was revised,
when the critic returned nothing, and which readers returned nothing.

Wait for the maintainer to agree to the commit list before creating the
branch. A question in the brief is asked, not guessed at.

## 3. Build

In this session, not in a workflow: a commit needs the maintainer to approve
its signature, and the commits land in order on one branch.

Create `task/<id>-<summary>`. Make the commits in the brief's order, each
passing `mise run check`. Write "who verifies" under the task in `STATE.md`
from the brief. Where a commit outgrows its estimate past the split point, cut
it and say so; the brief is an estimate and the code is the fact.

## 4. Review

When the branch is finished, start both at once:

- the `task-review` workflow with `{task: "<id>", claim: "<the claim the branch makes>"}`
- `mise run ci`, in this session, reading its own exit code

Fix each finding that stands in a commit of its own, or squashed into the
commit that introduced it while nothing is pushed. Read the refuted findings
too, and reopen one that was argued away wrongly. A side the workflow reports
as unreviewed is reviewed again before the push, not skipped.

## 5. Hand over

Push the branch and open the pull request as `CLAUDE.md` describes, with the
live-install steps from the brief under "With DCS". Report it as waiting and
stop.

## A branch that is no plan task

Invoked with a branch name in place of an id, such as a `fix/`, `docs/` or
`build/` branch, skip to step 4: there is no plan row to brief. Run
`task-review` with `{branch: "<name>", claim: "..."}`, then hand over as in
step 5.
