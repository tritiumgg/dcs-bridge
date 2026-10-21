# ADR 0062: A raising callback logs once until the next mission load

## Status

Accepted

## Context

SPEC 4's code rules:

> Put one `pcall` around the whole body of every DCS callback. **Log a raise
> once per callback per epoch and suppress the rest.** DCS never disables a
> raising hook callback: it keeps calling it. An unsuppressed error in a
> per-frame callback fills `dcs.log` at the callback rate

The plan's completion condition for the rule is that a deliberate error is
logged, the session survives, and a per-frame raise does not fill `dcs.log`.

The hook driver has no epoch id yet. Epochs arrive later, opened at
`onMissionLoadEnd` and closed at `onSimulationStop`. The rule names no log.

## Decision

A callback body that raises returns nothing to DCS, and its first raise
writes one ERROR line to `DCSBridge.log`:
`<name> raised: <error>; suppressed until the next mission load`. Later
raises of that name write nothing until the next `onMissionLoadEnd`.

- The registered function for `onMissionLoadEnd` clears the set of names
  that raised before its own body runs, so a body that raises at the load
  itself is logged in the new epoch. That is the boundary where the epoch
  opens; the menu between missions stays in the previous one.
- Each name is tracked alone, so two callbacks raising give two lines.
- The raised value is formatted with `tostring` under `pcall`, with a fixed
  text when that fails, and the log call itself runs under `pcall`. Nothing
  in the failure path can raise into DCS, which would bring back the fault
  the rule stops.
- Nothing is written to `dcs.log`. A raise line there competes with DCS's
  own per-frame output, and the bridge does not control that file's size.
  `DCSBridge.log` is the bridge's own, capped and rotated (ADR 0047), so
  even an unsuppressed line could not grow it without bound.

Alternatives:

- **Log to `dcs.log` as well.** One line per epoch is small, but it puts
  bridge faults among DCS's output, where an operator reads for DCS's own.
- **Suppress for the life of the process.** A fault that a mission's data
  causes would be logged for the first mission alone.
- **Count raises and log the count at the next load.** More useful, and it
  needs a stats channel that does not exist yet.

## Consequences

A body that raises in the main menu after a mission ends shares that
mission's suppression: when it already raised during the mission, the menu
raise writes nothing.

When epoch ids land, the clearing stays at `onMissionLoadEnd`, which is the
boundary they open at. A count of suppressed raises, if one is wanted, joins
`stats` then.
