# ADR 0006: The host-native module resolves Lua symbols at load

## Status

Accepted

## Context

SPEC 5.1.1 fixes how the product artifact finds its Lua:

> **The public API is stock Lua 5.1**, so stock headers are safe to compile
> against. Bind to it through an import library generated from a checked-in
> `.def` naming `lua.dll`, which needs no DCS install at build time and pins
> exactly which Lua symbols the broker depends on.

That covers the Windows DLL. It says nothing about the host-native build task
2.1 adds, because no such build existed when it was written. That one is opened
by whatever `lua` the machine has, and there is no `.def` for it.

The three hosts do not agree on what a shared object may leave unresolved. ELF
permits an undefined symbol in a shared object and looks it up at load. Mach-O
resolves one only where the link was told to expect it. Windows resolves
nothing at load at all: a DLL names its imports at link time or it has none.

## Decision

The host-native module leaves its Lua symbols undefined and takes them from the
interpreter that opens it. `crates/lua-module/build.rs` says how per host:

| Build | Link |
|---|---|
| Windows, `dcs-lua` on | The import library from `vendor/lua/lua.def`, unchanged. |
| Windows, `dcs-lua` off | Nothing, and the Lua surface is `cfg`'d out. |
| macOS | One `-Wl,-U,_<symbol>` per name read out of `vendor/lua/lua.def`. |
| Linux | Nothing. `-shared` permits it, and Lua's `linux` target exports its own symbols with `-Wl,-E`. |

macOS reads the same `.def` the import library is generated from, rather than
carrying a second list. `-U` is the per-symbol spelling of `-undefined
dynamic_lookup`; the blanket form is deprecated on current `ld64` and admits
every undefined symbol, including a misspelled one, which then fails at load
inside the interpreter instead of at link. Reading the `.def` keeps SPEC
5.1.1's pin doing the same work on both platforms: a symbol outside the 114 is
a link error.

**The Lua load test runs on Linux and macOS.** On Windows it would need a
fetched `lua51.dll`, a second `.def` naming it, and an interpreter to run,
all for a configuration that never ships. The Windows leg builds and tests the
broker, and `Windows cross-build from Linux` already links DCS's `lua.dll`
through the `.def` and greps the staged DLL for it.

The alternatives, and the line that rejected each:

- **`-undefined dynamic_lookup` on macOS.** Deprecated, and it turns a
  misspelled symbol from a link error into a crash inside DCS.
- **Link a host `liblua` instead.** The module would then carry a second Lua
  into a process that already has one.
- **Fetch a Lua 5.1 for the Windows runner.** A download and a second `.def` to
  keep current, for a build nobody ships.

## Consequences

`vendor/lua/lua.def` now feeds two link strategies rather than one, so its
header note to re-measure after a DCS update reaches macOS too. A symbol added
there for the Windows build is silently granted to the host-native build as
well, which is correct — both bind the same stock 5.1 API — but it means the
file is no longer only about DCS.

Nothing checks the Windows host-native configuration, because there is nothing
to check: the Lua surface is `cfg`'d out and the crate compiles to an empty
cdylib. A Windows contributor running `cargo test` sees the broker's tests and
not the module's load. This reopens if the module ever needs to be exercised
from a Windows host without DCS.

`package.loadlib` does not agree with itself about the opener's name. Lua 5.1
predates `dlopen` on macOS and ships a second implementation over the old dyld
API, which a build reaches unless it defines `LUA_USE_MACOSX`. That one passes
the name through untouched and so wants the Mach-O spelling,
`_luaopen_dcsbridge`; every `dlopen` build takes the bare name and adds the
underscore itself. This is a property of the interpreter, not of the module,
and DCS takes the bare name SPEC 5.1.1 writes. `tests/lua/load.lua` tries the
bare name first and reports which one answered, so the Linux leg exercises the
spelling DCS uses.

The Linux leg rests on the interpreter exporting its own symbols. Lua's `linux`
makefile target passes `-Wl,-E`, and a build that drops it would fail the load
with a missing symbol rather than silently passing. The fallback is the
distribution's own `lua5.1` package.
