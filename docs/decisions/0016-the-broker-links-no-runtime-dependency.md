# ADR 0016: The broker links no runtime dependency

## Status

Accepted

## Context

`crates/broker` and `crates/lua-module` compile into `lua_dcsbridge.dll`, and
DCS loads it into its own process. Every crate the broker depends on is code
running inside the simulator, on the thread that runs the mission, with no
process boundary between a fault in it and the game. SPEC 14.2 states the
trade for the parser, and the statement holds for every line the artifact
carries:

> The parser runs inside the DCS process, before authentication completes. A
> fault there is a fault in the game server. **There is no process boundary
> between an attacker's bytes and the sim.** That is the accepted trade for
> having no second process.

The specifications say nothing about dependencies. The ledger has no row on
the subject, so the position this record states is the project's own.

The broker's rings, its park-and-wake protocol, its protobuf encoder and its
framed transport are written by hand, and each of them has a well-tested
crate that looks like a replacement. The question of whether to take one
comes up at every task that touches them, and the answer has lived only in a
`Cargo.toml` comment.

Two tasks ahead need a protobuf decoder: 2.9 puts five inbound messages on
the reader thread, and 3.1 builds the protoc plugin, which reads a
`CodeGeneratorRequest` from protoc. SPEC 14.2 calls the parser the
highest-risk code in the product.

## Decision

The broker's shipped build links no crate outside `std`. Dependencies that
reach only tests, Loom models or the CLI and generator binaries are free,
because none of them runs inside DCS.

The rule bends in one place. The reader thread decodes inbound messages
through `prost`, from task 2.9. A decoder is where an attacker's bytes are
read, a decoder written beside the encoder shares its misreadings, and
`prost` is fuzzed, bounds its recursion, and rejects over-long varints and
truncated fields. The frame-length cap before any allocation stays the
broker's own, ahead of the library. The encoder stays hand-written: `prost`
encodes an owned message, and the put-call path (ADR 0013) has no such
message to hand it.

The generator reads the plugin request and writes its response through
`prost-types`, which carries both messages. It hand-parses no descriptor.

Where the rule holds, the hand-written piece and the crate it was weighed
against:

- **The ring, against `crossbeam-queue`, `rtrb`, `ringbuf` and `thingbuf`.**
  Evicting the oldest record while a consumer reads is not a single-producer
  structure: the producer touches the consumer's slot, which is what the
  stamps are for. `rtrb` and `thingbuf` refuse when full. `ringbuf` overwrites
  only through the whole ring, with both ends in one hand. `ArrayQueue` is the
  same design priced for many producers, a compare-exchange on every push
  where this ring's fast path is a load, a write and a store, and it has no
  held-by-consumer state and so no `Refused`. ADR 0008.
- **The park flag, against `crossbeam-utils`'s `Parker`, `event-listener`
  and `atomic-wait`.** `std::thread::park` already carries the token. The
  flag is what lets the pushing side skip the system call when the sleeper is
  awake, and no crate's parker says whether its sleeper is parked. ADR 0011.
- **The Loom shim, against nothing.** Every Loom user writes it.
- **Varints and tags, against `prost::encoding`.** The library covers thirty
  lines. The state machine, the padded length gaps and the allocation-free
  put path are the module. ADR 0012, ADR 0013.
- **`write_all_vectored`, against `std`'s.** The method in `std` is
  unstable.
- **A thread per connection, against `mio` and `tokio`.** A stalled socket
  stalls its own thread and nothing else. ADR 0011.
- **The Lua declarations, against `mlua` and `mlua-sys`.** `mlua` wraps every
  callback in unwind guards and protected-call plumbing, on the path
  ADR 0013 priced at 26 ns per call. `mlua-sys` carries a `build.rs` whose link
  decisions fight the `lua.def` import library and the macOS undefined-symbol
  list. ADR 0006.

What Loom and Miri establish is a second reason. They reach the hand-written
concurrency because it names the shim's atomics; a crate's atomics are
outside the model, and a test on three hosts cannot establish what the model
does. ADR 0008.

## Consequences

The concurrency code carries its own correctness argument, beside each
ordering, and the argument is the review. A contributor reads the ring rather
than trusting a crate's reputation, and a change to it reruns `mise run loom`
and the Miri job.

`prost` and `bytes` become the first crates inside DCS's process, at 2.9. The
reader thread already runs under `panic = "unwind"` with a catch at its
boundary, so a decoder fault drops one connection, and that is the rule
SPEC 14.2 sets for a hand-written decoder too.

`crates/cli` and `crates/generator` take what they need. Neither runs inside
DCS.

A measurement reopens the ring's half of this: PROBE-7 at task 9.7 prices the
rings, and a probe that finds `ArrayQueue`'s push within noise of this ring's
is a reason to weigh the exchange again.
