# ADR 0010: `CallbackHz` is `LOSSY`, not `LIFECYCLE`

## Status

Accepted

## Context

SPEC 9 lists thirteen `LIFECYCLE` topics. Twelve of them are edges: a mission
load began, a mission loaded, an epoch opened, an epoch closed, a resync
started, a sim driver reloaded. The thirteenth is `CallbackHz`, which SPEC 9.3
computes by counting `onSimulationFrame` invocations against
`DCS.getRealTime()` and describes as "the render-loop callback rate for the last
second". It is a periodic gauge, and it is the only periodic member of the set.

The specification never argues that membership. SPEC 8.1 gives one test:

> **Every `LIFECYCLE` topic must have last-value semantics.** The broker retains
> the latest record per `LIFECYCLE` topic and replays the retained set to each
> newly authenticated connection (Section 5.2), so a topic's latest record must
> be meaningful on its own. The thirteen Section 9 topics qualify.

That test is necessary and not sufficient. `CallbackHz` passes it — the latest
reading does mean something alone — but so would a great many records that are
plainly not lifecycle events. Nothing else in the document defends the choice
topic by topic.

What the class actually confers is stronger than last-value semantics. SPEC 8.1
again: `LIFECYCLE` is "never discard. Reserve headroom in the outbound ring,
ahead of every other class." That guarantee exists for records whose loss
silently corrupts a consumer's view. Miss an `EpochClosed` and every unit
reference held is invalid with nothing to say so. Miss a `CallbackHz` and the
next one arrives a second later carrying a better number.

The cost is concentrated rather than spread. SPEC 5.2 drops a connection whose
ring fills with `LIFECYCLE` alone, and gives its reason:

> the consumer is so far behind that it has already missed epoch boundaries and
> holds references into a world that no longer exists. Its view is
> unrecoverable.

A render-loop rate is not an epoch boundary. So the class whose saturation
triggers that disconnect is, in practice, filled almost entirely by the one
topic that has nothing to do with the reason for disconnecting. SPEC 13.1 says
as much where it sets the reserve: "`CallbackHz` alone adds one per second while
a consumer stalls." Every other lifecycle topic arrives a handful of times per
mission.

ADR 0009 sizes the outbound `LIFECYCLE` ring, and that arithmetic is entirely
this one topic's. At one record a second it wants 4096 slots to hold an hour.
Without it the same ring holds many mission rotations in a few hundred.

## Decision

`CallbackHz` is `LOSSY`. The bridge defines twelve `LIFECYCLE` topics, eight of
them the hook driver's and four the sim driver's.

`LOSSY` is what SPEC 8.1 describes as "discard freely under pressure", which is
the correct answer for a reading that a later reading supersedes. A consumer
that misses one is not missing information; it is missing a sample of a value it
is about to be told again.

`CallbackHz` is no longer retained or replayed. A consumer that connects learns
the callback rate on the next emission rather than immediately, which is a wait
of about one second.

The alternatives, and the line that rejected each:

- **`DURABLE`.** It means never discard silently and count every drop, so a
  stalled consumer would generate a steady counted drop per second for a gauge
  nobody misses, and `records_dropped_total` would carry that noise for the life
  of the connection.
- **Leave it `LIFECYCLE` and size the ring for it.** That is ADR 0009's other
  branch. It spends 4096 slots per connection, and it makes the disconnect
  threshold a function of a frame-rate gauge rather than of missed epochs.
- **Leave it `LIFECYCLE` and exempt it from the reserve.** A class with an
  exception is two classes with one name, and the registration maps carry one
  class per topic.

## Consequences

SPEC 9's thirteen become twelve, and SPEC 8.1's "the thirteen Section 9 topics
qualify" now covers twelve. The plan's task 6.4 counts them and is edited to
match. Nothing else counts on the number: `max_lifecycle_topics` is a bound of
64, with the bridge's own set well inside it either way.

The retained set loses a member, so the standing memory SPEC 13.1 attributes to
retention falls by one slot. That is 16 KiB of a stated 1 MiB and changes
nothing.

`CallbackHz` becomes droppable, which is the point, and it will be dropped
first, ahead of `DURABLE`, whenever a consumer falls behind. A consumer that
wants an uninterrupted rate series cannot have one from this topic. Nothing in
the documents asks for one; SPEC 9.3 presents it as a diagnostic.

The `LOSSY` ring gains one record a second. Against the 3,040 records a second
SPEC 10 models at heavy load, that is not a rate worth sizing for.

This reopens if `CallbackHz` acquires a consumer that needs every sample — an
alerting rule on frame-rate dips, say, where a missing sample and a healthy
sample look the same. The answer then is a counter or a windowed statistic
inside the record, not a stronger class: a gauge that must not be dropped is a
gauge that should carry its own history.
