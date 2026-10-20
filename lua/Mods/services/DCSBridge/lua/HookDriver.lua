-- The hook driver, run by the loader in Scripts/Hooks/DCSBridge.lua. It
-- returns one table and sets no global. ADR 0048.

-- DCS's own log, which writes dcs.log. The bridge's logger below is a
-- different thing, so this one is reached under this name only.
local dcslog = log

local LEVELS = { error = 1, warn = 2, info = 3, debug = 4 }
local NAMES = { 'ERROR', 'WARN', 'INFO', 'DEBUG' }

local LIVE = 'DCSBridge.log'

-- A rotated file's name: its UTC stamp, and a suffix when the second was
-- already taken. ADR 0047.
local ROTATED = '^DCSBridge%.(%d%d%d%d%d%d%d%d%-%d%d%d%d%d%d)%-?(%d*)%.log$'

local logger = {
  max_bytes = 33554432,
  retention_days = 14,
  level = LEVELS.info,
}

local function exists(path)
  local f = io.open(path, 'r')
  if f then
    f:close()
    return true
  end
  return false
end

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

-- Hold the rotated files together to the cap, oldest deleted first. The
-- live file is not counted, so the file just rotated survives whenever it
-- fits alone.
local function prune(dir)
  local files = rotated(dir)
  local total = 0
  for _, file in ipairs(files) do
    file.size = size(dir .. '/' .. file.name)
    total = total + file.size
  end
  for _, file in ipairs(files) do
    if total <= logger.max_bytes then
      break
    end
    os.remove(dir .. '/' .. file.name)
    total = total - file.size
  end
end

local function rotate()
  local dir = logger.dir
  local live = dir .. '/' .. LIVE
  logger.file:close()

  local stamp = os.date('!%Y%m%d-%H%M%S')
  local target = dir .. '/DCSBridge.' .. stamp .. '.log'
  local n = 1
  while exists(target) do
    n = n + 1
    target = dir .. '/DCSBridge.' .. stamp .. '-' .. n .. '.log'
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
  logger.file = assert(io.open(live, 'w'))
  prune(dir)
end

local function write(level, message)
  if level > logger.level or not logger.file then
    return
  end
  local line = os.date('!%Y-%m-%d %H:%M:%S') .. ' ' .. NAMES[level] .. ' '
    .. message .. '\n'
  local at = logger.file:seek('end')
  if at > 0 and at + #line > logger.max_bytes then
    rotate()
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
  lfs.mkdir(dir)
  logger.dir = dir
  logger.file = assert(io.open(dir .. '/' .. LIVE, 'a'))
  prune(dir)
end

return {
  log = log,
}
