# ADR 0063: shim.tick answers the effective enabled value

## Status

Accepted

## Context

SPEC 11 on the kill switch:

> **Kill switch.** `enabled` in `Config\DCSBridge.lua`, applied like any live
> key: at load, by `ReloadConfig`, or by `SetEnabled`, which the broker handles
> itself so a re-enable arrives even while dispatch is down. Disabled means: the
> hook driver stops injection, drain, command ferrying and eval

and, below it, the hook driver keeps calling `shim.tick` while disabled, so
the heartbeat stays fresh and `Pong` reports a disabled bridge rather than a
dead sim.

`SetEnabled` arrives on a reader thread and swaps the broker's table between
two `configure` calls (`Bridge::set_enabled`). The hook driver hands the
broker its table at load and at reload, so it knows the file's value, but no
shim call hands it the value a `SetEnabled` set. The hook driver is the one
that stops injection, drain, ferrying and eval, so it has to learn the
effective value on the logic thread.

ADR 0053 lets a call grow an extra return value without moving the
interface version, because a caller that ignores the value sees what it saw
before. ADR 0061 gives `onSimulationFrame` one body, owned by the runtime,
which calls its consumers in a fixed order.

## Decision

`shim.tick(mission_time)` returns the effective `enabled` value as a boolean,
read from the same configuration load the heartbeat throttle reads.

- The hook driver's `onSimulationFrame` body reads the clock, calls
  `shim.tick`, and stores the answer as the switch. `driver.enabled()`
  returns it, and it is the one predicate every path that injects, drains,
  ferries commands or evaluates consults.
- The tick is the first statement of the `onSimulationFrame` body, ahead of
  every consumer ADR 0061 orders. A later consumer that raises then cannot
  stop the heartbeat, which would make `Pong` report a dead sim.
- The hook driver assigns the switch only when `shim.tick` answers a
  boolean. An answer of nothing keeps the value it held. A hook driver
  paired with a module from before this decision passes the interface
  check, since both say `'5'`, and its tick answers nothing; keeping the
  previous value is what makes that pairing harmless. That is the condition
  under which the interface version stays at `'5'`.
- At start and at reload the hook driver takes the file's value from the
  table it hands `configure`, so the switch is right before the first frame.
- A transition logs one INFO line to `DCSBridge.log`, `bridge disabled` or
  `bridge enabled`. A frame that changes nothing logs nothing.
- A `configure` swaps the broker's table whole, so a reload puts the file's
  value back over a `SetEnabled`. That is what "applied like any live key"
  means: the last writer wins, and the file is a writer.

Alternatives:

- **A `shim.enabled()` call.** One more call every frame, and a new name on
  the module, which moves the interface version for a value `tick` can
  carry.
- **The broker pushes the value into the hook ring.** It arrives behind the
  dispatch loop, which a disabled bridge stops, and a re-enable could then
  never be read.
- **Read the value only at load and reload.** `SetEnabled` would change what
  `Pong` reports and nothing the hook driver does.

## Consequences

The switch follows `SetEnabled` within one frame. Between a `SetEnabled`
and the next frame, `Pong` already reports the new value while the hook
driver still runs the previous one.

A `SetEnabled` lasts until the file is read again, at a reload or a DCS
restart. An operator who disables the bridge live and then reloads to add a
token turns it back on unless the file says `enabled = false`.

`driver.enabled()` has no caller until the gated paths land. The plan rows
that build injection, the dispatch loop, the eval poller and the epoch
teardown each name it, so the obligation is held where the work is ordered.

A raise in `DCS.getModelTime` takes no tick for that frame, and ADR 0062
logs it once per mission load. The heartbeat then goes stale and `Pong`
reports `dcs_alive` false, which is the right reading of a hook state that
cannot read its clock.
