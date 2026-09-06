# ADR 0020: The schema hash comes from `sha2` on its portable backend

## Status

Accepted

## Context

The handshake carries a hash of the schema the broker serves, and the
broker computes it. SPEC 5.2:

> The handshake frame carries the protocol version, the broker version, the
> instance id, and the schema hash, which is the SHA-256 of the compiled
> `FileDescriptorSet` the hook driver handed the broker at start (Section
> 5.1).

The broker runs inside DCS's process, and ADR 0016 keeps its shipped build
on `std` alone, with one bend: the reader thread decodes an attacker's
bytes through `prost`, because a decoder written beside the encoder shares
its misreadings and `prost` is fuzzed. ADR 0016 gives a second reason for
writing the rings and the park flag by hand: Loom and Miri reach the
concurrency they can see, and a crate's atomics are outside the model.

Neither reason reaches a hash. The bytes come from the hook driver's own
deployment, read from `Mods\services\DCSBridge\schema.pb`, not from a
socket. A hash has no concurrency. So ADR 0016 neither permits a crate for
it nor asks for it to be hand-written, and the plan leaves the choice to
this record.

The `sha2` crate compiles two SHA-256 paths on x86_64, a portable one and
one over the SHA-NI intrinsics, and picks between them at first use by
running CPUID through `cpufeatures`. The intrinsics path is `unsafe` SIMD
code selected at run time.

## Decision

The broker hashes the schema with `sha2`, pinned to its portable backend:

```toml
sha2 = { version = "0.10", default-features = false, features = ["force-soft"] }
```

`force-soft` compiles the portable safe-Rust backend alone, so nothing
detects the CPU and no intrinsics run inside the sim for a hash computed
once per process. `default-features = false` drops `digest`'s `std`
feature, which a slice hash never needs. The pin lives on the dependency
line and nowhere else.

The alternatives, and what rejected each:

- **Hand-written, about eighty lines.** A wrong hash is silent: a consumer
  compares it to the file and gets a mismatch nothing explains. The
  crate's portable backend is the reference every consumer agrees with,
  and the two reasons ADR 0016 gives for hand-writing do not apply here.
- **`sha2` 0.11.** Its backend is pinned with a `--cfg` in rustflags, which
  is workspace-wide, and the loom task sets `RUSTFLAGS` itself and would
  bypass it.
- **The default backend.** Run-time selection of intrinsics, for one call.

## Consequences

`sha2` is the second crate inside DCS's process. It brings `digest`,
`crypto-common`, `block-buffer`, `generic-array`, `typenum` and `cfg-if`;
`cpufeatures` resolves but compiles to nothing under `force-soft`, and its
`libc` dependency reaches only the aarch64 hosts the tests run on, never
the Windows target.

The `Cargo.toml` comment that read "prost and nothing else" names both
exceptions.

The choice reopens if the 0.10 line stops receiving fixes, which is when
0.11's `--cfg` pin gets weighed against a `.cargo/config.toml` the loom
task honors.
