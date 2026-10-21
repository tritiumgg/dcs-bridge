# ADR 0072: The hook driver registers its generated tables after the schema hand-off

## Status

Accepted

## Context

ADR 0055 loads `HookDriver.gen.lua` once, before the first `configure`, and
keeps it as `state.gen`, so registering its tables uses that table and
loads nothing again. Until the hook package held a record routed to the hook
driver, the tables were empty and nothing registered them.

Three records bear on where the four calls go:

- ADR 0029 lets a driver register before it configures, and says the hook
  driver does not: a `LIFECYCLE` row registered first freezes
  `max_lifecycle_topics` at the default, and the file's value then stays
  pending until a restart that repeats the same order.
- ADR 0054 and ADR 0056 keep every stop of the start before the first
  `configure`, which binds the listener, except `shim.schema` raising.
- ADR 0052 has the generated hash ride `configure`. The broker compares it
  with the schema's when `shim.schema` hands the bytes over, and counts a
  mismatch in `schema_sha256_mismatch_total`. `shim.classes` takes an
  optional hash and compares it again.

## Decision

`driver.start` registers `state.gen`'s classes, routes, capabilities and
replies right after the schema line and before the extension chain, and a
refused call stops the start.

- **The order.** `shim.classes`, `shim.routes`, `shim.caps`, then
  `shim.replies`, each under `pcall`, after the first `configure` and the
  schema hand-off.
- **No hash.** `shim.classes` is called with the table alone. The same hash
  was compared at the schema hand-off, and a second comparison would count
  one mismatch twice.
- **A refusal stops the start.** The ERROR line names the call and the
  broker's reason, and the start raises through the loader as any other
  stop does. It is the second stop after the bind, beside ADR 0056's, and
  it sits above `state.started`, so no callback body runs behind it.

Alternatives:

- **Register before `configure`, with the callbacks.** The first table that
  holds a `LIFECYCLE` row would freeze `max_lifecycle_topics` at the
  default, which ADR 0029 rules out.
- **Log a refusal and run on.** Every record routed to the hook driver,
  `ReloadConfig` among them, would be refused `UNKNOWN_TOPIC` by a bridge
  that looks healthy. A stop says the install is broken.
- **Pass the hash, as the sim driver does.** The sim driver's hash rides
  no `configure`, so `shim.classes` is its one comparison. The hook
  driver's would be its second.

## Consequences

Between the bind and the registration a consumer that sends a hook-routed
record is refused `UNKNOWN_TOPIC`. The window is the length of the schema
hand-off, inside the start, before the first frame.

At DCS start nothing has registered before the hook driver, so no row it
offers can conflict with another. A refusal means a generated file the
broker cannot take, or a file whose `max_lifecycle_topics` is below the
number of `LIFECYCLE` topics the hook driver registers. A sim driver whose
tables conflict with the hook driver's is refused on its own side, later.

A stop after the bind leaves a broker listening behind a hook driver that
did not start, as ADR 0056's does. The ERROR line in `DCSBridge.log` and the
loader's line in `dcs.log` both name the cause.
