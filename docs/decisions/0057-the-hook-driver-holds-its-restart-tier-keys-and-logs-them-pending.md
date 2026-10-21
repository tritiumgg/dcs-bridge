# ADR 0057: The hook driver holds its restart-tier keys and logs them pending

## Status

Accepted

## Context

SPEC 13.2 puts two of the hook driver's own keys in the DCS-restart tier:

> `route`, because the hook driver decides it at load and cannot reload itself
> (Section 6.9); `hook_driver_disabled_files`, because the hook-driver-side
> extension chain is read once at DCS start (Section 6.10)

It gives the pending report to the broker alone:

> **Report what has not taken effect.** A `configure` carrying a restart-tier
> key with a changed value resizes nothing. It logs the key, the old value and
> the new one, and counts it in `config_keys_pending_restart`.

The broker never sees either key: `shim.configure` carries the broker's 30
and nothing else (ADR 0050). A `ReloadConfig` that re-reads a changed `route`
would otherwise either apply it to a hook driver already running the other
route, or drop it with no line saying so.

SPEC 13.2 also places `sim_driver_path`, a hook-owned key (ADR 0050), in the
mission-reload tier.

## Decision

On a reload, the hook driver holds `route` and `hook_driver_disabled_files`
at the values in force and reports each one whose file value differs, in the
broker's shape: `{ key, effective, file }`, logged as one WARN line,
`<key>: effective <X>, file <Y>, waits for a DCS restart`.

- `config.pending(running, file)` returns that list and a copy of the file's
  hook table with those keys held. A file list compares by its names in
  order, so the same list read again is no change.
- The broker's own pending keys go through the same line, so an operator
  reads one shape for both.
- `sim_driver_path` is stored on reload and takes effect at the next
  injection, which reads it. It is not compared, and neither are the sim
  driver's keys.
- Every other hook-owned key is live: the reload stores it and the code that
  reads it sees the new value.
- The hook driver's pending keys are not counted in
  `config_keys_pending_restart`, which is the broker's counter.

Alternatives:

- **Apply the new route on reload.** The injection chain already chosen keeps
  running, so the stored value would describe a route that is not in force.
- **Drop a changed restart-tier key silently.** The operator who edited it sees
  nothing move and nothing say why.
- **Hand the two keys to the broker so it counts them.** It counts them as
  unknown, and ADR 0050 keeps its table to its own keys.

## Consequences

`doctor` and `stats` see only the broker's pending count. The hook driver's
pending keys are in `DCSBridge.log` alone until something reports them.

A key moved to the restart tier later needs a row in the hook driver's
`RESTART` table as well.
