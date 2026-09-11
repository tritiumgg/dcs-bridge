# ADR 0026: The rate caps are fixed one-second windows on the reader thread

## Status

Accepted

## Context

The specification states four rates and says what each answers, SPEC 14.5:

> Rate-limit inbound at `inbound_records_per_sec` per connection **and at
> `inbound_records_per_sec_total` across all of them**. [...] A connection
> over its own limit is misbehaving alone: refuse the record with `Rejected`
> reason `RATE_LIMITED` and keep the connection, bounded by
> `rejected_max_per_sec` so the refusal cannot be amplified. A connection
> that pushes the aggregate over is a capacity problem no refusal fixes:
> disconnect it.

and SPEC 5.2:

> **The broker emits at most `rejected_max_per_sec` `Rejected` records per
> connection for `UNKNOWN_TOPIC`, `NO_CAPABILITY` and `RATE_LIMITED`, and
> `busy_max_per_sec` for `BUSY`**; refusals above that rate are counted and
> not answered, so a flood cannot buy amplification.

It does not say how a rate is measured, what an inbound record is for the
purpose, where the shared total lives, or whether a refusal answered with
nothing is still counted under its reason. Each is a choice the build has to
make.

The reader thread is one per connection (ADR 0018) and the rings it feeds
keep their producer behind a lock only reader threads take (ADR 0024). The
logic thread never waits on a reader thread, and every choice below keeps
that.

## Decision

Each rate is a fixed one-second window: a count that opens at the first
event after the last window closed, holds for a second, and admits what
fits under the cap. A refused event is not counted, so a cap of zero
refuses everything. The cap is read from the configuration in force at each
event, the way the frame cap is, so a rate a later `configure` lowers binds
on the next event.

The three per-connection windows live on the reader thread's stack, beside
its frame buffer: what the connection sent for Lua, what it was told of
its refusals for an unknown topic, a missing capability or its own rate,
and what it was told of full rings. The total is one window on the bridge,
behind a mutex reader threads take for the length of one count. The logic
thread never takes it, which is the argument ADR 0024 made for the inbound
producers.

An inbound record, for `inbound_records_per_sec` and its total, is a record
for Lua: a frame the reader hands to the rings. The messages the broker
answers itself, `Ping`, `Auth`, `GetSchema`, `SeqAck` and `SetEnabled`, are
not counted. The total exists to protect what the sim driver can dispatch,
and they never reach it; and `busy_max_per_sec` is non-binding only if what
the inbound limit counts is what a full ring can refuse.

A refusal is counted under its reason whether or not it was answered, and
one answered with nothing is counted as suppressed as well. So
`commands_rejected_total` is the count of refusals and
`rejections_suppressed_total` the part of it a consumer never heard.

The capability check on a record for Lua is not made here. `NO_CAPABILITY`
answers `SetEnabled` from a token without `reload`, which is the one
capability the reader checks today; the check on every other topic lands
with the capability filter.

Alternatives:

- **A token bucket.** The same code size with a refill per millisecond, and
  a test cannot state what it admits without arithmetic. A fixed window
  admits at most twice its cap across a boundary, which every cap here can
  afford: three of them bound log noise, and the fourth disconnects, where
  lenient is the safe side.
- **Every frame after authentication counts.** A `Ping` flood would be
  refused with `Rejected`, which is one answer for one, the same as `Pong`;
  and the barrier the tests rely on, a `Ping` behind a burst, would be
  refused with the burst.
- **A lock-free total.** An atomic pair with a compare-exchange on the
  rollover: a second algorithm to check, for a lock contended at most at
  the total's own rate.
- **Count only answered refusals by reason.** A flooding consumer would hold
  `commands_rejected_total` at the cap and hide the flood behind it.

## Consequences

A `Ping` flood after authentication is bounded by nothing in the broker.
Each costs the writer thread one `Pong`, pushed on the flooder's own ring,
where it evicts the flooder's own frames first; the writer thread's time is
what is shared.

The disconnect for the total is counted by nothing of its own. It shows as
a session closed, and the specification names no counter for it; one is
added when `stats` needs it.

The total drops whichever connection arrived last, not whichever is
greediest, as the specification says it will. A well-behaved consumer can
lose its session to a noisy neighbour and reconnects.

The window's clock is the caller's, so the unit tests drive it; the
loopback tests send a burst in one write and rely on it arriving well
inside a second, which loopback does.

This reopens if PROBE-7 at task 9.7 reads the total's lock as a measurable
share of the inbound path, or if a rate is ever stated in a unit other than
per second.
