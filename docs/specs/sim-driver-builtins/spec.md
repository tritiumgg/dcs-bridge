# DCS-Bridge — The sim driver built-ins

The record set and command set the sim driver ships with.

Companion to the DCS-Bridge specification. `SPEC §N` refers to it; `HOOK §N`
refers to the hook driver's built-ins, the other half of what ships in the box.
Terms used here without definition are defined in the DCS-Bridge
specification's preamble.

**This document specifies a record set, not a component.** The sim driver
itself — the four-slot global, the bounded frame loop, the registration
surface, reload, error containment and resource release — is SPEC §6. What is
here is the set of records and commands shipping in `SimDriver.builtin.lua`,
which registers through that surface like any adopter's file.

**Status:** Draft.

---

## 1. What the built-ins are

The built-ins are the record set and command set that ship in the box. A user
installs, configures one token, connects, and sees records before writing any
code. The token is not optional: `tokens` defaults to none and the broker
closes a connection that fails authentication (SPEC §13.1 and §5.2). **The
built-ins are shipped with DCS-Bridge but are not part of it.** The sim driver
is component 3 and always present; its built-in set is replaceable, and
DCS-Bridge carries records with the set gone (SPEC §9.5).

**Every built-in runs in the mission-scripting state.** They live in one file,
`SimDriver.builtin.lua`. Player, network and server-control surface belongs to
the hook driver's built-ins (HOOK §1), which run in the other state. One
document per state.

Nothing here reaches outside that state, and SPEC §6.6 is what keeps it that
way. Under Route A the state holds no `net`, so the boundary is also physical.
Under Route B the state holds `net` (measured, 2.9.29.27278; SPEC §7.2) and the
boundary is the rule alone. Coordinate calibration, magnetic declination and
every player, network and server-control command are the hook driver's either
way.

`SimDriver.builtin.lua` is DCS-Bridge-owned: a release overwrites it, so an
adopter who edits it loses the edit at the next update. The supported ways to
change built-in behaviour are the three grades in SPEC §6.10 — options,
overrides by key, and `sim_driver_disabled_files`, which skips a shipped
sim-driver file at a mission reload — not editing the file. **The built-ins
hold no privilege an adopter's files lack**: they register through the same
surface, in the same order, under the same containment.

**How the scope was chosen.** Tier 1 is what five years of third-party DCS
tooling has shown people actually use. Tier 2 is what that tooling still does
not cover, which is where a campaign consumer runs out of road. Tier 3 is what
this document declines, with the reason stated in each row.

**A gap in existing tooling is weaker evidence than a covered case.** An
unimplemented item may be unwanted, or it may be merely unfinished, and from
outside a project the two look identical. So Tier 1 is sized on demand and Tier
2 on need, and Tier 2 carries the burden of saying why each item earns its
place.

## 2. Tier 1 — first release

Proven demand, low cost, and nothing here is a research problem.

| Area | Records and commands |
|---|---|
| Mission events | Every `S_EVENT_*` the build defines. On 2.9.28.26385 `world.event` carries 63 constants, ids 0 to 62 contiguous: `S_EVENT_INVALID` (0), `S_EVENT_MAX` (62), and **61 real events** between them. One probe run fires only the subset its mission produces, so size the record set from the enum rather than from a run. Forward all of them, including three that are commonly skipped: `S_EVENT_TOOK_CONTROL` (13), `S_EVENT_TRIGGER_ZONE` (35), `S_EVENT_BDA` (37). **One record type per event id, not one record type carrying an event id.** SPEC §14.4 gates a capability per topic and never per field, so a single forwarding record type could not disclose `S_EVENT_SHOT` to a consumer while withholding `S_EVENT_PLAYER_ENTER_UNIT`. A topic is its message's fully-qualified name (SPEC §5.2), so 61 types cost no registry coordination. Section 10 governs what each record carries. |
| Event handler | One permanent table in `DCSBridge.resources`, registered once, `onEvent` wrapped in `pcall`. See SPEC §4.1 and §6.2. |
| World reads | `coalition.getGroups`, `getStaticObjects`, `getAirbases`, `getPlayers`, `getMainRefPoint` (bullseye), `world.getMarkPanels`, mission theatre |
| Unit reads | Name, id, type, coalition, country, category, callsign, point, velocity, `getDrawArgumentValue` |
| Group reads and control | `activate`, `destroy`, `getCategory`, `getCoalition`, `getName`, `getID` |
| Static objects | `coalition.addStaticObject`, static reads |
| Messages | `trigger.action.outText` and its coalition, country, group and unit variants |
| Map markup | `markToAll`, `markToCoalition`, `markToGroup`, `removeMark`, `markupToAll`. **There is no `markupToCoalition`** — confirmed absent from every state's index and from every shipped `.lua`. Scope a shape to one coalition with the per-shape functions in Tier 2: **the first argument is a coalition id and `-1` means all** (measured, 2.9.29.27278 — a blue client saw the `-1` and `2` lines and not the `1` line, with `markToCoalition` as the control). |
| Effects | `explosion`, `smoke`, `illuminationBomb`, `signalFlare` |
| Flags | `trigger.action.setUserFlag` and `trigger.misc.getUserFlag`. **The pair is split across two tables.** `trigger.action.getUserFlag` does not exist and calling it raises. |
| F10 menu | The full `missionCommands` set: `addCommand`, `addSubMenu`, `removeItem`, and their coalition and group variants |
| Time | `timer.getTime`, `getAbsTime`, `getTime0` |
| Weather | `getWind`, `getWindWithTurbulence`, `getTemperatureAndPressure` |
| Coordinates | The `ConvertCoords` command, which wraps the MGRS pair. `CoordinateCalibration` and the magnetic declination on it are DCS-Bridge's and the hook driver's (SPEC §6.3, HOOK §10). See Section 4. |

**Every row above runs in the mission-scripting state**, which is where SPEC
§6.6 keeps a sim driver handler. No row declares SPEC §8.2's `target` option.

**What is deliberately absent.** Player identity and lifecycle, chat, the
roster, kick and slot control, bans, and every server-control command are the
hook driver's (HOOK §1). A consumer wanting them reads that document. The
division is by state, not by taste: those bindings are unreachable from here.

## 3. Tier 2 — what the catalogue's gaps ask for

These are the items existing DCS tooling has left unbuilt after five years, and
several are what a campaign consumer needs most.

**`coalition.addGroup`, complete.** This is the highest-value item in the
document. Spawning is widely available in a reduced form — ground units only,
placed at a point, with no waypoints, no tasking and no loadout — and the
reduction is what makes it useless for a campaign, which needs a group to
arrive somewhere, do something and carry the right stores.

The group table carries fuel, pylons, gun percentage, chaff, flares, route,
tasking, callsign, livery and skill. **Implement the whole table, not a
subset**, and implement it for air and sea as well as ground.

**A partial implementation must fail rather than silently narrow.** A request
carrying waypoints that spawns a group standing still, or one naming an air
category that yields a ground unit, is worse than a refusal: it looks like it
worked. Where a field or a category is unimplemented, refuse the command and
say which.

| Area | Items |
|---|---|
| Controller | `setTask`, `pushTask`, `resetTask`, `setCommand`, `setOnOff`, `getDetectedTargets`, `setOption` in full. `AI.Option` in the mission-scripting state carries **30 real option ids** across three groups — Air 26, Ground 8, Naval 2, overlapping on `ROE`, `FORMATION` and `NO_OPTION` (measured, 2.9.29.27278). ROE and Reaction To Threat are the two a consumer reaches for first. |
| Unit | `getLife`, `getLife0`, `getFuel`, `getAmmo`, `getPlayerName`, `isActive`, `enableEmission`, `getGroup`, `getController`, `getByName` |
| Group | `getUnits`, `getSize`, `getController`, `enableEmission`, `getByName`, `isExist` |
| Airbase | `setCoalition`, `autoCapture`, `autoCaptureIsOn`, `getWarehouse`, `getParking`, `getRunways` |
| Static objects | `getLife`, `getByName`, and `destroy` and `isExist` **as inherited from `Object`**. Neither is a member in its own right — `rawget` returns nil — but both resolve through the metatable's `__index`, and `StaticObject.parentClass_` is present (measured, 2.9.29.27278). Reach them as methods on an instance, never as `StaticObject.destroy`. |
| Terrain | `land.getHeight`, `getSurfaceType`, `isVisible`, `getIP`, `profile`, `getClosestPointOnRoads` |
| Group control | `activateGroup`, `deactivateGroup`, `setGroupAIOn`, `setGroupAIOff`, `groupStopMoving`, `groupContinueMoving` |
| Markup | The shape family — `lineToAll`, `circleToAll`, `rectToAll`, `quadToAll`, `textToAll`, `arrowToAll` — and the eight `setMarkup*` mutators: `setMarkupColor`, `setMarkupColorFill`, `setMarkupFontSize`, `setMarkupPositionStart`, `setMarkupPositionEnd`, `setMarkupRadius`, `setMarkupText`, `setMarkupTypeLine` |
| Sound and radio | `outSound` and its variants, `radioTransmission`, `stopRadioTransmission` |
| World | `searchObjects` with full volume support: sphere, box, segment, pyramid |
| Weapons in flight | `getLauncher`, `getTarget`, position, velocity, type name. See Section 5. |
| Reference points | `addRefPoint`, `getRefPoints`, `getCountryCoalition`, `getServiceProviders` |

## 4. Coordinate conversion

Records carry DCS-local coordinates: +x north, +z east, +y up. SPEC §8.2 pins
that convention.

**Do not convert at emit.** `coord.LOtoLL` is one engine call per position, on
the logic thread. At 500 units per sweep that is 500 extra calls, which defeats
the sampling discipline the design exists to protect. A local Cartesian frame
is also the cheaper frame for the sim driver's own predicate evaluation:
proximity and range checks are subtraction, not great-circle arithmetic.

**Do not push the problem to the consumer either.** DCS's x/z frame is a
projection local to the loaded terrain, and its parameters differ per map. A
consumer converting on its own must embed a per-terrain table and maintain it
whenever ED ships a map. Only DCS holds those parameters authoritatively.

Two mechanisms answer that, neither per record, and the built-ins own one.

**The calibration is DCS-Bridge's.** SPEC §1.2 enumerates
`CoordinateCalibration` among its own records, and SPEC §6.3 and §9 make
DCS-Bridge emit it. Every measured DCS terrain projects with one family —
transverse Mercator on WGS84 with `k_0 = 0.9996`, a central meridian at an odd
multiple of 3 degrees, and a per-terrain false easting and northing — and
DCS-Bridge derives that parameter set once per epoch and publishes it, together
with a PROJ string and the four verification points HOOK §10.2 fixes. A consumer feeds the
string to any PROJ binding and converts its own bulk data with no further
traffic.

**That work is the hook driver's, and the reason is state placement rather than
preference.** The derivation reads `terrain.convertMetersToLatLon`, which lives
in the hook state; `coord.*` lives in the mission-scripting state and nowhere
else. HOOK §10 specifies the derivation, the publication format, and what
happens on a terrain that does not fit the family. The sim driver converts
nothing for it, and adds no field to `EpochOpened`, whose seven fields SPEC §6.3
fixes and which DCS-Bridge emits before the sim driver loads.

**Three norths.** A heading derived from carried data — a velocity vector, an
orientation, a bearing between two positions — is referenced to grid north, DCS
`+x`. True north differs from it by the projection's convergence angle, which
the published parameters give a consumer at no further cost. Magnetic north
needs declination, and only DCS holds that authoritatively: its own model is
what the sim's runway numbers and instruments show, and an external model may
not match it.

**Declination is the hook driver's from end to end.** The only binding is
`magvar`, which lives in the gui state and nowhere else, and reaching it needs
`net.dostring_in('gui', ...)`. Under Route A that call is beyond the sim
driver's reach: `"server"` holds no `net`. Under Route B it is not — the mission
scripting state holds `net` and reached `magvar` in a measurement
(2.9.29.27278) — so on that route this is SPEC §6.6's rule rather than a
platform limit, and the rule still binds.
The mission-scripting state has no candidate of its own: `Export.LoGetMagneticYaw`
takes no argument, so it reports own-ship yaw and cannot answer "declination at
point P". HOOK §10.3 carries the route, the units, the seeding question, and the
`autoexec.cfg` entry the route costs an operator.

A consumer reads declination off `CoordinateCalibration`, beside the projection
parameters, as an optional value per verification point — it varies across a
large map, so per-point beats one scalar per theatre. Where it is absent,
magnetic north is unavailable; grid and true north are not, because both fall
out of the projection.

**A batch conversion command.** `ConvertCoords` takes N points and a direction
and replies with the converted set, using the request and reply pattern in SPEC
§8.5.2.

`N` is capped at `convert_max_points_per_command`. Each point is one
`coord.LOtoLL` call on the logic thread, so an uncapped batch is an uncapped
frame. A request above the cap is answered by a `CommandAck` with outcome
`REFUSED` and the cap in `detail` (SPEC §8.5.3). A consumer with more points
sends more commands. The SPEC §6.4 dispatch cap then spreads them across frames
on its behalf. It wraps the MGRS pair:

| Function | Why it is exposed |
|---|---|
| `coord.LLtoMGRS` | A library could do this. DCS's result is what the player sees on the F10 map, and matching the sim is worth more than matching a library. |
| `coord.MGRStoLL` | Same reasoning, and it completes the set. |

**`ConvertCoords` does not wrap `coord.LOtoLL` or `coord.LLtoLO`.** Once
`CoordinateCalibration` carries the projection, a consumer converts both
directions locally and exactly, so wrapping them would spend a dispatch slot
and a frame on work the consumer can already do. The two that remain are the
two a consumer cannot reproduce: MGRS parity with what the player reads off the
F10 map. A consumer that meets a `CoordinateCalibration` with no parameters —
the family-mismatch case above — uses `ConvertCoords` for MGRS and asks the
operator to report the residual.

Batch the call. One command carrying N points costs one dispatch slot. N
commands cost N.

**`ConvertCoords` is a built-in; the calibration is DCS-Bridge's.** SPEC §1.2
enumerates `CoordinateCalibration` among DCS-Bridge's own records, so the
"DCS-Bridge defines no domain record" rule does not reach it. `ConvertCoords`
is built-in vocabulary in its topic range (SPEC §8.2). A replacement set that
drops it produces the unknown-topic silence of SPEC §9.5.

**Its MGRS is DCS's own, and that is not the same as the F10 map's** (measured,
2.9.29.27278, six points across Caucasus). Given identical latitude and
longitude, `coord.LLtoMGRS` and the map's own `Terrain.GetMGRScoordinates`
agree on zone, latitude band, digraph and easting, and **differ by one metre in
the northing at four of six points** — the signature of one truncating where
the other rounds. So the command gives a consumer DCS's conversion under DCS's
projection; it does not give a string the player can read back off the map
digit for digit.

**Two shape traps come with it.** `coord.LLtoMGRS` returns a table — `UTMZone`,
`MGRSDigraph`, `Easting`, `Northing` — where the map returns a formatted
string, and the map spaces the zone from the band (`37 T FH`). And **the
easting and northing are numbers, so leading zeros are lost**: a point measured
at easting `03901` comes back as `3901`. A consumer that formats without
zero-padding to the precision's digit count produces an invalid grid reference.
State the padding in the reply's field documentation.

## 5. Weapons

Split the class. The two halves have opposite cost profiles.

**Instance state is worth carrying.** `S_EVENT_SHOT` hands the sim driver a
weapon object. From it: `getLauncher`, `getTarget`, position, velocity and type
name. That is who fired what at whom, which is kill attribution, launch
warning, and engagement analysis. Only DCS holds it and it is gone once the
weapon is.

Weapon tracking is **opt-in and scoped**, for the same reason subscriptions
are. A busy mission with many SAM sites produces a high `S_EVENT_SHOT` rate.
Attaching a tracking loop to every weapon is an unbounded per-frame cost.

A consumer subscribes to weapon tracking with a filter. The filter has three
terms — launcher, target, and weapon category — and a weapon matches only where
it satisfies every term the request supplies. An omitted term matches
everything. The sim driver tracks what matches, up to `max_tracked_weapons`,
under the caps in SPEC §6.4. **The subscription and the weapons it tracks count
against different caps.** The subscription is a subscription like any other and
counts against `max_subscriptions` (SPEC §6.7); the weapons it holds count
against `max_tracked_weapons`, which `weapon_max_samples` throttles at half its
depth.

**Static descriptor fields are not carried, and are not exported either.** They
are type properties: warhead mass and type, calibre, explosive mass, guidance,
and the range and altitude envelopes. They do not change, so querying them per
weapon asks DCS for a constant — which is why Tier 3 excludes them from the
per-weapon record.

**Nothing replaces them in the first release.** An earlier draft specified a
bulk `ExportWeaponTypes` command. It is dropped: the demand for it is weak, and
weak demand does not justify a command with the awkward shape this one has.

The reasoning is worth keeping, because it is what a later proposal has to
answer. **A sweep is a snapshot, not a manifest.** `coalition.addGroup` and
warehouse rearm introduce types after mission start, so "every type the mission
uses" is not fixed at load and no single call can return it. And **the
descriptor shape varies by category** — a shell carries a warhead and no
guidance or range envelope, a guided weapon carries more — so any record set
for it must tolerate absent fields per entry rather than assume one flat shape.
Those two together are what made the command clumsy rather than merely unbuilt.

**The route exists if it is ever wanted** (measured, 2.9.29.27278).
`Unit.getAmmo` returns a list of `{count, desc}`, and `desc` holds `typeName`,
`displayName`, `category`, `life`, a `box` extent, and a `warhead` table of
`caliber`, `explosiveMass`, `mass` and `type`. It is the only route:
`Weapon.getDescByName` exists in no state, and every one of `Weapon`'s eight
members is an instance method needing a live weapon, which exists only between
`S_EVENT_SHOT` and impact. `db.Weapons.ByCLSID` is reachable from this state —
2213 entries — but holds loadout records with no warhead, calibre or guidance,
so it is the wrong table.

**Guard the nil either way.** `Unit.getAmmo` returns nil rather than an empty
table on a unit carrying nothing, measured on an unarmed helicopter. Anything
reading a loadout meets that case.

## 6. Laser and infrared spots

`Spot.createLaser` and `Spot.createInfraRed` are heavily used on public
servers. That use lives in mission scripting, because a spot has to be updated
as its target moves. This architecture moves the decision out and leaves the
tracking in.

**A spot is a managed resource, not a primitive.** Do not expose `createLaser`,
`setPoint` and `destroy` as three separate commands. A consumer issuing
`setPoint` per frame over the network is the failure mode SPEC §8.5.1 exists to
prevent.

The command pair is:

| Command | Behaviour |
|---|---|
| `LaserOn` | Source object, target — a unit, a static, or a fixed point — laser code, and an update interval. The sim driver creates the spot and tracks the target locally. |
| `LaserOff` | Destroys the spot by handle. |

**`LaserOn` and `IrOn` declare `reply_to`.** The consumer needs the spot handle
to call `LaserOff`, and a `CommandAck` cannot carry one: SPEC §8.5.3 gives it
`request_id`, `idempotency_key`, `outcome` and `detail` and nothing else. So
each `On` command carries a typed reply whose field is the handle, and
`CommandAck` stays what SPEC §8.5.3 makes it — the failure channel.

**They carry both correlation fields.** SPEC §8.5.2 gives `request_id` to a
read and `idempotency_key` to a mutation, and SPEC §8.5.3 carries the exception
these two need: never both, except where a mutation declares `reply_to`. They
are the first commands that are a mutation *and* need a typed reply. They need
the key so a retry does not create a second spot, and the id so the generated
wrapper resolves on the reply the way it resolves on any other. A duplicate is
answered with `CommandAck` outcome `DUPLICATE` rather than with the typed
reply.

That exception is the smallest of the three available — the alternatives are
teaching every generated wrapper a second match path, or making spot creation
non-idempotent and accepting an orphaned laser after a lost reply.

Infrared spots use their own pair, `IrOn` and `IrOff`, identical in shape minus
the code. Two pairs, not one with an optional code: an absent field must never
select the spot type — SPEC §5.1 notes protobuf omits an absent field, so a
forgotten code would silently create the wrong spot.

Spots reuse the subscription machinery in SPEC §6.7 exactly:

- Each spot declares an update interval. Per-frame is a choice, not a default.
- Active spots are capped at `max_spots`. A request above the cap is answered
  by a `CommandAck` with outcome `REFUSED` and the reason in `detail` (SPEC
  §8.5.3), never by silence.
- Update cost is counted per spot and reported. See SPEC §12.
- **Every spot is destroyed at `EpochClosed`.** A spot is a handle into a world
  that no longer exists.
- A spot whose source or target ceases to exist is destroyed, and the sim
  driver emits a record saying so. A consumer must not have to poll to discover
  this.

**DCS does not do that for you, and this is measured.** A laser spot whose
source unit is destroyed **keeps working**: with the source gone, `getCode`,
`getPoint` and `getCategory` all still answer, and `setPoint` still moves the
beam (2.9.29.27278). The spot has to be destroyed explicitly. So the
destruction above is the sim driver's own work — its update loop checks that
source and target still exist, and destroys and reports when either does not.
Nothing in the engine will do it.

That makes SPEC §6.1's warning concrete rather than theoretical: an orphaned
spot is a laser that goes on designating for the rest of the mission, and the
unit it was attached to no longer being in the world does not stop it.

**Validate the laser code as an integer in 0 to 9999. Enforce no band.** A
value outside that range, or one that is not an integer, is a `CommandAck` with
outcome `REFUSED`, not a silently dead laser. That is the whole check.

**The band is not DCS-Bridge's to decide, and no band is defensible today.**
The range 1111 to 1788 — second digit 1 to 7, third and fourth 1 to 8 — is what
most documentation gives, and it is where the AH-64D's own preset channels sit.
But that aircraft's cockpit lists ten ranges: 1111–1788, 2111–2888, 4111–4288,
4311–4488, 4511–4688, 4711–4888, 5111–5288, 5311–5488, 5511–5688 and 5711–5888
— and that list is a display table, not a validator. The Mission Editor's JTAC
laser-code field is a spin box bounded 0 to 9999 with no band check at all, and
ED's own per-weapon validator clamps digit by digit against bounds declared per
airframe, which differ: the F-4E bounds its second digit 5 to 7, the AV-8B's
GBU 5 to 8, the F-15E's trailing three digits 111 to 888. There is no global
range to enforce.

**Nothing validates a code on the scripted path, and that is now measured**
(2.9.29.27278). `Spot.createLaser` accepted every code it was offered — 0, 1,
999, 1234, 9999, **10000**, and every AH-64D band edge — and refused none. Two
values came back changed rather than refused: **`-1` read back as 4294967295**,
wrapping to unsigned 32-bit, and **`1.5` read back as `1`**, truncating. Both
are silent corruption.

So the engine is not a validator and will not become one. DCS-Bridge's check is
the only check, which is why it tests integer-ness as well as range: an
out-of-range code and a non-integer code both produce a laser that lases on
something other than what the consumer asked for. A validator narrowed to
1111–1788 would additionally reject codes the AH-64D treats as valid and
silently break JTAC coordination, which is why the check is a range check and
not a band check.

The sim driver tracks. The consumer decides. Which target to lase, when to
switch, and how to coordinate codes between several JTACs are all consumer
logic. None of that needs a round trip during the lase.

## 7. Tier 3 — deliberately excluded

| Excluded | Reason |
|---|---|
| Text-to-speech and SRS radio integration | Not a DCS binding, so nothing here reaches it. A consumer implements it. |
| `env.info`, `env.warning`, `env.error` | A consumer uses its own logging |
| Any exposure of `net.dostring_in` | SPEC §14.6 |
| `Weapon` descriptor **static** fields | Type constants rather than instance state, and no bulk export replaces them. Section 5 gives the reasoning and the route, should the case ever be made. |

## 8. Bindings that must not ship unguarded

These are the bindings in SPEC §4.2 that also appear in the catalogue.

Five bindings terminate the DCS process with an access violation on a bare call
**despite being called inside `pcall`**:

- `Unit.getSensors`
- `land.findPathOnRoads`
- `SceneryObject.getDescByName`
- `Disposition.getRandomSort`
- `coalition.remove_dyn_group`

All five appear in Tier 2 territory, which is what makes the register worth
stating rather than assuming. `land.findPathOnRoads` sits beside terrain
functions a consumer will want. `Unit.getSensors` is actively sought — it
exposes radar, IRST, RWR and optical data that nothing else reaches — so the
gate will be tested by someone. And `coalition.remove_dyn_group` sits directly
beside `coalition.addGroup`, which Section 3 calls the highest-value item in
the whole tier.

**Treat the whole `getDescByName` family as suspect.** `SceneryObject`, `Unit`,
`StaticObject` and `Airbase` each carry a static `getDescByName`. Only the
`SceneryObject` one terminates the process.

**The three usable members are one lookup under three names** (measured,
2.9.29.27278). `Unit.getDescByName`, `StaticObject.getDescByName` and
`Airbase.getDescByName` each returned the identical aircraft descriptor for
`'Su-25T'` — the class you call it on does not scope the lookup. So the family
is one binding wearing four names, which is why `SceneryObject`'s crashing is
worth treating as a property of the family rather than of one member. The other
three were probed on 2.9.28.26385 with a valid type name and with no argument,
and answered cleanly both ways. Per SPEC §4.2 that licenses nothing about other
arguments, but it does mean only the `SceneryObject` member is on the crasher
register.

No built-in implements any of the five. If a consumer needs one, it ships
behind `unsafe_bindings_enabled`, default false, with the crash behaviour
documented at the call site. SPEC §4.2's rule applies in full: a clean probe
licenses nothing about the same binding under other arguments.

## 9. Required guards

Each of these has bitten production tooling. Build the guard first rather than
discovering the case on a live server.

- `S_EVENT_HIT` can arrive with an empty initiator.
- A unit's `Group` can be nil.
- The `type` field is not always set on a unit, a weapon or a static object.
- `getID` is not implemented on every object type. Do not call it in a
  catch-all path.
- An object's category can be undetectable. Have a fall-back that does not
  depend on the category.
- **`destroy()` does not take effect within the calling frame.** A unit
  destroyed and then looked up by name in the same chunk is still found
  (measured, 2.9.29.27278). Treat destruction as a request and confirm it on a
  later frame, or from the event, rather than asserting it immediately.
- `Unit.getAmmo` returns nil rather than an empty table on a unit carrying
  nothing.
- `coord.LLtoLO` returns a point table, not three numbers, and the coordinates
  it returns sit on a float32 grid. Metre-level precision is all DCS carries.

SPEC §4.2's blacklist probe covers bindings that raise. These are bindings that
answer with something unexpected, which no probe finds.

**These guards, and Section 8's gate, bind any handler in this state — not only
the built-ins.** They are recorded here because the built-ins are what found
them, but an adopter's file meets the same bindings and needs the same checks.
SPEC §6.10 draws no distinction between the two, and neither should a reader of
this section.

---

## 10. Record-set philosophy

**Built-in records mirror DCS.** The built-in record set forwards `S_EVENT_*`
payloads as DCS defines them. Do not invent smoothed abstractions. DCS event
semantics change between versions. A forwarding record set needs a new record
type when that happens. An abstraction changes meaning silently for every
consumer.

---

## 11. Classes and capabilities

Every message crossing into or out of Lua declares a record class and a
required capability. SPEC §5.1 refuses a topic that is missing either, in both
directions, so an unassigned topic is a topic that does not work.

**This section states the policy. The generated tables carry the values.** The
per-topic map is what `shim.classes` and `shim.caps` register at load (SPEC
§8.3), and it is checkable there against the schema. Repeating it here per row
would duplicate a generated artifact and drift from it.

| Group | Class | Capability |
|---|---|---|
| Forwarded `S_EVENT_*` records | `DURABLE` | `read` |
| World, unit, group, static, airbase and terrain reads | `DURABLE` | `read` |
| Their reply messages | `DURABLE` | `read` |
| `ConvertCoords` | `COMMAND` | `read` |
| Mutating commands — messages, markup, effects, flags, F10 menu, group control, spawning | `COMMAND` | `command` |
| `LaserOn`, `LaserOff`, `IrOn`, `IrOff` | `COMMAND` | `command` |
| Spot and weapon-tracking notifications | `DURABLE` | `read` |

Three rules govern the table.

**A read command takes `read`, not `command`.** `ConvertCoords` is a `COMMAND`
message because it travels inbound, which is what the class means, but it does
not change the world. Gating it at `command` would force a consumer that only
observes to hold a capability that can spawn a group.

**The built-ins define no capability of their own.** `read` and `command` are
two of the three SPEC §14.4 defines. They need no third: they hold nothing
whose disclosure is narrower than `read`, because player identity and chat are
the hook driver's and gated under its own capabilities (HOOK §7). An adopter
who needs a narrower gate adds a capability in the 100-and-above range and a
record type to carry it, per SPEC §14.4's rule that a capability gates a record
type and never a field.

**No built-in record is `LIFECYCLE`.** The four DCS-Bridge reserves for the sim
driver are the resync brackets and the two driver-state records (SPEC §9), and
the built-ins add none. They therefore consume no `max_lifecycle_topics` slot
and carry no last-value obligation.

---

## 12. Test method

SPEC §17 gives the method for DCS-Bridge's own layers, and the rules below it
bind here too. Most rows need the sim; the two marked otherwise run against
the broker with no DCS present.

| Layer | Method | Host |
|---|---|---|
| Coordinates | Every position the sim driver emits is DCS-local, and converting one with the projection DCS-Bridge published lands it where `ConvertCoords` puts it. | Windows + DCS |
| MGRS parity | `ConvertCoords` returns the MGRS string the F10 map shows for the same point. That parity is the command's whole remaining justification. | Windows + DCS |
| Event coverage | Every id from 1 to 61 in `world.event` has a record type. A record type exists for an event the running mission never fires. | Windows + DCS |
| Event forwarding | A forwarded record's fields match the `S_EVENT_*` payload DCS delivered, with no smoothing. An event with an empty initiator forwards rather than raising. | Windows + DCS |
| State boundary | Every built-in topic routes to the sim driver ring, and no built-in declares a `target` option. | Windows + DCS |
| Classes and capabilities | Every built-in topic carries a class and a capability in the generated tables. A `read` token is refused a mutating command and accepted for `ConvertCoords`. | Any (native module) |
| Spots | `LaserOn` returns a handle in its typed reply; `LaserOff` destroys by that handle. A spot whose target is destroyed emits its record without the consumer polling. `EpochClosed` destroys every spot. A code outside 0–9999 is refused; 5711 and 9999 are both accepted. | Windows + DCS |
| Spot retry | `LaserOn` re-sent with the same `idempotency_key` creates one spot and answers twice, the second with outcome `DUPLICATE`. Both answers correlate on `request_id`. | Any (native module) |
| Weapon tracking | A filter's terms combine conjunctively. Tracking stops at `max_tracked_weapons` and the subscription counts against `max_subscriptions`. | Windows + DCS |
| Guards | Each Section 9 case is provoked and handled: an empty `S_EVENT_HIT` initiator, a nil `Group`, a missing `type`, an unimplemented `getID`, an undetectable category. | Windows + DCS |
| Unsafe bindings | With `unsafe_bindings_enabled` false, a handler that reaches a Section 8 binding is refused rather than called. | Windows + DCS |
| Flags | `trigger.misc.getUserFlag` reads what `trigger.action.setUserFlag` wrote. | Windows + DCS |

---

## 13. Open questions

DCS-Bridge keeps its own register in SPEC §16. These are this document's, in
the same shape. Each is a claim the text above rests on and the install cannot
settle.

**Six were settled on a live sortie, 2.9.29.27278, Caucasus**, and their
answers are folded into the sections above rather than left here: what
`Spot.createLaser` accepts, the shape of `env.mission.date`, whether the shape
family's first argument is a coalition id, whether `StaticObject` reaches
`destroy` and `isExist`, whether `Unit.getAmmo` entries carry a descriptor, and
whether the projection derivation reproduces published parameters. A retired
number is not reused.

| # | Question | Why it matters |
|---|---|---|
| — | Nothing is open on the sim side. | Every question this document raised has been answered against a running mission. The eight that remain belong to the hook driver and are listed in HOOK §12. |
