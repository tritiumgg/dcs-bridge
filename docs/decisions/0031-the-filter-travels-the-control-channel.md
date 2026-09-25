# ADR 0031: The filter travels the control channel

## Status

Accepted

## Context

The specification publishes a connection's topic filter from the reader
thread to the writer thread without a lock, SPEC 5.2:

> **The filter is published by pointer swap.** `SetTopicFilter` arrives on the
> reader thread and fan-out runs on the writer thread (Section 5.2), so the
> reader builds the new set, publishes it with one atomic store, and the old
> set is reclaimed after the writer's next pass. No mutex, matching the ring
> discipline. **The filter applies from the first record fanned out after that
> store.** Records already in a connection's queue are delivered, because the
> queue holds encoded bytes and re-examining it would put the broker's own work
> on the critical path for no correctness gain.

The writer thread already takes what the reader thread tells it about a
connection through one channel it drains at the top of every pass: the
attach, the detach, the answers and the capability set (ADR 0027). An
atomic pointer per connection would be a second path to the same thread,
with its own reclamation rule for the set the swap retires.

## Decision

The filter is sent whole down the control channel, as the capability set
is, and the writer thread installs it on the connection when it drains the
message; the set it replaces is dropped there, on the thread that was
reading it. The channel is the store the specification describes: one
hand-off, no lock the writer thread waits on, and the old set reclaimed at
the writer's next pass. The filter applies from the first record fanned out
after the writer thread takes it, and a record already in the connection's
queue is delivered as numbered.

The filter is sent before the `TopicFilterResult` that reports it, on the
same channel, so every record numbered after the answer obeys it: a
consumer that reads the answer and then a record it did not ask for has
found a bug.

Alternatives:

- **An atomic pointer per connection.** A second path into the writer
  thread, a reclamation scheme for the retired set, and a type the Loom
  build has to model; the channel already exists and is drained first.
- **Apply the filter to the connection's queue.** The specification refuses
  it: the queue holds encoded bytes.

## Consequences

A filter is in force only after the writer thread drains its inbox, so a
test that counts what a narrowed connection receives waits for the
`TopicFilterResult` before committing the records it counts, as the
capability tests wait for a `Pong`.

A `SetTopicFilter` costs the channel one allocation per message, on the
reader thread; the writer thread's pass is unchanged.
