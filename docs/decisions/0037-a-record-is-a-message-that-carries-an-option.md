# ADR 0037: A record is a top-level message that carries an option

## Status

Accepted

## Context

The generator refuses a message that names no capability, and the
specification scopes that rule to the messages that reach Lua, SPEC 8.1:

> Section 8.2's rule that the generator refuses a message with no capability
> is scoped to messages that reach Lua; the list in this paragraph is the
> whole set it does not reach.

The descriptor says nothing about which messages reach Lua. A plugin
request carries three kinds of message that never do. The imports are in
`proto_file`: `google/protobuf/descriptor.proto`, which `broker.proto` needs
for its `extend` block, and `google/protobuf/any.proto`. The broker's own
package is in it too, and buf names it in `file_to_generate`: `Envelope`,
the handshake and the broker's answers, with `SetEnabled` carrying a
capability and no class and `CommandAck` a class and no capability. And a
record's fields can be messages, which the specification names as types no
list of records covers, SPEC 8.4:

> `MissionDate`, `Projection` and `Verification` under
> `CoordinateCalibration` (Section 6.3) are definitions no enumeration of
> records covers

Such a message is a field type. It is encoded inside its record and never
travels alone.

## Decision

A record is a top-level message, in a file named in `file_to_generate`,
outside the package `dcsbridge.broker`, that carries any of the four
options. Every record names a `record_class` and a `required_capability`,
and generation fails naming the message and the missing option when one is
absent or zero. So a message that names only a `target` or only a
`reply_to` fails too.

- **Imports are never read.** A file in `proto_file` that is not in
  `file_to_generate` was not asked for.
- **`dcsbridge.broker` is skipped whole.** The broker frames, answers or
  knows by name every message in it (ADR 0021), and holds `CommandAck`'s
  class and capability itself (ADR 0017), so a generated table naming them
  would say twice what the broker already holds.
- **A message with no option is a field type** and has no row.
- **A nested message is never a record**, whatever it carries. A topic id
  is a fully-qualified name, and a nested name would be a topic nothing
  registers.

Two records with the same name in two packages fail generation, because the
topics table is keyed by name (ADR 0021). A `reply_to` names a record, by
its name in the requesting message's package or by its fully-qualified
name; anything else fails generation.

Alternatives:

- **Every top-level message is a record.** Every field type would need a
  capability it does not use, and the specification's own field types would
  fail.
- **A message referenced by no field is a record.** A record can be a field
  of another record, and a field type can be declared before any record uses
  it, so reference says nothing about travel.

## Consequences

A message meant as a record that carries none of the four options is a
field type, silently, and generates nothing. The broker then refuses its
topic as unknown at registration (ADR 0027), so the miss surfaces at a live
install rather than at build time. Nothing else in the descriptor tells the
two apart. A check that refuses a message referenced by no field and
carrying no option would catch the common case, and is the place to start if
this bites.

An adopter's package is read by the same rule, so a field type in it needs
no options and a record in it needs two.
