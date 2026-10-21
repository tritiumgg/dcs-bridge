# ADR 0070: The hook chain runs after the schema hand-off

## Status

Accepted

## Context

SPEC 6.10.1 gives the hook driver a chain of its own:

> The hook driver mirrors the chain at DCS start: `HookDriver.lua` loads
> `HookDriver.gen.lua`, then `HookDriver.builtin.lua`, then `<write
> dir>\DCSBridge\hookdriver.d\` in name order. Hook-driver-side files load once
> per DCS process, and there is no hook driver reload (Sections 3.1 and 6.9).

SPEC 6.10.2 contains each file on the sim driver's side: one that "fails to
compile, raises while loading, or is refused whole is logged, counted in
`sim_driver_files_failed_total`, and skipped", and the rest load. The plan
gives the hook driver the same rule and the counter
`hook_driver_files_failed_total`. SPEC 6.10.4 says `hook_driver_disabled_files`
lists "shipped file names the loader skips".

ADR 0055 takes `HookDriver.gen.lua` out of the chain: it loads before the
first `configure`, and a failure stops the start. That leaves four things the
specification does not settle:

- Where the rest of the chain runs against `configure`, which binds the
  listener (ADR 0054).
- What a file is handed. The hook driver's registration surface,
  `on`/`command`/`off`/`replace`/`wrap` in SPEC 6.10.2, has not been built,
  so a file has nothing to register through.
- What a missing `HookDriver.builtin.lua` does. The release zip carries it,
  and the specification names no case where it is absent.
- Where the counter is held. Nothing publishes the hook driver's counters
  until `shim.stats` carries them.

## Decision

The hook driver runs its chain in `start`, after the schema is handed to the
broker and before the start is marked finished.

- **The order.** `Mods\services\DCSBridge\lua\HookDriver.builtin.lua`, unless
  `hook_driver_disabled_files` names it, then every `*.lua` in
  `<write dir>\DCSBridge\hookdriver.d\`, listed as `injection.files` lists
  `simdriver.d\` for the sim driver (ADR 0065): the `*.gen.lua` files first,
  each part in name order. An absent directory is no files and logs nothing.
- **The disabled list covers the directory too.** A name in
  `hook_driver_disabled_files` drops a file of that name from `hookdriver.d\`
  as well, with no line and no count. `sim_driver_disabled_files` already does
  this for `simdriver.d\`, through the same function, and one key that behaves
  two ways by side would surprise an operator who learned the other.
- **Each file is a bare chunk.** It is compiled with `loadfile` and run under
  `pcall` with no argument, in the hook state's global environment, as a
  `Scripts\Hooks` file is. Plan row 5.13 builds the registration surface, and
  each file then registers through it.
- **Containment.** A file that is missing, does not compile or raises logs one
  ERROR line, `extension file skipped: <path>: <reason>`, adds one to
  `hook_driver_files_failed_total`, and is skipped. The rest load, and the
  start finishes.
- **A missing built-ins file is contained.** The specification leaves this
  open. ADR 0055 stops the start for a missing generated file because a bridge
  without it serves no built-in record. A bridge without the built-ins file
  serves every record and loses only the hook built-ins' handlers, so the
  failure is the size of one extension file's and is treated as one. The ERROR
  line names the path, so the broken install is visible.
- **The counter.** `hook_driver_files_failed_total` is held in the hook
  driver's state, and `driver.counters()` returns a copy of the counters it
  holds. Publishing them is `shim.stats`'s work.
- **The shipped file.** `HookDriver.builtin.lua` ships beside `HookDriver.lua`
  and registers nothing. Plan row 10.6 fills it with the hook built-ins.

Alternatives:

- **Run the chain before `configure`.** ADR 0054 stops before the bind
  because a stop after it leaves a broker listening. A contained file never
  stops the start, so that reason does not apply, and a file running after
  the schema line can reach a configured broker.
- **Stop the start on a missing built-ins file, as ADR 0055 does for the
  generated file.** The bridge would refuse to run over handlers that, until
  plan row 10.6, do nothing.
- **Hand each file the shim, as the generated file is handed it.** That gives
  every extension file the broker module with no surface to bound what it
  calls, and plan row 5.13 would have to take the argument back.
- **Apply the disabled list to shipped names alone, as SPEC 6.10.4 words it.**
  The two sides would differ on one rule for no gain.

## Consequences

The chain writes a line only for a file it skips. A start with no
`hookdriver.d\` and an intact install ends its `DCSBridge.log` lines with the
schema line.

Until plan row 5.13 lands, an extension file can do only what a `Scripts\Hooks`
file can, and the specification's rule against calling
`DCS.setUserCallbacks` from one is not enforced.

The count is visible only to a test through `driver.counters()` until
`shim.stats` publishes it. An operator sees the ERROR line.

A file in `hookdriver.d\` that shares a name with a shipped file and is named
in `hook_driver_disabled_files` is skipped without a trace in the log.
