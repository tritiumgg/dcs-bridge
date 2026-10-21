-- The sim driver. The hook driver injects a chunk into the 'server' state
-- on every mission load, and that chunk runs this file with dofile. The
-- chunk assigns the load's parameters to DCSBridge_load first. This file
-- takes them and clears the name, so DCSBridge is the one global it leaves.
-- ADR 0065.

-- The grammar of what the hook driver hands this file. The hook driver
-- reads this line from disk by its shape before it injects, and refuses a
-- file whose value is not its own, since this file's source never crosses
-- into the hook state. ADR 0065.
local GRAMMAR_VERSION = '1'

-- The sim driver's keys, with their defaults, in the order the hook
-- driver's KEYS list gives them. The two drivers never share a Lua state,
-- so the rows are copied, and tests/lua/simkeys.lua reads both files by
-- the row shape and fails on any drift. One row per line, in the shape
-- { 'name', default, 'sim' }, or the test does not see the row. ADR 0069.
local KEYS = {
  { 'sim_driver_buffer_max_records', 8192, 'sim' },
  { 'drain_max_records', 256, 'sim' },
  { 'drain_max_bytes', 16384, 'sim' },
  { 'bridge_return_max_bytes', 32768, 'sim' },
  { 'dispatch_max_commands', 32, 'sim' },
  { 'subscription_max_evals', 64, 'sim' },
  { 'spot_max_updates', 16, 'sim' },
  { 'weapon_max_samples', 32, 'sim' },
  { 'resync_slice_records', 32, 'sim' },
  { 'max_subscriptions', 64, 'sim' },
  { 'max_spots', 16, 'sim' },
  { 'max_tracked_weapons', 64, 'sim' },
  { 'convert_max_points_per_command', 256, 'sim' },
  { 'recent_idempotency_keys', 1024, 'sim' },
  { 'command_ack_detail_max_bytes', 256, 'sim' },
  { 'spawn_max_groups_per_command', 8, 'sim' },
  { 'handler_failures_per_epoch', 3, 'sim' },
  { 'unsafe_bindings_enabled', false, 'sim' },
  { 'sim_driver_disabled_files', {}, 'sim' },
  { 'options', {}, 'sim' },
  { 'mission_sim_driver_dirs', false, 'sim' },
}

local params = DCSBridge_load
DCSBridge_load = nil
if type(params) ~= 'table' then
  error('DCSBridge_load is not a table: the hook driver loads this file', 0)
end

-- The hook driver's chunk reads load.epoch back from here, which is how it
-- knows this file ran and not only that the chunk did. ADR 0065.
DCSBridge = {
  grammar = GRAMMAR_VERSION,
  load = params,
}
