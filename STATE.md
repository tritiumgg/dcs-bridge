# Working state

**Last updated:** 2026-09-03

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

Nothing. 2.C1 is closed; 2.8 is next.

*One task at most. Say what is done, what is not, and where to resume. Say what
is committed and what is only in the working tree. Say what is knowingly
broken. Empty this when the task closes.*

## Just finished

- **2.7** — `Any` wrapper, per-connection `seq`, the listener, a frame per
  record; `commit` queues and returns a boolean. PRs #27 to #31. ADR 0014.
- **2.C1** — `dcsb tail` over clap and prost; a stalled socket's evictions
  read as a `seq` gap in a loopback test. PRs #32 and #33; #33 carries the live steps.

*The last three at most, one line each. Git log holds the rest.*

## Next

**Task 2.8** — `begin_to` and per-connection addressing; `poll` returns the
connection id; ids unique for the process and never reused. A `begin_to` on a
topic the schema did not mark a reply or an acknowledgement is refused and
counted in `misaddressed_total`. Done when two `dcsb tail` sessions show a
`begin_to` record reaching one and a `begin` record reaching both, and a
hand-written `begin_to` on a fan-out topic is refused.

**An agent verifies both** with two loopback connections from a Rust test and
a refused `begin_to` from a stock Lua; **a person repeats the two-session
check** against an install, since no handshake exists to name a connection.

## After that

- **2.9** — handshake, then auth, then the five reader-thread answers: `Ping`,
  `Auth`, `GetSchema`, `SeqAck`, `SetEnabled`. The reader thread decodes them
  through `prost`, the one crate the shipped build takes; ADR 0016.
- **Phase 3** opens on `protoc-gen-dcsbridge-lua`, which reads the four message
  options this schema defines and splits its output by `Target`. It reads the
  plugin request through `prost-types`; ADR 0016.

## Carries forward

Things that must not be lost between sessions. Delete an entry when it is
resolved, and say where. Mark an entry only the maintainer can settle. Ten
entries at most: an eleventh means something here is finished, or belongs in
`docs/decisions/` or `CLAUDE.md` instead.

- **Maintainer decision — task 1.2's gate cannot be set on this plan.**
  `dcs-bridge` is private on GitHub Free, so branch protection and rulesets
  both answer 403. Make the repository public or upgrade to Pro; an agent can
  set the rule after that. Require `Preflight`, `Linux` and `Windows`;
  `macOS` runs weekly and on request, so it cannot be required.
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
- **What 2.4, 2.5 and 2.7 left to 2.15 and 9.7.** The module's first open
  starts the outbound path on `127.0.0.1:7742` with 4096-record rings, and each
  Lua state's record buffer is 1 MiB at open; `configure` owns all of it, and
  an open that cannot bind raises until then. `commit` allocates once per
  record on the logic thread, the copy the rings share; PROBE-7 at 9.7 prices
  it and would schedule a slab. ADR 0014. A connection drains one frame per
  socket call, and 2.C1's live check priced that: a 20000-record single-frame
  burst kept one record in about forty on Windows loopback with a consumer
  that never stalled, because a push is nanoseconds and a call is tens of
  microseconds. PR #36's one call per frame took it from seventy to forty.
  Batching a drain pass into one call is the rest; 2.18 or 9.7 owns it, and
  the ring size 2.15 picks should count it.
- **Task 7.10 does not exist.** Phase 7 runs 7.1 to 7.9 then 7.11, with no note
  explaining the gap. Retired ID or omission, unresolved.
