# ADR 0048: The hook driver returns its logger and a start function

## Status

Accepted

## Context

DCS loads `Scripts\Hooks\DCSBridge.lua` once at start. SPEC 13 asks for a
thin loader plus a payload, "so the file that needs a restart is the file
you rarely edit". The payload is `Mods\services\DCSBridge\lua\HookDriver.lua`.
The specification does not say how the loader hands control to it.

The three log keys are live: a `ReloadConfig` changes them with no restart.
Nothing reads `Config\DCSBridge.lua` yet, so the keys take their defaults
in code for now, and the config reader that follows has to reach the logger.

The hook state already has a global named `log`, DCS's own, which writes
to `dcs.log`.

## Decision

`HookDriver.lua` returns one table and sets no global.

- `log` is the bridge's logger, writing `DCSBridge.log`. It has `error`,
  `warn`, `info` and `debug`, each taking one string, plus `open(dir)` and
  `configure(opts)`. `configure` takes `log_max_bytes`,
  `log_retention_days` and `log_level`, and a key it is not given keeps its
  value.
- `start(ctx)` opens the logger under `Logs\DCSBridge\` with the defaults
  and writes `ctx.banner` to it. The loader calls it once, inside `pcall`.
- The file keeps DCS's `log` global in a local and reaches it under that
  name only.

Alternatives:

- **A global table.** Two loaders, or a reload, would share it. A returned
  table belongs to whoever ran the file, as a generated file's tables do
  (ADR 0038).
- **The loader opens the log itself.** The loader is the file that needs a
  restart, so it holds as little as it can.

## Consequences

The config reader calls `log.configure` with what it reads, and a
`ReloadConfig` calls it again.

A test runs `HookDriver.lua` with `dofile` and a stubbed `lfs` and `log`,
with no DCS present.
