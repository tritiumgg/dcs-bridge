# ADR 0027: The required capability rides with the record from `begin`

## Status

Accepted

## Context

The specification enforces a capability per topic in both directions,
SPEC 14.4:

> **Enforcement is per topic.** The generator emits topic-to-capability
> tables and the hook driver and the sim driver register them like the class
> tables (Sections 5.1 and 8.3). Inbound, the broker refuses a record whose
> required capability the token lacks, with `Rejected` reason
> `NO_CAPABILITY` (Section 5.2). Outbound, it withholds a record whose
> capability the connection lacks, at fan-out, before `seq` assignment,
> counted in `records_filtered_total` and never in `records_dropped_total`
> (Section 5.2). Point-to-point records are addressed, not fanned out, so
> the outbound filter does not apply to an acknowledgement, a typed reply or
> a `Rejected`.

and orders the filter against numbering, SPEC 5.2:

> **Sequence numbers.** The broker assigns `seq` per connection, monotonic,
> after the capability filter (Section 14.4) and before the drop decision. A
> gap in `seq` means records were dropped, and only dropped: a record a
> connection is not entitled to was filtered before numbering, so filtering
> leaves no gap.

It does not say how the writer thread learns which capability a record
needs. The writer thread fans out from one ring the logic thread fills (ADR
0011), and what the ring carries is an encoded envelope tail: opaque bytes,
with the topic inside the `Any`'s type URL behind two length prefixes. The
capability tables live in the registry, behind a read lock, and the fan-out
module is built under Loom, where the registry and its types are not. The
writer thread holds no registry and cannot name a registry type.

The inbound side has the registry at hand: the reader thread already asks
it for the route (ADR 0024), and a session already carries its token's
capability set for `SetEnabled` (ADR 0026), which is the one topic the
reader checked before this.

## Decision

The capability a record needs is looked up once, when the record is opened,
and travels with it to fan-out. `begin` and `begin_to` already ask the
registry whether the topic is complete; that answer is now the capability
itself, kept beside the record in progress, and `commit` hands it to the
broker with the tail. The commit ring carries it as the capability's
number.

The writer thread holds each connection's capability set as a mask of
numbers, delivered by the `Authenticated` control that already marks the
connection as receiving. At fan-out the mask is asked before the record is
numbered; a connection it does not cover is passed over, its `seq` unmoved,
and the pass-over counted in `records_filtered_total`, one per connection
per record. An addressed record is not asked: the connection it names sent
the command it answers. The acknowledgement's capability is `command`,
which is what it answers; it is only ever addressed, so the value is
carried and never consulted.

Inbound, the check lives beside the route lookup, under the same read of
the registry: a routed topic whose registered capability the session lacks
is handed back to the reader thread, which answers `NO_CAPABILITY` with the
sender's `seq` and the topic and keeps the connection, as it does for an
unknown topic. A routed topic with no capability registered is refused the
same way: nothing says the token covers it, so it fails closed as a
`begin` on such a topic does. The route is asked first, so an unrouted
topic stays `UNKNOWN_TOPIC` whatever the token holds.

Alternatives:

- **Look the topic up on the writer thread.** A registry read lock per
  record per pass, on the thread every consumer waits on, and a type the
  Loom build cannot see.
- **Parse the type URL out of the tail at fan-out.** A decode per record
  where the logic thread has the answer for free, and still the lookup.
- **A set type shared with the registry.** `HashSet` under Loom's `Arc` and
  a hash per connection per record; the mask is one AND.
- **Check the capability before the rate windows.** A read-only token
  flooding commands would then cost nothing against its own rate; the
  refusal is a refusal of a record for Lua and is counted as one.

## Consequences

The mask holds numbers below 64. The built-in numbers are 1 to 3 and the
bridge's range is 1 to 49, so every registered capability fits; a number
past the mask is neither added nor covered, so a record needing one is
withheld from everyone rather than disclosed. An adopter's range, 100 and
above, does not fit, and the registry refuses it today. The mask widens
when the registry admits that range, which has no task.

Every record for Lua is refused or delivered under the token's capability
set; a route registered with no capability now refuses what it used to
deliver. The generated tables carry both, and a hand-written route without
its capability is the misconfiguration `partial_registration_total`
already counts at `begin`.

`records_filtered_total` is read through the outbound path and nothing
reports it yet; `stats` picks it up when it lands.

The `Authenticated` control carries the set once. A token's capabilities do
not change for the life of a session, so nothing re-sends it; revocation
dropping a session is the broker-hardening row and has no owner.
