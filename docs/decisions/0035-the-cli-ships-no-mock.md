# ADR 0035: The CLI ships no `mock`, and the load test commits in-process

## Status

Accepted

## Context

The specification names `mock` among the CLI's verbs, SPEC 15:

> **Verbs.** `tail`, `send`, `schema`, `ping` and `stats`, plus `record`,

and gives it one line:

> **`mock`.** Synthetic traffic conforming to the schema at a configurable rate.

The plan gave `mock` two jobs. One was a stream a consumer can be written
against before any capture exists. The other was the load generator for the
load test at ten times a captured stream's rate. ADR 0034 rests that test's
load on the broker on `mock`, since `replay` loads no broker.

A `mock` that serves records to `tail` stands where the bridge stands. It
loads no broker either, so it does not do the second job. Records reach the
broker through `commit`, which the Lua side calls in the DCS process, and a
CLI in another process cannot call it.

The scope is being cut to what the release needs.

## Decision

The CLI ships no `mock` verb. `dcsb` has eight verbs.

The load test drives the broker from a Rust test in the broker's own
process. The test calls `commit` at ten times a captured stream's rate,
measured from the mission times inside an epoch (ADR 0034), with consumers
connected over loopback. `replay` drives a consumer under test on its own.

- A `mock` verb that serves records: rejected. It loads no broker, and a
  consumer author has `replay` once one capture exists.
- A `mock` verb that loads a broker: rejected. It needs a way for one process
  to commit into another's broker, which the bridge has no reason to expose.

## Consequences

A consumer can be written only after a capture exists. A capture needs a DCS
install once; after that, `replay` serves it to anyone.

The load test exercises the broker's commit path and its drop policy
directly, without the Lua side's cost per record. The Lua side's cost is the
live session's to measure.

This record replaces the consequence in ADR 0034 that rests the load test's
load on the broker on `mock`. ADR 0034's decision stands.

A request for synthetic traffic from a consumer author would reopen this.
