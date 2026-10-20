# ADR 0050: The hook driver holds every key and splits the table by owner

## Status

Accepted

## Context

SPEC 13.2 makes the hook driver the one reader of `Config\DCSBridge.lua`:

> It is a Lua file and the hook driver runs in a Lua state, so the hook driver
> parses it, validates it, and distributes it: broker keys through
> `shim.configure`, sim driver keys through the injection chunk and through
> the reload in Section 6.9.

SPEC 13.1 lists 79 keys in groups, each group naming an owner. The broker
holds its own 30 in the `keys!` block of `crates/broker/src/config.rs`, with
their defaults, and counts any other name it is handed as unknown.
`shim.configure` refuses a table value other than `tokens`, so the `options`
table cannot ride it.

The broker's 30 are not SPEC 13.1's broker rows verbatim. ADR 0028 replaced
`ring_out_records` and `ring_out_lifecycle_reserve` with one size per class:
`ring_out_lossy_records`, `ring_out_durable_records` and
`ring_out_lifecycle_records`.

The hook driver's and the sim driver's keys have no home in code yet. The sim
driver's travel in the injection chunk, so the hook driver has to know them
and their defaults before any sim driver runs.

SPEC 13.2 places three pairs. The broker checks the ones whose keys it holds.
Three pairs cross the line between owners and only the hook driver sees both
sides: `recent_idempotency_keys` at least `ring_in_sim_driver_records`,
`recent_admin_keys` at least `ring_in_hook_driver_records` "where the hook
driver built-ins are deployed", and `subscription_max_evals` at least
`max_subscriptions`.

## Decision

`HookDriver.lua` holds one list, `KEYS`, naming every key with its default and
its owner: `broker`, `hook` or `sim`. It is the file's whole vocabulary.

- The 30 broker rows are the `keys!` block's fields, in its order, with its
  defaults. The old ring names have no row.
- `route` and `sim_driver_path` sit in SPEC 13.1's broker group but the broker
  holds neither. The hook driver decides the route and injects the file, so
  both are `hook`. `pause_poll_interval_ms` and `eval_poll_interval_ms` are
  `hook` for the same reason.
- `sim_driver_disabled_files`, `options` and `mission_sim_driver_dirs` are
  `sim`; `hook_driver_disabled_files` is `hook`.
- One row per line, `{ 'name', default, 'owner' },`. A Rust test reads the
  broker rows by that shape and compares each default with
  `Config::default()`, so a default lives in two places and a drift fails CI.

The reader overlays the file on a copy of the defaults, then splits the
result by owner. `shim.configure` gets the broker's table and nothing else,
all 30 keys every time, so its unknown count is zero and one call carries
every broker value. A file key no row names is a warning in `DCSBridge.log`
and is dropped, which is what an operator writing `ring_out_records` from the
specification sees.

The hook driver checks the three crossing pairs. The `recent_admin_keys` pair
is checked always: no built-in exists to be disabled yet, and the defaults
pass it.

The table the file returns gains two fields. `config` is the reader, so a test
reaches it without running `start`. `open(path)` is the function `start` calls
to load the broker module. A test replaces it, so `start` runs end to end with
no DLL at the shipped path, and the shipped `open` asks for
`luaopen_dcsbridge` only. The Mach-O `_luaopen_dcsbridge` spelling a macOS
harness needs stays in the test.

Alternatives:

- **Forward the whole file to `shim.configure`.** The unknown count then
  counts every hook and sim key, and `options` is refused.
- **Hold only the names and let Rust fill the broker's defaults.** The
  vocabulary for the keys the injection chunk carries would then live nowhere
  in Lua.
- **A sibling `Config.lua` beside `HookDriver.lua`.** A test runs the driver
  with `dofile` from a scratch directory, where a sibling is one more file to
  install for nothing a single file cannot hold.
- **An opener passed in `ctx`.** The loader builds `ctx` and is the file that
  needs a restart; a test could not change it without editing the loader.

## Consequences

A key added to the broker needs a row here as well, or the Rust test fails
naming it.

Reformatting a `KEYS` row onto two lines breaks the Rust test. A comment
beside the list says so.

When the hook driver built-ins arrive and `hook_driver_disabled_files` can
remove them, the `recent_admin_keys` pair becomes conditional on them.

The shipped `open`'s success path runs in no test, because the harness
replaces it. A live install's first start is that check.
