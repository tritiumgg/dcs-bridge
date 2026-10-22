# ADR 0076: The bridge keeps no binding blacklist

## Status

Accepted

## Context

SPEC §4.2 lists DCS bindings that end the process even inside `pcall`:

> Six functions on the mission-scripting surface terminate the process with an
> access violation despite being called inside `pcall`.

Five are named and the sixth is unattributed. The same section records a
seventh, starting "**A seventh, in the hook state, observed on
2.9.29.27278** while measuring the", with two candidates and no
attribution. From it the section asks the build to "Treat the hook
driver's `terrain` table as a crasher family until probed member by
member." Its remedy is "Probe once at startup and blacklist what raises."

Plan row 5.2 built that remedy: a binding probe run at startup, a
blacklist of what raised, and an `unsafe_bindings_enabled` key gating the
call site. Plan row 6.3 calls `terrain.convertMetersToLatLon`, a member of
the table the section calls a crasher family. `docs/audit.md` flagged the
seventh crasher's attribution as unverified, with no probe covering it.

## Decision

The bridge keeps no binding blacklist, runs no binding probe, and has no
`unsafe_bindings_enabled` key. This is the maintainer's decision.

The bridge calls DCS bindings the way any mission script would. A binding
that crashes the process is the concern of the operator, or of the author
of the handler that calls it. It is not a list the bridge maintains.

Alternatives:

- **Ship the blacklist incomplete.** It names five of seven crashers and
  cannot name the two that are unattributed.
- **Probe and report only.** A bare-call probe misses a binding that dies
  only on an argument, so its report would clear what it never tested.
- **Keep only the gate.** A key with no list behind it gates nothing.

## Consequences

Plan row 5.2 is removed. The `unsafe_bindings_enabled` key leaves the key
lists of the hook driver and the sim driver, so a `Config\DCSBridge.lua`
that still names it gets the unknown-key warning.

Row 6.3 calls `terrain.convertMetersToLatLon` with no exemption to claim,
since no list exists to exempt it from.

A built-in or an extension that calls a crashing binding takes DCS down
with it. That risk is accepted.
