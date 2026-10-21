# ADR 0060: The hook driver reaches DCS through one call

## Status

Accepted

## Context

SPEC 4.4 records that ED is renaming the hook namespace from `DCS` to `Sim`,
with both names bound to one table on the measured build:

> Isolate every `DCS.*` call behind one accessor table in the hook driver.
> Section 4 requires that, and it turns the migration into one edit.

SPEC 4.3, after an injected chunk that called five `DCS.*` functions in
sequence crashed DCS:

> Call `DCS.*` functions one at a time, each guarded.

Neither section says what shape the accessor takes.

The first call the hook driver makes is `DCS.setUserCallbacks` at start. Later
work adds `DCS.getPause`, `DCS.getRealTime`, the mission name and its siblings.

## Decision

Every `DCS.*` call in `HookDriver.lua` goes through one function,
`dcs.call(name, ...)`, over a namespace named once in a string constant.

- `dcs.call` looks the namespace up in `_G` at each call, so a namespace DCS
  replaces after the file loads is the one reached.
- It runs the function under its own `pcall` and returns `true` and every
  result, or `false` and a reason: the namespace is missing, the function is
  missing, or what it raised. It never raises.
- The reason formats a raised value with `tostring` under `pcall`, so a value
  that does not convert still gives a reason.
- The table is `driver.dcs`, so a test reaches it.
- A test reads the driver's source and fails on any line, comments aside,
  that writes `DCS.` or `DCS[`.

Alternatives:

- **One wrapper per DCS function, listed in a table.** Each new call needs a
  new row, and a call that was not listed goes around the table.
- **Bind the namespace at load, `local DCS = DCS`.** A namespace DCS replaces
  later would be missed, and an absent one would raise at the first call.
- **Leave the guard to each caller.** Each caller would write its own `pcall`,
  and one that forgets raises into DCS.

## Consequences

The rename to `Sim` is an edit to `NAMESPACE`.

Every call pays one table lookup and one `pcall`. Per-frame calls arrive with
the dispatch loop; if one turns out costly, it can hold its function between
frames without leaving the accessor.

The source test covers the hook driver only. A check across every built-in
set is owed separately.
