# ADR 0011: A connection's thread drains its own ring, and the writer parks

## Status

Accepted

## Context

SPEC 5.2 fixes the shape of the outbound path and leaves one end of it open:

> **The logic thread writes one ring, not N.** `commit()` appends to a single
> producer ring. The writer thread reads that ring and fans each record into a
> per-connection queue. Fan-out on the logic thread would make `max_connections`
> multiply logic-thread cost per record. A configuration change would then become
> a performance change.
>
> Each connection then has its own queue. A consumer that stops reading must not
> cost another consumer its records. Count drops per connection. Disconnect a
> consumer whose queue stays full.

It names the producer of every ring. The commit ring's producer is the logic
thread and its consumer is the writer thread. A connection's ring has the writer
thread as its producer:

> **Only the writer thread pushes to an outbound ring.** [...] The writer thread
> then does all three things that must agree with each other: it assigns `seq`,
> it applies the drop rule, and it pushes.

It never says who pops a connection's ring and writes the bytes to the socket.
Two readings fit. The writer thread does it too, so a connection's ring never
crosses a thread and is a buffer the writer fills and empties in turn. Or each
connection has a thread of its own, and the ring is what carries records from
the writer thread to it.

The specification also gives the rings no way to wait. A ring is two indices
and a slot array; a consumer finding it empty learns nothing about when that
changes. The writer thread cannot spin, because it shares the machine with the
simulation, and it cannot sleep on a timer without adding that timer to every
record's latency. The logic thread has to wake it, and the only rule about that
side is the one that governs everything on it:

> Use no mutex. A lock would let the logic thread block behind a slow socket.

Two smaller things fall out of the same paragraph and have no owner. ADR 0008
hands an evicted record back to whoever pushed it, and said that what the logic
thread does with one is decided here. And SPEC 13.1 sizes `ring_out_records`
per connection and the two inbound rings, but names no size for the commit ring
the paragraph above describes.

## Decision

Each connection has its own thread, and that thread is the consumer of the
connection's ring. The writer thread is the producer. A connection's ring is
therefore crossed by two threads, which is what ADR 0008's ring is built for,
and a socket that stops taking bytes stalls its own thread on a blocking write
while the writer thread pushes past it: the ring fills, evicts, and counts,
and no other connection notices. With `max_connections` at 8, that is at most
eight threads, each doing nothing but blocking on one socket.

ADR 0009's "the writer thread drains the three by merging on `seq`" is read
under this record as the connection's thread doing that merge. The merge is one
comparison per record and it moves with the pop.

The writer thread parks when the commit ring is empty, and the logic thread
wakes it through a flag rather than a call. The writer raises `parked`, then
reads the ring's depth, and parks only if it is still empty. The logic thread,
after each push, reads `parked` and calls `unpark` only if it is raised. A
`SeqCst` fence sits between the store and the load on each side, so under one
total order either the pusher sees the flag or the writer sees the record, and
a wake that arrives before the park makes the park return at once. The cost on
the logic thread is one fence and one load per record, and one system call per
transition from idle to busy rather than per record. The attach side, which
does not run on the logic thread, wakes unconditionally.

The fences are there for Loom as much as for the hardware. Loom treats a
`SeqCst` load or store as acquire or release, so a flag handshake written with
`SeqCst` accesses alone fails its model with a lost wake the memory model
forbids. A `SeqCst` fence it models in full, and with the fences in place the
model passes, and fails again when one is removed. That is what makes the Loom
run over this module a check rather than a formality.

An evicted commit-ring record comes back to the logic thread and is dropped
there. Nothing else is in a position to drop it without a second crossing, and
the record type does not yet exist, so the cost of the drop is unknown. Task
2.4's benchmark measures the full-ring case so the figure is on record.

The commit ring takes its capacity as a parameter and has no configuration key.
Task 2.15, where rings size from configuration, decides whether it gets one or
follows `ring_out_records`.

The alternatives, and the line that rejected each:

- **The writer thread drains every socket itself.** Needs non-blocking sockets
  and a readiness loop so one slow socket does not hold up the rest, and turns
  the per-connection ring into a single-threaded buffer that pays for atomics
  it never needs.
- **The writer thread polls the commit ring on a timer.** Adds the timer's
  period to every record's latency and burns a wake per period on an idle
  server, to save the logic thread one load.
- **A condition variable between the logic thread and the writer.** The notify
  side takes the variable's mutex, which is the lock SPEC 5.2 forbids on the
  logic thread.
- **`SeqCst` accesses with no fence.** Correct under the memory model and
  cheaper by one barrier, but Loom cannot check it, and an unchecked wake
  protocol on the logic thread is the wrong place to save a barrier before a
  probe has priced one.

## Consequences

One thread per connection, on top of the reader and the writer. Eight is the
ceiling and the threads block rather than spin, so the cost is stack memory
and nothing else while a socket is keeping up.

The fence is on the logic thread's path for every record. On x86-64 a `SeqCst`
store is already a full barrier, so the fence is a second one per push. PROBE-3
at task 2.6 and PROBE-7 at 9.7 are where that gets priced, and either may
license removing it in favor of the accesses alone, with the Loom model then
resting on the fence-free argument written beside the code rather than on a
passing run.

A record committed while no connection is attached is dropped on the writer
thread and counted nowhere. Once the retention set exists at task 2.17,
`LIFECYCLE` records have somewhere to go regardless, and that is the only class
whose loss with no one listening means anything.

Connection ids are numbered per writer, from one. Task 2.8 owns the rule that
they are unique for the process and never reused; with one writer per process
this counter already satisfies it, and 2.8 is where that becomes a stated
guarantee rather than an accident of there being one.

Disconnecting a consumer whose queue stays full is not done here. The ring
counts its drops per connection, which is the input that rule needs, and task
2.18 is where the rule lands.

This reopens if a socket write cannot be made to block per connection. A
transport that needs a single event loop would put the writer thread back in
charge of draining, and the ring back to being crossed by one thread.
