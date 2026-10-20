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
    lua\HookDriver.lua             The hook driver, which the loader runs.
    bin\lua-dcsbridge.dll          The broker, which the hook driver loads.
```

A working install writes a banner line to `Logs\dcs.log` at DCS start, under
`DCSBridge`. It names the loader and the hook driver with each file's size
and modification time, which is how a build is identified. The same line
opens `Logs\DCSBridge\DCSBridge.log`. No banner means DCS did not find the
loader or could not parse it.

A release overwrites every file under `Mods\services\DCSBridge\`. Do not put
your own files there. Your own extension files go under `DCSBridge\` in the
Saved Games directory. A release never touches that directory.

To uninstall, delete `Scripts\Hooks\DCSBridge.lua` and
`Mods\services\DCSBridge\`. Delete `DCSBridge\`, `Config\DCSBridge.lua` and
`Logs\DCSBridge\` if you do not want to keep them.

### Configure

Settings live in `Config\DCSBridge.lua` in the Saved Games directory. The file
is Lua that returns one table, and each key you name replaces its default:

```lua
return {
  port = 7743,
  log_level = 'debug',
}
```

The bridge runs with the file absent and uses its defaults. The defaults bind
the broker to `127.0.0.1:7742`. The full set of keys is not final.

The file reaches no global, so it holds values and nothing else. A key the
bridge does not know is logged as a warning in `DCSBridge.log` and ignored. A
file that cannot be opened, does not compile, raises, returns something other
than a table, or holds a value the bridge refuses stops the bridge for that
DCS run. The reason is in `Logs\DCSBridge\DCSBridge.log` and in
`Logs\dcs.log`. Fix the file and restart DCS.

The hook driver reads the file at DCS start, loads the broker, and hands it
its settings with `shim.configure`. Nothing listens until that first call,
which binds the address and sizes the queues. A later call, on
`ReloadConfig`, changes the keys marked live, such as the timeouts and the tokens; a change to the
address, the port, the connection cap or a queue size waits for a DCS
restart, and the call reports it as pending. Each connection has a queue per
record class, so a flood of records a consumer can afford to lose pushes out
none of the others. Their sizes are `ring_out_lossy_records`,
`ring_out_durable_records` and `ring_out_lifecycle_records`.

After that first call the hook script hands the bridge the bytes of
`Mods\services\DCSBridge\schema.pb` once, with `shim.schema`. From then on
every connection's handshake carries the schema's SHA-256, which `dcsb tail`
prints on its first line; the bytes are served back to an authenticated
consumer that asks, and `dcsb schema` is one.

A consumer authenticates with a token: an id, a secret, and the capabilities
the token grants, from `read`, `command` and `reload`. Every topic names the
capability it requires. A consumer receives only the records its token
covers, with no gap in `seq` where the others were, and a record it sends on
a topic its token does not cover is refused. The `tokens` key holds one entry
per consumer, and the hook driver hands the list to the broker in the first
call:

```lua
return {
  tokens = {
    { id = 'map', secret = 'change-me', caps = { 'read' } },
  },
}
```

A consumer that wants less than its token covers sends `SetTopicFilter` with
the whole set of topics it wants; each message replaces the last, and
`LIFECYCLE` topics arrive whatever it names. `GetTopics` answers which topics
this install registered that the token can see. `topic_filter_max_topics`,
256 by default, bounds one list and is a live key.

The loader has an `ENABLED` flag on its first lines, in
`Scripts\Hooks\DCSBridge.lua`. Set it to `false` to keep the bridge installed
but inactive. A change takes a DCS restart. The loader then writes one line
to `dcs.log` saying it is disabled, and nothing else. A release overwrites
the loader, so set the flag again after an upgrade.

The bridge writes its own log to `Logs\DCSBridge\DCSBridge.log`. Three keys
govern it, and all three are live.

| Key | Default | What it does |
|---|---|---|
| `log_max_bytes` | 33554432 (32 MiB) | The most `DCSBridge.log` holds. At the cap it is renamed to `DCSBridge.<UTC>.log` and a new one starts. The renamed files together stay under the same cap, oldest deleted first, so the directory holds at most twice the cap. |
| `log_retention_days` | 14 | A renamed file older than this is deleted. |
| `log_level` | `info` | `error`, `warn`, `info` or `debug`. |

The bridge reads them from `Config\DCSBridge.lua` after the banner, so the
banner is written whatever `log_level` says.

## The Stock route and the Modified route

The bridge loads its mission-side script, the sim driver, in one of two ways.
The `route` key in `Config\DCSBridge.lua` selects one: `stock`, the default, or
`modified`, in lower case.
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
names. Set these names in each list:

| Key | Names to set |
|---|---|
| `net.allow_unsafe_api` | `"userhooks"` |
| `net.allow_dostring_in` | `"server"`, `"mission"`, `"scripting"`, `"gui"`, `"export"`, `"config"` |

DCS-Bridge calls into `"server"`, `"mission"` and `"gui"`. It asks for the
other three because ED's two descriptions of the setting disagree, and no
current DCS build shows which one is right. The list is not final.

If the file does not exist, or has neither key, add these two lines:

```lua
net.allow_unsafe_api = {"userhooks"}
net.allow_dostring_in = {"server", "mission", "scripting", "gui", "export", "config"}
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
net.allow_dostring_in = {"server", "mission", "scripting", "gui", "export", "config"}
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
dcsb ping                          Ask whether the sim is alive.
dcsb schema fetched.pb             Write the schema the bridge serves to a file.
dcsb send dcsbridge.builtin.sim.SetFlag --hex 082a
                                   Send one record, its bytes given as hex.
dcsb send dcsbridge.builtin.sim.SetFlag --file record.bin --wait 2
                                   Send the file's bytes and print what answers.
dcsb record session.bin            Append each record the bridge sends to a file.
dcsb replay session.bin            Serve that file to one consumer, with no DCS.
dcsb tail --addr 127.0.0.1:7743    Read the replayed session.
dcsb --help                        List the available commands.
```

`tail` connects to the bridge, authenticates, prints one line per record, and
prints a line wherever the sequence numbers show that records were dropped.
It runs until the bridge closes the connection. The token's secret comes from
the `DCSB_TOKEN` environment variable, or from the first line of the file
`--token-file` names. It is never taken from the command line, where every
process on the machine can read it. A refused token prints the bridge's
answer and exits 1.

A consumer that reads too slowly loses records in a fixed order. Records it
can afford to lose, such as unit positions, are dropped first, oldest first,
and show as a gap. Events are dropped only by other events. Records that mark
a mission boundary are never dropped: a consumer that has fallen behind by
enough that one would be lost is disconnected, and reconnects into a fresh
sequence.

The bridge keeps the latest record of each mission-boundary topic and sends
that set to a consumer as soon as it authenticates, before any live record,
so a `tail` started or restarted mid-mission sees the epoch it joined. The
set holds at most `max_lifecycle_topics` topics, and a boundary record over
`max_lifecycle_record_bytes` is refused when it is committed.

`ping` needs no token. It prints one line, such as `dcs_alive=true
dcs_last_heard_ms=312 bridge_enabled=true`, and exits 1 when the sim is not
alive, so a script can ask too. The sim is alive when the hook driver's
per-frame call reached the bridge within `dcs_alive_threshold_ms`, and a dash
in place of the age means it never has.

`schema` authenticates like `tail`, fetches the schema the bridge serves, and
writes it to the file named on the command line, replacing one that exists.
It prints one line, `sha256=<hex> bytes=<n> path=<file>`, and the hash is the
one `Get-FileHash` or `sha256sum` prints for the file, so the fetched schema
can be compared to the deployed `Mods\services\DCSBridge\schema.pb`. It
exits 1, writing nothing, when the bridge holds no schema yet, refuses the
token, or serves bytes that do not hash to what its handshake said. Nothing
learned exits 2.

`send` authenticates like `tail` and sends one record on the topic named,
which is the record's fully qualified message name. The record's encoded
bytes come from the file `--file` names, or from `--hex` as two hex digits
per byte, with spaces between bytes allowed; with neither, the record is
sent with no fields. The bridge does not check the bytes, so a record that
does not decode fails in the Lua handler that reads it. `send` prints `sent
topic=<name> bytes=<n>` and exits 0 once the bridge accepts the token, or
exits 1 when it refuses it. A record the bridge delivers nowhere is answered
with a `Rejected` naming it by the `seq` `send` gave it, its topic and the
reason: an unknown topic, a capability the token lacks, a connection over
its rate, or a full queue. A record that was delivered is answered only if
a handler answers it. `--wait <seconds>` keeps reading for that long and
prints each frame that comes back as `tail` prints it, with a `Rejected`'s
inside on its line. Encoding a record from JSON through the served schema
is planned.

The bridge limits what one consumer may send in a second and what all of
them may send together, `inbound_records_per_sec` and its `_total`. Over
its own limit a consumer's record is refused and its connection kept; the
consumer that pushes the total over is disconnected. A consumer hears of at
most `rejected_max_per_sec` refusals a second, and `busy_max_per_sec` for a
full queue; the rest are counted and not answered.

`record` authenticates like `tail` and appends every frame the bridge sends
to the file named, byte for byte as the wire carried it, handshake first,
until the bridge closes the connection or the process is stopped. Each
frame is written whole before the next is read, so a stopped run leaves
whole frames. A second run onto the same file appends, and `tail` reads
the join as `seq` going back to 1. A file that ends inside a frame is
refused, naming where its last whole frame ends, because a session
appended after a partial frame reads as garbage from the join on. It prints
`recorded <n> frames, <n> bytes to <file>` when the bridge closes, exits 1
when the token is refused or the stream is cut mid-frame, and exits 2 when
there was nothing to record.

`replay` serves a capture to the first consumer that connects, frame for
frame, then exits. It listens on `127.0.0.1:7743` unless `--addr` names
another, beside the bridge's port so both can run on one machine, and
prints `listening on <address>` once it is bound. It checks no token: the
consumer's own bytes are read and discarded, and the recorded bridge's
answer is what the consumer reads. The consumer sees the recorded bridge's
handshake, epochs and mission times, not this machine's clock, and reads
frames as fast as it can take them. A capture that ends inside a frame
serves the whole frames before the cut and exits 1; a file that cannot be
opened or an address that cannot be bound exits 2.

`tail`, `ping`, `schema`, `send`, `record` and `replay` are built. The
others are planned: `doctor` and `stats`.

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
