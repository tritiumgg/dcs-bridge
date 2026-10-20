# ADR 0052: schema_sha256 rides configure, and is no configuration key

## Status

Accepted

## Context

SPEC 8.3 says how the hook driver's generated hash reaches the broker:

> embeds the run's schema hash in `HookDriver.gen.lua` and in the sim driver's
> generated code; the hook driver passes its hash to the broker at the first
> `configure` as `schema_sha256`, `shim.classes` carries the sim driver's, and
> the broker compares both against its own hash of the `shim.schema` bytes,
> reports any mismatch in `stats`

SPEC 13.3's version table gives the outcome: compared by the broker at
registration, a "Warning in `stats`". The `stats` key list names no counter
for it.

ADR 0050 says `shim.configure` gets "the broker's table and nothing else, all
30 keys every time, so its unknown count is zero". That sentence is about the
keys of `Config\DCSBridge.lua`. `schema_sha256` is not one: no operator writes
it, and it has no default, tier or row in `KEYS`. It comes from the generated
file.

Task 4.4's `ReloadConfig` calls `configure` again with the same table.

## Decision

`schema_sha256` is a key of the `configure` table, as SPEC 8.3 words it, and
the broker takes it out of every table before the table is applied.

- The value is a SHA-256 as 64 hex digits. Anything else refuses the call,
  naming the key, before any of the table applies.
- The first call keeps the hash. A later call checks the value and keeps
  nothing, since the generated file cannot change under a running DCS.
- The key is never counted as unknown, on the first call or any later one.
- `shim.schema` hashes the bytes as before. A hash that differs from the kept
  one counts in `schema_sha256_mismatch_total`, and the set is held all the
  same. The hook driver writes a WARN line from the hash the call answers.
- `KEYS` and `Config` stay at 30 keys. ADR 0050's "nothing else" still
  holds, because it governs configuration keys, and this key is not one.
  ADR 0050 stays Accepted.

Alternatives:

- **A second argument, `shim.configure(table, hash)`.** It departs from SPEC
  8.3's wording, and the reload path would need to pass it or be told why not.
- **Refuse the hand-off on a mismatch.** SPEC 13.3 makes it a warning. The
  bytes are the deployed file, so the served set is right even when the
  generated tables are stale.
- **Make it a row in `KEYS`.** An operator could then set it in the file,
  and the hook driver would hand the file's value rather than the generated
  one.

## Consequences

The broker's table carries 31 keys at start: 30 configuration keys and this
one.

The counter has no reader until `stats` exists. Until then the hook driver's
WARN line in `DCSBridge.log` is the visible report.

A table without the key compares nothing, so a caller with no generated file,
such as a test, configures as before.
