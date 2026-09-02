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

Nothing. 2.3 is closed; 2.4 is next.

*One task at most. Say what is done, what is not, and where to resume. Say what
is committed and what is only in the working tree. Say what is knowingly
broken. Empty this when the task closes.*

## Just finished

- **2.2** — one `Bridge` per process, two states' tables over it, PRs #12, #13
  and #14. ADR 0007. Its banner clause is carried forward.
- **2.3** — a fixed-size ring, one producer, one consumer, drop-oldest with a
  counter, PRs #18 and #19. ADR 0008, which says what Loom does not settle.

*The last three at most, one line each. Git log holds the rest.*

## Next

**Task 2.4** — one producer ring and a writer thread fanning out to
per-connection queues. `ring::Ring` is the primitive; what 2.4 adds is who owns
each end. Decide there whether a per-connection ring is crossed by two threads
at all, because the specification never says who drains one to its socket.

**An agent sees the fan-out itself**, in a test with no DCS present. The
done-when is a cost claim — that adding a consumer does not move logic-thread
cost per record — and a runner's timings cannot carry that. Land a benchmark an
agent can run, and leave the figure to the maintainer.

## After that

- **2.5** — put calls emitting protobuf tags and values, decoded by a stock
  library including a non-minimal length varint.
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
  Task 2.18 evicts the oldest non-`LIFECYCLE` record, which a single ring can
  only answer by refusing when its oldest is one. A ring per class makes the
  rule O(1) and turns `ring_out_lifecycle_reserve` into an allocation, and pays
  by reordering across classes, which per-connection `seq` may forbid. Needed
  before 2.18. ADR 0008 names it.
- **Task 7.10 does not exist.** Phase 7 runs 7.1 to 7.9 then 7.11, with no note
  explaining the gap. Retired ID or omission, unresolved.
