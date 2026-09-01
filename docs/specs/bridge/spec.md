# DCS-Bridge — Specification

A bridge between a running DCS mission and external programs.

Repository `DCS-Bridge`. Binary `dcsb`. Lua module `lua-dcsbridge.dll`.

**The bridge is not a lifeline.** Section 11's first row is the contract: **the
mission runs whether or not a consumer is connected**, and disconnecting the
bridge costs the mission nothing.

**Status:** Draft. Resolve every **[PROBE-n]** before you commit to the design.

## Provenance

These are measured on DCS 2.9.28.26385: Lua state contents,
per-state module search paths, the mission-load blackout, and
the crash results in Section 4. The instrument was a GameGUI hook evaluating
chunks with `net.dostring_in`, on a single-player host.

A single-player host is the most permissive vantage. A dedicated server is
unmeasured. Every figure is a property of that build. Re-measure after a DCS
update.

Some later figures were measured in a **hosted multiplayer** session rather
than single player, and say so where they appear: the `onPlayerTry*` thread
result in Section 4, the clock comparison in Section 5.2, the injected
environment's menu and timer surfaces in Section 5.1.2, and the integrity
baseline in Section 14.8. Hosted multiplayer is not a dedicated server, so
**[PROBE-10]** stands.

Figures measured on 2.9.29.27278 come from a GameGUI eval bridge on a
single-player host and a self-hosted multiplayer session; each figure that
depends on a build names its build.

**Mission identity is readable from the hook driver before the load ends.**
`DCS.getMissionTheatre()`, `DCS.getMissionName()` and
`DCS.getMissionFilename()` all return the incoming mission's values at
`onMissionLoadBegin`, a full callback before `onMissionLoadEnd`, and the
theatre is not a stale value carried over: a Caucasus mission followed by a
Syria one read `"Syria"` at the second load's begin. The theatre then reads
**nil at `onSimulationStop`** while the name and filename still read. So the
readable window opens earlier than the design needs and closes before teardown
finishes.

**`DCS.getMissionName()` is not the `.miz` name.** A mission flown from the
Mission Editor reports `"tempMission"` on every load, while
`DCS.getMissionFilename()` reports the real path. Section 6.10.5 depends on
this and says so.

One figure is explicitly incomplete.

`net.dostring_in` cost is linear at roughly 30 to 40 µs per KB of source
(2.9.29.27278) — about 0.5 ms at a full 16 KiB drain payload — with no size
ceiling found below 1 GiB (Section 5.3). It compiles its argument on every call
and has no chunk cache.

**The per-frame gap figures are now a distribution, not single observations.**
162,432 gaps were collected across three phases on 2.9.29.27278, bucketed at
5/10/20/50/100/200/500/1000/2000/5000/10000 ms.

| Phase | Gaps | Max | p50–p99 | p99.9 | p99.99 |
|---|---|---|---|---|---|
| running | 129,497 | **282.24 ms** | ≤20 ms | ≤50 ms | ≤200 ms |
| menu | 27,141 | 13,398.35 ms | ≤20 ms | ≤50 ms | ≤10,000 ms |
| paused | 5,794 | 1,759.79 ms | ≤20 ms | ≤20 ms | ≤2,000 ms |
| loading | **0** | — | — | — | — |

**The earlier 2.15 s running-phase figure was unrepresentative.** It came from
a 22.5-second window; against 129,497 samples the running-phase maximum is 282
ms, and 99.3% of gaps are 20 ms or under. The document was built around a worst
case about eight times larger than anything since measured.

**Read the menu and paused maxima carefully — they are transitions, not idle.**
A gap is credited to the phase in force when it *began*, so a mission load
appears in `menu` and a mission teardown appears in `paused`. The two menu
entries above 10 s are terrain loads of 5.2 s and 13.4 s; the paused maximum is
a 1.76 s teardown. Menu idle and paused idle are both ≤20 ms at every
percentile measured. **The largest gap this build produces that is not a phase
transition is 282 ms.**

**A destruction does not take effect within the calling frame** (measured,
2.9.29.27278). A unit destroyed and then looked up by name in the same chunk is
still found. Any test or handler asserting post-destruction state in the frame
that destroyed it is wrong.

**What is still not known.** These are lower bounds from one evening on one
machine, in single player, with synthetic spawns rather than a populated
server. The 282 ms maximum came from spawning 120 vehicles at once and is a
lower bound on spawn cost. A dedicated server under real load is **[PROBE-10]**
and remains unmeasured. Numbers derived from this distribution are sized with
margin for that reason, not because the distribution is doubted.

## Terms

**Language:** Short sentences. Active voice. One idea per sentence. Rules use
the imperative.

**Terms:** A **record** is one typed message. A **consumer** is an external
program connected to the bridge. The **logic thread** is the DCS thread that
runs the sim and all Lua. An **epoch** is one mission's lifetime on the wire:
it opens when a mission finishes loading and closes when the mission stops, and
every unit handle and subscription is void across its boundary (Section 9.4).
The **drain** is the once-per-frame transfer of buffered sim driver records to
the broker (Section 6.4 stage 1). **Route A** and **Route B** are the two sim
driver injection routes (Section 5.4.1). The four **record classes** —
`DURABLE`, `LOSSY`, `COMMAND`, `LIFECYCLE` — are broker drop policy (Section
8.1). A topic's **target** — sim driver or hook driver — names which component
owns it: emits it outbound, handles it inbound (Sections 5.2 and 8.2).
**`doctor`** is the diagnostic verb of the CLI (Section 15).

---

## 1. Scope

### 1.1 What the bridge does

The bridge moves typed records between a DCS mission and external programs, in
both directions.

**It is a publish-and-subscribe broker embedded in the game process.** Naming
the pattern buys the vocabulary: record classes are quality-of-service levels,
`LIFECYCLE` retention is a retained message, a capability set is a topic access
list, `SetTopicFilter` is per-connection topic selection, and `SeqAck` with the
spool is a persistent session. Where this document reasons about drop policy,
replay or filtering, it is reasoning about a broker, and a reader who knows one
already knows the shape of this.

It carries records. It does not know what a record means.

The bridge's own cost on the logic thread is small. The sim driver's cost is
whatever the mission asks of it. Section 10 budgets the sum, not the broker
alone.

The bridge targets a host or a dedicated server. On a connected multiplayer
client, `net.dostring_in` returns nil for every state name, so Route A cannot
inject the sim driver. Route B does not use `net.dostring_in` and is not
blocked by this. A client is still not a supported deployment. No figure in
this document describes one.

### 1.2 What the bridge does not do

- The broker holds no domain logic. The sim driver holds all of it.
- It defines no domain record. Its own records are enumerated in Section 5.2
  (broker-answered), Section 9 (lifecycle, including `CoordinateCalibration`),
  Section 9.5 (its own commands), Section 7.6 (the operator-eval audit record),
  and Section 8.5.3 (the acknowledgement record). Nothing else in the schema
  belongs to the bridge. `CoordinateCalibration` is the one that looks like an
  exception and is not: Section 8.2 makes every position field in every record
  DCS-local, so the calibration is how a consumer reads a convention the bridge
  imposes, not a description of the world.
- It is not an RPC framework. **The broker holds no per-request state**: it
  routes an answer to a connection and remembers nothing about the command it
  answers, so a command and its acknowledgement are two independent records on
  the wire. Correlation, timeouts and retries belong to the consumer, and
  Section 8.5.2 supplies all three through generated wrappers. There is no
  cancellation and no server-side request registry. Section 8.5.2 defines a
  request and reply convention on top, with generated wrappers, and Section
  8.5.1 explains why a subscription is usually the better shape.
- It does not make the mission depend on any consumer. See Section 11.

### 1.3 Workload and design target

These are two different things and the specification names both.

**First deployment.** One developer on a laptop with no DCS running, then a
small public server of at most five concurrent players. This governs build
order and default configuration values.

**Design target.** Servers substantially larger than the first deployment: more
concurrent players, more units in the air, more connected consumers. This
governs which features exist and how every limit in Section 13.1 is sized.

**Scale is the axis, and it is the only one.** A larger adopter is a busier
one, not a less trusted one. Section 14.1 assumes a trusted operator at every
size, and the Section 13.1 defaults are sized for the first deployment so that
a larger adopter raises a number rather than rewrites a design.

**The operator authors the missions they run.** That is the deployment this
document is written for, at every size. An operator who hosts mission content
from authors they do not vet is running a different service, and this document
neither serves nor defends that case. Section 14.1 states the consequence for
the threat model.

A developer laptop is a development workflow, not a workload. A mock producer
emits whatever schema is invented for it, so it cannot discipline record
content. **The small public server owns every decision about which records
exist and what fields they carry.**

### 1.4 One broker, one seam

The broker is a compiled module loaded into the DCS process. It owns the rings,
the fan-out to consumers, the authentication and the protocol parser. It runs
its own threads.

There is no second process. Broker lifetime is DCS lifetime. Starting DCS
starts it. Stopping DCS stops it. Nothing needs supervising.

**Interface A is a seam, not a promise.** Generated emitters target the
put-call API in Section 5.1, not the broker behind it. A second implementation
is therefore possible later without changing the sim driver, the generator or
any consumer. The obvious candidate is a rented host that restricts what an
operator may install.

No such implementation exists and none is planned. **This document makes no
claim that two implementations would be indistinguishable.** That claim is the
expensive part, and it is not made.

---

## 2. Components

| # | Component | Language | Location | Lifetime |
|---|---|---|---|---|
| 1 | Broker | Rust | `Mods\services\DCSBridge\bin\lua-dcsbridge.dll` | DCS process |
| 2 | Hook driver | Lua 5.1 | `Scripts\Hooks\DCSBridge.lua` loader, `Mods\services\DCSBridge\lua\HookDriver.lua` payload | DCS process |
| 3 | Sim driver | Lua 5.1 | Injected into the `"server"` state | One mission |
| 4 | Generator | Rust | Build machine | Build time |

The sim driver runs inside the `"server"` state. It can call anything its own
environment holds and that survived the Section 4.2 binding probe. **Which
environment that is depends on the route** (measured, 2.9.29.27278; Sections
5.1.2 and 5.4.1): a Route A sim driver lives in the injected environment, which
shares no globals with mission scripts, so MOOSE, MIST and mission-created
state are out of its reach. A Route B sim driver is loaded into the
mission-script environment itself and may delegate to a framework the mission
loads.

**`"server"` and `"scripting"` are two names for one state.** For this state,
ED's API reference in the install uses only `"scripting"`. Do not "correct"
`"server"` toward `"mission"`, the other state name that reference uses: that
is a different state and holds only mission-editor trigger-action wrappers.

The broker uses no garbage collector and no language runtime. A collector
inside the DCS process can stop the logic thread. That stops the sim for every
player at once.

**Everything outside the Lua files is Rust**: the broker, the CLI and the
generator. Rust satisfies the no-collector rule and Section 14.2 already names
it in the one place its runtime behaviour matters — a parser fault must unwind
rather than abort, so the broker is never built with `panic = "abort"`. The
broker links no protobuf runtime, per Section 3: it writes tags and varints by
hand into a preallocated buffer and parses only the envelope header. The CLI
and the generator run outside the DCS process and are under no such
restriction.

```
                DCS process
  ┌─────────────────────────────────────────────────┐
  │  logic thread                                   │
  │   ┌─────────────┐  dostring_in  ┌────────────┐  │
  │   │ Hook driver │ ←───────────→ │ Sim driver │  │
  │   └──────┬──────┘               └──────┬─────┘  │
  │          │  put / poll                 │        │
  │          └──────────┐      ┌───────────┘        │
  │                     ▼      ▼                    │
  │                ┌─────────────────┐              │
  │                │     Broker      │              │
  │                │  rings, threads │              │
  │                └────────┬────────┘              │
  └─────────────────────────┼───────────────────────┘
                            │ TCP, framed
                            ▼
              ┌─────────────┼─────────────┐
              ▼             ▼             ▼
         Consumer 1    Consumer 2    Consumer N
```

---

## 3. Extension rule

The broker does not know what a record is. It turns a field number and a value
into bytes. It holds no schema and links no protobuf runtime.

### 3.1 Adding a record type

Five steps. None rebuilds the broker.

1. Add the message to the `.proto`. Nothing else: the `Envelope` names no
   payload type and there is no shared registry to edit.
2. Run `buf generate`.
3. Call the generated emitter from the sim driver, or add a command handler
   there. Section 8.3 generates emitters and decoders. It never generates
   handlers.
4. Add a handler in the consumer.
5. Reload the mission, or send `ReloadSimDriver` (Section 6.9) — either
   replaces `DCSBridge.code`.

Under Route A the hook driver reads the sim driver from disk at each mission
load. Under Route B `MissionScripting.lua` loads it at bootstrap. Either way no
DCS restart is needed. Editing the hook driver itself does need one: DCS globs
`Scripts\Hooks\*.lua` once, at start.

**The five-step rule has two tiers.** Step 5's cost depends on the topic's
target (Section 8.2):

| Target | Cost of adding the record type |
|---|---|
| Sim driver | Five steps. Reload the mission, or `ReloadSimDriver`. |
| Hook driver | Five steps. Restart DCS. |

A hook-driver-targeted topic changes the hook-driver-side generated file
(Section 8.3), which the hook driver payload reads once at DCS start. The "no
DCS restart is needed" sentence above holds for the sim driver tier only.

**The extension sources follow the same two tiers.** A sim-driver-side
extension file — `SimDriver.gen.lua`, `SimDriver.builtin.lua`, or a file in the
operator's `simdriver.d\` directory — is re-read at each mission load and by
`ReloadSimDriver`, so it sits at the sim driver tier. A hook-driver-side
extension file — `HookDriver.gen.lua`, `HookDriver.builtin.lua`, or a file in
`hookdriver.d\` — is read once at DCS start, so it sits at the hook driver
tier. Section 6.10 defines both sets.

### 3.2 When the broker must be rebuilt

- The broker or the frame format changes. The Section 5.1 surface is part of
  the broker: adding a call — `routes`, `caps`, `schema`, `message`,
  `end_message` — or changing one is a rebuild.
- A new value type is needed, such as a binary blob or a 64-bit integer.
- The thread model or the drop policy changes.
- A bug is fixed.

A new `Capability` member is not a rebuild. An enum member is not a value type:
the broker enforces capabilities from a registered map (Section 14.4), so a new
member is a schema change and a regenerated map.

A new record type is free. A new value type is not. Choose the value types in
Section 5.1 with margin.

**No configuration value requires a rebuild. Not one.** Every key in Section
13.1 is read from `Config\DCSBridge.lua` at load and applied at one of the
three tiers in Section 13.2. Ring sizes in particular are configuration: an
adopter with a fifty-player server raises them and restarts DCS.

If a future setting would need a new binary to change, it is not a setting. It
is a build constant, and it belongs in the list above rather than in Section
13.1.

---

## 4. Code rules

- Write all Lua for reference Lua 5.1. DCS does not use LuaJIT.
- `bit`, `ffi`, `jit`, `string.pack`, and `goto` do not exist. Do not use them.
  A varint encoder is therefore `math.floor` and `%` arithmetic, and a double
  encoder needs `math.frexp`. The broker encodes, so no Lua encoder exists.
  Keep it that way.
- **Never call a `luaopen_*` export.** `lua.dll` exports nine standard-library
  openers — `base`, `bit32`, `debug`, `io`, `math`, `os`, `package`, `string`
  and `table` (Section 5.1.1) — and ED's own Lua calls none of them by name.
  Two hazards follow, and the second is the serious one.
  - **`bit32` is compiled in and registered in no state.** It is a Lua 5.2
    library; it is absent from every state surveyed on 2.9.29.27278. The rule
    above therefore stands, and it stands for a reason worth recording — **do
    not "correct" it after finding the symbol in the export table.** Calling
    `luaopen_bit32` would install a global into a state shared with every other
    tool, which is the collision Section 6.1 exists to prevent.
  - **Under Route B, `luaopen_io`, `luaopen_os` and `luaopen_package` would
    undo the mission sandbox.** `Scripts\MissionScripting.lua` removes exactly
    those three names, and a Route B sim driver runs in that same environment,
    beside the mission's own scripts (Sections 5.1.2 and 5.4.1). Its bootstrap
    runs before the sanitisation block, so those names are gone by the time its
    per-frame loop runs — which is exactly the moment someone would reach for
    an opener to get them back. Doing so would restore them for every mission
    script on the server, defeating the block that Section 5.4.1 says is "the
    whole point". Under Route A the question does not arise: the injected
    environment already holds `require`, `package`, `lfs`, `io` and `os` as
    direct members, so an opener would restore nothing. **The rule is
    route-independent because the reason above is name-independent** — no
    bridge Lua calls any opener, on either route.
- Use `unpack`, not `table.unpack`.
- Use `table.concat`. Repeated concatenation is O(n²).
- Put one `pcall` around the whole body of every DCS callback. **Log a raise
  once per callback per epoch and suppress the rest.** DCS never disables a
  raising hook callback: it keeps calling it. An unsuppressed error in a
  per-frame callback fills `dcs.log` at the callback rate, which is observed
  behaviour from a third-party hook on the measured install.
- **Return nothing from an `onPlayerTry*` callback unless you mean to veto.**
  Returning *any* value breaks the hook call chain, so every script loaded
  after yours never sees the event. Returning nothing or nil continues the
  chain to ED's default allow-all handlers. `onPlayerTryConnect` and
  `onPlayerTryChangeSlot` return a boolean, and `onPlayerTryConnect` may pair
  `false` with a second return value, the disconnect reason string;
  `onPlayerTrySendChat` returns a filtered message string, where an empty
  string drops the message. ED's shipped reference lists only three
  return-value callbacks. The list is incomplete: ED's own
  `Scripts\Hooks\multiplayerCoalitionBlocker.lua` registers and returns from a
  fourth, `onPlayerTryChangeCoalition`, including a three-value form — `false`,
  a reason string, and a cooldown in seconds. Apply this rule to the
  `onPlayerTry*` shape, not to ED's list. Never let a `pcall` failure path
  return a value. Section 4.5 records two projects that learned this the hard
  way.
- **Route every `DCS.*` call through one accessor table in the hook driver.** A
  namespace change is then one edit. See Section 4.4.
- **The `onPlayerTry*` callbacks run on the logic thread** (measured,
  2.9.29.27278, hosted multiplayer). ED's reference says they "are called
  directly from the network code, so try to make them as fast as possible",
  which describes the call site rather than a distinct thread. A deliberate 200
  ms spin inside `onPlayerTrySendChat` and inside `onPlayerTryChangeSlot`
  advanced the `onSimulationFrame` counter by **zero** on both, against a
  measured 72 Hz baseline where a separate thread would have shown about
  fourteen frames. So ED's advice stands for the reason it always did — these
  callbacks block the sim — but they need no cross-thread machinery.
  `onPlayerTryConnect` was not measured: it needs a second player joining, and
  ED groups all three identically. Treat it as the same thread until something
  says otherwise.

### 4.1 ED's event dispatch has no `pcall`

`Scripts\World\EventHandlers.lua` is the whole implementation. `world.onEvent`
iterates `world.eventHandlers` with `pairs` and calls `handler:onEvent(event)`
bare.

- **Wrap the sim driver's `onEvent` body in `pcall`.** An error that escapes
  aborts dispatch for that event. Every handler after yours never sees it,
  including MOOSE, MIST and the mission's own scripts. This is the
  mission-scripting analogue of the `onPlayerTry*` veto rule.
- **The exposure runs both ways and cannot be fixed.** A third party's raising
  handler starves the sim driver of every event dispatched after it. You cannot
  detect this from inside your own handler. See Section 11. Measured scope
  (2.9.29.27278): the injected environment carries its own
  `world.eventHandlers` registry, separate from the mission scripts' one, and
  the engine dispatches to both (Section 5.1.2). The starvation exposure is
  therefore per registry: a Route A sim driver shares a dispatch loop with
  other *injecting* tools, and mission-script handlers such as MOOSE live in
  the other one. That containment is inferred from the separate registries, not
  measured.
- **Handler order is unspecified.** `pairs` over a table keyed by object
  identity has no defined order. Never assume you run first or last.
- **Never call `world.addEventHandler` from inside an event handler.** Lua 5.1
  permits modifying or clearing an existing field during a `next` traversal.
  Assigning a field that does not yet exist is undefined. `world.onEvent` is a
  live `pairs` traversal of that table. Removal from inside a handler is
  defined; registration is not.

### 4.2 `pcall` does not contain every fault

A `pcall` around a callback body is necessary and **not sufficient**.

Six functions on the mission-scripting surface terminate the process with an
access violation despite being called inside `pcall`. Five are identified on
2.9.28.26385: `Unit.getSensors`, `land.findPathOnRoads`,
`SceneryObject.getDescByName`, `Disposition.getRandomSort` and
`coalition.remove_dyn_group`. All five die on a bare call. A sixth crash was
observed from a call that answered a bare probe and died once an argument was
supplied; it could not be attributed to a specific binding. An unattributable,
argument-dependent crasher is the reason the startup probe exists rather than a
static blacklist.

**A static survey under-reports, so absence from one is not absence.** A
packaged API index enumerates namespaces, not the contents of data tables.
`AI.Option` is measured present in the mission-scripting state with 30
distinct option ids across Air, Ground and Naval — 26, 8 and 2, overlapping on
`ROE`, `FORMATION` and `NO_OPTION` (measured, 2.9.29.27278; SIM §3) — and
`db.Weapons.ByCLSID` and `env.mission.date` are measured present too, while all
three are absent from the packaged index and from ED's shipped Lua. A survey
concludes wrongly that they do not exist. Read a survey as evidence that a
binding is present, never as evidence that one is missing.

**A seventh, in the hook state, observed on 2.9.29.27278** while measuring the
hook-driver-side coordinate conversion Section 6.3 now specifies. One chunk
took DCS down with a mission running. It contained two candidates and the crash
was not attributed to either: `terrain.convertLatLonToMeters`, called for the
first time, and an unguarded `DCS.getMissionLoaded()` in the same chunk as four
other calls — which is the batching Section 4.3 forbids, so that chunk broke
this document's own rule and is not clean evidence against the `terrain`
binding. Both are suspect until someone runs them one per chunk.

**Treat the hook driver's `terrain` table as a crasher family until probed
member by member.** `terrain.convertMetersToLatLon` is measured safe across
repeated calls (Section 6.3), and `terrain.GetTerrainConfig` is measured safe
for the `SW_bound` and `NE_bound` keys, which the hook driver built-ins call
once per epoch (HOOK §10.2). Everything else in that table is unproven, and
one member, `terrain.findPathOnRoads`, shares a name with
`land.findPathOnRoads` on the register above — which is reason to suspect the
two tables wrap the same bindings, and reason to probe rather than assume.

Therefore:

- Probe once at startup and blacklist what raises.
- **A clean probe licenses nothing about the same binding under other
  arguments.** A bare-call sweep does not find a function that dies only on
  input.
- A command handler that touches an unfamiliar binding stays behind a config
  flag until it has run against a throwaway mission.

### 4.3 Never batch `DCS.*` calls

One injected chunk called `DCS.getMissionName`, `getMissionTheatre`,
`getPlayerUnitType`, `getSimulatorMode` and `getMissionLoaded` in sequence. It
crashed with an access violation inside `ED_lua_copyindex`. That routine copies
a value between Lua states. At least one of the five therefore reaches into the
simulation state for its answer.

- Call `DCS.*` functions one at a time, each guarded.
- Prefer a value a state can produce without crossing into another. Where a
  per-frame value has no such source, use the `DCS.*` binding ED uses for it
  itself, called alone and guarded. `mission_time` is that case: the hook
  driver publishes it and the hook driver has no `timer` table, so it calls
  `DCS.getModelTime()` (Section 5.2).
- The routine is fragile beyond batching: a 2.9.29.27278 session crashed with
  `ED_lua_copyindex` recursing to full stack depth during multiplayer slot
  entry, with no injected chunk in flight.


### 4.4 A confirmed platform migration

Eagle Dynamics is replacing the `DCS.` function namespace with `Sim.`. This is
confirmed from the install, not a rumour, and the alias is already live.

- ED's own API reference shipped in the install, `API\Sim_ControlAPI.md`, is
  written entirely in `Sim.*`. The token `DCS.` does not appear in it.
- On 2.9.28.26385 `_G.DCS` and `_G.Sim` are the **same table object** in both
  the hook state and the mission-scripting state. Only the removal of one name
  remains.
- ED's own tree is barely migrated: a handful of Mission Editor launcher and
  updater paths call through `Sim.`, while the rest of the Mission Editor and
  every shipped hook script still call through `DCS.`.

**Write `DCS.*` today.** The runtime global in a hook is `DCS`, and ED's
reference is ahead of the runtime rather than behind it. Do not "correct" this
document's `DCS.*` spellings toward the shipped doc.

The migration affects Section 4.3, Section 9.2, Section 9.3 and Section 14.8,
which name `DCS.getMissionName` and its siblings, `DCS.getPause`,
`DCS.getRealTime`, `DCS.setMaxFPS`, `DCS.getTaintedFiles` and
`DCS.getTaintedCategories`. It does not affect the mission-scripting API, the
sim driver, or the broker.

Isolate every `DCS.*` call behind one accessor table in the hook driver.
Section 4 requires that, and it turns the migration into one edit.

### 4.5 A precedent for the return-value rule

Two production projects shipped a returning handler and removed it.

DCS-gRPC returned the message from `onPlayerTrySendChat` through release 0.7.1.
Pull request 265 removed the return, and release 0.8.0 shipped the fix. The
note reads: "Don't return a value from `onPlayerTrySendChat` as this stops
other hook functions from getting their shot at reacting to the event."

DCS-SimpleSlotBlock returned `true` from `onPlayerTryChangeSlot`. Commit
`1bcdd4f`, "Fix for multicrew prompt", removed it: "Removed return true value
that interupted the multicrew check occurring from onPlayerTryChangeSlot."

Both broke another script before they found the Section 4 rule.


---

## 5. Interfaces

The bridge has three interfaces. Specify and test each one against a fake of
the other side.

### 5.1 Interface A — broker to Lua

#### The calls

This is the whole surface.

```
shim.begin(topic)          -- the payload's type URL, a generated constant
shim.integer(field, n)     -- integer
shim.double(field, x)      -- double
shim.string(field, str)    -- string
shim.boolean(field, bool)  -- boolean
shim.message(field)        -- open a nested message on that field number
shim.end_message()         -- close the innermost open nested message
shim.commit()              -- close the record and queue it
shim.begin_to(conn_id, topic_id)  -- open a point-to-point record (Section 8.5.3)
shim.poll(target)          -- return conn_id, topic_id, bytes from that target's
                           -- ring (Section 5.2). Return nil when empty.
shim.configure(table)      -- apply settings. Once at load, and on live changes.
shim.tick(mission_time)    -- publish mission time (DCS.getModelTime, 5.2) and
                           -- stamp the heartbeat atomic
shim.epoch(id)             -- publish the current epoch id, or nil outside one.
                           -- Once per boundary, not per record. See Section 5.2.
shim.classes(table)        -- register topic -> record class. Additive; see below.
shim.routes(table)         -- register topic -> target. Additive; see below.
shim.caps(table)           -- register topic -> capability. Additive; see below.
shim.schema(bytes)         -- hand the broker the compiled schema, opaque. Once.
shim.stats()               -- return counters. See Section 12.
```

Every put call carries its field number. The generated emitter supplies it.

There is no absent marker. Protobuf omits an absent field.

#### Records and values

**Point-to-point records.** A record opened with `begin_to` goes to one
connection: the broker writes it to that connection's queue and to no other.
Close it with `commit`, like any record. `begin_to` is a separate call rather
than a flag on `begin`: a mode set by one call and read by another survives an
abandoned record and then mis-addresses the next one. `poll` returns the
sending connection's id so a handler can address the answer. The broker assigns
connection ids, unique for the life of the DCS process and never reused, so a
late answer to a closed connection cannot reach a new one. A `begin_to` record
whose connection has closed is discarded and counted against that connection.
Acknowledgements, typed replies and the broker-answered `Rejected` are the only
point-to-point records (Sections 5.2 and 8.5.3); everything else fans out.

**The broker refuses a `begin_to` on a topic the schema did not mark as a reply
or an acknowledgement**, and counts it in `misaddressed_total`. The generator
only emits `send_to` for those messages (Section 8.3), so a refused call means
hand-written Lua. Refusing is worth the check: a record that silently reaches
one consumer instead of all of them presents as missing data at every other
consumer, which is a miserable fault to trace.

**Integer range.** `shim.integer` accepts a signed 64-bit value. Lua 5.1
numbers are doubles and represent integers exactly only to 2^53. A value beyond
that range loses precision before the broker ever sees it. Carry anything
larger as a string, such as a 64-bit id or a hash. `Envelope.seq` is `uint64`.
The broker assigns it per connection (Section 5.2), not Lua, so it is exempt
from this limit.

**Nested and repeated message fields.** `message` opens a submessage on a field
number and `end_message` closes it. Every put call between the two writes into
that submessage, and submessages may nest. A repeated message field is one
`message` and `end_message` pair per element, every pair on the same field
number. The broker writes the submessage body to a scratch buffer and emits the
tag and length when the pair closes, which is the same one copy per record the
length-prefixing rule below already describes. A `commit` leaving a submessage
open is a defect: the broker discards the record and counts it, exactly as it
does an abandoned `begin`. The generated depth and repetition bounds in Section
14.2 apply to the outbound side as well as the inbound one.

**Field order does not matter, except inside a repeated field.** Protobuf
identifies fields by number, so reordering put calls on distinct fields breaks
nothing. Repeating one field number appends an element instead of replacing it,
so for a repeated field **put-call order is element order**. Reordering there
reorders the array.

**Length prefixing.** A length-delimited protobuf field needs its length before
its tag. Write the body to a scratch buffer, then emit the tag, the varint
length, and the body. That is one copy per record.

**The put calls add no allocation beyond producing the value.** The embedded
broker writes into a preallocated buffer. A put call crosses into C and
returns. The cost of that crossing is a measured input to **[PROBE-3]**.

**There is no raw-bytes put.** Every record crosses as typed put calls, so the
broker remains the only encoder and the only producer of record bodies. If a
forwarding use case ever needs one, adding it is a broker rebuild (Section 3.2)
and a deliberate widening of the Section 14.2 attack surface.

**Error handling.** The next `begin` discards a record that started and never
committed, and counts it.

#### Configuration and registration

**`configure` comes first.** The hook driver calls it once before any other
Interface A call. Until it does, the broker has allocated no ring and opened no
listener. A `begin` or a `poll` before the first `configure` is a defect, and
the broker answers it with an error rather than a default.

**`configure` takes the same shape `stats` returns.** A flat table of string
keys to numbers, strings and booleans. No nesting. It carries only the rows
Section 13.1 marks **broker**.

**A later `configure` applies the live keys and nothing else.** Section 13.2
says which those are. The broker applies them as one atomic swap, ignores the
restart-tier keys, and returns the count of each. It never reallocates and
never rebinds.

**Validate at the boundary.** The table is operator input crossing into the
broker, which Section 14.2 calls the highest-risk code. Range-check every value
and reject the whole call on any failure, rather than applying part of it.

**The broker reads no file and parses no Lua.** The hook driver reads
`Config\DCSBridge.lua` — it is a Lua file in a Lua state, so reading it is free
— and hands the broker a table. One source of truth, one parser, and the broker
parses nothing. An unknown key is ignored and counted, so a config written for
a newer build does not fail an older broker.

**`configure` answers with the broker's interface version.** The hook driver
compares it against the version its payload was built for. On a mismatch the
hook driver takes the Section 11 broker-failure path: log both versions,
disable, leave the mission unaffected. A partial update — Lua files replaced
while a running DCS kept the DLL locked — is detected here instead of surfacing
as undefined behaviour. `doctor` prints both versions.

**`stats` return shape.** A flat table of string keys. Values are numbers,
except `sim_driver_code_sha256`, `sim_driver_route` and the
`connection_token_id.<conn>` keys, which are strings, and
`sim_driver_direct_broker`, which is a boolean. A per-connection, per-class or
per-subscription breakdown is a composite key such as
`subscription_eval_us.<id>`, never a subtable. No nesting. See Section 12 for
the key set. The sim-driver-owned and hook-driver-owned metrics in that table
do not come from here.

**`classes`, `routes` and `caps` are additive over disjoint topic sets.** Two
registrars exist: the hook driver registers the hook-driver-side tables from
the generated file at DCS start (Section 8.3), and the sim driver registers the
sim-driver-side tables at each load. The broker merges a registration whose
topics are new. A registration naming an already-registered topic with the
identical value is a no-op — a sim driver reload re-registers its own tables
and must succeed. A registration naming a registered topic with a different
value is an error and the broker refuses that whole call. No registration ever
replaces or removes an entry; retiring a topic is a DCS restart.

**The three tables do not cover the same topic set, and that is deliberate.**
`routes` carries inbound topics only, because routing is what an inbound record
needs; `classes` and `caps` carry every topic that crosses in either direction.
An outbound-only topic therefore has a class and a capability and no route, and
that is correct rather than a gap.

**A topic missing a class or a capability is refused, not defaulted.** The
broker cannot recover either value: it holds the schema bytes opaque and parses
none of them. So it has no drop policy for a record it holds no class for, and
no answer to "does this token's capability set cover it" for a record it holds
no capability for. Both fail closed. The broker refuses a `begin` or `begin_to`
naming such a topic, refuses an inbound record on it with `Rejected` reason
`UNKNOWN_TOPIC`, and counts it in `partial_registration_total`. **The
capability case is the one that matters**: a missing entry that failed open
would disclose a record to every connection, so it fails closed even though
that loses records. `doctor` names every such topic, because the condition
always means generated files from two different runs — which the Section 8.3
hash check should also have caught.

The listener opens at the first `configure`, before the hook driver registers
its tables, so a window exists in which inbound records can arrive unrouted. No
separate rule closes it: the broker refuses every record whose topic id is not
in the route map (Section 5.2), and before registration that is every record.

**`schema.pb` crosses once.** After the first `configure`, the hook driver
reads `Mods\services\DCSBridge\schema.pb` (Section 8.3) and hands the bytes to
`shim.schema`. The broker stores them opaque, hashes them, serves them from
`GetSchema`, and puts the hash in the handshake (Section 5.2). It parses none
of them — the broker still holds no schema it understands, and still reads no
file. Until the hand-off, the handshake omits the hash and `GetSchema` answers
with an error. A second `shim.schema` call is refused: replacing the served set
is a DCS restart, the same tier as `HookDriver.gen.lua` (Section 3.1).

### 5.1.1 Loading the broker

`require` will not find it. `Scripts\UserHooks.lua` ships with its
`package.cpath` line commented out, so the hook state's C search path does not
reach the write directory.

Load by explicit path:

```lua
local path   = lfs.writedir() .. 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll'
local loader = assert(package.loadlib(path, 'luaopen_dcsbridge'))
local shim   = loader()
```

`package.loadlib` is the Lua 5.1 spelling and is present in the hook state. The
bare `loadlib` global is Lua 5.0 and is absent.

If both states load the broker, `luaopen_*` runs twice and each state gets its
own Lua table. Make the rings, the sockets, the threads, and the class, route
and capability maps process-global. The maps have two registrars — the hook
driver and the sim driver, in different states (Section 5.1) — and a per-table
map means the two registrars never see each other.

**The DLL lives under `Mods\services\DCSBridge\bin\`.** Two DCS conventions
compete for it and neither is scanned in a way that matters here. Nothing globs
`Scripts\` for DLLs: DCS globs `Scripts\Hooks\*.lua`, and `Scripts\?.lua` is
first on the hook state's `package.path`, but neither reaches a subdirectory's
DLL. `Mods\tech\` and `Mods\services\` are both walked at startup, but what DCS
does there is driven entirely by `entry.lua`. A mod mounts its own paths by
calling `mount_vfs_model_path` and `mount_vfs_texture_path` with explicit
arguments — ED's own `CoreMods\tech\` modules do exactly that, and the paths
they name are arbitrary rather than matched by directory name. Every directory
under the install's `CoreMods\tech\` and `CoreMods\services\` ships an
`entry.lua`, and each one mounts author-chosen literal paths.

**A directory with no `entry.lua` runs nothing, measured.** Bait files under
five plausible names — `init.lua`, `autoexec.lua`, `main.lua`, `load.lua` and
their own directory's name — were placed in write-directory trees under both
`Mods\services\` and `Mods\tech\`, each with a `bin\` child, and none executed
across two DCS sessions.

**Whether anything merely opens such a file is unmeasured, and is left that
way.** Answering it needs a filesystem trace rather than a sortie, and the
answer changes nothing: the DLL is loaded by explicit path, so an inert
directory and a walked one behave the same for this bridge. A surprise there
would be a surprise about surface, not about function.

**The plugin loader was not located and is not in the obvious places.** The
string `entry.lua` appears in no binary under `bin\` or `bin-mt\`, in neither
ASCII nor UTF-16, and in no shipped Lua under `Scripts\` — yet the `CoreMods\`
plugins demonstrably load. So the scanner lives somewhere neither search
reached. That is recorded because it is the reason the paragraph above stops
where it does, and **not** as evidence for the paragraph before it: a string
that is absent everywhere cannot distinguish a walked directory from an inert
one.

`Mods\services\` is the better of the two. In practice `Mods\tech\` holds
content: liveries and asset packs, and the folder an operator empties when
hunting a content conflict. `Mods\services\` says what this is.

Neither precedent is exact, and the document should not pretend otherwise.
DCS-SRS established `Mods\Services\` as a location — that is the shipped casing
— but it ships an `entry.lua` and is a declared plugin. Tacview established the
loading pattern this bridge uses, a GameGUI hook loading its own DLL from
`Mods\tech\Tacview\bin\` by explicit path, but it uses `tech`. Combining the
location with the pattern is new.

**Tacview's lack of a plugin declaration is unverified.** Tacview is not in the
measured install, and it registers an Options special page, which DCS normally
serves from a `declare_plugin` in an `entry.lua`. Treat Tacview as evidence for
the explicit-path loading pattern and for the `bin\` subdirectory, and as
evidence for nothing else.

**Ship no `entry.lua`.** A directory under `Mods\` with one becomes a declared
plugin, loaded at DCS startup into the main `globalL` state, before the hook
driver runs. That is more surface, not less, and it is the change most likely
to move the integrity check in Section 14.8. Loading by explicit path removes
any dependency on module registration, which is the whole point:
`package.loadlib` takes a path and needs no plugin declaration to find it. That
argument stands on its own and does not rest on the Tacview precedent above.

The `lua-dcsbridge.dll` naming matches the convention DCS uses for its own
Lua-loadable modules. DCS ships 23 of them in `bin\` and again in `bin-mt\`,
every one named `lua-<module>.dll`. Twenty export `luaopen_<module>` —
`lua-terrain.dll` exports `luaopen_terrain`, and so on. The three LuaSocket
modules, `lua-md5`, `lua-mime` and `lua-socket`, export `luaopen_<module>_core`
instead. `lua-dcsbridge.dll` plus `luaopen_dcsbridge` follows the majority
form.

**Enabling the commented `package.cpath` line would not find this DLL.** That
line reads `package.cpath = 'bin/lua-?.dll;bin/?.dll;'..package.cpath`. Both
entries are relative to the install root, so `require('dcsbridge')` would
resolve `<install>\bin\lua-dcsbridge.dll` and never reach the write directory.
The naming convention is worth following for recognisability; it buys no
`require` path.

**What the broker links against.** DCS ships its own Lua in `bin\lua.dll` — not
`lua51.dll`, which is the name an import library must carry. Measured on
2.9.29.27278: PE32+ x86_64, 130 named exports, undecorated, and **no LuaJIT
symbol of any kind**, which confirms Section 4's first rule from the binary
rather than from behaviour. `bin\` and `bin-mt\` ship byte-identical copies.

The 130 divide into three sets:

| Set | Count | Use |
|---|---|---|
| `lua_*` and `luaL_*` | 114 | The stock public API. This is what the broker binds. |
| `luaopen_*` | 9 | Standard-library openers: `base`, `bit32`, `debug`, `io`, `math`, `os`, `package`, `string`, `table`. Section 4 forbids calling them. |
| Internal | 7 | `luaD_growstack`, `luaF_newproto`, `luaM_realloc_`, `luaM_toobig`, `luaS_newlstr`, `luaU_dump`, `luaU_print` |

The internal seven are symbols a stock build keeps private. Do not call them.
They are an artefact of how ED built the DLL, not an interface.

**The public API is stock Lua 5.1**, so stock headers are safe to compile
against. Bind to it through an import library generated from a checked-in
`.def` naming `lua.dll`, which needs no DCS install at build time and pins
exactly which Lua symbols the broker depends on.

**`lua.dll` is an MSVC build importing `VCRUNTIME140.dll` and the UCRT.** A
module built with a different toolchain therefore carries a different C
runtime. Two runtimes in one process are harmless until memory crosses between
them, so: **never free across the boundary, never pass a `FILE*`, never pass a
CRT handle.** The Lua C API needs none of that — Lua copies every string it is
given and allocates through its own allocator.

Keep the failure path from Section 11. An `assert` inside the outer `pcall`
triggers it. The hook driver logs the error, disables itself, and leaves the
mission unaffected.

### 5.1.2 Where the sim driver writes

**Under Route A the sim driver routes its output through the hook driver.**
That is the specified path.

Where the sim driver can reach the broker without the hook driver, it writes
into the broker directly. Under Route B it always can, because the sim driver
captured `package.loadlib` at bootstrap. `SimDriverLoaded` reports three facts:
the injection route, whether the sim driver reaches the broker directly, and
the sandbox level. **A consumer must not depend on any of them.**

The `"server"` state as the sim driver sees it retains `require` on any
operator install (measured, 2.9.29.27278; below).
`Scripts\MissionScripting.lua` contains a block that removes six names: `os`,
`io` and `lfs` from both `_G` and `package.loaded`, then `require`, `loadlib`
and `package` from `_G`. The `loadlib` line is a defensive no-op — reference
Lua 5.1 never defines that global, which is why Section 5.1.1 says the bare
name is absent. Those names are nonetheless present in the `"server"` state on
a stock install, when reached through `net.dostring_in`.

The surviving names are **direct members** of that state's globals. A
reflection traversal reported them at bare top-level paths. That traversal
enumerates with `next`, reads with `rawget`, and files a metatable as its own
node at its own path. It would therefore have filed an inherited member under
the metatable's path instead. No `__index` fallback is involved. Raw reads
confirm it independently: direct members, no `__index` fallback.

**The environments are separate** (measured, 2.9.29.27278, both directions,
with execution controls: the probing triggers demonstrably ran — engine flags
set, on-screen text observed). A global written by a mission trigger is
invisible to an injected chunk, and a global written by an injected chunk is
invisible to a mission trigger. The injected environment persists across calls
and across a mission restart (Section 9.4), and holds `require`, `package`,
`lfs`, `io` and `os` as direct members with no metatable. The sim driver's
reach is therefore a property of the broker and survives any operator sandbox
setting — under Route A. The same separation cuts the other way: an injected
sim driver cannot see mission-script globals (Section 2).

The separation runs deep: the two environments hold **different `world` tables
and different `world.eventHandlers` registries**, confirmed by table address —
and the engine dispatches events to both. A handler registered from the
injected environment receives live world events; an observed sequence ended in
`S_EVENT_UNIT_LOST`. **Route A's event model works.**

**So do its menu and timer surfaces** (measured, 2.9.29.27278, hosted
multiplayer). From the injected environment, `missionCommands.addCommand`
returned a path table and its callback **fired** when a player picked the item
from the F10 menu; `missionCommands.removeItem` accepted that path and
succeeded; and `timer.scheduleFunction` **fired** on schedule. Section 6.10 can
therefore make the runtime the sole owner of every `missionCommands` handle on
both routes, and Section 6.1's warning about a discarded
`timer.scheduleFunction` id describes a real leak rather than a theoretical
one.

The sim driver reads `mission_scripting_sandbox_level` from the `config` state
at injection and reports it, so an operator can see which regime they are in.
Two sibling keys, `user_hooks_sandbox_level` and `unit_database_sandbox_level`,
govern other states and are not covered by that report.

### 5.2 Interface B — broker to consumer

**Connections.** DCS listens. Consumers connect. Reconnect logic lives in the
consumer. Bind to loopback or to a private interface. See Section 14.3.

**Frame format.** `[u32 length, little-endian][payload]`.

**Payload.** Protobuf wire format. Each frame is one `Envelope`.

```proto
import "google/protobuf/any.proto";

message Envelope {
  uint64              seq          = 1;
  optional uint32     epoch        = 2;
  optional double     mission_time = 3;
  google.protobuf.Any payload      = 4;
}
```

**The topic is the payload's type, and protobuf already names it.** An `Any`
carries the serialised message as bytes together with a type URL of the form
`type.googleapis.com/<package>.<Message>`. That URL is the topic id. There is
no topic table to negotiate, no number to allocate, and no registry to
coordinate: a message's identity is its fully-qualified name, which the schema
already fixes and every protobuf runtime already exposes through its
descriptor.

Three consequences follow, and each removes machinery rather than adding it.

**Nobody assigns a topic id.** An adopter or a mission author writes a
`.proto`, and the type URL falls out of the package and message name. Section
8.2 therefore partitions no numbered space for topics, and two independently
written extensions cannot collide unless they choose the same fully-qualified
name — which their package names already prevent.

**The `Envelope` is closed.** It names no payload type, so adding a record type
touches no shared file. Section 8.4's schema-ownership check therefore stops
policing a shared numbered registry — there is none — and becomes a naming
check over the `dcs.bridge` package instead.

**The wire is self-describing.** A capture is readable without a schema, and
`dcsb tail` names types with nothing loaded. The cost is about 43 bytes of type
URL per record; Section 10 prices it.

`epoch` and `mission_time` are `optional` because each has a case where zero is
wrong rather than meaningful (Section 8.4). A record emitted outside any epoch
omits both. `MissionLoadBegan` is the standing case: at `onMissionLoadBegin`
the previous epoch has closed and the next is not allocated. A consumer treats
an absent epoch as not epoch-scoped and never discards the record under Section
9.4's rule, and treats an absent mission time as the sim not running.

Protobuf earns its place for four reasons:

- Generators exist for every consumer language.
- Field numbers make order drift impossible.
- A decoder that meets an unknown field number skips it and keeps its place. A
  consumer one release behind logs the number and continues.
- `Any` is the language's own answer to carrying a message whose type is not
  known at compile time, so every runtime ships `pack`, `unpack` and a type
  check for it. Exhaustive dispatch is no longer free from the schema; Section
  8.3 generates a sealed dispatch type per consumer instead, with an explicit
  unknown case that carries the type URL and the bytes. A consumer meeting a
  record it does not know can therefore name it, which a `oneof` could not.

Most runtimes treat a wire-type mismatch on a known field number as an unknown
field and skip it, for the skippable wire types — varint, 64-bit, 32-bit and
length-delimited. It therefore fails to populate the field rather than
producing a garbage value, and it does not raise. **A skipped field is
indistinguishable from an absent one at the consumer**, so a mismatch reads as
a missing value rather than as an error.

The exception is a wire type that cannot be skipped: the deprecated group types
3 and 4, and the invalid types 6 and 7. A decoder that meets one of those
raises rather than skipping, whatever field number carries it. The schema uses
no groups, so this arises only from a corrupt or hostile frame, where a raise
is the correct outcome and Section 14.2 drops the connection.

`buf breaking` in CI is the real guarantee against drift.

Do not use a schema-driven untagged encoding such as Avro binary. Its reader
reconstructs types from the schema alone, so drift decodes as wrong values.

**Several consumers.** The listener accepts up to a configured maximum and fans
each outbound record to every connection. Inbound records merge into two
inbound rings, one per target.

**Routing.** The broker reads the payload's type URL from every inbound frame,
so it knows the topic before Lua sees it. That is one length-delimited descent
further than a top-level field — into the `Any`, then its first field — and
Section 14.2 bounds it like any other read. Inbound volume is capped at
`inbound_records_per_sec_total`, so the per-frame string comparison this
implies is bounded by configuration rather than by traffic. The registered
route map (Section 5.1) sends each inbound record to the sim driver ring or the
hook driver ring. The broker-handled topics — `Ping`, `Auth`, `GetSchema`,
`GetTopics`, `SetTopicFilter`, `SeqAck`, `SetEnabled` — are consumed on the
reader thread and reach no ring. **The broker refuses a record whose topic id
is not in the route map**: it answers with `Rejected` reason `UNKNOWN_TOPIC`
(below), counts
it in `unrouted_topic_total`, and delivers it nowhere. It never defaults an
unrouted topic to the sim driver. The reader thread writes both rings; the sim
driver polls one and the hook driver polls the other, so each ring keeps one
producer and one reader — under Route A the hook driver polls both, ferrying
sim driver records over Interface C, and each ring still has exactly one
reader.

Every limit named in this section has a default and a stated basis in Section
13.1.

**The logic thread writes one ring, not N.** `commit()` appends to a single
producer ring. The writer thread reads that ring and fans each record into a
per-connection queue. Fan-out on the logic thread would make `max_connections`
multiply logic-thread cost per record. A configuration change would then become
a performance change.

Each connection then has its own queue. A consumer that stops reading must not
cost another consumer its records. Count drops per connection. Disconnect a
consumer whose queue stays full.

Route an acknowledgement to the connection that sent the command.

**Acknowledgements and replies are point-to-point.** The broker fans out
everything else. A handler addresses a point-to-point record with `begin_to`
and the connection id `poll` returned (Section 5.1). Two consumers therefore
see different record streams. That is why `seq` is per connection rather than
global. A capture taken on one connection replays only that connection's view.

**Sequence numbers.** The broker assigns `seq` per connection, monotonic, after
the capability filter (Section 14.4) and before the drop decision. A gap in
`seq` means records were dropped, and only dropped: a record a connection is
not entitled to was filtered before numbering, so filtering leaves no gap.
Ordering is total per connection.

A consumer assigns `seq` on the records it sends, per connection, monotonically
increasing from 1. The broker does not validate it; it reads it from the
envelope header and echoes it in `Rejected` (below), and nothing else consumes
it.

`seq` orders **emissions, not observations**. Section 6.8 states the sim driver
constraint that makes the two equivalent.

**Backpressure.** Allocate every ring once, at the first `shim.configure`, from
the size it carried: `ring_out_records` per connection,
`ring_in_sim_driver_records` and `ring_in_hook_driver_records` inbound. Fixed
for the life of the process. `luaopen_dcsbridge` allocates nothing. The
record-ring sizes are provisional until **[PROBE-7]**.

When an inbound ring is full, drop the newest: a queued command is not stale.
Never block the logic thread. Never allocate. **Answer the sender with
`Rejected` reason `BUSY`**, counted in `commands_rejected_total` by reason like
any other refusal. The drop happens on the reader thread, which is the ring's
producer and already holds the connection, so answering costs no logic-thread
work.

**What `BUSY` buys is knowing *which* command was dropped, not knowing that
something was.** The dominant cause of a full inbound ring is the Section 9.1
mission-load blackout: the ring holds about 2.5 s of
`inbound_records_per_sec_total` and a load stops the drain for tens of seconds,
so a consumer can already infer congestion from `MissionLoadBegan`. What it
cannot infer is whether the command it sent a moment earlier was executed or
discarded. Without `BUSY` that question is answered only by
`request_timeout_ms` expiring, which is indistinguishable from a handler that
ran and lost its reply.

**`BUSY` does not mean retry immediately.** Inside a blackout the sim cannot
run the command for as long as the load takes, and Section 13.1 sizes the ring
on the premise that commands arriving then are mostly meaningless. A consumer
that receives `BUSY` between `MissionLoadBegan` and `MissionLoaded` waits for
`MissionLoaded` before re-sending. Outside a blackout it backs off and retries.
Either way it re-sends with the correlation field it used the first time —
`request_id` for a read, `idempotency_key` for a mutation (Section 8.5.2) —
and the refusal happened before Lua, so no key entered the recent-key set and
the retry is the first execution.

`BUSY` has its own rate cap, `busy_max_per_sec`, separate from
`rejected_max_per_sec`. The split is about what the refusal means, not about
amplification: all four reasons answer a record the broker delivered nowhere,
so none of them amplifies anything, and over TCP to an authenticated peer there
is nothing to reflect. The other three indicate a misbuilt or misbehaving
consumer, where a low cap keeps a defective client from filling a log. `BUSY`
answers a correct consumer during normal congestion, so its cap is set to
`inbound_records_per_sec` and **is non-binding by invariant**: one inbound
record yields at most one answer, so the cap cannot fire while the inbound
limit holds. It exists so that the bound is stated rather than assumed, like
every other structural cap in Section 13.1. `doctor` marks it non-binding.
Refusals above either cap are counted in `rejections_suppressed_total` and not
answered.

**A broker answer is treated as `DURABLE` by the drop rule.** `Pong`, `Schema`,
`Topics`, `TopicFilterResult`, `AuthResult` and `Rejected` carry no record class
(Section 8.1), and every rule below is written as `LIFECYCLE` against
non-`LIFECYCLE`. Without this they would fall outside the drop policy entirely,
and a `BUSY` — whose whole purpose is to tell a consumer its command died —
could vanish uncounted. **Count both paths under the label `broker_answer`**:
an eviction, and a refusal at the `ring_out_lifecycle_reserve` watermark. The
refusal is the path a broker answer actually meets, because it is always the
newest push, so counting evictions alone would leave the original fault
exactly where it was.

**The outbound drop rule has two halves, and both are needed.**

- **Evict the oldest record that is not `LIFECYCLE`.** Plain drop-oldest is not
  enough. The oldest record in a saturated ring may well be an `EpochClosed`,
  and evicting it is exactly the failure this rule exists to prevent. Count
  every eviction by class.
- **Refuse the newest non-`LIFECYCLE` record once free space falls to
  `ring_out_lifecycle_reserve`.** Reserving occupancy keeps room for the
  boundary records a consumer needs, and costs one comparison per push.

Occupancy and eviction are different decisions. A reserve alone leaves
`LIFECYCLE` evictable. An eviction rule alone lets a `LOSSY` flood leave no
room to write the next `EpochClosed`. Specify both.

**A ring that is full of `LIFECYCLE` is a disconnect, not a drop.** If eviction
finds no non-`LIFECYCLE` record to remove, the consumer is so far behind that
it has already missed epoch boundaries and holds references into a world that
no longer exists. Its view is unrecoverable. Drop the connection, count it in
`lifecycle_disconnects_total`, and let it reconnect into a fresh handshake and
a fresh `seq` origin. Never discard the record instead.

**Single producer, single consumer.** Every ring, inbound and outbound, has one
atomic write index and one atomic read index — which is why a broker answer is
handed to the writer thread rather than pushed by its producer. Use no mutex. A
lock would let the logic thread block behind a slow socket.

**Time and epoch.** Every `Envelope` carries mission time and the current
epoch, and the broker stamps both from values the hook driver publishes — it
reads neither from Lua per record. The logic thread publishes mission time to
an atomic each frame through `shim.tick`. **The hook driver publishes the epoch
id the same way, once per boundary**, with `shim.epoch(id)` at
`onMissionLoadEnd` and `shim.epoch(nil)` at `onSimulationStop`; between the two
the broker stamps every record with the current id, and outside them it omits
the field, which is the `MissionLoadBegan` case Section 5.2 describes. The hook
driver makes that call from `onSimulationFrame` under both routes: the hook
driver loads the broker in every configuration, and the render-loop callback is
the only per-frame callback the process offers. A thread that never touches Lua
can then stamp a reply. Put no wall clock in a per-record stamp: the sim owns
the clock and the sim pauses. The one wall-clock value on the wire is the
mission-start time in the handshake and in `EpochOpened`, which is an epoch
anchor rather than a timestamp.

**The hook driver reads mission time from `DCS.getModelTime()`, not from
`timer.getTime()`.** `timer` is a mission-scripting table and is absent from
the hook state, so the hook driver cannot call it. `DCS.getModelTime()` is
present in the hook, mission-scripting and GUI states alike, and **ED's own
hook and UI code use it as the sim clock**: `Scripts\Hooks\webGUI.lua` returns
it as a field it names `mission_time`, and `Scripts\UI\GameInfo.lua` and
`Scripts\UI\TriggerPictures.lua` both call it from per-frame update functions
and subtract two readings to age an animation. That establishes the state and
the frequency, which is what this call needs.

**The two clocks are the same clock** (measured, 2.9.29.27278, hosted
multiplayer). `DCS.getModelTime()` in the hook state and `timer.getTime()` in
the mission-scripting state returned an identical value, 181.438000, read
either side of one cross-state call. So a consumer may correlate a bridge
record against a mission script's own log without adjustment.

**Model time is frame-quantised.** Two `DCS.getModelTime()` calls bracketing
that cross-state call returned the same value, so the clock advances once per
frame rather than freely. That is the right shape for a per-record stamp —
every record from one frame carries one mission time — and it is why Section
5.2 can publish it to an atomic once per frame instead of reading it per
record.

This is one `DCS.*` call per frame, alone and guarded, which is what Section
4.3 permits. It is the only per-frame `DCS.*` call the design makes.

**Handshake and order of operations.** The connection proceeds in a fixed
order: handshake, then authentication, then everything else.

The handshake frame carries the protocol version, the broker version, the
instance id, and the schema hash, which is the SHA-256 of the compiled
`FileDescriptorSet` the hook driver handed the broker at start (Section 5.1).
**That is everything an unauthenticated peer learns.** It discloses that a
bridge is present and which schema build it runs. It discloses nothing about
the mission or its players.

Mission start wall-clock time and current mission time are **optional**
handshake fields. A consumer may connect before any mission has loaded, when
neither exists. `EpochOpened` carries the authoritative pair.

A schema hash mismatch produces a warning, not a refusal.

**Five request/reply pairs answered by the broker.** These never reach Lua and
carry no record class.

| Request | Reply | Purpose |
|---|---|---|
| `Ping` | `Pong` | Liveness |
| `Auth` | `AuthResult` | Authentication |
| `GetSchema` | `Schema` | Returns the `FileDescriptorSet` the broker holds (Section 5.1) |
| `SetTopicFilter` | `TopicFilterResult` | Narrows what this connection is sent (below) |
| `GetTopics` | `Topics` | Lists the topics this connection can see (below) |

**Authentication is two messages.** `Auth` carries the token and nothing else.
`AuthResult` answers it: `ok`, and on failure an error code — bad token, empty
capability set, or server full. After a failed `AuthResult` the broker closes
the connection.

`GetSchema` requires authentication. Unauthenticated, it would disclose the
whole API surface to a port scan.

**`GetTopics` answers what this deployment actually registered.** `GetSchema`
serves the compiled `FileDescriptorSet`, which is every message type the build
knows. That is not the set a connection will receive: registration is what
decides that (Section 5.1), and it varies with which built-in files are in
place, what an adopter added under `simdriver.d\` and `hookdriver.d\`, and
whether a sim driver loaded at all — Section 9.5's no-driver case is a working
configuration with a much smaller registered set. Without this a consumer
cannot tell a topic that will never arrive from one whose first record has not
happened yet, and cannot choose topic ids for `SetTopicFilter`.

```proto
message GetTopics {}

message Topics {
  repeated TopicEntry topic = 1;
}

message TopicEntry {
  string           topic_id     = 1;
  RecordClass      record_class = 2;  // Section 8.2
  optional Target  target       = 3;  // inbound topics only, per Section 5.1
}
```

**The reply lists only topics the token's capability set covers.** That is what
keeps it from being the registration oracle Section 14.4 exists to prevent: it
discloses exactly the set this connection could already send or receive, and
nothing about the rest of the vocabulary. A token covering three topics learns
about three topics. `GetTopics` therefore needs no capability of its own, and
requires authentication like every message except `Ping` and `Auth`.

**`GetTopics` answers with an error until the class table is registered.** The
listener opens at the first `configure`, before the hook driver registers its
tables (Section 5.1), so a window exists in which the registered set is empty.
An empty `Topics` in that window is indistinguishable from a correct answer,
which is the failure `GetSchema` already avoids by erroring until the schema
hand-off completes. `GetTopics` does the same.

**The answer is point-in-time.** Registration is additive across a DCS process
and irreversible (Section 5.1), so the set can grow at a mission reload and
never shrinks. A consumer that cares re-asks after `EpochOpened`; one that does
not will simply start receiving a topic it did not know about, which
`SetTopicFilter` lets it ignore.

**`SetTopicFilter` narrows what a connection is sent.** Capability decides what
a connection *may* see (Section 14.4). The topic filter decides what it
*wants*. The two are different questions and the broker asks both at fan-out.
**This is not Section 6.7's subscription**, which is a predicate the sim driver
evaluates in the mission. This one creates nothing and reaches no Lua: it
selects among records the broker would otherwise fan out.

```proto
message SetTopicFilter {
  TopicFilterMode mode     = 1;  // Section 8.2. Refused at 0.
  repeated string topic_id = 2;  // the whole set under ONLY, empty under ALL
}

message TopicFilterResult {
  bool                    ok       = 1;
  optional TopicFilterRefusal refusal = 2;  // set when ok is false
  uint32                  admitted = 3;  // see below
  repeated string         unknown  = 4;  // named but not admissible now
}
```

Eleven rules govern it.

- **The default is `ALL`.** A connection that never sends `SetTopicFilter`
  receives every fan-out record its capability set permits. Narrowing is
  opt-in, so a consumer that does not know about the mechanism is never starved
  by it, and one that forgets to set a filter never meets a silent empty
  stream.
- **`SetTopicFilter` replaces the set. It never accumulates.** Each message
  carries the complete topic list, so there is no removal message, no order
  dependence between two of them, and no drift between what the consumer
  believes and what the broker holds. Re-sending the same message changes
  nothing the broker holds. It may still return a different reply: registration
  is additive, so `admitted` can rise and `unknown` can shrink at a mission
  reload, which is the supported way to watch a covered topic become
  registered.
- **A topic is admissible when the class table holds it** (Section 5.1), not
  when the route map does. `routes` carries inbound topics only, and a topic
  filter names outbound topics, so reading "registered" as "in the route map"
  would admit nothing. A topic registered with a class but no capability is
  refused at fan-out under Section 5.1's fail-closed rule and counts as not
  admissible here.
- **`LIFECYCLE` topics are always admitted.** Naming them under `ONLY` is
  unnecessary and omitting them changes nothing. A consumer that could filter
  away `EpochOpened` would keep receiving records whose `epoch` field it can no
  longer interpret, and Section 5.2's rule that a consumer never misses a
  `LIFECYCLE` record would become conditional on consumer configuration. It
  stays absolute **with respect to this filter**. The capability set still
  bounds it: a token whose set does not cover the epoch topics misses them at
  fan-out (Section 14.4), and an operator who issues such a token has built a
  consumer that cannot interpret its own stream. Grant every token the
  capability covering the Section 9 lifecycle set. The retained replay is
  filtered the same way and is otherwise unaffected.
- **Point-to-point records are never filtered.** An acknowledgement, a typed
  reply and a `Rejected` are addressed rather than fanned out, so the topic
  filter does not reach them, exactly as the capability filter does not
  (Section 14.4). A consumer always receives the answer to its own command.
- **A topic filter narrows and never widens.** A topic the token's capability
  set does not cover stays invisible whether or not `SetTopicFilter` names it.
  The capability filter runs regardless, so `SetTopicFilter` needs no
  capability of its own: it can only reduce what a connection already had.
  Section 8.1 exempts it from carrying one, as it does every broker-answered
  message.
- **`unknown` is computed after the capability filter, and that is a security
  rule.** A topic id is listed there when the class table does not hold it
  **or** when the token's capability set does not cover it. The two are
  deliberately indistinguishable. Reporting them apart would make the field a
  registration oracle **for ids outside the token's capability set** — name any
  id, read the answer, learn whether this mission registered it. Registration
  state *within* the covered set is disclosed by design, and `GetTopics`
  answers it directly; the conflation exists to stop the question being asked
  about everything else. A consumer reading its own id back in `unknown` learns
  that it will receive nothing on that topic, which is all it needs and all it
  may have.
- **`admitted` counts the topics this connection will now be sent**: every
  admissible topic its capability set covers and its filter admits, including
  the `LIFECYCLE` topics admitted by rule 4, and excluding everything listed in
  `unknown`. It is zero when `ok` is false. It is a point-in-time answer:
  later registration can raise it, and nothing re-reports it.
- **Filtering happens before `seq` assignment**, like the capability filter, so
  a withheld record leaves no gap and a consumer's own narrowing never looks
  like data loss. Count it in `records_filtered_total` (Section 12), by
  connection and cause, never in `records_dropped_total`.
- **An unadmissible topic id is kept, not refused.** Registration is additive
  across a DCS process (Section 5.1), so a consumer may name a topic an
  adopter's file registers at the next mission reload. The broker keeps it in
  the set and admits it once it becomes admissible, having already reported it
  in `unknown` so a typo is visible rather than silent.
- **Four shapes are refused whole, with `ok` false, a `refusal` reason and the
  previous filter left in force.** Mode `TOPIC_FILTER_MODE_UNSPECIFIED`, which
  proto3 makes indistinguishable from an omitted field, so a consumer that
  forgets the mode is told rather than guessed at. Mode `ALL` with a non-empty
  `topic_id`, which states two intentions at once. A list longer than
  `topic_filter_max_topics`. And a list the broker cannot parse. Duplicate ids
  are not a refusal: the broker de-duplicates before applying the cap.

**Mode `ONLY` with an empty list is legal** and yields the `LIFECYCLE` set
alone. It is the one way to ask for boundaries and nothing else, and a consumer
that reaches it by accident sees `admitted` equal to the `LIFECYCLE` count it
can see, which distinguishes it from a refusal wherever that count is non-zero.
Before registration completes it is zero, and `GetTopics` is what tells the two
apart.

**Only the writer thread pushes to an outbound ring.** Every reply in the
five-pair table above is produced on the reader thread, which hands it to the
writer thread through a single-producer answer queue per connection rather than
pushing it itself. The writer thread then does all three things that must agree
with each other: it assigns `seq`, it applies the drop rule, and it pushes.
Every ring in the broker therefore keeps one producer and one consumer, and the
rule below holds without exception.

**This is a deliberate simplification and it is worth its cost.** A second
producer on the outbound ring would need a published-commit index, a `seq`
counter ordered against the ring claim rather than against itself, and an
eviction rule that is a read-modify-write of interior slots while another
producer appends. Each is a different algorithm, none of them is one sentence,
and all three are on the path that carries every record to every consumer. The
answer queue costs one hand-off and removes the need for all of them. **A
broker answer therefore reaches a connection in the writer thread's order, not
the reader thread's**, which is what makes the ordering rules below stateable
at all.

**The filter is published by pointer swap.** `SetTopicFilter` arrives on the
reader thread and fan-out runs on the writer thread (Section 5.2), so the
reader builds the new set, publishes it with one atomic store, and the old set
is reclaimed after the writer's next pass. No mutex, matching the ring
discipline. **The filter applies from the first record fanned out after that
store.** Records already in a connection's queue are delivered, because the
queue holds encoded bytes and re-examining it would put the broker's own work
on the critical path for no correctness gain.

**A resync stream is filtered like any other fan-out.** Section 6.8's scan
emits ordinary records, so a narrowed consumer receives a narrowed resync. That
is the intended reading: `Resync` asks for the current state of what this
consumer is watching.

**The filter survives every epoch boundary unchanged.** It is connection state,
not epoch state, so a mission load neither clears it nor rebuilds it, and
`SimDriverReloaded` does not touch it. A topic that a differently-configured
mission registers for the first time becomes admitted silently, per rule 10.

**The filter dies with the connection, and a reconnect starts at `ALL`.** No
state outlives the connection that owns it. **A reconnecting consumer must
re-send its filter**, and must expect full `ALL` traffic between authentication
and that message, including the retained `LIFECYCLE` replay. This matters most
in the case that produces it: Section 14.5 disconnects a consumer whose queue
stays full, and a consumer that narrowed its filter *because* it could not keep
up is rewidened to the firehose exactly when it reconnects. Re-send before
processing live records.

**`SetTopicFilter` requires authentication, like every message except `Ping`
and `Auth`.** Those two are the whole pre-authentication surface. Section
5.2's handshake paragraph states what an unauthenticated peer learns, and the
topic filter adds nothing to it.

**`Rejected` is the broker's refusal record.** Like `Pong` and `Schema` it is
broker-answered, carries no record class, and is point-to-point to the refused
sender.

```proto
message Rejected {
  uint64         seq      = 1;  // the inbound envelope's seq, echoed
  string         topic_id = 2;
  RejectedReason reason   = 3;  // Section 8.2
}
```

The broker already parses all three; it copies them into the refusal and
remembers nothing, so Section 1.2's no-correlation
rule stands. It answers four refusals: an unrouted topic id, a topic the
token's capability set does not cover (Section 14.4), a rate-limited record
(Section 14.5), and a record the target ring had no room for (Section 5.2). A
refusal the parser cannot attribute — a frame whose envelope header does not
parse — is a connection drop per Section 14.2, never a `Rejected`. Every
`Rejected` is counted in `commands_rejected_total` by reason. **The broker
emits at most `rejected_max_per_sec` `Rejected` records per connection for
`UNKNOWN_TOPIC`, `NO_CAPABILITY` and `RATE_LIMITED`, and `busy_max_per_sec` for
`BUSY`**;
refusals above that rate are counted and not answered, so a flood cannot buy
amplification (Section 14.5).

**`Pong` carries DCS liveness, not broker liveness.** The broker runs its own
threads and answers `Ping` without touching Lua. A wedged logic thread with a
live reader thread is the case `Ping` exists to detect.

The hook driver calls `shim.tick` every frame and the broker decides what to do
with it: mission time is published on every call, and the heartbeat atomic is
stamped at most once per `heartbeat_interval_ms`. **The throttle lives in the
broker, not at the call site** — the hook driver cannot skip the call without
also skipping the mission-time publish. The reader thread reads the stamp's
age.

| Field | Meaning |
|---|---|
| `dcs_alive` | Age of the stamp is under `dcs_alive_threshold_ms` |
| `dcs_last_heard_ms` | Age of the stamp |
| `bridge_enabled` | Effective value of the `enabled` key. Section 11. |

**The threshold must exceed the drain tail, which is now measured.** Across
129,497 running-phase gaps the maximum is 282 ms and p99.99 is under 200 ms
(see Provenance). The expensive case — a gap while running and producing — is
the best-characterised of the phases and the smallest.

**Phase transitions are what the threshold has to survive, not steady state.**
A mission load is a total frame blackout of tens of seconds (Section 9.1),
covered explicitly by `dcs_alive_threshold_loading_ms` between
`MissionLoadBegan` and `MissionLoaded`. **Mission teardown is the case with no
record raising the threshold**: ending a mission produced stalls of 1.16 s and 1.76 s, which
arrive with no warning to the consumer. `dcs_alive_threshold_ms` must clear
that with margin, which is what its default does.

`MissionLoadBegan` raises the threshold for the load window specifically,
instead of pinning it at a worst case nobody has measured.

**Connection lifetime.** DCS owns the listener, so DCS exiting drops every
consumer connection. Reconnection produces a fresh handshake and a fresh `seq`
origin. No state outlives the world it describes.

**LIFECYCLE retention.** The broker retains the most recent record of each
`LIFECYCLE` topic: the payload bytes **and the envelope's `epoch` and
`mission_time` as stamped, including their absence**, keyed by the type URL it
already reads,
holding their relative emit order. After a connection authenticates, the broker
delivers the retained set in that order, **rewriting only `seq`**: each record
keeps the `epoch` and `mission_time` it carried, and a record that carried
neither still carries neither. That is what makes Section 9.4's discard rule
work on a replay — a consumer joining epoch N receives the retained
`EpochClosed` stamped with epoch N−1 and discards it, rather than reading the
epoch it just joined as ended. **`EpochClosed` is emitted before
`shim.epoch(nil)`**, so it carries the closing epoch rather than no epoch;
emitting it after would make it undiscardable under Section 9.4 and reproduce
the fault this rule prevents. The writer thread pushes the retained set as the
first records on the connection's ring, ahead of any live record and ahead of
any queued broker answer, so the ordering needs no second mechanism and no
record is skipped to obtain it; records emitted during the replay queue behind
it. A consumer that connects or reconnects mid-mission therefore receives the
current
epoch's context — `EpochOpened` among it — and one that connects during a load
receives `MissionLoadBegan` and applies `dcs_alive_threshold_loading_ms`
(Section 9.1) until `MissionLoaded`. Retention is
class policy, not schema knowledge: the slots are allocated when `shim.classes`
registers the class table, one per `LIFECYCLE` topic, and a newer record of a
topic replaces the older in place. **The slots are allocated at the first
`configure`, all `max_lifecycle_topics` of them, and registration only binds a
slot to a topic**: allocating at `shim.classes` would allocate again at every
mission load, against the allocate-once rule above. Standing memory is
therefore `max_lifecycle_topics` × `max_lifecycle_record_bytes`,
unconditionally and from the first `configure`.

**A `LIFECYCLE` record larger than a slot is refused at `commit`**, counted in
`lifecycle_oversize_total` by topic, and named by `doctor`. It is a schema
defect rather than a runtime condition: a boundary record that large is
carrying something that belongs in a `DURABLE` one, and the alternative —
retaining it and letting the slot grow — puts an allocation on the commit path
that the allocate-once rule forbids. The emitting handler learns nothing,
because `commit` reports no per-record status (Section 5.1); the counter and
`doctor`
are how an adopter finds it, which is the same treatment
`partial_registration_total` gets for the same class of build-time mistake.

**The retained set is bounded by `max_lifecycle_topics`.** A slot holds a
record up to `max_lifecycle_record_bytes`, registration is additive and
irreversible for the life of the DCS process (Section 5.1), and an adopter may
add `LIFECYCLE` topics, so the count needs a bound like every other limit
(Section 13.1). A `shim.classes` call that would take the registered
`LIFECYCLE` count above the cap is refused whole, like any other refused
registration. This extends the class's meaning — a consumer must never miss a
`LIFECYCLE` record — to consumers that were not connected at emit. It requires
every `LIFECYCLE` topic to carry last-value semantics; Section 8.1 states that
constraint. The replay passes the same capability filter as live fan-out
(Section 14.4), before `seq` assignment, like any other record.

**Commands.** Commands use the same framing in the other direction. `poll`
hands them to Lua from the target's ring.

### 5.3 Interface C — hook driver to sim driver

This interface uses `net.dostring_in("server", ...)`.

**ED marks this API obsolete.** The API reference shipped in the install,
`API\Sim_ControlAPI.md`, documents the signature as `net.dostring_in(state,
string) -> string` and labels it **OBSOLETE and UNSAFE!!!**. Route A therefore
rests on a call ED has already deprecated. Section 5.4 covers the policy gate.
Section 5.4.3 covers the replacement ED points at.

**Cost scales with source length.** `net.dostring_in` compiles its argument on
every call and there is no chunk cache. The batching rule below produces large
chunks, so the large-chunk figure governs cost, not the minimal one. Measured
on 2.9.29.27278: about 30 to 40 µs per KB of source, plus 15 to 30% for the
`%q`-escaped form the drain sends.

The capacities are measured (2.9.29.27278): **no chunk-size ceiling exists
below 1 GiB**, and a returned string comes back byte-exact to at least **1
MiB**. A very large chunk does not fail — it stalls the whole process for
minutes and leaves the state holding gigabytes of garbage until a collection
runs. The bounds on `drain_max_bytes` and `bridge_return_max_bytes` are
therefore frame-cost bounds, not protocol bounds: at 30 to 40 µs per KB, a full
16 KiB drain costs about 0.5 ms of logic-thread time.

**The two bounds measure two different representations of one crossing, and
that is what makes both necessary.** `drain_max_bytes` bounds the record
payload the sim driver has buffered — the figure a handler's emissions add up
to, and the only one the sim driver can measure before it builds anything.
`bridge_return_max_bytes` bounds the assembled `%q`-escaped table literal that
actually crosses, which the escaping above makes 15 to 30% larger. The sim
driver fills the drain to `drain_max_bytes`, serialises, and stops adding
records at `bridge_return_max_bytes` whichever cap it reaches first. The
defaults leave headroom for the worst escaping ratio: 16 KiB of records at 30%
expansion is about 21 KiB against a 32 KiB return bound. **Cost is a property
of the escaped form**, so `bridge_return_max_bytes` is the figure Section 10's
budget prices.

**Under Route B neither bound describes a crossing**, because there is no
Interface C. `drain_max_bytes` still applies there, as a bound on the work one
frame's drain does; `bridge_return_max_bytes` does not apply at all.

**The crossing carries a defined payload grammar in each direction.** Outbound,
the drain returns `"OK|"` followed by a Lua table literal: a list of records,
each a `topic_id`, an optional `connection_id` for a record opened with
`begin_to`, and an ordered put-call log of `(kind, field, value)` triples using
the Section 5.1 kinds. The sim driver serialises values with `%q` for strings
and a `%g` form for numbers — the established table-literal approach, not a
protobuf encoder, so Section 4's rule stands.

**Number precision is a frame-cost decision, not a formatting preference.**
`%.17g` round-trips a double exactly and costs up to 24 characters. `%.9g`
costs about half that and resolves a metre-scale coordinate to roughly a
millimetre over a 500 km map, which is far below anything the sim models. A
position record carries five numbers, so the choice moves that record's
crossing by about 23%, and position records are the ones emitted at rate.

**DCS carries positions as single-precision floats, which settles it.**
`coord.LLtoLO` returned `x = -219720.796875` for an input of `-219720.8`
(measured, 2.9.29.27278). The source data holds no more precision than `%.9g`
prints, so `%.17g` on a position field spends bytes on noise.

**Use `%.9g` by default and `%.17g` only where a value must round-trip
exactly.** The emitter picks per field from the schema, not per call: mission
time, epoch anchors and any field a consumer subtracts from another keep
`%.17g`; positions, velocities, headings and angles take `%.9g`. Where the
distinction is unclear, keep `%.17g` — the cost is bytes, and the alternative
failure is a silently wrong value. This applies to Route A only; Route B
crosses no text. The hook driver loads the body with `loadstring` and replays
each log into the shim verbatim. Inbound, each polled command crosses as its
`connection_id` and `topic_id` plus the opaque body bytes as one `%q`-escaped
string literal in the injected chunk; the sim driver hands the bytes to its
generated decoder unchanged. The grammar ships with the hook driver payload and
the sim driver together, and carries its own `GRAMMAR_VERSION`. **It is not
`STATE_VERSION`.** The two change for different reasons: a grammar change moves
bytes between the hook driver and the sim driver and touches no stored state,
so bumping `STATE_VERSION` for it would force a cold reload on every running
server and drop every subscription and spot for nothing. The hook driver
refuses to inject a sim driver declaring a different `GRAMMAR_VERSION` and
takes Section 11's injection-failure row.

- Treat every returned string as untrusted. A chunk that raises returns its
  error text as a normal string, and `pcall` still succeeds. Return `"OK|"`
  plus the body. Check the prefix before you parse.
- Separate three refusal cases. An unknown state name returns nil. A known but
  unavailable state name returns `"Invalid state name"`. A call refused by
  operator policy is a third case. These three shapes are measured behaviour,
  not documented behaviour: ED's shipped reference specifies no refusal
  contract. See Section 5.4. Report each distinctly and fail closed.
- Escape injected source with `string.format("%q", src)`. A chunk that contains
  `]]` closes a `[[` literal early.
- Make at most one call per frame in each direction. Batch everything else.
  **Operator eval is the one exception** (Section 7.3): a `server\` script
  reaches the mission state through a second `net.dostring_in` on the frame it
  runs. The rule bounds steady-state cost, and eval is neither steady nor
  unattended — it is operator-initiated, capped at `eval_max_files_per_poll`
  per `eval_poll_interval_ms`, and carries its own row in the Section 10
  budget.
- Initialise the slots of `DCSBridge` one at a time. A whole-table `or {...}`
  never runs if another section created the table first. Every field then stays
  nil and the failure is silent.

### 5.4 Interface C is conditional on operator policy

Eagle Dynamics has gated `net.dostring_in` before and intends to gate it again.

The gate is configured in `Saved Games\<write dir>\Config\autoexec.cfg`. It
carries two keys, both documented in `API\Sim_ControlAPI.md`.

| Key | Names the states that may | Values in the shipped reference | Values in ED's announcement |
|---|---|---|---|
| `net.allow_unsafe_api` | make the call | `"userhooks"`, `"scripting"`, `"gui"` | the same three |
| `net.allow_dostring_in` | be addressed by it | `"mission"` | `"server"`, `"mission"`, `"scripting"`, `"gui"`, `"export"`, `"config"` |

**The two ED sources disagree, and the shipped one is the narrower.**
`API\Sim_ControlAPI.md` gives one `allow_dostring_in` value, `"mission"`, under
a comment saying it enables `net.dostring_in("scripting", "lua code")`. ED's
forum announcement of the gate lists six state names, `"server"` among them.
Neither source is dated against the other.

**Both keys, not one.** A configuration that sets `net.allow_dostring_in` and
leaves `net.allow_unsafe_api` unset fails closed on the caller side: the API is
not visible to a `Scripts\Hooks\*.lua` script at all. `doctor` reports both.

The bridge's own minimum is `"userhooks"` in `net.allow_unsafe_api` — the hook
driver makes the call — and **`"server"`, `"mission"` and `"gui"`** in
`net.allow_dostring_in`. That minimum is what `doctor` folds into the union it
prints (Section 5.4.2).

**`"gui"` is in the minimum because magnetic declination lives only there.**
`magvar` is present in the GUI state and in no other (measured, 2.9.29.27278),
so the hook driver reaches it with `net.dostring_in('gui', ...)` to fill the
declination on `CoordinateCalibration` (Section 6.3). Section 5.4.2 has
`doctor` print the union of the stated minimums, so a state this document does
not name is invisible to an operator: an enforcing build would drop the field
and the operator's own diagnostic would never name the state that fixes it. The
field is Route A only, because Route B has no such channel. Where the field is
absent for either reason, one log line names why.

**Ask for both because the two ED sources disagree and neither has been tested
against enforcement.** `"server"` is what shipping tools address and is
therefore the value with call-site evidence behind it: DCS-SRS's GameGUI hook
calls `net.dostring_in('server', ...)` to reach `trigger.action.*`, which
exists only in the mission-scripting state, and DCS Olympus requests
`{"server"}`. `"mission"` is the only value ED's shipped reference documents,
and LotATC requests `{"mission", "server"}` for the same reason this document
does. Asking for both costs nothing — Section 5.4.2 unions the lists anyway —
and survives ED re-enabling enforcement against either spelling. Enforcement
was reverted before this bridge existed, so **no measurement distinguishes the
two spellings**; that is what makes the union the correct ask rather than a
hedge.

DCS 2.9.18.12722 enforced it. A hotfix reverted that enforcement. ED states
they are considering a GUI toggle for a future update.

**Treat policy refusal as a live runtime condition.** Tools in this space ship
the `autoexec.cfg` step today, because users report needing it on post-revert
builds. The gate is therefore neither reliably absent nor reliably present.

Measured on 2.9.29.27278 with no `net.allow_*` entry in `autoexec.cfg`: the
call is not refused, and the `"server"` state evaluates chunks even at the main
menu with no mission loaded. The rule stands everywhere else: treat refusal as
a live runtime condition.

**ED considers the mechanism unsafe.** The reference shipped in the install
labels `net.dostring_in` **OBSOLETE and UNSAFE!!!** and marks enabling it for
the mission scripting state **DANGEROUS!!!**. Section 14.1 answers that
warning.

### 5.4.1 Two injection routes

The sim driver is one file on disk. Both routes load the same artifact. Both
run on every mission load, so both preserve the per-mission dev loop.

**Route A — bootstrap injection.** The hook driver resolves the sim driver's
absolute path with `lfs.writedir`, then injects a fixed-size chunk that loads
it:

```lua
-- built by the hook driver; <path> escaped with string.format('%q', path)
local ok, err = pcall(dofile, <path>)
return ok and 'OK|loaded' or ('OK|failed: ' .. tostring(err))
```

**The escaping is required, not stylistic: `net.dostring_in` carries no
arguments.** Positional values after the source do not reach the chunk, which
reads `select('#', ...)` as zero whether they are supplied or not — measured on
2.9.29.27278 against a probe proven to read arguments through a door that does
pass them, so the negative belongs to `net.dostring_in` and not to the probe. Every value
the loader passes in therefore crosses as text inside the chunk, and `%q` is
what keeps it text. Do not replace it with concatenation, and do not expect an
argument channel to appear.

**Sim driver source never crosses `net.dostring_in`.** Whatever the sim
driver's size, what crosses is a path and the load-time parameters below, never
the sim driver's own text. Tracebacks name the real file.

**The chunk is variable-size, and the Section 5.3 byte bound applies to it.**
Beyond the path it carries every value the loader passes in: the epoch id
(Section 6.3), the mission name (Section 6.10), the sim-driver-tier settings
and the `options` table (Section 13.2), and the enabled extension file list
(Section 6.10). The `options` table is adopter-authored and has no size bound
of its own, so the loader measures the assembled chunk and refuses to inject
above `bridge_return_max_bytes`, logging the size and taking Section 11's
injection-failure row. A silent truncation here would start a sim driver with
half its configuration.

**The prefix certifies only that the chunk ran.** A missing prefix means the
state was refused or unreachable. A `failed:` body means the chunk ran and the
sim driver raised while loading. The hook driver reads both and Section 11's
injection row covers both.

This needs `dofile` and `pcall` in the target state and nothing else. `dofile`
is a base-library function that performs its own file I/O in C. It does not
read the `io` table, and `MissionScripting.lua` does not remove it: the
sanitisation block touches only `os`, `io`, `lfs`, `require`, `loadlib` and
`package`. ED's own `dofile('Scripts/ScriptingSystem.lua')` runs above that
block, and `ScriptingSystem.lua` calls `dofile` five more times. Both `dofile`
and `pcall` are measured present in `"server"`, and the bootstrap is measured
working end to end (2.9.29.27278).

Route A is the default. It requires no edit to the DCS install tree. It depends
on operator policy for `net.dostring_in` itself.

**Fallback if `dofile` is unavailable.** Split the source into pieces below the
Section 5.3 safe chunk size. Inject each as a string literal accumulated in a
table. Then `loadstring(table.concat(...))` and call it. Name the chunk
`@SimDriver.lua` so tracebacks stay readable. This is documented, not built;
build it only if a DCS build breaks `dofile` in `"server"`.

**Route B — bootstrap.** The installer adds a `dofile` of the sim driver to
`Scripts\MissionScripting.lua`, positioned **before** the sanitisation block.
This is the same position and the same mechanism DCS-gRPC uses. The sim driver
then runs at the mission-scripting environment's own bootstrap, where it
captures `package.loadlib` while that name still exists and holds the reference
afterwards. **Route B loads without `net.dostring_in`.** The sim driver loads
the broker directly and never talks to the hook driver.

**That immunity covers loading and not running.** `net` is present in the
mission scripting state (measured, 2.9.29.27278; Section 7.2), so a Route B sim
driver holds `net.dostring_in` at runtime and is exposed to the gate from the
other side the moment it calls one. A Route B driver that stays inside its own
environment is genuinely independent of the gate; one that reaches out is not,
and Section 6.6 forbids it reaching out.

**The two routes load the sim driver into different environments** (measured,
2.9.29.27278; Section 5.1.2). Route A's injected chunks run in an environment
separate from mission scripts. Route B's `dofile` runs in the mission-script
environment itself. A Route B sim driver therefore shares globals with MOOSE,
MIST and the mission; a Route A sim driver does not. Section 6.1's collision
rule covers both cases.

**Route B cadence.** The mission state has no per-frame callback, and
`timer.scheduleFunction` runs on mission time, which freezes while the sim is
paused. The Route B sim driver therefore drives its Section 6.4 loop from a
self-rescheduling `timer.scheduleFunction` at a short interval, and that work
suspends for the duration of a pause: no drain, no dispatch, no subscription
evaluation until the sim resumes. Liveness does not depend on it — the hook
driver owns `shim.tick` under both routes (Section 5.2). Events that arrive
while paused accumulate in the sim driver buffer under
`sim_driver_buffer_max_records`.

**Both routes install the hook driver.** The route changes only how the sim
driver is loaded and how it reaches the broker. Hook-driver-side duties are
unaffected and belong to the hook driver in every configuration: the `DCS.*`
callbacks, player events, mission lifecycle, pause polling (Section 9.2),
`CallbackHz` (Section 9.3), `shim.tick` (Section 5.2), `EpochClosed` (Section
9.4), and the hook driver dispatch loop with its ring and tables (Sections 6.4
and 8.3). Under Route B the hook driver carries no sim driver traffic: the sim
driver polls its own ring directly, and the hook driver still polls the hook
driver ring — which is how `ReloadConfig` reaches the hook driver on a route
with no Interface C. The Section 5.3 batching rules then do not apply, because
there is no bridge in the path.

Route B is the framework-integration route, and the route for operators who
will not enable the API. It buys mission-environment reach — the measured
environment separation (Section 5.1.2) puts MOOSE, MIST and mission globals out
of a Route A sim driver's reach, and a Route B sim driver lives beside them —
at an operational cost: it edits a file under the DCS install tree, and every
DCS update overwrites that file, so the edit must be reapplied after each
patch.

**What a Route B install does without.** Every row is Route A's alone, and each
names the section that specifies it.

| Not available under Route B | Section |
|---|---|
| `ReloadSimDriver`; replacing sim driver code costs a mission reload | 6.9 |
| Mission-adjacent files | 6.10.6 |
| `eval\server\`; `eval\hook\` still runs | 7.2 |
| `DCSBridge.code.mission`'s `name` and `filename` | 6.10.5 |
| `CoordinateCalibration`'s mission date | 6.3 |
| `CoordinateCalibration`'s magnetic declination | 6.3 and 5.4 |

**The revert is silent.** The updater copies the pre-update file to
`_backup.NNN\Scripts\MissionScripting.lua` before replacing it, so the previous
edit is recoverable, but it is never reapplied. The sim driver simply stops
loading with no error anywhere. This is observed, not hypothetical: a July 2026
update on the measured install wiped a DCS-gRPC `dofile` line that
`_backup.007` still holds. `doctor` checks for the `dofile` line and reports
its absence.

Route B exists for two reasons. An operator who will not or cannot enable the
API still has a working install. And a sim driver that must delegate to a
framework the mission loads must live where the framework lives (Section 2).

Do not delete the sanitisation block. Placing the `dofile` before it leaves
mission scripts sandboxed, which is the whole point of the block.

The sim driver reports which route loaded it in `SimDriverLoaded`. A consumer
must not depend on the answer.

### 5.4.2 `autoexec.cfg` is a merge, not a write

Every tool using this API wants the same file and the same two keys, and their
state lists differ. Observed in the wild: one widely deployed tool requests
`net.allow_dostring_in = {"server"}`, another requests `{"mission", "server"}`,
and a third modifies the file automatically.

**An installer that writes this file destroys another tool's configuration
silently**, and the failure surfaces as "your tool broke my other tool."

- Never write `autoexec.cfg`. Merge into it.
- **Union both lists, under both keys.** Never replace either.
  `net.allow_unsafe_api` and `net.allow_dostring_in` are separate keys and a
  merge that touches one and not the other fails closed. Removing a state
  another tool needs is the same defect as overwriting the file.
- Preserve every unrelated setting in the file.
- Section 13 documents the merge as an explicit install step. It is the one
  part of installation that is not covered by extracting an archive.
- `doctor` reads the file, reports which zones each present tool needs, and
  prints the **union** the operator should have. **It never prints this
  bridge's own minimum**, which would break another tool if an operator pasted
  it over the file.

### 5.4.3 ED documents a replacement

The same reference states: "There's no need for net.dostring_in anymore. You
can return values from a_do_script() mission scripting API directly", with the
example `local a, b, c = a_do_script("return 1,2,3")`.

`a_do_script` is a mission trigger action. It is reachable from a loaded
mission, not from the hook state at will, which is why Interface C does not use
it. On 2.9.29.27278, `a_do_script` is not visible from the hook state:
`type(a_do_script)` is nil at the menu and with a mission loaded, so it cannot
be called directly from there.

**A two-hop path exists, and it is measured.** `net.dostring_in('mission', ...)`
answers, `a_do_script` is a global in the `mission` state, and a chunk handed to
it runs in the mission scripting state — the one holding `env`, `world`,
`trigger`, `timer` and `coord`, and the one a Route B sim driver lives in
(measured, 2.9.29.27278). Section 5.4 records that `API\Sim_ControlAPI.md` names
exactly one value for `net.allow_dostring_in`, and that value is `"mission"`, so
ED's own reference documents the zone the path addresses.

**It costs more than Route A rather than less.** It uses `net.dostring_in`, so
it needs both halves of the gate, and `mission` in the addressable list on top
of the zone Route A already asks for: two zones rather than one. Its calling
convention is also its own: returns arrive shifted by one with the last value
dropped, so a single returned value is lost unless a sacrificial second one
follows it.

**This document specifies no such route.** It is recorded because a reader
weighing Route B's framework reach should know that reach is not Route B's
alone.

If instead a later build exposes `a_do_script` to the hook state directly, that
is a different third route, and `net.allow_unsafe_api` is the only half of the
gate it would need — `net.allow_dostring_in` governs `net.dostring_in` targets
and would not apply. That one would be cheaper than Route A. **Nothing in this
document assumes a third route.**

---

## 6. The sim driver

The sim driver is the only component that runs inside the simulation. It runs
in interpreted Lua, on the thread that advances the sim. An unguarded error
there ends the session, and a slow loop affects every connected player. It is
specified at the same rigour as Interface A.

### 6.1 One global, four slots

**The sim driver claims exactly one global, and its name is `DCSBridge`.** The
environments are separate (Section 5.1.2; measured, 2.9.29.27278): a Route A
sim driver does not share globals with mission scripts — but it shares the
injected environment with every other tool that injects through
`net.dostring_in`, and a Route B sim driver is loaded into the mission-script
environment itself, beside MOOSE, MIST and the mission's own globals. Every
global the sim driver creates is therefore still a collision risk on at least
one route, and a short or generic name is a collision waiting to happen. One
long, distinctive table is the whole mitigation.

**Refuse to load on a collision.** The sim driver stamps its table with
`DCSBridge.__dcsbridge = STATE_VERSION` when it creates it. At load, if
`_G.DCSBridge` exists without that field, log the collision and stop. Do not
merge into it and do not overwrite it. A silent merge produces a failure nobody
can diagnose from either side. The same field is what a reload reads to find
the table it must adopt, so one marker serves both. A stamped table is also
what a **new mission's load** finds: the injected environment survives a
mission reload (Section 9.4), so the previous epoch's `DCSBridge` is still
there. A load that finds its own stamp treats it as epoch leftovers — release
anything `EpochClosed` missed, discard `state` and `code`, rebuild — never as a
collision.

The sim driver separates code, bookkeeping and world resources. The split is
structural because reload depends on it.

| Slot | Holds | On reload |
|---|---|---|
| `DCSBridge.code` | Handlers, generated emitters and decoders, dispatch table, subscription predicates, the Section 6.10 registration surface and its registration maps, and the merged options table | Replaced every time |
| `DCSBridge.state` | Unit registry, active subscription set, recent idempotency keys | Preserved on warm, discarded on cold |
| `DCSBridge.resources` | DCS-owned handles: spot handles, `timer.scheduleFunction` ids, the event handler table | **Released, never discarded.** See below. |
| `DCSBridge.shim` | The Interface A put-call surface | Set once at load. Never replaced, never released. |

**Generated emitters live in `DCSBridge.code.emit`, not beside it.** An emitter
is generated from the schema, and the schema changes with the sim driver, so an
emitter is code. A handler does not call `emit` directly; it calls the buffered
`send` wrapper in Section 6.10. Code is replaced on every reload. An emitter
left outside `DCSBridge.code` survives a reload that changed the schema, and
then writes retired field numbers that a consumer decodes as the wrong fields
or skips entirely. The same applies to generated decoders and to the class
table.

**The hook-driver-side generated file is the deliberate exception, guarded by
the schema hash instead of by reload.** A hook-driver-targeted topic must work
when no sim driver is loaded, so its generated tables and code live in
`HookDriver.gen.lua` (Section 8.3), outside `DCSBridge.code`, and a reload does
not replace them. The stale-schema hazard above is answered by the hash check
in Section 8.3: a hook driver file and a sim driver built from different
generator runs are detected and reported rather than silently mis-routed.

**`DCSBridge.shim` is the seam, not the broker.** Under Route B, and under
Route A where the sim driver reaches the broker directly, it is the table
`package.loadlib` returned. Under Route A otherwise it is a hook-driver-backed
object offering the same calls, per Section 5.1.2. A generated emitter calls
`DCSBridge.shim.begin` either way and never learns which it got — that is
Section 1.4's seam doing its job.

It is deliberately not held in `DCSBridge.resources`. That slot exists to
release DCS-owned objects on reload, and the put-call surface is neither
DCS-owned nor released.

**Discarding a DCS-owned handle leaks it into the world.** `Spot` has no
enumerator — there is no `world.getSpots`, and `world.searchObjects` does not
return them. A discarded spot handle is a laser that keeps lasing for the rest
of the mission with no way to destroy it. A discarded `timer.scheduleFunction`
id for a self-rescheduling function is a function that runs forever with no way
to cancel it. A one-shot — a callback returning nil — costs at most one more
invocation. Periodic sim driver work is the self-rescheduling kind. The reload
release step (Section 6.9 step 3) runs on **every** reload, warm or cold, and
releases both.

**The event handler table is the exception inside `DCSBridge.resources`.** It
is registered once and survives every reload. The reload release step never
touches it. Only epoch teardown at `EpochClosed` releases it. See Section 6.2.

### 6.2 One permanent event handler

Register one handler table, once. Never touch the registration again.

```lua
-- created and registered on the first frame (Section 6.3);
-- lives in DCSBridge.resources, never in DCSBridge.code
DCSBridge.resources.handler = DCSBridge.resources.handler or {
  onEvent = function(_, event)
    local ok, err = pcall(function() DCSBridge.code.on_event(event) end)
    if not ok then DCSBridge.code.log_handler_error(err) end
  end,
}

if not DCSBridge.resources.registered then
  world.addEventHandler(DCSBridge.resources.handler)
  DCSBridge.resources.registered = true
end
```

`onEvent` reads `DCSBridge.code` at call time, so replacing `DCSBridge.code`
replaces the behaviour with no registration churn.

**The handler must not live in `DCSBridge.code` or in `DCSBridge.state`.**
`world.removeEventHandler` removes nothing unless given the identical table:
`world.eventHandlers` is keyed by the handler table's own identity. Registering
the same table twice is therefore a no-op, and only a *new* table doubles
dispatch. A handler held in a slot that a reload replaces or discards can never
be removed, and the new code registers a second one. Every event then
dispatches twice, which appears as duplicate records rather than as a crash.

Registering the handler once removes event deregistration from reload entirely.
There is no window in which zero or two handlers are registered.

### 6.3 Lifecycle

| Stage | Trigger | Work |
|---|---|---|
| Load | Route A: `onMissionLoadEnd`. Route B: mission-environment bootstrap. | Read `mission_scripting_sandbox_level`. Detect whether the broker is directly reachable. **Adopt the sim-driver-tier settings the loader passed in.** Build the four slots of `DCSBridge` field by field. Emit `SimDriverLoaded` with the route. |
| First frame | First drain after load | Register the permanent handler. Run the binding probe from Section 4.2. Emit whatever epoch-scoped content the loaded sim driver defines. |
| Steady state | Each frame | Section 6.4. |
| Reload | `ReloadSimDriver`, at a frame boundary only | Section 6.9. |
| Calibration | `onMissionLoadEnd`, after `EpochOpened` and before injection | Derive the projection, read the date and the declination, emit `CoordinateCalibration`. HOOK §10. |
| Teardown | The hook driver observes `onSimulationStop` | Release every handle in `DCSBridge.resources`, including the event handler. **The hook driver emits `EpochClosed`**, because the sim driver may already be gone. |

The hook driver emits `EpochOpened` at `onMissionLoadEnd`, before it injects.
The sim driver never emits it. See below.

**Epoch ownership. Boundaries belong to the hook driver. Contents belong to the
sim driver.** The hook driver allocates the epoch id at `onMissionLoadEnd`,
emits `EpochOpened` there, and passes the id to the sim driver at load. It
emits `EpochClosed` at `onSimulationStop`, because the mission-scripting state
may be torn down before the sim driver gets a frame. Both boundaries are the
hook driver's, and **both are emitted whether or not a sim driver ever loads**
— which is what makes the Section 9.5 no-sim-driver case a working
configuration rather than a silent one.

Every field of `EpochOpened` is hook-driver-reachable: the epoch id it
allocated, the mission-start wall-clock time from `os.time()`, the mission time
from `DCS.getModelTime()` (Section 5.2), the terrain name from
`DCS.getMissionTheatre()`, the mission name from `DCS.getMissionName()`, and
the deployment pair from `DCS.isServer()` and `DCS.isMultiplayer()`. Each
`DCS.*` call is made alone and guarded, per Section 4.3.

**The mission name identifies the epoch, which is why it sits here.** An epoch
is one mission's lifetime on the wire (Section 9.4), so the mission that
defines it names it. It is reachable without a crossing, like the terrain, and
unlike the mission date. **It is not the `.miz` name** — a mission flown from
the Mission Editor reports `tempMission` (measured, see Provenance). **Carry no
mission filename**: `DCS.getMissionFilename()` returns an absolute path that
discloses the operator's directory layout, which Section 14.7 refuses.

**The deployment pair tells a consumer which records can ever arrive.** The two
booleans are not redundant and neither alone is enough: `DCS.isServer()` is
true on a single-player host as well as a server (measured, 2.9.29.27278), so
it means "this process is authoritative for the sim" rather than "a server is
running". Read together, `isServer` with `isMultiplayer` false is a
single-player host, where no remote player ever connects to produce a player
record and `net.banlist_get` answers nil (HOOK §6). A consumer that branches on
one boolean gets that case wrong.

**The pair is constant for the process and repeats every epoch.** That costs
nothing — `EpochOpened` is retained and replayed once per connection (Section
5.2) — and it saves a topic. The one gap it leaves: a consumer connecting
before any mission has loaded receives `MissionLoadBegan` and no `EpochOpened`,
so it learns the pair at the first load rather than at connect. The handshake
does not close that gap on purpose; Section 5.2 discloses nothing about the
mission to an unauthenticated peer.

**The theatre is populated at `onMissionLoadEnd`, measured across three
terrains.** Caucasus, Syria and GermanyCW each returned their name there, on a
warm load and a cold one. ED's own calls to `DCS.getMissionTheatre()` are from
GUI code rather than from a lifecycle callback, so precedent did not settle
this and a measurement had to. It is in fact readable a callback earlier still,
at `onMissionLoadBegin`, carrying the incoming mission's value rather than the
previous mission's (see Provenance). `EpochOpened` is emitted at
`onMissionLoadEnd` and needs no contingency.

**The theatre is gone by `onSimulationStop`.** `DCS.getMissionTheatre()`
returns nil there while `DCS.getMissionName()` and `DCS.getMissionFilename()`
still read (measured, three epochs). `EpochClosed` is emitted from that
callback and carries no terrain, so nothing breaks today. **Add no field to
`EpochClosed` that the theatre would have to supply.** A consumer that wants
the terrain at a boundary has it from `EpochOpened` and from
`CoordinateCalibration`, both of which are retained (Section 5.2).

**The coordinate calibration set is the bridge's own record, and the hook
driver emits it.** Those are two separate facts and both matter.

**It is the bridge's** because Section 8.2 makes every position field in every
record carry DCS-local coordinates — the bridge's own records, the sim driver
built-ins', and an adopter's alike. The calibration set is the key to reading a
convention this document imposes on everybody, so it is broker metadata in the
same sense the schema hash is, not a description of anything that happened in
the mission. Leaving it to a sim driver would mean an adopter who replaces the
built-ins ships records whose coordinates nothing can convert, which is a trap
rather than a design. It is `CoordinateCalibration`, class `LIFECYCLE`, in the
`dcs.bridge` package, and Section 1.2 enumerates it.

**The hook driver emits it**, from `terrain.convertMetersToLatLon` in its own
state. The hook driver has no `coord` table — `coord.LOtoLL` and its siblings
exist in the mission-scripting state and nowhere else — but it does not need
one.

**Measured, 2.9.29.27278, Caucasus.** `terrain.convertMetersToLatLon(x, z)`
returns latitude and longitude and **agrees with `coord.LOtoLL` to every
printed digit** at three points, including an off-origin point with mixed signs
that would expose an axis swap or a sign flip:

| x, z | Both functions return |
|---|---|
| 0, 0 | 45.129497060329, 34.265515188456 |
| 100000, 200000 | 45.971166157788, 36.866202976754 |
| -50000, 75000 | 44.665183098022, 35.201462257523 |

The two agree because they are the same projection: any residual is the float32
grid rather than the arithmetic. Handed a coordinate float32 represents exactly
they agree to twelve decimal places, and handed one it cannot they appear to
differ by a few millimetres.

`coord.LOtoLL` additionally returns an altitude the hook-driver-side call does
not. Missing arguments default to zero and a third argument is ignored, so the
signature is `(x, z)`. Its mission-scripting inverse `coord.LLtoLO` returns one
point table rather than three numbers. No `terrain.Init` call is needed: the
function works once a terrain is loaded and raises a catchable `"no terrain"`
at the menu. **Never call `terrain.Init`, `terrain.Create` or
`terrain.Release`** to change that — they mutate loaded terrain state and
nothing here needs them.

**So the no-sim-driver case loses no coordinate conversion.**
`CoordinateCalibration` arrives on every configuration, like both epoch
boundaries. The sim driver computes nothing for it.

```proto
message CoordinateCalibration {
  option (record_class) = RECORD_CLASS_LIFECYCLE;

  string                 terrain            = 1;
  optional MissionDate   date               = 2;  // Route A only
  optional Projection    projection         = 3;  // absent off-family
  repeated Verification  point              = 4;  // HOOK §10 fixes the count
  DeclinationStatus      declination_status = 5;
}

message MissionDate {
  // Proleptic Gregorian, no timezone. env.mission.date, which is the sim's
  // own calendar and carries no zone.
  int32 year  = 1;
  int32 month = 2;
  int32 day   = 3;
}

message Projection {
  int32  central_meridian = 1;  // degrees, an odd multiple of 3
  double false_easting    = 2;  // metres
  double false_northing   = 3;  // metres
  string proj             = 4;  // built from the three above
}

message Verification {
  double          x           = 1;  // DCS-local, north
  double          z           = 2;  // DCS-local, east
  double          latitude    = 3;  // WGS84 degrees
  double          longitude   = 4;  // WGS84 degrees
  optional double declination = 5;  // radians, true to magnetic
}

// NEVER EMITTED at 0. Extensible per Section 8.2.
enum DeclinationStatus {
  DECLINATION_STATUS_UNSPECIFIED    = 0;
  DECLINATION_STATUS_PRESENT        = 1;
  DECLINATION_STATUS_POLICY_REFUSED = 2;  // "gui" not granted
  DECLINATION_STATUS_ROUTE_B        = 3;  // no channel to reach it
  DECLINATION_STATUS_CALL_FAILED    = 4;  // magvar raised, guarded
}
```

**Presence is one bit per group, never a sentinel.** `projection` is a
submessage rather than three loose fields because a false easting of zero is a
legitimate value and Section 8.4 forbids a shape where absent and zero look
alike. A consumer tests whether `projection` is set; it never inspects a
number to guess.

**The record publishes a projection, not a conversion service.** Every measured
DCS terrain projects with one family: transverse Mercator on WGS84 with `k_0 =
0.9996`, a central meridian at an odd multiple of 3 degrees, and a per-terrain
false easting and northing. `CoordinateCalibration` therefore carries the
terrain name, the mission date, those three derived parameters, a PROJ string
built from them, and the verification points HOOK §10.2 fixes at four. A consumer feeds the
PROJ string to any PROJ binding and converts in bulk with no further traffic.
**Publish no EPSG code**: a re-origined UTM zone has none and can be given
none. HOOK §10 specifies the derivation and the hook driver performs it.

**Each verification point carries an optional magnetic declination.** The value
comes from `magvar` in the GUI state, which the hook driver reaches under
Section 5.4's `"gui"` minimum. It is per point rather than one scalar per
theatre because it moves **3.84 degrees across Caucasus** — 4.957 degrees at
the terrain's south-west corner against 8.797 at its north-east (measured,
2.9.29.27278, June 2016 epoch). That is an order of magnitude beyond the
heading precision a consumer will quote, and the variation is not uniform
across the map, so a consumer must read the value at the point it cares about
rather than interpolate. HOOK §10.3 carries the measurement. Absence is
meaningful, so the field is `optional` per Section 8.4, and the verification
points are a repeated message field per Section 5.1.

**Declination is dated, so the record carries the mission date.** `magvar`
answers for an epoch seeded by `magvar.init(month, year)`, and DCS seeds it
from the mission date at load. At one point on Caucasus the answer moves from
4.8732 to 6.6394 degrees between a 1990 mission and a 2016 one — about 0.068
degrees a year, which is why a stale epoch is a quietly wrong answer. The
hook driver reads `env.mission.date` from the mission-scripting state, seeds
`magvar` from its `Month` and `Year` explicitly rather than trusting the
default, and publishes the same date on this record.

**The date and the declination are both Route A only, and for the same
reason.** Each needs a `net.dostring_in` crossing the hook driver has only
under Route A — the declination into the `"gui"` state for `magvar`, the date
into the mission-scripting state for `env.mission.date`. That is two crossings
per epoch, not one. Under Route B the hook driver makes no such call
(Section 5.4.1), so both fields are absent and `declination_status` reads
`ROUTE_B`. Where `env.mission.date` is absent or does not parse as a date, the
hook driver omits the date, omits the declination, sets `declination_status` to
`CALL_FAILED`, and logs once: seeding `magvar` from a nonsense epoch would give
a quietly wrong answer, which is worse than none.

**Add the date to no other record.** Section 6.3 requires every field of
`EpochOpened` to be hook-driver-reachable without a crossing, and the mission
date is not.

**A terrain outside the family is detected, not assumed away.** The hook driver
checks the derived parameters against a point it did not derive them from.
**HOOK §10 fixes the residual threshold, the number of verification points and
the rule that chooses them**, because HOOK §10 specifies the derivation. This
document fixes only what crosses the wire.
Where the check fails, the record carries the verification points, omits the
parameters and the PROJ string, and one log line records the residual. A
consumer that finds no parameters falls back to the sim driver's
`ConvertCoords` (SIM §4). Only Caucasus has been derived at runtime, so
deriving per epoch and checking is what makes the remaining terrains safe to
assume.

**The record describes the current epoch. Hold it for the epoch and replace it
at the next one.** The projection is a property of the theatre and the
declination is a property of the theatre and the date together, but neither
fact makes a cross-mission cache workable: Section 5.2 retains exactly one
record per topic, replaced in place, so a consumer can never obtain any
terrain's calibration but the current one, and no request exists for an earlier
one. Keying a longer-lived cache on terrain name alone would also serve a stale
declination to every mission flown at a different date on a terrain already
seen.

**A retained calibration from a closed epoch is discarded like any other
epoch-scoped record.** The envelope carries the epoch the record was emitted
in, so Section 9.4's rule reaches it unchanged. A consumer that connects
between `onMissionLoadBegin` and `onMissionLoadEnd` receives the previous
mission's calibration in the retained replay, together with the `EpochClosed`
that voids it, and therefore **holds no calibration until the new epoch
opens**. It converts no position in that window. That is the correct outcome
rather than a gap: the retained record describes a terrain the next mission may
not use, and applying it would produce a position that looks reasonable and is
tens of kilometres wrong — the plausible-wrong-answer failure Section 8.2 exists
to prevent.

The sim driver holds no state across an epoch. Every unit handle is void at
`EpochClosed`.

### 6.4 Bounded work model

**A frame reaches the sim driver differently on each route.** Under Route A the
hook driver drives the loop: it makes one `net.dostring_in` call per frame from
`onSimulationFrame` (Section 5.3), and that call runs the stages below and
returns the drain. Under Route B the sim driver drives its own loop from a
self-rescheduling `timer.scheduleFunction`, which suspends while the sim is
paused (Section 5.4.1). "Each frame" below means one invocation of that loop,
whichever drives it.

Each frame the sim driver performs, in this order:

**The buffer is bounded between frames, not by a stage.** Events accumulate in
the sim driver between drains, and a drain happens only when a frame fires.
Frames stop for seconds at a time, and nobody knows for how long at worst.

   **The buffer is not sized to survive a stall.** It cannot be. The stall
   distribution is unmeasured. A buffer sized for an unknown tail is either
   unbounded or arbitrary.

   It holds at most `sim_driver_buffer_max_records`, a figure chosen for
   affordable memory. Beyond that it drops the oldest and counts them.

   `sim_driver_buffer_dropped_total` is the real instrument, and **it should
   never move.** The largest measured running-phase stall is 282 ms
   (Provenance), and the largest measured event rate is 23.3 per second at 120
   vehicles, so a worst-case stall buffers on the order of ten records against
   a default of
   8192. If this counter moves, either the emission rate is far above anything
         measured or a stall far outside the distribution occurred; both are
         worth investigating rather than tuning away.
1. Emit buffered events, up to `drain_max_records` and `drain_max_bytes`.
2. Dispatch inbound commands, up to `dispatch_max_commands`.
3. Evaluate due subscriptions, up to `subscription_max_evals`.
4. Update due spots, up to `spot_max_updates`.
5. Sample due tracked weapons, up to `weapon_max_samples`.
6. Advance one resync slice of `resync_slice_records`.
7. Apply a pending reload, if one is queued. See Section 6.9.

Every stage that does bulk work has a configured cap. Stage 7 applies at most
one reload per frame and needs none. Defaults and their basis are in Section
13.1. **Work above a cap is deferred to the next frame, never dropped and never
run late in the same frame.** A deferred stage increments a counter.

The order is not arbitrary. Stage 1 precedes stage 6 for the reason in Section
6.8. Stage 7 is last so a reload never lands mid-frame.

**The hook driver runs a second, smaller loop.** This model is the sim
driver's; the hook driver's loop polls the hook driver ring and dispatches to
the hook-driver-side handlers, at most `hook_driver_dispatch_max_commands` per
invocation. It runs from `onSimulationFrame`, `onPlayerConnect`,
`onPlayerDisconnect`, `onPlayerChangeSlot` and `onMissionLoadEnd` — the
player-event callbacks so a moderation handler is not waiting out a menu-phase
frame gap (a measured 8.33 s at the menu, Section 5.2). It dispatches nothing
between `onMissionLoadBegin` and `onMissionLoadEnd`. Work above the cap defers
to the next invocation and increments `hook_driver_dispatch_deferred_total`.
Frames fire at the menu at a measured 68 Hz, and in a hosted multiplayer
mission at a measured 72 Hz (2.9.29.27278).

### 6.5 Error containment

The outer `pcall` in Section 4 keeps the session alive. It does not keep the
sim driver working.

- **Load containment and dispatch containment are separate.** An extension file
  that fails to compile or raises while loading is logged, counted, and
  skipped; the remaining files load and the sim driver runs (Section 6.10). The
  rules below govern a handler that raises once it is registered.
- Each handler runs inside its own `pcall`. A handler that raises is logged and
  **disabled for the remainder of the epoch**, not retried each frame.
- A disabled handler emits a record naming itself once.
- `handler_failures_per_epoch` failures in one epoch **disable the sim driver
  for that epoch**, not the bridge. The hook driver keeps running, lifecycle
  records keep flowing, and the next mission load starts a clean sim driver.
  The operator kill switch in Section 11 is a separate, manual control.
- Section 4.2 still applies: `pcall` does not contain an access violation, so
  containment is a degradation strategy and not a safety guarantee.

### 6.6 What a handler may call

- Anything in the `"server"` state that survived the Section 4.2 probe, with
  arguments it has exercised.
- The generated `send` wrappers, which buffer a record for the next drain.
  **Not the generated emitters.** An emitter writes to the broker at the moment
  it is called, so a handler that calls one bypasses
  `sim_driver_buffer_max_records` and the Section 6.4 stage 1 caps. `emit` is
  drain-side and runtime-owned. See Section 6.10.
- Nothing in the `hook` state. Nothing under `DCS.*`. See Section 4.3.
- No blocking call of any kind.

A handler that must reach the hook state does so by emitting a record and
letting the hook driver act on it.

### 6.7 Subscriptions

A subscription is a predicate the sim driver evaluates locally and a record it
emits when the predicate fires.

- **Every subscription declares an evaluation interval.** Per-frame is a
  choice, not a default.
- Active subscriptions are capped at `max_subscriptions`. The sim driver
  rejects a subscribe command above the cap, with a reason, never silently.
- The sim driver counts evaluation cost per subscription and reports it. See
  Section 12.
- A subscription is discarded at `EpochClosed`. The consumer re-subscribes.

**Spots are subscriptions with a handle.** A spot declares an interval. It
counts against a cap. The sim driver updates it on that schedule. `EpochClosed`
destroys it.

A spot differs from a subscription in two ways. It owns a DCS object that must
be released. It emits a record when its source or target ceases to exist. See
SIM §6.

Subscription evaluation is the largest sim driver cost at scale and the one
most likely to be blamed on the bridge. Instrument it before optimising
anything else.

### 6.8 Resync

Resync answers a consumer that joins mid-mission and knows nothing about the
world. It does not carry epoch context: the consumer already received the
current epoch's `LIFECYCLE` records from the broker's retention (Section 5.2)
before its first live record. The scan supplies world state only.

**The consistency rule:**

> Resync is a live scan. Records continue to be emitted normally throughout. A
> consumer applies a resync record only where it holds no record with a higher
> `seq` for that identity. `EpochClosed` during a resync invalidates the whole
> scan.

**The sim driver constraint that makes the rule work:** within a frame, the sim
driver emits buffered events **before** it emits any resync slice.

Without that ordering, an event observed early in a frame and emitted late
could carry a higher `seq` than a resync record reflecting later state. "Higher
`seq`" would then stop meaning "newer observation." The broker cannot supply
this guarantee. The sim driver must.

`ResyncBegan` and `ResyncEnded` bracket the scan. It spreads across frames
under the Section 10 budget.

The bridge defines the trigger and the brackets; the record set is the sim
driver's. A sim driver that implements no resync answers `Resync` with a
`CommandAck` carrying outcome `REFUSED` (Section 8.5.3) — its decoder is
generated from the bridge schema, so it can always decode and refuse the
command.

**A resync emits the record types a live update would emit**, one per identity
in the scan — the same shapes, from the same emitters. The consumer's apply
path is identical for live and resync records, which is what lets the
higher-`seq` rule work unmodified. The concrete record set is the sim driver
built-ins', listed in their coverage document (PLAN §4), not the bridge's.

**Ordering is sufficient for state, not for transitions.** The `seq` rule
orders successive states of one identity. A transition record may arrive after
a resync slice that already reflects the transition's outcome. A destruction, a
capture and a slot change are all transitions. **Every transition record must
be idempotent at the consumer.** Applying a destruction to a unit already
absent is a no-op, not an error. The broker cannot supply this, and the
ordering rule does not cover it.

### 6.9 Reload

Reload replaces `DCSBridge.code` on a running mission. It exists so an operator
can change behaviour without ending everyone's flight.

**Warm reload keeps `DCSBridge.state`. Cold reload discards it.**
`DCSBridge.resources` is released either way, per Section 6.1.

The sim driver declares `STATE_VERSION`, which versions **the shape of
`DCSBridge.state` and nothing else**. Incoming code declaring a different
version forces a cold reload. New code reading an old state shape is how silent
corruption happens instead of clean failure.

**Sequence, with rollback:**

1. The hook driver reads the new source from disk — the runtime,
   `SimDriver.gen.lua`, and every enabled sim-driver-side extension file
   (Section 6.10) — and re-reads the sim-driver-tier settings from
   `Config\DCSBridge.lua`. All of it travels together, so a reload can change
   behaviour and the caps that bound it in one step.
2. The hook driver asks the target state to **compile without executing**, file
   by file. A syntax error in any one of them dies here, with the file named.
   The running sim driver never learns it happened.
3. The sim driver releases every handle in `DCSBridge.resources` except the
   event handler: destroy spots, cancel scheduled functions.
4. The hook driver loads the new code.
5. New init adopts `DCSBridge.state` when `STATE_VERSION` matches. Otherwise it
   discards it and rebuilds the registry.
6. **On any raise in step 4 or 5, the hook driver re-injects the previous
   source**, which it kept in memory for this purpose.
7. The sim driver emits `SimDriverReloaded`.

**Step 3 releases. It does not discard.** A cold reload that forgets a spot
handle orphans a laser for the rest of the mission. See Section 6.1.

**There is no event deregistration step.** Section 6.2 explains why.

**Reload runs at a frame boundary only**, as stage 7 of Section 6.4. Never from
inside an event handler: `world.onEvent` is a live `pairs` traversal, and
Section 4.1 gives the rule.

**The reload issues no new epoch.** The world did not change. Unit references
remain valid and `seq` continues.

`SimDriverReloaded` is a `LIFECYCLE` record:

| Field | Meaning |
|---|---|
| `state_preserved` | False means a cold reload happened |
| `state_version` | The version now running |
| `code_sha256` | Hash of the source now running |
| `subscriptions_dropped` | Count, so a consumer can assert rather than guess |
| `spots_dropped` | Count |

**The reload set is whole.** A reload replaces every file in the set or none of
them, and the rollback in step 6 restores the whole previous set. A
half-applied set would leave one file's registrations addressing another file's
keys.

**On a cold reload the consumer re-establishes subscriptions and spots.** The
record is a partial resync trigger, not a notice.

**Trigger.** A `ReloadSimDriver` command, with its own capability separate from
`command`. A consumer that can reload can run whatever source is on disk.

**`ReloadSimDriver` is Route A only.** Under Route B no component can execute
this sequence: the hook driver has no channel to the sim driver. A
`ReloadSimDriver` received on a Route B install is acknowledged with an error
naming the route. Replacing Route B sim driver code costs a mission reload —
one more row in Section 5.4.1's table of what Route B does without.

**The hook driver cannot reload itself.** `DCS.setUserCallbacks` has no
deregistration counterpart, and a second call adds a callback set rather than
replacing one — measured on 2.9.29.27278, where three separate calls each
registered `onSimulationFrame` and all of them kept firing. Section 13's loader
and payload split lets the payload change on disk, but it takes a DCS restart
to take effect.

### 6.10 The extension model

An adopter changes sim driver behaviour by adding files, never by editing a
shipped one. This section defines where those files live, in what order they
load, and what they may call.

**Ownership is the one rule.** Section 13 lists the bridge tree's files. A
bridge release overwrites every one of them and touches nothing else. `buf
generate` rewrites the two `.gen` files and touches nothing else. An adopter
writes nowhere under `Mods\services\DCSBridge\`.

**Adopter files live in the operator's extension directories**: `<write
dir>\DCSBridge\simdriver.d\` for the sim driver side and `<write
dir>\DCSBridge\hookdriver.d\` for the hook driver side. The installer creates
neither. An operator creates one to opt in and deletes it to opt out, and a
missing directory is not an error — the Section 7.2 eval-directory pattern.
Both sit under the write directory, so enumeration uses the Section 7.3
mechanism already in service.

#### 6.10.1 Load chain

Nothing changes at the injection boundary. Route A's bootstrap chunk and Route
B's `dofile` load exactly one file, `SimDriver.lua`. The runtime loads the rest
from disk, in this order:

1. `SimDriver.gen.lua`. Its tables register through `shim.classes`,
   `shim.routes` and `shim.caps`, and its `schema_sha256` rides the Section 8.3
   hash check.
2. `SimDriver.builtin.lua`, unless `sim_driver_disabled_files` names it.
3. Every `*.lua` in `<write dir>\DCSBridge\simdriver.d\`, where that directory
   exists, in ascending name order.
4. The runtime then registers the permanent event handler (Section 6.2) and
   starts the Section 6.4 loop.

Names order the files within one directory. The list above orders the sources.
A later file overrides an earlier one by key. Within an extension source,
`*.gen.lua` files load before the rest.

The shipped files are a fixed, named list, and nothing enumerates the bridge
tree. Only the extension directory is enumerated: under Route A the hook driver
lists it and passes the list in the injection chunk, under Route B the runtime
lists it at bootstrap.

The hook driver mirrors the chain at DCS start: `HookDriver.lua` loads
`HookDriver.gen.lua`, then `HookDriver.builtin.lua`, then `<write
dir>\DCSBridge\hookdriver.d\` in name order. Hook-driver-side files load once
per DCS process, and there is no hook driver reload (Sections 3.1 and 6.9).

**Only the runtime touches the platform's registration points.** One
`world.addEventHandler`, one `DCS.setUserCallbacks` set, and every
`missionCommands` handle belong to the runtimes. An extension file that calls
one directly is a defect: Section 6.2 depends on the handler table being
registered exactly once, and a handle nothing owns is the leak Section 6.1
describes. Extension files register through the surface below. A mission that
wants its own F10 menu uses its own environment's `missionCommands`, as ever.

#### 6.10.2 The registration surface

The runtime defines the surface before it loads any extension file, so the
surface exists whether or not any extension file does. The sim driver built-ins
are a customer of it, never their provider: disabling or deleting the built-ins
removes none of it.

```lua
-- events: many handlers per event id, dispatched in registration order
--   fn(event) -> nothing
DCSBridge.code.on(key, event_id, fn)

-- inbound topics: exactly one owner per topic; the topic comes from the
-- generated constants, never from a literal
--   fn(conn_id, msg) -> nothing
DCSBridge.code.command(key, DCSBridge.code.topics.<Message>, fn)

-- buffered emission, one generated member per outbound message: the call
-- buffers the record and the Section 6.4 stage 1 drain emits it.
-- Arguments are the message's fields in declaration order.
DCSBridge.code.send.<Message>(...)

-- the same, for a reply or acknowledgement: connection id first, mirroring
-- the emitter in Section 8.3. This is how a handler answers a command.
DCSBridge.code.send_to.<Message>(conn_id, ...)

-- overrides, by key
DCSBridge.code.off(key)              -- remove a registration
DCSBridge.code.replace(key, fn)      -- substitute its function
DCSBridge.code.wrap(key, fn)         -- compose: fn receives the previous
                                     -- function and the arguments, and may
                                     -- decline to call it
```

**A `command` handler receives `(conn_id, msg)`.** `conn_id` is the connection
the command arrived on, which Section 5.1's `poll` returns; `msg` is the table
the generated decoder produced. It returns nothing — the runtime holds a table
from topic to registration and dispatches through it, and a handler's answer
travels by `send_to`, never by a return value.

**`send_to` is how a handler answers.** Section 8.5.3 requires exactly one
point-to-point record per command that reaches Lua, and Section 8.3 puts the
connection id on the emitter for a reply or acknowledgement. A handler may not
call an emitter (Section 6.6), so the generator emits a matching `send_to`
member for every reply and acknowledgement message. It buffers like `send`, so
the Section 6.4 stage 1 caps apply to an acknowledgement like any other record.
`DCSBridge.code.send_to.CommandAck(conn_id, outcome, detail)` is the common
case.

Every registration carries a stable key, so a later file can address an earlier
file's registration by name. Keys are namespaced by convention: `builtin.*` for
shipped handlers, an adopter's own prefix for theirs. The built-ins' keys are
part of their documented surface and change only with a release note.

**A file's registrations commit when its load returns, not at each call.** The
runtime collects a file's registration calls and applies them as one set. That
is what lets a conflict refuse a file whole instead of leaving half of it in
force. Two consequences follow. A registration call is valid only while its own
file is loading; a call made later, from inside a handler, is a defect. And a
file cannot read back its own registrations during its own load.

Four rules govern the surface:

- **A duplicate `command` topic refuses the registering file whole.** Ownership
  of an inbound topic is explicit. Take a topic over with `replace`, never by
  racing load order.
- **`off`, `replace` or `wrap` on a key that does not exist refuses the calling
  file whole.** The error names the key and the source order. Every adopter
  source loads after the shipped built-ins, so a missing `builtin.*` key is a
  typo or a key a release renamed, never an ordering problem. Ordering explains
  only adopter-on-adopter overrides inside one directory.
- **A file that fails to compile, raises while loading, or is refused whole is
  logged, counted in `sim_driver_files_failed_total`, and skipped.** The
  remaining files load. Dispatch-time containment is unchanged (Section 6.5).
- **Strings never name a message.** A file reaches a message as a generated
  member: `send.RocketImpact` to emit, `topics.RocketImpact` to claim the
  inbound topic. A typo indexes nil and fails at first use with the name in the
  traceback, instead of queueing nothing. Registration keys stay strings,
  because they are cross-file references, and the refuse-whole rule above is
  their typo check.

The hook driver runtime exposes the same surface on its own table, with hook
callbacks in place of world events: `on(key, callback_name, fn)`, `command` for
hook-driver-targeted topics, and the same `off`, `replace` and `wrap` under the
same rules. Section 4's return-value rule is then enforced structurally: an
extension handler returns nothing into the callback chain, because only the
runtime sits in it.

#### 6.10.3 Options

The built-ins and any extension read their settings from an `options` table in
`Config\DCSBridge.lua`. It is carried at the sim driver tier with the other
sim-driver-tier keys (Section 13.2) and merged into `DCSBridge.code.options`
before any extension file loads.

```lua
-- Config\DCSBridge.lua (excerpt)
options = {
  builtin = {
    -- keys documented in SimDriver.builtin.lua's header; an unknown key is
    -- logged and counted, matching the Section 5.1 config rule
    events_excluded = { 'S_EVENT_MARK_ADDED' },
    resync = true,
  },
  rocketimpacts = { interval = 0.05 },
}
```

An event name in `options` is a string. The hook driver reads the config file
and the hook state has no `world.event` table, so the sim driver resolves the
name.

#### 6.10.4 Changing the built-ins without editing them

Three grades, all from files the adopter owns.

**Options.** Set a key under `options.builtin`.

**Overrides.** A later file suppresses, substitutes or decorates a built-in
registration by key:

```lua
-- simdriver.d\50-mine.lua
DCSBridge.code.off('builtin.mark_added')

DCSBridge.code.wrap('builtin.chat', function(prev, e)
  if e and not blocked(e) then prev(e) end
end)
```

**Suppression.** `sim_driver_disabled_files` lists shipped file names the
loader skips, and `hook_driver_disabled_files` does the same hook-driver-side.
Deleting a shipped file does not last, because a release restores it.
Configuration is the durable form.

#### 6.10.5 Mission scope

**The sim driver loads on every mission.** There is no per-mission switch, and
that is a decision rather than an omission — see below. Every extension file
therefore loads on every mission too, and one mechanism narrows that.

**A file scopes itself.** The runtime exposes `DCSBridge.code.mission` before
any extension file loads. A file that is not for this mission returns early:

```lua
-- simdriver.d\10-campaign.lua
local m = DCSBridge.code.mission   -- { name = ..., filename = ..., theatre = ... }
if m.filename and not m.filename:match('JustCause') then return end
-- registrations for this campaign only
```

**Scope on `filename`, not on `name`.** A mission flown from the Mission Editor
reports its name as `"tempMission"` on every load, whatever the `.miz` is
called, while the filename stays correct (measured, three loads). Scoping on
name therefore fails silently for the one person most likely to be using it — a
mission developer testing from the editor — and fails by matching nothing,
which reads as "my file did not load" rather than as "my guard did not match".
`name` stays in the table because it is what a log line should show a human; it
is not an identity. Match a distinctive path segment rather than anchoring with
`^`: a filename is a full path, so an anchored pattern matches nothing.

`theatre` comes from `env.mission.theatre` in the sim driver's own state.
`name` and `filename` are supplied by the loader, which reads
`DCS.getMissionName()` and `DCS.getMissionFilename()` in the hook driver, so
both are present under Route A and **absent under Route B**, where no hook
driver injects. The `"server"` state does carry both bindings — the two states
hold the same `DCS` table — so the absence is this document's own rule rather
than a platform limit: Section 6.6 forbids the sim driver calling `DCS.*`
because Section 4.3 measured that namespace crossing Lua states and crashing. A
file that scopes on either guards for nil, as above. Route A re-runs every file
at each mission load, so the guard re-evaluates per mission. A `hookdriver.d\`
file loads once per DCS process (Section 3.1) and gates inside its handlers
instead.

**There is no per-mission sim driver switch, by choice.** Neither the operator
nor the mission can ask for a mission to run without a sim driver. The sim
driver loads wherever the bridge is installed and enabled, and the only off
switch is `enabled` (Section 11), which is global and live.

Three things make that safe, and they are why no per-mission switch is needed.
Section 6.5 already contains a misbehaving handler without configuration: a
handler that raises is disabled for the epoch, and `handler_failures_per_epoch`
failures disable the sim driver for the epoch, with the next mission load
starting clean. A file that should not act on a mission scopes itself, above.
And the faults a per-mission switch could not contain anyway are Section 4.2's
access violations, which end the process — an operator learns of one only after
it has happened, and by then `enabled` is the control they reach for.

**A mission that must run without a sim driver is a bug report, not a
configuration.** Either a handler is at fault, which Section 6.5 contains and
names, or a binding is, which Section 4.2's probe and `unsafe_bindings_enabled`
gate. Both are fixable. A per-mission exclusion list would let the fault
survive under a name nobody revisits.

**Under Route B a mission can still decline the sim driver's behaviour**,
because the environments are one: its own `init.lua` runs beside the sim driver
and can call `off` on any registration it does not want (Sections 5.1.2 and
6.10). That is mission-declared control where it costs nothing, and it is the
shape DCS-gRPC's `GRPC.load()` has for the same reason. Route A has no
equivalent and needs none.

#### 6.10.6 Mission-adjacent files

**A mission folder carries its own sim driver files.** The working convention
for mission development is a directory beside the `.miz` —
`Missions\MyMission\` holding the mission, an `init.lua` its `DO SCRIPT` loads,
and the rest of its Lua. With `mission_sim_driver_dirs` enabled, the loader
derives the mission's directory from `DCS.getMissionFilename()` and enumerates
one well-known directory inside it, `<mission dir>\dcsbridge\`, in ascending
name order. The loader normalises separators before deriving the
directory. A mission with no such directory loads nothing.

**The directory name is fixed and no mission file names a path.** Nothing a
mission author writes can direct the loader outside the mission's own
directory, so no path check is needed. A mission that wants a load order names
its files for it, exactly as `simdriver.d\` does.

**A mission sets no built-in option.** `options` is operator configuration
(Section 6.10.3). A mission that needs different built-in behaviour ships a
file that scopes itself, per Section 6.10.5.

The mission's files load after the shipped and operator sources, under the same
containment, naming and override rules, and its own `*.gen.lua` loads before
the directory's other files (Section 6.10.7). `ReloadSimDriver` re-reads them,
so the mission-development loop costs no mission reload.
**The mechanism is Route A's only**: the loader needs
`DCS.getMissionFilename()`, and only the hook driver may call it — the binding
exists in both states, but Section 6.6 forbids the sim driver using it, for the
Section 4.3 reason. Under Route B mission-adjacent files therefore do not load
(Section 5.4.1). Under Route B a mission's own `init.lua` may instead call the
registration surface directly, or `dofile` its own sim driver scripts, because
the environments are one and the bootstrap builds the surface before any
trigger runs. A file
written against the registration surface loads identically through either door.

**`mission_sim_driver_dirs` defaults to false, and Section 14.6 gives the
reason.**

#### 6.10.7 An extension's own vocabulary

An extension that speaks the existing vocabulary installs by copying one file.
One that defines its own messages is a `.proto` and a Lua file together: its
schema merges into the adopter's, `buf generate` reruns, and the new
`schema.pb` costs the Section 5.1 restart.

**An adopter's messages generate into their own file**, never into the bridge
tree's. The generator, run over the adopter's `.proto` merged with the shipped
set, emits a split output holding only the adopter's emitters, decoders, `send`
and `topics` members and registration tables, with its own schema-slice hash
(Section 8.3). That file rides with its source: a `*.gen.lua` in the extension
directory loads before the directory's other files.

Its tables register through the same additive `shim` calls (Section 5.1). New
topics merge. An identical re-registration is a no-op, so a reloaded mission
re-registers cleanly. A topic already registered with different values is
refused, loudly. Merging into `send` and `topics` follows the same rule: a name
collision refuses the file.

Two consequences are stated rather than hidden. Topic registrations live for
the DCS process (Section 5.1), so retiring a topic is a DCS restart, and a
server that rotates between missions with distinct vocabularies accumulates
their union until it restarts. That is bounded and inert — the maps are small
and an unused topic costs nothing — but `max_lifecycle_topics` is the figure
that binds, because only `LIFECYCLE` topics hold a retention slot.

A collision needs no coordination to avoid. A topic is its payload's
fully-qualified type name (Section 5.2), so two extensions collide only by
choosing the same package and message name, which package ownership already
prevents.

And `GetSchema` serves the shipped set only: a consumer of an extension
vocabulary obtains the `.proto` from the extension's own repository, the same
place its Lua comes from.

---

## 7. Operator eval

An operator needs to change behaviour on a running server without ending
everyone's flight. Section 6.9 covers replacing the sim driver. This section
covers running an ad-hoc script.

**The rule is: code comes from the filesystem, code never comes from the
wire.** Section 14.6 states it as a security requirement.

### 7.1 Why the filesystem is the right authority

Section 14.1 states that the bridge does not defend against an attacker who can
write files on the server, because that attacker already owns the machine. An
operator on the box **is** that person. They can already edit `SimDriver.lua`.

A faster path to something they can already do adds no attack surface. It
removes only the mission reload.

A network eval record is a different thing. It makes every consumer a potential
remote code execution vector, and it invalidates the answer Section 14.1 gives
to ED's own warning.

### 7.2 Layout

Input, which the operator creates:

```
Saved Games\<write dir>\Mods\services\DCSBridge\eval\
    server\        injected into the "server" state
    hook\          run in the hook state
```

Output, which the bridge creates:

```
Saved Games\<write dir>\Logs\DCSBridge\
    eval-audit.log                 append-only, one line per execution
    eval\<stem>.<UTC>.log          one file per execution
```

**Input and output are separate trees.** `Logs\` already has the rotation and
retention tooling an operator expects, and an operator hunting a result looks
there first, beside `dcs.log` and every other tool's output. Results are logs
and belong with logs.

The split also keeps a result readable without making the reader able to run
anything, which is a tidy property to have. It is not a defence: Section 14.1
does not treat the filesystem as a boundary, and an operator who can read
`Logs\` on this server can generally write `eval\` on it too.

**One consequence cuts the other way, and Section 14.7 states it.** An operator
who zips their logs for support now ships the eval results too. An eval script
can print unit positions, player names and slot occupancy, so the result tree
holds the same class of data as the replay spool. Set its permissions to match,
and say so where an operator will read it.

**The directory names the target state.** That is the whole reason for the
nesting. The two states have different globals. `"server"` has the world API —
`coord`, `land`, `trigger`, `world`, `timer`, `Unit`, `missionCommands` — and
no `net`. The hook state has `net.*` plus `DCS.*`, which both states carry.
**`net` is not the hook state's alone**: it is absent from `"server"` and
present in the mission scripting state, with 58 members including `dostring_in`
(measured, 2.9.29.27278). These two directories exist because these two states
differ, which they do, and not because only one state holds `net`. The hook
state also still works when the mission scripting state is wedged.

This design rejects a flat directory with the state in the filename. A mistyped
suffix runs in the wrong state silently. A file cannot be dropped into a
directory that does not exist.

**The feature is inert unless the directory exists.** The installer creates
neither. An operator creates one to opt in and removes it to opt out. **There
is no on-off key**: the directory's existence is the switch, so there is no
default to get wrong and no way to have the feature enabled in configuration
and missing on disk. The `eval_*` keys in Section 13.1 tune a feature that is
already on; one of them, `eval_instruction_budget`, refuses to enable it when
invalid (Section 7.5).

**`eval\hook\` carries a specific hazard.** An ad-hoc hook script is exactly
where an operator writes several `DCS.*` getters one after another. Section 4.3
forbids that. Put the warning at the top of the `eval\hook\` documentation. Do
not bury it in a section the operator may never read.

### 7.3 Behaviour

The hook driver polls each existing subdirectory every `eval_poll_interval_ms`
from `onSimulationFrame`.

For each file matching `*.lua`, in ascending mtime order so drop order is
execution order — two files with equal mtimes order by name, because the
clock's resolution is whole seconds — up to `eval_max_files_per_poll`:

1. **Stability check.** Record the file size. Execute only when the size is
   unchanged across `eval_stable_polls` consecutive polls.
2. **Size check.** A file above `eval_max_file_bytes` is renamed `.failed`
   unread, with the reason in its result log.
3. **Rename to `.running`.** Before executing, not after. See Section 7.4.
4. **Compile without executing.** A syntax error fails here and nothing runs.
5. **Execute** under the instruction budget in Section 7.5. `server\` uses
   `pcall(dofile, <path>)`, the same mechanism as Section 5.4.1, so it reaches
   the mission state through `net.dostring_in` and **requires Route A**. Under
   Route B the hook driver makes no such call and only `eval\hook\` works.
   `hook\` runs directly.
6. **Write results** to `Logs\DCSBridge\eval\`: return value, captured `print`
   output, and any error with its traceback.
7. **Rename** the input to `.done` or `.failed`.

**Rename the input in place. Write the result elsewhere.** The poller matches
`*.lua` only, so renaming out of that extension prevents re-execution. The
rename is a state machine on the input file and belongs beside it. The result
is a log and belongs in `Logs\`. The filesystem carries both across a DCS
restart.

**Name a result log `<stem>.<UTC>.log`.** `<stem>` is the input filename as
first seen, with its extension removed and before any rename to `.running`.
`<UTC>` is `os.date('!%Y%m%d-%H%M%S')`, taken when the poller selects the file
rather than when execution starts, so a file rejected on size or failing to
compile still has a defined name. `spawn-test.lua` becomes
`spawn-test.20260825-161104.log`. On a same-second collision, append `-2`,
`-3`, and so on. `os` is present in the hook state, where the poller runs.

Both the format and the time base follow DCS. `dcs.log` stamps its lines in
UTC, and DCS names its own rotated logs `dcs.log-20260824-170927.zip` — the
same `%Y%m%d-%H%M%S` form. An operator can line the two up without reformatting
either.

The `.log` extension is deliberate: it matches what every other tool writes
into `Logs\`, so an operator's existing log tooling picks these up.

**Give every result log a header line.** Name, target state, `source_sha256`,
start time, outcome and duration. A result log is often read detached from the
input that produced it, and the hash is what correlates it to `EvalExecuted`
and to `eval-audit.log`.

**Bound the result directory.** `eval_log_max_bytes` binds first and
`eval_log_retention_days` second, deleting oldest first. This is the same order
Section 13.1 states for the spool, and for the same reason: bounded disk is a
hard requirement, retention is a preference.

**Do not use an mtime threshold for step 1.** DCS does not ship LuaFileSystem.
`lfs` is ED's own VFS binding, and it both adds and drops members against stock
LFS. It adds `add_location`, `create_lockfile`, `del_location`, `locations`,
`md5sum`, `normpath`, `realpath`, `tempdir` and `writedir`. It drops `lock`,
`unlock`, `touch`, `link`, `symlinkattributes` and `setmode`.

The `lfs.attributes` field set is undocumented. ED's own code reads only `mode`
and `modification` off the result, and passes `'mode'` and `'size'` as the
optional second argument. That is a lower bound on what the call returns, not a
complete field list, so assume the set is narrower than stock LFS rather than
wider and depend on those three fields only.

The resolution of `modification` is unmeasured on this build. On reference LFS
under Windows it is a `time_t` in whole seconds, and ED's field names match, so
that is the safe assumption. A sub-second threshold therefore sits below the
clock's likely resolution. A file written and polled inside the same second
reports an unchanged mtime while still being written. The failure is in the
dangerous direction.

**`lfs.dir` enforces a sandbox on this build, and the write directory is inside
it.** ED's own `Scripts\Hooks\webGUI.lua` enumerates write-directory paths from
the hook state with `lfs.dir` — `lfs.writedir() .. "Missions"` among them — and
clamps a request that escapes the Saved Games root to that root rather than
refusing it. A write-directory path is therefore listable from the hook state,
which is what the eval directories and the Section 6.10 extension directories
need. A path outside the write directory may still be refused; build on none.
`lfs.mkdir` is present in the same state, so tooling can create a directory it
needs.

**Write-and-rename is the reliable path.** A `.lua.tmp` written and then
renamed to a `.lua` name that does not yet exist lands as one directory-entry
update, so a reader sees the whole file or no file. It is the only mechanism
correct for an editor writing in several passes.

**It is not a general atomic replace.** Lua 5.1's `os.rename` is C `rename`,
which on Windows fails when the destination already exists, and no Lua-visible
call offers `MOVEFILE_REPLACE_EXISTING`. Re-dropping a filename that already
has a `.done` or `.failed` sibling is an operator error the poller reports
rather than papers over. The size check is a convenience layered on top, not a
guarantee. Document both.

### 7.4 Rename before executing

A script that takes DCS down never reaches step 7. Left as `*.lua`, it executes
again on the next start. That is a **crash loop**. The operator's only escape
is to find and delete the file with DCS not running. That is the moment they
are least equipped to reason about what happened.

```
foo.lua  →  foo.lua.running  →  [execute]  →  foo.lua.done | foo.lua.failed
foo.lua  →  foo.lua.failed                     (rejected on size, unread — step 2)
```

The `*.lua` match already excludes `.running` from re-execution. A crash leaves
`foo.lua.running` on disk. The operator sees which script was in flight, and
the renamed file does not run again.

**At hook driver load, sweep for `.running` files.** Log each one as a
suspected crasher. Leave it alone.

### 7.5 Bounding execution

**Both kinds run on the render thread.** The poller fires from
`onSimulationFrame`, which is a render-loop callback, and a `server\` script
reaches the mission state through a synchronous `net.dostring_in` from that
same callback. An unbounded script therefore stalls the render loop, and a
`server\` script stalls the sim with it. The render loop keeps ticking while
the sim is paused, so eval continues to poll and execute during a pause.

`debug.sethook` is present in the `"server"` state. A count hook bounds
execution:

```lua
local function bounded(f)
  debug.sethook(function()
    debug.sethook()                    -- clear first, so the error path is clean
    error('eval exceeded instruction budget', 2)
  end, '', eval_instruction_budget)
  local ok, err = pcall(f)
  debug.sethook()
  return ok, err
end
```

The raise propagates as an ordinary Lua error. The surrounding `pcall` catches
it. The result log records why. The sim continues.

**`eval_instruction_budget` must be a positive integer.** Lua 5.1's
`debug.sethook` sets the count bit of the mask only when the count is above
zero. With the empty mask string this specification uses, a count of `0` or
`nil` produces an empty mask, and `lua_sethook` reads an empty mask as hooks
off: it installs **no hook at all**, not a hook that never fires.
`debug.gethook()` nonetheless still returns the function, because `db_sethook`
stores it in the registry before calling `lua_sethook`, so a naive readback
looks armed. A missing value therefore disables the budget silently. Validate
it at load and on every apply, and refuse to enable eval otherwise.

Three limits, stated rather than hidden:

- A count hook counts VM instructions. **It cannot interrupt a single
  long-running C call** that blocks inside DCS's own code.
- The count hook costs something every `eval_instruction_budget` instructions.
  At a generous default that is negligible. At a small one it would not be.
- It is a guardrail against accident, not a security boundary. The script can
  clear the hook itself. Under Section 14.1's threat model that is acceptable.

**A count hook in the `"server"` state is measured safe (2.9.29.27278)**: the
budget error propagates, the surrounding `pcall` catches it, and the sim
continues. Re-test after a DCS update.

**A long eval makes `dcs_alive` go false.** The heartbeat atomic stops updating
while the logic thread is stalled. That is correct — DCS really is unresponsive
— and it is expected behaviour, not a fault.

### 7.6 Audit

Every execution emits `EvalExecuted`, class `DURABLE`:

| Field | Meaning |
|---|---|
| `file_name` | The dropped file name |
| `target_state` | `server` or `hook` |
| `source_sha256` | Hash of the source that ran |
| `succeeded` | Whether it completed without raising |
| `duration_us` | Callback time consumed, measured around the execution |

**The record is a notification, not the audit trail.** Section 11's first row
discards records when no consumer is connected. A `DURABLE` record therefore
exists only if somebody was watching. That is the wrong property for an audit.

**The result log and `Logs\DCSBridge\eval-audit.log` are authoritative.** The
audit log is append-only, one line per execution, carrying the same fields as
the record. Write the audit line before emitting the record, and write it even
when the execution never starts — a file rejected on size or failing to compile
is still an execution attempt an auditor needs to see.

`eval_audit_max_bytes` and `eval_audit_retention_days` bound the audit log
(Section 13.1). The hook driver rotates the file at the size cap and deletes
the oldest rotated file first: size binds before age, the same order as the
spool and the result tree.

### 7.7 Limits

- Eval does not run during the mission-load blackout, because polling runs from
  `onSimulationFrame` and no frame fires. That is the moment an operator might
  most want it. No alternative callback offers a timer.
- Filesystem permissions on the input `eval\` directory are the entire
  authorisation model. Say so in those words. Permissions on
  `Logs\DCSBridge\eval\` are a separate, weaker grant: reading a result confers
  nothing.
- Compile-before-execute catches syntax errors and nothing else. A script that
  runs and corrupts sim driver state does so at full speed.

---

## 8. Schema and generation

### 8.1 Record classes

Every message that crosses into or out of Lua declares a class. A class is
broker policy. The broker-answered messages in Section 5.2 and `SeqAck` carry
neither a record class nor a required capability: they never reach Lua, and
none of them can disclose or change anything a capability would gate. **They
are not exempt from the drop rule** — Section 5.2 treats a broker answer as
`DURABLE` for that purpose, because a record with no class would otherwise fall
outside a policy written entirely in terms of classes. **`SetEnabled` is the
exception and carries `CAPABILITY_RELOAD`** (Section 9.5): it stops every
record on the server, which
is exactly what a capability exists to gate. Section 8.2's rule that the
generator refuses a message with no capability is scoped to messages that reach
Lua; the list in this paragraph is the whole set it does not reach.

| Class | Direction | On drop |
|---|---|---|
| `DURABLE` | Mission to consumer | Never discard silently while a consumer is connected. Count every drop. With none connected, records are discarded uncounted — Section 11's first row. |
| `LOSSY` | Mission to consumer | Discard freely under pressure. |
| `COMMAND` | Consumer to mission | Retry with an idempotency key. |
| `LIFECYCLE` | Mission to consumer | Never discard. Reserve headroom in the outbound ring, ahead of every other class. Retained latest-per-topic and replayed to each newly authenticated connection. Section 5.2. |

**Every `LIFECYCLE` topic must have last-value semantics.** The broker retains
the latest record per `LIFECYCLE` topic and replays the retained set to each
newly authenticated connection (Section 5.2), so a topic's latest record must
be meaningful on its own. The thirteen Section 9 topics qualify. An adopter
adding a `LIFECYCLE` topic accepts the same constraint.

### 8.2 The schema

```proto
syntax = "proto3";
package dcs.bridge;
import "google/protobuf/descriptor.proto";

enum RecordClass {
  RECORD_CLASS_UNSPECIFIED = 0;
  RECORD_CLASS_DURABLE     = 1;
  RECORD_CLASS_LOSSY       = 2;
  RECORD_CLASS_COMMAND     = 3;
  RECORD_CLASS_LIFECYCLE   = 4;
}

// EMITTED at 0: an unspecified target is generated into the sim driver route set.
// Not extensible: the broker implements each target.
enum Target {
  TARGET_UNSPECIFIED = 0;
  TARGET_SIM_DRIVER  = 1;
  TARGET_HOOK_DRIVER = 2;
}

// NEVER EMITTED at 0: the generator refuses a message with no capability.
// Extensible per the range table below. The bridge defines three members and
// no more (Section 14.4).
enum Capability {
  CAPABILITY_UNSPECIFIED = 0;
  CAPABILITY_READ        = 1;
  CAPABILITY_COMMAND     = 2;
  CAPABILITY_RELOAD      = 3;
}

// NEVER EMITTED at 0. Not extensible: the broker implements each member.
// Carried by SetTopicFilter (Section 5.2).
enum TopicFilterMode {
  TOPIC_FILTER_MODE_UNSPECIFIED = 0;
  TOPIC_FILTER_MODE_ALL         = 1;
  TOPIC_FILTER_MODE_ONLY        = 2;
}

// NEVER EMITTED at 0. Not extensible: the broker implements each member.
// Carried by TopicFilterResult (Section 5.2).
enum TopicFilterRefusal {
  TOPIC_FILTER_REFUSAL_UNSPECIFIED   = 0;
  TOPIC_FILTER_REFUSAL_NO_MODE       = 1;
  TOPIC_FILTER_REFUSAL_LIST_WITH_ALL = 2;
  TOPIC_FILTER_REFUSAL_TOO_MANY      = 3;
  TOPIC_FILTER_REFUSAL_MALFORMED     = 4;
}

// NEVER EMITTED at 0. Not extensible: the broker produces every member.
// Carried by Rejected (Section 5.2).
enum RejectedReason {
  REJECTED_REASON_UNSPECIFIED   = 0;
  REJECTED_REASON_UNKNOWN_TOPIC = 1;
  REJECTED_REASON_NO_CAPABILITY = 2;
  REJECTED_REASON_RATE_LIMITED  = 3;
  REJECTED_REASON_BUSY          = 4;
}

extend google.protobuf.MessageOptions {
  RecordClass record_class        = 50001;
  // On a request message, names the reply message. See Section 8.3.
  string      reply_to            = 50002;
  Target      target              = 50003;
  Capability  required_capability = 50004;
}

message UnitDestroyed {
  option (record_class) = RECORD_CLASS_DURABLE;

  string unit_name = 1;
  int32  coalition = 2;
}
```

Extension numbers 50000 to 99999 are reserved for internal use.

**Adopters extend upward; the bridge owns the low numbers.** The rule covers
every numbered space an adopter may extend, not topic ids alone. Without it,
two adopters take the same next free number, and a capture from one decodes as
the wrong type against the other; the schema hash reports a mismatch only as a
warning (Section 5.2), so the failure surfaces late and looks like corruption.

| Space | Bridge | The built-ins | An adopter's own |
|---|---|---|---|
| Any bridge enum marked extensible | 1 to 49 | 50 to 99 | 100 and above |

**The built-ins share one band, and they are one owner.** `SimDriver.builtin.lua`
and `HookDriver.builtin.lua` ship in one release, from one owner, generated
from one schema, so the generator refuses a duplicate before it can reach a
consumer. Splitting the band between them would reserve numbers against a
collision the build already catches.

**The table binds an extension of a bridge enum, and nothing else.** Three
qualify: `CommandAckOutcome`, `Capability` and `DeclinationStatus`. An
extension's own enum is a new enum in its own package and numbers from zero
with no constraint at all — the built-ins' `ChatTarget` and `ChatSource` are
that case (HOOK §5), not extensions of anything here.

**Topics are not in that table, because topics are not numbered.** A topic is
the payload's fully-qualified type name (Section 5.2), so package naming does
the partitioning that a numbered range would otherwise have to. The bridge's
own records live in `dcs.bridge`, **both built-in sets' in `dcs.builtin`**, and
an adopter's in a package they own. One package for the built-ins, because the
Lua state a topic is produced from is an implementation detail a consumer
should not have to read off a topic name; the two sets choose distinct message
names, and the generator refuses a collision. Nothing is reserved and nothing
can be taken.

**An enum is extensible only if it says so.** Mark each enum in a comment.
`CommandAckOutcome` (Section 8.5.3), `Capability` and `DeclinationStatus`
(Section 6.3) are extensible: a handler knows outcomes the bridge does not, an
adopter defines capabilities the bridge does not (Section 14.4), and a later
build may find a further reason the declination is unavailable.
`RecordClass`, `Target`, `RejectedReason`, `TopicFilterMode` and
`TopicFilterRefusal` are not: the broker implements each member, so a new one
is a broker change rather than a schema one.

**The broker needs the class, the target and the capability, and holds no
schema.** Section 3 keeps the schema out of the broker, but Sections 5.2, 10
and 11 make the broker apply per-class drop policy, Section 5.2 makes it route
by target, and Section 14.4 makes it enforce capabilities. Registration bridges
the gap: the generated tables are maps of type URL to integer, registered
through `shim.classes`, `shim.routes` and `shim.caps` (Section 5.1) — by the
sim driver for sim-driver-targeted topics at each load, by the hook driver for
hook-driver-targeted topics at DCS start. That is not a schema and it links no
protobuf runtime.

Five schema rules prevent wire-breaking changes and silently-wrong values:

- Give every enum an `_UNSPECIFIED = 0` member from the first commit. Adding
  one later renumbers every other member.
- Pin the coordinate convention. **DCS is +x north, +z east, +y up.** State all
  three axes in a comment on every position field. Records carry DCS-local
  coordinates, never latitude and longitude: converting at emit costs an engine
  call per position. **This rule binds every record — the bridge's, the sim
  driver built-ins' and an adopter's — so the bridge supplies the key to it**,
  in `CoordinateCalibration` (Sections 6.3 and 9). SIM §4 specifies the
  conversion a consumer performs with it.
- Pin the heading reference. Every heading, bearing or course field states
  **grid**, **true** or **magnetic** in a comment, the way a position field
  states axes. DCS's own data mixes references — runway headings are magnetic,
  velocity-derived directions are grid — and a mismatch produces a plausible
  wrong answer rather than an error. SIM §4 gives the three norths.
- Never reuse one message for both directions.
- Use snake_case field names. Prefix enum members with the enum name. `buf
  lint` enforces both.

### 8.3 What is generated

**Message types come from stock plugins, in every language protobuf supports.**
Someone writing a Node consumer runs `buf generate` and gets every message type
without touching this repository.

**Consumer ergonomics come from this repository's own plugin**, and are
optional. It emits the typed request and reply wrappers below. A consumer that
wants neither decodes the envelope itself and never runs it. State the split
plainly in the consumer documentation: the types are free and universal, the
wrappers are ours and per-language.

**Generated names are the schema's names, verbatim.** The generator performs no
case conversion. `message RocketImpact` generates `send.RocketImpact` and
`topics.RocketImpact`, member for member, and a decoder keys its output table
by field name unchanged. The `.proto` side is already fixed by `buf lint`
(Section 8.2): PascalCase messages, snake_case fields. One name therefore greps
from the schema through the sim driver, the coverage document and most consumer
languages' generated types. A conversion would need acronym rules; identity
needs none.

You write one generator: `protoc-gen-dcsbridge-lua`. It emits seven things,
split by each message's `target` option (Section 8.2):

1. **Emitters**, one per outbound message. Shown below. An emitter for a reply
   or acknowledgement message takes a connection id as its first argument and
   opens with `begin_to` (Section 5.1); every other emitter opens with `begin`.
   A sim-driver-targeted message's emitter lands in `DCSBridge.code.emit`; a
   hook-driver-targeted message's emitter lands in the hook-driver-side file.
2. **Send wrappers**, one per outbound message. A wrapper buffers the record;
   the Section 6.4 stage 1 drain calls the emitter. Handlers and extension
   files call these, never the emitters (Sections 6.6 and 6.10). A fan-out
   message generates into `DCSBridge.code.send`; a reply or acknowledgement
   generates into `DCSBridge.code.send_to` and takes a connection id as its
   first argument, mirroring its emitter. **Arguments are the message's fields
   in declaration order.** A repeated field takes one Lua array. A repeated
   scalar field emits one put call per element, in array order. A repeated
   message field emits one `message` and `end_message` pair per element, in
   array order, taking each element from a table keyed by field name. An
   emitter never reorders elements, because Section 5.1 makes put-call order
   element order.
3. **The topics table**, message name to topic id, in `DCSBridge.code.topics`.
   A `command` registration names its topic through this table, so no extension
   file writes a type URL as a literal.
4. **Decoders**, one per `COMMAND` message. `shim.poll` returns a topic id and
   an opaque body. Something must turn that body into named fields. Neither Lua
   state has a protobuf runtime. The generator emits a field-number switch per
   message, placed by target like the emitters. The decoder skips unknown field
   numbers by length, which is what stops a consumer one release ahead from
   breaking the sim driver. Its depth and repetition bounds are generator
   constants derived from the schema (Section 14.2).
5. **The class tables**, topic to record class, one per target. The sim driver
   registers its table at load and the hook driver registers its own at DCS
   start, through `shim.classes` (Section 5.1), so the broker can apply the
   Section 8.1 drop policy without holding a schema. The sim driver also uses
   it locally to decide which commands need an idempotency check.
6. **The route tables**, topic to target, registered the same way through
   `shim.routes`, so the broker can route inbound records (Section 5.2).
7. **The capability tables**, topic to required capability, registered the same
   way through `shim.caps`, so the broker can enforce Section 14.4.

```lua
function DCSBridge.code.emit.UnitDestroyed(unit_name, coalition)
  DCSBridge.shim.begin(500)
  DCSBridge.shim.string(1, unit_name)
  DCSBridge.shim.integer(2, coalition)
  DCSBridge.shim.commit()
end
```

**The generated files carry LuaLS annotations** — parameter names and types
mapped from the proto fields — so a language server completes a message name
and flags a wrong argument at edit time. PLAN §2 ships the matching stub and
editor configuration.

A protoc plugin reads a `CodeGeneratorRequest` from stdin and writes a
`CodeGeneratorResponse` to stdout. It is Rust, like the broker and the CLI
(Section 2), and unlike the broker it may link a protobuf runtime freely — it
runs on the build machine, not in the DCS process.

**The plugin must advertise `FEATURE_PROTO3_OPTIONAL`.** The schema uses proto3
`optional` for every field where absent must differ from zero (Section 8.4),
and protoc refuses to hand a file using it to a plugin that has not set that
bit in its `CodeGeneratorResponse.supported_features`. The failure is at build
time and names the plugin, so it is cheap to hit and cheap to fix; it is stated
here because it is the one generator requirement that is not visible from the
generated output. Proto3 `optional` needs protoc 3.15 or newer, which `buf`
satisfies.

**Typed request and reply wrappers.** Where a message carries `reply_to`, the
consumer plugin emits the paired call:

```kotlin
suspend fun BridgeClient.getUnitPosition(unitName: String): UnitPositionReply
```

**Everything generated for the sim driver lands in one file,
`Mods\services\DCSBridge\lua\SimDriver.gen.lua`**, mirroring the hook driver
side. Its tables register into `DCSBridge.code` at each load, per Section 6.1,
so a reload replaces them.

**The generator supports split output.** The shipped messages generate into the
bridge tree's two `.gen` files. An adopter's own messages generate into a
`*.gen.lua` that rides with their extension source, carrying its own
schema-slice hash. Section 6.10 gives the loading rule.

**Everything generated for the hook driver lands in one file,
`Mods\services\DCSBridge\lua\HookDriver.gen.lua`** — the hook-driver-targeted
emitters and decoders, and the hook-driver-side class, route and capability
tables. The hook driver payload loads it at DCS start and registers its tables
(Section 5.1). It is outside `DCSBridge.code` deliberately: a
hook-driver-targeted topic must work when no sim driver is loaded. It is
guarded by the schema hash instead of by reload (Section 6.1): the generator
embeds the run's schema hash in `HookDriver.gen.lua` and in the sim driver's
generated code; the hook driver passes its hash to the broker at the first
`configure` as `schema_sha256`, `shim.classes` carries the sim driver's, and
the broker compares both against its own hash of the `shim.schema` bytes,
reports any mismatch in `stats`, and answers registration with a warning
`doctor` prints. A hook driver and a sim driver from different generator runs
are detected rather than silently mis-routed.

**The build also writes the compiled `FileDescriptorSet` to
`Mods\services\DCSBridge\schema.pb`.** It deploys with the other generated
files. The hook driver reads it at DCS start and hands the bytes to the broker
with `shim.schema` (Section 5.1). The hook driver reads it once, so a record
type added without a DCS restart leaves the served set one restart behind; the
hash comparison above reports exactly that, and a restart clears it.

**Not generated:** handlers, and anything in the broker.

### 8.4 Evolution

Field numbers are permanent. Never reuse one. Mark a removed number `reserved`.

Run `buf lint` and `buf breaking` in CI against the previous release. Use
`optional` on any scalar where zero must differ from absent.

**Express a per-element optional inside a repeated message, never as a parallel
array.** `optional` marks a field absent in one message. Parallel scalar arrays
have no way to say that one element omits a value, and they cannot carry
elements whose shape differs from one another. A repeated message field carries
both, so it is the shape for any list whose elements are more than one number.

CI also checks schema ownership, and it is now a naming check rather than a
numbering one. The bridge's own message definitions are the records Section 1.2
enumerates **and the nested types those records carry** — `MissionDate`,
`Projection` and `Verification` under `CoordinateCalibration` (Section 6.3) are
definitions no enumeration of records covers, and a check that counts only
enumerated records rejects them. Those are the only messages permitted in the
`dcs.bridge` package. Nothing else needs policing: the `Envelope` names no
payload type, so no shared file exists for two owners to contend over.

### 8.5 Application-layer patterns

The bridge is not an RPC framework (Section 1.2). A read-decide-write loop
across the network would block the logic thread or race a world that has
already moved.

#### 8.5.1 Subscription

Prefer this pattern. Most requests for a value are requests for a condition.

Do not ask whether a unit is within 5 km of a zone. Subscribe to zone entry
through Section 6.7, which specifies the sim driver side. This is Section 6.7's
mechanism, not Section 5.2's topic filter.

#### 8.5.2 Request and reply

Use this pattern for ad-hoc reads only: admin tools, debugging, an operator
console.

```proto
message GetUnitPosition {
  option (record_class) = RECORD_CLASS_COMMAND;
  option (reply_to)     = "UnitPositionReply";
  uint64 request_id = 1;
  string unit_name  = 2;
}

message UnitPositionReply {
  option (record_class) = RECORD_CLASS_DURABLE;
  uint64 request_id = 1;
  bool   found      = 2;
  double x = 3;  // DCS-local, north
  double y = 4;  // DCS-local, up
  double z = 5;  // DCS-local, east
}
```

The consumer allocates an id, parks a slot in a pending map, sends the command,
and suspends. The sim driver handles it in its bounded dispatch and emits a
reply carrying the same id.

**`request_id` is `uint64` on the wire and must stay under 2^53.** The sim
driver decodes it into a Lua 5.1 number, which is a double, and echoes it
through `shim.integer` (Section 5.1). A consumer that allocates ids above that
range gets a reply correlated to a different request, silently. Allocate from a
per-connection counter starting at 1 rather than from a random 64-bit value or
a timestamp in nanoseconds. The same bound governs any adopter field a handler
round-trips through Lua; carry anything wider as a string.

**`request_id` is the idempotency key for a request.** A read request is
idempotent by nature. Re-executing one is harmless. The sim driver therefore
needs no recent-key set for it. A `COMMAND` that mutates the world carries a
separate `idempotency_key` field and **is** checked against the recent-key set,
bounded at `recent_idempotency_keys`. The two fields exist because the two
guarantees differ: one correlates a reply, the other prevents a double spawn.

A duplicate — a key already in the recent-key set — executes nothing and is
acknowledged with a `CommandAck` carrying outcome `DUPLICATE` (Section 8.5.3).
No outcome is stored or replayed. The sender's retry loop therefore always
terminates, and a retry after a lost acknowledgement costs one round trip, not
a re-execution.

The consumer waits. The sim does not.

Three rules make the pattern reliable:

- **Always reply.** Every path out of a request handler emits a reply,
  including the not-found path.
- **Always time out**, at `request_timeout_ms`. A reply may never arrive.
- **Discard pending requests on `EpochClosed`.** Complete the waiting call with
  an error, never with a cancellation.

If a consumer polls with this pattern, the correct shape is Section 8.5.1.

#### 8.5.3 The acknowledgement record

**At most one point-to-point record answers a command.** Which record depends
on whether the command declares `reply_to`. The answer is exactly one in every
case but three, and none of the three is design: a `Rejected` withheld by
`rejected_max_per_sec` (Section 5.2), a `Rejected` withheld by
`busy_max_per_sec`, and an answer the outbound ring refused at its
`LIFECYCLE` reserve. `request_timeout_ms` covers all three. A consumer must
therefore treat a timeout as a possible outcome of every command, not as a
fault.

| Command | Answered on success by | Answered on failure by |
|---|---|---|
| Declares `reply_to` | its typed reply | `CommandAck` |
| Declares none | `CommandAck` | `CommandAck` |
| Refused before Lua | — | `Rejected` |

The third row is the case the other two do not cover: the command never reached
a handler, so no handler can answer it. `Rejected` carries the reason (Section
5.2) and echoes the inbound `seq` rather than a correlation field the broker
never parsed.

A read already has an answer: Section 8.5.2 gives it a typed reply carrying the
same `request_id`. **A successful read emits that reply and no
acknowledgement.** Two records for one request would leave a generated wrapper
with two things to resolve on.

**`CommandAck` is the failure channel for a read.** A typed reply cannot carry
"the handler raised" or "the key was a duplicate": its fields describe a
result, not an outcome. A not-found is a result, not a failure —
`UnitPositionReply.found` travels in the typed reply.

**A generated wrapper resolves on either and matches on `request_id`.** It
returns the typed reply. It raises on a `CommandAck`.

The name says what it acknowledges. `SeqAck` acknowledges a sequence number and
this acknowledges a command. A bare `Ack` beside `SeqAck` would read as the
general case of it, which it is not.

```proto
message CommandAck {
  option (record_class) = RECORD_CLASS_DURABLE;

  optional uint64   request_id      = 1;  // set on a read, absent on a mutation
  optional string   idempotency_key = 2;  // set on a mutation, absent on a read
  CommandAckOutcome outcome         = 3;
  string            detail          = 4;  // short, for a human reading a log
}

// Never emitted at 0. Extensible per Section 8.2: 50 to 99 for the built-ins,
// 100 and above for an adopter.
enum CommandAckOutcome {
  COMMAND_ACK_OUTCOME_UNSPECIFIED = 0;
  COMMAND_ACK_OUTCOME_OK          = 1;
  COMMAND_ACK_OUTCOME_FAILED      = 2;
  COMMAND_ACK_OUTCOME_DUPLICATE   = 3;
  COMMAND_ACK_OUTCOME_REFUSED     = 4;
}
```

**A consumer that meets an outcome it does not know treats it as `FAILED`.**
The four above cover every case the bridge itself produces. A handler with more
to say adds a member in its own range and puts the human-readable part in
`detail`.

`request_id` and `idempotency_key` follow Section 8.5.2: a read carries the
first, a mutation the second, **never both, except where a mutation declares
`reply_to`**. Both are `optional` because absence is meaningful and zero is a
value (Section 8.4).

**A mutation that declares `reply_to` carries both.** It needs the key so a
retry does not perform the mutation twice, and the id so the generated wrapper
resolves on the reply the way it resolves on any other. The alternatives are
teaching every generated wrapper a second match path, or making the mutation
non-idempotent. Such a command is checked against the recent-key set like any
other mutation, and a duplicate is answered with `CommandAck` outcome
`DUPLICATE` rather than with the typed reply.

The emitting handler truncates `detail` to `command_ack_detail_max_bytes`. An
acknowledgement never fails because of its own detail.

**A `CommandAck` is a record and can be dropped.** It is `DURABLE`, so Section
8.1 forbids a silent discard while a consumer is connected and requires the
drop to be counted. A consumer that receives no answer re-sends the command
with the same `idempotency_key`. The duplicate executes nothing and is
acknowledged with outcome `DUPLICATE`, per Section 8.5.2; no outcome is stored
or replayed.

**The handler correlates. The broker does not.** The broker routes an answer to
a connection (`begin_to`, Section 5.1) and holds no memory of the command it
answers. The recent-key set that spots a duplicate belongs to the handler, per
Section 8.5.2.

**Two commands sit outside this contract.** `SeqAck` and `SetEnabled` never
reach Lua (Section 9.5), so nothing can answer them from it; `SetEnabled`'s
effect is visible in `Pong`'s `bridge_enabled` field. A command the broker
refuses before it reaches Lua — an unrouted topic, a missing capability, a rate
limit (Section 14.4), a full inbound ring (Section 5.2) — is answered by
`Rejected` (Section 5.2), which makes three possible answers in all: at most
one point-to-point record answers a command, and it is the typed reply, a
`CommandAck`, or a `Rejected` where the command never reached Lua. The broker
cannot recover `request_id` from a body it never parses, so `Rejected` echoes
the inbound envelope's `seq` instead; a generated wrapper records the `seq` its
request was sent with and resolves — by raising — on a `Rejected` echoing it,
the same way it resolves on `request_id` for the other two. A `Rejected`
withheld by its rate cap (Section 5.2) leaves `request_timeout_ms` to cover the
silence.

Fan-out records a command triggers — a resync stream, a spawned group's events
— are not its answer. The answer is the one point-to-point record.

---

## 9. Lifecycle

```
DCS start
  └─ hook driver loads, broker starts, listener opens
       └─ consumer connects, handshake, auth
            └─ onMissionLoadBegin        ← no per-frame callback for the whole load
            └─ onMissionLoadEnd
                 ├─ hook driver reads the sim driver from disk
                 ├─ epoch opens
                 └─ injects it with dostring_in("server", ...)
                      └─ mission runs
                 └─ onSimulationStop, epoch closes
            └─ next mission, new epoch
```

**Thirteen lifecycle topics.**

**The Emitter column is the point of the table.** Nine of the thirteen are the
hook driver's, so they arrive whether or not a sim driver ever loads — which is
what makes the Section 9.5 no-sim-driver case a working configuration. The four
the sim driver owns are the two resync brackets and the two sim-driver-state
records, all of which describe the sim driver itself and are meaningless
without one.

| Topic | Emitter | Meaning |
|---|---|---|
| `MissionLoadBegan` | Hook driver | A mission load started. Silence follows. |
| `MissionLoaded` | Hook driver | A mission finished loading |
| `MissionStopped` | Hook driver | The mission stopped |
| `EpochOpened` | Hook driver | A new epoch is current. Carries the epoch id, the mission-start wall-clock time, the mission time, the terrain name, the mission name, and the `is_server` and `is_multiplayer` pair. Section 6.3. |
| `EpochClosed` | Hook driver | The epoch ended. Every unit reference is void. |
| `CoordinateCalibration` | Hook driver | How to convert this theatre's DCS-local coordinates to latitude and longitude, from `terrain.convertMetersToLatLon`. Arrives with or without a sim driver. Describes the current epoch; a consumer holds it for the epoch. Section 6.3. |
| `SimulationPaused` | Hook driver | Mission time stopped advancing. Section 9.2. |
| `SimulationResumed` | Hook driver | Mission time is advancing again. Section 9.2. |
| `ResyncBegan` | Sim driver | A resync scan started. Section 6.8. |
| `ResyncEnded` | Sim driver | The resync scan is complete. Section 6.8. |
| `CallbackHz` | Hook driver | Render-loop callback rate for the last second. Section 9.3. |
| `SimDriverLoaded` | Sim driver | The sim driver loaded. Carries the injection route, whether it reaches the broker directly, and the sandbox level. |
| `SimDriverReloaded` | Sim driver | `DCSBridge.code` was replaced. Carries whether state was preserved and what was dropped. Section 6.9. |

### 9.1 The mission-load blackout

During a mission load, `onSimulationFrame` fires **zero** times. Only
`onMissionLoadBegin` and `onMissionLoadEnd` run. There is no drain, no poll, no
command dispatch.

**The blackout is total, and that is measured rather than inferred.** A frame
collector armed across two mission loads credited **zero** samples to the
loading phase, while the load callbacks fired normally either side of it. A
later hook instrumenting three further loads on three terrains counted zero
frames between `onMissionLoadBegin` and `onMissionLoadEnd` in every one. So a
load is not a sequence of slow frames; it is an absence of frames.

**The duration varies widely.** Five instrumented loads measured 4.7 s, 5.2 s,
13.4 s, 23.6 s and 34.7 s, the last a cold-cache GermanyCW. The same machine's
`dcs.log` records five more at 20.2 s, 39.1 s, 52.7 s, 59.9 s and 45.1 s —
larger missions on colder caches. The instrumented range now reaches into the
logged one, so the two agree rather than describing different things. Treat the
load window as tens of seconds and size `load_timeout_ms` for the upper end.

`MissionLoadBegan` is emitted from `onMissionLoadBegin`. It is a **correctness
requirement, not a convenience.** It raises the liveness threshold for the load
window, from `dcs_alive_threshold_ms` to `dcs_alive_threshold_loading_ms`. A
consumer then does not read a normal load as a dead sim. `MissionLoaded` lowers
it again.

Section 11 states the consumer rule.

### 9.2 Pause

Pause state is **derived from `DCS.getPause()`**, polled every
`pause_poll_interval_ms`, default one second. The callbacks are change hints,
not the source of truth.

Three measured behaviours break a purely callback-derived flag. DCS can begin a
mission already paused without firing either callback. A resume can arrive with
no preceding pause. The callback fires on the render loop, so a pause does not
stop the drain under Route A; under Route B the sim driver's timer-driven drain
suspends while paused (Section 5.4.1).

Poll `DCS.getPause()` alone and guarded, never batched, per Section 4.3. Once
per second is chosen to reduce exposure at no functional cost.

The poll rides `onSimulationFrame`, so the configured interval is a floor:
while the callback gaps, the effective interval stretches with it.

### 9.3 `CallbackHz`

DCS exposes no frame-rate reading to a hook. `maxFPS` is a graphics setting in
`Config\graphics.lua`, and `DCS.setMaxFPS` writes it — the options UI calls it.
No getter was found. Either way it is a cap the operator chose, not a
measurement of what the loop is doing.

`CallbackHz` is computed by counting `onSimulationFrame` invocations against
`DCS.getRealTime()`, which is seconds since DCS process start rather than since
mission start. **It is the render-loop rate, not the simulation rate.** The
callback continues while a mission is paused and mission time is frozen, **at
close to its normal rate**: across 5,794 gaps collected while paused, 5,790
were 20 ms or under. An earlier figure near 11 Hz was an average over too short
a window and is superseded. `onSimulationPause` and `onSimulationResume` both
fire on this build.

The paused maximum of 1.76 s is a teardown stall credited to the paused phase,
not paused idle (see Provenance). Do not build a timeout on an average, but do
not expect a paused sim to be slow either.

### 9.4 Epochs

The hook driver and the broker survive a mission reload. The sim driver and
every unit handle do not. Every frame carries its epoch. A consumer discards
frames from a closed epoch. A frame with no epoch field is not epoch-scoped and
is never discarded by this rule (Section 5.2).

The injected environment itself survives a mission reload (measured,
2.9.29.27278; Section 5.1.2), and **so does a `world` event handler registered
from it**: a handler registered during one mission went on to receive 19 events
in 88.5 seconds from the *next* mission's world, and 2,677 events across that
mission in total. Nothing tears a Route A sim driver down for free.

The epoch rule is therefore the sim driver's to enforce: teardown (Section 6.3)
and the load-time stamp check (Section 6.1) do the discarding, not the
platform. Two consequences follow from the handler surviving, and both are
already handled. Re-arming logic guarded by `rawget(_G, ...)` silently no-ops
after a mission change, which is why Section 6.1 checks its own stamp rather
than mere presence. And an unguarded registration would double-register, which
is why Section 6.2 registers exactly once and never touches the registration
again.

**Mission time restarts while the globals persist.** `timer.getTime()` read
51.9 s in the new mission against a stored 20.0 s from the old one. Any
baseline held across an epoch boundary is meaningless, which is what Section
6.3's rule that the sim driver holds no state across an epoch exists to
prevent.

### 9.5 The bridge's own commands

The bridge defines these and no others. Everything else in the schema belongs
to the sim driver built-ins (SIM §1) or to a consumer's own vocabulary.

The distinction is load-bearing, and it is exceptionless: **no bridge-defined
command reaches the mission-scripting world API.** The bridge's vocabulary is
control plane — it configures, reloads, acknowledges and brackets the stream.
Everything a mission can observe or do belongs to a sim driver's vocabulary.
The sim driver built-ins are a default vocabulary shipped alongside the bridge,
not part of it, and the bridge runs with it replaced or absent — Section 11's
injection-failure row is the no-sim-driver case. With no sim driver, the
broker-answered pairs, the hook-driver-emitted lifecycle records, the
hook-driver-targeted topics, the hook-driver-handled commands
(`ReloadSimDriver` and `ReloadConfig`) and the broker-handled `SetEnabled`
still work. **`EpochOpened` and `EpochClosed` are among them** (Section 6.3), so a consumer gets its epoch
anchor and its time pair on every configuration, and the Section 5.2 retention
replay has the boundary record it promises a late joiner. What a consumer loses
with no sim driver is world content, not epoch structure.
`CoordinateCalibration` is the hook driver's too (Section 6.3), so positions in
hook-driver-emitted records stay convertible with no sim driver loaded.
Sim-driver-bound commands are then not dispatched: their topics are routed, so
the broker accepts them, and no component can decode a correlation id to refuse
them with — the consumer's `request_timeout_ms` covers the silence. A command
whose topic the running sim driver does not know is the same silence, counted
in `unknown_topic_total`. A topic in no route map at all is refused with
`Rejected` and counted in `unrouted_topic_total` (Section 5.2).

**The two counters are separate because the two conditions are.** An unrouted
topic means a consumer sent something no build of this bridge routes, and the
sender learns so from `Rejected`. An unknown topic means the broker routed a
record that the component behind the route could not handle, and the sender
learns nothing. One counter for both would leave an operator unable to tell a
misbehaving consumer from a sim driver a release behind.

| Command | Class | Purpose |
|---|---|---|
| `Resync` | `COMMAND` | Requests initial state. Section 6.8. |
| `SeqAck` | none | Consumer to broker. Highest durably processed `seq`. Never reaches Lua, so no class policy applies. Section 11. |
| `ReloadSimDriver` | `COMMAND` | Replaces `DCSBridge.code`. Needs the `reload` capability. Section 6.9. |
| `SetEnabled` | none | The kill switch: sets the effective `enabled` value. Handled by the broker and never reaches Lua — a disabled sim driver could not receive its own re-enable. Needs the `reload` capability. Section 11. |
| `ReloadConfig` | `COMMAND` | Re-reads `Config\DCSBridge.lua` and applies the live keys. Needs the `reload` capability. Section 13.2. |

`Ping`, `Auth`, `GetSchema`, `GetTopics`, `SetTopicFilter`, `Rejected` and the
other broker-answered messages are in Section 5.2. Of the commands above,
`Resync` is
sim-driver-targeted; `ReloadSimDriver` and `ReloadConfig` are
hook-driver-targeted (Section 8.2), handled where the work is; `SeqAck` and
`SetEnabled` are broker-handled and reach no ring (Section 5.2).

Section 8.5.3 states what answers each command: every command that reaches Lua
is answered by exactly one point-to-point record. `SeqAck` and `SetEnabled` sit
outside that contract and are answered by nothing.

### 9.6 Sim driver injection

Inject the sim driver from disk, not from the `.miz`. The bridge then works
with any mission the server loads. A sim driver edit costs a mission reload,
not a DCS restart.

---

## 10. Budgets

Measure in microseconds of logic-thread time per frame.

| Channel | Budget | Measured |
|---|---|---|
| Emit path: put calls and commit | | |
| Record drain, sim driver to broker | | |
| Command poll and dispatch | | |
| **Subscription evaluation** | | |
| **Spot updates** | | |
| **Weapon tracking** | | |
| **Event handler time** | | |
| Resync slice | | |
| Eval poller, directory scan | | |
| Eval execution | | |
| **Total, steady state** | **5% of a frame, provisional** | |
| **Total, burst** | **10% of a frame, provisional** | |

**The budget is a share of a frame, not a fixed microsecond figure.** Section
6.4 measures 68 Hz at the menu and 72 Hz in a hosted mission, so a frame is
13.9 to 14.7 ms and a fixed figure silently means different things across that
range. At 70 Hz, 5% is about 715 µs and 10% is about 1.4 ms.

**Steady state and burst are separate numbers because the drain is bursty.** A
frame that drains a full `bridge_return_max_bytes` costs several times one that
drains a few records, and Section 5.2's measured frame gaps mean a drain
sometimes carries seconds of buffered records rather than one frame's worth.
Budgeting only the average would make the cap meaningless; budgeting only the
peak would waste the frame. Section 12 alerts on each separately.

**Route A and Route B do not cost the same.** Route A pays the
`net.dostring_in` text crossing *and* the put calls, because Section 5.3 has
the hook driver replay the returned log into the shim. Route B pays the put
calls only. Modelled over a heavy load — 500 units, 3,040 records per second —
Route A came out near 2.4× Route B. A single figure cannot cover both, so the
shares above are Route A's; Route B should sit well inside them.

The totals are provisional until **[PROBE-3]** completes their measured basis.
The frame-gap half is now measured (Provenance), and it is favourable: at a 282
ms worst-case stall the burst share covers a full drain comfortably. Its
derivation is an incumbent's shipped figure: DCS-gRPC ships a configurable
limit on calls per second executed inside the mission scripting environment,
defaulting to 600. That confirms independently that a cap is necessary, and it
is a starting figure from a project with production deployments behind it.
DCS-gRPC sustains 600 mission-scripting calls per second in production; at the
measured 68–72 Hz callback rate (Section 6.4) that is roughly nine calls per
frame, and 500 µs prices those calls plus one drain round trip with an order of magnitude of
margin over the 0.010 ms minimal figure. The measured put-call and ring figures
fill the per-channel rows once their probes report.

Two measured figures bound the table (2.9.29.27278). A generic Lua-to-C
crossing proxies at 0.6 to 0.85 µs — the put-call figure itself needs the
broker (**[PROBE-3]**) — so ten put calls per frame are cheap and a full
drain's several hundred are not free. And `net.dostring_in` costs 30 to 40 µs
per KB, so a full 16 KiB drain crossing alone costs about 0.5 ms: the 500 µs
total holds only while the drain runs well under its byte cap. Treat a full
drain as a burst ceiling, not a steady state (Section 13.1).

- Cap the drain loop by record count and by byte count. Section 13.1 names
  both. They are not independent: at `drain_max_bytes` of 16 KiB, the record
  cap of 256 is reachable only for records averaging 64 bytes or less.
  Whichever binds first, binds.
- The drop policy is Section 5.2 and Section 8.1: `LOSSY` before `DURABLE`,
  `LIFECYCLE` never.
- Subscription evaluation is the row most likely to grow without anyone
  noticing. A mission with 200 units and 30 per-frame subscriptions performs
  thousands of interpreted evaluations per frame.
- Cap active subscriptions at `max_subscriptions`. Every cap in Section 6.4 has
  a default in Section 13.1.

---

## 11. Failure modes

| Failure | Detection | Degraded behaviour | Recovery |
|---|---|---|---|
| No consumer connected | No connection | Mission unaffected. Records discarded. | Consumer connects |
| Consumer redeployed | Connection closed | Records drop once its ring fills | Reconnect |
| Broker fails to load | `assert` on `package.loadlib` | Hook driver logs and disables itself. Mission unaffected. | Restart DCS |
| Hook driver/broker version mismatch | Version check at first `configure` | Hook driver logs both versions and disables itself. Mission unaffected. | Finish the update, restart DCS |
| Sim driver injection fails | No `"OK\|"` prefix, or a `failed:` body | Hook driver logs it and runs with no sim driver | Next mission load |
| `net.dostring_in` refused by policy | Distinct refusal, not nil and not `"Invalid state name"` | Hook driver logs the exact `autoexec.cfg` needed and disables itself. **No retry loop.** Mission unaffected. | Operator merges the config, or installs Route B |
| Route B `dofile` line lost to a DCS update | No `SimDriverLoaded` after `MissionLoaded`, `sim_driver_route` absent from `stats` | Runs with no sim driver, exactly the Section 9.5 case. **Nothing in DCS reports the edit was reverted.** | Operator reapplies the edit; `doctor` names it |
| Third party's event handler raises | **Undetectable from inside the sim driver** | Events stop arriving while frames continue. `world.onEvent` aborts dispatch for that event. | None. Section 4.1. |
| Reload source fails to compile | Compile-before-execute | Running sim driver untouched. `ReloadSimDriver` acknowledged with the error. | Fix the file |
| Reload raises during load | Raise in step 4 or 5 | Hook driver re-injects the previous source | Automatic |
| Eval script raises | `pcall` | Input renamed `.failed`; reason in the result log. Sim driver keeps running. | Fix the file |
| Eval script exceeds budget | Count hook | Same as a raise | Raise `eval_instruction_budget` or fix the script |
| Eval script takes DCS down | `.running` left on disk | **Does not re-execute.** Startup sweep logs it. | Operator inspects and deletes |
| Handler raises | Per-handler `pcall` | Handler disabled for the epoch, named in a record | Next epoch |
| `handler_failures_per_epoch` failures in one epoch | Counter | Sim driver disabled for the epoch. Hook driver and lifecycle records continue. | Next mission load |
| Error in a DCS callback | Outer `pcall` | Logged once per callback per epoch, then suppressed. The callback returns. The session survives. | Automatic, next invocation |
| Outbound ring saturated | Drop counter | Oldest non-`LIFECYCLE` record evicted. `LIFECYCLE` survives. | Alert |
| `LIFECYCLE` record over slot size | `lifecycle_oversize_total` | Refused at `commit`. The topic keeps its previous retained record. | Fix the schema; `doctor` names the topic |
| Inbound ring saturated | `commands_rejected_total` by reason | Newest command dropped, sender answered with `Rejected` reason `BUSY` up to `busy_max_per_sec`. | Sender waits for `MissionLoaded`, or backs off |
| Ring saturated with `LIFECYCLE` alone | `lifecycle_disconnects_total` moves | **Connection dropped, record never discarded.** Section 5.2. | Consumer reconnects; fresh handshake and `seq` origin |
| Mission load blackout | `MissionLoadBegan`, then silence | **Not a fault.** Consumer waits for the load timeout. | `MissionLoaded` |
| DCS logic thread wedged | Heartbeat gap, `dcs_alive` false | Reader thread still answers `Ping` and reports DCS down | DCS restarts |
| Mission reload during reconnect | Epoch mismatch | Consumer discards stale frames | Automatic |

The first row is the contract. **The mission runs whether or not a consumer is
connected.**

**Silence after `MissionLoadBegan` is not a fault** until `load_timeout_ms`
expires. The default is two minutes, not the shortest observed load. Observed
loads on one machine ranged from 20 s to 60 s. A false "DCS is dead" is worse
than a slow alarm.

**Kill switch.** `enabled` in `Config\DCSBridge.lua`, applied like any live
key: at load, by `ReloadConfig`, or by `SetEnabled`, which the broker handles
itself so a re-enable arrives even while dispatch is down. Disabled means: the
hook driver stops injection, drain, command ferrying and eval; the sim driver
is torn down with every `DCSBridge.resources` handle released; the broker keeps
the listener and `Ping`, and `Pong` carries the effective `enabled` value, so a
connected consumer sees a disabled bridge rather than a dead sim. Lifecycle
records stop with the hook driver.

**The hook driver keeps calling `shim.tick` while disabled.** It is the only
caller under either route (Section 5.2), so stopping it would let the heartbeat
go stale and make `Pong` report `dcs_alive` false — a dead sim, which is the
exact reading the kill switch exists to avoid. Ticking is one atomic store per
frame and carries no records. `doctor` reports the effective state, and a file
value that differs from it, like any other key (Section 13.2). No DCS restart
at any point.

**Replay spool.** The broker may append `DURABLE` records to an on-disk spool.
On reconnect it replays from a consumer's last acknowledged `seq`.
`spool_max_bytes` and `spool_retention_hours` bound it. Optional, and not in
the first release.

The acknowledgement it needs is `SeqAck`, a consumer-to-broker record carrying
the highest `seq` that consumer has durably processed. It never reaches Lua. A
consumer that does not send it gets no replay, which is the current behaviour,
so adding the spool later breaks nothing. Specify the record now so the wire
does not change when the spool ships.

---

## 12. Observability

| Metric | Purpose |
|---|---|
| `logic_thread_us_per_frame`, p50 / p99 / max | Primary health signal |
| `bridge_calls_per_frame` | Detects a broken batching rule |
| `subscriptions_active` | The cost that grows unnoticed |
| `subscription_eval_us`, per subscription | Attributes that cost |
| `spots_active`, `spot_update_us` | Laser and infrared tracking cost |
| `weapons_tracked`, `weapon_track_us` | Unbounded without a filter |
| `handler_us`, per handler | Same, for event handlers |
| `stage_deferred_total`, per stage | A cap being hit every frame |
| `sim_driver_buffer_dropped_total` | The drain stall outran `sim_driver_buffer_max_records`. If this moves, emission rate is the problem. |
| `commands_rejected_total`, by reason | A cap refusing work, told apart from a cap silently dropping it |
| `lifecycle_oversize_total`, by topic | A `LIFECYCLE` record too large to retain (Section 5.2). Should never move: it means a schema defect, not load. |
| `rejections_suppressed_total`, by connection and reason | Refusals the `Rejected` rate caps withheld. Without it `commands_rejected_total` cannot say whether the sender was told. |
| `handlers_disabled_total` | Section 6.5 containment firing |
| `eval_executed_total`, `eval_failed_total` | Operator eval activity |
| `reloads_total`, `reloads_cold_total` | Reload activity, and how often state is lost |
| `sim_driver_code_sha256` | Which source is running now. The same value `SimDriverReloaded.code_sha256` carries. |
| `ring_depth_out` / `ring_depth_in`, current and max, `ring_depth_in` per ring — `.sim_driver`, `.hook_driver` | Sizing and backpressure |
| `records_dropped_total`, by class and connection | Correctness. `LIFECYCLE` must never appear here. |
| `records_filtered_total`, by connection and cause | Outbound records withheld by the capability filter (Section 14.4) or by the connection's topic filter (Section 5.2). Never counted as drops: filtering happens before `seq`, so it leaves no gap. The cause separates a disclosure rule from a consumer's own choice. |
| `hook_driver_dispatch_deferred_total` | The hook driver loop's cap being hit every invocation (Section 6.4). |
| `lifecycle_disconnects_total`, by connection | A consumer dropped rather than lose a boundary record. Section 5.2. |
| `lifecycle_replayed_total`, by connection | Retained `LIFECYCLE` records delivered at authentication. Section 5.2. |
| `records_uncommitted_total` | An error path inside a record |
| `record_lag_ms`, emit to consumer receipt | End-to-end health |
| `drain_gap_ms`, max | The measured tail. See below. |
| `epoch_id`, `sim_driver_route`, `sim_driver_direct_broker` | Lifecycle correctness. `sim_driver_route` and `sim_driver_direct_broker` match the `SimDriverLoaded` field names. |
| `unrouted_topic_total` | An inbound topic in no route map, refused by the broker with `Rejected` (Section 5.2). The sender was told. |
| `unknown_topic_total` | A routed topic the running sim driver or hook driver does not know, dispatched nowhere (Section 9.5). The sender was not told, and waits out `request_timeout_ms`. |
| `partial_registration_total` | A topic registered in some of the class, route and capability tables and not the others (Section 5.1). Always a mismatched pair of generated files. |
| `misaddressed_total` | A `begin_to` on a topic that is not a reply or an acknowledgement, refused (Section 5.1). Always hand-written Lua. |
| `config_keys_pending_restart` | Edited in the file, not yet in force. Section 13.2. |
| `config_keys_unknown_total` | A config written for a different build |
| `connection_token_id.<conn>` | The id of the token that authenticated the connection. The hook driver reads it to stamp audit lines (HOOK §9). |
| `callback_hz` | Render-loop health |
| `harvest_preempted_total` | A veto-callback harvest did not arrive and `net.get_player_info` answered instead (HOOK §3) |
| `player_info_nil_total` | The `get_player_info` fallback returned nil and a field was omitted (HOOK §3) |
| `admin_commands_total`, by command and outcome | Moderation activity (HOOK §6) |
| `bounces_suppressed_total` | Self-caused slot changes recognised (HOOK §8) |
| `bounces_refused_total` | A second bounce inside `bounce_min_interval_ms` refused as a fault (HOOK §8) |
| `admin_audit_write_failures_total` | The admin audit could not be written; the command proceeded (HOOK §9) |
| `roster_entries` | Current player count as the hook driver's map sees it (HOOK §3) |
| `sim_driver_files_failed_total` | A sim-driver-side extension file skipped at load; the name is in the log (Section 6.10) |
| `hook_driver_files_failed_total` | The same, hook driver side |

**`record_lag_ms` expectations must match measured behaviour.** Across 129,497
running-phase gaps, 99.3% are 20 ms or under, p99.9 is under 50 ms, and the
maximum is **282 ms** (Provenance). Tens-of-milliseconds lag is therefore the
right expectation during a running mission, which an earlier 22.5-second sample
had wrongly ruled out.

**Alert on the transitions, not on the steady state.** A mission load is a
total frame blackout of tens of seconds (Section 9.1), telegraphed by
`MissionLoadBegan`. A mission teardown stalls 1 to 2 s and is telegraphed by
nothing. Those are what a lag alert will actually fire on, so set the threshold above them or gate it on
`MissionLoadBegan` — and remember these are lower bounds from single player,
not a loaded server (**[PROBE-10]**).

Build these before any optimisation.

---

## 13. Layout and versions

```
Saved Games\<write dir>\
  Scripts\Hooks\DCSBridge.lua     Thin loader, ENABLED flag, registers callbacks
  Mods\services\DCSBridge\
      bin\lua-dcsbridge.dll       The broker
      schema.pb                   Compiled schema. Read by the hook driver at start. Section 8.3.
      lua\HookDriver.lua          Hook driver runtime. Bridge-owned.
      lua\HookDriver.gen.lua      Generated hook-driver-side file. Section 8.3.
      lua\HookDriver.builtin.lua  The hook driver built-ins' handlers. HOOK §1.
      lua\SimDriver.lua           Sim driver runtime, loaded into "server". Bridge-owned.
      lua\SimDriver.gen.lua       Generated sim-driver-side file. Section 8.3.
      lua\SimDriver.builtin.lua   The sim driver built-ins' handlers. SIM §1.
      eval\server\                Operator eval, "server" state. Created by the operator.
      eval\hook\                  Operator eval, hook state. Created by the operator.
  DCSBridge\simdriver.d\          Adopter extension files, sim driver side. Created by the operator. Section 6.10.
  DCSBridge\hookdriver.d\         Adopter extension files, hook driver side. Created by the operator. Section 6.10.
  Config\DCSBridge.lua            Bind address, port, tokens, budgets, caps, route
  Logs\DCSBridge\
      DCSBridge.log               The bridge's own log. Rotated. Section 13.1.
      eval-audit.log              Append-only. Section 7.6.
      admin-audit.log             Append-only, one line per admin command. HOOK §9.
      eval\<stem>.<UTC>.log       One per execution. Section 7.3.
```

- Resolve the write directory from `lfs.writedir()`. Never hard-code it: its
  name is set at launch and is not one of a fixed set of values.
- The tree has five parts: the loader under `Scripts\Hooks\`, everything the
  bridge ships under `Mods\services\DCSBridge\`, an adopter's own files under
  `DCSBridge\`, settings under `Config\`, and everything the bridge writes
  under `Logs\DCSBridge\`. Section 5.1.1 gives the reasoning for the second.
  All five sit under the write directory, so installing is still one extraction
  plus the `autoexec.cfg` merge.
- **The bridge tree holds shipped and generated files only, and holds no
  directories below `lua\`.** A release overwrites every file under
  `Mods\services\DCSBridge\lua\` unconditionally, so an adopter file placed
  there is lost at the next update. Adopter files live under `DCSBridge\`
  instead, which a release never touches. Section 6.10 states the rule and PLAN
  §2 states how `doctor` reports a breach of it.
- Uninstalling is the reverse: delete the loader and the
  `Mods\services\DCSBridge\` tree, and `DCSBridge\`, `Config\DCSBridge.lua` and
  `Logs\DCSBridge\` if wanted. Leave `autoexec.cfg` alone: the state lists are
  a union other tools may still need, and a stale entry enables nothing by
  itself.
- `Scripts\Hooks\` is a flat glob loaded at DCS startup. DCS globs `*.lua`,
  sorts by name, and loads each file into its own environment inside the shared
  GUI Lua state. Callbacks then fire in load order, registered with
  `DCS.setUserCallbacks`. ED documents only the write directory's tree, but on
  the measured install it globs **two**: the install's `Scripts\Hooks\` first,
  then the write directory's, with uppercase filenames before lowercase.
  Neither the second tree nor the collation is documented, so do not depend on
  load order. Use a thin loader plus a payload, so the file that needs a
  restart is the file you rarely edit.
- Put no version number in a filename. Both copies would load and both would
  register callbacks. Identify the running build from the file's own
  modification time and size.
- Under Route A, require no edit to the DCS install directory. Place no file
  under `Program Files`. Only the loader goes anywhere DCS loads from, and it
  is the one file that has to. Everything the bridge writes at runtime goes to
  `Logs\DCSBridge\`, which DCS never reads.
- Under Route B, add a `dofile` to `Scripts\MissionScripting.lua` before the
  sanitisation block. DCS updates overwrite that tree, so reapply the edit
  after every patch. Document this as the cost of the degraded route.

- **Load the module with `package.loadlib` and a full path, never `require`.**
  `package.cpath` is not set in the hook state: `Scripts\UserHooks.lua` ships
  that line commented out. `require('lua-dcsbridge')` will not find the DLL on
  any install. Build the path from `lfs.writedir()`.
- Build paths with forward slashes in ordinary quoted strings.
- Write a load banner before registering anything, to your log and to
  `dcs.log`. No banner means the file never parsed. A banner and then silence
  means a callback does not fire.

### 13.1 Configuration defaults

Every limit in this document has a name, a default, and a basis. The defaults
are sized for the first deployment in Section 1.3 and are all configurable. An
adopter with a fifty-player server raises them. Nothing in the design assumes
these figures, and **nothing here needs a new binary to change** — Section 13.2
gives the cost of applying each one.

Values marked **provisional** are placeholders until a measurement replaces
them. Where a probe covers the figure, the probe is named.

**Connections and framing** — owner **broker**. **Live**, except
`bind_address`, `port`, `allow_public_bind` and `max_connections`, which are
**DCS restart**: the listener binds once, `allow_public_bind` is what decides
whether it may, and the per-connection queue array is allocated with
`max_connections`.

| Key | Default | Basis |
|---|---|---|
| `enabled` | true | The kill switch. **Live** — applied at load, by `ReloadConfig`, or by `SetEnabled`, which the broker handles itself. Section 11. |
| `bind_address` | `127.0.0.1` | Loopback by default. Section 14.3. |
| `rejected_max_per_sec` | 10 | Caps `Rejected` emission per connection for `UNKNOWN_TOPIC`, `NO_CAPABILITY` and `RATE_LIMITED` (Section 5.2), so a refused flood cannot buy amplification. All three indicate a misbuilt or misbehaving consumer, so 10/s is well above any legitimate rate. Refusals above it move `rejections_suppressed_total`. |
| `busy_max_per_sec` | 100 | Caps `Rejected` reason `BUSY` per connection, separately from the row above, because `BUSY` answers a well-behaved consumer rather than a misbehaving one. Set to `inbound_records_per_sec` so every refused command can be answered: a `BUSY` replaces a record the broker already discarded, so it adds no traffic the consumer did not already generate, and the inbound cap bounds both. |
| `port` | 7742 | A free choice, fixed so `doctor` and the CLI need no argument. Nothing on a reference install binds it. It is clear of the numbers DCS tooling puts on the wire nearby — Tacview's telemetry port defaults to 42674, and SRS advertises 5002 — but that is a sanity check, not a survey. Change it on a collision. |
| `allow_public_bind` | false | A non-loopback, non-private bind address warns and refuses to listen unless this is set. Section 14.3. |
| `route` | `A` | Which injection route the hook driver uses. Section 5.4.1. Owner **hook driver**, and **DCS restart**, because the hook driver decides at load. |
| `tokens` | none | One entry per consumer: an id, a secret and a capability set. The id names the token in `stats` and in audit lines and is never the secret (Section 12 and HOOK §9). Section 14.4. **Live** — rotation must not need a restart. |
| `sim_driver_path` | `Mods\services\DCSBridge\lua\SimDriver.lua` | Section 14.6 forbids a record naming it, so it comes from config only. Owner **hook driver**, tier **mission reload**. |
| `max_connections` | 8 | A bot, a map, a stats collector, and headroom |
| `max_unauthenticated_connections` | 4 | Half the total, so a slowloris cannot exhaust the pool. Must stay below `max_connections`. |
| `handshake_timeout_ms` | 5000 | A connection that has not authenticated in 5 s is not going to |
| `max_frame_bytes` | 1048576 (1 MiB) | Each record is its own frame, so the largest legitimate frame is a single large reply — a `Schema` reply, near 100 KB — and this is about 10× that. **Not** sized by resync, which sends many small frames. |
| `max_type_url_bytes` | 256 | Caps the payload type URL the reader thread reads out of every inbound `Any` (Sections 5.2 and 14.2). A real one is about 43 bytes — `type.googleapis.com/` plus a package and a message name — so this is generous headroom over any legitimate name. It is attacker-controlled and it sizes a read, which is the same reason `max_frame_bytes` exists. |

| `inbound_records_per_sec` | 100 per connection | Well above any legitimate command rate. `max_connections` × this stays under aggregate dispatch capacity. |
| `inbound_records_per_sec_total` | 400 | `dispatch_max_commands` × the measured callback rate of 68–72 Hz (Section 6.4) is roughly 2200/s of theoretical capacity, and that assumes the sim driver does nothing else. 400 leaves the frame for its actual work. |
| `auth_failures_per_min` | 5 per source address | Not a defence against brute force — a token of any reasonable length makes that irrelevant. It bounds log noise and hash cost. |

**Rings** — owner **broker**, tier **DCS restart**, because Section 5.2
allocates every ring at the first `configure` and never allocates again. Two
exceptions: `sim_driver_buffer_max_records` is the sim driver's and applies at
a **mission reload**, and `ring_out_lifecycle_reserve` is a watermark tested on
push rather than an allocation, so it is **live**. The four ring keys are
provisional until **[PROBE-7]**. `sim_driver_buffer_max_records` is not: a
drain stall backs up in that buffer rather than in a ring.

| Key | Default | Basis |
|---|---|---|
| `sim_driver_buffer_max_records` | 8192 | Sized against the measured worst case with three orders of magnitude of margin. The largest running-phase stall is 282 ms and the largest measured event rate is 23.3/s, so a worst-case stall buffers about ten records (Provenance). The default is kept large because memory is cheap and a dedicated server is unmeasured (**[PROBE-10]**), not because ten is in doubt. `sim_driver_buffer_dropped_total` should never move; Section 6.4 says what it means if it does. |
| `ring_out_records` | 4096 per connection | **Provisional until [PROBE-7].** Sizes against a consumer outage, not a drain stall — the drain stall is now measured at 282 ms and is not what fills this ring. A stall backs up in the sim driver buffer above, because the broker is not being fed at all. This ring fills only when the socket cannot absorb what the writer thread offers. |
| `ring_out_lifecycle_reserve` | 64 | Slots each outbound ring keeps free for `LIFECYCLE`. A few epochs of boundary records plus the periodic topics — `CallbackHz` alone adds one per second while a consumer stalls. A ring holding nothing else is a disconnect, not a drop. Section 5.2. **Provisional until [PROBE-7]**, because it partitions a ring whose size is also provisional. |
| `ring_in_sim_driver_records` | 1024 | The sim driver's half of the inbound split (Section 5.2). About 2.5 s of `inbound_records_per_sec_total`. Commands arriving during a mission-load blackout are mostly meaningless anyway, and drop-newest discards them in the right order. |
| `ring_in_hook_driver_records` | 256 | Hook driver traffic is human-paced — moderation and control commands — and needs far less than the sim driver's ring. |

**Sim driver per-frame caps** — owner **sim driver**, tier **mission reload**
or a `ReloadSimDriver` command. Section 6.4.

| Key | Default | Basis |
|---|---|---|
| `drain_max_records` | 256 | Paired with `drain_max_bytes`; whichever binds first, binds. 256 is reachable only for records averaging 64 bytes or less. |
| `drain_max_bytes` | 16384 (16 KiB) | A burst ceiling, not a steady state: no protocol ceiling exists, and at the measured 30–40 µs/KB a full drain costs about 0.5 ms of logic-thread time. Section 5.3. |
| `bridge_return_max_bytes` | 32768 (32 KiB) | Returns measured byte-exact to 1 MiB (Section 5.3); 32 KiB keeps a return's cost inside the frame budget. |
| `dispatch_max_commands` | 32 | Roughly 2200/s at the measured 68–72 Hz callback rate (Section 6.4), the ceiling `inbound_records_per_sec_total` sits well below. |
| `subscription_max_evals` | 64 | **A structural cap, held non-binding by invariant.** A subscription is evaluated at most once per frame (Section 6.7), so evaluations due never exceed subscriptions active, which never exceeds `max_subscriptions`, which Section 13.2 holds at or below this. It therefore cannot fire under any valid configuration. It exists so that Section 6.4's rule — every bulk stage has a cap — has no exception, and so the guard is already in place if a future change ever lets a subscription evaluate twice in a frame. The real bound on work is `max_subscriptions` times each predicate's own cost, and `subscription_eval_us` is the instrument for it. `doctor` marks it non-binding. |
| `spot_max_updates` | 16 | Equal to `max_spots`. A structural cap held non-binding by invariant, exactly as above. Contrast `weapon_max_samples`, which is deliberately half its resource cap and therefore does throttle. |
| `weapon_max_samples` | 32 | **Half** of `max_tracked_weapons`, so this cap does throttle. A full tracker samples each weapon every other frame, around 35 Hz — ample for a missile at 1 km/s, where sampling every frame buys nothing. |
| `resync_slice_records` | 32 | A 500-unit resync completes in 16 frames, about 0.3 s |

**Sim driver resource caps** — owner **sim driver**, tier **mission reload** or
a `ReloadSimDriver` command. `unsafe_bindings_enabled` gates a call site rather
than the Section 4.2 probe, so it applies on the next dispatch after a reload.

| Key | Default | Basis |
|---|---|---|
| `max_subscriptions` | 64 | Sets `subscription_max_evals`, which matches it so the per-frame cap never binds |
| `max_spots` | 16 | One laser per objective is already a busy mission |
| `max_tracked_weapons` | 64 | Twice `weapon_max_samples`, so a full tracker samples each weapon on alternate frames |
| `convert_max_points_per_command` | 256 | Each point is one `coord.LOtoLL` call on the logic thread. **Provisional**: the per-call cost is unmeasured, and this figure assumes single-digit microseconds. Measure it before trusting the number. |
| `recent_idempotency_keys` | 1024 | Matches `ring_in_sim_driver_records`. The window must be at least as deep as the queue that can hold duplicates. A shallower set evicts a key before its duplicate arrives, which is the one case the set exists for. |
| `command_ack_detail_max_bytes` | 256 | Truncation bound for `CommandAck.detail` (Section 8.5.3). The emitting handler truncates; an acknowledgement never fails for its own detail. Far under `max_frame_bytes`. |
| `spawn_max_groups_per_command` | 8 | The Section 14.5 sanity limit, made concrete. **Provisional and deliberately low**: spawn cost per group is unmeasured, and 32 in one frame risks a visible hitch. A consumer needing more sends more commands; the dispatch cap spreads them. |
| `handler_failures_per_epoch` | 3 | Section 6.5 |
| `unsafe_bindings_enabled` | false | Gates a handler that calls a binding from the Section 4.2 crasher register or an untested member of a suspect family. SIM §8. |

**Operator eval** — owner **hook driver**, tier **live**. An operator changing
these is usually mid-incident. `eval_instruction_budget` is validated on every
apply, not only at load, per Section 7.5. Section 7.

| Key | Default | Basis |
|---|---|---|
| `eval_max_files_per_poll` | 1 | Files dropped during the mission-load blackout all become due on the first frame afterwards. Draining across polls keeps drop order as execution order without a burst. |
| `eval_max_file_bytes` | 65536 (64 KiB) | An operator script above this is a program, not a fix |
| `eval_log_max_bytes` | 268435456 (256 MiB) | Total size of `Logs\DCSBridge\eval\`. Binds before age, per the spool's precedent: bounded disk is a hard requirement. Delete oldest first. |
| `eval_log_retention_days` | 30 | An upper bound on age, applied after the size cap. |
| `eval_stable_polls` | 2 | Size unchanged across this many polls. A count of polls is meaningful; a millisecond threshold below LFS's one-second mtime resolution is not. |
| `eval_instruction_budget` | 10000000 (1e7) | Generous. The count hook costs something every budget instructions, so a small value is its own tax. Section 7.5. |
| `eval_audit_max_bytes` | 16777216 (16 MiB) | Bounds `eval-audit.log`, rotated, oldest rotated file deleted first. One line per execution attempt, so at the eval poll rate the cap is years away; the row exists because a limit with no default is a defect. Section 7.6. |
| `eval_audit_retention_days` | 90 | An upper bound on age, applied after the size cap. Longer than the result tree's 30: an audit answers a question asked months later. |

**Hook driver dispatch** — owner **hook driver**, tier **live**. Section 6.4's
hook driver loop.

| Key | Default | Basis |
|---|---|---|
| `hook_driver_dispatch_max_commands` | 8 | Per invocation, not per frame — the loop also runs from the player-event callbacks. Hook driver traffic is human-paced. |

**Extensions** — Section 6.10. The two file lists are read by the loader that
enumerates the source, so each takes that source's tier. `options` is sim
driver data and takes the sim driver tier.

| Key | Default | Basis |
|---|---|---|
| `sim_driver_disabled_files` | empty | Names of shipped sim-driver-side files the loader skips, `SimDriver.builtin.lua` among them. Durable suppression: a release restores a deleted file, and this survives it. Owner **hook driver**, tier **mission reload**. |
| `hook_driver_disabled_files` | empty | The same for the hook driver side. Owner **hook driver**, tier **DCS restart**, because hook-driver-side files load once per DCS process (Section 3.1). |
| `options` | empty | Built-in and extension settings, merged into `DCSBridge.code.options` before any extension file loads. A nested table, like `tokens`; the loader passes it through unread beyond a shape check, because its keys belong to the files that read them. An unknown key is the reading file's business to log. Owner **sim driver**, tier **mission reload**. |
| `mission_sim_driver_dirs` | false | Enumerate a mission's own `dcsbridge\` directory and load that mission's sim driver files (Section 6.10). Off by default because those files run unsanitised in the sim driver environment — Section 14.6. Route A only. Owner **hook driver**, tier **mission reload**. |

**Hook driver built-ins** — owner **hook driver**, tier **live**. HOOK §1.

| Key | Default | Basis |
|---|---|---|
| `roster_max_players` | 128 | Above any DCS server's practical population; bounds the `Roster` reply. |
| `recent_admin_keys` | 256 | Matches `ring_in_hook_driver_records`: the window must be as deep as the queue that can hold duplicates, and Section 13.2 checks the pair. Bounds the twelve mutating commands only. |
| `kick_message_max_bytes` | 256 | A consumer string DCS puts on the wire to a client. Over the cap fails the command (HOOK §6). |
| `ban_reason_max_bytes` | 256 | The same. The reason is stored in the ban record. |
| `chat_message_max_bytes` | 512 | The same. `SendChatAll` reaches every player. |
| `mission_briefing_max_bytes` | 16384 (16 KiB) | Bounds the `GetMissionBriefing` reply (HOOK §6). The largest briefing in ED's shipped missions is 924 bytes, so this is generous headroom over measured content while staying far under `max_frame_bytes`. Outbound and author-written, so the hook driver truncates and flags rather than failing — the opposite of the consumer-string rule above, for the opposite direction of travel. |
| `idempotency_key_max_bytes` | 64 | The hook driver stores `recent_admin_keys` of them. |
| `slotlist_max_entries` | 512 | Bounds one `SlotList` record under `max_frame_bytes` (HOOK §4). |
| `banlist_max_entries` | 512 | Bounds one `Banlist` record; the set fans out in chunks, like `SlotList` (HOOK §6). |
| `bounce_min_interval_ms` | 2000 | HOOK §8. The re-fire is measured (HOOK §8); the interval guards against a consumer loop and is a policy choice. |
| `admin_audit_max_bytes` | 16777216 (16 MiB) | Bounds `admin-audit.log`, rotated, oldest rotated file deleted first — the eval audit's twin (Section 7.6 and HOOK §9). |
| `admin_audit_retention_days` | 90 | An upper bound on age, applied after the size cap, matching `eval_audit_retention_days`: an audit answers a question asked months later. |

`ban_period_max_seconds` is deliberately absent. A permanent ban is a very
large period — ED's "ban forever" checkbox writes the sentinel `16293600000`
seconds, about 516 years, in `MissionEditor\modules\mul_banned.lua`, and passes
it straight to `net.banlist_add` — and a cap would break the case an operator
most wants. Validate the period as a non-negative integer of seconds instead
(HOOK §6). Note that the sentinel exceeds 2^31 and must not be carried in a
32-bit field.

**Timing** — owner **broker** or **hook driver**, tier **live**. A timeout or a
threshold is read as a number at decision time, so an apply swaps it with no
restart. `request_timeout_ms` and `load_timeout_ms` are the exceptions: the
consumer owns both, and Section 13.2 says how it gets them.

| Key | Default | Basis |
|---|---|---|
| `heartbeat_interval_ms` | 1000 | Thirty intervals fit inside `dcs_alive_threshold_ms`, so a verdict never rests on one missed beat |
| `dcs_alive_threshold_ms` | 30000 | The largest non-transition gap measured is 282 ms and the largest untelegraphed transition is a 1.76 s teardown (Provenance), so 30 s carries roughly 17× margin over the case it must actually survive. Kept generous because a dedicated server under load is unmeasured (**[PROBE-10]**) and because a false "DCS is dead" is worse than a slow alarm. |
| `dcs_alive_threshold_loading_ms` | 120000 | Applies between `MissionLoadBegan` and `MissionLoaded`, where the frame blackout is total (Section 9.1). Instrumented loads measured 4.7 s to 34.7 s across three terrains; `dcs.log` records up to 59.9 s on larger missions. **Equal to `load_timeout_ms` by construction** — a lower value would flag DCS dead during silence Section 11 calls normal. |
| `load_timeout_ms` | 120000 | Section 11. Instrumented loads measured 4.7 s to 34.7 s across three terrains; `dcs.log` on the same machine records five between 20.2 s and 59.9 s. Twice the largest logged load, because a false "DCS is dead" beats a slow alarm. |
| `pause_poll_interval_ms` | 1000 | Section 9.2 |
| `eval_poll_interval_ms` | 1000 | A directory scan from `onSimulationFrame` measured about 0.098 ms on one machine. At 1 Hz that is negligible even an order of magnitude out. |
| `request_timeout_ms` | 2000 | Section 8.5.2. A request normally resolves within one frame. Across a mission load it does not resolve at all — but `EpochClosed` discards pending requests with an error first, so this timeout rarely fires at a boundary. |

**Spool** — owner **broker**, tier **live**: both are reclaim bounds applied by
the writer thread as it appends. Optional. Section 11.

| Key | Default | Basis |
|---|---|---|
| `spool_max_bytes` | 268435456 (256 MiB) | **Size wins over age.** At 100 records/s and the 64-byte average used in Section 10, a full day is closer to 550 MB, so this cap binds first and the spool holds under 24 h on a busy server. That order is intended: bounded disk is a hard requirement, retention a preference. |
| `spool_retention_hours` | 24 | Section 14.7 requires a retention limit. An upper bound on age, applied after the size cap. |

**The bridge's own log** — owner **hook driver**, tier **live**. Section 13's
`Logs\DCSBridge\DCSBridge.log`.

| Key | Default | Basis |
|---|---|---|
| `log_max_bytes` | 33554432 (32 MiB) | Bounds `DCSBridge.log`, rotated, oldest rotated file deleted first — the same shape as the two audit logs. The rule above this table is that a limit with no default is a defect, and an unrotated log is a limit with no default at all. |
| `log_retention_days` | 14 | An upper bound on age, applied after the size cap. Shorter than either audit's: this log is a diagnostic, not a record of who did what. |
| `log_level` | `info` | `error`, `warn`, `info` or `debug`. `debug` is what an operator raises while reproducing a fault, and Section 14.7 forbids record contents at any level. |

**Schema and registration** — owner **broker**, tier **DCS restart**, because
registration lives for the DCS process (Section 5.1).

| Key | Default | Basis |
|---|---|---|
| `max_lifecycle_record_bytes` | 16384 (16 KiB) | The size of one retention slot, and the largest `LIFECYCLE` record the broker will retain. Slots are fixed and allocated at the first `configure`, so this figure times `max_lifecycle_topics` is standing memory — 1 MiB at both defaults. Sizing them at `max_frame_bytes` instead would stand 64 MiB for records the bridge's own thirteen fill to tens of bytes. Section 5.2 states what happens to a `LIFECYCLE` record larger than a slot. |
| `max_lifecycle_topics` | 64 | Slots the broker allocates for `LIFECYCLE` retention (Section 5.2), one per registered `LIFECYCLE` topic, each holding a record up to `max_lifecycle_record_bytes`. The bridge defines thirteen and the built-ins none; the rest is adopter headroom. A registration taking the count above this is refused whole. |
| `topic_filter_max_topics` | 256 | Topic ids one `SetTopicFilter` may name (Section 5.2). **Live**, and the one key in this group that is: it bounds a per-connection allocation an authenticated consumer can ask for, not a process-lifetime registration. Well above any plausible registered set — the bridge defines about twenty topics and the rest is adopter vocabulary — so a refusal here means a defective consumer rather than a large one. A longer list is refused whole and leaves the filter unchanged. |

Configuration keys, metric names and record fields are separate namespaces.
`dcs_last_heard_ms` is a `Pong` field (Section 5.2) and `record_lag_ms` is a
Section 12 metric. Neither is a setting.

Three rules about this table:

- **A limit with no default is a defect.** If a future rule introduces a bound,
  it gets a row here in the same commit.
- **`doctor` prints the effective values**, not the defaults, so an operator
  can see what is actually in force, and the tier at which each one applies.
- **Every group of rows names its tier.** A row that differs from its group
  says so on the group heading. A group with no tier is the same defect as a
  row with no default. See Section 13.2.

The loader's `ENABLED` flag is not a key in this file — it lives in
`Scripts\Hooks\DCSBridge.lua` itself — but it has a tier, **DCS restart**, and
Section 13.2 lists it.

### 13.2 Applying a configuration change

**No setting requires a rebuild.** Not one. What a change costs follows from
two facts about each key: which component owns it, and whether that component
reads it once or every time.

**The hook driver is the only reader of `Config\DCSBridge.lua`.** It is a Lua
file and the hook driver runs in a Lua state, so the hook driver parses it,
validates it, and distributes it: broker keys through `shim.configure`, sim
driver keys through the injection chunk and through the reload in Section 6.9.
One file, one parser, one authority. The broker never opens it, and the sim
driver opens it only in the Route B bootstrap (below).

| Owner | Reads it | Tier | What it costs an operator |
|---|---|---|---|
| Hook driver | At load, and on `ReloadConfig` | **Live** | Send `ReloadConfig` |
| Broker, per decision | At each `configure` | **Live** | Send `ReloadConfig` |
| Broker, at claim | Once, at the first `configure` | **DCS restart** | Edit and restart |
| Sim driver | At injection, and at each reload | **Mission reload** | `ReloadSimDriver`, or load a mission |
| Consumer | Its own business | **Not the bridge's** | See below |

**`ReloadConfig` is a command, not a file watch.** It carries the `reload`
capability, the same as `ReloadSimDriver`, because re-reading the file means
executing operator Lua on a running server. It gets the same discipline Section
6.9 gives a sim driver reload: compile without executing, apply only if the
whole file parses and validates, and keep the previous table in memory to roll
back to. A config that raises leaves the running configuration untouched and
reports why.

**Do not watch the file by mtime.** Section 7.3 establishes that `modification`
is whole seconds on this build, so a file written and polled inside the same
second reports unchanged while still being written. Executing a half-written
config is worse than executing a half-written eval script. `ReloadConfig` is
the supported way to pick up a change, and it is one command.

**Live.** Everything the hook driver owns except `route` and
`hook_driver_disabled_files` (**DCS restart**) and `sim_driver_path` and
`sim_driver_disabled_files` (**mission reload**), plus every broker value read
as a number at decision time: the timeouts, the thresholds, the rate limits,
the eval settings, `ring_out_lifecycle_reserve`, and the tokens and capability
sets in Section 14.4. These are deliberately the ones an operator reaches for
while something is going wrong, which is when a restart costs the most.

**Mission reload.** Everything the sim driver owns: the per-frame stage caps,
the sim driver resource caps, `sim_driver_buffer_max_records`, and the
`options` table Section 6.10 merges. The hook driver passes them in the
injection chunk, and Section 6.9 step 1 re-reads them alongside the source, so
`ReloadSimDriver` applies them with no restart — under Route A. Under Route B
the hook driver does not inject and `ReloadSimDriver` is unavailable (Section
6.9): the sim driver reads the sim-driver-tier keys from `Config\DCSBridge.lua`
itself at bootstrap — the one exception to the single-reader rule, confined to
that route and those keys — and only a mission reload re-reads them.

**No hook-driver-owned key crosses that line.** The Route B exception covers
sim-driver-tier keys only. A hook-driver-owned key is read by the hook driver
on both routes, without exception, which is what keeps the single-reader rule
worth stating.

**DCS restart.** Five things, each because a decision is made once and never
revisited: the ring sizes — `ring_out_records`, `ring_in_sim_driver_records`,
`ring_in_hook_driver_records` — because Section 5.2 allocates at the first
`configure` and never allocates again; `max_connections`, because the
per-connection queue array is allocated with it; `bind_address`, `port` and
`allow_public_bind`, because the listener binds once and that flag is what
decides whether it may; `route`, because the hook driver decides it at load and
cannot reload itself (Section 6.9); `hook_driver_disabled_files`, because the
hook-driver-side extension chain is read once at DCS start (Section 6.10); and
the loader's `ENABLED` flag, because DCS globs `Scripts\Hooks\*.lua` once at
startup. Raising a ring is the moment an operator is restarting anyway: they
saw `records_dropped_total` move, they edited the file, and the restart is one
action rather than a build.

**Not the bridge's.** `request_timeout_ms` and `load_timeout_ms` govern
consumer behaviour. No component of the bridge reads either. They appear in
Section 13.1 because a consumer needs a value and a defensible basis for one,
and because `dcs_alive_threshold_loading_ms` is pinned to `load_timeout_ms` by
construction — a pin that only holds if both sides use the same number. The
broker therefore publishes both as advisory values in the `Schema` reply, after
authentication, so a consumer can adopt them instead of being configured
separately. A consumer may override them and is on its own if it does.

**Apply a `configure` as one swap, or not at all.** Validate every value, then
check the broker's cross-key invariants — `max_unauthenticated_connections`
below `max_connections`, `recent_idempotency_keys` at least
`ring_in_sim_driver_records`, `recent_admin_keys` at least
`ring_in_hook_driver_records` where the hook driver built-ins are deployed
(HOOK §1) — against effective values, not file values: a restart-tier key may
carry a pending file value. Reject the whole call on any failure. A partial
apply lets paired keys drift, and drifting pairs are exactly the ones whose
basis text explains why they must not.

**An invariant is checked where both its keys are visible.** `configure`
carries only the rows Section 13.1 marks **broker** (Section 5.1), so an
invariant over sim-driver-tier keys cannot be checked there —
`subscription_max_evals` at least `max_subscriptions` is the one such pair, and
both keys are the sim driver's. The hook driver checks it when it assembles the
sim-driver-tier settings, at injection and again at Section 6.9 step 1, and
refuses the load or the reload on failure. Same rule, different reader.

**Report what has not taken effect.** A `configure` carrying a restart-tier key
with a changed value resizes nothing. It logs the key, the old value and the
new one, and counts it in `config_keys_pending_restart`. `doctor` prints every
key whose file value differs from its effective value, so an operator sees a
pending restart rather than wondering why nothing changed.

**The `autoexec.cfg` merge.** `Config\autoexec.cfg` is shared with every other
tool that uses `net.dostring_in`. The installer merges. It never replaces the
file or an existing entry. It unions the state lists under **both** keys —
`net.allow_unsafe_api` and `net.allow_dostring_in` — with whatever is already
there, and preserves unrelated settings. See Section 5.4.2. `doctor` verifies
the result rather than assuming it.

### 13.3 Versions

Six version numbers exist. Each row below states what it guards, who compares
it, and what happens on a mismatch. **A version nothing compares is a
diagnostic, and this table says which those are** — so a future one has to earn
its place rather than accumulate.

| Version | Guards | Compared by | On mismatch |
|---|---|---|---|
| `protocol` | The frame format and handshake shape | Consumer, at handshake | Consumer's choice. The broker states its version and does not refuse. |
| `interface` | The Interface A call surface between hook driver payload and broker | Hook driver, at the first `configure` | Hook driver logs both, disables, leaves the mission alone (Section 5.1). |
| `GRAMMAR_VERSION` | The Interface C payload grammar between hook driver and sim driver | Hook driver, before injection | Injection refused; Section 11's injection-failure row (Section 5.3). |
| `STATE_VERSION` | The shape of `DCSBridge.state` | Sim driver, at reload | Cold reload rather than warm (Section 6.9). |
| `schema_sha256` | That every generated file came from one generator run | Broker, at registration | Warning in `stats`, printed by `doctor` (Section 8.3). |
| `broker` | Nothing. It names the broker build. | Nobody | Diagnostic only, carried in the handshake for bug reports. |

**Format.** `protocol` is a single integer, incremented on any change a
consumer must know about. The other four compared versions are opaque equality
checks and carry no ordering — a build either matches or does not, and nothing
compares greater-than. `schema_sha256` is a hash rather than a version for the
same reason.

**Bump rules.** Increment `protocol` when the frame header, the handshake, or
the broker-answered set changes. Bump `interface` when a Section 5.1 call is
added, removed or changes signature. Bump `GRAMMAR_VERSION` when the Section
5.3 payload grammar changes. Bump `STATE_VERSION` when the shape of anything in
`DCSBridge.state` changes. **None of the four moves for a reason outside its
own row**, which is the rule Section 6.9 depends on: a grammar change that also
bumped `STATE_VERSION` would drop every subscription and spot on a running
server for nothing.

**Which pairs must match exactly.** The hook driver payload and the broker must
agree on `interface`. The hook driver payload and the sim driver must agree on
`GRAMMAR_VERSION`. Every generated file in one install must agree on
`schema_sha256`. Nothing else is required to match, and a consumer at a
different `protocol` is the consumer's problem to resolve rather than the
bridge's to refuse.

**The DCS build is not a version of this software, and it is the one that will
answer most support questions.** Every figure in this document is a property of
one build (see Provenance), and a DCS update is the likeliest cause of a bridge
that stopped working. The hook driver reads it once at load and publishes it in
the handshake and in `SimDriverLoaded`, so a consumer and an operator can both
see which build produced the behaviour they are looking at without asking.

---

## 14. Security

### 14.1 Threat model

The bridge assumes a trusted operator, trusted mission content, and a hostile
network. It defends against an attacker who reaches the port, and against a
compromised or buggy consumer. **The broker boundary is where the defences are,
and it is the only boundary this document hardens.**

It does not defend against an attacker who can write files on the server. That
attacker already owns the machine.

**It does not treat the filesystem as a boundary at all.** The operator owns
the disk, authors the missions, and installs the bridge. Every file the bridge
reads — the sim driver, the extension directories, the eval directories, the
config — is a file that operator put there. A rule below that reads as a
filesystem authorisation model is describing an operational convenience, not a
defence, and should be read as one.

Players are not operators. A player is untrusted, reaches the sim over DCS's
own network path rather than this bridge's, and is the reason Section 14.7
gates disclosure by capability.

**Eagle Dynamics' own position on this mechanism.** The API reference shipped
in the install, `API\Sim_ControlAPI.md`, labels `net.dostring_in` **OBSOLETE
and UNSAFE!!!**, marks enabling it for the mission scripting state
**DANGEROUS!!!**, and states the call is superseded — see Section 5.4.3. ED's
forum announcement of the gate put it more bluntly still: the API is to be
used, in their capitals, ONLY WITH TRUSTED SCRIPTS AND MISSIONS. Reaching it
needs two `autoexec.cfg` lists: `net.allow_unsafe_api` for the states that may
call it, `net.allow_dostring_in` for the states that may be addressed.

That warning is correct. An operator who enables the API for this bridge is
entitled to a direct answer to it. The answer is Sections 14.6 and 14.4: **the
API grants the bridge the ability to run code in the mission environment. The
bridge does not pass that ability on.** No record carries Lua source or a
script path (Section 14.6). No release build ships a record that carries an
eval body from the wire; the operator-eval audit record in Section 7.6 carries
no source. A consumer's reach is bounded by its capability token, and a `read`
token cannot execute anything.

An operator who enables the API is trusting one artifact with a stated
capability model. They are not opening the API to every consumer that connects.
The capability model in Section 14.4 is what bounds it.

### 14.2 The parser is the highest-risk code

The parser runs inside the DCS process, before authentication completes. A
fault there is a fault in the game server. **There is no process boundary
between an attacker's bytes and the sim.** That is the accepted trade for
having no second process. The rules below are the mitigations.

- **Cap the frame length before allocating.** The `[u32 length]` prefix is
  attacker-controlled. Reject any frame above `max_frame_bytes`, default 1 MiB.
  Never allocate from an unvalidated length.
- **Parse as little as possible.** Read the length, `seq`, `epoch`, and the
  payload's type URL — one descent into the `Any` and one string out of it.
  Everything else passes through opaque. Cap the type URL at
  `max_type_url_bytes` before reading it, for the same reason the frame length
  is capped: it is attacker-controlled and it sizes a read.
- **Bound every decode.** The broker reads only the envelope header, so it
  enforces neither depth nor repetition — it cannot count elements of a field
  it never descends into. Depth and repetition bounds are generator constants:
  `protoc-gen-dcsbridge-lua` derives them from the schema's actual shape plus
  margin and bakes them into the generated decoders, so the limit is provably
  sufficient for the schema it ships with and no configuration can set it below
  what the schema needs. A consumer's stock library enforces its own. The
  broker's own bound is `max_frame_bytes`, which caps every decode at once and
  is checked before a byte is allocated.

Two further rules follow from that:

- **Catch a fault at the thread boundary.** A fault logs, drops that
  connection, and continues. Never configure the runtime to abort the process
  on one — in Rust that means not building with `panic = "abort"`.
- **Never let a parser fault reach the logic thread.** The reader thread owns
  every byte from the socket. The logic thread sees only whole records from the
  ring.

### 14.3 Connections

- Bind to loopback or to a private interface.
- A public bind address requires `allow_public_bind`, default false: without it
  the broker warns and refuses to listen (Section 13.1); with it the broker
  warns and listens. **A public bind is gated, not categorically refused**,
  because inside a container, `0.0.0.0` binds within a private network
  namespace and a sibling container has no other route to the port. Detect
  containerisation where possible and adjust the warning.
- Tunnel with WireGuard or SSH across an untrusted network. Do not terminate
  TLS in the broker. Certificate handling and rotation do not belong in a DLL
  inside a game process. Terminate it at the tunnel instead.
- Cap connections at `max_connections`, and unauthenticated connections
  separately at `max_unauthenticated_connections`.
- Close a connection that has not authenticated within `handshake_timeout_ms`.
  Rate-limit failed authentication at `auth_failures_per_min` per source
  address.

### 14.4 Authentication and capabilities

- One token per consumer. Do not share one secret. A token entry carries a
  short id, distinct from the secret, that names it in `stats` and in audit
  lines (Section 13.1 and HOOK §9).
- Compare tokens in constant time.
- **Grant a capability set with each token.** A live map needs `read` only. In
  `Config\DCSBridge.lua` a capability set is a list of `Capability` enum
  numbers (Section 8.2); the three bridge members may be written by name —
  `read`, `command`, `reload`.
- **The bridge defines three capabilities and no more.** `read` is disclosure.
  `command` is mutation. `reload` is loading code. Each names a broker concern.
  A capability named for a role — moderation, admin — is domain content and
  belongs to an adopter's range: the built-ins use 50 to 99, an adopter 100
  and above (Section 8.2). HOOK §7 defines the hook driver built-ins' five.
- **Enforcement is per topic.** The generator emits topic-to-capability tables
  and the hook driver and the sim driver register them like the class tables
  (Sections 5.1 and 8.3). Inbound, the broker refuses a record whose required
  capability the token lacks, with `Rejected` reason `NO_CAPABILITY` (Section
  5.2). Outbound, it withholds a record whose capability the connection lacks,
  at fan-out, before `seq` assignment, counted in `records_filtered_total` and
  never in `records_dropped_total` (Section 5.2). Point-to-point records are
  addressed, not fanned out, so the outbound filter does not apply to an
  acknowledgement, a typed reply or a `Rejected`.
- **A capability gates a record type, never a field.** Per topic id is the only
  granularity the broker can enforce. Anything needing narrower disclosure
  becomes its own record type.
- **A topic filter never widens a capability set.** Section 5.2's
  `SetTopicFilter` narrows what a connection is sent and is
  consumer-controlled; a capability set bounds what it may be sent and is
  operator-controlled. The broker applies both at fan-out, so `SetTopicFilter`
  needs no capability of its own. Naming a topic the token does not cover
  discloses nothing **because Section 5.2 reports an uncovered topic and an
  unregistered one identically** — without that rule the reply would be a
  registration oracle for the whole outbound vocabulary.
- **`reload` is a capability of its own, separate from `command`.** A consumer
  that can reload the sim driver can run whatever source is on disk. That is a
  different power from spawning a group.
- Order of operations is handshake, authentication, then everything else.
  `GetSchema` requires authentication.
- Read the token from a file or an environment variable, never from a command
  line argument.
- **Tokens and capability sets reload live**, through `ReloadConfig` in Section
  13.2. Rotation must not need a restart, because the moment you need to rotate
  is the moment a restart costs the most.
- **Revoking a token drops every session authenticated with it**, immediately
  and without waiting for the connection to do anything. A revocation that
  leaves the compromised session running is not a revocation.

### 14.5 Denial of service

- Rate-limit inbound at `inbound_records_per_sec` per connection **and at
  `inbound_records_per_sec_total` across all of them**. A per-connection limit
  alone does not bound the aggregate. `max_connections` consumers can each obey
  their own limit and still exceed what the sim driver can dispatch. The excess
  then arrives as inbound-ring drops rather than as a disconnect.

  **The two limits answer differently, and the difference is deliberate.** A
  connection over its own limit is misbehaving alone: refuse the record with
  `Rejected` reason `RATE_LIMITED` and keep the connection, bounded by
  `rejected_max_per_sec` so the refusal cannot be amplified. A connection that
  pushes the aggregate over is a capacity problem no refusal fixes: disconnect
  it. Note what that costs — the aggregate rule drops whichever connection
  arrived last, not whichever is greediest, and a well-behaved consumer can
  lose its session to a noisy neighbour. It reconnects and replays the retained
  `LIFECYCLE` set (Section 5.2).
- Drop the newest records when the inbound ring is full, and answer each with
  `Rejected` reason `BUSY`, bounded by `busy_max_per_sec`. The answer replaces
  a record already discarded rather than adding one, so it buys no
  amplification, and the cap bounds it regardless.
- Give every command handler its own sanity limit. A single well-formed command
  that spawns hundreds of groups will stall the sim, and the broker cannot see
  that coming. `spawn_max_groups_per_command` is the concrete instance. Any
  handler that loops over a consumer-supplied list needs one of its own.

### 14.6 Code execution paths

**Code comes from the filesystem. Code never comes from the wire.**

- **No record carries Lua source or a script path, in any build.** Not the sim
  driver path. Not an eval body. Not a file name to load. `ReloadSimDriver`
  names nothing. It reloads the configured path.
- **The sim driver path comes from config only.**
- **Operator eval reads from a fixed directory** the operator created. See
  Section 7. Filesystem permissions are its entire authorisation model, and
  that is the correct authority for the operation.
- Write access to the sim driver file, to `eval\`, or to either extension
  directory is equivalent to shell access on the server.

**One development-only exception, and it never ships.** A network-eval record
is built for development and compiled out of release. It is the reason the rule
above names the wire rather than the record. `EvalExecuted` (Section 7.6) is
unaffected: it carries a hash, never source. A release build that contains a
wire-sourced eval path is a defect, whatever the plan says.

**A mission-adjacent file runs unsanitised.** A mission's own scripts run in
the sandboxed mission-scripting environment: `Scripts\MissionScripting.lua`
removes `os`, `io`, `lfs`, `require`, `loadlib` and `package`, and ED's comment
on that block names the reason — a mission downloaded from a server may carry
harmful code. A file loaded from a mission folder under Section 6.10 does not
run there. It runs in the sim driver's environment, which holds `require`, `io`
and `os` (Section 5.1.2).

**Section 14.1's operator authored the mission, so that is a capability rather
than an exposure.** Loading your own file from your own disk with your own
libraries available is the feature. `mission_sim_driver_dirs` still defaults to
false, for the ordinary reason a file-loading path defaults off: an operator
opts into a loader that reads a directory they may not have looked in, rather
than discovering it by surprise. It is not a trust boundary, and Route B has no
equivalent key precisely because none is needed — there the sim driver and the
mission scripts share an environment by construction, so a mission's `init.lua`
reaches the registration surface directly (Sections 5.1.2 and 6.10).

An operator who does host mission folders from authors they have not vetted is
outside Section 14.1's model, on either route. This key does not bring them
inside it.

**The off state is visible, never silent.** Where the key is false and the
loaded mission carries a `dcsbridge\` directory, the loader logs
one line naming what it did not load, and `doctor` reports it. A mission
author's files failing to run on an operator's server is a configuration fact
somebody can see, not a mystery.

### 14.7 Data sensitivity

The bridge carries both coalitions' unit positions, player names, and slot
occupancy.

- Any consumer with `read` sees every `read`-gated record. The bridge gates
  every fan-out record it defines at `read`. The sim driver built-ins narrow:
  `read` covers world data, and the hook driver built-ins gate player identity
  and chat under their own capabilities (HOOK §7). A ucid is a durable
  cross-server identifier and ships only under `players`; a network address
  ships only under `ipaddr`. A leak to one coalition is cheating.
- Do not forward player network addresses unless a consumer needs them.
  `PlayerAddress` under `ipaddr` is the only record that carries one (HOOK §4).
- Do not log record contents to `dcs.log`. Administrators share that file.
- The replay spool persists this data. `spool_retention_hours` bounds it. Set
  directory permissions to match.
- **`Logs\DCSBridge\eval\` persists whatever an eval script printed**, which
  can be the same data. `eval_log_max_bytes` and `eval_log_retention_days`
  bound it. Set its permissions to match the spool's, and warn operators that
  zipping `Logs\` for support ships these files.

### 14.8 Integrity reporting

The hook state exposes `DCS.getTaintedFiles` and `DCS.getTaintedCategories`.
Both exist to report exactly this shape: a hook script plus a third-party DLL.

**The pre-DLL baseline is measured and clean** (2.9.29.27278, hosted
multiplayer, with a `Scripts\Hooks\` script loaded and a live
`DCS.setUserCallbacks` set registered): `getTaintedFiles()` returns nil, and
`getTaintedCategories()` returns four flags — `models`, `textures`, `scripts`
and `others` — all false. So neither a hook script nor registered callbacks
taint an install by themselves.

**Whether adding the DLL changes that is still unmeasured, and it is the half
that matters.** `scripts` staying false was the likelier outcome; `others` is
the flag a loaded third-party binary would plausibly move. **[PROBE-9]**

Operators will recognise the consequence. The Mission Editor's server browser
calls `DCS.getTaintedFiles()` and treats a non-empty result as tainted for its
integrity badge, and `net.ERR_TAINTED_CLIENT` is a documented disconnect reason
code. So a tainted verdict has a network consequence, not merely a UI one.
Release notes must say plainly that the answer is unmeasured.

---
## 15. The CLI

`dcsb` is one binary with several verbs. It is how an operator observes a
running bridge and diagnoses a broken one, so its checks are requirements on
what the bridge must expose rather than a convenience.

**Verbs.** `tail`, `send`, `schema`, `ping` and `stats`, plus `record`,
`replay`, `mock` and `doctor`. Schema reflection makes `send` work for record
types the CLI was never compiled against: it reads the `FileDescriptorSet` from
`GetSchema` (Section 5.2) rather than a compiled-in copy.

**`record` and `replay`.** `record` appends framed records to a file. `replay`
feeds that file to a consumer with no DCS running. The format is the wire
format, appended — no second format exists.

**`mock`.** Synthetic traffic conforming to the schema at a configurable rate.

**`doctor` is the diagnostic verb.** It checks file placement, hook driver
load, port, load banner, sim driver load and route, and component versions. It
prints the effective configuration with each key's owner and tier, and flags
any key whose file value has not taken effect yet (Section 13.2). It prints
both interface versions (Section 5.1). It checks for the Route B `dofile` line
where Route B is configured (Section 5.4.1), and reports both `autoexec.cfg`
keys under the rule in Section 5.4.2.

It reports the extension layer too. Per shipped file it compares the file
against the release's own hash and names any local edit as lost-on-update, and
it prints the Section 6.10 load order with each file's status: loaded, failed,
or disabled. `doctor` runs outside DCS, so it hashes with whatever its own
runtime provides.

---

## 16. Open questions

Ordered by value. PLAN §3 holds the method for each.

| # | Question | Why it matters |
|---|---|---|
| **PROBE-3** | Cost of one put call crossing from Lua into C | Decides whether one call per field is the right API shape, or whether a batched form is needed. The sim driver makes one call per field on every record, and Section 5.1's surface now costs two further crossings per element of a repeated message field. **The answer can invalidate Interface A's shape**, which every generated emitter targets. |
| **PROBE-7** | Ring sizing | The drop policy is safe only if the ring absorbs a real stall |
| **PROBE-9** | Does `DCS.getTaintedFiles` react to the DLL? | Narrowed: hook scripts and registered callbacks are measured clean, so only the DLL half is open, and `others` is the flag to watch. Needs the broker built. Operators will ask, and a tainted verdict can disconnect a client. Section 14.8. |
| **PROBE-10** | Behaviour on a dedicated server | Every figure is from a single-player host, so **every budget in Section 10 and every default in Section 13.1 rests on it**. The margins are sized for that, but a margin is not a measurement. |
| **PROBE-14** | Does a hook-driver-targeted record reach the hook driver ring under both injection routes? | Route B loads the sim driver at bootstrap and the hook driver carries no sim driver traffic, but the hook driver registers the route table in every configuration (Section 8.3). |

---

## 17. Test method

**The Host column says where a row can run.** *Any* means any build host with
no DCS present. *Any (native module)* means the same, against a host-native
build of the module loaded by a stock Lua 5.1 — the broker touches nothing
DCS-specific, so its behaviour is checkable off-platform. *Windows + DCS* means
the row needs the sim and cannot run in CI. Roughly half the rows below need no
game install, which is what makes the broker and the CLI developable and
testable on any of the three build hosts.

| Layer | Method | Host |
|---|---|---|
| Sim driver and hook driver Lua | Stub harness with stubbed DCS globals and a recording mock for the put calls. The mock captures calls into a table; it encodes nothing and opens no socket. | Any |
| Sim driver under real conditions | Throwaway mission on a live host. The harness cannot exercise object lifetimes, real error modes, or frame budget. | Windows + DCS |
| Lua to broker | Real broker, fake consumer decoding with a stock protobuf library | Any (native module) |
| Broker internals | Unit tests in the broker's own language: ring fill and overflow, varint and double encoding, framing, drop counters, per-class drop policy. No DCS involved. | Any (native module) |
| Broker to consumer | Real consumer, fake broker replaying a captured frame stream | Any |
| Generator | Golden files, for emitters **and** decoders | Any |
| Decoder | A body encoded by a stock protobuf library decodes to the same fields in the sim driver. An unknown field number is skipped, not fatal. | Any (native module) |
| Injection routes | The same sim driver file loaded by Route A and Route B. Emitted record content must be identical apart from `SimDriverLoaded`; timing may differ — Route B's drain suspends during pause, and `ReloadSimDriver` is Route A only (Sections 5.4.1 and 6.9). | Windows + DCS |
| Reload | Deliberate syntax error rolls back and the old sim driver keeps running. A `STATE_VERSION` bump goes cold **and destroys spots before discarding state**. A reload causes no duplicate event dispatch. | Windows + DCS |
| Eval | A script exceeding the instruction budget fails without stalling the frame. A crashing script leaves `.running` and does not re-execute. A half-written file is not picked up. | Windows + DCS |
| Binding probe | Throwaway mission. A binding on the Section 4.2 register is blacklisted and refused. | Windows + DCS |
| Resync ordering | Buffered events emit before any resync slice in the same frame, per Section 6.8. | Any |
| Broker hardening | Malformed and oversize frames, repeated-element backstop, unauthenticated caps, auth rate limits. One connection drops and the sim is unaffected. | Any (native module) |
| Acknowledgement | A command with no `reply_to` produces one `CommandAck`, on the sending connection and no other. | Any (native module) |
| Request and reply | A successful read produces its typed reply and no `CommandAck`. A failing read produces a `CommandAck` and no typed reply. | Any (native module) |
| Retry | A command re-sent with the same `idempotency_key` executes once and is acknowledged twice, the second with outcome `DUPLICATE`. | Any (native module) |
| Point-to-point | A record opened with `begin_to` reaches one connection. A record opened with `begin` reaches all of them. | Any (native module) |
| Late join | A consumer connecting mid-mission receives the current epoch's retained `LIFECYCLE` set, `EpochOpened` first among its needs, before any live record. One connecting during a load receives `MissionLoadBegan`. A reconnect receives the same set on its fresh `seq` origin. | Any (native module) |
| Envelope | `MissionLoadBegan` omits `epoch` and `mission_time`. A consumer does not discard it. | Any (native module) |
| Audit bound | `eval-audit.log` rotates at `eval_audit_max_bytes` and stops growing. | Any |
| Empty sim driver | With `SimDriver.lua` absent or empty, the broker-answered pairs, the hook-driver-emitted lifecycle records and the hook-driver-handled commands pass their rows. **`EpochOpened` and `EpochClosed` both arrive, with a terrain name and a time pair.** A sim-driver-bound command times out at the consumer and is counted. | Windows + DCS |
| Epoch boundaries | `EpochOpened` precedes injection and carries all seven fields, `is_server` true and `is_multiplayer` false on a single-player host. A late joiner receives it from retention on every configuration, sim driver or no sim driver. `DCS.getMissionTheatre()` and `DCS.getModelTime()` are each called alone. `EpochClosed` is emitted from `onSimulationStop`, where the theatre reads nil, and carries no field that would need it. | Windows + DCS |
| Calibration | `CoordinateCalibration` arrives from the hook driver and converts a known airfield to its published latitude and longitude within tolerance. **It arrives with `SimDriver.lua` absent**, like both epoch boundaries. Its values match `coord.LOtoLL` read from the mission-scripting state. | Windows + DCS |
| Calibration shape | A terrain failing the family check yields verification points, no `projection`, and a logged residual; a consumer distinguishes that from a zero false easting by presence, not by value. Under Route B the record carries no date and `declination_status` reads `ROUTE_B`. With `"gui"` ungranted it reads `POLICY_REFUSED`. | Windows + DCS |
| Calibration ordering | A consumer joining mid-epoch receives `EpochOpened` before `CoordinateCalibration` in the retained replay. | Any (native module) |
| Routing | A hook-driver-targeted record reaches the hook driver ring and never the sim driver ring, under both injection routes. | Windows + DCS |
| Inbound backpressure | A command arriving at a full inbound ring is answered with `Rejected` reason `BUSY` and reaches no handler. A mutation retried with the same `idempotency_key` executes once; a read retried with the same `request_id` resolves once. Refusals above `busy_max_per_sec` move `rejections_suppressed_total` and are not answered. | Any (native module) |
| Topic discovery | `GetTopics` lists every topic the token's capability set covers, with its class, and no topic outside that set. A token covering three topics sees three. The set grows after an adopter file registers new topics at a mission reload and never shrinks. | Any (native module) |
| Topic filter | A connection that sends no `SetTopicFilter` receives every fan-out record its capability set permits. One naming a single topic under `ONLY` receives that topic and every `LIFECYCLE` topic, and no other. A second `SetTopicFilter` replaces the first rather than adding to it. `records_filtered_total` moves and `records_dropped_total` does not, and the consumer sees no `seq` gap. | Any (native module) |
| Topic filter bounds | A `SetTopicFilter` naming an unadmissible topic succeeds and lists it in `unknown`; a topic the token cannot see is listed identically. One with no mode, one carrying a list under `ALL`, and one naming more than `topic_filter_max_topics` are each refused with `ok` false and a `refusal` reason, leaving the previous filter in force. A narrowed connection still receives its own `CommandAck`, its typed replies and its `Rejected` records. | Any (native module) |
| Registration | A `classes`, `routes` or `caps` call naming a registered topic with a different value is refused whole. An identical re-registration is a no-op. A call naming only new topics merges. | Any (native module) |
| Capability | A token without the capability is refused inbound with `Rejected`. The matching outbound record is withheld at fan-out with no `seq` gap. `records_dropped_total` does not move. | Any (native module) |
| Veto callbacks | A record emitted from `onPlayerTrySendChat` reaches a consumer. The callback returns nothing on every path, and a second hook script that vetoes still runs. `onPlayerTryChangeSlot` tests the same rule only where ED's `Scripts\Hooks\multislot.lua` is absent: it registers that callback and returns a value on every path, which ends the hook call chain before a write-directory hook runs. | Windows + DCS |
| Interface C | Live host only. `dostring_in` returns nil on a client. | Windows + DCS |
| Extension load order | Shipped files, then the operator's extension directory; names order within a directory. The merged `options` table is visible to every extension file. | Windows + DCS |
| Extension containment | A file that raises at load is skipped and counted in `sim_driver_files_failed_total`; the remaining files load and dispatch. | Windows + DCS |
| Overrides | `off`, `replace` and `wrap` act on the named registration. A `wrap` that does not call the previous function suppresses it. Each of the three, on a missing key, refuses that file whole with the key named, and the file's earlier registrations do not take effect. | Any |
| Topic ownership | A second `command` on one topic refuses the registering file whole; `replace` on the key succeeds. | Any |
| Reload set | A reload re-reads the runtime, `SimDriver.gen.lua` and every enabled extension file; one file failing to compile aborts the reload with the running set untouched. | Windows + DCS |
| Release discipline | A release overwrites bridge-owned files, leaves `DCSBridge\simdriver.d\` and `DCSBridge\hookdriver.d\` untouched, and restores a deleted `SimDriver.builtin.lua`; `sim_driver_disabled_files` still suppresses it. | Any |
| The built-ins are a customer | With `SimDriver.builtin.lua` deleted, the registration surface still exists and a file in `simdriver.d\` registers and dispatches. | Windows + DCS |
| Operator directory | A file in `<write dir>\DCSBridge\simdriver.d\` loads after the shipped files and may `wrap` a `builtin.*` key. An absent directory is silent and normal; a refused one is logged and counted, not fatal. | Windows + DCS |
| Message references | A `send` or `topics` member that does not exist fails at first use with the name in the error. The generated annotations and the runtime stub load clean in a stock Lua language server. | Any |
| Extension vocabulary | An extension's `*.gen.lua` registers its topics additively and loads before its directory's other files; a reloaded mission re-registers as a no-op; a second source claiming a registered topic with different values is refused loudly. `GetSchema` serves the shipped set only. | Windows + DCS |
| Global loading | Every mission loads the sim driver. No configuration key suppresses it on one mission. `enabled` false suppresses it on all of them, live, and `Pong` reports the disabled state. | Windows + DCS |
| Mission scope | A file scoping on `DCSBridge.code.mission` matches the same mission whether it is launched from the mission list or flown from the Mission Editor, where `name` reads `tempMission`. Under Route B both `name` and `filename` are nil and a scoped file guards for it. | Windows + DCS |
| Mission-adjacent files | With `mission_sim_driver_dirs` on, a mission's `dcsbridge\` directory loads for that mission and not for the next, in name order, with its `*.gen.lua` first. With the key off, none of it loads and one line names what was skipped. Route A only. | Windows + DCS |
| Registration points | An extension file registers no `world.addEventHandler`, no `DCS.setUserCallbacks` and no `missionCommands` handle. A reload causes no duplicate dispatch and orphans no menu item. | Windows + DCS |
| Budgets | Live server under load, Section 12 metrics | Windows + DCS |

State how each piece was checked: static reading, `luac5.1 -p`, stub harness,
throwaway mission, single player, or live server.

A clean harness run that exercised nothing is a failure, not a pass. `luac5.1
-p` catches syntax only. After a refactor, also check for a second `local` with
the same name, and for a call with multiple returns passed as one argument
among several. No syntax check finds either.
