# DCS-Bridge

DCS-Bridge connects DCS World to programs that run outside it. A program
connects to the bridge over TCP. It receives typed records as the mission runs,
and it sends commands back. The bridge runs inside the DCS process as a small
message broker. Two Lua scripts feed it: one in the DCS hook environment, one
in the mission environment.

DCS has no supported way for an external program to watch a mission and act on
it. Existing tools each solve a part of this for themselves. DCS-Bridge is one
transport that any program can use, with a published schema.

## Status

The bridge is in early development and does not yet run inside DCS.

## Get the latest release

Download it from the releases page:
<https://github.com/tritiumgg/dcs-bridge/releases/latest>

A release carries four files.

| File | What it is |
|---|---|
| `write-directory-<version>.zip` | The files that go into your DCS Saved Games folder. |
| `dcsb.exe` | The command-line tool. |
| `lua-dcsbridge.dll` | The broker, on its own. The zip already contains it. |
| `SHA256SUMS` | Checksums of the three files above. |

A release marked "pre-release" is a development build.

### Versions

The release version is 0.1.0. A tag `v<version>` publishes a release. A tag
with a suffix, such as `v0.2.0-rc1`, publishes a pre-release.

Below 1.0, a minor version bump may break compatibility. A patch bump may
not. The release version says nothing about the wire protocol. The bridge
compares its protocol, interface, grammar and state versions at runtime, and
each of those moves only for its own reason.

## Install

Installation is not final. The current releases contain placeholder files.
The steps below describe the intended procedure.

1. Extract `write-directory-<version>.zip` over your DCS Saved Games folder.
   This is normally `%USERPROFILE%\Saved Games\DCS\` or
   `%USERPROFILE%\Saved Games\DCS.openbeta\`.
2. Put `dcsb.exe` anywhere on your `PATH`. It does not belong in the Saved
   Games folder.
3. Configure the injection route. See the next section.
4. Restart DCS.
5. Run `dcsb doctor` to check the installation. This command is planned and
   not yet built.

The zip places these files.

```
Saved Games\DCS\
  Scripts\Hooks\DCSBridge.lua      The loader. DCS loads it at start.
  Mods\services\DCSBridge\         Everything the bridge ships.
```

A release overwrites every file under `Mods\services\DCSBridge\`. Do not put
your own files there. Your own extension files go under `DCSBridge\` in the
Saved Games folder. A release never touches that directory.

To uninstall, delete `Scripts\Hooks\DCSBridge.lua` and
`Mods\services\DCSBridge\`. Delete `DCSBridge\`, `Config\DCSBridge.lua` and
`Logs\DCSBridge\` if you do not want to keep them.

### Configure

Settings live in `Config\DCSBridge.lua` in the Saved Games folder. The bridge
runs with the file absent and uses its defaults. The defaults bind the broker
to `127.0.0.1:7742`. The full set of keys is not final.

The loader has an `ENABLED` flag. Set it to `false` to keep the bridge
installed but inactive.

## Route A and Route B

The bridge loads its mission-side script, the sim driver, in one of two ways.
Set the `route` key in `Config\DCSBridge.lua` to `A` or `B`. Both routes
install the same files. Both load the sim driver on every mission load.

**Route A is the default.** The hook script injects the sim driver into the
mission environment through the DCS API `net.dostring_in`. Route A edits no
file in the DCS install directory. It survives DCS updates.

Route A depends on a DCS policy setting. Add these two keys to
`Config\autoexec.cfg` in the Saved Games folder:

```lua
net.allow_unsafe_api = {"userhooks"}
net.allow_dostring_in = {"server", "mission", "gui"}
```

Other tools, such as DCS-SRS and DCS Olympus, use the same two keys. If the
file already has them, add the values above to the existing lists. Do not
replace the lists. A removed value breaks the tool that needed it.

**Route B edits a file in the DCS install directory.** Add one `dofile` line
to `Scripts\MissionScripting.lua`, before the block that removes `os`, `io`
and `lfs`. The sim driver then loads as part of the mission scripting
environment itself. Route B does not use `net.dostring_in` and needs no
`autoexec.cfg` change. The exact line to add is not final.

Every DCS update overwrites `MissionScripting.lua` and removes the line. The
bridge then stops loading with no error. Reapply the edit after every update.

Use Route A when you can enable the API. Use Route B in these two cases:

- You will not or cannot enable `net.dostring_in`.
- Your mission-side code must share the environment with a mission framework
  such as MOOSE or MIST. Route A runs the sim driver in a separate
  environment, and it cannot reach those globals.

Route B does without some features. Reloading the sim driver without a mission
reload, mission-adjacent files, server-side eval files, the mission name and
filename, and two fields of the coordinate calibration record are all Route A
only.

## Use `dcsb`

`dcsb` observes a running bridge and diagnoses a broken one. Run it on the
machine that runs DCS, or on any machine that can reach the bridge's address.

```
dcsb tail                          Print each record the bridge sends.
dcsb tail --addr 192.0.2.10:7742   Connect to a bridge on another address.
dcsb --help                        List the available commands.
```

`tail` connects to the bridge, prints one line per record, and prints a line
wherever the sequence numbers show that records were dropped. It runs until
the bridge closes the connection.

`tail` is the only command built so far. These commands are planned:
`ping`, `schema`, `send`, `doctor`, `stats`, `record`, `replay` and `mock`.

## Build from source

The product target is 64-bit Windows, because DCS runs nowhere else. You can
build that target on Windows, Linux or macOS. A Linux or macOS host
cross-compiles it and does not need a Windows machine.

Tool versions come from `mise.toml` and `rust-toolchain.toml`. Install
[mise](https://mise.jdx.dev) first. Then, in a fresh checkout:

```sh
mise install
mise run check
```

`mise run check` builds and tests the host-native build. It is the same set of
checks CI runs on a pull request.

### The Windows artifacts on Linux or macOS

You need a full LLVM install and `cargo-xwin`. A rustup `llvm-tools` component
is not enough.

```sh
cargo install --locked cargo-xwin
sh tools/mkimplib.sh
mise run windows
```

`tools/mkimplib.sh` reports whether this machine can build the Lua import
library. The build output is under `target/x86_64-pc-windows-msvc/release/`.

### The Windows artifacts on Windows

You need the Visual Studio Build Tools with the C++ workload. Then:

```sh
mise run windows
```

The build output is under `target\x86_64-pc-windows-msvc\release\`.

### Assemble a release

`tools/stage-release.sh` produces the same four files a release carries.

```sh
mise run schema
mise run windows
sh tools/stage-release.sh
```

The output is under `dist/`.

`docs/developing.md` covers the layout of the source tree and the rules for
changing it.

## License

MIT. See [LICENSE](LICENSE).
