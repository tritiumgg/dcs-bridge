# ADR 0069: The sim driver bootstraps itself on the Modified route

## Status

Accepted

## Context

SPEC 5.4.1 defines the Modified route (ADR 0015) as a line in
`Scripts\MissionScripting.lua`:

> **Route B — bootstrap.** The installer adds a `dofile` of the sim driver to
> `Scripts\MissionScripting.lua`, positioned **before** the sanitisation block.
> This is the same position and the same mechanism DCS-gRPC uses. The sim driver
> then runs at the mission-scripting environment's own bootstrap, where it
> captures `package.loadlib` while that name still exists and holds the reference
> afterwards. **Route B loads without `net.dostring_in`.**

On the Stock route the hook driver's chunk hands the sim driver its
parameters in `DCSBridge_load` (ADR 0065). On the Modified route nothing
hands it anything: the hook driver injects nothing, and the line runs the
file before any hook callback for the mission. The sim driver has to read
its own keys from `Config\DCSBridge.lua`, which ADR 0051 says is read with
one shape on both routes.

The line runs in the mission scripting state, which every mission script
shares. The sanitization block after it removes `os`, `io`, `lfs`,
`require`, `loadlib` and `package` from that state. A raise out of the line
stops `MissionScripting.lua` before the block runs, so those names would
stay in reach of every mission script. A bare `dofile` raises when the file
is absent, as after an uninstall that leaves the line, and when the file
does not compile, as after a partial update. No code inside the file can
catch either.

## Decision

The sim driver bootstraps itself when `DCSBridge_load` is nil. A nil load
is the Modified route, so one file serves both routes and the hook driver
passes no flag.

- **The line.** `MissionScripting.lua` gains
  `pcall(dofile, lfs.writedir() .. 'Mods/services/DCSBridge/lua/SimDriver.lua')`,
  after `dofile('Scripts/ScriptingSystem.lua')` and before the sanitization
  block. This departs from the specification's bare `dofile`: the `pcall`
  keeps an absent or uncompilable file from skipping the block. An absent
  file is then silent in `dcs.log`, and reporting a line left after an
  uninstall is `doctor`'s.
- **Never raise.** The bootstrap runs under the file's own `pcall`. Any
  failure logs one ERROR line,
  `the sim driver did not load on the modified route: <reason>`, sets no
  global and returns nothing. A missing `package.loadlib` is a failure.
- **The keys.** `SimDriver.lua` holds a `KEYS` list of the 21 `sim` rows
  of the hook driver's, in the same `{ 'name', default, 'sim' }` shape and
  order. The two drivers never share a Lua state, and the hook driver's file
  never crosses, so the rows are copied and a test reads both files by
  their shape and fails on any drift.
- **The read.** `lfs.writedir() .. 'Config/DCSBridge.lua'` is read as
  `config.read` reads it: opened `rb`, ENOENT alone meaning the defaults, a
  byte-order mark dropped, compiled under its path, run under `pcall` in an
  empty environment, one table returned. Each `sim` key the table names
  overlays its default, and a value whose type differs from the default's
  refuses the file. Every other key, a hook or broker key or one no row
  names, is ignored without a line, since the hook driver already logs each
  unknown key once at start.
- **The checks.** The checks the hook driver makes on `sim` keys alone, with
  its wording: `sim_driver_disabled_files` is a list of strings, a number is
  whole and from 0, and `subscription_max_evals` is at least
  `max_subscriptions`. A test runs both readers on one file and compares.
- **The load.** `DCSBridge` is `{ grammar, load }`, as on the Stock route.
  `load` holds `settings`, `files` and `route = 'modified'`; `epoch` and
  `mission` are nil, since the hook driver allocates the epoch at
  `onMissionLoadEnd`, after this file ran. `files` lists
  `DCSBridge/simdriver.d/` as ADR 0065's hook driver does. On the Stock
  route the file sets `route = 'stock'` on the load it was handed.
- **The log line.** A bootstrap that loads logs one INFO line, such as
  `the sim driver loaded on the modified route: read <path>, drain_max_records 128`.
  It names the file read, or its absence, and each scalar `sim` key whose
  value differs from its default as `name value`; a table key that holds
  anything is named alone. The hook driver injects nothing on this route, so this line is the
  one sign in `dcs.log` that the sim driver loaded. Both lines are written
  with `log.write('DCSBridge', log.INFO, ...)` or `log.ERROR`, as the loader
  writes its own. A probe of DCS 2.9.28's globals lists `log` as a table in
  the hook, `server` and `scripting` states; the state the line runs in was
  not captured, so the call is made only when `log.write` is a function.
- **`package.loadlib`.** The bootstrap holds it in a file-local upvalue and
  returns an opener, a closure that loads
  `Mods/services/DCSBridge/bin/lua-dcsbridge.dll` under the write directory
  with `luaopen_dcsbridge` and returns the module table. The opener is never
  stored under `DCSBridge` or any global: that state is every mission
  script's, and a reachable opener hands any of them the broker module the
  sanitization block withholds. Both lines that run the file, the
  `MissionScripting.lua` line and the Stock chunk's `pcall(dofile, path)`,
  drop the return value. The file's own later code reaches the opener as a
  local; the return is there so a test can call it.

Alternatives:

- **A bare `dofile` line, as the specification shows.** An absent or
  uncompilable file leaves `os`, `io`, `lfs` and `package` in every mission
  script's reach.
- **A route flag in `DCSBridge_load`.** The Modified route has no chunk to
  carry it, so the flag would be read from a value that cannot exist there.
- **Publish the opener as `DCSBridge.load.open`.** Any mission script could
  then open the broker.
- **Share the hook driver's `KEYS`.** No state holds both files.
- **Log unknown keys here as well.** The hook driver logs them once at start;
  this would log them again at every mission load.
- **`env.info` and `env.error`.** They write under the scripting subsystem,
  where the loader's and the hook driver's lines are under `DCSBridge`.

## Consequences

The two routes can disagree on one file. The hook driver refuses a file that
fails a pair across owners, such as `recent_idempotency_keys` below
`ring_in_sim_driver_records`, or that gives a hook or broker key the wrong
type, and stops its start (ADR 0051). The sim driver reads only its own
keys and loads. Nothing opens the broker from the sim driver yet, so the
effect is a sim driver with nobody to talk to. Plan row 5.9, where the sim
driver reaches the broker through `DCSBridge.shim`, is the row that opens
the module through this opener, and it has to handle a broker the hook
driver never configured.

The two drivers can hold different `sim` values. The hook driver reads the
file at start and at a reload; the sim driver reads it at every mission
load. An edit between the two is in force for the sim driver first.

A `stock` file with the line still in `MissionScripting.lua` loads the sim
driver twice, once by the line and once by injection. Nothing here detects
it; plan row 5.1's stamp on `DCSBridge` and `doctor`, plan row 5.C2, are
where it is seen.

Plan row 5.9 also names `DCSBridge.shim`. Under the Modified route a module
table published there is the broker module in every mission script's
reach, which the opener's rule above exists to prevent; that row decides
what the seam exposes.

Whether `log.write` exists at the line, and whether `lfs.writedir` and
`io.open` work there in a current build, is unverified from the checkout.
The first live run with the line in place shows the INFO line or not.
