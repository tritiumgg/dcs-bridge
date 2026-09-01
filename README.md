# Scaffolding

Unzip at the repository root. The tree is the tree you should end up with.

No project code exists yet. This is your documents, with the three
specifications frozen, plus the smallest tooling that makes them usable and
gets a Windows build out of CI.

## Start

1. **Fill the two slots in `CLAUDE.md`** — the name and the description, both
   marked `<!-- FILL: ... -->`. Nothing else names or describes the project.

2. **Make the scripts executable and check they run.**

   ```
   chmod +x tools/*.sh .claude/hooks/*.sh
   tools/ledger.sh codes
   tools/ledger.sh lint
   ```

   ```
   tools/statecheck.sh
   ```

   `lint` reads all 553 ledger rows and every anchor. `statecheck` holds
   `STATE.md` to a line budget per section. Both should exit 0.

3. **Read `STATE.md`.** It is the entry point and exit point of every session,
   and it already carries the open questions from the audit, including two that
   only you can settle.

4. **Read `docs/audit.md`.** It records what the documents disagree about,
   checked once before anything was built. Two problems in it are worth
   deciding on before Phase 4 and Phase 5.

5. **Initialize git and commit.** `.gitattributes` is load-bearing: it disables
   line-ending conversion, without which every ledger stamp breaks on a Windows
   checkout.

6. **Start Phase 1, task 1.1.** The workflows fail until the Cargo workspace
   exists. That is deliberate: your plan builds the pipeline before any
   behaviour, so every later phase flows through one already known to work.

## What ships here

| Path | What it is |
|---|---|
| `README.md` | This file. Replace it when the project has something to describe. |
| `CLAUDE.md` | Root instructions. Two fillable slots. |
| `STATE.md` | The handoff between sessions: in progress, next, and what carries forward. |
| `docs/specs/*/` | Your three specifications with their ledgers and glossaries. Frozen. |
| `docs/plan/` | Your plan with its ledger and glossary. Changes as build order does. |
| `docs/index.tsv` | Document code, path, and whether it is frozen. |
| `docs/audit.md` | What the documents disagree about, checked once. |
| `docs/conventions/documents.md` | Which are frozen, how to read them, the ledger format. |
| `docs/conventions/decision-records.md` | Where the build departs from them. |
| `docs/decisions/TEMPLATE.md` | Copy this to start a record. |
| `docs/decisions/0001-specifications-are-frozen.md` | The first record, and the worked example. |
| `vendor/lua/lua.def` | Your import definition for DCS's Lua, unchanged. |
| `tools/ledger.sh` | Locate and retrieve specification text. Lint and stamp. |
| `tools/mkimplib.sh` | Build the Windows import library from the `.def`. |
| `tools/statecheck.sh` | Enforce `STATE.md`'s per-section line budgets. |
| `.github/workflows/ci.yml` | Three-host checks, guards, Windows cross-build. |
| `.github/workflows/release.yml` | Tag-triggered build, zip, checksums, release. |
| `.github/workflows/version-bump.yml` | Release-version bump, guarded by SPEC §13.3. |
| `rust-toolchain.toml` | One toolchain across three hosts and CI. |
| `.claude/settings.json` | Wires the read guard. |
| `.claude/hooks/guard-spec-reads.sh` | Refuses an unbounded `Read` of a specification. |
| `.gitattributes` | Disables line-ending conversion. |
| `.gitignore` | Cargo output and local Claude Code settings. |

Your source files moved and were renamed: `dcs-bridge-spec/dcs-bridge-spec.md`
is now `docs/specs/bridge/spec.md`, and so on. The eight stamp lines that named
a `-revised` file now name the living file. No document byte changed, and every
hash still matches.

## What you still have to create

| What | Why it is not here |
|---|---|
| The name and description in `CLAUDE.md` | Yours to write. |
| `LICENSE` | PLAN §4 says permissive. Which one is your call. |
| The Cargo workspace, `.proto`, and the broker's `build.rs` | Phase 1, tasks 1.1 to 1.7. |
| A decision on the two unmeasured claims in `docs/audit.md` | Both are scope calls. |
| How you track work beyond `STATE.md` | Deliberately absent. See below. |

## What is deliberately absent

**Per-task files.** The plan is 115 rows in build order and `STATE.md` holds
the working state. A file per row with a status field is a project management
system, and you do not need one to read a table.

**A cross-reference checker.** Every reference already resolves — 231 prefixed,
597 bare, 64 probe references, all checked once. They cannot break in the frozen
specifications. The plan can break one by renumbering; if that starts happening,
this is worth twenty lines of awk.

**A findings process.** Categories, severities and review cycles manage
specification edits that will not happen. What the documents get wrong is in
`docs/audit.md`; what you decide to do instead is a decision record.

## Building for Windows

`vendor/lua/lua.def` pins the 114 Lua symbols the broker may link against.
`tools/mkimplib.sh` turns it into an import library and tells you whether the
machine you are on can do it. `rust-toolchain.toml` pins the toolchain and the
`x86_64-pc-windows-msvc` target. The `windows-cross` job proves the path from a
Linux runner on every pull request.

The piece you write is the broker's `build.rs`. It runs the same probe and
emits the link directives:

```
llvm-dlltool -m i386:x86-64 -d vendor/lua/lua.def -l $OUT_DIR/lua.lib
llvm-lib /def:vendor/lua/lua.def /out:$OUT_DIR/lua.lib /machine:x64
lib.exe  /def:vendor/lua/lua.def /out:$OUT_DIR/lua.lib /machine:x64
```

Both LLVM commands are verified against this `.def` on a Linux host with no DCS
and no Windows machine. Each produces a library with 114 `__imp_` symbols
recording `lua.dll`. Then emit `cargo:rustc-link-search=native=<OUT_DIR>`,
`cargo:rustc-link-lib=dylib=lua` and
`cargo:rerun-if-changed=vendor/lua/lua.def`, guarded on the target being
`windows-msvc`. Task 2.1 builds the same crate host-native against a stock
Lua 5.1, and that path must not touch the `.def`.

Reimplement the probe in `build.rs` with `std::process::Command` rather than
calling the script, so a Windows host with no `sh` still builds.

## Two portability limits

**`sh` on Windows.** The tools and the hook are POSIX `sh`, resolved through
Git for Windows, which Claude Code already expects. Without it, nothing here
runs.

**The read guard has one hole.** `PreToolUse` fires on tool calls only, so a
file pulled in with an `@` reference loads whole. Closing it needs a permission
rule, which also blocks the bounded reads the hook permits:

```json
"permissions": { "deny": ["Read(./docs/specs/*/spec.md)", "Read(./docs/plan/plan.md)"] }
```

## Add these when the problem shows up

**Pin the actions to commit SHAs.** A major-version tag is mutable, so `@v6`
today is not `@v6` next month. Versions were current on 2026-09-01.

**A `.def` regeneration check.** `vendor/lua/lua.def` omits sixteen of the 130
symbols `lua.dll` exports: the nine `luaopen_*` openers SPEC §4 forbids calling
and seven SPEC §5.1.1 calls an artefact rather than an interface. A
regeneration that dumps the whole export table would make all sixteen linkable.
Add a check the first time a DCS update forces a re-measure.
