# ADR 0002: A cargo feature separates the broker's two builds

## Status

Accepted

## Context

The broker builds twice from one source: a `cdylib` that links DCS's `lua.dll`
through `vendor/lua/lua.def`, and a host-native build for tests that must never
touch the `.def`. SPEC §5.1.1 describes the first:

> Bind to it through an import library generated from a checked-in `.def`
> naming `lua.dll`, which needs no DCS install at build time and pins exactly
> which Lua symbols the broker depends on.

The specification says nothing about a host-native build. PLAN §4 introduces
it: *The broker also builds host-native, for tests only.*

Nothing in the build already separates the two. A build script is told the
target triple, not the crate type, and on a Windows host both builds carry the
same triple.

## Decision

The `dcs-lua` feature, on by default, gates the import library in
`crates/broker/build.rs` and the `extern` block in `crates/broker/src/lib.rs`
together, so neither exists without the other. The host-native path passes
`--no-default-features`, which CI's three-host matrix and every `mise` task do;
the Windows cross-build and the release workflow take the default.

Rejected:

- **The target triple.** `x86_64-pc-windows-msvc` is both the product target
  and a Windows contributor's host target, so a triple test links `lua.dll`
  into every test binary on that host. A machine with no DCS has none to load,
  and the test executable fails before `main`.
- **`cfg(test)`.** It reaches unit tests only. Integration tests and benchmarks
  link the crate as an ordinary dependency, and the planned test harness is
  integration tests, so the seam would break in the phase after the one that
  built it.
- **Splitting the crate**, logic in an `rlib` under a thin `cdylib` shim. This
  is the structure a larger broker wants and it needs no feature at all, but
  `crates/broker/Cargo.toml` already states one source and two crate types, and
  rearranging that belongs to the phase that builds the broker proper.
- **Delay-loading `lua.dll`.** One binary would serve both, at the price of
  turning a link error into a crash at the first Lua call, on the path that
  matters most, for a seam the feature already gives.

## Consequences

- The default points at the safer failure. A forgotten flag fails at `cargo
  test` on a Windows host, in front of the person who can fix it. Off by
  default, a forgotten flag would ship a DLL that binds no Lua, links clean,
  collects clean, and fails inside DCS in front of a user.
- CI reads the built DLL's import table for `lua.dll` on every run, which
  catches a Lua-less DLL whichever way the default points.
- Every host-native invocation must carry `--no-default-features`, so a new
  `mise` task or CI job that omits it breaks on Windows alone.
- The feature and the `extern` block move together. Splitting them reintroduces
  the failure this ADR removes.
- The crate split reopens if the `cdylib` shim grows past a handful of exports.
- Once the host-native build has a stock Lua 5.1 to link against, the feature
  chooses which Lua rather than whether.
