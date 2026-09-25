# ADR 0030: The topic name rides with the record

## Status

Accepted

## Context

The specification narrows what a connection is sent by topic, at fan-out,
SPEC 5.2:

> **`SetTopicFilter` narrows what a connection is sent.** Capability decides
> what a connection *may* see (Section 14.4). The topic filter decides what it
> *wants*. The two are different questions and the broker asks both at fan-out.

and keeps a name the consumer files before its topic is registered, SPEC 5.2:

> **An unadmissible topic id is kept, not refused.** Registration is additive
> across a DCS process (Section 5.1), so a consumer may name a topic an
> adopter's file registers at the next mission reload. The broker keeps it in
> the set and admits it once it becomes admissible, having already reported it
> in `unknown` so a typo is visible rather than silent.

A filter is a set of topic names. Fan-out runs on the writer thread, which
reads no envelope and holds no registry: it is generic over the record it
carries and parses nothing, which is the rule ADR 0027 and ADR 0029 rest on.
What it knows of a record is the header the commit ring carries beside the
bytes, and that header names a capability number and a retained slot, not a
topic. To ask a filter, the writer thread has to learn the record's topic
from the header.

## Decision

The record's header carries its topic name, as one `Arc<str>` per registered
topic. The registry makes the allocation as the topic's class arrives and
hands it out with the capability and the slot at `begin`, so `commit` costs
a reference count and not a copy; an answer and an addressed record carry
none, since no filter reaches them. Each connection holds its filter as a
set of names, and fan-out asks the set with the name the record carries,
after the capability mask and before `seq`.

The rule that the writer thread reads no envelope is about parsing, and a
name in the header is not the envelope. The rule that it holds no registry
is kept whole: a name is admitted by the record that carries it, so a
filter naming a topic before its registration needs no table on the writer
thread and nothing to reconcile when the registration lands.

Alternatives:

- **A registry-assigned number in the header and a bitset per connection.**
  Cheaper per record, but the writer thread then has to learn which name is
  which number: a registration message on the control channel, a snapshot at
  listen, an ordering between the two, and a name filed before its number
  exists held aside until the batch that numbers it. The rule against a
  registry on the writer thread is what that machinery re-creates.
- **The name as a `String` in the header.** A copy per record on the logic
  thread, where the shared allocation costs an increment.
- **Look the topic up on the writer thread.** A registry read lock per
  record per pass, under Loom, where the registry is not built.

## Consequences

The header grows by a fat pointer, and a commit on a named topic takes one
reference-count increment and its pop one decrement. Under `ONLY` a record
costs a short-string hash per narrowed connection; under `ALL`, the default,
it costs an enum match. Both are below anything PROBE-7 could measure and
sit beside the per-record allocation ADR 0014 already leaves to it.

`Addressed<T>` is no longer `Copy`, which nothing needed: the production
record is already an `Arc<[u8]>`.

The acknowledgement's name is allocated per `begin_to`, because it is
addressed and never matched; one allocation on a path that already opened
an encoder.

The retained replay applies no topic filter. Every retained record is
`LIFECYCLE`-class and always admitted, and replay runs at authentication,
before any `SetTopicFilter` can arrive. A later task that retains a record
of another class has to ask the filter there.

`records_filtered_total` is one process-wide count for both causes, and a
second counter keeps the topic cause apart; `stats` reports them when it
lands, as ADR 0027 already leaves the first.
