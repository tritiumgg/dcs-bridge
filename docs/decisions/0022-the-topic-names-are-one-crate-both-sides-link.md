# ADR 0022: The topic names are one crate both sides of the wire link

## Status

Accepted

## Context

A topic is the payload's fully-qualified type name, so the broker and
`dcsb` both spell every topic the broker knows by name, and both spell the
prefix every type URL carries. Spelled as literals, a rename touches every
site and a typo in one is a wire bug no test catches unless it happens to
assert that string.

Two rules stand in the way of sharing one spelling. ADR 0016:

> The broker's shipped build links no crate outside `std`. Dependencies that
> reach only tests, Loom models or the CLI and generator binaries are free,
> because none of them runs inside DCS.

And `crates/cli/Cargo.toml`, which has `dcsb` link no part of the broker,
so that what it knows of the wire it decodes through a stock protobuf
library and a misreading in the encoder cannot be shared by the tool that
checks it.

The broker cannot take the names from the schema at build time: ADR 0016
keeps `prost-build` out, and the broker sees the descriptor set only at run
time, handed over by the hook driver.

## Decision

`crates/topic`, package `dcsbridge-topic`, holds the type URL prefix and the
bridge's own topics, and the broker and `dcsb` both depend on it. It is
`no_std`, has no dependencies, and holds constants and nothing else. A
macro builds each topic from the package, and the crate's own test reads
`proto/dcsbridge/broker/broker.proto` and fails on a constant that names a
message the file does not declare.

ADR 0016's rule is about code running inside the simulator, and each
exception it names, `prost` and then `sha2` (ADR 0020), is code. A crate of
the project's own holding string constants runs nothing, so it is not a
third exception; the rule reads "no crate outside `std` that runs code."
The CLI's rule is about encoder logic, and a name is not encoder logic.

The alternatives, and what rejected each:

- **A `topic` module per crate.** Two spellings of every name; a rename
  touches both, and the two sets can drift.
- **Generate the module from the descriptor set.** The generator does not
  exist yet. When it does, it can emit this crate, and the crate's test
  becomes the check that the checked-in file is current.

## Consequences

Both sides read the same constant, so a wrong one is agreed on by both and
no loopback test sees it. The crate's schema test is the one guard, which
is why it runs over every constant rather than the two the broker matched
by name before.

A fifth workspace member, an rlib that ships nowhere. The crate table in
`docs/developing.md` lists it.

The `Cargo.toml` comment on the broker's dependencies cites this record
beside ADR 0016 and ADR 0020, so the dependency list and its reasons stay
in one place.
