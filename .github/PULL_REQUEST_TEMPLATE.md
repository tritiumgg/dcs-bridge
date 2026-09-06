## Summary

<!--
One to three sentences: what this changes and why it is needed, written for
someone who has not read the plan. Do not repeat the title. End with what
the change is reviewed against: the plan task or the decision record.
-->

## Details

<!--
Describe behavior, not files. Delete either subsection that is empty.
-->

### Visible to users

<!--
What a user notices, one bullet each.
-->

### Not visible to users

<!--
Refactors, dependency bumps, schema and tooling changes, one bullet each,
with why each changes nothing a user sees.
-->

## README

<!--
Always present. What this change alters in what a user downloads, installs,
configures or runs, and the README paragraph that now says so. Name any
"not final" or "planned" note the change takes out. When nothing a user
sees changed, say so in one sentence.
-->

## Screenshots

<!--
One image per change a user sees, with a line naming what to look at. Show
before and after when the change alters something that already exists.
Delete the section when nothing visible changed.
-->

## Testing

<!--
Numbered steps a reviewer follows, not a record of who has run them. Start
from a clean checkout and leave out anything CI already runs. Delete the
section when CI covers everything.

Each step is one full imperative sentence, of two kinds:

  - an action: "Run `mise run check`." "Open a mission with one unit."
  - a check: "Verify the tail prints one line per frame and no `gap`."

Place a Verify wherever the reviewer needs to know the steps so far worked
before going on.

Give one numbered list for the common path. Add a heading per platform only
where the steps differ (Windows in PowerShell, macOS and Linux in bash), and
repeat nothing the common list covers.

Steps that need DCS go under their own heading: build the artifacts, install
them into the write directory, edit any files (say which and the exact
edit), then what to do in DCS, with a Verify after each thing the reviewer
should see.
-->

1.

### With DCS

1.

### Not covered

<!--
Every part of the task's completion condition these steps do not reach, and
where that gap is recorded (STATE.md, an issue, the plan). Delete the
subsection when the steps reach all of it.
-->

## Notes for reviewers

<!--
Optional. Where to start, decisions worth challenging, follow-up work left
out on purpose, anything temporary, and what running the steps produced
where that matters: a measurement, a log excerpt, a run that failed and why.
Delete the section when empty.
-->

Closes #NNN

<!--
Drop the line above when no issue exists.
-->
