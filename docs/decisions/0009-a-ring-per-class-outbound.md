# ADR 0009: The outbound path keeps one ring per class

## Status

Accepted

## Context

SPEC 5.2 gives the outbound ring a drop rule in two halves:

> - **Evict the oldest record that is not `LIFECYCLE`.** Plain drop-oldest is
>   not enough. The oldest record in a saturated ring may well be an
>   `EpochClosed`, and evicting it is exactly the failure this rule exists to
>   prevent. Count every eviction by class.
> - **Refuse the newest non-`LIFECYCLE` record once free space falls to
>   `ring_out_lifecycle_reserve`.**

and an end to it:

> **A ring that is full of `LIFECYCLE` is a disconnect, not a drop.** If
> eviction finds no non-`LIFECYCLE` record to remove, the consumer is so far
> behind that it has already missed epoch boundaries and holds references into a
> world that no longer exists.

The classes are not two but three on this path, and they are ranked. SPEC 10
states the rank in one line — "the drop policy is Section 5.2 and Section 8.1:
`LOSSY` before `DURABLE`, `LIFECYCLE` never" — and SPEC 8.1 gives each class
its own sentence: `LOSSY` is discarded freely under pressure, `DURABLE` is never
discarded silently while a consumer is connected, and `LIFECYCLE` is never
discarded at all. `COMMAND` is inbound and does not appear here.

So eviction is a search over the ring's whole contents, in priority order: take
the oldest `LOSSY`, and failing that the oldest `DURABLE`, and failing both drop
the connection.

**ADR 0008's ring cannot perform that search, and no tuning of it can.** A slot
there is addressed by its record number: record N occupies slot N modulo
capacity. The producer publishing record N can free exactly one slot, the one
holding record N minus capacity, which is the oldest record. If that record is
`LIFECYCLE`, no other slot will serve, because freeing an interior slot does not
free the slot the producer must write. The limit is the addressing, not the
protocol.

Keeping one ring therefore means giving up array addressing for a slab and a
free list — a lock-free structure supporting interior removal, which is harder
than the ring ADR 0008 describes and is not wait-free. The producer here is the
writer thread, and behind it the logic thread.

**Ordering is not the obstacle it appears to be.** SPEC 5.2:

> **Sequence numbers.** The broker assigns `seq` per connection, monotonic,
> after the capability filter and before the drop decision. [...] Ordering is
> total per connection.

A record is numbered before it reaches any ring. Three rings drained by a merge
on `seq` therefore deliver exactly the order one ring would, at one comparison
per record. An earlier reading of this record had the split paying for itself in
reordering; it does not.

**The disconnect is what a split really changes, and the numbers are not
small.** `CallbackHz` is a `LIFECYCLE` topic carrying the render-loop rate for
the last second, so it arrives once a second whether or not anything else does.
SPEC 13.1 says so where it sets the reserve: "`CallbackHz` alone adds one per
second while a consumer stalls."

Take a consumer that stops reading, at the defaults of `ring_out_records` 4096
and `ring_out_lifecycle_reserve` 64.

Under one ring, the ring fills, and non-`LIFECYCLE` records are refused once
free space reaches 64. `LIFECYCLE` keeps arriving and evicts the oldest `LOSSY`,
then the oldest `DURABLE`. The connection survives until nothing but
`LIFECYCLE` is left — about four thousand records, so on the order of an hour.

Under a split whose `LIFECYCLE` ring is literally the reserve, 64 slots, that
ring is full after 64 records: about a minute.

Worse, SPEC 5.2 replays the retained set to each newly authenticated connection,
and SPEC 13.1 bounds that set by `max_lifecycle_topics`, default 64. A
`LIFECYCLE` ring of 64 can be filled by the replay alone, before the connection
has seen a single live record.

**The reserve does not survive translation into a capacity.** It was a floor on
free space inside a ring that `LIFECYCLE` could also grow into by eviction. A
capacity is a ceiling. Reading one as the other shortens a consumer's survival
from about an hour to about a minute.

That arithmetic is what prompted ADR 0010. Every figure above is one topic's:
`CallbackHz` is the only periodic member of a class otherwise made of edges, and
it is what turns a disconnect threshold into a stopwatch. ADR 0010 moves it to
`LOSSY`, and the sizes this record settles assume that. Without it the
`LIFECYCLE` ring is 4096 slots holding a frame-rate gauge; with it the ring is a
few hundred holding mission boundaries, which is what SPEC 5.2's disconnect is
about.

## Decision

Each connection gets three outbound rings, one per class: `LOSSY`, `DURABLE`
and `LIFECYCLE`. Each is the ring ADR 0008 describes, unchanged.

The drop rule becomes structure rather than search. A `LOSSY` record evicts only
`LOSSY`. A `DURABLE` record evicts only `DURABLE`. The `LIFECYCLE` ring never
evicts, and a push into a full one drops the connection and counts
`lifecycle_disconnects_total`. Eviction is O(1) and the priority is which ring a
record is in.

The writer thread drains the three by merging on `seq`, lowest first. `seq` is
assigned before the record reaches a ring, so the merge reproduces the total
order per connection, and a gap in it still means records were dropped and only
dropped.

The `LIFECYCLE` ring is **not** sized at `ring_out_lifecycle_reserve`. It is
sized at what "full of `LIFECYCLE` alone" meant under one ring, which is
`ring_out_records`. `ring_out_lifecycle_reserve` is retired: it partitioned a
ring that no longer exists. SPEC 13.1 already calls it provisional "because it
partitions a ring whose size is also provisional", and PROBE-7 at task 9.7 is
where all three sizes get a measured basis.

Two of the three sizes can be derived from what the documents already state,
and one cannot. The one that cannot is the split between `LOSSY` and `DURABLE`.
SPEC 10 models a heavy load as 500 units and 3,040 records per second, but that
traffic is adopter-registered mission data — unit positions and events — and
the bridge's own schema says nothing about how it divides by class. Only a
capture from a real adopter settles it, which is PROBE-7's job at task 9.7.

What is derivable is the total and the two ends of it. At 3,040 records per
second, today's `ring_out_records` of 4096 is about 1.4 seconds of consumer
outage, so any split has to preserve roughly that total or say it is changing
it. The bridge's own `DURABLE` traffic is bounded by keys that are already set:
`CommandAck` follows inbound commands at `inbound_records_per_sec`, 100 per
connection, and a broker answer counts as `DURABLE` under the drop rule at
`rejected_max_per_sec`, 10 per connection. So the bridge alone cannot push more
than about 110 `DURABLE` records a second at a connection, whatever an adopter
adds on top.

The `LIFECYCLE` ring derives cleanly once ADR 0010 moves `CallbackHz` to
`LOSSY`. What remains are twelve topics that are all edges — mission loads,
epoch boundaries, resync brackets, sim driver loads — so they arrive a handful
of times per mission rather than at any rate. Replay adds up to
`max_lifecycle_topics`, 64, at the moment a connection authenticates. Capacity
is therefore 64 for the replay plus room for the boundaries a stalled consumer
may miss before its view is beyond saving, which is what SPEC 5.2 says the
disconnect is for.

The provisional values, to be replaced by PROBE-7: `ring_out_lossy_records`
3584, `ring_out_durable_records` 512, `ring_out_lifecycle_records` 256. The
first two preserve today's 4096 total while giving `DURABLE` about five seconds
against the bridge's own traffic, and their ratio is a placeholder rather than a
finding. The third holds the 64-slot replay and 192 boundary records beyond it,
which at six to ten records per mission cycle is twenty mission rotations —
generous for a condition that means a consumer has stopped reading entirely.

That leaves 4352 slots a connection against today's 4096, so the split costs
almost nothing. Had `CallbackHz` stayed `LIFECYCLE`, the same ring would have
needed 4096 slots on its own to hold the hour that one ring holds today, and the
split would have doubled the outbound memory to buy it.

The alternatives, and the line that rejected each:

- **One ring with a slab and a free list, supporting interior removal.** It is
  the only way to keep one ring, and it trades a wait-free push for a structure
  that is harder to write and harder to check, on the thread that must not
  stall.
- **One ring that refuses the incoming record when its oldest is
  `LIFECYCLE`.** The incoming record is often `LIFECYCLE` too, so this discards
  a boundary record — the exact failure the rule exists to prevent.
- **Three rings with the `LIFECYCLE` ring sized at the reserve, 64.** The
  retained-set replay is bounded by 64 and can fill it before a live record
  arrives, whatever else is emitted.
- **Two rings, `LIFECYCLE` and everything else.** It leaves `LOSSY` before
  `DURABLE` needing a search inside the second ring, which is the problem this
  record exists to remove.

## Consequences

Capacity stops being shared. Under one ring a quiet `LOSSY` stream left its
space to `DURABLE`; under three, each class holds what it was given and cannot
borrow. That is less efficient in memory and it is what buys the O(1) priority.

The memory it costs is close to nothing. The three provisional sizes come to
4352 slots a connection against today's 4096, so across `max_connections` of 8
the slot overhead is a stamp and a handle apiece — of the order of a megabyte,
against the 1 MiB the retention set already stands. The records themselves
dominate either way, and at the roughly 64 bytes SPEC 13.1 implies by pairing
`drain_max_records` 256 with `drain_max_bytes` 16 KiB, full rings across every
connection are a couple of megabytes more.

`DURABLE` gains a guarantee it did not have. A `LOSSY` flood can no longer crowd
it out at all, where before it was protected only by being evicted second. This
is a strengthening, and worth knowing when reading `records_dropped_total`: the
classes now fill and drop independently.

The drain gains a three-way merge, one comparison per record, on the writer
thread. That thread already assigns `seq` and applies the drop rule, so the
merge joins work that is already there. It is not on the logic thread.

Three sizes now need measuring where two did. PROBE-7 was already going to size
`ring_out_records` and the reserve. It sizes `ring_out_lossy_records`,
`ring_out_durable_records` and `ring_out_lifecycle_records` instead, and the
`LIFECYCLE` figure carries the argument above: it is a disconnect threshold in
seconds of consumer stall, not a buffer.

The keys arrive before the structure does. Task 2.15's done-when is "rings size
from config", three tasks ahead of 2.18, so how many outbound sizes exist has to
be settled there. This record is what settles it, and nothing in 2.15 needs the
drop rule itself.

This reopens if a periodic topic joins `LIFECYCLE` again. The `LIFECYCLE` ring
is sized for edges, and one topic arriving on a timer turns its capacity into a
countdown, which is how the sizing came to be examined in the first place. An
adopter registering such a topic is the case to watch, since SPEC 8.1 lets one
in on the last-value test alone.
