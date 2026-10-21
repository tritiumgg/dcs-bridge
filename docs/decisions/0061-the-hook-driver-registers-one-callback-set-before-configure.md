# ADR 0061: The hook driver registers one callback set before configure

## Status

Accepted

## Context

SPEC 6.9 on the hook callbacks:

> **The hook driver cannot reload itself.** `DCS.setUserCallbacks` has no
> deregistration counterpart, and a second call adds a callback set rather than
> replacing one

SPEC 6.10 gives the runtime "one `DCS.setUserCallbacks` set", and extension
files register through the runtime's surface rather than calling it.

The bodies arrive over several tasks: the dispatch loop (4.12), the epoch
boundaries (6.1), pause hints (6.6), the hook built-ins (10.6), and
extension files (4.11). Each needs a callback registered before it lands.

ADR 0054 and ADR 0056 keep every stop of the start before the first
`configure`, which binds the listener, except `shim.schema` raising.

## Decision

`driver.start` registers one table of sixteen callbacks through
`dcs.call('setUserCallbacks', ...)` (ADR 0060), after `schema.pb` is read and
before the first `configure`.

- The set is every callback a specification or a later plan row consumes:
  `onSimulationFrame`, `onMissionLoadBegin`, `onMissionLoadEnd`,
  `onPlayerConnect`, `onPlayerDisconnect` and `onPlayerChangeSlot` (the
  dispatch loop, 4.12); `onSimulationStop` (epochs, 6.1); `onSimulationPause`
  and `onSimulationResume` (pause hints, 6.6); `onPlayerStart`,
  `onPlayerStop`, `onPlayerChangeCoalition`, `onPlayerTryConnect`,
  `onPlayerTryChangeSlot`, `onPlayerTrySendChat` and `onChatMessage` (the
  hook built-ins, 10.6).
- `onSimulationStart` and `onGameEvent` are left out: no specification or
  plan row consumes them. `onPlayerTryChangeCoalition` is left out because
  HOOK §4 says not to register it: ED's coalition blocker consumes every
  event before a later hook sees it.
- A registration that fails stops the start before `configure`, so nothing
  binds and ADR 0056's one stop after the bind stays the only one.
- Each registered function returns nothing until `state.started` is set,
  which is the last statement of `driver.start`. A start that stops after
  registering leaves the table registered, since DCS cannot take it back,
  and its functions call no body.
- Each registered function looks its body up in `driver.callbacks[name]` at
  each call and returns the body's results unchanged. A name with no body
  returns nothing, which for an `onPlayerTry*` callback passes the event on
  down the hook chain, as SPEC 4 requires of a callback that does not veto.
- `driver.callbacks` is created once when the file loads, and `start` never
  replaces it.
- `driver.callbacks[name]` has one owner, the runtime. Where several
  consumers share a name, the runtime's body calls each in a fixed order:
  the dispatch loop and the epoch boundaries first, then the built-ins, then
  extension files. A consumer adds itself to the owner's body. A second
  assignment to `driver.callbacks[name]` replacing the first is a defect.

Alternatives:

- **Register after `configure` and stop on a refusal.** A second stop after
  the bind, which ADR 0054 refuses for the reason it gives.
- **Register after `configure` and log a refusal without stopping.** The
  bridge would listen with no callbacks, and nothing would dispatch.
- **Register each name when its consumer lands.** A second
  `setUserCallbacks` call adds a set; one call is what SPEC 6.10 asks for.
- **Register every callback ED documents.** A name nothing consumes is a
  call every frame or event for no body, and the stub harness has nothing to
  exercise it with.

## Consequences

Until the consumers land, every registered name has no body and returns
nothing. Under the stub harness, a name with no body counts as exercised
when its registered function is called and returns nothing.

A stop added to `driver.start` later goes above `state.started = true`, or
a start that stopped leaves bodies running.

A callback a later task needs and this set lacks costs a change to this set,
since a second registration adds a set rather than replacing this one.
