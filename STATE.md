# Working state

**Last updated:** 2026-10-21

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

Nothing. 4.10 is built; 4.11 is next.

*One task at most. Say what is done, what is not, and where to resume. Say what
is committed and what is only in the working tree. Say what is knowingly
broken. Empty this when the task closes.*

## Just finished

- **4.10** — with no `DCSBridge_load` the sim driver bootstraps itself: reads its 21 sim keys from `Config/DCSBridge.lua` with the hook driver's checks, lists `simdriver.d`, publishes `route = 'modified'`, logs one INFO or ERROR line to `dcs.log`, never raises; holds `package.loadlib` and returns an opener it never publishes; the shipped line is `pcall(dofile, ...)`; agent-verified under stock Lua, the edited `MissionScripting.lua` is the maintainer's (PR #103 "With DCS"). PR #103, ADR 0069.
- **4.9** — `absent` (no `net` or `net.dostring_in`) and `noprefix` with an unpositioned string answer are policy refusals: one ERROR line per `autoexec.cfg` line, the union with the operator's file read under a stub environment, then a latch `driver.enabled()` consults until DCS restarts; `unreached` and a positioned answer do not fire; agent-verified under stock Lua, the live refusal is the maintainer's (PR #102 "With DCS"). PR #102, ADR 0067, 0068.
- **4.8** — `injection.answer` sorts the `"server"` answer by shape into `loaded`, `epoch`, `failed`, `unknown`, `nil`, `invalid` and `noprefix`; each case but `loaded` logs one ERROR line of its own; `driver.inject` answers the case as a second value, `skipped`, `unprepared` or `unreached` before an answer; nothing disables; agent-verified under stock Lua, the live refusal text is the maintainer's (PR #101 "With DCS"). PR #101, ADR 0066.

*The last three at most, one line each. Git log holds the rest.*

## Next

**Task 4.11** — Hook-driver-side extension chain after `HookDriver.gen.lua`:
`HookDriver.builtin.lua`, then `hookdriver.d\` in name order, with
`hook_driver_disabled_files` honored and a raising file skipped and counted in
`hook_driver_files_failed_total` (ADR 0055). Done when a raising file in
`hookdriver.d\` is skipped and counted, the rest load, and an absent directory
is silent. Agent-verifiable under stock Lua with a stub write directory.

## After that

- **4.12** onward through Phase 4; 3.5 waits behind 7.6.

## Carries forward

Things that must not be lost between sessions. Delete an entry when it is
resolved, and say where. Mark an entry only the maintainer can settle. Ten
entries at most: an eleventh means something here is finished, or belongs in
`docs/decisions/` or `CLAUDE.md` instead.

- **Maintainer decision — the binding blacklist ships incomplete.** SPEC §4.2
  records a seventh crasher with two unattributed candidates and no probe. Task
  5.2 ships it anyway and task 6.3 calls into the same table. Needed before
  Phase 5. See `docs/audit.md`.
- **`buf breaking` has no baseline until the next release.** `v0.1.0` predates
  the schema, so `tools/schema-breaking.sh` reports that and passes. It starts
  comparing at the first tag whose tree carries `proto/`. Delete this then.
- **Ring sizes are provisional until task 9.7.** 2.18 took ADR 0009's
  outbound sizes, `ring_out_lossy_records` 3584, `ring_out_durable_records`
  512 and `ring_out_lifecycle_records` 256, and sized the commit ring, which
  has no key, at their sum (ADR 0028); 2.12 took the two inbound sizes, 1024
  and 256. PROBE-7 measures them seven phases later, so 2.18's done-when
  reopens at 9.7. The by-class drop counts wait for `stats` to report them.
- **SPEC §17's *Any (native module)* rows land with their behaviour.** Task 2.1
  built the carrier, `mise run lua`, and closed on that. Each row is owed by
  the task implementing what it describes. The plan's
  2.1 done-when reads as though all seventeen run at 2.1, which none of
  them can. Point-to-point landed at 2.8 on the acknowledgement, at 2.16 on
  the typed replies and at 2.12 on `poll` returning the id; capability at
  2.14, late join at 2.17 and topic filter and topic discovery at 2.20, in
  the Rust loopback tests, since a bare Lua
  5.1 opens no socket. ADR 0017, ADR 0023, ADR 0027, ADR 0029, ADR 0030.
- **Task 2.2's DLL load inside DCS is owed by 4.2's live run.** 4.2's start
  makes that load through `open`, the first `package.loadlib` a live install
  runs. Delete this when the maintainer's live run of 4.2's steps reports
  step 2 matched.
- **What 2.4 to 2.16 left to 9.7 and nobody.** Until 9.1 and 9.5, a
  `configure` that drops a token leaves its session open, and
  `max_unauthenticated_connections` and `auth_failures_per_min` bind nothing.
  `commit` allocates once
  per record and a connection drains one frame per socket call, one record
  in forty at a 20000-record burst on Windows loopback; PROBE-7 at 9.7
  prices both. ADR 0014.
  The registry refuses an adopter's capability, which the generator writes
  by number, and the fan-out mask holds numbers below 64 (ADR 0027); SPEC
  §14.4's range has no task. On Windows a
  full `LIFECYCLE` ring's close leaves the drainer blocked in `send` until
  the peer reads or resets; aborting it has no task (ADR 0028).
- **The loading flag has no setter until 6.5.** `Bridge::set_loading` picks
  `dcs_alive_threshold_loading_ms` and nothing in Lua calls it, so a load
  over 30 s reads as a dead sim until `MissionLoadBegan` sets it. Delete
  this when 6.5 closes.
- **4.11 and 4.12 build on `state.gen`.** 4.3 loads `HookDriver.gen.lua` at
  start, fatally; 4.11's chain starts after it, 4.12 registers its tables
  without a second load (ADR 0055). Delete when 4.12 closes.
- **4.4's live reload check is owed by 4.12's PR.** With `ReloadConfig` sendable,
  set `ring_out_lossy_records = 64` and expect `configured: ... pending_restart 1`
  then `WARN ring_out_lossy_records: effective 3584, file 64, waits for a DCS
  restart`; a live key moves; a file that raises logs one ERROR and nothing else.
- **Task 7.10 does not exist.** Phase 7 runs 7.1 to 7.9 then 7.11, with no note
  explaining the gap. Retired ID or omission, unresolved.
