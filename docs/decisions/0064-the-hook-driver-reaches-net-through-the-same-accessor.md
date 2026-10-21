# ADR 0064: The hook driver reaches net through the same accessor

## Status

Accepted

## Context

ADR 0060 puts every `DCS.*` call in `HookDriver.lua` behind `dcs.call`,
which looks the namespace up at each call, runs the function under `pcall`
and never raises. It says nothing about `net`.

Injection is the second namespace the hook driver calls. SPEC 5.4.1:

> **Route A — bootstrap injection.** The hook driver resolves the sim driver's
> absolute path with `lfs.writedir`, then injects a fixed-size chunk that loads
> it

through `net.dostring_in`. The call has the same hazards a `DCS.*` call
has: the namespace can be absent, an operator can remove `net` by hand, and
a raise escaping into a DCS callback is contained only by the callback's own
guard.

## Decision

`dcs.call` and `dcs.dostring` are two wrappers over one local function,
`reach(namespace, name, ...)`, which holds what ADR 0060 gave `dcs.call`.

- `dcs.call(name, ...)` reaches the `DCS` namespace, with the same reasons
  as before: `no DCS namespace`, `DCS.<name> is not a function`,
  `DCS.<name> raised: ...`.
- `dcs.dostring(state, chunk)` reaches `net.dostring_in` and returns
  `true` and the state's answer, or `false` and a reason in the same three
  shapes.
- Each namespace name is held once, in a string constant.
- The source test that refuses a code line writing `DCS.` or `DCS[` also
  refuses `net.` and `net[` at a word boundary.

Alternatives:

- **A second accessor table, `driver.net`.** A name `net` on a wrapper
  table would trip the source scan, and the scan is what proves no call goes
  around the accessor.
- **Call `net.dostring_in` directly under a local `pcall`.** The second
  namespace would have its own lookup and its own reasons, and a rename or a
  removal would read differently on each path.

## Consequences

Both namespaces are looked up at each call, so an operator who removes `net`
gets a logged reason and not a raise.

A comment can write `net.dostring_in` only on a comment line. A trailing
comment on a code line that names it fails the source test.
