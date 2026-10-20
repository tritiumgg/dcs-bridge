-- The hook driver's start: the banner first, then Config/DCSBridge.lua read
-- and its log keys handed to the logger, so every line after the banner is
-- written at the file's level, then the broker module opened and handed
-- every broker key in one call. A file that does not read stops the start
-- before the module opens. ADR 0048, ADR 0050, ADR 0051.
--
-- Each case installs the hook driver into a scratch write directory and
-- runs its start directly, under a stock Lua 5.1 with lfs and DCS's log
-- stubbed. Every case replaces the driver's open, so no module needs to sit
-- at the shipped path. One case opens the real module, and it is the only
-- one: the broker is one per process, so only the first configure in this
-- process binds.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookstart.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local dcs = require('dcs')

local ROOT = dcs.root()
local DRIVER = 'Mods/services/DCSBridge/lua/HookDriver.lua'
local DLL = 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll'

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

-- An opener whose module records each configure and answers as the broker
-- does, without one.
local function recording()
  local calls = {}
  local function open()
    return {
      configure = function(table)
        calls[#calls + 1] = table
        return { interface = 'x', applied = 0, unknown = 0, pending_restart = 0, pending = {} }
      end,
    }
  end
  return open, calls
end

-- DCSBridge.log as lines, without their timestamps.
local function lines(dir)
  local found = {}
  for line in assert(read(dir .. 'Logs/DCSBridge/DCSBridge.log')):gmatch('[^\n]+') do
    found[#found + 1] = line:gsub('^%S+ %S+ ', '')
  end
  return found
end

-- Absent: the banner, the defaults, and one configure carrying exactly the
-- broker's keys, each at its default.
do
  local driver, dir = install()
  local open, calls = recording()
  driver.open = open
  driver.start({ banner = 'the banner' })

  assert(#calls == 1, #calls .. ' configure calls, expected 1')
  local handed, n = calls[1], 0
  for _ in pairs(handed) do
    n = n + 1
  end
  assert(n == 30, n .. ' keys handed to the broker, expected 30')
  for _, row in ipairs(driver.config.KEYS) do
    local name, default, owner = row[1], row[2], row[3]
    if owner ~= 'broker' then
      assert(handed[name] == nil, 'the broker was handed ' .. name)
    elseif name == 'tokens' then
      assert(type(handed.tokens) == 'table' and next(handed.tokens) == nil,
        'tokens is not an empty list')
    else
      assert(handed[name] == default, name .. ' was handed as ' .. tostring(handed[name]))
    end
  end

  local own = lines(dir)
  assert(own[1] == 'INFO the banner', 'the first line is not the banner: ' .. tostring(own[1]))
  assert(own[2] == 'INFO no ' .. dir .. 'Config/DCSBridge.lua, defaults in force',
    'the second line is not the defaults: ' .. tostring(own[2]))
  assert(own[3] == 'INFO configured: interface x, applied 0, unknown 0, pending_restart 0',
    'the third line is not the answer: ' .. tostring(own[3]))
end

-- The file's broker values reach the broker, tokens with them, and its hook
-- values stay out of the broker's table.
do
  local driver = install([[
    return {
      port = 7743,
      tokens = { { id = 'map', secret = 'map-secret', caps = { 'read' } } },
      route = 'modified',
    }
  ]])
  local open, calls = recording()
  driver.open = open
  driver.start({ banner = 'the banner' })

  local handed = calls[1]
  assert(handed.port == 7743, 'port was handed as ' .. tostring(handed.port))
  local token = handed.tokens[1]
  assert(#handed.tokens == 1 and token.id == 'map' and token.secret == 'map-secret'
    and token.caps[1] == 'read', 'the token did not arrive as written')
  assert(handed.route == nil, 'the broker was handed route')
end

-- A file at error: the banner is there, and no INFO line follows it.
do
  local driver, dir = install("return { log_level = 'error' }")
  driver.open = recording()
  driver.start({ banner = 'the banner' })
  local own = lines(dir)
  assert(own[1] == 'INFO the banner', 'the banner is missing at error')
  assert(#own == 1, 'a line followed the banner at error: ' .. tostring(own[2]))
end

-- A key that is not one: a WARN line naming it.
do
  local driver, dir = install('return { prot = 1 }')
  driver.open = recording()
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
  driver.open = recording()
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

-- A file that does not read: start raises the reason after logging it, and
-- the module is never opened.
do
  local driver, dir = install('return 1')
  local open, calls = recording()
  local opened = false
  driver.open = function(...)
    opened = true
    return open(...)
  end
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran on a file that does not read')
  assert(err:find('returned a number, not a table', 1, true), 'start raised ' .. tostring(err))
  assert(not opened and #calls == 0, 'the module opened on a file that does not read')
  local own = lines(dir)
  assert(own[1] == 'INFO the banner', 'the banner is missing before the error')
  assert(own[2] == 'ERROR ' .. err, 'the reason is not logged: ' .. tostring(own[2]))
end

-- A refusal from the broker: logged and raised.
do
  local driver, dir = install()
  driver.open = function()
    return { configure = function() error('configure refused: `port` must be 0 to 65535', 0) end }
  end
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran past a refusal')
  assert(err == 'configure refused: `port` must be 0 to 65535', 'start raised ' .. tostring(err))
  local own = lines(dir)
  assert(own[#own] == 'ERROR ' .. err, 'the refusal is not logged: ' .. tostring(own[#own]))
end

-- The real module, asked for at the shipped path and opened at the
-- harness's. Port 0 lets the system pick, so the run collides with nothing.
-- load.lua says why both spellings are tried.
do
  local driver, dir = install('return { port = 0 }')
  local shim
  driver.open = function(path)
    assert(path == dir .. DLL, 'start asked for ' .. tostring(path))
    for _, name in ipairs({ 'luaopen_dcsbridge', '_luaopen_dcsbridge' }) do
      local opener = package.loadlib(MODULE, name)
      if opener then
        shim = opener()
        return shim
      end
    end
    error('no opener in ' .. MODULE, 0)
  end
  driver.start({ banner = 'the banner' })

  local own = lines(dir)
  for _, line in ipairs(own) do
    assert(not line:find('defaults in force', 1, true), 'the file was not read')
  end
  local want = 'INFO configured: interface ' .. shim.interface .. ', applied '
  assert(own[#own]:sub(1, #want) == want
    and own[#own]:find(', unknown 0, pending_restart 0$'),
    'the last line is not the broker\'s answer: ' .. tostring(own[#own]))
end

for _, dir in ipairs(scratch) do
  dcs.remove(dir)
end

print('start reads the configuration after the banner and configures the broker')
