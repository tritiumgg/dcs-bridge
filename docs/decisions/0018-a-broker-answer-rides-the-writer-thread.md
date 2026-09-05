# ADR 0018: A broker answer rides the writer thread

## Status

Accepted

## Context

The broker answers some messages itself, on the thread that reads a
connection's socket, and every connection begins with a frame the broker
sends unasked. SPEC 5.2:

> **Handshake and order of operations.** The connection proceeds in a fixed
> order: handshake, then authentication, then everything else.
>
> The handshake frame carries the protocol version, the broker version, the
> instance id, and the schema hash, which is the SHA-256 of the compiled
> `FileDescriptorSet` the hook driver handed the broker at start (Section
> 5.1). **That is everything an unauthenticated peer learns.**

and, of the five request and reply pairs:

> **Five request/reply pairs answered by the broker.** These never reach Lua
> and carry no record class.

> **A broker answer is treated as `DURABLE` by the drop rule.**

Every frame a connection receives carries a `seq`, and SPEC 5.2 makes that
number total per connection. The writer thread assigns it as it pushes a
record into the connection's ring, before the ring decides whether the
record stays, so that an eviction leaves a gap (ADR 0014). The ring has one
producer, the writer thread, and one consumer, the connection's own thread,
and its correctness argument rests on there being one of each (ADR 0008).

An answer decided on the reader thread therefore has no straight path to the
socket. Pushing it into the connection's ring makes a second producer.
Writing it to the socket directly makes a second writer, and leaves it with
no number to carry. Committing it through the commit ring takes the producer
the logic thread holds behind a `try_lock`, which ADR 0014 refuses and
counts as a defect when a second thread reaches for it. Sharing the `seq`
counter between two producers through an atomic gives each a number, and
then delivers them out of order whenever the two push in the other order.

The handshake has the same problem one step earlier: it has to be frame one,
and the writer thread is the only thread that can make anything frame one.

The specification says what the handshake carries and not how. It names the
five pairs and gives the shape of none of the first three. It says the
consumer reads a protocol version and an instance id and defines neither.

## Decision

An answer goes to the writer thread, and the writer thread pushes it into
the connection's ring numbered like any other record.

The channel exists already: the listener attaches and detaches connections
through it, from threads that are not the logic thread, and a channel's
lock costs nothing off that thread. `Connections::answer` sends a record
addressed to one connection on the same channel. The writer takes control
messages before records on every pass, pushes the answer into the one ring
with that connection's next `seq`, and wakes the connection's thread. A
connection that has gone by then is a record with nowhere to go, dropped
and counted as unaddressed like a late acknowledgement.

The handshake rides inside the message that attaches the connection. The
writer thread pushes it as it attaches the ring, so it is numbered 1 and
nothing fanned out to the new ring can be. Sent as a second message after
the attach it could not be: the writer takes every control message it finds
and then drains the commit ring, so a record committed between the two
messages would reach the new ring first. The handshake is asked for at each
accept, so a field that arrives after the listener is up, the schema hash,
is in the next connection's handshake.

An answer lives in the same ring as the fanned-out records, so the drop rule
treats it exactly as it treats a `DURABLE` record, which is what SPEC 5.2
asks and nothing more has to be written for it.

`Pong` answers during a mission-load blackout because nothing on the path
waits for the logic thread: the reader thread decides, the channel carries,
and the writer thread, parked on an empty commit ring, is woken by the
channel's send.

The shapes the specification leaves open:

- **The handshake is an `Envelope` like every other frame**, at `seq` 1,
  whose payload is `dcs.bridge.Handshake`: `protocol` as one `uint32`,
  `broker` as the crate version string, `instance_id` as one `uint64`, and
  `schema_sha256` as optional bytes, absent until the hook driver hands the
  schema over. A consumer with no schema reads it the way it reads any
  frame. The mission-start and mission-time fields SPEC 5.2 marks optional
  are added when the epoch exists to fill them.
- **`instance_id` is a random 64-bit number taken once per process** from
  the keys `std` seeds its hasher with. Its one job is to differ between two
  starts of the broker, so a consumer that reconnects can tell a restart,
  whose `seq` origin and retained set are new, from the broker it was
  talking to.
- **`protocol` is a constant in the broker crate**, `PROTOCOL_VERSION`,
  starting at 1, moved by one when the frame header, the handshake or the
  broker-answered set changes in a way a consumer must know about.

Alternatives:

- **A second producer on the connection's ring.** ADR 0008's ring is built
  for one, and its Loom model checks one.
- **The reader thread writes the socket.** Two threads interleave bytes on
  one stream, and the answer has no `seq`, or takes one from a counter the
  writer thread also advances, which reorders.
- **The reader thread commits through the commit ring.** The producer is
  the logic thread's, behind a `try_lock` that refuses and counts a second
  thread rather than waiting on it.
- **Assign `seq` on the connection's thread as it writes.** An evicted
  record then leaves no gap, which is the one signal a consumer has that
  records were dropped.

## Consequences

An answer costs the reader thread a channel send and the writer thread one
pass, and reaches the socket one hop later than a direct write would. The
answers are bounded by the inbound rate limits, so the channel's allocation
per send is bounded the same way.

The writer thread is on the path of every broker answer. A writer thread
that has stopped stops the `Pong`, so a broker whose writer has died reads
as a dead sim rather than as a live bridge that cannot send. Nothing stops
the writer thread short of a fault in it, and a fault there already stops
every record.

The handshake is known to the broker by name, as the acknowledgement is
(ADR 0017), and a test holds the name to the schema.

`PROTOCOL_VERSION` is the protocol version's home in the code, beside
`BROKER_VERSION`. `version-bump.yml` rewrites the release version in
`Cargo.toml` and the README and nothing else, so a release leaves it where
it is.
