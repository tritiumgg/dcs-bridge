# DCS-Bridge — Project plan

Build order, what ships, how each probe is run, and the conventions the three
specifications are written under.

This document changes weekly. The specifications it points at do not. `SPEC §N`
is the bridge specification, `SIM §N` the sim driver built-ins, `HOOK §N` the
hook driver built-ins. **Nothing in those three documents depends on this one
for a requirement.**

---

## 1. Task breakdown

**A task ID is a name, not a position.** The row order inside a phase is the
build order; the ID identifies the task across revisions. Phases are ordered so
that every done-when is satisfiable with what exists when it runs.

**The CLI is built in lock-step with the bridge, not after it.** Most of the
broker phase's done-whens name a consumer — a `seq` gap somebody observes, a
`Pong` somebody receives, a late join somebody makes. That consumer is `dcsb`,
so it is built as `dcsb` rather than as a throwaway that gets rewritten later.
Section 1.1 gives the CLI's arc across the phases.

### 1.1 CLI increments

| Verb | Lands in | Because |
|---|---|---|
| `tail` | 2.C1 | The first done-when that needs a record observed off the wire |
| `ping` | 2.C2 | `Pong` exists and liveness has to be checked from outside |
| `schema` | 2.C3 | `GetSchema` exists and its bytes must be compared to `schema.pb` |
| `send` | 2.C4 | Inbound rings exist and routing has to be exercised |
| `record`, `replay` | 2.C5 | The frame format is settled; captures let later phases test with no DCS |
| `mock` | 2.C6 | The schema is settled; a consumer can now be written before any capture |
| `stats` | 5.C1 | `shim.stats` publishes counters worth reading |
| `doctor` | 4.C1, 5.C2, 8.C1, 9.C1 | It can only check what exists, so it grows in four increments |

**Every remaining probe runs inside the phase that can answer it.** PROBE-3
needs the broker's put calls and runs at 2.6. PROBE-14 needs both routes and
runs at 6.9. PROBE-7 needs a real production rate and runs at 9.7. PROBE-9 and
PROBE-10 need a shipped build and a real server, and run in Phases 9 and 10.
Nothing is measured before Phase 1 any more: the two questions that had to be
settled before a line was written are settled.

### Phase 1 — Repository, CI, release and schema

**A walking skeleton, built first so that everything after it ships.** The
crates here are stubs; the point is that a tag produces a downloadable build
before any behaviour exists, so every later phase flows through a pipeline
already known to work rather than through one bolted on at the end. Section 4
states which hosts build what.

| ID | Task | Done when |
|---|---|---|
| 1.1 | Cargo workspace with three crates — module, CLI, generator — each a stub that builds. Licence, README, `rustfmt` and `clippy` configuration. | `cargo build` succeeds on Linux, macOS and Windows. |
| 1.2 | GitHub Actions: build, test, `rustfmt --check` and `clippy -D warnings` on all three hosts against host-native targets. | A pull request is gated on all three. |
| 1.3 | A checked-in `.def` naming `lua.dll` and the Lua symbols the broker uses, with the import library generated at build time (SPEC §5.1.1). | The broker stub links against DCS's Lua from a host with no DCS installed. |
| 1.4 | Windows cross-build in CI: `x86_64-pc-windows-msvc` through `cargo-xwin`, the only Windows target (ADR 0003). The broker is never built with `panic = "abort"` (SPEC §14.2). | `lua-dcsbridge.dll` and `dcsb.exe` are produced from a Linux runner. |
| 1.5 | Release workflow: tag-triggered, builds the write-directory zip, attaches the broker, the CLI and the zip, and publishes checksums. A prerelease channel carries development builds. | A tag produces a downloadable build with no manual step. |
| 1.6 | `.proto` importing `google/protobuf/any.proto` and `google/protobuf/descriptor.proto`: the `Envelope` with its `Any` payload, `RecordClass`, `Target`, `Capability`, `RejectedReason`, the four message options, and one `DURABLE` message. Build `schema.pb` with `buf`. | `buf lint` passes and `schema.pb` exists for Phase 2 to hash and serve. |
| 1.7 | `buf lint` and `buf breaking` in CI against the previous release, plus the SPEC §8.4 ownership check: only the SPEC §1.2 records may live in `dcs.bridge`. | An incompatible change fails the build. A message added to `dcs.bridge` from outside that set fails it too. |

### Phase 2 — The broker, and the CLI beside it

| ID | Task | Done when |
|---|---|---|
| 2.1 | Host-native build of the broker crate for testing: the same source as a `.so` or `.dylib`, loaded by a stock Lua 5.1. | SPEC §17's *Any (native module)* rows run in CI with no DCS present. |
| 2.2 | Export `luaopen_dcsbridge`. Load task 1.4's cross-built DLL with `package.loadlib` by explicit path. Rings, sockets, threads and the three registration maps are process-global, not per-state. | The load banner appears in `dcs.log`. Loading from both states gives two Lua tables over one set of maps. |
| 2.3 | SPSC ring buffer, fixed size, drop-oldest with a counter. | A unit test fills, drains, and overflows it. |
| 2.4 | One producer ring; writer thread fans out to per-connection queues. | Adding a consumer does not change logic-thread cost per record. |
| 2.5 | Put calls emitting protobuf tags and values. | A stock library decodes the output, including a non-minimal length varint. |
| 2.6 | Run PROBE-3, put-call crossing cost, per Section 3.1. | The one-call-per-field API shape is confirmed, or a batched put form is scheduled. |
| 2.7 | `Envelope` wrapping with an `Any` payload, length-prefixed framing, per-connection `seq`. | A forced drop shows as a gap. A capture names its record types with no schema loaded. |
| 2.C1 | CLI `dcsb`: the binary, a connection, and `tail`. | 2.7's forced drop is observed as a `seq` gap by `dcsb tail`. |
| 2.8 | `begin_to` and per-connection addressing; `poll` returns the connection id; connection ids unique for the process and never reused. A `begin_to` on a topic the schema did not mark a reply or an acknowledgement is refused and counted in `misaddressed_total`. | Two `dcsb tail` sessions show a `begin_to` record reaching one and a `begin` record reaching both. A hand-written `begin_to` on a fan-out topic is refused. |
| 2.9 | Handshake, then auth, then five messages the broker answers itself on the reader thread: `Ping`, `Auth`, `GetSchema`, `SeqAck`, `SetEnabled`. `SetTopicFilter` and `GetTopics` join them at 2.20. | `Pong` answers during a mission-load blackout. None of the five reaches a ring. |
| 2.C2 | CLI `ping`. | It reports `dcs_alive`, `dcs_last_heard_ms` and `bridge_enabled`, and still answers while the logic thread is stalled. |
| 2.10 | `shim.schema`: opaque bytes accepted once after the first `configure`, hashed, served by `GetSchema`; refused twice; an error before the hand-off, and the handshake omits the hash until it. `GetSchema` requires authentication. | `GetSchema` returns bytes identical to `schema.pb`. An unauthenticated `GetSchema` is refused. |
| 2.C3 | CLI `schema`. | `dcsb schema` returns bytes identical to the deployed `schema.pb`, which is how 2.10 is checked. |
| 2.11 | `shim.tick`: mission time published every call, the heartbeat atomic stamped at most once per `heartbeat_interval_ms`, the throttle inside the broker. `shim.epoch(id)` and `shim.epoch(nil)` stamp and clear the epoch. | `Pong` carries `dcs_alive` and `dcs_last_heard_ms`. Killing the logic thread flips it; a mission load does not. A record emitted outside an epoch omits `epoch` and `mission_time`. |
| 2.12 | Reader thread; the payload type URL read out of every inbound `Any` under `max_type_url_bytes`; two inbound rings routed by the registered route map, drop-newest. A topic in no route map is refused with `Rejected` reason `UNKNOWN_TOPIC` and counted in `unrouted_topic_total`, never defaulted to the sim driver. | `poll(target)` returns a sent record from the right ring and only that ring. An unregistered topic is refused and delivered nowhere. |
| 2.C4 | CLI `send`. | A record sent by `dcsb send` arrives on the right ring, which is how 2.12 is checked. |
| 2.13 | `Rejected` on the reader thread, carrying the inbound envelope's `seq`, the topic id and one of the four `RejectedReason` members; `rejected_max_per_sec` cap per connection. | A refused command is answered once, and a flood of them is not. A frame whose header does not parse drops the connection instead. |
| 2.14 | Outbound capability filter at fan-out, before `seq`; `records_filtered_total`. Point-to-point records are addressed rather than fanned out, so the filter does not touch a reply, an acknowledgement or a `Rejected`. | A filtered consumer sees no `seq` gap. `records_dropped_total` does not move. |
| 2.15 | `shim.configure`: allocate on the first call, apply live keys on later ones as one atomic swap, validate and reject whole, count unknown keys. It answers with the broker's interface version. Cross-key invariants are checked against effective values, and a changed restart-tier key counts in `config_keys_pending_restart`. | Rings size from config. A later call changes a rate limit and refuses a ring size. A call before it errors rather than defaulting. A version mismatch takes SPEC §11's broker-failure path. |
| 2.16 | `shim.classes`, `shim.routes`, `shim.caps` registration: additive over disjoint sets, idempotent on identical rows, refused on conflict, process-global. `routes` carries inbound topics only. A topic missing a class or a capability is refused rather than defaulted, counted in `partial_registration_total`. | `LOSSY` drops before `DURABLE` under pressure and `LIFECYCLE` survives. A second registrar merges; a conflicting row is refused whole. An outbound-only topic with a class and a capability and no route registers cleanly. |
| 2.17 | `LIFECYCLE` retention: latest per topic, slots allocated at `shim.classes` under `max_lifecycle_topics`, replayed after auth in emit order before live traffic, through the same capability filter. `lifecycle_replayed_total`. | An `dcsb tail` started mid-epoch receives `EpochOpened` before any live record. A `shim.classes` call above the cap is refused whole. |
| 2.18 | The outbound drop rule, both halves: evict the oldest non-`LIFECYCLE` record, and refuse the newest non-`LIFECYCLE` record once free space reaches `ring_out_lifecycle_reserve`. A ring holding only `LIFECYCLE` drops the connection and counts `lifecycle_disconnects_total`. | An `EpochClosed` survives a `LOSSY` flood. A consumer far enough behind is disconnected rather than losing a boundary record. |
| 2.19 | `SeqAck` accepted and counted, with no spool behind it and no per-connection tracking: nothing reads an acknowledged `seq` until the spool ships (SPEC §11). | The wire carries it and the broker accepts it without error. |
| 2.20 | `SetTopicFilter` and `GetTopics` on the reader thread: `ALL` as the default, replace rather than accumulate, `LIFECYCLE` always admitted, the four refused shapes with `ok` false and a `refusal` reason, `topic_filter_max_topics`, and the filter published to the writer thread by pointer swap. Filtering runs at fan-out before `seq` and counts in `records_filtered_total`. | A connection naming one topic under `ONLY` receives that topic and every `LIFECYCLE` topic and no other. `records_dropped_total` does not move and the consumer sees no `seq` gap. `GetTopics` lists every topic the token's capability set covers and no topic outside it. |
| 2.C5 | CLI `record` and `replay`. | A captured session replays to a consumer with no DCS running. |
| 2.C6 | CLI `mock` — synthetic traffic at a configurable rate. | A consumer can be written before any capture exists, and 9.8 has its load generator. |

**Task 2.2 lands as a sequence of three**, named here because this is the last
moment the boundaries are free to move:

| Branch | What it holds | Reviewable against |
|---|---|---|
| `task/2.2-1-process-global-state` | The process-global owner in `crates/broker`: three empty registration maps, unallocated slots for the rings, the listener and the threads, and the record deciding what a second open does. No FFI. | SPEC §5.1.1's process-global paragraph, and SPEC §5.1's "`configure` comes first" |
| `task/2.2-2-lua-surface` | `luaopen_dcsbridge` over that owner, each call returning its own table. | SPEC §5.1.1's load recipe, ADR 0006 |
| `task/2.2-3-load-test` | Two opens in one process read one counter through two tables; the staged DLL is checked for the export `package.loadlib` asks for. | SPEC §17's rule that a harness run exercising nothing is a failure |

Neither the merge rules nor any allocation belongs to 2.2. Registration
semantics are 2.16, the rings and the listener are allocated at the first
`shim.configure` at 2.15, and the load banner is 4.1's.

**Task 2.3 lands as a sequence of two**, for the same reason:

| Branch | What it holds | Reviewable against |
|---|---|---|
| `task/2.3-1-ring` | The ring: fixed size, one producer, one consumer, drop-oldest with a counter, and the per-slot stamp that lets the producer evict a record its consumer may be reading. Single-threaded tests, and the record that argues the departure. | The done-when — a unit test fills, drains and overflows it — and ADR 0008 |
| `task/2.3-2-ring-verification` | A two-thread accounting test, Loom over a shrunk model behind a `cfg` shim, and the CI jobs that run Loom and Miri. No change in behavior. | No schedule loses, duplicates, reorders or double-drops a record |

The class-aware drop rule is not 2.3's. Evicting the oldest non-`LIFECYCLE`
record and the `ring_out_lifecycle_reserve` watermark are both 2.18, and the
sizes come from config at 2.15.

**Task 2.4 lands as a sequence of two**, for the same reason:

| Branch | What it holds | Reviewable against |
|---|---|---|
| `task/2.4-1-fanout` | The writer thread over one commit ring, fanning each record into a ring per connection; attach and detach at runtime; the flag the writer parks on and the logic thread wakes it through. Thread tests, a Loom model of the wake, and the record deciding who drains a connection's ring. | SPEC §5.2's "The logic thread writes one ring, not N", and ADR 0011 |
| `task/2.4-2-fanout-cost` | A release-mode benchmark of logic-thread cost per commit against zero to eight consumers, runnable with no DCS, and the CI filters widened so Miri reads the new module. No change in behavior. | The done-when: adding a consumer does not move the per-record figure |

`seq`, the capability filter, framing and the socket itself are 2.7 and 2.14.
The benchmark reports a figure; whether it satisfies the done-when is the
maintainer's reading, because a runner's timings cannot carry a cost claim.

**Task 2.5 lands as a sequence of three.** Each branch is sized to about 400
changed lines with its tests counted, because 2.3 and 2.4 were split by
concept and each half still landed at twice that:

| Branch | What it holds | Reviewable against |
|---|---|---|
| `task/2.5-1-encoder` | The encoder: one buffer allocated at construction, the tag and varint writers, the four scalar puts, `begin` and `commit`, and the errors that poison a record. Tests decoding every scalar wire form through a stock library. | The done-when's first half: a stock library decodes the output |
| `task/2.5-2-nested-messages` | `message` and `end_message` over a preallocated stack of open messages, each length reserved in place at a fixed width and back-patched at close, and the depth cap. Tests for nesting, repetition and the padded length, and the record that argues the departure from a scratch buffer. | The done-when's second half, a non-minimal length varint, and ADR 0012 |
| `task/2.5-3-put-calls` | The Lua binding of `begin`, the six puts and `commit`, one encoder per Lua state, a second harness script under `tests/lua`, and `tools/luatest.sh` running every script there. | SPEC §5.1's put surface, reachable from a stock Lua 5.1 with no DCS present |

`commit` returns the body to Lua until 2.7 queues it, and the buffer is
allocated at open until 2.15 allocates it at the first `configure`. Topic
checks against the registration maps are 2.16.

**Task 2.7 lands as a sequence of five**, sized the same way. The park-flag
refactor takes its own branch because it changes no behavior and the transport
should show only what the transport adds:

| Branch | What it holds | Reviewable against |
|---|---|---|
| `task/2.7-1-envelope` | `begin(topic)` writes the payload field, the `Any` type URL and the `Any` value field ahead of the record, each length a padded gap filled at `commit`. Tests decode the tail through a stock `Any`. | SPEC §5.2's `Envelope` and `Any`, and ADR 0012 |
| `task/2.7-2-seq` | The writer thread numbers each connection's records from one, before the push. A stalled ring's survivors show the gap. | SPEC §5.2 "Sequence numbers" |
| `task/2.7-3-park-flag` | The writer's park flag and waker factored out for any drainer; `attach_with` takes a waker. A second Loom model. No change in behavior. | ADR 0011 |
| `task/2.7-4-transport` | The listener, a thread per connection, and the frame: length, `seq`, then the shared tail. Tests read frames off loopback. | SPEC §5.2 "Frame format", and the done-when's capture |
| `task/2.7-5-commit-queues` | The bridge starts the outbound path at open; Lua `commit` pushes and returns a boolean. ADR 0014, this table, `STATE.md`. | SPEC §5.1's `commit`, and ADR 0014 |

The listener binds at open with the specification's defaults until 2.15 moves
it to `configure`. The handshake is 2.9; nothing here authenticates.

### Phase 3 — Generator

| ID | Task | Done when |
|---|---|---|
| 3.1 | `protoc-gen-dcsbridge-lua`: emitters, send wrappers, the topics table, decoders, and the class, route and capability tables, split by target into `SimDriver.gen.lua`, whose tables register into `DCSBridge.code`, and `HookDriver.gen.lua`, each carrying the schema hash. The plugin advertises `FEATURE_PROTO3_OPTIONAL`. | Golden files show all seven, split by target. Sim-driver-side execution is verified at 5.9. |
| 3.2 | `Target` and `Capability` options; the generator refuses a message with no capability. | A message without `required_capability` fails generation. |
| 3.3 | `send` and `send_to` wrappers, the `topics` table, LuaLS annotations, and split output: shipped messages into the two bridge-tree `.gen` files, an adopter's into a `*.gen.lua` carrying its own schema-slice hash. Names are the schema's, verbatim. | `message RocketImpact` yields `send.RocketImpact` and `topics.RocketImpact`. A reply or acknowledgement message yields a `send_to` member taking a connection id first. An adopter's slice registers additively beside the shipped set. |
| 3.4 | Golden-file tests for the plugin. | A template edit shows as a reviewable diff. |
| 3.5 | Typed request and reply wrappers for `reply_to` messages, resolving on the typed reply, on a `CommandAck` matching `request_id`, or on a `Rejected` echoing the request's `seq`. | A request reads as a suspend function. All three answers resolve it, and two of them raise. |
| 3.6 | Decoder depth and repetition bounds baked in as generator constants derived from the schema's own shape plus margin (SPEC §14.2). | No configuration can set a bound below what the shipped schema needs. |

### Phase 4 — Hook driver and sim driver bootstrap

| ID | Task | Done when |
|---|---|---|
| 4.1 | Thin hook driver loader, `ENABLED` flag, load banner. `Logs\DCSBridge\DCSBridge.log` rotated at `log_max_bytes` under `log_retention_days`, at `log_level`. | The banner appears in both logs. The bridge's own log stops growing at its cap. |
| 4.2 | Config with the same defaults in code. Hook driver reads `Config\DCSBridge.lua` and calls `shim.configure`. It is the only reader of that file on Route A, and the only reader of a hook-driver-owned key on either route. | It runs with the config absent. Every broker-owned value in SPEC §13.1 arrives through one call. |
| 4.3 | Hook driver reads `schema.pb` after the first `configure` and calls `shim.schema`; `schema_sha256` from `HookDriver.gen.lua` rides `configure`. Interface versions compared at the same call. | A consumer's handshake hash equals the SHA-256 of the deployed `schema.pb`. A hook driver and a module from different builds disable rather than run. |
| 4.4 | `ReloadConfig` command with the `reload` capability: compile-before-apply, whole-file validation, rollback to the previous table on a raise. | A live key changes with no restart. A ring size logs a pending restart and does not resize. A config that raises leaves the running one in force. |
| 4.5 | Register DCS callbacks, one `pcall` per body, a raise logged once per callback per epoch and suppressed after. One accessor table in the hook driver for every `DCS.*` call, built here because SPEC §4.4 makes it a code rule from the first one. | A deliberate error is logged and the session survives. A per-frame raise does not fill `dcs.log`. |
| 4.6 | Kill switch: `enabled` at load, by `ReloadConfig`, or by `SetEnabled`, which the broker handles itself. The hook driver stops injection, drain, ferrying and eval, and keeps calling `shim.tick`. | The bridge disables with no restart. `Pong` reports `bridge_enabled` false and `dcs_alive` true. A re-enable arrives while dispatch is down. |
| 4.7 | Sim driver injection from disk, escaped with `%q`, carrying the epoch id, the mission name, the sim-driver-tier settings, the `options` table and the enabled extension file list. The loader measures the assembled chunk and refuses above `bridge_return_max_bytes`. `GRAMMAR_VERSION` compared before injection. | The sim driver sets a global the hook driver reads back. An oversize chunk is refused and logged rather than truncated. |
| 4.8 | `"OK\|"` prefix; nil, `"Invalid state name"` and policy refusal separated. A `failed:` body told apart from a missing prefix. | The four cases log differently. |
| 4.9 | Policy refusal disables the hook driver and logs the union `autoexec.cfg`. | No retry loop. The operator gets a paste-ready fix. |
| 4.10 | Route B: `dofile` bootstrap from `MissionScripting.lua` before the sanitisation block, capturing `package.loadlib`. The sim driver reads the sim-driver-tier keys itself at bootstrap. | The same sim driver file runs with `net.dostring_in` unavailable. |
| 4.11 | Hook-driver-side extension chain per SPEC §6.10: `HookDriver.gen.lua`, then `HookDriver.builtin.lua`, then `hookdriver.d\` in name order, with `hook_driver_disabled_files` and per-file containment counted in `hook_driver_files_failed_total`. | A raising file in `hookdriver.d\` is skipped and counted, and the rest load. An absent directory is silent. |
| 4.12 | Hook driver dispatch loop per SPEC §6.4: hook driver ring poll, five callbacks, `hook_driver_dispatch_max_commands` per invocation, `hook_driver_dispatch_deferred_total`, nothing between `onMissionLoadBegin` and `onMissionLoadEnd`. Loads `HookDriver.gen.lua` and registers its tables. | A hook-driver-targeted command dispatches at the menu with no mission loaded. A burst above the cap defers rather than extending the callback. |
| 4.C1 | CLI `doctor`, first increment: install placement, hook driver load, port, load banner, and both interface versions. | A broken install is diagnosed in one command before any sim driver exists. |
| 4.13 | Run everything under the stub harness. | Exit 0. No unexercised callback. |

### Phase 5 — The sim driver runtime

| ID | Task | Done when |
|---|---|---|
| 5.1 | One global `DCSBridge`, its four slots built field by field, stamped `__dcsbridge = STATE_VERSION`. A table without the stamp stops the load; a table with it is epoch leftovers to release and rebuild. Sandbox level read and reported. | An operator can see which regime they are in. A foreign `_G.DCSBridge` stops the load instead of merging. A second mission load adopts its own leftovers instead of reporting a collision. |
| 5.2 | Binding probe and blacklist per SPEC §4.2, run unconditionally. `unsafe_bindings_enabled` gates the call site instead, so it applies on the next dispatch after a reload. | A known-bad binding is refused. |
| 5.3 | Bounded work model, seven stages, with per-stage caps and deferral counters. `subscription_max_evals` and `spot_max_updates` are present as structural caps and never bind. | A cap hit every frame is visible in `stage_deferred_total`. |
| 5.4 | `sim_driver_buffer_max_records` with drop-oldest and a counter. | A forced 3-second frame stall bounds memory instead of growing it. |
| 5.5 | Per-handler `pcall`, disable-for-epoch, sim driver disabled after `handler_failures_per_epoch` failures. | A raising handler stops without ending the session. The bridge keeps running. |
| 5.6 | Event drain emitted before any resync slice. | SPEC §6.8's ordering holds under test. |
| 5.7 | One permanent handler table in `DCSBridge.resources`, registered once. `onEvent` wrapped in `pcall` with the SPEC §4.1 reason in a comment. | A reload causes no duplicate dispatch. A deliberate raise does not abort dispatch for other handlers. |
| 5.8 | Sim-driver-side load chain and registration surface per SPEC §6.10: the four-step chain, `on`/`command`/`send`/`send_to`/`topics`/`off`/`replace`/`wrap`, `DCSBridge.code.mission`, end-of-file registration commit, refuse-whole on a duplicate `command` topic or a missing key, per-file containment with `sim_driver_files_failed_total`, and the merged `options` table. | The surface exists with `SimDriver.builtin.lua` deleted. A file in `simdriver.d\` wraps a `builtin.*` key. A file naming a missing key is refused whole and the rest load. A file scoped on `mission.name` returns early under Route A and guards for nil under Route B, and a mission flown from the Mission Editor is matched on filename rather than on the `tempMission` name DCS reports for it. |
| 5.9 | Generated code executing in the sim driver — the sim-driver-side half of 3.1. `DCSBridge.shim` is the seam; the sim driver reports `sim_driver_direct_broker`. | The sim driver calls a generated `send` wrapper and decodes a command with a generated decoder. A hook-driver-targeted message's artifacts land hook-driver-side. An emitter called from a handler is a defect the review catches. |
| 5.10 | `shim.stats()` for the broker-owned counters, plus the sim-driver-owned and hook-driver-owned metrics published alongside them. Flat keys per SPEC §5.1, with composite keys rather than subtables. | Every metric named in SPEC §12 is readable, and every broker-owned one comes from `stats`. |
| 5.C1 | CLI `stats`. | Every metric named in SPEC §12 is readable through `dcsb stats`, which is how 5.10 is checked. |
| 5.C2 | CLI `doctor`, second increment: sim driver load and route, the Route B `dofile` line, the SPEC §6.10 extension load order with each file's status, shipped-file hashes against the release, and any topic missing a class or a capability. | An edited `SimDriver.builtin.lua` is reported as lost-on-update before an update. |

### Phase 6 — Lifecycle

| ID | Task | Done when |
|---|---|---|
| 6.1 | The hook driver owns both boundaries: it allocates the epoch id at `onMissionLoadEnd`, emits `EpochOpened` there before injecting, publishes the id with `shim.epoch`, passes it to the sim driver, and emits `EpochClosed` at `onSimulationStop`. `EpochClosed` reads no theatre: `DCS.getMissionTheatre()` returns nil at `onSimulationStop` while the mission name and filename still read. | Both boundaries arrive with `SimDriver.lua` absent. The boundary survives a sim driver that never got a frame. `EpochClosed` carries no field the teardown cannot supply. |
| 6.2 | `EpochOpened` carries the epoch id, the mission-start wall-clock time, the mission time, the terrain name, the mission name and the `is_server` / `is_multiplayer` pair, each `DCS.*` call made alone and guarded, emitted at `onMissionLoadEnd`. | A consumer holds an epoch anchor and a time pair on every configuration. |
| 6.3 | `CoordinateCalibration`, class `LIFECYCLE`, emitted by the hook driver from `terrain.convertMetersToLatLon`, never by the sim driver. HOOK §10 fixes the derivation, the verification points and the declination route. | A consumer converts DCS coordinates with no per-record traffic and with no sim driver loaded. A known airfield converts to its published latitude and longitude within tolerance, and the values match `coord.LOtoLL` read from the mission-scripting state. |
| 6.4 | All twelve `LIFECYCLE` topics, eight of them the hook driver's. `CallbackHz` is not among them; ADR 0010 makes it `LOSSY`. | An `dcsb tail` logs the full sequence. The four sim-driver-emitted topics are the only ones missing with no sim driver. |
| 6.5 | `MissionLoadBegan` raising the liveness threshold to `dcs_alive_threshold_loading_ms`, `MissionLoaded` lowering it, and the consumer load timeout. | A minute of silence during a load is not reported as a fault. |
| 6.6 | Pause from `DCS.getPause()` at `pause_poll_interval_ms`, alone and guarded. | A mission that starts paused reports paused. A resume with no preceding pause is handled. |
| 6.7 | `CallbackHz` computed against `DCS.getRealTime()`. | It reads close to the running rate while paused, which is what SPEC §9.3 measured. |
| 6.8 | Consumer discards frames from a closed epoch; a frame with no epoch field is never discarded by that rule. | A stale frame replayed by `dcsb replay` is rejected. A `MissionLoadBegan` with no epoch is kept. |
| 6.9 | Run PROBE-14: does a hook-driver-targeted record reach the hook driver ring under both injection routes. | The hook driver dispatches it and the sim driver ring never sees it, on both routes. |
| 6.10 | `mission_sim_driver_dirs`: the mission's own `dcsbridge\` directory enumerated in name order with its `*.gen.lua` first, and the one not-loaded log line when the key is off. | A mission's files load for that mission and not for the next. With the key off, a mission carrying the directory logs one line naming what was skipped. Under Route B nothing loads. |
| 6.11 | `Resync` with the SPEC §6.8 consistency rule and the `ResyncBegan`/`ResyncEnded` brackets. A sim driver implementing no resync answers with `CommandAck` outcome `REFUSED`. | A consumer joining mid-mission gets a complete picture. Every transition record is idempotent at the consumer. |

### Phase 7 — Commands

| ID | Task | Done when |
|---|---|---|
| 7.1 | Hook driver polls the broker and ferries commands to the sim driver under both caps. | A burst never extends a frame. |
| 7.2 | Sim driver decodes and dispatches by topic id. Dispatch is SPEC §6.4 stage 2, in the sim driver, not the hook driver. A handler receives `(conn_id, msg)` and returns nothing. | One command executes. |
| 7.3 | `idempotency_key` on mutating commands, checked against a recent-key set bounded at `recent_idempotency_keys`. `request_id` on reads is not checked. | A duplicate executes once and is acknowledged with outcome `DUPLICATE`. |
| 7.4 | Unknown-topic path logs, counts `unknown_topic_total`, and continues. | An unknown command does not break the stream, and the counter tells it apart from `unrouted_topic_total`. |
| 7.5 | Exactly one point-to-point record per command that reaches Lua, sent with `send_to` per SPEC §8.5.3: a typed reply on a successful read, a `CommandAck` otherwise, `detail` truncated at `command_ack_detail_max_bytes`. | The consumer learns the result of every command that reaches Lua. A successful read produces its reply and no acknowledgement. |
| 7.6 | Request and reply helper: pending map, `request_timeout_ms`, epoch discard. `request_id` allocated from a per-connection counter and held under 2^53. | A request outliving a reload fails with an error, not a hang. |
| 7.7 | Batch coordinate conversion wrapping the `coord` functions, capped at `convert_max_points_per_command`. | N points convert in one dispatch slot. |
| 7.8 | Laser and infrared spots as managed resources with interval, `max_spots`, `spot_max_updates` and epoch teardown. | A lased target tracks with no consumer round trip. A killed source destroys the spot and emits a record. |
| 7.9 | Weapon tracking subscription with launcher, target and category filters, bounded by `max_tracked_weapons` and sampled at `weapon_max_samples`. | `S_EVENT_SHOT` volume does not become an unbounded per-frame cost. |
| 7.11 | Subscriptions with declared intervals under `max_subscriptions`. | Evaluation cost appears per subscription in `subscription_eval_us`. |

### Phase 8 — Reload

| ID | Task | Done when |
|---|---|---|
| 8.5 | The reload release step releases every `DCSBridge.resources` handle except the event handler. | A reload destroys spots rather than orphaning them. |
| 8.6 | `ReloadSimDriver` with the `reload` capability, Route A only: compile-before-teardown over the whole reload set, sim-driver-tier settings re-read alongside the source, rollback on raise. A Route B install answers with an error naming the route. | A syntax error leaves the running sim driver untouched. One file failing to compile aborts the whole reload. |
| 8.7 | `SimDriverReloaded` with `state_preserved`, `state_version`, `code_sha256`, `subscriptions_dropped`, `spots_dropped`. | A consumer can assert against what it held. |
| 8.C1 | CLI `doctor`, third increment: the effective configuration with each key's owner and tier, any key whose file value has not taken effect yet, and the non-binding caps marked as such. | An operator can see which edits are pending a restart. |

### Phase 9 — Hardening

| ID | Task | Done when |
|---|---|---|
| 9.1 | Per-consumer tokens with an id distinct from the secret, capability sets, constant-time compare; per-topic enforcement from the registered capability maps, inbound and at fan-out; live rotation through `ReloadConfig`. | A `read` token is refused with `Rejected` when it sends a command, and never receives a record its capabilities do not cover. Revoking a token drops its live sessions. |
| 9.2 | Public-bind warning plus `allow_public_bind`. | It runs in a container without editing the source. |
| 9.3 | Network eval record removed from release builds. `EvalExecuted` (SPEC §7.6) is unaffected. | The release `.proto` and sim driver contain no wire-sourced eval path. |
| 9.4 | `max_frame_bytes` checked before allocation, `max_type_url_bytes` before reading the type URL, generated-decoder depth and repetition bounds, fault caught at the thread boundary. | A malformed frame drops one connection only. A group wire type raises and drops the connection. |
| 9.5 | `max_connections`, `max_unauthenticated_connections`, `handshake_timeout_ms`, `auth_failures_per_min`. | A client that never authenticates is closed. |
| 9.6 | Inbound rate limits, per connection and aggregate, answering differently: `inbound_records_per_sec` refuses with `Rejected` and keeps the connection, `inbound_records_per_sec_total` disconnects. Drop-newest on a full ring. | A flooding consumer is refused, then disconnected. `max_connections` legal consumers cannot together exceed dispatch capacity. |
| 9.C1 | CLI `doctor`, fourth increment: policy-refusal detection and the union `autoexec.cfg`, under both `net.allow_unsafe_api` and `net.allow_dostring_in`. | Pasting its output breaks no other installed tool. |
| 9.7 | Run PROBE-7, ring sizing at peak record rate, now that a real rate exists. | `ring_out_records`, `ring_out_lifecycle_reserve`, `ring_in_sim_driver_records` and `ring_in_hook_driver_records` have a measured basis rather than a provisional one. |
| 9.8 | Load test at 10× a captured stream's rate, driven by `dcsb mock` or `dcsb replay`. | Budgets hold. Drops behave as specified. |
| 9.9 | Live public session with metrics recorded. | Real figures under real load. |
| 9.10 | Run PROBE-9: does `DCS.getTaintedFiles` react to the shipped DLL, and does `others` move. | The release notes state the answer rather than calling it unmeasured. |

### Phase 10 — Adoption and packaging

| ID | Task | Done when |
|---|---|---|
| 10.1 | Extend task 1.5's release zip to the full write-directory tree: the built-in files, the editor stub, and the `Config` sample. It creates neither extension directory and neither eval directory. | Installing is one extraction plus the config merge. |
| 10.2 | Installer merges `autoexec.cfg`, unioning the state lists under both keys. | Installing beside another tool leaves that tool working. |
| 10.3 | Sim driver built-ins, SIM §2 Tier 1. | A user installs, connects, and sees records without writing code. |
| 10.4 | Guard list from SIM §9 implemented and unit-tested. | An empty `S_EVENT_HIT` initiator and a nil `Group` are handled. |
| 10.5 | CI check that no `DCS.*` call bypasses task 4.5's accessor table, across the hook driver and both built-in sets. | A namespace change is one edit. See SPEC §4.4. |
| 10.6 | Hook driver built-ins, HOOK §1: identity map, records, commands, capabilities, bounce guards, audit. | The HOOK §13 rows pass. |
| 10.7 | Run PROBE-17, PROBE-18, PROBE-19 and PROBE-20 against the built hook driver built-ins on a dedicated server, and PROBE-A2, PROBE-A5, PROBE-A6 and PROBE-A11 where a second player, a cockpit or an enforcing build allows. | HOOK §12's register is empty. |
| 10.8 | Run PROBE-10: the whole probe set on a dedicated server. | No figure in any document rests on a single-player host alone. |
| 10.9 | The `---@meta` runtime stub and the `.luarc.json` template (Section 2). | Both load clean in a stock Lua language server against a checkout. |
| 10.10 | Reference consumer in one language. | Under 200 lines and obviously a template. |

### Phase 11 — Operator eval

**Deferred past adoption on purpose.** Eval runs operator-authored Lua on a
live server, and SPEC §7 exists so an operator need not end everyone's flight
to change behaviour. At the first deployment's five concurrent players
(SPEC §1.3) a mission reload is cheap, and `ReloadSimDriver` already covers the
sim driver side. So this phase runs after the load test in Phase 9 and the
installer in Phase 10, rather than before either. The task IDs are unchanged:
an ID is a name, not a position.

| ID | Task | Done when |
|---|---|---|
| 8.1 | Eval poller: existing directories only, `*.lua` match, ascending mtime with name as tiebreak, `eval_stable_polls` size check, `eval_max_files_per_poll`. | A half-written file is not executed. Drop order is execution order. |
| 8.2 | Size check against `eval_max_file_bytes`, then rename to `.running`, then compile without executing; `.done`/`.failed` after; startup sweep. | A crashing script does not re-execute. An oversize file is renamed `.failed` unread. |
| 8.3 | `pcall(dofile, path)` for `server\`, direct run for `hook\`, both under the count hook. `eval_instruction_budget` validated as a positive integer at load and on every apply, refusing to enable eval otherwise. Under Route B only `eval\hook\` runs. | A runaway script fails without stalling the frame indefinitely. A budget of zero disables eval loudly instead of silently. |
| 8.4 | Result log `Logs\DCSBridge\eval\<stem>.<UTC>.log` with its header line and `source_sha256`, bounded by `eval_log_max_bytes` then `eval_log_retention_days`; `eval-audit.log` rotated at `eval_audit_max_bytes` under `eval_audit_retention_days`; both written before `EvalExecuted`. | The audit survives with no consumer connected, and a rejected file still produces an audit line. |

---

## 2. What ships alongside

Adoption fails on friction, not on capability. Seven artifacts, in order of
importance.

**Prebuilt binaries in every release.** Ship the broker, the CLI, and a zip
that mirrors the write directory. Installing is one extraction.

**Record and replay.** `record` captures a live session to a file. `replay`
feeds that file to a consumer with no DCS running.

This matters most. Without it, every contributor needs a DCS install, a
licensed map, and a running mission to change one line. The cost is a file
format you already have: framed records, appended.

**A mock producer.** Synthetic traffic conforming to the schema at a
configurable rate. Also the load generator for the Section 1 load test.

**A CLI, and `doctor` among its verbs.** SPEC §15 specifies both: the verbs,
the schema reflection that makes `send` work for uncompiled record types, and
every check `doctor` performs. What ships is one binary.

**The two built-in sets.** SIM §1 specifies the sim driver's record and
command set and HOOK §1 the hook driver's administrative surface. A user
installs, connects, and sees records before writing code.

**Editor support for extension authors.** A `---@meta` stub annotating the SPEC
§6.10 runtime surface — `on`, `command`, `send`, `send_to`, `topics`, `off`,
`replace`, `wrap`, `options` and `mission` — and a `.luarc.json` template
wiring that stub and the generated files into a Lua language server. Completion
and diagnostics then work in an adopter's own repository with no setup beyond
pointing at the checkout.

**A reference consumer** in one language. Minimal, and obviously a template.

---

## 3. Probe method

The register itself is in the specifications: SPEC §16 for the bridge's probes,
HOOK §12 for the hook driver built-ins'. Each row there states the question and
why it matters. This is how to answer them, and it changes as the measurement
does.

| # | Method |
|---|---|
| **PROBE-3** | Time N calls in a tight loop in the state the sim driver runs in. Report per call, and separately report bytes allocated in Lua per call. A generic-crossing proxy stands at 0.6–0.85 µs (SPEC §10). See Section 3.1. |
| **PROBE-7** | Size for a few seconds of production at peak record rate plus a chosen consumer-outage window. **Neither the mission-load blackout nor the drain stall is the rationale**: nothing produces between `onMissionLoadBegin` and `onMissionLoadEnd`, and a drain stall backs up in the sim driver buffer rather than in this ring. Size the outbound ring against a consumer that stops reading. |
| **PROBE-9** | Needs a real server with the shipped hook driver and DLL. The hook-driver-script half is measured clean, so watch `others` specifically; the pre-DLL baseline is in SPEC §14.8. |
| **PROBE-10** | Run the probes there |
| **PROBE-14** | Send a hook-driver-targeted command under each route. Confirm the hook driver dispatches it and the sim driver ring never sees it. |
| **PROBE-17** | Register it in a write-directory hook on a dedicated server and log every invocation's arguments. The host-side argument shape is in HOOK §5. |
| **PROBE-18** | A banned second client attempts to connect. The other three properties of the call are measured in HOOK §6. |
| **PROBE-19** | Instrument with the built bridge and a real consumer. The script-side floor is in HOOK §2. |
| **PROBE-20** | Send to one client with a second client watching. |

### 3.1 PROBE-3 in detail

The broker encodes, so there is no Lua encoder to compare. What remains on the
logic thread is the put calls themselves: one crossing from Lua into C per
field, per record.

Measure two things per call:

- **Microseconds**, timed over N calls in a tight loop, in the state the sim
  driver runs in.
- **Bytes allocated in Lua**, which should be zero. A put call passes a number
  or a string that already exists. If it is not zero, something in the emitter
  is building a value rather than passing one.

If the crossing cost turns out to dominate, the answer is a batched put form —
one call carrying several fields — not a Lua-side encoder.

Two measurement traps in reference Lua 5.1:

- `collectgarbage("count")` reports memory **in use**, not bytes allocated.
  Take the delta with the collector stopped across N calls and divide. Sampling
  with the collector running measures allocation minus collection, which is not
  the quantity wanted.
- There is no per-call-site collector attribution. The incremental collector
  runs in steps triggered by allocation, so its cost appears wherever the next
  allocation happens. The honest measurement is total frame time under a fixed
  record load, collector running versus stopped. Anything finer looks precise
  and means nothing.

Demoted to a Phase 1 optimisation: whether target decoders accept a non-minimal
length varint. The scratch-buffer copy is cheap and the compatibility risk is
not worth carrying as an open question. Task 2.5's done-when covers it.

---

## 4. Project conventions

**Licence.** Permissive. State it in the README.

**A coverage document.** One file lists every record type and command the
bridge and both built-in sets implement.

**A stated versioning policy.** The README states what a version bump means and
what compatibility it promises below 1.0. SPEC §13.3 holds the six version
numbers, four of which carry a bump rule; the README states the release version
and nothing else.

**The specification is four documents.** `SPEC` is the bridge: the broker, the
hook driver, the sim driver runtime, the generator, the CLI, and every record
the bridge itself defines. It changes when the design changes. `SIM` is the sim
driver built-ins' record and command set, and `HOOK` is the hook driver
built-in set that ships beside it; both change when ED ships a release, and
they change independently of each other because one is sim-driver-tier and the
other hook-driver-tier. This document is the plan, and it changes weekly.
Keeping all four in one file meant every plan edit touched the file adopters
cite, and a built-in inventory churned the specification for reasons that had
nothing to do with it.

**One rule governs every assignment: a document may cite a document that
changes less often than it does, never one that changes more often.** The
bridge specification cites nothing outside itself for a requirement. The two
built-in documents cite the bridge. The plan cites all three. Nothing cites the
plan for a requirement, and the bridge cites neither built-in document for one.
Where a rule and the evidence for it sit in different documents today, the
evidence moves to the rule.

Three further rules make the split work. Each document assigns its sections by
**which vocabulary they define**: SPEC §1.2 enumerates the bridge's own records
and SPEC §9.5 its own commands, and SPEC §8.2 does the same by package name — a
topic is its payload's fully-qualified type name, so `dcs.bridge`,
`dcs.builtin` and an adopter's own package partition the space without a
registry. A section belongs to the document that owns the topics it describes.
The probe *register* stays with the component whose behaviour each probe
decides, because the bridge's ring sizes are provisional on PROBE-7 and the
specification must not depend on the plan. And references gain a document
prefix: `SPEC §6.4`, `SIM §3`, `HOOK §7`, `PLAN §1`. A reference inside one
document keeps the bare "Section N.M" form; only a reference that crosses a
document boundary carries a prefix.

**A cross-cutting table is not split.** The SPEC §10 budget is a sum, the SPEC
§12 metrics are one flat namespace, SPEC §13.1 requires that every bound gets a
row in the same commit that introduces it, and SPEC §11 is only useful read end
to end. `Config\DCSBridge.lua` is one file with one reader and `shim.stats`
returns one table — the Route B sim driver reading its own sim-driver-tier keys
at bootstrap is the single exception — so documenting either across several
documents would make an operator read all of them to learn what goes in one.
Each stays whole in the bridge specification, and a built-in document that adds
a row adds it there.

**A table row may name the document that defines what it governs.** That is a
pointer outward, not a dependency: the bridge still knows its own behaviour
without reading the other document. The rule the invariant forbids is a bridge
*rule* whose justification lives elsewhere.

**A probe and a test row travel with the behaviour they cover**, because their
value is per-behaviour rather than per-completeness. A probe that decides a
built-in set's behaviour belongs to that set's document.

**Everything outside the Lua files is Rust**, and it builds on Linux, macOS or
Windows. SPEC §2 pins the language and SPEC §5.1.1 pins what the broker links
against. The product target is `x86_64` Windows for as long as DCS runs nowhere
else, and it is cross-compiled: the broker and the CLI are built for Windows
from any of the three hosts, so no contributor needs a Windows machine to
produce a release artifact.

**The broker also builds host-native, for tests only.** Nothing in the broker
touches DCS, so the same crate compiles to a `.so` or `.dylib` that a stock Lua
5.1 can load. SPEC §17's Host column says which rows that reaches — about half
of them, including every ring, framing, drop-policy, registration, capability
and hardening row. Those run in CI, and so do the *Any* rows the stub harness
covers — the sim driver and hook driver Lua against stubbed `DCS` globals and a
recording mock for the put calls. The rows marked *Windows + DCS* need the sim
and cannot, and no amount of build portability changes that: object lifetimes,
real error modes, the injection routes and the frame budget are checkable only
where DCS runs.

**Both injection routes stay documented.** Route A is the default. Route B is
the framework-integration route, and the route for operators who will not
enable the API. SPEC §5.4.1 states both and states Route B's cost, and four
features are Route A's alone — `ReloadSimDriver`, mission-adjacent files,
`eval\server\`, and the injected mission name. Neither route is quietly dropped
from the documents because one is less used.

**Cross-references are checked in CI**, across four documents rather than
within one. SPEC §8.4 runs `buf breaking` so the schema cannot drift. Run the
same discipline over the prose: every bare "Section N.M" must resolve to a
heading in its own document, every prefixed `SPEC §`, `SIM §`, `HOOK §` and
`PLAN §` reference must resolve in the document it names, and every `[PROBE-n]`
must resolve to a row in whichever register owns it. A stale pointer in a
document people cite is the same defect as a renumbered field.

**The citation direction is reviewed, not linted.** SPEC names SIM, HOOK and
PLAN in a good many places, and nearly all of them are navigational: a config
row saying which document defines what a setting governs, a schema rule saying
where a consumer's conversion is specified, a layout line naming a file's
owner. Those are correct and a linter that failed them would be turned off in a
week. **The test is whether a reader could implement the bridge with only SPEC
open.** Where the answer is no — where a SPEC requirement is incomplete without
a downstream document — the content moves to SPEC. That review runs when a
document is edited, not on every commit.

---
