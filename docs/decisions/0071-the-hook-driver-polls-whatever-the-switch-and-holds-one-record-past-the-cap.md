# ADR 0071: The hook driver polls whatever the switch, and holds one record past the cap

## Status

Accepted

## Context

SPEC 6.4 gives the hook driver a loop of its own:

> the hook driver's loop polls the hook driver ring and dispatches to the
> hook-driver-side handlers, at most `hook_driver_dispatch_max_commands` per
> invocation. It runs from `onSimulationFrame`, `onPlayerConnect`,
> `onPlayerDisconnect`, `onPlayerChangeSlot` and `onMissionLoadEnd` [...] It
> dispatches nothing between `onMissionLoadBegin` and `onMissionLoadEnd`.
> Work above the cap defers to the next invocation and increments
> `hook_driver_dispatch_deferred_total`.

SPEC 11 says a disabled bridge stops "command ferrying", and ADR 0063 makes
`driver.enabled()` the predicate every such path consults. `ReloadConfig` is
how a file that sets `enabled = true` turns the bridge back on, so it has to
reach a disabled hook driver.

Five things are left open:

- **The poll and the switch.** The hook driver ring is the one way a
  `ReloadConfig` arrives, and nothing else drains it.
- **How the loop learns there is work above the cap.** `shim.poll` pops one
  record or answers `nil`. The module has no call that looks at the ring
  without popping it, or that says how full it is.
- **Where the blackout ends when `onMissionLoadEnd` never fires.** Whether
  DCS fires it after a load that fails is not measured. SPEC 9.1 measured
  zero frames between the two load callbacks.
- **A routed topic with no handler.** The schema routes a topic to the hook
  driver before its handler exists, and the registration surface of SPEC
  6.10.2 is plan row 5.13's.
- **The answer.** The one `CommandAck` per command, with its detail
  truncated at `command_ack_detail_max_bytes`, is plan row 7.5's.

## Decision

The hook driver polls its ring on every invocation whatever the switch says,
dispatches at most the cap, and holds the one record it popped past the cap
for the next invocation.

- **The cap.** An invocation dispatches the held record first, if there is
  one, then pops until it has dispatched `hook_driver_dispatch_max_commands`
  records or the ring answers `nil`. At the cap it pops once more. A record
  there is held in the hook driver and counted once in
  `hook_driver_dispatch_deferred_total`. A ring that held exactly the cap
  counts nothing, because no work waited.
- **The blackout.** `onMissionLoadBegin` sets a flag and the loop dispatches
  nothing while it is set. `onMissionLoadEnd` clears it before its own
  dispatch, and so does `onSimulationFrame`, since a frame means no load is
  running. A held record waits out the blackout and is dispatched first
  after it.
- **The switch.** While `driver.enabled()` is false the loop still polls.
  `dcsbridge.hook.ReloadConfig` dispatches, and every other record is
  discarded. The first record of each topic discarded writes one WARN,
  `<topic> discarded: bridge disabled`, and the next one writes nothing
  until the switch turns back on.
- **The handlers.** `driver.handlers` maps a topic to a function called with
  the decoded record, the sending connection's id and the topic. The table
  is created once when the file loads and the runtime owns it. Plan row 5.13
  builds its registration surface over the same table.
- **Containment.** A record whose topic has no handler, or whose decode or
  handler raises, is dropped with one ERROR line. The line is written once
  per topic until the next mission load, through the set ADR 0062 keeps for
  callbacks. The loop goes on to the next record.
- **No answer.** The loop sends nothing back. `ReloadConfig` reports what it
  did in `DCSBridge.log`, as `driver.reload` already does.
- **The order in a callback.** In `onSimulationFrame` the loop runs right
  after the tick (ADR 0063), and before the raise a clock that does not read
  makes, so a broken clock does not stop the commands. In `onMissionLoadEnd`
  it runs after the injection, so the commands that waited out the load run
  once the mission is in place. Later consumers come after it (ADR 0061).

Alternatives:

- **Poll only while enabled, as plan row 4.12 first read.** The ring would
  fill while the bridge is off, and a `ReloadConfig` sent to turn it on
  would be refused `BUSY` or never popped.
- **Count a deferral when the cap-th pop returned a record.** A ring holding
  exactly the cap would count a deferral where nothing waited, and the
  counter would read as pressure that is not there.
- **A peek or a length call on the module.** A new name on the module moves
  the interface version for a number the extra pop learns.
- **Hold the records a disabled bridge pops.** The ring has no push-back, so
  the hook driver would buffer without bound and run stale commands after a
  re-enable.
- **Clear the blackout at `onSimulationStop`.** Its order against
  `onMissionLoadBegin` when one mission replaces another is not measured. A
  stop after the begin would end the blackout during the load.
- **Answer each command now.** The refusals while disabled, the failure
  with no handler and the detail's truncation are row 7.5's to decide
  together.

## Consequences

One record may sit in the hook driver between two invocations. It is the
oldest work waiting, so dispatching it first keeps the ring's order.

A consumer whose command is discarded, dropped for want of a handler, or
fails learns nothing until row 7.5 answers it. The WARN and ERROR lines in
`DCSBridge.log` are the only trace.

`hook_driver_dispatch_deferred_total` is held in the hook driver and read
through `driver.counters()` until `shim.stats` publishes it, as ADR 0070
holds `hook_driver_files_failed_total`.

A load that ends without `onMissionLoadEnd` holds dispatch back until the
next frame. Frames fire at the menu, so that wait is one frame gap.

A dropped record takes its place under the cap, as a dispatched one does.

The hook driver refuses `hook_driver_dispatch_max_commands = 0`, at start and
at reload, though every other hook key takes 0. At 0 the loop would dispatch
nothing, `ReloadConfig` included, so no reload could lift the cap again and
only a DCS restart would undo it.

`ReloadConfig` takes no idempotency check. Its key is carried for plan row
7.3, and two reloads of the same file apply the same values.
