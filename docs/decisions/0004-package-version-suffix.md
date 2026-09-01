# ADR 0004: `buf lint` runs without `PACKAGE_VERSION_SUFFIX`

## Status

Accepted

## Context

SPEC §8.4 puts `buf lint` in CI, and buf's standard rule set requires every
package name to end in a version component such as `v1`. The bridge's package
is `dcs.bridge`, so the rule fails on the first file the schema contains.

The package name is not a naming preference here. SPEC §5.2:

> **The topic is the payload's type, and protobuf already names it.** An `Any`
> carries the serialised message as bytes together with a type URL of the form
> `type.googleapis.com/<package>.<Message>`. That URL is the topic id. There is
> no topic table to negotiate, no number to allocate, and no registry to
> coordinate: a message's identity is its fully-qualified name, which the schema
> already fixes and every protobuf runtime already exposes through its
> descriptor.

The package also partitions the topic space. SPEC §8.2:

> **Topics are not in that table, because topics are not numbered.** A topic is
> the payload's fully-qualified type name (Section 5.2), so package naming does
> the partitioning that a numbered range would otherwise have to. The bridge's
> own records live in `dcs.bridge`, **both built-in sets' in `dcs.builtin`**, and
> an adopter's in a package they own.

So the package name is wire contract twice over: it is half of every topic id a
consumer subscribes to, and it is the boundary SPEC §8.4's ownership check
polices. Renaming it to `dcs.bridge.v1` renames every topic the bridge, both
built-in sets and every adopter publish.

## Decision

`buf.yaml` uses the `STANDARD` rule set with `PACKAGE_VERSION_SUFFIX` excepted,
and no other exception. Every other standard rule holds, including the
`_UNSPECIFIED` zero value, the enum-name value prefix and snake_case fields
that SPEC §8.2 names `buf lint` as the enforcer of.

The alternatives:

- **Adopt `dcs.bridge.v1`.** Renames every topic id and contradicts three
  specification sections that spell `dcs.bridge` out.
- **Drop to a narrower rule set.** Loses the rules the specification relies on
  to catch a real wire mistake, to silence one that catches none.
- **Version the package on a breaking change.** buf's rule exists for schemas
  that publish `v1` beside `v2`. Two live topic namespaces is a bridge design
  the specification does not have, and SPEC §8.4 forbids the breaking change
  that would call for one.

## Consequences

- A breaking schema change has no in-band way to announce itself. SPEC §8.4
  already forbids one — field numbers are permanent and `buf breaking` runs
  against the previous release — so the exception removes an escape hatch the
  design had closed anyway.
- The exception is a standing invitation to add a second. Each one is a rule
  the specification named as its enforcement, so a second needs a record of its
  own arguing why that rule does not apply.
- This reopens if the bridge ever has to run two topic namespaces at once. That
  is a `buf breaking` failure first, and this record is the second thing to
  revisit after it.
