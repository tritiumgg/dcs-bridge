# Developing DCS-Bridge

How the source tree is laid out and how it is built, tested and released.
`README.md` is for users of the bridge. `CLAUDE.md` holds the rules an agent
follows. This file holds what is left.

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

The workspace holds four Rust crates:

| Crate | Package | Artifact |
|---|---|---|
| `crates/broker` | `dcsbridge-broker` | rlib, linked into the module |
| `crates/lua-module` | `lua-dcsbridge` | `lua_dcsbridge.dll`, renamed to `lua-dcsbridge.dll` |
| `crates/cli` | `dcsb` | `dcsb.exe` |
| `crates/generator` | `protoc-gen-dcsbridge-lua` | `protoc-gen-dcsbridge-lua.exe` |

The broker is split in two. `crates/broker` holds the rings, threads, framing
and drop policy and names no Lua symbol, so it links into a test binary on any
host. `crates/lua-module` holds `luaopen_dcsbridge` and every declaration that
does name one, and it is a `cdylib` and nothing else, because a module leaves
those symbols undefined and only a shared object may. ADR 0005.

Cargo rejects a hyphen in a library target name, so the module's library is
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
`cargo clippy -D warnings`, `cargo build`, `cargo test` and the Lua load
below, against the host target. `mise tasks` lists the rest.

`cargo run -p dcsb -- tail` connects to a running bridge on the module's
default address, `127.0.0.1:7742`, and prints one line per frame and one line
per gap in its numbering. `--addr` names another.

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

### The module under a stock Lua

SPEC §17 marks about half its test rows *Any (native module)*: runnable against
a host-native build of the broker opened by a stock Lua 5.1, with no DCS
present. That is what makes the broker developable on any of the three build
hosts.

```sh
mise run lua
```

It builds the module with `--no-default-features` and opens it with
`package.loadlib`, the way SPEC §5.1.1 does inside DCS. The host-native build
takes its Lua symbols from the interpreter that opens it rather than from a
library, so nothing here needs a DCS install. ADR 0006 says how that resolves
per host, and why the step runs on Linux and macOS but not on Windows, where a
module resolves no symbol at load.

### Windows

The product target is `x86_64` Windows for as long as DCS runs nowhere else,
and it is cross-compiled, so no contributor needs a Windows machine to produce
a release artifact. `x86_64-pc-windows-msvc` is the only Windows target, and
`crates/lua-module/build.rs` refuses any other; ADR 0003 says why there is no
second one.

```sh
cargo install --locked cargo-xwin      # once
sh tools/mkimplib.sh                   # check this machine can build the import library
mise run windows
```

On a Windows host the same task runs plain `cargo build` for the target, and
the MSVC Build Tools stand in for both `cargo-xwin` and LLVM: `build.rs` finds
`lib.exe` and builds the import library with it.

`vendor/lua/lua.def` pins the 114 Lua symbols the broker may link against.
`crates/lua-module/build.rs` turns it into the import library the DLL links, and
`tools/mkimplib.sh` does the same from a shell to report whether the machine you
are on can do it at all. A full LLVM install is the one prerequisite; a rustup
`llvm-tools` component ships neither `llvm-dlltool` nor `llvm-lib`.

The module builds twice from one source, and the `dcs-lua` feature is which one
you get. On, the `cdylib` binds DCS's Lua through the `.def` — that is the
default, and it is what the cross-build and the release workflow take. Off, the
host-native build the tests run against never touches the `.def`. `mise run
check` and CI's three-host matrix pass `--no-default-features` for it, which is
what a plain `cargo test` on a Windows host needs too. ADR 0002 says why the
default points that way.

### Schema

`proto/` holds the record schema, and `mise run schema` lints it and compiles
`target/schema.pb`:

```sh
mise run schema
```

The output is a `FileDescriptorSet`. It ships inside the write-directory zip at
`Mods\services\DCSBridge\schema.pb`, where the hook driver reads it at DCS start
and hands the bytes to the broker; the broker hashes them and serves them back,
and a consumer compares that hash against the one its handshake carries.

So the bytes are part of the wire contract, and two builds of the same tree have
to produce the same ones. `mise.toml` pins buf for that reason and CI reads the
version from there rather than naming its own. buf vendors its own
`google/protobuf/descriptor.proto`, which is in the set, so a buf bump can move
the hash on its own; `tools/mkschema.sh` says what else it holds fixed.

`buf lint` runs in CI with one standard rule excepted, because a topic id is its
payload's fully-qualified type name and the `dcs.bridge` package cannot take a
version suffix without renaming every topic. ADR 0004 has the argument.

## Releases

A tag carries four assets: `lua-dcsbridge.dll`, `dcsb.exe`,
`dcs-bridge-<version>.zip` and `SHA256SUMS` over the three. The zip
mirrors the write directory described in SPEC §13, so installing it is one
extraction over `Saved Games\<write dir>\`, and it carries the broker and
`schema.pb`. `dcsb` runs outside DCS and has no home in that tree, so it ships
beside the zip rather than inside it.

The tag names the version, and the part before any `-rc` suffix must match
`[workspace.package]` in `Cargo.toml`. Staging fails on a mismatch, so bump the
version and merge that first: `.github/workflows/version-bump.yml` opens the
pull request.

`tools/stage-release.sh` builds all four from a cross-build directory. CI runs
it on every pull request and `.github/workflows/release.yml` runs it on a tag,
so publishing is the only step a tag reaches first:

```sh
mise run schema
cargo xwin build --release --workspace --target x86_64-pc-windows-msvc
sh tools/stage-release.sh target/x86_64-pc-windows-msvc/release
```

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

## Hooks

`.claude/hooks/` holds the checks the harness runs around an agent's tool
calls, wired in `.claude/settings.json`. Each is POSIX `sh` with no `jq`;
`payload.sh` parses the JSON payload with `awk` and every hook sources it.

| Hook | Event | Does |
|---|---|---|
| `guard-spec-reads.sh` | before `Read` | refuses an unbounded read of a specification |
| `guard-frozen-writes.sh` | before `Edit`, `Write` | refuses a write to a frozen document or `.gitattributes` |
| `guard-bash.sh` | before `Bash` | refuses `sed -i`, `grep -P`, a bare toolchain command, `rustup`, a merge without `--ff-only`, a force push without a lease, a shell write to a frozen document, a pull request without the template's headings; asks before a push to `main`, a tag, a release, a merge |
| `precommit.sh` | before `Bash` | before `git commit`, runs `nospecrefs.sh`, `statecheck.sh` and a portability scan of changed `.sh` files |
| `postcommit.sh` | after `Bash` | after `git commit`, checks the message at `HEAD` against Conventional Commits |
| `session-start.sh` | session start | prints `STATE.md`, the README's open claims and the working tree into context |
| `check-state-stamp.sh` | stop | refuses to end a turn that changed the tree without stamping `STATE.md` today, once |

A refusal exits 2 with the reason on stderr, which the agent reads. An ask
prints a permission decision, which prompts the person at the keyboard.
`tools/hooktest.sh` feeds each hook the payloads it must refuse and the ones
it must pass, and `mise run docs` runs it.

## Two portability limits

**`sh` on Windows.** The tools and the hooks are POSIX `sh`, resolved
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
