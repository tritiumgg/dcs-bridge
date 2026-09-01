# DR-0002: A cargo feature separates the broker's two builds

date: 2026-09-01
supersedes: none
superseded-by: none
diverges-from: none

## Question

The broker builds twice from one source: a `cdylib` that links DCS's `lua.dll`
through `vendor/lua/lua.def`, and a host-native build for tests that must never
touch the `.def`. What separates them?

A build script is told the target triple, not the crate type, and on a Windows
host both builds carry the same triple.

## Decision

The `dcs-lua` feature, on by default. It gates the import library in
`crates/broker/build.rs` and the `extern` block in `crates/broker/src/lib.rs`
together, so neither exists without the other.

The host-native path passes `--no-default-features`. CI's three-host matrix and
every `mise` task do that; the Windows cross-build and the release workflow take
the default.

## Why

Four ways to draw the line, and the triple is the obvious one that does not
work. `x86_64-pc-windows-msvc` is both the product target and a Windows
contributor's host target, so a triple test links `lua.dll` into every test
binary on that host, and a machine with no DCS has none to load. The test
executable then fails before `main`.

`cfg(test)` reaches unit tests only. Integration tests and benchmarks link the
crate as an ordinary dependency, and task 2.1's harness is integration tests, so
the seam would break in the phase after the one that built it.

Splitting the crate — logic in an `rlib`, a thin `cdylib` shim over it — is the
structure a larger broker wants, and it needs no feature at all. It is rejected
here rather than refuted: `crates/broker/Cargo.toml` already states one source
and two crate types, and rearranging that is a Phase 2 question. This reopens if
the shim grows past a handful of exports.

Delay-loading `lua.dll` would let one binary serve both. It turns a link error
into a crash at the first Lua call, on the path that matters most, and buys a
seam the feature already gives.

**On by default, because the two failure modes are not symmetric.** Off by
default, a forgotten flag ships a DLL that binds no Lua, links clean, collects
clean, and fails inside DCS in front of a user. On by default, a forgotten flag
fails at `cargo test` on a Windows host, in front of the person who can fix it.
CI reads the built DLL's import table for `lua.dll` on every run, which closes
the first hole whichever way the default points.

SPEC §5.1.1 says what the `cdylib` does and this follows it: *Bind to it through
an import library generated from a checked-in `.def` naming `lua.dll`, which
needs no DCS install at build time and pins exactly which Lua symbols the broker
depends on.* The specification says nothing about a host-native build. PLAN §4
introduces it: *The broker also builds host-native, for tests only.*

Task 2.1 gives that build a stock Lua 5.1 to link against, and the feature then
chooses which Lua rather than whether.
