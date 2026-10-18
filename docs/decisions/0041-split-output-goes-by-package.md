# ADR 0041: Split output goes by package

## Status

Accepted

## Context

An adopter declares records of their own and runs the generator over them.
SPEC 8.3:

> The shipped messages generate into the bridge tree's two `.gen` files. An
> adopter's own messages generate into a `*.gen.lua` that rides with their
> extension source, carrying its own schema-slice hash.

and SPEC 6.10.1 loads that file after the shipped ones:

> A later file overrides an earlier one by key. Within an extension source,
> `*.gen.lua` files load before the rest.

Neither names the adopter's files, says how its rows join the shipped ones
in `DCSBridge.code`, or says what reads the slice's hash. The generator
writes `SimDriver.gen.lua` and `HookDriver.gen.lua` for every request, so an
adopter's run would overwrite the shipped names with a file holding only
the adopter's rows.

## Decision

The generator groups records by package and writes a pair of files for each
group.

- **Records in `dcsbridge` or a package under it** write `SimDriver.gen.lua`
  and `HookDriver.gen.lua`. The pair is written only when the request names
  a file in such a package.
- **Every other package named in the request** writes
  `<package>.sim.gen.lua` and `<package>.hook.gen.lua`, such as
  `fixture.adopter.sim.gen.lua`. A file of that package with no records
  still writes the pair, so the names a run produces depend only on the
  files it is given.
- **The adopter's sim file returns a slice table of its own** holding
  `topics`, `classes`, `routes`, `caps`, `replies` and `schema_sha256`. It
  adds its topics, emitters and wrappers into the `DCSBridge.code` tables
  the shipped file built, by key, and leaves the shipped rows alone. The
  driver registers the slice's four maps the way it registers the shipped
  ones.
- **The adopter's hook file** has the shape of the shipped hook file: a
  local table, no global, returned to the driver.
- **The slice's hash is of the `schema.pb` its own run names** (ADR 0039).
  Nothing compares it yet. The broker holds one schema and hashes it, and a
  slice built from a different `schema.pb` would mismatch it by design.
- **An adopter name that equals a shipped one replaces it** in
  `DCSBridge.code.topics`, `emit`, `send` and `send_to`. That is SPEC
  6.10.1's later-overrides-earlier, and the topics table is keyed by bare
  name (ADR 0021). The two topics stay distinct in the broker, which keys
  by full name.

Alternatives:

- **One `<package>.gen.lua` for both sides.** The hook driver would load a
  file that writes into `DCSBridge.code`, which ADR 0038 rules out.
- **A plugin parameter naming the output files.** Two adopters' parameters
  could collide, and a package already names an adopter's schema uniquely.
- **The adopter's sim file assigns fresh tables.** It would wipe the
  shipped rows.

## Consequences

A run over the shipped tree writes the shipped pair and nothing else, and
`tools/schema-ownership.sh` keeps a package outside `dcsbridge` out of
`proto/`. A run over an adopter tree writes only the adopter's pair, because
the shipped files are imports and are never named for generation.

The shipped file's reload assigns its tables fresh, so the driver loads the
adopter's file again after it, in the SPEC 6.10.1 order, or the adopter's
rows disappear.

An adopter can replace a shipped send wrapper by declaring a record of the
same name, with no warning. That is the override the load chain allows, and
it is also how a typo would hide a shipped member.

What reads the slice's hash, and what it is compared with, is open. A
second schema registered beside the shipped one would settle it.
