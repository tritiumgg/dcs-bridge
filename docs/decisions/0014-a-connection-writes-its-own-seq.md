# ADR 0014: A connection's thread writes `seq`, and the rest of the envelope is shared

## Status

Accepted

## Context

SPEC 5.2 fixes the frame and numbers it per connection:

> **Frame format.** `[u32 length, little-endian][payload]`.
>
> **Payload.** Protobuf wire format. Each frame is one `Envelope`.

> **Sequence numbers.** The broker assigns `seq` per connection, monotonic,
> after the capability filter (Section 14.4) and before the drop decision. A
> gap in `seq` means records were dropped, and only dropped: a record a
> connection is not entitled to was filtered before numbering, so filtering
> leaves no gap.

> **Only the writer thread pushes to an outbound ring.** [...] The writer
> thread then does all three things that must agree with each other: it
> assigns `seq`, it applies the drop rule, and it pushes.

So the one field of the envelope that differs between connections is `seq`,
and it is known only on the writer thread, after the record has left the
logic thread. Everything else in the envelope, the epoch, the mission time and
the `Any` holding the record, is the same for every connection. A frame
cannot be built once on the logic thread, because `seq` is not known there,
and cannot be built once on the writer thread, because there are as many
frames as connections.

Two rules constrain where the bytes may be assembled. SPEC 5.1:

> **The put calls add no allocation beyond producing the value.** The
> embedded broker writes into a preallocated buffer.

And SPEC 5.2 has the writer thread pushing into every connection's ring, so
whatever the ring carries is pushed once per connection; a copy of the frame
per connection would make the writer thread's cost per record proportional to
the record's size times the connection count.

Two smaller things need an owner. ADR 0011 gave each connection a thread that
drains its ring and left it no way to wait on the ring. And both Lua states
commit records, through one `commit` that SPEC 5.2 makes single-producer, with
the rule that the logic thread takes no lock.

## Decision

The commit ring carries the envelope tail, every field but `seq`, as one
allocation shared by reference between every connection's ring. The
connection's thread writes the frame in two pieces: the length, then `seq` as
the envelope's first field, from the number the writer thread assigned when
it pushed; then the shared tail. No connection copies the record and the
writer thread's push is a reference count.

The tail is built where the record is: `begin(topic)` writes the payload
field, the `Any` type URL and the `Any` value field into the encoder's buffer
ahead of the record, with padded length gaps filled at `commit` the way ADR
0012 fills a nested message's. A put appends to a body already in its place.
The epoch and the mission time, when a later task stamps them, are fields of
the same tail.

`commit` copies the tail once, out of the encoder's buffer into the shared
allocation, and that is the one allocation on the commit path. It replaces
the Lua string `commit` returned before the ring existed, which allocated the
same bytes on the Lua heap. A slab that hands the allocation back is not
scheduled; the allocation is on the logic thread and PROBE-7 at task 9.7 is
where its cost is read under a production rate.

The writer thread numbers the record as it pushes: each connection holds its
own counter, from one, taken before the ring decides whether the record
stays, so an evicted record leaves its gap.

A connection's thread sleeps on its ring the way the writer thread sleeps on
the commit ring, with the writer thread on the waking side: the flag and the
fence ADR 0011 put between the logic thread and the writer are one type used
twice. The writer's cost per push per connection is the fence and a load.

The commit ring's one producer is shared by the two Lua states behind a lock
that is never waited on. Both states run on DCS's main thread, so the lock
is uncontended by construction; `try_lock` takes it, and a `commit` that finds
it held is refused, counted, and returned as false, because a second thread
committing is a defect and not a case to serve.

The alternatives, and the line that rejected each:

- **Build the whole frame on the logic thread, with a gap for `seq`.** The
  writer thread then copies the frame per connection to fill the gap, which
  makes its cost per record proportional to the record's size times the
  connection count.
- **Build the frame on the connection's thread from the record's fields.**
  The connection's thread would then encode, which puts a protobuf writer on
  every connection thread and the record's fields in a form other than bytes
  in the ring.
- **A wrapper written at `commit` rather than at `begin`.** The record's
  bytes would have to move to make room for it, which is the copy ADR 0012
  removed.
- **A condition variable or a channel from the writer to a connection's
  thread.** Both take a lock on the writer thread per push, per connection,
  for a wake the flag delivers with a load.
- **One commit producer per Lua state.** Two producers on one ring is the
  second-producer algorithm SPEC 5.2 rejects, and two rings would need
  merging on the writer thread for an order the single thread already has.

## Consequences

One allocation per committed record on the logic thread, and one
deallocation wherever the last connection's ring lets go of it, which under
pressure is the writer thread and with no connection attached is also the
writer thread. The allocation is unmeasured until PROBE-7.

A frame is two writes on the connection's thread rather than one. With
`TCP_NODELAY` set the two may leave as two segments; a consumer reading by
length prefix does not notice.

`seq` is not in the ring. What the ring carries is the tail and the number
beside it, so a `LIFECYCLE` replay that rewrites only `seq` is a push with a
new number and the same tail.

The listener binds at the first module open, on the specification's default
address with the specification's default ring sizes, until the first
`shim.configure` exists to do it. An open that cannot bind raises. Task 2.15
moves the bind and the sizes there, and until it does, an install with the
port taken cannot load the module.

Contention on the commit lock is counted and not otherwise reported. A
non-zero count means something other than DCS's main thread is committing,
and the metric that surfaces it is a later task's.

This reopens if PROBE-7 reads the allocation as a measurable share of the
commit path, which would schedule the slab, or if a DCS build runs the two
Lua states on different threads, which would make the contended case one to
serve rather than refuse.
