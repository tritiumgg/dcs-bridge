-- The sim driver, loaded on every mission load by one of two routes.
--
-- On the Stock route the hook driver injects a chunk into the 'server'
-- state, and that chunk runs this file with dofile. The chunk assigns the
-- load's parameters to DCSBridge_load first. This file takes them and
-- clears the name, so DCSBridge is the one global it leaves. ADR 0065.
--
-- On the Modified route a line in MissionScripting.lua runs this file
-- before the block that sanitizes the mission scripting state, and nothing
-- sets DCSBridge_load. This file then reads its own keys from
-- Config/DCSBridge.lua. It never raises there: a raise would stop
-- MissionScripting.lua before the sanitization, leaving os, io, lfs and
-- package in every mission script's reach. ADR 0069.

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

-- The operator's file, under the write directory. ADR 0051.
local CONFIG = 'Config/DCSBridge.lua'

-- A value as text, for a log line. A table whose __tostring raises would
-- otherwise raise in the path that reports a raise.
local function text(value)
  local ok, shown = pcall(tostring, value)
  if ok and type(shown) == 'string' then
    return shown
  end
  return 'a value that does not convert to text'
end

-- One line in dcs.log under DCSBridge, as the loader writes its own. level
-- is INFO or ERROR. Whether log exists in the state MissionScripting.lua
-- runs this file in is unverified, so a missing one loses the line and
-- raises nothing. ADR 0069.
local function say(level, message)
  pcall(function()
    if type(log) == 'table' and type(log.write) == 'function' then
      log.write('DCSBridge', log[level], message)
    end
  end)
end

-- Every key at its default. A table default is a new table each time.
local function defaults()
  local values = {}
  for _, row in ipairs(KEYS) do
    local default = row[2]
    if type(default) == 'table' then
      default = {}
    end
    values[row[1]] = default
  end
  return values
end

-- The file at path overlaid on the defaults, and true when it was absent;
-- or nil and the reason it does not read. It reads as the hook driver's
-- config.read does, so one file reads the same on both routes. A key this
-- file does not own is ignored without a line: the hook driver logs the
-- ones no row names, once, at DCS start. ADR 0051, ADR 0069.
local function read(path)
  local values = defaults()
  -- Only ENOENT means absent. Its number is 2 on every platform.
  local f, reason, errno = io.open(path, 'rb')
  if not f then
    if errno == 2 then
      return values, true
    end
    return nil, 'the configuration does not open: ' .. text(reason)
  end
  local source = f:read('*a')
  f:close()
  -- A byte-order mark does not compile in Lua 5.1, and the '@' names the
  -- chunk by its path in a compile error.
  source = source:gsub('^\239\187\191', '')
  local chunk, err = loadstring(source, '@' .. path)
  if not chunk then
    return nil, 'the configuration does not compile: ' .. text(err)
  end
  -- No global reaches the file, so it can name values and nothing else.
  setfenv(chunk, {})
  local ran, file = pcall(chunk)
  if not ran then
    return nil, 'the configuration raised: ' .. text(file)
  end
  if type(file) ~= 'table' then
    return nil, path .. ' returned a ' .. type(file) .. ', not a table'
  end
  for _, row in ipairs(KEYS) do
    local name = row[1]
    local value = file[name]
    if value ~= nil then
      values[name] = value
    end
  end
  return values, false
end

-- Each key the file moved from its default, as 'name value'. A table key
-- that holds anything is named alone.
local function moved(values)
  local found = {}
  for _, row in ipairs(KEYS) do
    local name, value = row[1], values[row[1]]
    if type(value) == 'table' then
      if next(value) ~= nil then
        found[#found + 1] = name
      end
    elseif value ~= row[2] then
      found[#found + 1] = name .. ' ' .. tostring(value)
    end
  end
  return found
end

-- The Modified route's load, from the operator's file, and the INFO line
-- that reports it. Raises when the file does not read; the caller contains
-- it. The epoch and the mission are nil: the hook driver allocates the
-- epoch after this file runs, and injects nothing on this route. ADR 0069.
local function bootstrap()
  local path = lfs.writedir() .. CONFIG
  local settings, absent = read(path)
  if not settings then
    error(absent, 0)
  end
  local parts = { absent and 'no ' .. path .. ', defaults in force' or 'read ' .. path }
  for _, part in ipairs(moved(settings)) do
    parts[#parts + 1] = part
  end
  return {
    settings = settings,
    route = 'modified',
  }, 'the sim driver loaded on the modified route: ' .. table.concat(parts, ', ')
end

local params = DCSBridge_load
DCSBridge_load = nil

-- No handoff is the Modified route. A failure here is one line and no
-- DCSBridge, never a raise. ADR 0069.
if params == nil then
  local ok, load, line = pcall(bootstrap)
  if not ok then
    say('ERROR', 'the sim driver did not load on the modified route: ' .. text(load))
    return
  end
  DCSBridge = {
    grammar = GRAMMAR_VERSION,
    load = load,
  }
  say('INFO', line)
  return
end

if type(params) ~= 'table' then
  error('DCSBridge_load is not a table: the hook driver loads this file', 0)
end
params.route = 'stock'

-- The hook driver's chunk reads load.epoch back from here, which is how it
-- knows this file ran and not only that the chunk did. ADR 0065.
DCSBridge = {
  grammar = GRAMMAR_VERSION,
  load = params,
}
