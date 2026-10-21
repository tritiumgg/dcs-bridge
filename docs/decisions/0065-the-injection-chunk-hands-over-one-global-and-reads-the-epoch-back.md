# ADR 0065: The injection chunk hands over one global and reads the epoch back

## Status

Accepted

## Context

SPEC 5.4.1 gives Route A's chunk as a fixed text that loads the sim driver
from disk:

> ```lua
> -- built by the hook driver; <path> escaped with string.format('%q', path)
> local ok, err = pcall(dofile, <path>)
> return ok and 'OK|loaded' or ('OK|failed: ' .. tostring(err))
> ```

and then says the chunk carries more than the path:

> Beyond the path it carries every value the loader passes in: the epoch id
> (Section 6.3), the mission name (Section 6.10), the sim-driver-tier settings
> and the `options` table (Section 13.2), and the enabled extension file list

`net.dostring_in` passes no arguments, so every value crosses as source text
inside the chunk, and the loader refuses a chunk above
`bridge_return_max_bytes`. The spec does not say how the values are named in
the target state, how a table becomes source, or how the hook driver learns
that the sim driver ran rather than only that the chunk did: a fixed
`'OK|loaded'` is the same answer whether the file set anything or not.

SPEC 5.3 gives the Interface C grammar its own `GRAMMAR_VERSION`, compared
before injection. The sim driver's source never crosses into the hook state,
so the hook driver has no value to compare with unless it reads one.

Three neighbors bear on the shape. The epoch id belongs to plan row 6.1,
which allocates it at `onMissionLoadEnd`, emits `EpochOpened` and publishes it
with `shim.epoch`; nothing allocates one yet. Plan row 5.1 makes `DCSBridge`
the sim driver's one global, stamped with `STATE_VERSION`. Plan row 4.10's
Modified route loads the same sim driver from `MissionScripting.lua`, where
it reads its settings itself.

## Decision

The chunk assigns every parameter to one global, `DCSBridge_load`, runs the
sim driver with `pcall(dofile, <path>)`, and returns the epoch read from the
global the sim driver set: `OK|loaded <epoch>`.

- **The handoff.** `DCSBridge_load` is a table with four fields: `epoch`,
  `mission`, `settings` (every key the hook driver's table gives the `sim`
  owner, `options` among them) and `files`. The sim driver's first
  statements read it and set it to `nil`, and the chunk sets it to `nil`
  again after the `dofile`, so it is gone whether the sim driver loaded or
  raised. That is how it coexists with plan row 5.1's one global: it lives
  only for the length of one chunk. When 5.1 rebuilds `DCSBridge` with its
  four slots and its stamp, it keeps the parameters somewhere the chunk's
  last line can read the epoch from, and changes that line in the same
  commit.
- **The literal.** A value crosses as Lua source the chunk compiles back to
  the same value. Strings are written with `%q`, with a carriage return
  escaped after it, since Lua 5.1.0's `%q` leaves one bare and the lexer
  refuses a bare one inside a string. Numbers are written with `%.17g`, and
  table keys in sorted order, numbers before strings, so one table always
  gives one text. A function, userdata, thread, a key that is neither a
  string nor a whole number, an infinity, a NaN or a cycle is refused, and
  the refusal names the path to the value, such as `options.builtin.resync`.
  `config.read` checks only the top level of `options`, so this is the first
  place a nested function is caught.
- **The read-back.** The chunk's last line returns `'OK|loaded '` and the
  `epoch` field of `DCSBridge.load`, or `nil` where the sim driver set no
  such table. The hook driver compares it with the epoch it sent. A match is
  a load; `OK|loaded` with any other value is an ERROR, and the load is not
  counted. Every other answer is logged verbatim, for the later rows that
  sort the answers into their cases.
- **The grammar.** `SimDriver.lua` declares `local GRAMMAR_VERSION = '...'`
  on one line, the shape ADR 0054 gives `INTERFACE`. The hook driver reads
  the file from disk before it builds the chunk and refuses with one ERROR
  when the file does not open, declares no grammar line, or declares a value
  that is not its own. A test compares the two files' lines.
- **The bound.** The assembled chunk is measured against
  `bridge_return_max_bytes` from the table in force. Above it, one ERROR
  names the size and the bound, and nothing crosses. Nothing is shortened.
- **The extension list.** `files` lists `<write dir>DCSBridge/simdriver.d/`,
  the only enumerated source in SPEC 6.10.1's load chain. It holds each
  `*.lua` the directory has, as a full path, `*.gen.lua` first and each part
  in ascending name order, less every name `sim_driver_disabled_files`
  holds. `SimDriver.builtin.lua` is a shipped file, loaded by name from the
  fixed list before the directory, so it is not in `files`; the sim driver
  reads `sim_driver_disabled_files` from `settings` to skip it. An absent or
  unreadable directory is an empty list and no log line.
  `mission_sim_driver_dirs` crosses in `settings` and nothing reads it here;
  loading a mission's own files is the sim driver's and is out of scope.
- **The mission name.** It is read with `DCS.getMissionName` through the
  accessor. A read that fails refuses the injection with one ERROR.
- **The target state.** `"server"`, the one SPEC 5.4.1 measured `dofile`
  and `pcall` present in.
- **The epoch.** Until plan row 6.1 lands, `onMissionLoadEnd` allocates the
  epoch from a counter that starts at 1 for each DCS process and moves on
  every mission load. 6.1 replaces the counter with the epoch it owns and
  publishes.
- **The gate.** `onMissionLoadEnd` reads the clock and calls `shim.tick`
  before anything else, and takes the switch from its answer, since no frame
  runs during a load and a `SetEnabled` sent then would otherwise be seen a
  frame late. A clock that does not read is one ERROR, and the switch keeps
  the value it held. Injection runs only while `driver.enabled()` is true;
  a disabled bridge logs one INFO line and injects nothing.
- **The route.** Injection runs only when `route` is `stock`. Under
  `modified` the sim driver bootstraps itself from `MissionScripting.lua`,
  so an injection there would load it twice. That load end logs one INFO
  line and injects nothing.

Alternatives:

- **Keep the spec's fixed `'OK|loaded'` and read the global with a second
  `net.dostring_in`.** One more crossing per load, and a second answer to
  classify, for a value the first chunk can return.
- **Pass each parameter as its own global.** Five names in the target
  state, each one another collision with plan row 5.1's one global.
- **Compare the grammar inside the chunk.** The chunk would carry the
  hook driver's value and the sim driver would compare, so a mismatch would
  be found after the source had already run.
- **List `SimDriver.builtin.lua` in `files`.** It would load twice, once
  from the fixed list and once from `files`, or the fixed list would stop
  being fixed.

## Consequences

The answer `OK|loaded <epoch>` is not the spec's `OK|loaded`. The row that
reads the `OK|` prefix has to treat `loaded` as the start of the body, not
the whole of it.

The hook driver opens `SimDriver.lua` on every mission load to read its
grammar. A file another program holds open with an exclusive lock refuses
the injection, which is correct and can surprise a developer editing it.

The counter's epoch is per process and starts again at 1 after a restart.
A consumer that compares epochs across restarts has nothing to compare
until 6.1.

The read-back proves the sim driver set its global in the state the chunk
ran in. It does not prove the sim driver is running a loop; nothing does
until Phase 5.
