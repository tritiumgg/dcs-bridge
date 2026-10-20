# ADR 0046: The bridge asks for every state the announcement names

## Status

Accepted

## Context

`net.dostring_in` is gated by two keys in `autoexec.cfg`. ED's two sources
agree on `net.allow_unsafe_api` and disagree on `net.allow_dostring_in`.
SPEC 5.4:

> **The two ED sources disagree, and the shipped one is the narrower.**
> `API\Sim_ControlAPI.md` gives one `allow_dostring_in` value, `"mission"`, under
> a comment saying it enables `net.dostring_in("scripting", "lua code")`. ED's
> forum announcement of the gate lists six state names, `"server"` among them.

The specification sets the bridge's own minimum at `"server"`, `"mission"`
and `"gui"`. It has not been measured. DCS 2.9.18.12722 enforced the gate and
a hotfix reverted it, so no current build can tell the two lists apart.

The policy-refusal handling, the paste-ready `autoexec.cfg` the hook driver
logs, `doctor`'s printed union and the installer's merge all write this list.
A list that misses a state an enforcing build needs breaks the bridge on the
user's machine. The fix the bridge offers would then be wrong as well.

## Decision

The bridge asks for all six states ED's announcement names in
`net.allow_dostring_in`: `"server"`, `"mission"`, `"scripting"`, `"gui"`,
`"export"` and `"config"`. In `net.allow_unsafe_api` it asks for
`"userhooks"`, as the specification states. ED's sources agree on that key,
and the hook driver is the only caller, so there is nothing to hedge.

The six are unioned with what the file already holds, so another tool's
entries stay.

Alternatives:

- **The specification's minimum of three.** Unmeasured, and a missing state
  under enforcement breaks the bridge with no fix in its own log.
- **The shipped reference's `"mission"` alone.** Shipping tools address
  `"server"`, and the GUI state is needed for magnetic declination.
- **Measure before Phase 4.** No current build enforces the gate, so a
  measurement now shows nothing.

## Consequences

A user grants three states the bridge does not call into: `"scripting"`,
`"export"` and `"config"`. ED labels the mechanism unsafe, and each extra
state widens what any hook script on the machine may reach. The bridge
documents the six and says three are there because ED's sources disagree.

The list is provisional. A plan task revisits it once the hook and sim
driver states are complete, on a build that enforces the gate where one
exists. It narrows the list to the states measured as needed, or records
why the six stay.
