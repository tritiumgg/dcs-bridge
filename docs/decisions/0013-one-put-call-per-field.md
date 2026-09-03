# ADR 0013: One put call per field stands

## Status

Accepted

## Context

The sim driver's generated emitter makes one call from Lua into the broker
per field, per record. SPEC 5.1 fixes that shape and leaves its cost to a
probe:

> **The put calls add no allocation beyond producing the value.** The embedded
> broker writes into a preallocated buffer. A put call crosses into C and
> returns. The cost of that crossing is a measured input to **[PROBE-3]**.

SPEC 16 says what the probe decides:

> Decides whether one call per field is the right API shape, or whether a
> batched form is needed. [...] **The answer can invalidate Interface A's
> shape**, which every generated emitter targets.

SPEC 10 budgets the sim driver's frame share on a stand-in for the figure:

> A generic Lua-to-C crossing proxies at 0.6 to 0.85 µs — the put-call figure
> itself needs the broker (**[PROBE-3]**) — so ten put calls per frame are
> cheap and a full drain's several hundred are not free.

Two earlier records also wait on this probe. ADR 0011 puts a `SeqCst` fence
and a flag load on the logic thread for every record and names PROBE-3 as
where that is priced. ADR 0012 pads a nested message's length in place to save
a copy and names PROBE-3 as what would show the copy never mattered.

The probe is `tools/putcost.lua`, run by `mise run bench-put`. It times a
million calls of each put in a tight loop, from a stock Lua 5.1 against the
release build of the host-native module, and reads the Lua heap across the
same loop with the collector stopped. A stock `string.len` call is timed the
same way as this machine's generic crossing. `tests/lua/alloc.lua` asserts the
allocation column in CI.

## Decision

One put call per field stands. No batched put form is scheduled.

Two readings. The first is from the sim driver's state: the release DLL in a
DCS install, the probe run by `dofile` from `MissionScripting.lua` at the point
Route B places the sim driver, read out of `dcs.log`:

```
999424 calls per row
row                              us/run   us/cross      bytes
empty loop                        0.003      0.003        0.0
string.len (proxy)                0.015      0.015        0.0
integer                           0.035      0.035        0.0
double                            0.024      0.024        0.0
string, 28 bytes                  0.026      0.026        0.0
boolean                           0.026      0.026        0.0
message + end_message             0.044      0.022        0.0
begin + commit, empty             0.034      0.017        0.0
ten-field record                  0.294      0.025        0.0
```

The second is host-native, from an Apple M3 Max with other things running,
three runs within a few nanoseconds of each other:

```
999424 calls per row
row                              us/run   us/cross      bytes
empty loop                        0.003      0.003        0.0
string.len (proxy)                0.021      0.021        0.0
integer                           0.023      0.023        0.0
double                            0.022      0.022        0.0
string, 28 bytes                  0.027      0.027        0.0
boolean                           0.022      0.022        0.0
message + end_message             0.032      0.016        0.0
begin + commit, empty             0.030      0.015        0.0
ten-field record                  0.293      0.024        0.0
```

A put costs the same as the stock crossing beside it, so the broker adds
nothing measurable to the call: the figure is Lua's, and a batched form would
save calls into C that cost what a call into any C function costs. A ten-field
record is twelve crossings and 0.3 µs. The heavy load in SPEC 10, 3,040 records
per second at 70 Hz, is 43 records per frame, which at that size is 13 µs of
puts in a 500 µs share. At the specification's own proxy of 0.85 µs per
crossing the same load is 440 µs, which still fits the share with no batching.

The two readings agree on the record and on the allocation column, and differ
by a few nanoseconds on single puts, so the figure is not the host's. The
specification's proxy is twenty-five to forty times either one. Where it was
taken is not recorded, and the shape holds at either figure regardless.

The alternatives, and the line that rejected each:

- **A batched put: one call carrying several fields.** Trades N crossings for
  one crossing plus N table reads on the C side, and every generated emitter
  would carry the batching. At 25 ns a crossing there is nothing to trade.
- **A Lua-side encoder, crossing once with the bytes.** The plan set this
  aside before the probe: it makes Lua an encoder, which the broker is meant to
  be alone, and a put call already allocates nothing.

## Consequences

The fence and the wake stay as ADR 0011 placed them. PR #22 read the logic
thread's push at 28 to 38 ns with the fence and the flag load in it, and a bare
ring at 24 to 50 ns without them, so the fence does not show above the noise.
Against 0.3 µs of puts per record it is at most a few percent of the record's
cost, and removing it would buy that back at the price of a wake protocol Loom
cannot check. The wake at 7 to 10 µs is paid once per burst, which against
13 µs of puts per burst is real but comes to 2% of the frame share; the
writer's bound on empty passes is not revisited here.

ADR 0012's copy stays. A nested message's open and close together cost less
than one scalar put, so the put path's cost sits in the crossing and not in
the length handling.

Interface A's shape is settled, so Phase 3's generator emits one call per
field with nothing held back for a batched form.

This reopens if a later DCS build's reading comes out above 0.5 µs per
crossing, which puts the heavy load past half of its share. To take the
reading again, on an install with the DLL in the write directory:

1. Copy `tools/putcost.lua` to `<write directory>\Scripts\putcost.lua`.
2. In `<install>\Scripts\MissionScripting.lua`, above the sanitization block,
   add:

   ```lua
   PUTCOST_MODULE = lfs.writedir() .. 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll'
   dofile(lfs.writedir() .. 'Scripts/putcost.lua')
   ```

   Route B places the sim driver at the same point, so `package.loadlib` and
   `os.clock` are both still present there.
3. Start a mission. The table lands in `dcs.log` under the `putcost` tag.
4. Revert the edit. A DCS update reverts it silently otherwise.

PROBE-7 at task 9.7 measures the same path under a real production rate and is
the other reading that could reopen this.
