# ADR 0024: The inbound rings share one producer behind a lock

## Status

Accepted

## Context

SPEC 5.2 gives the inbound path two rings, one per target, and names the
thread that writes them:

> **Routing.** The broker reads the payload's type URL from every inbound
> frame, so it knows the topic before Lua sees it. [...] The registered route
> map (Section 5.1) sends each inbound record to the sim driver ring or the
> hook driver ring. [...] The reader thread writes both rings; the sim driver
> polls one and the hook driver polls the other, so each ring keeps one
> producer and one reader

and states the rule every ring is built under:

> **Single producer, single consumer.** Every ring, inbound and outbound, has
> one atomic write index and one atomic read index — which is why a broker
> answer is handed to the writer thread rather than pushed by its producer.
> Use no mutex. A lock would let the logic thread block behind a slow socket.

The specification has one reader thread. The build has one per connection:
ADR 0018 gave each connection a thread that reads its socket, because a
read blocks the way a write does and neither may wait on the other, and
that is what lets a `Ping` be answered while the sim loads a mission. So
the inbound ring's one producer is reached from as many threads as there
are connections, and the ring of ADR 0008 is built for one, with a Loom
model that checks one.

The rule's reason is the logic thread. A lock is refused because the thread
running the simulation must never wait on a socket. The consumer of an
inbound ring is the logic thread, at `poll`; the producers are reader
threads, which wait on sockets for a living.

## Decision

Each inbound ring has one producer, held behind a mutex that only reader
threads take, for the length of one push. The consumer end is lock-free
and the logic thread's, so the logic thread never touches the lock and
never waits on a reader thread.

The push is [`Producer::offer`]: a record goes into a free slot or comes
back, with no eviction and no compare-exchange, so what a reader thread
does under the lock is one stamp load, one move and one stamp store. A
reader thread that finds the lock held waits for another reader's push,
which is microseconds, and holds nothing else while it waits.

Alternatives:

- **A ring per connection per target.** Two rings per connection keep the
  one-producer rule to the letter, but `ring_in_sim_driver_records` and
  `ring_in_hook_driver_records` size one ring per target, and `poll` would
  scan every connection's ring for one record, on the logic thread, in
  proportion to `max_connections`.
- **A thread that funnels.** Reader threads send to one thread that owns
  both producers. A channel is a lock and an allocation per record, one more
  thread in the host, and one more hop between a command and Lua.
- **A multi-producer ring.** A second algorithm beside ADR 0008's, with its
  own model to check, for a path whose contention is bounded by the inbound
  rate limits.

## Consequences

The lock is on the reader threads' path and nowhere else. `Bridge::deliver`
takes the registry read lock to find the route, releases it, then takes the
producer lock to push; it never holds both, and the logic thread holds the
consumer lock only for the length of one pop under `try_lock`, the way the
commit ring's producer is taken (ADR 0014).

A reader thread can wait on another reader thread. What one holds while
waiting is the frame it has already read and nothing of the socket's, so a
slow socket stalls its own reader and no other.

The Loom model checks one producer and one consumer, which is what each
lock-holding reader is for the length of its push; the lock is the standard
library's and is not modelled.

This reopens if a DCS build runs the two Lua states on different threads,
which would make the consumer end contended too, or if PROBE-7 at task 9.7
reads the lock as a measurable share of the inbound path.
