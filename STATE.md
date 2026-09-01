# Working state

**Last updated:** 2026-09-01, task 1.1 written and green on macOS.

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

**Task 1.1.** Workspace written: `crates/{broker,cli,generator}`, root
`Cargo.toml`, `rustfmt.toml`, workspace lints, README, `mise.toml` tasks. All
of it is working tree only; nothing is committed. Nothing is knowingly broken.

`mise run check` — fmt, clippy `-D warnings`, build, test — passes on macOS.
**Only a maintainer can close this**, by reading the three-host CI matrix. An
agent on one host cannot observe the Linux or Windows leg.

## Just finished

- `rust-toolchain.toml` is the only file naming the Rust toolchain, and mise
  reads it. `mise.toml` clears `RUSTUP_TOOLCHAIN` so the components and the
  Windows target survive.

*The last three at most, one line each. Git log holds the rest.*

## Next

**Task 1.2** — Actions gating a pull request on all three hosts. Mostly
written; the task is making it pass, which also closes 1.1.

Verified by a maintainer reading a pull request's checks. An agent can read the
workflow but cannot run it.

## After that

- **1.3** — The broker's `build.rs`, turning `vendor/lua/lua.def` into an
  import library. Commands are verified and written up in the README.
- **1.4** — Windows cross-build through cargo-xwin. The target arrives from
  `rust-toolchain.toml`; `cargo check --target x86_64-pc-windows-msvc` passes.

## Carries forward

Things that must not be lost between sessions. Delete an entry when it is
resolved, and say where. Mark an entry only the maintainer can settle. Ten
entries at most: an eleventh means something here is finished, or belongs in
`docs/decisions/` or `CLAUDE.md` instead.

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
