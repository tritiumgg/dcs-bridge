# Working state

**Last updated:** 2026-09-08

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

Nothing. 2.16 is closed and M2.2 is re-measured; 2.12 is next.

*One task at most. Say what is done, what is not, and where to resume. Say what
is committed and what is only in the working tree. Say what is knowingly
broken. Empty this when the task closes.*

## Just finished

- **2.16** — the four registration calls, the merge, and `begin` refusing
  an unregistered topic. PR #75, ADR 0023.
- **2.11** — `shim.tick` and `shim.epoch`: the heartbeat, the stamp. PR #74.
- **2.C3** — `dcsb schema`: the set fetched, checked, written. PR #73.

*The last three at most, one line each. Git log holds the rest.*

## Next

**Task 2.12** — two inbound rings, drop-newest, sized from configuration;
the reader pushes to the ring the route map names, reading the URL under
`max_type_url_bytes` ahead of the decode; `poll(target)` returns the
connection id, the topic and the bytes; an unrouted topic is dropped and
counted in `unrouted_topic_total`. Done when `poll(target)` returns a sent
record from the right ring and only that ring, and an unregistered topic is
delivered nowhere. The plan lists its two runs of commits.

**An agent verifies** the rings and the routing over loopback and
`tests/lua/poll.lua`; the live steps wait for 2.C4's `dcsb send`.

## After that

- **M2.2**: after 2.12, 2.C4, then 2.13, 2.14.
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
- **Ring sizes are provisional until task 9.7.** 2.15 took the
  specification's defaults, `ring_out_records` 4096 and
  `ring_out_lifecycle_reserve` 64, and sized the commit ring as one
  connection's ring, which has no key; 2.18 picks the rest. PROBE-7
  measures them seven phases later, so 2.18's done-when reopens at 9.7.
- **SPEC §17's *Any (native module)* rows land with their behaviour.** Task 2.1
  built the carrier, `mise run lua`, and closed on that. Each row is owed by
  the task implementing what it describes: capability at 2.14, late join at
  2.17, topic filter at 2.20. The plan's 2.1 done-when reads as though all
  seventeen run at 2.1, which none of them can. Point-to-point landed at 2.8
  on the acknowledgement and at 2.16 on the typed replies; `poll` returning
  the id is 2.12's. ADR 0017, ADR 0023.
- **Task 2.2's load banner is owed by 4.1.** SPEC §13 addresses the banner to
  the Lua side and SPEC §15 has `doctor` check it. Nothing makes the DLL write
  one, and SPEC §4 leaves it no `io`. Delete this when 4.1 closes.
- **What 2.4 to 2.16 left to 2.12, 2.13 and 9.7.** Every broker key is in
  `Config` since 2.15, and nothing reads these yet: the rate limits
  (`rejected_max_per_sec`, `busy_max_per_sec`, `inbound_records_per_sec`
  and its total) wait for 2.12's routing, and SPEC §17 "Broker hardening"
  (`max_unauthenticated_connections`, `auth_failures_per_min`, revocation
  dropping sessions) has no owner: a later `configure` swaps the token table
  and leaves a session under a dropped token open. `SetEnabled` without
  `reload` is counted, not answered, until 2.13; an unknown topic after auth
  closes the connection until 2.12 routes, which also reads the URL cap
  ahead of the decode. `commit` allocates once per record and a connection
  drains one frame per socket call, one record in forty at a 20000-record
  burst on Windows loopback; PROBE-7 at 9.7 prices both. ADR 0014. The sim
  driver's schema hash SPEC §8.3 has ride `shim.classes` has no owner; 4.3
  carries the hook driver's on `configure`. The registry refuses an
  adopter's capability; SPEC §14.4's range has no task. Windows CI builds
  without `dcs-lua`, so nothing compiles the Lua surface for the product
  and a raise-shape regression (`push_error`) is caught only live.
- **The loading flag has no setter until 6.5.** `Bridge::set_loading` picks
  `dcs_alive_threshold_loading_ms` and nothing in Lua calls it, so a load
  over 30 s reads as a dead sim until `MissionLoadBegan` sets it. Delete
  this when 6.5 closes.
- **Task 7.10 does not exist.** Phase 7 runs 7.1 to 7.9 then 7.11, with no note
  explaining the gap. Retired ID or omission, unresolved.
