# ADR 0043: A decoder returns a table keyed by field name

## Status

Accepted

## Context

A command arrives at a driver as a topic id and an opaque body, and neither
Lua state has a protobuf runtime. SPEC 8.3 item 4:

> **Decoders**, one per `COMMAND` message. `shim.poll` returns a topic id and
> an opaque body. Something must turn that body into named fields. Neither Lua
> state has a protobuf runtime. The generator emits a field-number switch per
> message, placed by target like the emitters. The decoder skips unknown field
> numbers by length, which is what stops a consumer one release ahead from
> breaking the sim driver.

and SPEC 5.2 says what a wire type it cannot skip does:

> A decoder that meets one of those raises rather than skipping, whatever
> field number carries it.

Neither says how a decoder is called, what it returns, what a field the
sender left out reads as, or how a 64-bit integer lands in Lua 5.1, whose
only number is a double. A consumer's stock library writes a repeated
scalar packed, and a field type may appear twice in one body, which the
specification does not cover either.

## Decision

Each command gets `decode['<topic>'](bytes)`, which returns a table keyed by
field name. The table sits beside `emit`, `send` and `send_to` on the same
owner: `DCSBridge.code` in the sim files and the hook file's own table in
the hook files. Both shipped files always carry `decode = {}`, so an
adopter's sim slice adds its decoders by key (ADR 0041).

- **The switch is a table.** Each message the file decodes has a table
  keyed by field number, giving each field's name, kind and label. One
  reader in the file walks it (ADR 0044). A generated `if` chain per
  message would repeat the same reads in every branch.
- **A field the sender left out** reads as its zero value when it is a
  plain scalar: `0`, `false` or `''`. An `optional` scalar, an arm of a
  `oneof` and a message field read as `nil`, since a sender sends each of
  them set to zero. A repeated field and a map read as an empty table.
  That is what protobuf libraries return, so a consumer and a handler read
  one body the same way, and a handler never meets `nil` in arithmetic.
- **Integers** read exactly below 2^53. `int32`, `enum`, `sint32` and
  `sfixed32` are signed 32-bit values. `uint32` and `fixed32` are unsigned.
  The varint reader keeps the two 32-bit halves apart, so a negative
  `int64` arriving as ten bytes reads exact.
- **Floats and doubles** read as Lua numbers, including the infinities,
  NaN and negative zero.
- **Strings and bytes** read as Lua strings.
- **A repeated scalar** reads packed or one element per tag, and both may
  mix in one body. A map reads as a table from each entry's key to its
  value.
- **A field seen twice** keeps the last value for a scalar and merges into
  the first table for a message, as protobuf does.
- **A wire type the kind does not use** on a known field number is skipped
  like an unknown field.
- **A decoder raises** on wire types 3, 4, 6 and 7, field number zero, a
  body that ends inside a field, and either bound passed (ADR 0045).

Alternatives:

- **A generated `if` chain per message.** Larger files, and each kind's
  read repeated in every message that has one.
- **Absent as `nil` everywhere,** as the emitters treat it. A handler would
  test every scalar for `nil`, and a sent zero reads the same as `nil`
  anyway, because proto3 does not send a zero.
- **Every varint read as signed 64.** A `uint64` at 2^63 or above would read
  negative.

## Consequences

An `int64`, `uint64`, `fixed64`, `sfixed64` or `sint64` above 2^53 reads
rounded. A handler that needs an exact id above that has no way to get one.

A proto2 field's declared default is not read; it reads as its kind's zero.
The shipped schema is proto3.

A body that sets two arms of one `oneof` reads with both set, where a
protobuf library keeps the last. A handler reads which arm is set by which
is not `nil`.

A group is not decoded. Its start tag has wire type 3, so a command whose
sender sets a group field always raises.

Nothing catches a raise yet. The dispatch that calls a decoder catches it
and drops the connection.
