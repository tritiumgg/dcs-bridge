# Working state

**Last updated:** 2026-09-02

The handoff between sessions. Read it first; update it before a session ends,
not only when a task finishes. Stamp the date above each time; it carries a
date and nothing else, because what changed is what the sections below are
for.

**This file is loaded cold every session, so its size is a tax on all of them.**
Each section has a line budget and `tools/statecheck.sh` enforces it. Over
budget, nothing is deleted — it moves. A completion older than the last few
goes to git log. A choice with reasoning behind it becomes a decision record. A
durable fact about the project belongs in `CLAUDE.md`. A resolved carry-forward
is just deleted. Write entries as one or two lines, never paragraphs.

---

## In progress

Nothing. 2.4 is closed; 2.5 is next.

*One task at most. Say what is done, what is not, and where to resume. Say what
is committed and what is only in the working tree. Say what is knowingly
broken. Empty this when the task closes.*

## Just finished

- **2.3** — a fixed-size ring, one producer, one consumer, drop-oldest with a
  counter, PRs #18 and #19. ADR 0008, which says what Loom does not settle.
- **2.4** — one commit ring fanned out to a ring per connection by a writer
  thread, PRs #21 and #22. ADR 0011. The cost figure in #22 is the maintainer's.

*The last three at most, one line each. Git log holds the rest.*

## Next

**Task 2.5** — put calls emitting protobuf tags and values into a preallocated
buffer, no protobuf runtime. Done when a stock library decodes the output,
including a non-minimal length varint.

**An agent sees this itself**: a decoder in the test suite, with no DCS present.

## After that

- **2.6** — PROBE-3, put-call crossing cost, which also prices the fence and
  the wake ADR 0011 left unmeasured.
- **Phase 3** opens on `protoc-gen-dcsbridge-lua`, which reads the four message
  options this schema defines and splits its output by `Target`.

## Carries forward

Things that must not be lost between sessions. Delete an entry when it is
resolved, and say where. Mark an entry only the maintainer can settle. Ten
entries at most: an eleventh means something here is finished, or belongs in
`docs/decisions/` or `CLAUDE.md` instead.

- **Maintainer decision — task 1.2's gate cannot be set on this plan.**
  `dcs-bridge` is private on GitHub Free, so branch protection and rulesets
  both answer 403. Make the repository public or upgrade to Pro; an agent can
  set the rule after that. Require `Guards`, `Documents`, `ubuntu-latest`,
  `macos-latest`, `windows-latest`, `Windows cross-build from Linux`, `The ring
  under a model checker`.
- **Maintainer decision — the policy gate is unmeasured and no probe covers
  it.** Tasks 4.8, 4.9, 9.C1 and 10.2 rest on which `net.allow_dostring_in`
  value list is correct. Measure it, or ship the wider union and state the
  risk. Needed before Phase 4. See `docs/audit.md`.
- **Maintainer decision — the binding blacklist ships incomplete.** SPEC §4.2
  records a seventh crasher with two unattributed candidates and no probe. Task
  5.2 ships it anyway and task 6.3 calls into the same table. Needed before
  Phase 5. See `docs/audit.md`.
- **`buf breaking` has no baseline until the next release.** `v0.1.0` predates
  the schema, so `tools/schema-breaking.sh` reports that and passes. It starts
  comparing at the first tag whose tree carries `proto/`. Delete this then.
- **Ring sizes are provisional until task 9.7.** Tasks 2.15 and 2.18 pick
  values; PROBE-7 measures them seven phases later, so 2.18's done-when reopens
  at 9.7. Record the provisional reserve here when chosen.
- **SPEC §17's *Any (native module)* rows land with their behaviour.** Task 2.1
  built the carrier, `mise run lua`, and closed on that. Each row is owed by
  the task implementing what it describes: capability at 2.14, late join at
  2.17, topic filter at 2.20. The plan's 2.1 done-when reads as though all
  seventeen run at 2.1, which none of them can.
- **Task 2.2's load banner is owed by 4.1.** SPEC §13 addresses the banner to
  the Lua side and SPEC §15 has `doctor` check it. Nothing makes the DLL write
  one, and SPEC §4 leaves it no `io`. Delete this when 4.1 closes.
- **Maintainer decision — one outbound ring per connection, or one per class.**
  A slot is addressed by its record number, so a single ring cannot search for
  a non-`LIFECYCLE` victim the way 2.18 requires. `seq` is assigned before the
  drop decision, so a ring per class reorders nothing that a merge at drain
  cannot restore. What it changes is that a full `LIFECYCLE` ring disconnects
  where one ring would have evicted. Needed before 2.18. ADR 0008 names it.
- **Two things 2.4 left to later tasks.** The commit ring has no size key and
  takes a capacity parameter; 2.15 decides. A connection's thread has no way to
  wait on its ring; 2.7's socket thread needs the writer's flag handshake per
  connection, or a poll. ADR 0011 settles who drains, not how it waits.
- **Task 7.10 does not exist.** Phase 7 runs 7.1 to 7.9 then 7.11, with no note
  explaining the gap. Retired ID or omission, unresolved.
