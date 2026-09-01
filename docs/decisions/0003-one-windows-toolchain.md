# DR-0003: The MSVC target is the only Windows build

date: 2026-09-01
supersedes: none
superseded-by: none
diverges-from: none

## Question

Task 1.4 planned two Windows toolchains: `x86_64-pc-windows-msvc` through
`cargo-xwin`, with `x86_64-pc-windows-gnu` beside it as a documented fallback.
Does the broker keep the second one?

## Decision

No. `x86_64-pc-windows-msvc` through `cargo-xwin` is the only Windows build.
`rust-toolchain.toml` names that one target, `crates/broker/build.rs` refuses
any other Windows environment, and task 1.4 no longer carries a fallback.

## Why

The fallback existed to remove the Windows-machine requirement, and
`cargo-xwin` already removes it. CI's *Windows cross-build from Linux* job
produces `lua-dcsbridge.dll` and `dcsb.exe` on an Ubuntu runner and reads the
DLL's import table for `lua.dll` on every run, so the escape route the fallback
offered is the route already taken.

A mingw build is permitted and would work. SPEC §5.1.1:

> **`lua.dll` is an MSVC build importing `VCRUNTIME140.dll` and the UCRT.** A
> module built with a different toolchain therefore carries a different C
> runtime. Two runtimes in one process are harmless until memory crosses between
> them, so: **never free across the boundary, never pass a `FILE*`, never pass a
> CRT handle.** The Lua C API needs none of that — Lua copies every string it is
> given and allocates through its own allocator.

The broker stays inside those constraints, so this is a cost decision rather
than a correctness one.

The cost is what keeping it true takes. `build.rs` would emit a GNU-format
`liblua.a` beside the MSVC import library, `rust-toolchain.toml` would carry a
second target that every `mise install` downloads, and a mingw-w64 toolchain
would become a prerequisite for anyone building that path. None of that is
expensive. Verification is: without a CI job nothing exercises the fallback, and
with one it costs a runner on every pull request for an artifact that is never
shipped.

An unexercised fallback is documentation rather than insurance. The failure it
insures against — `cargo-xwin` unable to fetch the MSVC headers, or its licence
terms changing — is discovered on the same day the untried path is discovered
broken, which is the day the insurance was supposed to pay.

**This reopens if `cargo-xwin` stops being viable**, whether by breakage,
licence, or a DCS build that ships a Lua the MSVC target cannot bind. The
fallback then returns as a task of its own, with a CI job in its first commit.
