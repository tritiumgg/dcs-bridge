# ADR 0038: A generated file returns its tables, and the driver registers them

## Status

Accepted

## Context

The generator writes two files, one per Lua state, and the specification
places them differently, SPEC 8.3:

> Its tables register into `DCSBridge.code` at each load, per Section 6.1,
> so a reload replaces them.

and of the hook driver's file:

> It is outside `DCSBridge.code` deliberately: a hook-driver-targeted topic
> must work when no sim driver is loaded.

It says the drivers register the tables, the sim driver at load and the
hook driver at DCS start, and it names no shape for the file. ADR 0023's
consequences read the other way:

> The generator emits one more table and one more call per target file

which puts the `shim.classes`, `shim.routes`, `shim.caps` and
`shim.replies` calls inside the generated file. No driver exists yet to
weigh it against.

## Decision

A generated file is a chunk that builds its tables and returns them, and
makes no `shim` call. The driver that loads it registers the tables.

- **`SimDriver.gen.lua`** assigns `topics`, `classes`, `routes`, `caps`,
  `replies` and `schema_sha256` into `DCSBridge.code`, fresh each load, and
  returns `DCSBridge.code`.
- **`HookDriver.gen.lua`** builds the same six fields in a local table,
  touches no global, and returns it.
- **The four maps hold only the file's own target's rows.** `classes` and
  `caps` name every record of that target; `routes` names its `COMMAND`
  records, since an outbound record has a class and a capability and no
  route; `replies` lists the replies its requests name.
- **Values are the member names the shim reads**: `durable`, `lossy`,
  `command`, `lifecycle`; `read`, `command`, `reload`; `sim_driver`,
  `hook_driver`. A capability numbered past the bridge's own members is
  written as its number.
- **Rows are sorted by topic**, so a schema change moves the lines it
  touches and no others.

This amends the line of ADR 0023 quoted above: the generator emits one more
table per target file, and the driver makes one more call.

Alternatives:

- **The file makes the four calls.** Loading it would need the shim open
  and configured first, so no test could read the tables without a broker,
  and a driver could not check `schema_sha256` before it registers.
- **Both files write into `DCSBridge.code`.** The hook driver's file has to
  load when no sim driver is loaded.

## Consequences

Each driver carries four registration lines and a hash check, written once
by hand rather than generated into every file.

A stock Lua 5.1 loads both files and registers their tables with no DCS and
no configured broker, which is what lets the files be tested off-platform.

Emitters, send wrappers and decoders join the same files later and follow
the same placement: the sim side under `DCSBridge.code`, the hook side in the
returned table.
