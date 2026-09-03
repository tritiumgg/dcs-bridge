# ADR 0012: A nested message's length is written in place, padded

## Status

Accepted

## Context

A length-delimited protobuf field carries its length before its body, and the
body of a nested message is not known until the message closes. SPEC 5.1 says
how the broker is to handle that:

> **Nested and repeated message fields.** `message` opens a submessage on a
> field number and `end_message` closes it. [...] The broker writes the
> submessage body to a scratch buffer and emits the tag and length when the
> pair closes, which is the same one copy per record the length-prefixing rule
> below already describes.
>
> **Length prefixing.** A length-delimited protobuf field needs its length
> before its tag. Write the body to a scratch buffer, then emit the tag, the
> varint length, and the body. That is one copy per record.

Submessages nest, so one scratch buffer is not enough as written: a message
closing inside another has to land in its parent's scratch, not in the record,
which means a scratch per level or one scratch whose contents move. Either way
a body is copied once per level it is nested in, and the copy is a memmove
sized by the body on the logic thread's path.

The plan noticed the alternative and set it aside. PLAN 3.1, under PROBE-3:

> Demoted to a Phase 1 optimisation: whether target decoders accept a
> non-minimal length varint. The scratch-buffer copy is cheap and the
> compatibility risk is not worth carrying as an open question. Task 2.5's
> done-when covers it.

The alternative is to write the body where it will end up and fill the length
in afterwards. The length is then written into a gap reserved before the body
was known, so the gap has to be as wide as the largest length possible, and a
short length written into a wide gap is a varint with more bytes than it
needs. Protobuf's wire format permits that: a varint is read until a byte
without the continuation bit, and leading zero groups change nothing. Whether
every decoder honors it was the open question, and task 2.5's done-when — a
stock library decodes the output, including a non-minimal length varint — is
where it gets answered.

## Decision

`message` reserves a fixed-width gap for the length and `end_message` writes
the length into it, padded to the gap's width. There is no scratch buffer and
no copy: a record is one buffer, allocated once, and every put appends to it.

The gap is as wide as the varint of the buffer's capacity, because no body can
be longer than that. At the product buffer of 1 MiB that is three bytes, so a
nested message costs two bytes on the wire more than a minimal encoding would,
and nothing more than that at any depth.

The open messages are a stack of gap offsets, allocated to `MAX_DEPTH` at
construction and never grown. A record opening a message past that depth is
refused and discarded, like one that outgrows its buffer.

The test decodes the padded length through `prost` and reads the length
bytes back through the library's own varint reader.

Alternatives:

- **A scratch buffer per nesting level.** A copy per level, and a buffer per
  level to size and allocate up front.
- **One scratch buffer, with `memmove` at each close.** Still a copy per level,
  and the specification's "one copy per record" holds only for a record one
  level deep.
- **Write the body in place, then move it back over a minimal length.** No
  scratch buffer, but the move is the same copy, now sized by the body and
  done for every nested message regardless of its length.

## Consequences

Every consumer decoder has to accept a non-minimal varint. The protobuf wire
format permits one, the reference implementations read one, and the test
proves one library. A decoder that refuses would be non-conforming, and the
answer would be that decoder's rather than a return to copying.

Two extra bytes per nested message at the product buffer size. A record with
many small repeated messages pays that per element; a `LIFECYCLE` record
retained in a slot pays it against `max_lifecycle_record_bytes`.

The gap width is a property of the buffer's capacity, so a record's bytes
depend on the capacity it was encoded under. Two brokers configured with
different `max_frame_bytes` produce different bytes for the same record.
Nothing compares record bytes across brokers, and nothing should start to.

The depth bound is a constant in the broker and not a generated one. The
specification has the generator derive depth bounds from the schema for the
decoders it emits, and a schema deeper than `MAX_DEPTH` would need the
constant raised. No schema in this project comes near it.

This reopens if a consumer library that matters refuses a padded varint, or
if PROBE-3 at task 2.6 finds the put path's cost lies somewhere the copy would
not have mattered. Neither is expected.
