# ADR 0025: `dcsb send` takes encoded bytes until the schema holds a command

## Status

Accepted

## Context

The specification has `send` encode a record from the schema the bridge
serves, SPEC 15:

> Schema reflection makes `send` work for record types the CLI was never
> compiled against: it reads the `FileDescriptorSet` from `GetSchema`
> (Section 5.2) rather than a compiled-in copy.

Reflection turns a readable record, a JSON object or a text-format message,
into bytes by walking the descriptor of its type. It needs a library that
does that walk, `prost-reflect`, which no crate in the workspace depends on,
and it needs a message in the served schema to encode.

The schema holds one message, `UnitDestroyed`, and it is an event the sim
emits. No command exists for a reflection encoder to encode or for a test to
send. The commands arrive with the built-in sets, phases later.

The task `send` closes is the routing check: a record a consumer sends
reaches the ring its registered route names and no other. The broker reads
the payload's type URL and nothing inside the payload, so the check needs
the bytes on the wire and not their meaning.

## Decision

`dcsb send` takes the topic and the payload's encoded bytes, from a file or
as hex on the command line, and an empty payload when neither is given. It
does not fetch the schema and does not decode or check the payload.

Reflection encoding is the follow-up, added as a `--json` flag beside
`--file` and `--hex` once the schema holds a command. The three are one
group, so the flag adds without renaming and the verb's shape does not move.

Alternatives:

- **Reflection now.** Nothing in the schema to encode, a dependency added for
  a path no test can exercise, and a verb several times the size of the
  check it exists to make.
- **Bytes only, no follow-up named.** The specification's sentence stays
  unmet with nothing pointing at when it is met.

## Consequences

A payload `send` puts on a ring is whatever bytes it was given. The broker
passes it through and the Lua handler polling that ring is the first thing
that reads it, so a malformed payload fails there, by the handler's rules.

The topic is taken as typed. The route map holds the fully qualified message
name and nothing checks the name against the schema, so a misspelt topic is
counted unrouted and the sender learns nothing until the `Rejected` answer
exists.

The `--json` flag reopens this when a built-in set registers its first
command. It fetches the schema with the session it already has, so the
handshake and the token stay as they are.
