-- The hook driver, run by the loader in Scripts/Hooks/DCSBridge.lua. It
-- returns one table and sets no global. ADR 0048.

-- DCS's own log, which writes dcs.log. The bridge's logger below is a
-- different thing, so this one is reached under this name only.
local dcslog = log

-- A value as text, for a log line. A table whose __tostring raises would
-- otherwise raise in the path that reports a raise.
local function text(value)
  local ok, shown = pcall(tostring, value)
  if ok and type(shown) == 'string' then
    return shown
  end
  return 'a value that does not convert to text'
end

-- The namespace DCS's hook functions live under. ED is renaming it, and
-- every call in this file reaches it through dcs.call, so the rename is an
-- edit to this line alone. ADR 0060.
local NAMESPACE = 'DCS'

-- The namespace dostring_in lives under, the one way into another Lua
-- state. It is reached through the same accessor. ADR 0064.
local NET = 'net'

local dcs = {}

-- pcall's answer as reach returns it, every result kept, a trailing nil
-- included.
local function settled(namespace, name, ok, ...)
  if not ok then
    return false, namespace .. '.' .. name .. ' raised: ' .. text((...))
  end
  return true, ...
end

-- The function name in the global table namespace, called with the
-- arguments, alone and under pcall. Returns true and its results, or false
-- and the reason: the namespace or the function is missing, or what the
-- call raised. It never raises. The namespace is looked up at each call, so
-- a table DCS replaces after this file loads is the one reached. Both
-- lookups are raw, so a metatable whose __index raises cannot raise through
-- here.
local function reach(namespace, name, ...)
  local found = rawget(_G, namespace)
  if type(found) ~= 'table' then
    return false, 'no ' .. namespace .. ' namespace'
  end
  local fn = rawget(found, name)
  if type(fn) ~= 'function' then
    return false, namespace .. '.' .. name .. ' is not a function'
  end
  return settled(namespace, name, pcall(fn, ...))
end

-- A function of DCS's hook namespace, through reach.
function dcs.call(name, ...)
  return reach(NAMESPACE, name, ...)
end

-- The chunk run in the named Lua state, through reach. Returns true and
-- what the state answered, or false and the reason. ADR 0064.
function dcs.dostring(target, chunk)
  return reach(NET, 'dostring_in', target, chunk)
end

local LEVELS = { error = 1, warn = 2, info = 3, debug = 4 }
local NAMES = { 'ERROR', 'WARN', 'INFO', 'DEBUG' }

-- Every key Config\DCSBridge.lua may name, with its default and the
-- component that owns it. A file key with no row is dropped. ADR 0050.
--
-- One row per line, in the shape { 'name', default, 'owner' }. A Rust test
-- in crates/broker/src/config.rs reads the broker rows by that shape and
-- compares each default with the broker's own, so a row split over two
-- lines fails it.
local KEYS = {
  -- The broker's, in the order crates/broker/src/config.rs declares them.
  { 'enabled', true, 'broker' },
  { 'bind_address', '127.0.0.1', 'broker' },
  { 'port', 7742, 'broker' },
  { 'allow_public_bind', false, 'broker' },
  { 'max_connections', 8, 'broker' },
  { 'max_unauthenticated_connections', 4, 'broker' },
  { 'handshake_timeout_ms', 5000, 'broker' },
  { 'max_frame_bytes', 1048576, 'broker' },
  { 'max_type_url_bytes', 256, 'broker' },
  { 'rejected_max_per_sec', 10, 'broker' },
  { 'busy_max_per_sec', 100, 'broker' },
  { 'inbound_records_per_sec', 100, 'broker' },
  { 'inbound_records_per_sec_total', 400, 'broker' },
  { 'auth_failures_per_min', 5, 'broker' },
  { 'tokens', {}, 'broker' },
  { 'ring_out_lossy_records', 3584, 'broker' },
  { 'ring_out_durable_records', 512, 'broker' },
  { 'ring_out_lifecycle_records', 256, 'broker' },
  { 'ring_in_sim_driver_records', 1024, 'broker' },
  { 'ring_in_hook_driver_records', 256, 'broker' },
  { 'heartbeat_interval_ms', 1000, 'broker' },
  { 'dcs_alive_threshold_ms', 30000, 'broker' },
  { 'dcs_alive_threshold_loading_ms', 120000, 'broker' },
  { 'load_timeout_ms', 120000, 'broker' },
  { 'request_timeout_ms', 2000, 'broker' },
  { 'spool_max_bytes', 268435456, 'broker' },
  { 'spool_retention_hours', 24, 'broker' },
  { 'max_lifecycle_record_bytes', 16384, 'broker' },
  { 'max_lifecycle_topics', 64, 'broker' },
  { 'topic_filter_max_topics', 256, 'broker' },
  -- Which injection route runs, and the sim driver file it injects.
  -- ADR 0015.
  { 'route', 'stock', 'hook' },
  { 'sim_driver_path', 'Mods/services/DCSBridge/lua/SimDriver.lua', 'hook' },
  -- The sim driver's buffer, its per-frame caps and its resource caps.
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
  -- Operator eval.
  { 'eval_max_files_per_poll', 1, 'hook' },
  { 'eval_max_file_bytes', 65536, 'hook' },
  { 'eval_log_max_bytes', 268435456, 'hook' },
  { 'eval_log_retention_days', 30, 'hook' },
  { 'eval_stable_polls', 2, 'hook' },
  { 'eval_instruction_budget', 10000000, 'hook' },
  { 'eval_audit_max_bytes', 16777216, 'hook' },
  { 'eval_audit_retention_days', 90, 'hook' },
  -- The hook driver's dispatch loop.
  { 'hook_driver_dispatch_max_commands', 8, 'hook' },
  -- Extensions. Each file list belongs to the side that loads the files.
  { 'sim_driver_disabled_files', {}, 'sim' },
  { 'hook_driver_disabled_files', {}, 'hook' },
  { 'options', {}, 'sim' },
  { 'mission_sim_driver_dirs', false, 'sim' },
  -- The hook driver built-ins.
  { 'roster_max_players', 128, 'hook' },
  { 'recent_admin_keys', 256, 'hook' },
  { 'kick_message_max_bytes', 256, 'hook' },
  { 'ban_reason_max_bytes', 256, 'hook' },
  { 'chat_message_max_bytes', 512, 'hook' },
  { 'mission_briefing_max_bytes', 16384, 'hook' },
  { 'idempotency_key_max_bytes', 64, 'hook' },
  { 'slotlist_max_entries', 512, 'hook' },
  { 'banlist_max_entries', 512, 'hook' },
  { 'bounce_min_interval_ms', 2000, 'hook' },
  { 'admin_audit_max_bytes', 16777216, 'hook' },
  { 'admin_audit_retention_days', 90, 'hook' },
  -- Timing the hook driver keeps.
  { 'pause_poll_interval_ms', 1000, 'hook' },
  { 'eval_poll_interval_ms', 1000, 'hook' },
  -- The bridge's own log.
  { 'log_max_bytes', 33554432, 'hook' },
  { 'log_retention_days', 14, 'hook' },
  { 'log_level', 'info', 'hook' },
}

-- Each row by its name.
local BY_NAME = {}
for _, row in ipairs(KEYS) do
  BY_NAME[row[1]] = { default = row[2], owner = row[3] }
end

-- The operator's file, a Lua chunk returning one table. ADR 0051.
local CONFIG = lfs.writedir() .. 'Config/DCSBridge.lua'

local config = {
  KEYS = KEYS,
}

-- Every key at its default. A table default is a new table each time, so
-- nothing a caller does to one reaches the next.
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

-- Pairs whose two keys only the hook driver sees together, the first at
-- least the second. The two recent-key memories have to be as deep as the
-- ring that can hold a duplicate, or a key is forgotten before its
-- duplicate arrives. The per-frame evaluation cap has to cover every
-- subscription, so it never binds. ADR 0050 says why the admin pair applies
-- always.
local PAIRS = {
  { 'recent_idempotency_keys', 'ring_in_sim_driver_records' },
  { 'recent_admin_keys', 'ring_in_hook_driver_records' },
  { 'subscription_max_evals', 'max_subscriptions' },
}

local function strings(list)
  local n = 0
  for _ in pairs(list) do
    n = n + 1
  end
  for i = 1, n do
    if type(list[i]) ~= 'string' then
      return false
    end
  end
  return true
end

-- The checks on values the broker never sees: nil if they pass, or the
-- reason naming the key. The broker checks its own when it is handed them.
function config.validate(values)
  if values.route ~= 'stock' and values.route ~= 'modified' then
    return 'route is ' .. tostring(values.route) .. ', and it takes stock or modified'
  end
  if not LEVELS[values.log_level] then
    return 'log_level is ' .. tostring(values.log_level)
      .. ', and it takes error, warn, info or debug'
  end
  for _, name in ipairs({ 'sim_driver_disabled_files', 'hook_driver_disabled_files' }) do
    if not strings(values[name]) then
      return name .. ' is not a list of file names'
    end
  end
  for _, row in ipairs(KEYS) do
    local name, value = row[1], values[row[1]]
    if row[3] ~= 'broker' and type(value) == 'number'
      and (value < 0 or value ~= math.floor(value) or value == math.huge) then
      return name .. ' is ' .. value .. ', and it takes a whole number from 0'
    end
  end
  for _, pair in ipairs(PAIRS) do
    local more, less = pair[1], pair[2]
    if values[more] < values[less] then
      return more .. ' is ' .. values[more] .. ', below ' .. less .. ' at ' .. values[less]
        .. ', and it has to be at least that'
    end
  end
  return nil
end

-- The file at path overlaid on the defaults, and the lines to log about it,
-- each { level = 'info' or 'warn', message = ... }. The caller logs them, so
-- they are written at the level the file sets. Or nil and the reason the
-- file does not read. ADR 0051.
--
-- held, when given, maps keys to the values in force where those are not
-- the file's: the inbound ring sizes after the first configure. The checks
-- run with them in place, so a memory is held as deep as the ring it
-- covers really is, and the values returned are still the file's. ADR 0058.
function config.read(path, held)
  local values = defaults()

  -- Only ENOENT means absent. Its number is 2 on every platform, where the
  -- message text differs. A file that is there and will not open stops the
  -- start like any other file that does not read.
  local f, reason, errno = io.open(path, 'rb')
  if not f then
    -- The third value says the file was absent, which a reload refuses.
    -- ADR 0059.
    if errno == 2 then
      return values, { { level = 'info', message = 'no ' .. path .. ', defaults in force' } }, true
    end
    return nil, 'the configuration does not open: ' .. tostring(reason)
  end
  local text = f:read('*a')
  f:close()

  -- Some Windows editors begin a UTF-8 file with a byte-order mark, which
  -- Lua 5.1 does not compile. The '@' names the chunk by its path, so a
  -- compile error names the file as loadfile's would.
  text = text:gsub('^\239\187\191', '')
  local chunk, err = loadstring(text, '@' .. path)
  if not chunk then
    return nil, 'the configuration does not compile: ' .. tostring(err)
  end
  -- No global reaches the file, so it can name values and nothing else.
  setfenv(chunk, {})
  local ran, file = pcall(chunk)
  if not ran then
    return nil, 'the configuration raised: ' .. tostring(file)
  end
  if type(file) ~= 'table' then
    return nil, path .. ' returned a ' .. type(file) .. ', not a table'
  end

  -- In name order, so the lines read the same on every start.
  local names = {}
  for name in pairs(file) do
    names[#names + 1] = name
  end
  table.sort(names, function(a, b)
    return tostring(a) < tostring(b)
  end)

  local notes = { { level = 'info', message = 'read ' .. path } }
  for _, name in ipairs(names) do
    local row = BY_NAME[name]
    local value = file[name]
    if not row then
      notes[#notes + 1] = {
        level = 'warn',
        message = path .. ' names ' .. tostring(name) .. ', which is not a key, so it is ignored',
      }
    elseif type(value) ~= type(row.default) then
      return nil, path .. ': ' .. name .. ' is a ' .. type(value)
        .. ', and it takes a ' .. type(row.default)
    else
      values[name] = value
    end
  end
  local checked = values
  if held then
    checked = {}
    for name, value in pairs(values) do
      checked[name] = value
    end
    for name, value in pairs(held) do
      checked[name] = value
    end
  end
  local invalid = config.validate(checked)
  if invalid then
    return nil, path .. ': ' .. invalid
  end
  return values, notes
end

-- The values read, as one table per owner. The broker's carries its 30
-- keys and no other, so its unknown count is zero and the nested options
-- table never reaches it. ADR 0050.
function config.split(values)
  local split = { broker = {}, hook = {}, sim = {} }
  for _, row in ipairs(KEYS) do
    split[row[3]][row[1]] = values[row[1]]
  end
  return split
end

-- The hook driver's own keys that a reload does not move. The route is
-- decided at load and the hook driver cannot reload itself, and its
-- extension chain is read once at start. sim_driver_path is stored and
-- takes effect at the next injection. Every other hook key is live. The
-- sim driver's keys are stored for the next injection and not compared.
-- ADR 0057.
local RESTART = { route = true, hook_driver_disabled_files = true }

-- A value as a log line shows it: a file list by its names, in order.
local function shown(value)
  if type(value) == 'table' then
    return '{' .. table.concat(value, ', ') .. '}'
  end
  return tostring(value)
end

-- Whether two values are the same: a file list by its length and the name
-- at each place, so the same list read again into a new table is no change.
-- The joined text would not do: { 'a.lua, b.lua' } joins as two names do.
local function same(a, b)
  if type(a) ~= 'table' or type(b) ~= 'table' then
    return a == b
  end
  if #a ~= #b then
    return false
  end
  for i = 1, #a do
    if a[i] ~= b[i] then
      return false
    end
  end
  return true
end

-- Each key in RESTART whose value in file differs from the one running,
-- as { key, effective, file } in KEYS order, and a copy of file with each
-- of those keys held at the running value. ADR 0057.
function config.pending(running, file)
  local held, pending = {}, {}
  for name, value in pairs(file) do
    held[name] = value
  end
  for _, row in ipairs(KEYS) do
    local name = row[1]
    if RESTART[name] and not same(running[name], file[name]) then
      pending[#pending + 1] = { key = name, effective = running[name], file = file[name] }
      held[name] = running[name]
    end
  end
  return pending, held
end

-- Opened in binary mode, always. Text mode on Windows writes each newline
-- as two bytes, so the length counted against the cap would not be the
-- length written.
local LIVE = 'DCSBridge.log'

-- A rotated file's name: its UTC stamp, and a suffix when the second was
-- already taken. ADR 0047.
local ROTATED = '^DCSBridge%.(%d%d%d%d%d%d%d%d%-%d%d%d%d%d%d)%-?(%d*)%.log$'

local logger = {
  max_bytes = BY_NAME.log_max_bytes.default,
  retention_days = BY_NAME.log_retention_days.default,
  level = LEVELS[BY_NAME.log_level.default],
}

local function size(path)
  return lfs.attributes(path, 'size') or 0
end

-- Every rotated file in the directory, oldest first: by stamp, then by
-- suffix, with no suffix as 1.
local function rotated(dir)
  local found = {}
  for name in lfs.dir(dir) do
    local stamp, suffix = name:match(ROTATED)
    if stamp then
      found[#found + 1] = {
        name = name,
        stamp = stamp,
        suffix = tonumber(suffix) or 1,
      }
    end
  end
  table.sort(found, function(a, b)
    if a.stamp ~= b.stamp then
      return a.stamp < b.stamp
    end
    return a.suffix < b.suffix
  end)
  return found
end

-- Hold the rotated files together to the cap, oldest deleted first, then
-- delete any older than the retention. The live file is not counted, so the
-- file just rotated survives whenever it fits alone.
local function prune(dir)
  local files = rotated(dir)
  local total = 0
  for _, file in ipairs(files) do
    file.size = size(dir .. '/' .. file.name)
    total = total + file.size
  end
  local first = 1
  while first <= #files and total > logger.max_bytes do
    os.remove(dir .. '/' .. files[first].name)
    total = total - files[first].size
    first = first + 1
  end

  -- Both stamps are UTC in the same form, so they compare as strings.
  local cutoff = os.date('!%Y%m%d-%H%M%S', os.time() - logger.retention_days * 86400)
  for i = first, #files do
    if files[i].stamp >= cutoff then
      break
    end
    os.remove(dir .. '/' .. files[i].name)
  end
end

local function rotate()
  local dir = logger.dir
  local live = dir .. '/' .. LIVE
  logger.file:close()

  -- One past the highest suffix this second has used. Filling a gap a
  -- prune left would give the newest file a name that sorts as older.
  local stamp = os.date('!%Y%m%d-%H%M%S')
  local highest = 0
  for _, file in ipairs(rotated(dir)) do
    if file.stamp == stamp and file.suffix > highest then
      highest = file.suffix
    end
  end
  local target = dir .. '/DCSBridge.' .. stamp .. '.log'
  if highest > 0 then
    target = dir .. '/DCSBridge.' .. stamp .. '-' .. (highest + 1) .. '.log'
  end

  local ok, err = os.rename(live, target)
  if not ok then
    -- Another process holding the file open is the usual cause. The cap
    -- holds by losing the live file's contents rather than growing past it.
    dcslog.write(
      'DCSBridge',
      dcslog.ERROR,
      'could not rotate ' .. live .. ', truncating it: ' .. tostring(err)
    )
  end
  prune(dir)

  local file, reason = io.open(live, 'wb')
  logger.file = file
  if not file then
    -- No handle means lines are dropped, never raised into the caller. The
    -- next line tries the file again.
    dcslog.write(
      'DCSBridge',
      dcslog.ERROR,
      'could not reopen ' .. live .. ', dropping lines: ' .. tostring(reason)
    )
  end
end

local function write(level, message)
  if level > logger.level or not logger.dir then
    return
  end
  if not logger.file then
    logger.file = io.open(logger.dir .. '/' .. LIVE, 'ab')
    if not logger.file then
      return
    end
  end
  local line = os.date('!%Y-%m-%d %H:%M:%S') .. ' ' .. NAMES[level] .. ' '
    .. message .. '\n'
  local at = logger.file:seek('end')
  if at > 0 and at + #line > logger.max_bytes then
    rotate()
    if not logger.file then
      return
    end
  end
  logger.file:write(line)
  logger.file:flush()
end

local log = {}

function log.error(message) write(LEVELS.error, message) end
function log.warn(message) write(LEVELS.warn, message) end
function log.info(message) write(LEVELS.info, message) end
function log.debug(message) write(LEVELS.debug, message) end

-- A key not given keeps its value, so a caller passes only what it read.
function log.configure(opts)
  if opts.log_max_bytes ~= nil then
    logger.max_bytes = opts.log_max_bytes
  end
  if opts.log_retention_days ~= nil then
    logger.retention_days = opts.log_retention_days
  end
  if opts.log_level ~= nil then
    logger.level = assert(LEVELS[opts.log_level],
      'log_level is not error, warn, info or debug: ' .. tostring(opts.log_level))
  end
end

function log.open(dir)
  if logger.file then
    logger.file:close()
    logger.file = nil
  end
  lfs.mkdir(dir)
  logger.dir = dir
  logger.file = assert(io.open(dir .. '/' .. LIVE, 'ab'))
  prune(dir)
end

-- The broker module, where the release zip puts it.
local MODULE = lfs.writedir() .. 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll'

-- The hook driver's generated tables and the schema hash they were built
-- from, beside this file in the release zip. ADR 0055.
local GEN = lfs.writedir() .. 'Mods/services/DCSBridge/lua/HookDriver.gen.lua'

-- The compiled schema the broker serves to consumers, from the same build
-- as the generated file. ADR 0056.
local SCHEMA = lfs.writedir() .. 'Mods/services/DCSBridge/schema.pb'

-- The broker module's call surface this file is written against. The
-- module puts its own on the table it opens with, and start compares the
-- two before anything is configured. A Rust test in crates/lua-module
-- reads this line by its shape and compares it with the module's, so the
-- two cannot move apart unnoticed. ADR 0054.
local INTERFACE = '5'

-- The table this file returns. start reaches open through it, so a test
-- that replaces open runs start with no module at MODULE. ADR 0050.
local driver = {
  INTERFACE = INTERFACE,
  config = config,
  dcs = dcs,
  log = log,
}

-- What start read, split by owner, and the broker module once it opened,
-- for the calls that come after it.
local state = {}

-- The kill switch as the hook driver last learned it: from the table it
-- handed configure, at start and at reload, and from shim.tick, which
-- answers what SetEnabled set. On until told otherwise, as the key's
-- default is. ADR 0063.
state.enabled = true

-- The kill switch in effect. Every path that injects, drains, ferries
-- commands or evaluates consults it, so a disabled bridge stops them all
-- while the per-frame tick runs on. ADR 0063.
function driver.enabled()
  return state.enabled
end

-- Every DCS callback the bridge uses, registered once at start. DCS keeps a
-- registered table for the life of the process, so the set carries every
-- name any part of the bridge will need. ADR 0061.
local CALLBACKS = {
  'onSimulationFrame',
  'onSimulationStop',
  'onSimulationPause',
  'onSimulationResume',
  'onMissionLoadBegin',
  'onMissionLoadEnd',
  'onPlayerConnect',
  'onPlayerDisconnect',
  'onPlayerStart',
  'onPlayerStop',
  'onPlayerChangeSlot',
  'onPlayerChangeCoalition',
  'onPlayerTryConnect',
  'onPlayerTryChangeSlot',
  'onPlayerTrySendChat',
  'onChatMessage',
}

-- The body each callback runs, by name, looked up at each call. Created
-- once here and never replaced, so a body set after this file loads is the
-- one that runs. A name with no body returns nothing, which lets an
-- onPlayerTry* callback pass the event on. ADR 0061.
driver.callbacks = {}

-- The names whose body raised since the last mission load. DCS keeps
-- calling a callback that raises, so a raise is logged the first time and
-- not again until the next load. ADR 0062.
state.raised = {}

-- A body's answer as DCS gets it: every value it returned, or nothing when
-- it raised. A raise is logged to DCSBridge.log and never to dcs.log. The
-- log call is guarded too, so nothing here raises into DCS.
local function contained(name, ok, ...)
  if ok then
    return ...
  end
  if not state.raised[name] then
    state.raised[name] = true
    pcall(log.error, name .. ' raised: ' .. text((...))
      .. '; suppressed until the next mission load')
  end
end

-- The table handed to DCS: one function per name, which calls nothing until
-- start has finished and runs each body under its own pcall.
local registered = {}
for _, name in ipairs(CALLBACKS) do
  registered[name] = function(...)
    if not state.started then
      return
    end
    if name == 'onMissionLoadEnd' then
      state.raised = {}
    end
    local body = driver.callbacks[name]
    if body then
      return contained(name, pcall(body, ...))
    end
  end
end

-- The broker module's table. DCS's package.loadlib takes the opener's bare
-- name, so that is the one asked for.
function driver.open(path)
  local opener, err = package.loadlib(path, 'luaopen_dcsbridge')
  if not opener then
    error('the broker module did not open: ' .. path .. ': ' .. tostring(err), 0)
  end
  return opener()
end

-- Log a failure and raise it, so the loader's line in dcs.log says the
-- same thing.
local function fail(reason)
  log.error(reason)
  error(reason, 0)
end

-- The generated file's table, run with the shim its emitters write
-- through. Every release carries the file, so one that is missing, raises
-- or carries no hash is a broken install and stops the start. ADR 0055.
local function load_gen(shim)
  local chunk, err = loadfile(GEN)
  if not chunk then
    fail('the generated file did not load: ' .. tostring(err))
  end
  local ran, gen = pcall(chunk, shim)
  if not ran then
    fail('the generated file raised: ' .. GEN .. ': ' .. tostring(gen))
  end
  if type(gen) ~= 'table' or type(gen.schema_sha256) ~= 'string'
    or not gen.schema_sha256:find('^' .. string.rep('%x', 64) .. '$') then
    fail('the generated file carries no schema_sha256: ' .. GEN)
  end
  return gen
end

-- schema.pb's bytes. Every release carries the file, so one that does not
-- read is a broken install and stops the start. ADR 0056.
local function read_schema()
  local file, err = io.open(SCHEMA, 'rb')
  if not file then
    fail('the schema did not open: ' .. tostring(err))
  end
  local bytes = file:read('*a')
  file:close()
  if not bytes or #bytes == 0 then
    fail('the schema is empty: ' .. SCHEMA)
  end
  return bytes
end

-- One WARN for each key in a pending list, the broker's or the hook
-- driver's own, in one shape. A pending key keeps its value in force, so
-- without the line an operator who edited it sees nothing change and
-- nothing say why. ADR 0057.
local function waits(pending)
  for _, key in ipairs(pending) do
    log.warn(tostring(key.key) .. ': effective ' .. shown(key.effective)
      .. ', file ' .. shown(key.file) .. ', waits for a DCS restart')
  end
end

-- The broker's answer to a configure, then the keys it left pending.
local function report(answer)
  log.info('configured: interface ' .. tostring(answer.interface)
    .. ', applied ' .. tostring(answer.applied)
    .. ', unknown ' .. tostring(answer.unknown)
    .. ', pending_restart ' .. tostring(answer.pending_restart))
  waits(answer.pending or {})
end

-- Hold the kill switch at enabled, with one INFO line when it moves. A call
-- that changes nothing writes nothing, so a caller passes every value it
-- learns through here. ADR 0063.
local function switch(enabled)
  if enabled ~= state.enabled then
    log.info(enabled and 'bridge enabled' or 'bridge disabled')
  end
  state.enabled = enabled
end

-- The per-frame body, which the runtime owns (ADR 0061). The tick is its
-- first statement, ahead of every consumer added to this body, so a
-- consumer that raises cannot stop the heartbeat and make Pong report a
-- dead sim. It runs whether the bridge is enabled or not, since the tick is
-- how a disabled bridge stays alive to Pong and how it hears a re-enable.
-- A clock that does not read raises, so the containment logs it once per
-- mission load and no tick is taken. ADR 0063.
driver.callbacks.onSimulationFrame = function()
  local read, time = dcs.call('getModelTime')
  if not read then
    error(time, 0)
  end
  local enabled = state.shim.tick(time)
  -- A module older than the answer returns nothing. The switch then keeps
  -- the value the file gave it rather than reading the bridge as off.
  if type(enabled) == 'boolean' then
    switch(enabled)
  end
end

-- The inbound ring sizes. The first configure allocates both rings and no
-- later one resizes them, so the sizes it was handed are the ones in force
-- until DCS restarts. ADR 0058.
local RINGS = { 'ring_in_sim_driver_records', 'ring_in_hook_driver_records' }

-- Called once by the loader, which wrote ctx.banner to dcs.log already and
-- logs anything this raises there. The banner comes first, at the default
-- level, so DCSBridge.log names the build even when the file does not read
-- or sets a level that hides INFO.
function driver.start(ctx)
  log.open(lfs.writedir() .. 'Logs/DCSBridge')
  log.info(ctx.banner)

  local values, notes = config.read(CONFIG)
  if not values then
    fail(notes)
  end
  local split = config.split(values)
  state.broker, state.hook, state.sim = split.broker, split.hook, split.sim
  state.rings = {}
  for _, name in ipairs(RINGS) do
    state.rings[name] = state.broker[name]
  end

  log.configure(state.hook)
  for _, note in ipairs(notes) do
    log[note.level](note.message)
  end

  local opened, shim = pcall(driver.open, MODULE)
  if not opened then
    fail(tostring(shim))
  end
  -- Compared here rather than in configure's answer, because the first
  -- configure binds the listener, and a stop after it would leave the
  -- broker listening behind a hook driver that did not start. ADR 0054.
  if shim.interface ~= INTERFACE then
    fail('the broker module is interface ' .. tostring(shim.interface)
      .. ' and this hook driver is interface ' .. INTERFACE
      .. ': they are from different builds')
  end
  -- Loaded before configure, so a broken install stops with nothing bound.
  -- Its hash rides the first configure for the broker to compare with the
  -- schema it is handed. ADR 0052.
  state.gen = load_gen(shim)
  state.broker.schema_sha256 = state.gen.schema_sha256
  -- Read before configure for the same reason, and handed over after it,
  -- since the broker takes a schema only once configured.
  local schema = read_schema()
  -- Registered before configure, so a refusal stops the start with nothing
  -- bound. DCS cannot take the table back, so a start that stops later
  -- leaves it registered, calling no body. ADR 0061.
  local accepted, refused = dcs.call('setUserCallbacks', registered)
  if not accepted then
    fail('the callbacks were not registered: ' .. refused)
  end
  -- Every broker key in one call, so the first call is the whole
  -- configuration and nothing is left at a default the file changed.
  local configured, answer = pcall(shim.configure, state.broker)
  if not configured then
    fail(tostring(answer))
  end
  state.shim = shim
  report(answer)
  switch(state.broker.enabled)

  -- The one stop after the bind. The broker then listens with no schema:
  -- its handshake carries no hash and GetSchema answers an error. ADR 0056.
  local held, sha256 = pcall(shim.schema, schema)
  if not held then
    fail(tostring(sha256) .. '; the broker listens with no schema to serve')
  end
  -- A hash that differs is a warning, not a stop: the bytes are what is
  -- deployed, and the broker counts the mismatch. ADR 0052.
  if sha256 == state.gen.schema_sha256:lower() then
    log.info('schema ' .. sha256)
  else
    log.warn('schema ' .. sha256 .. ' but HookDriver.gen.lua was generated from '
      .. state.gen.schema_sha256)
  end
  -- Last, so the callbacks run their bodies only behind a start that
  -- finished. A stop added to start goes above this line. ADR 0061.
  state.started = true
end

-- Read the file again and hand the broker its keys, which is what
-- ReloadConfig asks for. A file that does not read, or a table the broker
-- refuses, changes nothing: the reason is logged and returned. Nothing is stored until the broker has
-- taken its table, so the running configuration is the previous one until
-- then. The hook driver's restart-tier keys stay at the values in force
-- (ADR 0057), and the pairs are checked against the ring sizes in force
-- (ADR 0058).
--
-- Returns true, the broker's answer and the hook driver's own pending
-- keys, or false and the reason.
function driver.reload()
  if not state.shim then
    error('reload before the hook driver started', 2)
  end
  local values, notes, absent = config.read(CONFIG, state.rings)
  -- At start an absent file is the defaults. Here the defaults would drop
  -- every token in force, so an absent file changes nothing. ADR 0059.
  if absent then
    notes = 'no ' .. CONFIG .. ': the running configuration stays in force'
    values = nil
  end
  if not values then
    log.error(notes)
    return false, notes
  end
  local split = config.split(values)
  local pending, hook = config.pending(state.hook, split.hook)
  -- Checked again by the broker, which keeps nothing from a later call.
  -- ADR 0052.
  split.broker.schema_sha256 = state.gen.schema_sha256
  -- The broker applies a table whole or refuses it whole, so a refusal
  -- leaves both sides on the previous configuration.
  local configured, answer = pcall(state.shim.configure, split.broker)
  if not configured then
    answer = tostring(answer)
    log.error(answer)
    return false, answer
  end

  -- sim_driver_path and the sim driver's keys are stored here and take
  -- effect at the next injection, which reads them.
  state.broker, state.hook, state.sim = split.broker, hook, split.sim
  log.configure(state.hook)
  for _, note in ipairs(notes) do
    log[note.level](note.message)
  end
  report(answer)
  waits(pending)
  -- The broker swaps its table whole, so the file's value is the one in
  -- force now, over any SetEnabled since the last read.
  switch(state.broker.enabled)
  return true, answer, pending
end

return driver
