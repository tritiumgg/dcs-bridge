# ADR 0044: The wire reader is generated into each file that decodes

## Status

Accepted

## Context

Every decoder needs the same reads: a varint, a length, a fixed-width value,
a skip by wire type. SPEC 8.3 says the generator emits the decoders, and
says nothing of where the code they share lives.

A generated file loads with nothing around it but the shim. The hook file
touches no global (ADR 0038), and the sim file reaches only
`DCSBridge.code`. The driver runtime that could own a shared reader does
not exist yet.

## Decision

The generator writes the reader as locals into each file that holds a
command, once per file, ahead of the decoders. A file with no command
carries no reader.

The reader uses only `string.byte`, `string.sub`, `math.floor` and
`math.ldexp`, which DCS's Lua 5.1 and a stock Lua 5.1 both have. It does
arithmetic in place of bit operations, since Lua 5.1 has no bit library.

Alternatives:

- **A shared Lua module the drivers load.** It would need a path both Lua
  states can reach and a load order, and the hook file could not reach it
  without a global.
- **Decoding in the broker,** handing Lua a table. The broker forwards a
  command's body opaque and holds no descriptor for it.

## Consequences

The reader ships twice when both a package's sim file and its hook file
hold a command, and once more in each adopter package that has one. It is
about a hundred and fifty lines.

A fix to the reader reaches a driver only when its files are generated
again.
