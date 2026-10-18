# ADR 0042: `crates/topic` stays hand-written

## Status

Accepted

## Context

ADR 0022 put the broker's own topic names in `crates/topic`, written by
hand, and left a way open:

> **Generate the module from the descriptor set.** The generator does not
> exist yet. When it does, it can emit this crate, and the crate's test
> becomes the check that the checked-in file is current.

The generator exists. It is a protoc plugin that writes Lua into
`target/gen` through `buf generate`. The crate holds only names in
`dcsbridge.broker`, the package the generator skips (ADR 0037). Its own test
reads `proto/dcsbridge/broker/broker.proto` and fails on a constant naming a
message the file does not declare.

## Decision

The generator does not emit `crates/topic`. The crate stays hand-written,
checked by its own test.

Alternatives:

- **The plugin writes a Rust file as well.** It would read the package it
  skips for every other purpose, buf would write Rust into `crates/` from a
  configuration that writes Lua into `target/`, and a check that the
  checked-in file is current would duplicate the test the crate already
  has.

## Consequences

A message added to `broker.proto` that the broker must know by name needs a
hand-written constant. Nothing fails when that constant is missing; the test
catches only a constant whose message is gone.
