## Summary

<!--
A paragraph a reader can stop after: what the change does and why, ending
with what it is reviewed against (the plan task, the decision record, or
for a stacked branch the claim this branch alone makes).
-->

## Details

<!--
What Summary left out: the choice behind the change and the alternatives
it passed over, what the diff leaves alone and why, and anything the
reviewer needs to know before reading it. Not a file list.
-->

## Testing

<!--
How anyone tests this change, not a record of who has. Start from a
clean checkout. Write "none" under a heading with nothing in it, rather
than removing the heading.

Every step is one of two kinds, each an imperative sentence:

  - an action: "Run `mise run check`." "Open a mission with one unit."
  - a check: "Verify the tail prints one line per frame and no `gap`."

Place a Verify step wherever the tester needs to know the steps so far
worked before going on. Several actions, a Verify, more actions, another
Verify is the expected shape.

Without DCS: builds, unit tests, guards, loopback runs. Windows steps in
PowerShell, macOS and Linux steps in bash. Where the two are the same
command, give it once under "All platforms".

With DCS: build the artifacts, install them into the write directory,
edit any files (say which and the exact edit), then what to do in DCS,
with a Verify after each thing the tester should see.

Not covered: every part of the done-when these steps do not reach, and
where that gap is recorded (STATE.md, an issue, the plan).
-->

### Without DCS

#### All platforms

1.

#### Windows (PowerShell)

1.

#### macOS and Linux (bash)

1.

### With DCS

1.

### Not covered

## Results

<!--
Optional. What running the steps produced: log excerpts, measurements,
screenshots, a run that failed and why. Delete the section if empty.
-->
