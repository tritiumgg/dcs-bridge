# Working state

**Last updated:** 2026-09-01, task 1.4 closed on one Windows target.

The handoff between sessions. Read it first; update it before a session ends,
not only when a task finishes. Stamp the line above each time.

**This file is loaded cold every session, so its size is a tax on all of them.**
Each section has a line budget and `tools/statecheck.sh` enforces it. Over
budget, nothing is deleted — it moves. A completion older than the last few
goes to git log. A choice with reasoning behind it becomes a decision record. A
durable fact about the project belongs in `CLAUDE.md`. A resolved carry-forward
is just deleted. Write entries as one or two lines, never paragraphs.

---

## In progress

Nothing. PR #2 carries 1.4 and is green but unmerged.

*One task at most. Say what is done, what is not, and where to resume. Say what
is committed and what is only in the working tree. Say what is knowingly
broken. Empty this when the task closes.*

## Just finished

- **1.3** — The cross-built DLL imports `lua.dll`, CI run 33534747477 on PR #1.
  DR-0002 settles the broker's two builds.
- **1.4** — Both artifacts cross-build from Linux, CI run 33537626474 on PR #2.
  DR-0003 drops the gnu fallback rather than building one.

*The last three at most, one line each. Git log holds the rest.*

## Next

**Task 1.5** — `release.yml` already builds, stages and publishes on a `v*`
tag. What is open: the write-directory zip ships a placeholder tree, and no tag
has ever run the workflow end to end.

**The maintainer verifies this**: only they may tag, and a tag is a release. An
agent gets as far as `workflow_dispatch` against an existing tag.

## After that

- **1.6** — The `.proto` and `schema.pb` under `buf`, for Phase 2 to serve.
- **1.7** — `buf lint` and `buf breaking` in CI, plus the SPEC §8.4 ownership
  check on `dcs.bridge`.

## Carries forward

Things that must not be lost between sessions. Delete an entry when it is
resolved, and say where. Mark an entry only the maintainer can settle. Ten
entries at most: an eleventh means something here is finished, or belongs in
`docs/decisions/` or `CLAUDE.md` instead.

- **Maintainer decision — task 1.2's gate cannot be set on this plan.**
  `dcs-bridge` is private on GitHub Free, so branch protection and rulesets
  both answer 403. Make the repository public or upgrade to Pro; an agent can
  set the rule after that. Require `Guards`, `Documents`, `ubuntu-latest`,
  `macos-latest`, `windows-latest`, `Windows cross-build from Linux`.
- **Maintainer decision — the policy gate is unmeasured and no probe covers
  it.** Tasks 4.8, 4.9, 9.C1 and 10.2 rest on which `net.allow_dostring_in`
  value list is correct. Measure it, or ship the wider union and state the
  risk. Needed before Phase 4. See `docs/audit.md`.
- **Maintainer decision — the binding blacklist ships incomplete.** SPEC §4.2
  records a seventh crasher with two unattributed candidates and no probe. Task
  5.2 ships it anyway and task 6.3 calls into the same table. Needed before
  Phase 5. See `docs/audit.md`.
- **Ring sizes are provisional until task 9.7.** Tasks 2.15 and 2.18 pick
  values; PROBE-7 measures them seven phases later, so 2.18's done-when reopens
  at 9.7. Record the provisional reserve here when chosen.
- **Task 7.10 does not exist.** Phase 7 runs 7.1 to 7.9 then 7.11, with no note
  explaining the gap. Retired ID or omission, unresolved.
