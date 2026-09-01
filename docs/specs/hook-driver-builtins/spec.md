# DCS-Bridge — The hook driver built-ins

The administrative surface shipped in the box: the player records and the
commands a moderation or server-administration consumer needs, and the
coordinate calibration DCS-Bridge publishes.

Companion to the DCS-Bridge specification. `SPEC §N` refers to it; `SIM §N`
refers to the sim driver's built-ins, the other half of what ships in the box.
Terms used here without definition are defined in the DCS-Bridge
specification's preamble.

**Every topic in this document is hook-targeted** (SPEC §8.2), so the whole set
runs with no sim driver loaded. Its handlers live in `HookDriver.builtin.lua`
and register through the SPEC §6.10 surface. Its cost tier is therefore the
hook tier of SPEC §3.1 — a DCS restart — where the sim driver built-ins' is a
mission reload.

**Status:** Draft.

---

## 1. What the hook driver built-ins are

The hook driver built-ins are DCS-Bridge's administrative surface: the player
records, the commands a moderation or server-administration consumer needs,
and the coordinate calibration of Section 10. They add nothing to the bridge.
**Every topic in them is hook-targeted** (SPEC §8.2): the hook driver emits
every record and handles every command, so the whole set works with no sim
driver loaded (SPEC §9.5), before the first mission, and at the edges of a
mission load. The cost follows from the target: adding or changing a topic in
this set is the hook tier of SPEC §3.1 — a DCS restart.

**Every built-in here runs in the GameGUI hook state.** Mission-scripting
surface — events, world reads, unit and group reads, markup, effects, flags,
the F10 menu and `ConvertCoords` — belongs to the sim driver's built-ins
(SIM §1), which run in the other state. One document per state, and the
division is by state rather than by taste: the bindings each set uses are
unreachable from the other.

Its handlers live in `HookDriver.builtin.lua` and register as `builtin.*` keys
through the SPEC §6.10 surface. An operator overrides them by key with `off`,
`replace` and `wrap`, or drops a file into `<write dir>\DCSBridge\hookdriver.d\`,
exactly as the sim driver built-ins are overridden through `simdriver.d\`
(SPEC §6.10.2 and §6.10.4). `hook_driver_disabled_files` suppresses a shipped
hook-side file at the same restart tier (SPEC §13.1).

`HookDriver.builtin.lua` is DCS-Bridge-owned: a release overwrites it, so an
adopter who edits it loses the edit at the next update. **The built-ins hold
no privilege an adopter's files lack**: they register through the same
surface, in the same order, under the same containment.

## 2. Act, do not veto

**A veto cannot wait. An action can.** The `onPlayerTry*` callbacks return a
verdict before they return, and no consumer round trip fits inside one. A veto
answers from local state or it does not answer, and this set holds no policy
state. SPEC §4 permits a deliberate veto; this set never uses one. **Every
handler returns nothing.**

**There are four `onPlayerTry*` callbacks, not the three ED's reference
lists.** `API\Sim_ControlAPI.md:31-35` names `onPlayerTryConnect`,
`onPlayerTrySendChat` and `onPlayerTryChangeSlot`.
`onPlayerTryChangeCoalition` is a fourth, defined at
`Scripts/Hooks/multiplayerCoalitionBlocker.lua:43`, registered at `:135`, and
returning a three-value form at `:67` — `false`, a localisation key, and a
cooldown in seconds. It appears nowhere in the reference. **Apply the
return-value rule to the `onPlayerTry*` shape, not to ED's list**, and do not
assume the four are uniform: `onPlayerTryConnect` has a two-value
`false, "reason"` form, and `onPlayerTrySendChat` returns a filtered message
string where an empty string drops the message. Treating all four as
boolean-returning is a bug.

**Every duty here is corrected after the fact rather than prevented.** A
player picks a wrong slot. The hook driver emits a record. The consumer decides.
The hook driver moves the player back. The correction costs a round trip against a
human. The script-side half measures 1 to 2.5 ms against a stopwatched
2.1 to 2.8 s slot-to-Fly; the full consumer round trip is unmeasured
(**[PROBE-19]**). Section 11 names the fallback.

| Duty | Mechanism |
|---|---|
| Ban, unban, list bans | `net.banlist_add_by_ucid`, `net.banlist_remove`, `net.banlist_get` |
| Kick | `net.kick(playerID, message)` |
| Move out of a slot | `net.force_player_slot(playerID, sideID, slotID)` |
| Restrict by coalition, airframe or slot | Watch `onPlayerChangeSlot` and `onPlayerChangeCoalition`. Move the player if the slot is wrong. |
| Refuse an unlinked player | Move to spectator with a message. Repeat on the next attempt. |
| Read chat | `onPlayerTrySendChat`, harvested. Section 5. |
| Warn, direct message, announce | `net.send_chat_to`, `net.send_chat` |
| Server control | `net.missionlist_run`, `net.load_next_mission`, `DCS.stopMission`, `DCS.setPause` |
| Restart the server under a watchdog | `DCS.exitProcess`, behind its own capability (Section 7) |
| Roster | Records out, plus a reply |

## 3. Player identity

**The hook driver keeps a map of player id to ucid.** A command travels a round
trip, and the named player can disconnect inside it. A player id is a
session handle. A ucid is durable. ED treats them the same way:
`Scripts\Hooks\webGUI.lua` bans by ucid and kicks by id in one function, and
resolves neither from the other.

**A command that names a player names it by ucid.** Five commands do
(Section 6). The other thirteen name no player.

**The hook driver resolves a ucid to a session id only where the underlying call
takes one.** `net.kick`, `net.force_player_slot` and `net.send_chat_to`
take a player id: the hook driver resolves through the map at dispatch.
`net.banlist_add_by_ucid` and `net.banlist_remove` take the ucid itself: no
resolution happens, and the named player need not be connected — an offline
ucid is the normal case for `UnbanPlayer`.

**Resolution that finds nobody is a no-op.** Acknowledge `NO_MATCH`.
**Resolution that finds more than one connected player does nothing.**
Nothing in ED's API states a ucid is unique among connected players, and
acting on every match would turn an ambiguity into a multiple kick.
Acknowledge `AMBIGUOUS` and act on nobody.

The built-ins add two outcomes to `CommandAckOutcome`, in the built-ins' range
of 50 to 99 (SPEC §8.2):

```proto
COMMAND_ACK_OUTCOME_NO_MATCH  = 50;   // the ucid names nobody connected
COMMAND_ACK_OUTCOME_AMBIGUOUS = 51;   // the ucid names more than one
```

A consumer that does not know them reads `FAILED` (SPEC §8.5.3). A
moderation consumer must tell them apart, which is why they are members
rather than `detail` strings.

**Harvest and backstop.** `onPlayerConnect` carries no ucid.
`onPlayerChangeSlot` carries no side and no slot. The veto callbacks carry
both:

| Veto callback | Carries | Sibling | Carries |
|---|---|---|---|
| `onPlayerTryConnect` | addr, name, ucid, playerID | `onPlayerConnect` | id, and a measured name argument (Section 3.2) |
| `onPlayerTryChangeSlot` | playerID, side, slotID | `onPlayerChangeSlot` | id |
| `onPlayerTrySendChat` | playerID, msg, to | none | — |

**Harvest the delta from the veto callback. Emit from the sibling.** The
harvest parks the values. The sibling proves the event happened and emits
the record. A `Try` firing is not proof: a script later in the chain can
still refuse.

**A harvest parks values and decides nothing.** The callback stores what it
was handed, returns nothing, and the sibling emits the record.

**The slot harvest normally does not arrive.** ED's
`Scripts\Hooks\multislot.lua` registers `onPlayerTryChangeSlot` and returns
a value on every path, and the install tree is globbed before the write
directory's (SPEC §13). Read the side and the slot with
`net.get_player_info` under the Section 3.1 guard. Register the harvest
anyway: it costs nothing, and `multislot.lua` is not guaranteed present on
every build.

**Where a harvest does not arrive, fall back to `net.get_player_info`.**
Count it in `harvest_preempted_total`. Where the fallback returns nil, omit
the field and count it in `player_info_nil_total`. On a 2.9.29.27278 host,
every lookup answers inside both callbacks (Section 3.1), so the fallback
is the healthy path, not a rescue.

**Rebuild the map at `onMissionLoadEnd`.** Iterate `net.get_player_list` and
re-read each player's ucid under the guard below. The rebuild repairs the one
entry that never clears: `onPlayerDisconnect` never fires for a listen
server's own player. A dedicated server is unaffected. Report the map's
current size as `roster_entries` (SPEC §12).

### 3.1 The `net.get_player_info` guard

**`net.get_player_info` can return nil inside a hook callback.** Guard the
whole-table form before indexing it, and the two-argument form before using
the value. `onPlayerChangeSlot` is the confirmed case; treat the others as
unconfirmed rather than safe. On 2.9.29.27278, host-side, 22 consecutive
in-callback lookups all answer. The guard stays: the nil case is on record
from an earlier build, and a dedicated server is unmeasured. Every reference
in this document to the guard is this rule.

### 3.2 Three ED facts this document rests on

Measured on **2.9.29.27278**. They are the most perishable text here, and they
correct ED's own shipped reference. Re-measure them after a DCS update.

**`onPlayerConnect` carries a second argument, the player name.** ED's
reference documents `onPlayerConnect(id)` with one argument. ED's own GUI
handler at `MissionEditor\GameGUI.lua:865` is `function onPlayerConnect(id,
name)` and forwards the name onward, and `onGameEvent` documents `--"connect",
playerID, name` at `API\Sim_ControlAPI.md:606`. Prefer the argument over a
`net.get_player_info` call and fall back to that call under Section 3.1. *The
measurement is of ED's GUI handler, not of a write-directory hook.* Confirm
the argument arrives in a write-directory hook before depending on it.

**`onPlayerDisconnect`, `onPlayerStart` and `onPlayerStop` never fire for a
listen server's own player.** ED's reference carries the identical comment `--
this is never called for local playerID` on each of the three, at
`API\Sim_ControlAPI.md:645`, `:650` and `:655`. `onPlayerConnect` at `:641`
does not carry it, and whether it fires for the local player is unmeasured
(**[PROBE-A6]**). A consumer that counts players from the three is wrong by
one on a listen server; a dedicated server has no local player and is
unaffected.

**`onPlayerTryConnect` is documented but defined in no shipped file.** It
appears at `API\Sim_ControlAPI.md:33`, `:44` and `:670`, in no state's index
and in no ED `.lua` file. So no ED script preempts it, and none demonstrates
it either. Register the harvest and treat `net.get_player_info(id, 'ipaddr')`
under Section 3.1 as the expected source for `PlayerAddress` rather than as a
rescue.

## 4. Records

Fourteen. All hook-targeted. All `DURABLE`. **None is `LIFECYCLE`**, so the
built-ins hold no `max_lifecycle_topics` slot (SPEC §13.1). The server facts a
consumer needs to interpret this set — the mission name and the `is_server` and
`is_multiplayer` pair — are fields on DCS-Bridge's own `EpochOpened`, not
records here (SPEC §6.3).

| Record | Carries | Capability |
|---|---|---|
| `PlayerConnected` | ucid, name, player id | `players` |
| `PlayerAddress` | ucid, addr | `ipaddr` |
| `PlayerDisconnected` | ucid, player id, reason code | `players` |
| `PlayerStarted` | ucid | `players` |
| `PlayerStopped` | ucid | `players` |
| `PlayerChangedSlot` | ucid, player id, side, slot id | `players` |
| `PlayerChangedCoalition` | ucid, player id, side | `players` |
| `PlayerChatted` | ucid, text, `to`, `to_raw`, `source` | `players` |
| `SlotList` | coalition, slot entries | `read` |
| `Banlist` | ban entries, chunked; emitted on `GetBanlist` | `moderate` |
| `Roster` | reply to `GetRoster` | `players` |
| `MissionList` | reply to `GetMissionList` | `mission` |
| `ServerStatus` | reply to `GetServerStatus`: real time, paused | `read` |
| `MissionBriefing` | reply to `GetMissionBriefing`: text, `truncated` | `read` |

`Roster`, `MissionList`, `ServerStatus` and `MissionBriefing` are typed
replies (SPEC §8.5.2),
delivered with `begin_to` (SPEC §5.1). A reply is point-to-point, so the outbound
capability filter never tests it (SPEC §14.4): the capability that gates
the exchange is the one on the requesting command. `Banlist` is not a
reply: it fans out in bounded chunks on `GetBanlist`, the same shape as
`SlotList` on `GetSlotList` (Section 6).

| Record | Emitted from | Field | Source |
|---|---|---|---|
| `PlayerConnected` | `onPlayerConnect` | ucid | harvest, else `get_player_info` |
| | | name | the measured second argument, else `get_player_info`, both under the Section 3.1 guard |
| `PlayerAddress` | `onPlayerConnect` | addr | harvest, else `get_player_info(id,'ipaddr')` |
| `PlayerDisconnected` | `onPlayerDisconnect(id, code)` | ucid | the map |
| `PlayerStarted` | `onPlayerStart(id)` | ucid | the map |
| `PlayerStopped` | `onPlayerStop(id)` | ucid | the map |
| `PlayerChangedSlot` | `onPlayerChangeSlot(id)` | side, slot | `get_player_info` |
| `PlayerChangedCoalition` | `onPlayerChangeCoalition(id, side)` | side | the callback argument |
| `PlayerChatted` | the harvest | all | `onPlayerTrySendChat` args |
| `SlotList` | `onMissionLoadEnd` | entries | `DCS.getAvailableSlots` |

**Two slot-event gaps are measured on 2.9.29.27278.** A manual move to
spectators fires no `onPlayerChangeSlot`, so no `PlayerChangedSlot` reports
it: a consumer's slot view goes stale until the player's next slot entry,
and `GetRoster` is the recovery. A dynamic-slot join fires the callback
**twice**, about 2 ms apart with identical values, and dynamic slot ids are
synthetic incrementing values such as `1000001`. The hook driver emits both
records; a consumer applies `PlayerChangedSlot` idempotently, the same rule
SPEC §6.8 sets for every transition.

**Read the map before clearing it.** Three records take a ucid from the map.
The map entry is cleared at `onPlayerDisconnect`. The order inside that
callback is read, emit, then clear.

**`SlotList`.** `DCS.getAvailableSlots` works in the hook state; ED calls it
in `Scripts\Hooks\multislot.lua`. **The argument is a coalition name string,
and an id returns nil rather than raising** (measured, 2.9.29.27278):
`getAvailableSlots("blue")` answers a table and `getAvailableSlots(2)` answers
nil. ED builds `"neutrals"`, `"red"` and `"blue"`; the shipped reference calls
the argument a coalition id and is wrong. **Guard the nil**, because a driver
that passed an id would emit nothing and look healthy. A coalition with no
slots answers an empty table rather than nil, so an empty `SlotList` is a real
answer.

**Forward the whole entry, renamed, rather than a chosen subset.** An entry
carries eighteen keys (measured, 2.9.29.27278): `airdromeId`, `callsign`,
`countryId`, `countryName`, `groupName`, `groupSize`, `multicrew_place`,
`name`, `onboard_num`, `parking_id`, `role`, `roleCategorie`, `startX`,
`startY`, `takeOffType`, `task`, `type`, `unitId`. Four traps come with them.

**DCS's own keys are mixed case**, so SPEC §8.2's snake_case rule renames some
and passes others through: `multicrew_place`, `onboard_num` and `parking_id`
already arrive snake_case. **There is no `country` key** — `countryId` and
`countryName` are two fields, and a record carrying one called `country`
invents it. **`roleCategorie` is ED's own misspelling**; carry it as
`role_categorie` rather than correcting it, because a corrected name no longer
matches what ED returns. **`callsign` is a table, not a string**, so it is a
submessage rather than a scalar field.

**`startX` and `startY` are the slot's position, and `startY` is DCS `z`,
east.** ED names the ground plane X/Y where SPEC §8.2 pins x north and z east.
Emit them under the specification's names, not ED's.

`unit_id` is a slot id and a string — measured `"1"` — and a multi-seat unit's
has the form `unitID_seatID`. The hook driver emits `SlotList` at
`onMissionLoadEnd`, once per coalition, and again on `GetSlotList` (Section 6),
at most `slotlist_max_entries` entries per record, repeating until done: a
large mission's slot table can exceed `max_frame_bytes`.

**`Banlist` and `Roster` carry no address.** `net.banlist_get` returns
`ipaddr` in every entry, and `ipaddr` is an attribute of
`net.get_player_info`. A capability gates a record type and never a field
(SPEC §14.4), so a reply carrying an address could not ship under
`moderate`. The hook driver drops the address on the way out. `PlayerAddress` is
the only record that carries one.

**`PlayerChangedCoalition` needs no harvest, and its `Try` sibling is
unusable.** `onPlayerChangeCoalition(id, side)` carries both fields itself, so
the record is emitted from it directly and reports a change that happened, not
an attempt. Two shipped scripts register it —
`Scripts/Hooks/multiplayerCoalitionBlocker.lua:135` and
`Scripts/Hooks/multislot.lua:220`, which maps it to `onPlayerLeaveSlot` — and
both coexist, which is what proves it does not end the hook chain. **Do not
register `onPlayerTryChangeCoalition`.** Every path through the blocker's
handler returns a value, including `params.isActive == false`, so it consumes
the event even where an operator has switched the blocker off; a harvest there
would never arrive in any configuration. Neither callback appears in ED's
reference — both are measured from ED's own source, 2.9.29.27278.

**`PlayerChatted` reports an attempt.** It is emitted from the harvest and
has no sibling, so a script later in the chain can still drop the line
after the record is emitted. `source` and Section 13's moderation harvest
row state the consequence.

## 5. Chat

**`PlayerChatted` comes from `onPlayerTrySendChat`, harvested.** The
handler returns nothing; SPEC §4.5 records the precedent. **Never
return the message. Never return an empty string.** Chat is read, not
intercepted. An empty string drops the line for every player.

Three sources:

| Source | Keys on | Carries `to` | Shipped by anyone |
|---|---|---|---|
| `onPlayerTrySendChat`, harvested | ucid | yes | DCS-gRPC |
| `onChatMessage` | player id | yes, measured | no project found |
| `net.get_chat_history`, polled | player name | no | no project found |

**Register the harvest. Use `onChatMessage` as well if it fires.** ED files
`onChatMessage` under GUI callbacks. The shipped reference documents it as
`onChatMessage(message, from)`; ED's own GUI handler is implemented as
`onChatMessage(message, from, to)` and reads the third argument — the same
reference-behind-implementation shape as `onPlayerConnect`'s name argument
(Section 3.2). No shipped hook registers it, and a dedicated server
replaces the GUI script. On a 2.9.29.27278 host, three arguments arrive as
`(message, from, to)`, with `-1` for all chat and `-2` for team chat.
Whether it fires on a dedicated server is unmeasured. **[PROBE-17]**

**Take the harvest when both fire.** Deduplicate on the player id and the
message text within one hook-loop invocation (SPEC §6.4). ED's GUI code
also calls `onChatMessage` itself for synthesized announcement lines that
carry no sender: treat an absent `from` as the system, never as a player.

**Use the history poll only if neither callback fires.** It carries no
player id and no ucid. It keys on a display name, which a duplicate name
defeats. Its `side` field is the sender's coalition and not a chat target.

`source` tells a consumer which mechanism produced the record. A consumer
must read it. The keying differs between them.

**Every record and command here lives in `dcs.builtin`**, the package the sim
driver built-ins also use (SPEC §8.2). One package for the built-ins: the Lua
state a topic is produced from is an implementation detail, and a consumer
should not have to read it off a topic name.

The two enums below are new enums in that package, not extensions of a bridge
enum, so SPEC §8.2's range table does not reach them and they number from
zero. SPEC §8.4 governs their evolution:

```proto
// EMITTED at 0: the driver maps what it recognises and sends UNSPECIFIED for
// the rest — see to_raw. Not extensible.
enum ChatTarget {
  CHAT_TARGET_UNSPECIFIED = 0;   // unrecognised; see to_raw
  CHAT_TARGET_ALL         = 1;
  CHAT_TARGET_TEAM        = 2;
}

// NEVER EMITTED at 0: the driver always knows which mechanism it read from.
// Not extensible.
enum ChatSource {
  CHAT_SOURCE_UNSPECIFIED   = 0;
  CHAT_SOURCE_TRY_SEND_CHAT = 1;
  CHAT_SOURCE_CHAT_MESSAGE  = 2;
  CHAT_SOURCE_CHAT_HISTORY  = 3;
}
```

**`to` is an enum and not a boolean.** DCS passes a target id. ED compares
it against `net.CHAT_ALL` in `MissionEditor\modules\mul_chat.lua`, and
DCS-gRPC tests a sentinel integer. Both treat it as an integer sentinel.
**`to_raw` carries the unmapped value.** `net.CHAT_ALL` is `-1` and
`net.CHAT_TEAM` is `-2` in the hook state (measured, 2.9.29.27278), which
fixes the `ChatTarget` mapping for both members. `to_raw` still ships: SPEC
§14.1 treats a player as untrusted, a modified client reaches the sim over
DCS's own network path, and an unrecognised target id must map to
`CHAT_TARGET_UNSPECIFIED` rather than be guessed at.

## 6. Commands

Eighteen. All hook-targeted. All class `COMMAND`.

**`GetRoster`, `GetMissionList`, `GetServerStatus` and `GetMissionBriefing`
declare `reply_to`** and are answered by their typed reply on success, or by
`CommandAck` on failure. **The other fourteen declare none** and are answered by `CommandAck` (SPEC
§8.5.3). Two of those, `GetBanlist` and `GetSlotList`, additionally
trigger fan-out records — the `Resync` shape (SPEC §6.8 and §8.5.3):
the records a command triggers are not its answer, so a bounded, chunked
result set does not break the one-answer contract. A consumer that wants
the result subscribes to the record it triggers.

**Each command requires exactly one capability.** SPEC §8.2's
`required_capability` option carries one member, and the broker enforces
one capability per topic (SPEC §5.1 and §14.4). A duty that needs two
powers is two commands.

| Command | Maps to | Capability |
|---|---|---|
| `KickPlayer` | `net.kick(id, message)` | `moderate` |
| `BanPlayer` | `net.banlist_add_by_ucid`, then `net.kick` | `moderate` |
| `UnbanPlayer` | `net.banlist_remove(ucid)` | `moderate` |
| `GetBanlist` | `net.banlist_get()` | `moderate` |
| `ForceSlot` | `net.force_player_slot(id, side, slot)` | `moderate` |
| `GetRoster` | `net.get_player_list`, then `net.get_player_info` per id, bounded at `roster_max_players` | `players` |
| `SendChatTo` | `net.send_chat_to` | `command` |
| `SendChatAll` | `net.send_chat` | `command` |
| `LoadMissionByIndex` | `net.missionlist_run(index)` | `mission` |
| `LoadNextMission` | `net.load_next_mission()` | `mission` |
| `ReloadCurrentMission` | `net.load_mission(DCS.getMissionFilename())` | `mission` |
| `GetMissionList` | `net.missionlist_get()` | `mission` |
| `StopMission` | `DCS.stopMission()` | `mission` |
| `SetPause` | `DCS.setPause(bool)` | `mission` |
| `GetSlotList` | re-emits the `SlotList` set (Section 4) | `read` |
| `GetServerStatus` | `DCS.getRealTime`, then `DCS.getPause` | `read` |
| `GetMissionBriefing` | `DCS.getMissionDescription()` | `read` |
| `ExitProcess` | `DCS.exitProcess()` | `process` |

**Reads carry `request_id`. Mutations carry `idempotency_key`.** SPEC §8.5.2
gives the rule. `GetBanlist`, `GetRoster`, `GetMissionList`,
`GetSlotList`, `GetServerStatus` and `GetMissionBriefing` are the six reads; a re-executed read is
harmless, so none needs an idempotency key. **A duplicate key executes nothing and is acknowledged with
outcome `DUPLICATE`** — SPEC §8.5.2's rule, unchanged: no outcome is
stored or replayed. The hook driver's recent-key set is bounded at
`recent_admin_keys` and covers the twelve mutating commands only.

**A consumer string above its byte cap fails the whole command**,
acknowledged `FAILED` with the reason in `detail`. SPEC §13.1 sets the caps —
`kick_message_max_bytes` 256, `ban_reason_max_bytes` 256 and
`chat_message_max_bytes` 512 — and states the same rule. Truncation would
silently alter what DCS stores or what a player reads.

**Validate the ban period as a non-negative integer of seconds.** SPEC §13.1
defines no `ban_period_max_seconds` on purpose: ED's own "ban forever"
checkbox writes `16293600000` seconds, about 516 years, in
`MissionEditor\modules\mul_banned.lua`, and a cap would break the case an
operator most wants. **That sentinel exceeds 2^31, so no field carrying a ban
period may be 32-bit** (SPEC §13.1).

**An idempotency key above `idempotency_key_max_bytes` fails the command** the
same way. SPEC §13.1 sets it at 64 bytes, and the hook driver stores
`recent_admin_keys` of them.

**Route every `DCS.*` call through the hook driver's accessor table**, one
call at a time, each guarded, per SPEC §4, §4.3 and §4.4.

**Bans.** Two functions ban. They differ:

| Function | Keys on | Kicks | Documented |
|---|---|---|---|
| `net.banlist_add(playerID, period, reason)` | session id | yes | `API\Sim_ControlAPI.md` |
| `net.banlist_add_by_ucid(ucid, period, reason)` | ucid | no | nowhere |

**Use the by-ucid form. Kick separately.** ED does the same in
`Scripts\Hooks\webGUI.lua`. **`BanPlayer` bans, then kicks.** A ban that
leaves the player flying is not what an operator asked for. **The outcome
reports the ban; the kick reports in `detail`.** A failed ban is `FAILED`.
A successful ban whose kick finds nobody connected is `OK`, with the kick's
result in `detail`: the ban is the commanded essential and can succeed
where the kick finds nobody.

**`banlist_add_by_ucid` is measured on 2.9.29.27278.** `period` is seconds:
`banned_until` is `banned_from` plus the period, so a permanent ban is a
very large period and no sentinel exists. The banlist survives a full DCS
restart, even client-hosted. **Re-adding a banned ucid replaces its
entry**, so `BanPlayer` on an already-banned player updates the period and
reason and acknowledges `OK`. `banlist_get` returns nil with no server
running — measured again on 2.9.29.27278, single-player host — and returns a
table once a server is up, empty where nothing is banned (measured, hosted
multiplayer, same build). So an empty banlist is a real answer and nil is not:
a `GetBanlist` dispatched with no server up is acknowledged `FAILED` with that
reason, and the nil is guarded rather than indexed. Whether DCS refuses a banned connection before
any Lua runs is unmeasured — `net.ERR_BANNED` is `101` in the hook state
(measured, 2.9.29.27278), a client-side disconnect reason code, not proof of
a pre-Lua refusal. **[PROBE-18]**

**Chat output.** **`SendChatTo` rejects `net.CHAT_ALL` and
`net.CHAT_TEAM`.** Both are special player ids that broadcast, and a
mis-addressed id would publish a private message to the whole server. It
also rejects a resolved id the map does not tie to a connected player —
the `NO_MATCH` path in Section 3. ED describes `net.send_chat_to` as
a direct chat message to a player and never says it is invisible to
others; **[PROBE-20]**. The rejection of the broadcast ids is the
mitigation that does not depend on the probe.

**`GetServerStatus` answers what a consumer asks for rather than subscribes
to**: the real time from `DCS.getRealTime` and the pause state from
`DCS.getPause`. The split against `EpochOpened` is push against pull, not fast
against slow. `EpochOpened` pushes what a consumer must hold to interpret the
stream at all — the epoch id, the terrain, the mission name, the deployment
pair (SPEC §6.3). This command answers what a consumer only sometimes wants,
and pushing either value would be a subscription to a clock. It takes `read`:
neither call mutates anything, and gating them at `command` would force an
observer to hold a capability that can kick a player. Both calls route through
the accessor table one at a time, per SPEC §4.3.

**`GetMissionBriefing` is a command rather than a field on any record**, and
the reason is its size. The mission briefing is author-written and unbounded in
principle: the largest in ED's shipped missions is 924 bytes, but nothing
bounds it. Carrying it on `EpochOpened` would couple an unbounded string to the
one record every consumer must hold to interpret the stream — and SPEC §5.2
refuses a `LIFECYCLE` record larger than `max_lifecycle_record_bytes`, so one
verbose briefing would cost every consumer its epoch id. A read command
carries it only to a consumer that asks.

**It truncates rather than fails, and says so.** SPEC §13.1's
`mission_briefing_max_bytes` bounds the text; over the cap the hook driver
truncates on a UTF-8 boundary and sets `truncated` on the reply. This is the
opposite of the rule for consumer strings above, and deliberately: an inbound
string that DCS will store must fail rather than be silently altered, while an
outbound read is more useful clipped than refused. The flag is what keeps it
from being silent.

**`ReloadCurrentMission` takes no argument.** It reloads
`DCS.getMissionFilename()`. `net.load_mission` is never exposed with a
consumer-supplied path: a `.miz` is an archive of Lua that runs in the
mission scripting state, so such a path is a file name that carries code,
which SPEC §14.6 forbids. There is no `net.reload_current_mission` — confirmed
absent from every state's index and from the whole install.

**The call is role-dependent, and it fails silently on the wrong role**
(measured, 2.9.29.27278, one process, both roles). In a hosted multiplayer
session `net.load_mission(<path>)` returns `0` and reloads the mission. On a
single-player host it returns nothing, raises nothing, logs nothing and reloads
nothing — with the path as `DCS.getMissionFilename()` gives it and with its
doubled separator normalised. Only the return arity tells the two apart: two
values against one.

**So `ReloadCurrentMission` checks the arity and refuses where the call answers
nothing.** Acknowledge `FAILED` naming the role rather than `OK`, because a
command that reports success while nothing happened is worse than one that
refuses. What `0` means beyond "answered" is unverified; a dedicated server is
unmeasured (**[PROBE-A11]**).

**What is not exposed.** **No command carries a file path.** SPEC §14.6
forbids it; a `.miz` is an archive of Lua. **Never expose
`net.missionlist_append`, `missionlist_delete`, `missionlist_move` or
`missionlist_clear`.** The first names a file. The rest let a consumer
rewrite the operator's rotation.

**`ExitProcess` is exposed, and deliberately not under `mission`.** Ending
the server process is a different power from ending a mission — SPEC §11's
kill switch disables only the bridge — and under a process watchdog
it is the restart-server button. It requires the `process` capability,
which no grant in Section 7's table includes.

## 7. Capabilities

The built-ins define five capability members in the built-ins' range of 50 to
99 (SPEC §14.4). They live here and not in SPEC §14.4: a capability named for
a role is domain content (SPEC §14.4).

```proto
// The built-ins' range, per SPEC §14.4.
CAPABILITY_PLAYERS  = 50;   // identity, chat, roster
CAPABILITY_MODERATE = 51;   // power over people
CAPABILITY_MISSION  = 52;   // power over the mission
CAPABILITY_IPADDR   = 53;   // network addresses
CAPABILITY_PROCESS  = 54;   // power over the process itself
```

**Split a capability when the two halves can hurt different things.**

`moderate` and `mission` are separate. A consumer that moderates people
should not be able to end everyone's flight. A rotation scheduler should
not be able to ban anyone.

`players` and `read` are separate. `read` carries world data, and leaking
that to one coalition is cheating. `players` carries identity and chat, and
leaking that is a privacy failure that outlives the mission (SPEC §14.7).

`process` is separate from `mission`. Ending the DCS process ends every
mission that would have followed it, and under a process watchdog it
restarts the server. That is an operator-infrastructure power, not a
mission power. No grant in the table below includes it.

`ipaddr` is separate from `moderate`. Most moderation works entirely by
ucid. A consumer that never reads an address should not hold every
player's address. `ipaddr` is named for the field it gates:
`net.get_player_info(id, 'ipaddr')` is the call behind `PlayerAddress`.

**The set is additive. No capability implies another.** Token grants for
common consumers:

| Consumer | Token's capability set |
|---|---|
| Live map | `read` |
| Chat archiver | `players` |
| Moderation consumer | `players`, `moderate`, `command` |
| Ban-evasion detection | the above plus `ipaddr` |
| Rotation scheduler | `mission`, plus `command` if it announces |
| Restart controller | `process` |

**A ucid ships under `players`.** It is a durable cross-server identifier.
A live map holding only `read` never receives one (SPEC §14.7).

## 8. The bounce

Three duties end in `net.force_player_slot` (Section 2).

**A bounce can re-trigger itself.** `ForceSlot` changes a slot. Changing a
slot fires `onPlayerChangeSlot` — and the hook driver's dispatch loop runs from that
callback (SPEC §6.4), so a queued `ForceSlot` can execute inside the
callback a previous one caused. A 2025 forum report describes
`net.force_player_slot` under `onPlayerChangeSlot` running in a loop
forever. On 2.9.29.27278 the forced move **re-fires the callback**, 1 to 2 ms
after the force, and the forced event reports side 0 with an empty slot. A manual
move to spectators fires nothing (Section 4), so on that build a
spectator-slot event is always a forced move. **The guard is required.**
An unguarded bounce is a consumer-triggerable server hang.

**Suppress the bounce you caused.** Record the ucid and the target slot
before calling `net.force_player_slot`. At the next `onPlayerChangeSlot`
for that ucid reporting that slot, emit `PlayerChangedSlot` normally — the
stream must reflect the world — but count it in
`bounces_suppressed_total` and clear the marker. **Refuse a second bounce
for one ucid inside `bounce_min_interval_ms`**, acknowledge it `FAILED`,
and count it in `bounces_refused_total`. The interval, not the marker, is
what breaks a consumer that re-forces on every record.

**Dynamic slots behave differently.** Two forum reports say
`onPlayerTryChangeSlot` does not fire for a dynamic slot, and that a
dynamic slot can report itself occupied after an interrupted spawn. The
first costs nothing here: Section 3 already treats the slot harvest
as normally absent. A dynamic join double-fires `onPlayerChangeSlot` with
synthetic ids (Section 4, measured); whether the `Try` callback
fires for one is unmeasured.

**A bounce cannot catch a slot that became invalid after it was taken.** A
slot is checked when a player takes it. A rule that changes afterwards
leaves a player in a slot the rule now forbids.

**Re-check on demand, not on a timer.** SPEC §8.5.2 ends with the rule:
a consumer that polls with the request-and-reply pattern wants SPEC §8.5.1
instead. A timed `GetRoster` loop is that mistake. The re-check has
one trigger and it is not the clock: a consumer changes its own policy,
sends one `GetRoster`, and acts on the reply. Every other case is already
covered by `PlayerChangedSlot`, which arrives when a player acts.

**There is nothing to subscribe to here.** The predicate is the consumer's
policy and the bridge cannot evaluate it. That is why this stays a request
and reply rather than becoming a subscription under SPEC §8.5.1.
DCS-SimpleSlotBlock sweeps on a timer because it has no consumer to ask.
This design does.

## 9. Audit

Every Section 6 command appends one line to
`Logs\DCSBridge\admin-audit.log` before its acknowledgement is emitted.
The file is the eval audit's sibling, not the same file: SPEC §7.6
defines `eval-audit.log` as one line per eval execution, and the two carry
different line shapes. `admin_audit_max_bytes` and
`admin_audit_retention_days` bound it, rotated at the size cap with the
oldest rotated file deleted first, the same order as SPEC §7.6.

| Field | Content |
|---|---|
| UTC timestamp | `os.date('!%Y%m%d-%H%M%S')` |
| Connection id | which session sent it |
| Token id | the credential behind the session. Never the secret. |
| Action | the command's topic name |
| Target ucid | or `-` |
| Arguments | truncated at the SPEC §13.1 byte caps |
| Correlation | `idempotency_key` or `request_id` |
| Outcome | the value `CommandAck` carried |

The hook driver resolves the token id from the `connection_token_id.<conn>` key
in `stats` (SPEC §5.1 and §12): the broker knows which token
authenticated each connection, and the id comes from the token's entry in
`Config\DCSBridge.lua` (SPEC §13.1). The secret appears nowhere.

**Log every command, not only the `moderate` ones.** `SendChatAll` puts a
string in the server's name in front of every player. Count every dispatch in
`admin_commands_total`, by command and outcome (SPEC §12).

**A failed audit write does not block the command.** An audit that can
stop moderation is a denial-of-service lever. Count it in
`admin_audit_write_failures_total` and log the failure — never the record
contents — to `dcs.log` (SPEC §14.7).

**A size-bounded local file is not tamper-evident.** An operator needing a
real audit ships lines off-box.

## 10. Coordinate calibration

SPEC §6.3 fixes what `CoordinateCalibration` carries on the wire. This section
fixes the derivation behind it: how the hook driver obtains the projection
parameters, how it publishes verification points, and how it samples magnetic
declination.

**The hook driver does this work because only the hook state can.**
`terrain.convertMetersToLatLon` is in the hook state and `coord.*` is in the
mission-scripting state and nowhere else; `magvar` is in the gui state and
nowhere else (measured, 2.9.29.27278). The derivation runs once per epoch,
before `CoordinateCalibration` is emitted. The sim driver computes nothing for
it, and every position the sim driver emits is DCS-local so the published
projection reads all of them (SIM §4).

### 10.1 The derivation

Every measured DCS terrain projects with one family: transverse Mercator on
WGS84 with `k_0 = 0.9996`, a central meridian at an odd multiple of 3 degrees,
and a per-terrain false easting and northing. That is UTM's parameter set with
a per-terrain origin in place of UTM's 500000/0.

**The central meridian is a choice among sixty, not a fitted quantity.** Odd
multiples of 3 degrees are exactly the sixty UTM zone meridians. Once the
meridian is fixed, the false easting and northing follow analytically from one
point: convert that point with the origin at zero and negate the result. So
the hook driver derives the whole parameter set in three steps and without a
least-squares fit:

1. Convert one reference point with `terrain.convertMetersToLatLon`.
2. For each of the sixty candidate meridians, compute the false easting and
   northing that place that point exactly, and keep the candidate that also
   places a second, distant point.
3. Convert a third point and check it against the derived parameters.

**One point is enough. Do not average.** The forward conversion is exact, so
every reference point yields the same origin.

**The derivation is measured, on Caucasus, 2.9.29.27278.** Step 2 selected
central meridian **33**, and the margin is not close: the runner-up, meridian
39, misplaces the second point by **88.8 km**. Nine reference points spanning
the full terrain — the four bounds corners of Section 10.2, the origin, a
float32-inexact point, and three interior points — each give `x_0 =
-99517.000000` and `y_0 = -4998115.000000` **from that point alone**. The
worst residual across all nine, against an origin derived from one of them, is
**0.000002 mm**. `proposals\caucasus-live-verification.py` carries the
measured points and reproduces the result against PROJ 9.5.1.

**The published community values are corroboration, never the reference.**
`-99516.9975` and `-4998115.0006` are 2.5 mm and 0.6 mm from the true ones.

**The float32 cost belongs to the reverse conversion, not to the derivation.**
`terrain.convertMetersToLatLon` and `coord.LOtoLL` agree to twelve decimal
places on a coordinate float32 represents exactly, and appear to disagree by a
few millimetres on one it cannot; the residual is the float32 grid rather than
the arithmetic. `coord.LLtoLO` returns float32: fed `-219720.8` it answers
`-219720.796875`, an error of 3.125 mm at 220 km. That is why SPEC §5.3 prints
position fields with `%.9g`, and it is not a reason to average anything here.

**No projection parameter appears in any shipped `.lua`.** The standalone
literal `0.9996` occurs in no `.lua` file in the install, and only Caucasus has
been derived at runtime. Deriving per epoch and checking is what makes the
remaining terrains safe to assume.

### 10.2 Verification points and the family check

**The verification points are published for checking, not for fitting.** A
consumer checks the parameters against them; it never derives parameters from
them. They also carry the declination of Section 10.3.

**Publish four points, at the corners of the terrain's own declared bounds.**
`terrain.GetTerrainConfig('SW_bound')` and `terrain.GetTerrainConfig('NE_bound')`
each return a three-vector in **kilometres**; multiply by 1000 and take the
four corners of the box they describe. This is ED's own construction rather
than an invention: `MissionEditor\modules\me_setCoordPanel.lua:337-348` reads
the same two keys and converts at both corners, and the AH-64D DTC modules
derive their whole legal coordinate range the same way and clamp every user
coordinate into it. Four corners rather than three, because a fourth corner is
the only point the other three cannot place by interpolation.

**The call and its axis order are measured** (2.9.29.27278, Caucasus).
`terrain.GetTerrainConfig` answers in the hook state, on a loaded mission,
with no `terrain.Init` of its own. Each key returns a three-vector in
kilometres whose second element is zero: Caucasus gives `SW_bound =
{-600, 0, -560}` and `NE_bound = {380, 0, 1130}`. **Index 1 is x, north;
index 3 is z, east** — converting the corners under that reading places them
at 39.61N 27.64E and 47.38N 49.31E, which is Caucasus, while the swapped
reading puts the north-east corner 55.07N 40.52E, some 700 km off the map.
ED's own code crosses these two readings, so take this one:
`MissionEditor\modules\me_map_window.lua:921` agrees with it and the AH-64D
DTC's `terrainBounds` assignment does not. **Converting at the corners does not
raise.** Where the call fails on some other terrain, fall back to the
derivation's own reference points and log once.

**Never publish a point the parameters were derived from.** Two points derive
them and a third checks them; all three are internal to Section 10.1 and none
of them ships.

**The residual threshold is 1 mm, in both easting and northing.** Step 3's
check runs entirely on `terrain.convertMetersToLatLon`, which takes and returns
full-precision doubles, so the float32 grid of the reverse conversion never
enters it. The measured Caucasus residuals sit at the sub-micrometre level and
a terrain genuinely outside the family would miss by centimetres or metres, so
1 mm sits in a gap four orders of magnitude wide at either end. It is a policy
choice inside that gap, not a fitted number.

**A terrain outside the family is detected, not assumed away.** Where the third
point misses by more than 1 mm, the hook driver emits `CoordinateCalibration` carrying the terrain
name and the verification points, omits `projection` and the PROJ string, and
logs the residual once. A consumer that finds no `projection` falls back to
the sim driver's `ConvertCoords` (SIM §4).

**Publish no EPSG code.** A re-origined UTM zone has none and can be given
none. SPEC §6.3 states the same rule for the wire format.

### 10.3 Magnetic declination

**The declination is the hook driver's end to end.** `magvar` lives in the gui
state and nowhere else, so reaching it needs `net.dostring_in('gui', ...)`. The
`"server"` state holds no `net`, so a Route A sim driver cannot make that call.
A Route B one can — the mission scripting state holds `net` and reached
`magvar` in a measurement (2.9.29.27278) — so on that route the hook driver owns
this by SPEC §6.6's rule rather than by reach. The mission-scripting state has
no candidate of its own: `Export.LoGetMagneticYaw` takes no argument, so it
reports own-ship yaw and cannot answer "declination at point P".

**Seed the epoch, then sample per point.** Call `magvar.init(month, year)` —
ED's own argument order, at `MissionEditor\modules\me_weather.lua:1479` —
then `magvar.get_mag_decl(lat, long)` once per verification point. SPEC §6.3
carries the result on each point and the mission date on the record.

**Degrees in, radians out**, and both units are ED's own. At
`MissionEditor\modules\me_statusbar.lua:376` the same `lat, long` pair is
wrapped in `toRadians` for every display path and passed **unwrapped** to
`get_mag_decl`, whose result is wrapped in `toDegrees` before formatting.
Measured: `get_mag_decl(45.0, 34.0)` returns `0.115879685`, which is 6.6394
degrees, within a few tenths of the published real-world value for that point
and epoch.

**Seed explicitly and do not depend on the default.** Called with no `init` of
its own, `magvar.get_mag_decl` already returned the value `init(6, 2016)`
produces on a mission dated June 2016, to every digit — DCS seeds it from the
mission date at load. The seed still matters: `init(6, 1990)` moved the same
point from 6.6394 to 4.8732 degrees, so a stale epoch gives a quietly wrong
answer.

**Guard the call and report the reason.** SPEC §6.3's `DeclinationStatus`
carries the four outcomes: present, refused by operator policy, unavailable
under Route B, and a guarded failure. The hook driver sets one on every record
and logs a refusal once per epoch rather than once per point.

**Declination varies by 3.84 degrees across Caucasus** (measured,
2.9.29.27278, June 2016 epoch, at the four bounds corners of Section 10.2):
4.957 degrees at the south-west corner, 8.797 at the north-east. That is an
order of magnitude larger than the heading precision a consumer will quote,
which is what makes the field per point rather than one scalar per theatre.
The variation is not uniform — a 7-degree step of longitude at 45N moves the
answer 0.717 degrees near the west of the map and about half that near the
east — so a consumer must read the value at the point it cares about rather
than interpolate between two.

The epoch drift is a separate and larger effect: the same point moved 4.8732
to 6.6394 degrees between a 1990 mission and a 2016 one, which is why the
record carries the mission date and why the seed is explicit.

**`magvar` is the sim's own magnetic model, and this is measured**
(2.9.29.27278, Caucasus, June 2016 epoch). The Export state publishes own-ship
true heading and magnetic yaw, whose difference is the declination the sim
itself applies. At one instant, at 45.0833416N 38.9293983E, that difference was
`0.12568947276` rad and `magvar.get_mag_decl` at the same coordinates returned
`0.1256892979` rad — **0.036 arcseconds apart**, one part in 700,000. A
consumer computing a magnetic heading from the published declination gets what
the sim is using.

This does not close the cockpit half: an aircraft's needle is driven by its own
avionics, and a slaved gyro drifts (**[PROBE-A5]**). It also does not make
Export a sampling route — `Export.LoGetMagneticYaw` still takes no argument, so
it answers only for own-ship and cannot answer "declination at point P". It
validates the gui-state route rather than replacing it.

**Whether an enforcing build accepts `"gui"` as a `net.dostring_in` target
remains open** (**[PROBE-A2]**). SPEC §5.4 owns the operator-policy
half of that route, including the two `autoexec.cfg` keys and what `doctor`
prints.

## 11. Worked example

A consumer links a Discord account before a player takes a slot.

1. The operator adds one token with `players`, `moderate` and `command`.
2. The consumer connects.
3. A player connects. The hook driver emits `PlayerConnected` with a ucid.
4. The consumer finds them unlinked. It sends `SendChatTo` with a one-time
   code.
5. The player picks a slot. The hook driver emits `PlayerChangedSlot`.
6. The consumer sends `ForceSlot(ucid, 0, "")`. Side 0 is spectators. An
   empty slot string means no slot.
7. The player redeems the code in Discord. The code never enters DCS chat.
8. The player picks a slot again. The consumer does nothing.

**Step 6 is a race.** The bounce is a full round trip against a human
pressing a button. The measured script-side floor sits three orders of
magnitude inside the human's 2.1 s; the full round trip is unmeasured
(**[PROBE-19]**).

**If the bounce loses, the consumer bounces again on `PlayerStarted`.** The
player spawns and is then returned to spectator. That is worse for the
player. It still works.

**Step 7's direction matters.** The code goes out in game and comes back in
Discord. Both legs are private. The opposite direction puts the code on a
public channel where another player can claim it.

Nothing is cached. A dead consumer means new players are not challenged.
That is SPEC §11's first row working as designed.

---

## 12. Open questions

PLAN §3 holds the method for this document's own four. The four inherited below carry their own.

| # | Question | Why it matters |
|---|---|---|
| **PROBE-17** | Does `onChatMessage` fire in the user-hook chain on a dedicated server, and what does its third argument carry? | Decides which chat source wins deduplication and fixes `ChatTarget`'s mapping (Section 5). The shipped reference documents two arguments; ED's own GUI handler takes three. No shipped hook registers any GUI-group callback. |
| **PROBE-18** | `net.banlist_add_by_ucid`: is `period` in seconds, what encodes a permanent ban, does the ban survive a restart, and does DCS refuse the connection without running Lua? | Blocking for `BanPlayer` (Section 6). The function appears nowhere in ED's reference; the documented sibling counts `period` in seconds. |
| **PROBE-19** | How long from `PlayerChangedSlot` to a `ForceSlot` landing, against a player's time to press Fly? | Section 2's whole rule rests on a correction arriving in time. Section 11 names the fallback. |
| **PROBE-20** | Is `net.send_chat_to` invisible to players other than its target? | Section 11's one-time code depends on it. Section 6's rejection of the broadcast ids is the mitigation that does not. |

**Four probes arrived with the coordinate work and the server-control rows.**
They keep the identifiers the sim driver retired, which does not reuse them.

| # | Question | What it needs |
|---|---|---|
| **PROBE-A2** | Does `"gui"` work as a `net.dostring_in` target under a build that enforces the gate? | A DCS build that enforces it. Enforcement was reverted before this project existed, so on 2.9.29.27278 the call is simply not refused — measured. Nothing observable distinguishes "the value is accepted" from "nothing is checking". |
| **PROBE-A5** | Does an aircraft's own compass show what `magvar` says? | A cockpit reading. The sim-model half is settled: `magvar` matches the declination the sim applies to own-ship magnetic yaw, to 0.036 arcseconds (Section 10.3). What remains is instrument error — a slaved gyro drifts and each airframe models its own compass. Comparison target, Caucasus, June 2016 epoch: **7.2015 degrees east at 45.0833N 38.9294E**, **6.6394 degrees east at 45.0N 34.0E**. |
| **PROBE-A6** | Does `onPlayerConnect` fire for a listen server's local player, and does its second argument reach a write-directory hook? | A second player joining a hosted session. Section 3.2 carries what is known. |
| **PROBE-A11** | Does `net.load_mission` reload on a dedicated server, and what does its return value mean? | Narrowed. The argument is a mission path: measured working in a hosted multiplayer session, returning `0`, and measured a silent no-op on a single-player host (Section 6). What remains is the dedicated server and the meaning of the return code. |

**Two of Section 10's questions were settled on a live mission** and their
answers are folded into that section rather than left here: whether
`terrain.GetTerrainConfig` answers in the hook state and with which axis order,
and how much magnetic declination varies across one terrain.
`proposals\caucasus-live-verification.py` carries the measurements. A retired
number is not reused.

---

## 13. Test method

SPEC §17 gives the method for the bridge's own layers, and the rules below it
bind here too. Every row here needs the sim, so none of it runs in CI.

| Layer | Method | Host |
|---|---|---|
| Moderation identity | A command naming a disconnected ucid produces `NO_MATCH`. One matching two connected players produces `AMBIGUOUS` and acts on nobody. | Windows + DCS |
| Moderation harvest | A `Try` refused by a later script produces no record. `PlayerChatted` is excluded: it has no sibling and reports the attempt. | Windows + DCS |
| Moderation return rule | The built-ins return no value from any `onPlayerTry*` callback. A second hook script that vetoes still runs, tested per callback. | Windows + DCS |
| Moderation chat | A line the set observed reaches every player unaltered. | Windows + DCS |
| Moderation bounce | A `ForceSlot` that re-fires `onPlayerChangeSlot` is suppressed once, counted, and never loops. A second bounce inside `bounce_min_interval_ms` is refused. | Windows + DCS |
| Moderation late query | `GetSlotList` re-emits the current `SlotList` set and is acknowledged once. `GetBanlist` fans out at most `banlist_max_entries` entries per record. `ExitProcess` from a token without `process` is refused with `Rejected`. | Windows + DCS |
| Moderation dynamic slots | A dynamic join's doubled `onPlayerChangeSlot` yields records a consumer applies idempotently. A manual move to spectators yields none, and `GetRoster` recovers the true view. | Windows + DCS |
| Mission briefing | `GetMissionBriefing` returns the author's text, and a briefing over `mission_briefing_max_bytes` comes back clipped on a UTF-8 boundary with `truncated` set. | Windows + DCS |
| Server state | `GetServerStatus` answers the clock and the pause state and carries no mission filename. The mission name and the deployment pair arrive on `EpochOpened` instead, and a single-player host reports `is_server` true with `is_multiplayer` false. | Windows + DCS |
| Coalition | A coalition change emits one `PlayerChangedCoalition` carrying the ucid and the target side. `onPlayerTryChangeCoalition` is not registered, and ED's blocker still runs. | Windows + DCS |
| Projection derivation | A stock PROJ binding fed the published string reproduces the published verification points to millimetres, on every installed terrain. | Windows + DCS |
| Verification points | The four published points are the corners of `SW_bound` and `NE_bound`, and none of them is a point the parameters were derived from. | Windows + DCS |
| Off-family terrain | A third point missing by more than 1 mm yields verification points, no `projection`, no PROJ string, and one logged residual. | Windows + DCS |
| Declination | `CoordinateCalibration` carries a declination per verification point tracking a published model, and `declination_status` names the reason on every record that omits one. | Windows + DCS |
