# ADR 0054: The hook driver compares the interface before configure

## Status

Accepted

## Context

SPEC 13.3's version table:

> | `interface` | The Interface A call surface between hook driver payload and broker | Hook driver, at the first `configure` | Hook driver logs both, disables, leaves the mission alone (Section 5.1). |

`shim.configure` answers the module's interface version. The first
`configure` is also the call that allocates the rings and binds the listener.
A hook driver that compared the answer and then stopped would leave a broker
listening on the port behind a hook driver that never started. ADR 0051's
did-not-start path promises that nothing listens.

The module already puts the same version on the table `luaopen_dcsbridge`
returns, as `interface`.

## Decision

The hook driver holds the version it is written against, `local INTERFACE`
in `HookDriver.lua`, and compares it with `shim.interface` right after the
module opens, before `configure`.

- A mismatch goes through ADR 0051's path: an ERROR line in
  `DCSBridge.log` naming both versions, a raise, and the loader's
  did-not-start line in `dcs.log`. `configure` is never called, so nothing
  binds.
- The `configured:` line still prints the version `configure` answers.
- A Rust test in `crates/lua-module` reads `local INTERFACE = '...'` from
  `HookDriver.lua` by its one-line shape and compares it with
  `INTERFACE_VERSION`, the pattern ADR 0050 uses for the defaults. A change
  to either alone fails CI.

Alternatives:

- **Compare `configure`'s answer, as SPEC 13.3 words it.** The listener is
  up by then, and a disabled hook driver has no way to stop it.
- **Pass the hook driver's version into `configure` and let the broker
  refuse.** The broker would then own a decision SPEC 13.3 gives the hook
  driver, and the refusal message would need both versions anyway.

## Consequences

The comparison happens at the same call in the sense that matters, the first
contact with a module, and the bind never happens on a mismatch.

A hook driver and a module from one build always match, which the test
guarantees. The check matters at a live install where a user replaced one
file and not the other.
