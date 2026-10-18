# ADR 0039: The generated hash is of the schema.pb the parameter names

## Status

Accepted

## Context

Each generated file carries the schema hash, so that a hook driver and a sim
driver from different generator runs are caught. SPEC 8.3:

> the generator embeds the run's schema hash in `HookDriver.gen.lua` and in
> the sim driver's generated code; the hook driver passes its hash to the
> broker at the first `configure` as `schema_sha256`, `shim.classes` carries
> the sim driver's, and the broker compares both against its own hash of the
> `shim.schema` bytes

The broker's hash is the SHA-256 of the bytes `shim.schema` receives, which
are `schema.pb` as `tools/mkschema.sh` wrote it (ADR 0020). For the
comparison to pass, the generated hash has to be the SHA-256 of those same
bytes.

A plugin request carries the schema's descriptors, not that file. The
generator could encode a `FileDescriptorSet` from the request and hash that,
but protobuf encoding is not canonical. `buf build` decides which imports
the set carries, whether source info stays, and the order of fields within
each descriptor; a set re-encoded from the request matches it only by
agreement nobody checks, and one byte apart is a mismatch reported on every
load with no cause anyone can see.

## Decision

The plugin takes a parameter, `schema=<path>`, naming the compiled
`schema.pb`, and embeds the lowercase hex SHA-256 of that file's bytes in
both generated files. buf passes it from `buf.gen.yaml`'s `opt`, protoc from
`--dcsbridge-lua_opt`. A request without the parameter, or naming a file the
plugin cannot read, fails generation naming the parameter.

Alternatives:

- **Hash a set re-encoded from the request.** Equal to the broker's hash
  only while buf and `prost-reflect` happen to encode alike, and no test
  could hold them to it across a buf release.
- **Leave the hash out until a driver checks it.** The specification puts
  the hash in the file the generator writes, and a file without it is one
  the driver cannot check.

## Consequences

The embedded hash equals the broker's by construction: both are the SHA-256
of one file's bytes.

Generation needs `schema.pb` first, so the generate step runs after
`tools/mkschema.sh`. A `schema.pb` rebuilt without regenerating leaves the
files carrying the old hash, which is the mismatch the comparison exists to
report.

The parameter is a path on the build machine, so a generated file depends on
which file the build pointed at. The golden tests point at a fixture holding
three bytes, whose hash is a constant, so the goldens do not move when the
schema's descriptors do.
