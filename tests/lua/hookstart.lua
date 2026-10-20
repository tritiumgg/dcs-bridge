-- The hook driver's start: the banner first, then Config/DCSBridge.lua read
-- and its log keys handed to the logger, so every line after the banner is
-- written at the file's level. A file that does not read stops the start.
-- ADR 0048, ADR 0051.
--
-- Each case installs the hook driver into a scratch write directory and
-- runs its start directly, under a stock Lua 5.1 with lfs and DCS's log
-- stubbed. Run it through tools/luatest.sh.

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local dcs = require('dcs')

local ROOT = dcs.root()
local DRIVER = 'Mods/services/DCSBridge/lua/HookDriver.lua'

local scratch = {}

local function read(path)
  local f = io.open(path, 'rb')
  if not f then
    return nil
  end
  local text = f:read('*a')
  f:close()
  return text
end

local function write(path, text)
  assert(os.execute("mkdir -p '" .. path:match('^(.*)/[^/]+$') .. "'") == 0)
  local f = assert(io.open(path, 'wb'))
  f:write(text)
  f:close()
end

-- A write directory with Logs/ and the hook driver in it, and the config
-- file when given. Returns the driver and the directory.
local function install(config)
  local dir = dcs.scratch()
  scratch[#scratch + 1] = dir
  assert(os.execute("mkdir -p '" .. dir .. "Logs'") == 0)
  write(dir .. DRIVER, assert(read(ROOT .. 'lua/' .. DRIVER)))
  if config then
    write(dir .. 'Config/DCSBridge.lua', config)
  end
  lfs = dcs.lfs(dir)
  log = dcs.log()
  return dofile(dir .. DRIVER), dir
end

-- DCSBridge.log as lines, without their timestamps.
local function lines(dir)
  local found = {}
  for line in assert(read(dir .. 'Logs/DCSBridge/DCSBridge.log')):gmatch('[^\n]+') do
    found[#found + 1] = line:gsub('^%S+ %S+ ', '')
  end
  return found
end

-- Absent: the banner, then one line saying the defaults are in force.
do
  local driver, dir = install()
  driver.start({ banner = 'the banner' })
  local own = lines(dir)
  assert(own[1] == 'INFO the banner', 'the first line is not the banner: ' .. tostring(own[1]))
  assert(own[2] == 'INFO no ' .. dir .. 'Config/DCSBridge.lua, defaults in force',
    'the second line is not the defaults: ' .. tostring(own[2]))
end

-- A file at error: the banner is there, and no INFO line follows it.
do
  local driver, dir = install("return { log_level = 'error' }")
  driver.start({ banner = 'the banner' })
  local own = lines(dir)
  assert(own[1] == 'INFO the banner', 'the banner is missing at error')
  assert(#own == 1, 'a line followed the banner at error: ' .. tostring(own[2]))
end

-- A key that is not one: a WARN line naming it.
do
  local driver, dir = install('return { prot = 1 }')
  driver.start({ banner = 'the banner' })
  local own = lines(dir)
  assert(own[2] == 'INFO read ' .. dir .. 'Config/DCSBridge.lua', 'the read was not logged')
  assert(own[3] and own[3]:find('^WARN .*names prot, which is not a key'),
    'no WARN line names prot: ' .. tostring(own[3]))
end

-- The file's cap and retention reach the logger. The open pruned at the
-- defaults, before the file was read, so a rotated file five days old
-- survives it. Then a rotation comes at the file's 4096-byte cap, not the
-- default 32 MiB, and its prune deletes that file at a one-day retention.
do
  local old = 'DCSBridge.' .. os.date('!%Y%m%d-%H%M%S', os.time() - 5 * 86400) .. '.log'
  local driver, dir = install('return { log_max_bytes = 4096, log_retention_days = 1 }')
  local logs = dir .. 'Logs/DCSBridge/'
  write(logs .. old, 'old\n')
  driver.start({ banner = 'the banner' })
  assert(read(logs .. old), 'the old file did not survive the open')

  local long = string.rep('x', 3000)
  driver.log.info(long)
  driver.log.info(long)

  assert(not read(logs .. old), 'the old file outlived a one-day retention')
  local rotated = 0
  for name in lfs.dir(logs) do
    if name:find('^DCSBridge%.%d+%-%d+.*%.log$') then
      rotated = rotated + 1
    end
  end
  assert(rotated == 1, rotated .. ' rotated files, expected the one rotated at 4096 bytes')
end

-- A file that does not read: start raises the reason, after logging it.
do
  local driver, dir = install('return 1')
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran on a file that does not read')
  assert(err:find('returned a number, not a table', 1, true), 'start raised ' .. tostring(err))
  local own = lines(dir)
  assert(own[1] == 'INFO the banner', 'the banner is missing before the error')
  assert(own[2] == 'ERROR ' .. err, 'the reason is not logged: ' .. tostring(own[2]))
end

for _, dir in ipairs(scratch) do
  dcs.remove(dir)
end

print('start reads the configuration after the banner')
