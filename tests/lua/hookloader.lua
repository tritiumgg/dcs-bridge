-- The loader writes its banner to dcs.log before running anything, and the
-- hook driver writes the same banner to its own log. With ENABLED false the
-- loader writes one line and nothing else. A configuration that does not
-- read stops the start, and both logs say why. ADR 0048, ADR 0049, ADR 0051.
--
-- Each case installs the two files into a scratch write directory, the way
-- the release zip lays them out, and runs the loader the way DCS does, under
-- a stock Lua 5.1 with lfs and DCS's log stubbed. Run it through
-- tools/luatest.sh.

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local dcs = require('dcs')

local ROOT = dcs.root()
local LOADER = 'Scripts/Hooks/DCSBridge.lua'
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

-- A write directory with Logs/ in it, as DCS makes one, and the files named.
-- edit, when given, changes the loader's text the way an operator would.
local function install(files, edit)
  local dir = dcs.scratch()
  scratch[#scratch + 1] = dir
  assert(os.execute("mkdir -p '" .. dir .. "Logs'") == 0)
  for _, file in ipairs(files) do
    local text = assert(read(ROOT .. 'lua/' .. file), 'no ' .. file .. ' in the checkout')
    if file == LOADER and edit then
      text = edit(text)
    end
    write(dir .. file, text)
  end
  lfs = dcs.lfs(dir)
  log = dcs.log()
  return dir
end

-- Enabled, with no broker module installed: one banner in dcs.log naming
-- both files by size and time, and the same banner in DCSBridge.log. The
-- shipped open then fails, naming the module's path in both logs. A real
-- module is opened in hookstart.lua alone.
do
  local dir = install({ LOADER, DRIVER })
  dofile(dir .. LOADER)

  assert(#log.lines == 2, #log.lines .. ' dcs.log lines, expected the banner and one error')
  local line = log.lines[1]
  assert(line.subsystem == 'DCSBridge' and line.level == 'INFO',
    'the banner is not an INFO line under DCSBridge')
  for _, file in ipairs({ LOADER, DRIVER }) do
    local size = lfs.attributes(dir .. file, 'size')
    assert(line.message:find(file .. ' ' .. size .. ' bytes, modified 2023-11-14 22:13:20', 1, true),
      'the banner does not name ' .. file .. ' by its size and UTC modification time')
  end

  local own = read(dir .. 'Logs/DCSBridge/DCSBridge.log')
  assert(own, 'the hook driver wrote no DCSBridge.log')
  assert(own:find('INFO ' .. line.message, 1, true), 'DCSBridge.log does not carry the banner')

  local module = dir .. 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll'
  assert(log.lines[2].level == 'ERROR'
    and log.lines[2].message:find('the hook driver did not start: the broker module did not open: '
      .. module, 1, true),
    'dcs.log does not say the module did not open')
  assert(own:find('defaults in force', 1, true), 'DCSBridge.log does not say the defaults are in force')
  assert(own:find('ERROR the broker module did not open: ' .. module, 1, true),
    'DCSBridge.log does not name the module')
end

-- Disabled: one line naming the flag, no banner, and nothing under
-- Logs/DCSBridge.
do
  local dir = install({ LOADER, DRIVER }, function(text)
    local edited, n = text:gsub('local ENABLED = true', 'local ENABLED = false')
    assert(n == 1, 'the loader has no ENABLED = true line to edit')
    return edited
  end)
  dofile(dir .. LOADER)

  assert(#log.lines == 1, #log.lines .. ' dcs.log lines, expected 1')
  assert(log.lines[1].message:find('disabled by ENABLED = false in ', 1, true),
    'the line does not name the flag')
  assert(not log.lines[1].message:find('loading', 1, true), 'a disabled loader wrote the banner')
  assert(not lfs.attributes(dir .. 'Logs/DCSBridge/DCSBridge.log'),
    'a disabled loader wrote DCSBridge.log')
end

-- A missing hook driver: the banner says so, then one error line, and the
-- loader raises nothing into DCS.
do
  local dir = install({ LOADER })
  dofile(dir .. LOADER)

  assert(#log.lines == 2, #log.lines .. ' dcs.log lines, expected 2')
  assert(log.lines[1].message:find(DRIVER .. ' missing', 1, true),
    'the banner does not say the hook driver is missing')
  assert(log.lines[2].level == 'ERROR'
    and log.lines[2].message:find('the hook driver did not load', 1, true),
    'no error line for the missing hook driver')
end

-- A configuration that does not read: the reason in DCSBridge.log after the
-- banner, and the loader's line saying the hook driver did not start.
do
  local dir = install({ LOADER, DRIVER })
  write(dir .. 'Config/DCSBridge.lua', "return { route = 'A' }")
  dofile(dir .. LOADER)

  assert(#log.lines == 2, #log.lines .. ' dcs.log lines, expected 2')
  assert(log.lines[2].level == 'ERROR'
    and log.lines[2].message:find('the hook driver did not start: ', 1, true)
    and log.lines[2].message:find('route is A', 1, true),
    'no error line in dcs.log naming route')
  local own = read(dir .. 'Logs/DCSBridge/DCSBridge.log')
  assert(own:find('ERROR [^\n]*route is A'), 'DCSBridge.log does not name route')
end

for _, dir in ipairs(scratch) do
  dcs.remove(dir)
end

print('the loader writes its banner to both logs')
