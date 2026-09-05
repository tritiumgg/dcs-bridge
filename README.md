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
| `dcs-bridge-<version>.zip` | The files that go into your DCS Saved Games directory. |
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

1. Extract `dcs-bridge-<version>.zip` over your DCS Saved Games directory.
   This is normally `%USERPROFILE%\Saved Games\DCS\` or
   `%USERPROFILE%\Saved Games\DCS.openbeta\`.
2. Put `dcsb.exe` anywhere on your `PATH`. It does not belong in the Saved
   Games directory.
3. Configure the injection route. See the next section.
4. Restart DCS.
5. Run `dcsb doctor` to check the installation. This command is planned,
   not built.

The zip places these files.

```
Saved Games\DCS\
  Scripts\Hooks\DCSBridge.lua      The loader. DCS loads it at start.
  Mods\services\DCSBridge\         Everything the bridge ships.
```

A release overwrites every file under `Mods\services\DCSBridge\`. Do not put
your own files there. Your own extension files go under `DCSBridge\` in the
Saved Games directory. A release never touches that directory.

To uninstall, delete `Scripts\Hooks\DCSBridge.lua` and
`Mods\services\DCSBridge\`. Delete `DCSBridge\`, `Config\DCSBridge.lua` and
`Logs\DCSBridge\` if you do not want to keep them.

### Configure

Settings live in `Config\DCSBridge.lua` in the Saved Games directory. The bridge
runs with the file absent and uses its defaults. The defaults bind the broker
to `127.0.0.1:7742`. The full set of keys is not final.

A consumer authenticates with a token: an id, a secret, and the capabilities
the token grants, from `read`, `command` and `reload`. The `tokens` key holds
one entry per consumer. Reading it from the file is not yet built; until then
a hook script hands the list to the bridge with `shim.tokens`.

The loader has an `ENABLED` flag. Set it to `false` to keep the bridge
installed but inactive.

## The Stock route and the Modified route

The bridge loads its mission-side script, the sim driver, in one of two ways.
The `route` key in `Config\DCSBridge.lua` selects one. Its values are not final.
Both routes install the same files. Both load the sim driver on every
mission load.

### Which route to use

**Use the Stock route unless you have a reason not to.** It is the default.
Use the Modified route in these two cases:

- You will not, or cannot, enable `net.dostring_in` in DCS.
- Your mission-side code must share an environment with a mission framework
  such as MOOSE or MIST. The Stock route runs the sim driver in a separate
  environment, and it cannot reach those globals.

| | Stock | Modified |
|---|---|---|
| Edits a file in the DCS install directory | No | Yes |
| Survives a DCS update | Yes | No. Reapply the edit after every update. |
| Needs `net.dostring_in` enabled in `autoexec.cfg` | Yes | No |
| Sim driver shares globals with MOOSE, MIST and the mission | No | Yes |
| Reload the sim driver without a mission reload | Yes | No |
| Mission-adjacent files, server-side eval files, mission name and filename | Yes | No |
| Mission date and magnetic declination in the coordinate calibration record | Yes | No |

### The Stock route

The hook script injects the sim driver into the mission environment through
the DCS API `net.dostring_in`. The Stock route edits no file in the DCS install
directory. It survives DCS updates.

The Stock route depends on a DCS policy setting in `Config\autoexec.cfg` in
the Saved Games directory. The setting is two keys. Each key holds a list of
names. DCS-Bridge needs these names in each list:

| Key | Names DCS-Bridge needs |
|---|---|
| `net.allow_unsafe_api` | `"userhooks"` |
| `net.allow_dostring_in` | `"server"`, `"mission"`, `"gui"` |

If the file does not exist, or has neither key, add these two lines:

```lua
net.allow_unsafe_api = {"userhooks"}
net.allow_dostring_in = {"server", "mission", "gui"}
```

If the file already has a key, keep the line and add the missing names to its
list. Other tools, such as DCS-SRS and DCS Olympus, set the same keys. A name
you remove breaks the tool that needed it. For example, this line from another
tool:

```lua
net.allow_dostring_in = {"server"}
```

becomes:

```lua
net.allow_dostring_in = {"server", "mission", "gui"}
```

Set both keys. A file with `net.allow_dostring_in` and no
`net.allow_unsafe_api` does not enable the API.

### The Modified route

The Modified route edits a file in the DCS install directory. Add one `dofile`
line to `Scripts\MissionScripting.lua`, after the line that loads
`ScriptingSystem.lua` and before the block that removes `os`, `io` and `lfs`.
The sim driver then loads as part of the mission scripting environment
itself. The Modified route does not use `net.dostring_in` and needs no
`autoexec.cfg` change. The exact line to add is not final.

The edited file looks like this. The `dofile` line is the addition.

```lua
--Initialization script for the Mission lua Environment (SSE)

dofile('Scripts/ScriptingSystem.lua')

dofile(lfs.writedir() .. 'Mods/services/DCSBridge/lua/SimDriver.lua')

--Sanitize Mission Scripting environment
--This makes unavailable some unsecure functions.
--Mission downloaded from server to client may contain potentialy harmful lua code that may use these functions.
--You can remove the code below and make availble these functions at your own risk.

local function sanitizeModule(name)
	_G[name] = nil
	package.loaded[name] = nil
end

do
	sanitizeModule('os')
	sanitizeModule('io')
	sanitizeModule('lfs')
	_G['require'] = nil
	_G['loadlib'] = nil
	_G['package'] = nil
end
```

Do not remove the sanitize block. It keeps mission scripts sandboxed.

Every DCS update overwrites `MissionScripting.lua` and removes the line. The
bridge then stops loading with no error. Reapply the edit after every update.

## Use `dcsb`

`dcsb` observes a running bridge and diagnoses a broken one. Run it on the
machine that runs DCS, or on any machine that can reach the bridge's address.

```
dcsb tail                          Print each record the bridge sends.
dcsb tail --addr 192.0.2.10:7742   Connect to a bridge on another address.
dcsb tail --token-file token.txt   Read the token from a file instead.
dcsb --help                        List the available commands.
```

`tail` connects to the bridge, authenticates, prints one line per record, and
prints a line wherever the sequence numbers show that records were dropped.
It runs until the bridge closes the connection. The token's secret comes from
the `DCSB_TOKEN` environment variable, or from the first line of the file
`--token-file` names. It is never taken from the command line, where every
process on the machine can read it. A refused token prints the bridge's
answer and exits 1.

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

`mise run check` builds and tests the host-native build. `mise run ci` runs
every check a pull request is gated on.

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
