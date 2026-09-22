# ADR 0029: The retained set is a slot store of references under one frozen cap

## Status

Accepted

## Context

The specification keeps the latest record of every `LIFECYCLE` topic and
replays the set to each connection that authenticates. SPEC 5.2:

> **LIFECYCLE retention.** The broker retains the most recent record of each
> `LIFECYCLE` topic: the payload bytes **and the envelope's `epoch` and
> `mission_time` as stamped, including their absence**, keyed by the type URL it
> already reads, holding their relative emit order.

It sizes the set as fixed buffers, allocated once. SPEC 13.1 on
`max_lifecycle_record_bytes`:

> The size of one retention slot, and the largest `LIFECYCLE` record the broker
> will retain. Slots are fixed and allocated at the first `configure`, so this
> figure times `max_lifecycle_topics` is standing memory — 1 MiB at both
> defaults.

and bounds the count by the same key that sizes the store. SPEC 5.2:

> **The retained set is bounded by `max_lifecycle_topics`.** [...] A
> `shim.classes` call that would take the registered `LIFECYCLE` count above
> the cap is refused whole, like any other refused registration.

Three things in the build do not fit that shape.

The record the broker holds is one `Arc<[u8]>` shared by every ring it is
fanned out to (ADR 0014). A slot that copied it into a fixed buffer would add
a copy per `LIFECYCLE` record and hold a megabyte of buffers that the
bridge's own thirteen topics fill to tens of bytes.

The writer thread, which owns the store and does the replay, reads no
envelope and holds no registry (ADR 0027). It cannot key a slot by the type
URL, because it never sees one.

The driver may register before it configures. The registration tests say so
on purpose: the hook driver registers at DCS start, right after its first
`configure`, and the order between the two is not the broker's to insist on.
So the first `LIFECYCLE` registration may bind slots before the file's
`max_lifecycle_topics` has been read, and the key is restart-tier, so the
number the registry bound under and the number the store was sized at could
be two numbers.

## Decision

**A slot holds a reference to the committed record, and the size bound is a
refusal at `commit`.** The store is `max_lifecycle_topics` entries, each an
`Option` of the record's `Arc<[u8]>` with the capability number the record
needs, plus a list of the occupied slots in emit order. A newer record of a
topic replaces the older in its slot and moves the slot to the back of the
order. Standing memory is the slot headers; the record bytes are the ones
the rings already share. What bounds them is `Bridge::commit`, which refuses
a `LIFECYCLE` tail over `max_lifecycle_record_bytes` before the copy and
counts `lifecycle_oversize_total`.

**The slot number is bound at `shim.classes` and rides with the record.**
The registry binds each fresh `LIFECYCLE` topic to the next slot as its class
arrives, and `begin` looks the slot up beside the capability and the ring.
The three travel the commit ring together, so the writer thread indexes the
store by a number and never by a name.

**The cap is one number, frozen by whichever comes first of the first
`LIFECYCLE` registration and the first `configure`.** A registration that
comes first freezes the specification's default. The first `configure` then
puts the frozen value in force, and if the file's value differs it is
reported pending a restart, as any restart-tier key that differs is. The
store is sized from the same number when the writer thread starts. A
registration of another class freezes nothing, so a file read after it
applies as written.

**The replay bytes are the committed bytes.** `seq` is written by the
connection's thread when it frames, in a header of its own ahead of the
shared tail, so a replayed record keeps the `epoch` and `mission_time` it
was stamped with, or their absence, with nothing rewritten.

**The replay follows the handshake and the `AuthResult`.** Those two are the
broker's own answers and are numbered ahead of it; the consumer reads that
it is authenticated before the first retained record. Every live record and
every later answer queues behind the replay.

Alternatives:

- **Fixed buffers of `max_lifecycle_record_bytes`, as specified.** A copy per
  `LIFECYCLE` record and a megabyte standing for a few hundred bytes.
- **Key the store by type URL.** The writer thread would have to parse the
  envelope it is told not to read.
- **Read the cap from the configuration at each use.** The registry and the
  store read it at different moments, and a driver that registers first
  binds slot 63 into a 16-entry store.
- **Refuse a `LIFECYCLE` registration before the first `configure`.** It
  changes what `shim.classes` accepts for a rule the registration tests
  reject on purpose.

## Consequences

`max_lifecycle_record_bytes` bounds nothing the writer thread holds; it is
enforced at commit, on the logic thread, and a record over it is refused
there rather than truncated or dropped later. The count is one number until
`stats` reports it by topic.

A file that lowers `max_lifecycle_topics` under a driver that registers
first sees the key pending a restart at every later `configure`, and the
larger default stays in force for the process. A file that raises it is
applied only if `configure` comes first. The hook driver configures before
it registers, so the built driver never hits either case.

Slots are never given back. Retiring a `LIFECYCLE` topic is a DCS restart,
as retiring any registration is.

The replay is pushed into a fresh connection's `LIFECYCLE` ring in one pass,
before its drainer has popped any of it, and a full `LIFECYCLE` ring closes
the connection as any push into a full one does (ADR 0028). So `configure`
refuses a `ring_out_lifecycle_records` at or below `max_lifecycle_topics`,
as ADR 0019 has it refuse every pair whose basis says one bounds the other;
a file that lowered the ring alone would close every consumer at its
authentication. The defaults, 64 into 256, leave room for the boundaries a
mission emits after the replay. The ring size stays provisional until
PROBE-7 at task 9.7, and this pair reopens with it.

**The commit ring can evict a `LIFECYCLE` record before the writer thread
keeps it.** The ring has one class and evicts its oldest record under a
burst (ADR 0011), and the writer thread keeps a record only once it has
popped it, so a boundary committed just ahead of a burst larger than the
ring, on a host that holds the writer thread off for that long, is lost to
the retained set as well as to every live consumer. The logic thread is the
one that finds out, and it counts the loss in `lifecycle_evicted_total`; a
late consumer is then replayed the slot's previous record until the next
boundary. Re-queuing the record would put an allocation or a retry on the
commit path, and a second commit ring per class is the sizing question
PROBE-7 prices, so the loss is counted here and not prevented. Nothing
reports the count until `stats`.

`lifecycle_replayed_total` is one number for the writer thread until `stats`
reports it per connection.
