# ADR 0053: shim.classes takes the hash second and answers a warning second

## Status

Accepted

## Context

SPEC 8.3 says "`shim.classes` carries the sim driver's" schema hash, and the
broker "answers registration with a warning `doctor` prints". It names no
carrier inside the call and no shape for the warning.

`shim.classes(table)` maps a topic to a class. A key in that table is a
topic, and a row whose key is not a topic is refused (ADR 0023's calls all
read their table that way). The call answers the number of rows it added.

## Decision

The hash is the second argument, `shim.classes(table, sha256)`, as 64 hex
digits. The warning is the second return value.

- An absent or nil second argument compares nothing, so a caller with no
  generated file registers as before.
- A second argument that is not 64 hex digits raises `classes refused:`
  before the broker is asked, like any other bad argument.
- With a schema held and the hashes equal, the call answers the count
  alone.
- With a schema held and the hashes different, the rows are registered, the
  mismatch counts in `schema_sha256_mismatch_total` (ADR 0052), and the call
  answers the count and a string naming both hashes. The caller logs it.
- With no schema held, nothing is compared. The hook driver hands the schema
  over at DCS start, before any sim driver loads, so this case is a test or
  a hook driver that did not start.

Alternatives:

- **A `schema_sha256` key in the class table.** It would read as a topic, and
  every reader of the table would need to skip it.
- **Raise on a mismatch.** SPEC 13.3 makes a hash mismatch a warning, and a
  raise would stop the sim driver's registration.
- **Report only through `stats`.** `stats` does not exist yet, and the caller
  is the one with a log to write the warning to.

## Consequences

`shim.classes` is the one registration call that can answer two values, so it
has an entry function of its own rather than the shared one.

The interface version does not move: the second argument is optional, and a
caller that ignores the second value sees what it saw before.
