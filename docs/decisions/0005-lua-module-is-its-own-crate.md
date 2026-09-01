# ADR 0005: The Lua entry point is its own cdylib crate

## Status

Accepted

## Context

The broker was one crate built to two artifacts, `crate-type = ["cdylib",
"rlib"]`. The cdylib is what DCS loads. The rlib is what `cargo test` links
into a test executable.

Task 2.1 wants a host-native build of the broker that a stock Lua 5.1 opens, so
that SPEC 17's *Any (native module)* rows run with no DCS present. SPEC 17
states the reason it is possible:

> *Any (native module)* means the same, against a host-native build of the
> module loaded by a stock Lua 5.1 — the broker touches nothing DCS-specific,
> so its behaviour is checkable off-platform.

Such a module names `lua_gettop` and links no Lua library. The symbol comes
from the interpreter that opens it. A shared object may carry an undefined
symbol and resolve it at load; an executable may not, because its linker has to
resolve everything.

One crate cannot serve both once the Lua calls are in it. The cdylib needs the
symbol undefined and the test executable cannot tolerate that. A cargo feature
does not separate them, because features are per-crate and not per-crate-type,
so no `cfg` expresses "this item belongs to the cdylib alone".

ADR 0002 rejected splitting the crate as the "right shape, wrong phase", when
the broker held a stub and there was nothing to test. Phase 2 is where the
rings, framing and drop policy arrive, so the phase is now right.

## Decision

The broker splits in two, and the directory names say which is which.

`crates/broker` is package `dcsbridge-broker`, a plain rlib. Rings, threads,
framing, drop policy and configuration live there. It names no Lua symbol, has
no `build.rs` and no features, and links into a test binary on any host.

`crates/lua-module` is package `lua-dcsbridge`, `crate-type = ["cdylib"]` and
nothing else. It holds `luaopen_dcsbridge` and every declaration that names a
Lua symbol, and depends on `dcsbridge-broker`. Cargo never links a cdylib-only
crate into an executable, so the conflict cannot arise rather than being
avoided by care.

`[lib] name` stays `lua_dcsbridge`, so the artifact is still
`lua_dcsbridge.dll` and `tools/stage-release.sh` and `.github/workflows/`
need no change.

ADR 0002 stands. A feature still separates the two builds, and `dcs-lua` keeps
its name, its default and its job; it moves to the crate that has a Lua surface
to gate. What that record says about the feature gating the `extern` block in
`crates/broker/src/lib.rs` is overtaken by the split, and ADR 0006 states what
gates the binding now.

The alternatives, and the line that rejected each:

- **A third feature on one crate**, off during `cargo test` and on for a
  separate module build. It works, but the boundary is a `cfg` to be remembered
  for the rest of Phase 2, and the FFI is then never compiled by `cargo test`.
- **Leave the crate whole and pass the test binary an undefined-symbol linker
  flag.** ELF has no per-symbol form for an executable, and the blanket one
  produces a binary that crashes when the symbol is called.
- **Keep `crates/broker` as the cdylib and add a core crate beside it.** Same
  split, smaller diff, but the crate named for the broker would hold almost
  none of it.

## Consequences

Phase 2's internals land in `crates/broker` by default, and anything reaching
for a Lua symbol has to move to `crates/lua-module` or be handed a value by it.
That is the boundary this record exists to draw, and it is worth stating at
review time: a broker file with an `extern "C"` block in it is in the wrong
crate.

`crates/lua-module` has no rlib, so it cannot carry a `#[test]` and cannot be
depended on by another crate. Everything testable therefore has to sit in
`dcsbridge-broker`, which is the intended pressure. The FFI itself is checked
by opening the built module from Lua, which `tools/luatest.sh` does.

Two crates now build from what was one, and a workspace-wide `cargo build`
compiles the module on every host. Where the module has no Lua to bind — a
Windows host without `dcs-lua` — it compiles to an empty cdylib rather than
failing, which keeps one command working across CI's three legs.

This reopens if the module surface grows enough that it needs tests of its own.
The answer then is a third crate holding the FFI as an rlib behind safe
wrappers, not a return to one crate.
