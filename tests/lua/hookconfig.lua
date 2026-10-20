-- The hook driver's configuration vocabulary: every key the file may name,
-- once, with a default and one of three owners, and the broker's 30 among
-- them. ADR 0050.
--
-- Runs the hook driver under a stock Lua 5.1 with lfs and DCS's log stubbed,
-- and opens no broker module. Run it through tools/luatest.sh.

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local dcs = require('dcs')

local DRIVER = dcs.root() .. 'lua/Mods/services/DCSBridge/lua/HookDriver.lua'

local scratch = {}

local function fresh()
  local dir = dcs.scratch()
  scratch[#scratch + 1] = dir
  assert(os.execute("mkdir -p '" .. dir .. "Logs'") == 0)
  lfs = dcs.lfs(dir)
  log = dcs.log()
  return dofile(DRIVER), dir
end

-- Every key named once, under one of three owners, with a plain default.
-- 79 specification rows, less the two ring rows ADR 0028 replaced with
-- three, is 80. The broker's are exactly the 30 it declares.
do
  local driver = fresh()
  local KEYS = driver.config.KEYS
  assert(#KEYS == 80, #KEYS .. ' keys, expected 80')

  local seen, owners = {}, { broker = 0, hook = 0, sim = 0 }
  for _, row in ipairs(KEYS) do
    local name, default, owner = row[1], row[2], row[3]
    assert(type(name) == 'string', 'a key with no name')
    assert(not seen[name], name .. ' is named twice')
    seen[name] = true
    assert(owners[owner], name .. ' has the owner ' .. tostring(owner))
    owners[owner] = owners[owner] + 1
    local kind = type(default)
    assert(kind == 'number' or kind == 'string' or kind == 'boolean' or kind == 'table',
      name .. ' has a ' .. kind .. ' default')
  end
  assert(owners.broker == 30, owners.broker .. ' broker keys, expected 30')

  for _, old in ipairs({ 'ring_out_records', 'ring_out_lifecycle_reserve' }) do
    assert(not seen[old], old .. ' has a row, though the broker counts it unknown')
  end
end

-- The route defaults to the Stock route, by its lower-case name, and the
-- logger starts on the three log rows' values. ADR 0015, ADR 0048.
do
  local driver, dir = fresh()
  local defaults = {}
  for _, row in ipairs(driver.config.KEYS) do
    defaults[row[1]] = row[2]
  end
  assert(defaults.route == 'stock', 'route defaults to ' .. tostring(defaults.route))
  assert(defaults.log_level == 'info', 'log_level defaults to ' .. tostring(defaults.log_level))

  -- At info, an info line is written and a debug line is not.
  driver.log.open(dir .. 'Logs/DCSBridge')
  driver.log.info('at info')
  driver.log.debug('at debug')
  local f = assert(io.open(dir .. 'Logs/DCSBridge/DCSBridge.log', 'rb'))
  local text = f:read('*a')
  f:close()
  assert(text:find('INFO at info', 1, true), 'the logger did not start at info')
  assert(not text:find('at debug', 1, true), 'the logger started below info')
end

for _, dir in ipairs(scratch) do
  dcs.remove(dir)
end

print('the configuration names every key once')
