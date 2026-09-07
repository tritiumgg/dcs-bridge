# ADR 0021: A package names the component that produces its records

## Status

Accepted

## Context

A topic id is the payload's fully-qualified type name, so the package name is
half of every topic a consumer subscribes to. SPEC §5.2:

> **Nobody assigns a topic id.** An adopter or a mission author writes a
> `.proto`, and the type URL falls out of the package and message name. Section
> 8.2 therefore partitions no numbered space for topics, and two independently
> written extensions cannot collide unless they choose the same fully-qualified
> name — which their package names already prevent.

SPEC §8.2 names the packages, and gives one reason for putting both built-in
sets in one of them:

> The bridge's own records live in `dcs.bridge`, **both built-in sets' in
> `dcs.builtin`**, and an adopter's in a package they own. One package for the
> built-ins, because the Lua state a topic is produced from is an
> implementation detail a consumer should not have to read off a topic name;
> the two sets choose distinct message names, and the generator refuses a
> collision.

Two things weigh against that layout. The `dcs.` prefix is not the bridge's:
an adopter can declare `dcs.anything`, and the ownership check SPEC §8.4 asks
for can police the one package it names and nothing beside it. And the
bridge's own records are produced by three components, the broker, the hook
driver and the sim driver, with different lifetimes and different failure
modes, which a consumer reading a capture or filtering a subscription does
have to know: a `Handshake` arrives before authentication, an `EpochOpened`
arrives whether or not a sim driver loaded, and a `ResyncBegan` arrives only
from a sim driver that is running. The spec's reason for one built-in package
is that the producing state is an implementation detail; for the bridge's own
records it is the interface.

The rename is free at this point and a breaking change at any later one: no
release carries the schema, no consumer exists, and `buf breaking` has no
baseline. Task 2.16 registers records by name, Phase 3 generates the routing
tables from the schema, and a release freezes every topic id under SPEC §8.4.

## Decision

The schema has five packages under one prefix, and each names the component
that produces its records:

| Package | Holds |
|---|---|
| `dcsbridge.broker` | What the broker frames, answers, consumes or knows by name, and the enums and message options every other file imports |
| `dcsbridge.hook` | The bridge's own records the hook driver emits or handles |
| `dcsbridge.sim` | The bridge's own records the sim driver emits or handles |
| `dcsbridge.builtin.hook` | The built-in hook set |
| `dcsbridge.builtin.sim` | The built-in sim set |

`tools/schema-ownership.sh` holds the list for each of the bridge's three
packages, leaves the two built-in packages open, and refuses any other
package under `dcsbridge`, including the bare prefix and `dcsbridge.builtin`.
An adopter's records go in a package they own, as before.

`CommandAck` lives in `dcsbridge.broker`. Both drivers emit it, and the
broker is the one component that knows it by name, so neither driver's
package owns it; ADR 0017.

The alternatives, and what rejected each:

- **Keep `dcs.bridge` and `dcs.builtin`.** The prefix is nobody's, so the
  ownership check cannot say what does not belong under it.
- **`dcsbridge.broker` and `dcsbridge.builtin`, no split by driver.** Keeps
  the spec's argument for the built-ins but leaves the bridge's own records,
  from three producers, in one package, where the producer is what a consumer
  needs to know.
- **Split the bridge's own records and not the built-ins.** Two rules for one
  schema, and a built-in record's producer is what decides whether it can
  arrive at all: a hook record arrives with no mission loaded, a sim record
  does not.

## Consequences

Every topic id changes. Nothing observes it: no consumer exists and no
release carries the schema.

A record's package and its `target` message option say the same thing twice.
The generator reads `target` to split its output by Lua state, and can check
that a record's package agrees with it, which turns a record filed in the
wrong package into a build failure. Until it does, the ownership lists are
the only check, and only for the bridge's own records.

A record that changes producer is renamed, and a rename is a topic break,
which SPEC §8.4 forbids after a release. Such a record is a new record in the
new package, and the old one stays until nothing subscribes to it. The
ownership lists say so.

A type URL grows by about ten bytes, from about forty-three to about
fifty-five for a built-in record, under a `max_type_url_bytes` default of
256. SPEC §5.2 prices the URL at about forty-three bytes a frame; the
difference is inside what PROBE-7 measures at task 9.7.

The built-in packages are open, so a name collision between them is a
generator failure and not this check's. The hook set's own enums, `ChatTarget`
and `ChatSource`, live in `dcsbridge.builtin.hook` with the records that carry
them.

This reopens if a record has to be produced by more than one component. The
one such record today is `CommandAck`, and its home is the broker's package
for the reason above; a second one is the sign that the rule needs a fourth
bridge package or a different one.
