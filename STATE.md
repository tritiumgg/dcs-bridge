# Working state

**Last updated:** 2026-09-01, task 1.2's gate blocked, task 1.3 started.

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

**1.3** — `crates/broker/build.rs` generates the import library from
`vendor/lua/lua.def`, and the stub calls `lua_gettop` through it. The `dcs-lua`
feature keeps that off the host-native build; DR-0002. Host-native is green
here and the msvc link is unverified: this machine has neither LLVM nor
cargo-xwin, so CI's cross-build is the observer. Resume by reading it.

*One task at most. Say what is done, what is not, and where to resume. Say what
is committed and what is only in the working tree. Say what is knowingly
broken. Empty this when the task closes.*

## Just finished

- **1.1** — Workspace of three stub crates. Green on all three hosts, CI run
  33532382191 on `main`.
- **1.2** — CI runs fmt, clippy, build and test on all three hosts and passes,
  run 33533254306. The gate half is blocked; see the carry-forward.

*The last three at most, one line each. Git log holds the rest.*

## Next

**Task 1.3** — The broker's `build.rs` turns `vendor/lua/lua.def` into an import
library at build time, so the stub links against DCS's Lua from a host with no
DCS installed. SPEC §5.1.1. Commands written up in the README.

**An agent verifies this**: the msvc cross-build links here and in CI, and the
host-native build must not touch the `.def`.

## After that

- **1.4** — Cross-build already emits `lua-dcsbridge.dll` and `dcsb.exe` from
  the Linux runner, but only because the broker links nothing yet. Reopens at
  1.3. The `x86_64-pc-windows-gnu` fallback is still undocumented.

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
- **The broker builds twice.** A `cdylib` for `x86_64-pc-windows-msvc` linking
  the `.def`, and the same source host-native for task 2.1's tests. The
  host-native path must not touch the `.def`.
- **Task 7.10 does not exist.** Phase 7 runs 7.1 to 7.9 then 7.11, with no note
  explaining the gap. Retired ID or omission, unresolved.
