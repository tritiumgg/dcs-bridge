-- The hook driver, run by the loader in Scripts/Hooks/DCSBridge.lua. It
-- returns one table and sets no global. ADR 0048.

-- DCS's own log, which writes dcs.log. The bridge's logger below is a
-- different thing, so this one is reached under this name only.
local dcslog = log

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
function config.read(path)
  local values = defaults()

  -- Only ENOENT means absent. Its number is 2 on every platform, where the
  -- message text differs. A file that is there and will not open stops the
  -- start like any other file that does not read.
  local f, reason, errno = io.open(path, 'rb')
  if not f then
    if errno == 2 then
      return values, { { level = 'info', message = 'no ' .. path .. ', defaults in force' } }
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
  local invalid = config.validate(values)
  if invalid then
    return nil, path .. ': ' .. invalid
  end
  return values, notes
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

-- Called once by the loader, which wrote ctx.banner to dcs.log already.
local function start(ctx)
  log.open(lfs.writedir() .. 'Logs/DCSBridge')
  log.info(ctx.banner)
end

return {
  config = config,
  log = log,
  start = start,
}
