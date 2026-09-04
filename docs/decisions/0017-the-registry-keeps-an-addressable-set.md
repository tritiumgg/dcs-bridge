# ADR 0017: The registry keeps an addressable set

## Status

Accepted

## Context

A record opened with `begin_to` goes to one connection, and the broker
refuses the call on a topic that is not a reply or an acknowledgement.
SPEC 5.1:

> **The broker refuses a `begin_to` on a topic the schema did not mark as a
> reply or an acknowledgement**, and counts it in `misaddressed_total`. The
> generator only emits `send_to` for those messages (Section 8.3), so a
> refused call means hand-written Lua. Refusing is worth the check: a record
> that silently reaches one consumer instead of all of them presents as
> missing data at every other consumer, which is a miserable fault to trace.

The broker has no way to know what the schema marked. It holds no schema:
SPEC 8.2 has it read three registered tables, class, route and capability,
and SPEC 5.1 says of the schema bytes it is handed that "it parses none of
them". None of the three tables says whether a topic is a reply. The mark the
schema carries is `reply_to`, an option on the request that names its reply,
and no table carries it across. The acknowledgement, `CommandAck`, is marked
by nothing at all; it is point-to-point because SPEC 8.5.3 says so.

Registration itself is task 2.16, eight rows after `begin_to`. At 2.8 there
is no registered table of any kind, and the done-when needs a `begin_to`
that reaches one of two `dcsb tail` sessions.

Record class does not carry the distinction. `CommandAck` and every typed
reply are `DURABLE`, the same class as a fan-out event; class is drop policy,
and addressing is delivery.

## Decision

The registry holds a set of addressable topics beside its three maps, and
`begin_to` is refused on a topic outside it.

The bridge's own acknowledgement, `dcs.bridge.CommandAck`, is in the set
from the start, by name, with no registration. It is the bridge's message in
the bridge's package, so the broker owns its name the way it owns the enums
it mirrors from the same file, and a test holds the name to the schema.

A typed reply enters the set through registration. The generator reads
every `reply_to` in the schema and writes the set of replies as a fourth
table beside the class, route and capability tables; the registrar hands it
to the broker with them, additive and idempotent under the same rules. Task
2.16 builds the call and the merge; task 3.1 emits the table. Until then the
set holds the acknowledgement alone, and that is what 2.8 ships and checks.

Alternatives:

- **Derive the set from the schema bytes `shim.schema` hands over.** The
  broker parses none of them, and a parser for descriptors inside DCS is the
  highest-risk code the product could add. ADR 0016.
- **Mark a reply in the classes table.** Class and addressing are orthogonal,
  and a value that meant both would make `DURABLE` mean two things.
- **Refuse every `begin_to` until 2.16.** Nothing addressed could be observed
  on a live install for eight tasks, and the acknowledgement needs no
  registration to be known.
- **Trust the caller.** The specification's own argument against it: a
  record reaching one consumer instead of all presents as missing data at
  every other.

## Consequences

The registration surface grows by one table. SPEC 5.1 names three `shim`
calls for three tables, and the fourth needs a call or an argument of its
own; 2.16 chooses, and the interface version moves with it if the choice is
a call.

The acknowledgement's name is fixed in the broker. Renaming the message in
the schema fails a test rather than a consumer.

An adopter's own acknowledgement record has no mark. The specification says
a handler may add outcomes to `CommandAckOutcome`, and says nothing about a
second acknowledgement message. One would be refused at `begin_to` until
something marks it, and that is the case that reopens this record.

`misaddressed_total` counts the refusal and nothing reports it until
`shim.stats` exists.
