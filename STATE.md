# Working state

**Last updated:** 2026-09-01, 1.6 is green and waits on a merge.

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

**1.6** — PR #5 is green, CI run 33545252753, and waits on a merge. The zip
carries `schema.pb`, and the Linux runner's copy is byte-identical to a macOS
build. All committed. What is left is the merge.

*One task at most. Say what is done, what is not, and where to resume. Say what
is committed and what is only in the working tree. Say what is knowingly
broken. Empty this when the task closes.*

## Just finished

- **1.5** — Tag `v0.1.0` published four assets, release run 33540903624.
  `tools/stage-release.sh` stages them and CI runs it on every pull request.

*The last three at most, one line each. Git log holds the rest.*

## Next

**Task 1.7** — `buf breaking` against the previous release and the SPEC §8.4
ownership check. `buf lint` already runs in CI, so 1.7 adds the other two.

**An agent verifies this**: a breaking change and a stray `dcs.bridge` message
must each fail a CI run, both observable with nothing running.

## After that

- **2.1** — The host-native broker build a stock Lua 5.1 can load, so SPEC §17
  runs in CI with no DCS present.
- **2.2** — `luaopen_dcsbridge`, loaded by explicit path, with rings, sockets
  and the three registration maps process-global rather than per-state.

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
