# ADR 0040: An emitter reaches the shim, and a wrapper hands it to the runtime

## Status

Accepted

## Context

The generator writes an emitter and a send wrapper for each outbound
record. SPEC 8.3 shows the emitter reaching the shim through a global:

> ```lua
> function DCSBridge.code.emit.UnitDestroyed(unit_name, coalition)
>   DCSBridge.shim.begin(500)
> ```

and says of the wrapper:

> A wrapper buffers the record; the Section 6.4 stage 1 drain calls the
> emitter.

ADR 0038 forbids the hook driver's file a global, so its emitters cannot
reach `DCSBridge.shim`. The buffer the wrapper fills belongs to the runtime,
which bounds it and drains it; no runtime exists yet. Several questions the
specification leaves open come up as soon as an emitter is written:

- **Which records are outbound.** A `COMMAND` record arrives from a
  consumer and is decoded, not emitted.
- **Which records are replies.** The schema marks the request with
  `reply_to`, not the reply.
- **Which side a reply's emitter goes to.** A request on one side may name
  a reply targeted at the other.
- **What a nil argument means.** `shim.string(1, nil)` raises.
- **Which field kinds the shim can write.** It has `integer`, `double`,
  `string`, `boolean`, `message` and `end_message`. `integer` writes a plain
  varint, so it cannot write `sint32` or `sint64` (zigzag), `fixed32`,
  `fixed64`, `sfixed32`, `sfixed64` or `float`.
- **How the acknowledgement is generated.** `CommandAck` is in
  `dcsbridge.broker`, which ADR 0037 skips, and SPEC 6.10.2 expects
  `send_to.CommandAck`.

## Decision

An emitter gets the shim from where its file can reach it, and a send wrapper
hands its emitter and arguments to a `buffer` function the runtime puts on
the file's table.

- **The sim file's emitters read `DCSBridge.shim` when called.** The hook
  file is a chunk that takes the shim as its argument,
  `loadfile(path)(shim)`, and its emitters close over it.
- **A wrapper calls `code.buffer(code.emit.<Name>, ...)`** in the sim file
  and `gen.buffer(gen.emit.<Name>, ...)` in the hook file. The generated
  file defines no `buffer`; the driver assigns it after loading the file,
  and the wrapper looks it up when called.
- **Every record except a `COMMAND` record is outbound** and gets an
  emitter and a wrapper.
- **A record is a reply when another record's `reply_to` names it.** A
  reply's emitter takes a connection id first and opens with `begin_to`, and
  its wrapper is `send_to.<Name>`. It has no `send` member: it answers one
  connection's request, and fanning it out would hand it to every
  subscriber.
- **An emitter goes to its own record's side.** A reply goes where its own
  `target` says, not where its request goes. A schema whose request runs on
  the hook side targets the reply at the hook driver too.
- **A nil argument writes no put.** That holds for every field, `optional`
  or not: an absent proto3 field and a zero one decode alike, except where
  `optional` makes them differ.
- **Arguments are the fields in declaration order.** A repeated field takes
  one Lua array and writes one put per element, in array order. A message
  field takes a table keyed by field name and writes one `message` and
  `end_message` pair around its own fields; a repeated one writes one pair
  per element.
- **A field kind the shim has no put for fails generation**, naming the
  message and the field: `sint32`, `sint64`, `float`, `fixed32`, `fixed64`,
  `sfixed32`, `sfixed64`, a group and a map. The rule covers every message a
  record's fields reach. `bytes` goes through `string`, an enum and every
  other integer through `integer`, and `bool` through `boolean`.
- **The acknowledgement is generated into both shipped files.** It is
  `dcsbridge.broker.CommandAck`, the one record of the broker's package that
  a handler sends. It gets an emitter and a `send_to` member on each side,
  because a command handler on either side answers with it. It gets no
  table rows: the broker knows it by name.
- **LuaLS annotations name each field type by its full name**, such as
  `---@class fixture.adopter.Point`. Two packages may each declare a
  `Point`, and LuaLS merges classes of one name.
- **A parameter that would hide a name the emitter reads is renamed** by
  appending `_` until it hides none: a Lua keyword; a name the file itself
  uses, `DCSBridge`, `ipairs`, `shim`, `put`, `code` or `gen`; `conn_id` in
  a reply; or a name an earlier parameter took. Table keys stay the field
  names verbatim.

Alternatives:

- **The hook file reads the shim from a field the driver sets.** A field
  must be set before the first emit and nothing checks that; a chunk
  argument is in place before any emitter can run.
- **The file defines its own queue.** The runtime bounds the queue and
  drains it on its own schedule, so a queue in the generated file would be
  a second one.
- **A reply's emitter follows its request's side.** A reply named by
  requests on both sides would need two emitters, and the record's own
  `target` would mean nothing for it.
- **A nil argument raises.** `optional` fields need absence, and a handler
  answering with half a record would have to spell every zero.
- **`CommandAck` is written by hand in the runtime.** Its fields are in the
  schema, and a hand copy would drift from them.

## Consequences

The driver loading a generated file sets `buffer` on its table before any
handler runs, and the hook driver passes the shim when it loads its file.
Until the runtime exists, the Lua tests supply a `buffer` that calls the
emitter at once.

A field kind outside the six puts is refused at build time, not encoded
wrongly at run time. The refused kinds can be added if the shim gains a put
for them.

A nil for a non-`optional` field reads the same as its zero value, so a
handler that forgets an argument sends a zero rather than failing. LuaLS
marks such a parameter as required, which is the only check.

The acknowledgement is the one exception to ADR 0037's skip of
`dcsbridge.broker`. A second broker record that handlers send would need
the same exception, named.
