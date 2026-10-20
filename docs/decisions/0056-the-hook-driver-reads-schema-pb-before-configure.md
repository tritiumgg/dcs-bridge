# ADR 0056: The hook driver reads schema.pb before configure

## Status

Accepted

## Context

SPEC 8.3 on the compiled schema:

> **The build also writes the compiled `FileDescriptorSet` to
> `Mods\services\DCSBridge\schema.pb`.** It deploys with the other generated
> files. The hook driver reads it at DCS start and hands the bytes to the broker
> with `shim.schema` (Section 5.1).

The broker refuses `shim.schema` before the first `configure`, and the first
`configure` binds the listener (ADR 0054). Before the hand-off, the handshake
carries no hash and `GetSchema` answers an error.

SPEC 13.3 makes a schema hash mismatch a "Warning in `stats`", not a stop.

## Decision

The hook driver reads `schema.pb` whole before the first `configure`, and
hands the bytes to `shim.schema` right after it.

- A file that does not open, or is empty, goes through ADR 0051's path
  before `configure`: an ERROR line naming the path, a raise, the loader's
  did-not-start line. Nothing binds. The zip always carries the file, so a
  missing one is a broken install.
- `shim.schema` raising is the one stop after the bind. At first start only
  a set too large for `max_frame_bytes` can cause it. The ERROR line says
  the broker listens with no schema to serve, and the start stops there.
- The hash `shim.schema` answers is logged: `INFO schema <hex>` when it
  equals the generated file's, and `WARN schema <hex> but HookDriver.gen.lua
  was generated from <hex>` when it differs. The start continues on a
  mismatch, and the broker counts it (ADR 0052).

Alternatives:

- **Read the file after `configure`, as SPEC 8.3's order reads.** A missing
  file would then stop the start with the listener already up.
- **Run on with no schema when the file is missing.** A consumer would see
  no hash and a `GetSchema` error, and nothing in the log would say the
  install is broken.
- **Check the size in Lua before `configure`.** It would copy the broker's
  answer overhead into the hook driver, and two copies drift.
- **Stop on a hash mismatch.** SPEC 13.3 makes it a warning, and the bytes
  are the deployed file either way.

## Consequences

The schema bytes stay in memory for the length of the start, about the size of
the file.

A `schema.pb` larger than `max_frame_bytes` allows leaves a listening broker
with no schema behind a hook driver that did not start. Lowering
`max_frame_bytes` that far is an operator's choice, and the ERROR line names
the cause.
