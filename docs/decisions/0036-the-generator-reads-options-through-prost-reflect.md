# ADR 0036: The generator reads the options through prost-reflect

## Status

Accepted

## Context

ADR 0016 has the generator read the plugin request through `prost-types`:

> The generator reads the plugin request and writes its response through
> `prost-types`, which carries both messages. It hand-parses no descriptor.

Everything the generator emits comes from the four message options
`broker.proto` declares, `record_class`, `reply_to`, `target` and
`required_capability`, at extension numbers 50001 to 50004 on
`google.protobuf.MessageOptions`. `prost` keeps no unknown fields, and
`prost-types` declares no extension, so a `CodeGeneratorRequest` decoded
through `prost-types` alone arrives with every one of the four already
dropped. What `prost-types` leaves is enough for file names and
`file_to_generate`, and nothing the tables are made of.

## Decision

The generator decodes the request twice from the same bytes. `prost-types`
reads `file_to_generate` and the parameter. A one-field `prost` message
reads `proto_file`, field 15, as raw bytes, and each file's bytes go into a
`prost-reflect` `DescriptorPool` through `decode_file_descriptor_proto`,
which keeps extension options. A message's options then come back from
`MessageDescriptor::options()` with the four extensions resolved by name
against `broker.proto`, which is in the pool because every schema file
imports it.

`prost-reflect` is built on `prost` 0.14, the version the workspace already
links, and runs only on the build machine, which ADR 0016 leaves free.

Alternatives:

- **Hand-declared mirrors of `FileDescriptorProto`, `DescriptorProto` and
  `MessageOptions` with the four fields added.** Three message types copied
  from `descriptor.proto` to read four numbers, which is the hand-parsed
  descriptor ADR 0016 rules out.
- **`DescriptorPool::add_file_descriptor_protos` over the `prost-types`
  values.** The options are gone before the pool sees them.
- **Decoding the whole request dynamically through `prost-reflect`.** It
  needs `plugin.proto` in a pool first, which nothing ships; the one-field
  message is four lines.

## Consequences

The rule in ADR 0016 holds as written: no descriptor is parsed by hand, and
the request and response types are still `prost-types`'.

The generator links `prost-reflect` and the tests link `protox`, which
compiles the fixture schemas into the request protoc or buf would send.
Neither reaches the broker.

A schema that never imports `broker.proto` has no records. Nothing in it can
carry the options, so the generator emits empty tables rather than
refusing.
