# DCS-Bridge

A bridge between DCS World and external applications, allowing them to interact
with and receive from the simulation state through events, subscriptions, and
commands. The bridge is comprised of three parts: a message broker that acts as
the data transport, and two scripts that interact with DCS World inside and
outside of the simulation.

Release version 0.1.0. The three crates are stubs. Phase 1 of `docs/plan/plan.md`
builds the release pipeline before any behaviour, so every later phase ships
through a path already known to work.

## Components

| # | Component | Language | Location | Lifetime |
|---|---|---|---|---|
| 1 | Broker | Rust | `Mods\services\DCSBridge\bin\lua-dcsbridge.dll` | DCS process |
| 2 | Hook driver | Lua 5.1 | `Scripts\Hooks\DCSBridge.lua` loader, `Mods\services\DCSBridge\lua\HookDriver.lua` payload | DCS process |
| 3 | Sim driver | Lua 5.1 | Injected into the `"server"` state | One mission |
| 4 | Generator | Rust | Build machine | Build time |

Everything outside the Lua files is Rust. The broker uses no garbage collector
and no language runtime, because a collector inside the DCS process can stop the
logic thread and that stops the sim for every player at once.

The workspace holds the three Rust crates:

| Crate | Package | Artifact |
|---|---|---|
| `crates/broker` | `lua-dcsbridge` | `lua_dcsbridge.dll`, renamed to `lua-dcsbridge.dll` |
| `crates/cli` | `dcsb` | `dcsb.exe` |
| `crates/generator` | `protoc-gen-dcsbridge-lua` | `protoc-gen-dcsbridge-lua.exe` |

Cargo rejects a hyphen in a library target name, so the broker's library is
`lua_dcsbridge` and the CI and release workflows rename the file. `protoc`
resolves a plugin by executable name, so the generator's binary name is fixed
by that contract.

## Build

Tool versions come from `mise.toml`. On a fresh checkout:

```sh
mise install
mise run check
```

`check` runs what CI gates a pull request on: `cargo fmt --all --check`,
`cargo clippy -D warnings`, `cargo build` and `cargo test`, against the host
target. `mise tasks` lists the rest.

Rust is the one tool `mise.toml` does not name a version for. `rust-toolchain.toml`
does, and mise reads it — `mise ls` shows rust sourced from that file, and
`mise install` provisions the channel it names. The file also carries
`components` and `targets`, which rustup installs on demand, so the Windows
cross-build's target arrives without a separate step.

Two settings in `mise.toml` make that work. mise's rust tool exports
`RUSTUP_TOOLCHAIN`, which overrides `rust-toolchain.toml` outright and discards
both lists, so `[env]` clears it; `idiomatic_version_file_enable_tools` then
lets mise read the version from the file. Change the toolchain by editing
`channel`, then `mise install`. Not with `mise use rust@<version>`: that writes
a version into `[tools]` and mise stops reading the file.

### Windows

The product target is `x86_64` Windows for as long as DCS runs nowhere else,
and it is cross-compiled, so no contributor needs a Windows machine to produce
a release artifact.

```sh
cargo install --locked cargo-xwin      # once
sh tools/mkimplib.sh                   # check this machine can build the import library
mise run windows
```

`vendor/lua/lua.def` pins the 114 Lua symbols the broker may link against.
`crates/broker/build.rs` turns it into the import library the DLL links, and
`tools/mkimplib.sh` does the same from a shell to report whether the machine you
are on can do it at all. A full LLVM install is the one prerequisite; a rustup
`llvm-tools` component ships neither `llvm-dlltool` nor `llvm-lib`.

The broker builds twice from one source, and the `dcs-lua` feature is which one
you get. On, the `cdylib` binds DCS's Lua through the `.def` — that is the
default, and it is what the cross-build and the release workflow take. Off, the
host-native build the tests run against never touches the `.def`. `mise run
check` and CI's three-host matrix pass `--no-default-features` for it, which is
what a plain `cargo test` on a Windows host needs too. DR-0002 says why the
default points that way.

## Versioning

The release version is `0.1.0`, and it is the only version this README states.
It lives under `[workspace.package]` in `Cargo.toml`. A tag `v<version>`
publishes a release; a tag carrying a hyphenated suffix, such as `v0.2.0-rc1`,
publishes to the prerelease channel.

Below 1.0 a minor bump may break compatibility and a patch bump may not. The
release version promises nothing about the wire: SPEC §13.3 holds six further
version numbers, four of which are compared at runtime, and none of the four
moves for a reason outside its own row. `.github/workflows/version-bump.yml`
touches the release version and fails if a bump reaches any of the four.

## Licence

MIT. See `LICENSE`.

## Documents

`docs/specs/` holds three frozen specifications: `SPEC` is the bridge, `SIM` is
the sim driver's built-in record and command set, and `HOOK` is the hook
driver's. `docs/plan/plan.md` states build order and is not frozen.

Never read a specification whole. The ledger beside each one holds a row per
claim with an anchor that locates the prose:

```sh
tools/ledger.sh codes                     document codes and paths
tools/ledger.sh subjects SPEC             every subject in a ledger
tools/ledger.sh find SPEC <text>          rows matching subject, claim or section
tools/ledger.sh show SPEC "<anchor>"      the prose around one anchor
tools/ledger.sh lint                      every stamp, anchor and glossary join
```

Where the build needs to go somewhere the specifications did not anticipate,
copy `docs/decisions/TEMPLATE.md` and number it next. `docs/audit.md` records
what the documents disagree about. `STATE.md` is the handoff between sessions.

## Two portability limits

**`sh` on Windows.** The tools and the read guard are POSIX `sh`, resolved
through Git for Windows. Without it, none of them run.

**The read guard has one hole.** `PreToolUse` fires on tool calls only, so a
specification pulled in with an `@` reference loads whole. Closing it needs a
permission rule, which also blocks the bounded reads the hook permits:

```json
"permissions": { "deny": ["Read(./docs/specs/*/spec.md)", "Read(./docs/plan/plan.md)"] }
```

## Add these when the problem shows up

**Pin the actions to commit SHAs.** A major-version tag is mutable, so `@v6`
today is not `@v6` next month.

**A `.def` regeneration check.** `vendor/lua/lua.def` omits sixteen of the 130
symbols `lua.dll` exports: the nine `luaopen_*` openers SPEC §4 forbids calling,
and seven SPEC §5.1.1 calls an artefact rather than an interface. A regeneration
that dumps the whole export table would make all sixteen linkable. Add a check
the first time a DCS update forces a re-measure.

**A cross-reference checker.** Every reference resolves today, and the frozen
specifications cannot break one. The plan can, by renumbering.
