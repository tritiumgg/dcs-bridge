# ADR 0028: The drainer merges a connection's rings, and the writer thread closes a full one

## Status

Accepted

## Context

ADR 0009 gives each connection three outbound rings and says who merges them:

> The writer thread drains the three by merging on `seq`, lowest first.

The writer thread does not drain. Under ADR 0011 it pushes into a
connection's ring, and the connection's own thread pops the ring and writes
the socket. So the merge has two threads in it, and ADR 0009's argument for
the order, that `seq` is assigned before the push, covers the pushing side
only. A drainer that finds the `LOSSY` ring empty and then pops `seq` 10 from
the `DURABLE` ring may have looked at the `LOSSY` ring a moment before the
writer thread pushed `seq` 9 into it.

The specification ends the drop rule with a disconnect, SPEC 5.2:

> **A ring that is full of `LIFECYCLE` is a disconnect, not a drop.**

and ADR 0009 places it: "a push into a full one drops the connection and
counts `lifecycle_disconnects_total`". Neither says how. The push is the
writer thread's. The socket is the listener's, the fan-out module holds no
socket and is built under Loom where none exists, and the thread that could
be told, the drainer, is blocked in a write to the consumer that stopped
reading. A flag does not reach a thread inside a system call.

Three kinds of record have no class to route by. SPEC 5.2:

> **A broker answer is treated as `DURABLE` by the drop rule.** `Pong`, `Schema`,
> `Topics`, `TopicFilterResult`, `AuthResult` and `Rejected` carry no record class
> (Section 8.1) [...] **Count both paths under the label `broker_answer`**:
> an eviction, and a refusal at the `ring_out_lifecycle_reserve` watermark.

The watermark is the path ADR 0009 removed, and a ring hands back an evicted
record with nothing that says it was an answer. The handshake is not in that
list and is the broker's own record too. `CommandAck` is a registered topic
with no class table entry of its own. And ADR 0009 says "`COMMAND` is inbound
and does not appear here", yet `begin` accepts a topic registered under the
`COMMAND` class and commits it outbound; a hook script may do that today.

ADR 0009 retires `ring_out_lifecycle_reserve` and names three keys, and
expected task 2.15 to land them. 2.15 shipped `ring_out_records` and the
reserve, with the invariant ADR 0019 lists between them, and sized the commit
ring at `ring_out_records`, which ADR 0011 had left open. The specification
gives the commit ring no key.

## Decision

**The connection's thread merges, and looks again before it trusts the lowest
head.** The drainer holds at most one popped record per ring. To choose the
next record it pops every ring it holds nothing from, and repeats that pass
until a pass pops nothing; then it writes the held record with the lowest
`seq`. The writer thread pushes a connection's records in `seq` order, so
every record numbered below the chosen one was pushed before it, and
therefore before the last, fruitless pass began. That pass found its ring
empty only if the record had been evicted, which is a gap and not a reorder.
A pass that pops something fills one of three places, so choosing a record
costs at most four passes. The rings share one waker, and the drainer has
work when any ring or any held place is occupied.

**The writer thread closes a connection through a closure it was handed at
attach.** The listener builds the closure over a handle to the socket, and
it calls `shutdown` in both directions. When the `LIFECYCLE` ring refuses a
record, the writer thread counts `lifecycle_disconnects_total`, calls the
closure, and forgets the connection once the fan-out pass is over. On
macOS and Linux the shutdown returns the blocked write with an error, the
drainer returns, and the detach and the reader's exit follow as they do for
any socket that fails. On Windows it does not: see Consequences. The refused record is not counted as dropped: the connection it was
for is gone. A connection attached with no closure, which only a test does,
is forgotten and counted the same way.

**What has no class takes the `DURABLE` ring**: every broker answer, the
handshake, `CommandAck`, and a record on a `COMMAND`-class topic committed
outbound. The class is looked up once at `begin`, beside the capability, and
rides the commit ring with it, for ADR 0027's reasons.

**Drops are counted by the ring they happened in, and a broker answer carries
a mark.** A record a ring evicts is of that ring's class, so the push site
knows which count to move. The one thing it cannot know is whether a record
evicted from the `DURABLE` ring was a broker answer, so a numbered record
carries one flag that says so, and `broker_answer` counts those evictions
and the refusal of an answer pushed while the oldest record was being read.

**`ring_out_lossy_records`, `ring_out_durable_records` and
`ring_out_lifecycle_records` replace `ring_out_records` and
`ring_out_lifecycle_reserve`**, at ADR 0009's provisional 3584, 512 and 256.
The reserve's pair leaves the list ADR 0019 checks. The commit ring is sized
at the sum of the three, 4352: it feeds all three rings of every connection,
and a burst one connection could hold should not be lost before fan-out.

Alternatives:

- **Merge on the writer thread into one ring per connection.** It is the
  single ring ADR 0009 rejected, with the search moved one step earlier.
- **Trust the lowest head after one pass.** It reorders under exactly the
  race in the Context, and `dcsb tail` would report a gap that is no loss.
- **A flag the drainer reads, in place of the closure.** The drainer of a
  stalled consumer is blocked in a write and reads nothing.
- **Refuse a `COMMAND`-class topic at `begin`.** The stricter reading of ADR
  0009, and it changes what `begin` accepts for no gain to the drop rule.
- **A class label on every numbered record.** Three of its four values
  repeat which ring the record is in, on the path ADR 0014 prices.

## Consequences

A configuration file naming `ring_out_records` or `ring_out_lifecycle_reserve`
gets the key named as unknown, as any unknown key is, and the default sizes
apply.

ADR 0011's "one ring per connection" reads as three from here on; the
one-producer-one-consumer rule it rests on holds for each.

The drop counts are read through the outbound path and nothing reports them
until `stats` exists, as with `records_filtered_total` in ADR 0027.

**On Windows the close does not free the connection's thread.** The Windows
CI job on this task's pull request observed it: a local `shutdown` does not
return a `send` that is already blocked, and the drainer stayed in its write
for the 30 seconds the test gave it. What the rule promises the consumer
still holds there. The writer thread has forgotten the connection, so nothing
more is queued for it; the shutdown does return the reader thread's `recv`,
as every test that drops a listener under open clients shows on that job, so
the session closes and its place is given up; and the consumer's stream
ends when it reads again. What stays, until the peer reads, exits or resets,
is one thread blocked in the kernel and the three rings it holds. A consumer
whose process is suspended for good is the case, and each such consumer
costs one thread, and a few megabytes of rings, until then. Nothing a
consumer sees differs from macOS and Linux, and no other consumer is
affected, so the cost is accepted and not fixed here.

Two cancels were tried on that job and neither returned the send:
`CancelIoEx` on the socket, which Windows documents for asynchronous I/O,
and `CancelSynchronousIo` on the draining thread's handle, which it
documents for this case. Whether the second was reached at all is not
known, because the test observes the drainer's join alone. What is left is
a send timeout, which on Windows makes the socket unusable and so is a
disconnect rule of its own, or a socket both threads poll; either is a
decision of its own and belongs with the broker-hardening work, which has
no task. The test of the close asserts the thread's return off Windows
only.

A consumer closed this way sees its stream end and nothing that says why.
It reconnects into a fresh `seq` and the retained set, which is what the
specification intends for a consumer that has missed boundaries.

The sizes stay provisional until PROBE-7 at task 9.7, the commit ring's with
them. The decision on the merge reopens if a connection's rings ever gain a
second producer: the order argument is that one thread pushes in `seq` order.
