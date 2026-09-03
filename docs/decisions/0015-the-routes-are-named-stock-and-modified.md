# ADR 0015: The injection routes are named Stock and Modified

## Status

Accepted

## Context

SPEC 5.4.1 defines two ways to load the sim driver and names them by letter:

> **Route A — bootstrap injection.** The hook driver resolves the sim driver's
> absolute path with `lfs.writedir`, then injects a fixed-size chunk that loads
> it:

> **Route B — bootstrap.** The installer adds a `dofile` of the sim driver to
> `Scripts\MissionScripting.lua`, positioned **before** the sanitisation block.

SPEC 13.1 carries the choice into configuration as `route`, with values `A`
and `B`, and SPEC 5.4.1 has the sim driver report which one loaded it in
`SimDriverLoaded`.

The letters order the routes and nothing else. A user choosing one has to read
the definition of each to learn the one fact that decides the choice: whether
the route edits a file under the DCS install directory. The README is written
for that user, and a name that carries the fact saves the reading.

## Decision

The routes are named **Stock** and **Modified**, after the state each leaves
the DCS install in. Route A is the Stock route. Route B is the Modified route.

The README, the `route` configuration value, `SimDriverLoaded`, `doctor`'s
output and every log line use these names. The specification and the plan
keep the letters, because they are frozen or cite the frozen text, and a
reader moving between the two uses this record as the map.

The configuration value is lower case, `stock` or `modified`, matching the
other string-valued keys in `Config\DCSBridge.lua`.

Alternatives:

- **Keep A and B.** They say nothing a user needs and force the reading above.
- **Injected and Bootstrap.** The specification's own words. They name the
  mechanism, which a user does not see.
- **Hook-only and In-mission.** Name where the sim driver runs. Accurate, but
  the deciding fact for a user is the edit to the install, which neither says.
- **Clean and Modified.** Same idea. "Clean" reads as a judgment of the other
  route. "Stock" is the word DCS users use for an unmodified install and
  carries no judgment.

## Consequences

Task 4.2 builds the config reader and takes the values `stock` and `modified`.
The sim driver's `SimDriverLoaded` and `doctor`'s second increment, task 5.C2,
report the same names. Nothing built before this record names a route, so
nothing is renamed.

A reader of the specification meets `Route A` and `Route B` and has to know
the mapping. This record is the only place it is written down, and the README
does not mention the letters.
