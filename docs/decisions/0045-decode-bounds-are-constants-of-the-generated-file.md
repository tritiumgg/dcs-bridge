# ADR 0045: Decode bounds are constants of the generated file

## Status

Accepted

## Context

The broker forwards a command's body without reading it, so it bounds
neither nesting depth nor element count. SPEC 14.2:

> Depth and repetition bounds are generator constants:
> `protoc-gen-dcsbridge-lua` derives them from the schema's actual shape plus
> margin and bakes them into the generated decoders, so the limit is provably
> sufficient for the schema it ships with and no configuration can set it below
> what the schema needs.

The schema's shape fixes how deep its messages nest, unless a type contains
itself. It says nothing about how many elements a repeated field holds. The
specification names no margin and no figure.

A decoder enters only the message fields it knows, and skips an unknown one
by its length without reading inside it. So in a schema with no recursive
type, no body can nest deeper than the schema does. The depth bound does
work only for a recursive type, where any bound refuses some valid bodies.

A frame is at most `max_frame_bytes`, 1 MiB by default, and an element can
be one byte. One frame could therefore hold about a million elements and
build a table that size inside the sim.

## Decision

Each generated file that decodes carries two local constants.

- **`MAX_DEPTH`** is the longest chain of field types below any command in
  that file, plus 8. A type met again along one chain counts once and ends
  the chain. The command's own body is depth zero, and a map entry counts as
  a level.
- **`MAX_ELEMENTS`** is 4096, the most elements one repeated field or map
  may hold in one message. Packed chunks, single tags and map entries all
  count toward the same field's total.

A decoder raises when a body passes either one.

The generator reads neither figure from its parameter, and a generated file
takes no argument that sets one. Changing either is a change to the
generator.

Alternatives:

- **A fixed depth of 64,** the broker's `MAX_DEPTH` for its own encoder.
  Not derived from the schema, and it says nothing a schema-derived figure
  does not.
- **Refusing a command that reaches a recursive type.** Depth would be exact,
  but the depth bound would then never fire, and a tree-shaped command could
  not exist.
- **A bound per run instead of per file.** A file decodes only its own
  commands, so its own are what it needs.

## Consequences

A recursive type nests at most 8 levels beyond its first. A sender with a
deeper tree gets a raise.

A command with more than 4096 elements in one field gets a raise. A schema
that needs more needs a new figure and a new record.

The shipped schema has no command, so the shipped files carry neither
constant until one is added.
