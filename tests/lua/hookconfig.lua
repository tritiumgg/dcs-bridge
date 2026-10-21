-- The hook driver's configuration vocabulary: every key the file may name,
-- once, with a default and one of three owners, and the broker's 30 among
-- them. ADR 0050. And the reader: an absent file is the defaults, a file's
-- keys overlay them, and a file that does not read gives the reason.
-- ADR 0051.
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

-- A file holding text, in a scratch directory of its own.
local function file(text)
  local dir = dcs.scratch()
  scratch[#scratch + 1] = dir
  local path = dir .. 'DCSBridge.lua'
  local f = assert(io.open(path, 'wb'))
  f:write(text)
  f:close()
  return path
end

-- Each value the reader gave matches its row's default, except the names in
-- changed, which hold the value given there.
local function only(driver, values, changed)
  for _, row in ipairs(driver.config.KEYS) do
    local name, default = row[1], row[2]
    local want = changed[name]
    if want == nil then
      if type(default) == 'table' then
        assert(type(values[name]) == 'table' and next(values[name]) == nil,
          name .. ' is not an empty table')
        assert(values[name] ~= default, name .. ' shares the default table')
      else
        assert(values[name] == default, name .. ' is ' .. tostring(values[name]))
      end
    else
      assert(values[name] == want, name .. ' is ' .. tostring(values[name]))
    end
  end
end

-- Absent: every default, and one INFO line saying so.
do
  local driver, dir = fresh()
  local path = dir .. 'Config/DCSBridge.lua'
  local values, notes = driver.config.read(path)
  assert(values, 'an absent file did not read: ' .. tostring(notes))
  only(driver, values, {})
  assert(#notes == 1 and notes[1].level == 'info'
    and notes[1].message == 'no ' .. path .. ', defaults in force',
    'the absent file was not noted once at info')
end

-- One key given: that key moves and no other does, and the file is noted.
do
  local driver = fresh()
  local path = file('return { port = 1 }')
  local values, notes = driver.config.read(path)
  assert(values, 'the file did not read: ' .. tostring(notes))
  only(driver, values, { port = 1 })
  assert(#notes == 1 and notes[1].level == 'info' and notes[1].message == 'read ' .. path,
    'the read was not noted once at info')
end

-- A key no row names: a WARN line naming it, and the key is gone.
do
  local driver = fresh()
  local values, notes = driver.config.read(file('return { prot = 1, port = 2 }'))
  assert(values, 'the file did not read: ' .. tostring(notes))
  assert(values.prot == nil, 'an unknown key was kept')
  only(driver, values, { port = 2 })
  assert(#notes == 2 and notes[2].level == 'warn'
    and notes[2].message:find('names prot, which is not a key', 1, true),
    'the unknown key was not warned about')
end

-- The file reaches no global: os is nil inside it.
do
  local driver = fresh()
  local values = assert(driver.config.read(file('return { port = os and 1 or 2 }')))
  assert(values.port == 2, 'the file saw os')
end

-- A file that does not read gives nil and says why.
for _, case in ipairs({
  { 'return {', 'does not compile' },
  { 'local t; return t.port', 'raised' },
  { 'return 1', 'returned a number, not a table' },
  { "return { port = 'x' }", 'port is a string, and it takes a number' },
  { 'return { tokens = true }', 'tokens is a boolean, and it takes a table' },
}) do
  local driver = fresh()
  local values, reason = driver.config.read(file(case[1]))
  assert(values == nil, case[1] .. ' read')
  assert(type(reason) == 'string' and reason:find(case[2], 1, true),
    case[1] .. ': the reason reads ' .. tostring(reason))
end

-- A compile error names the file, as loadfile's would.
do
  local driver = fresh()
  local path = file('return {')
  local _, reason = driver.config.read(path)
  assert(reason:find(path, 1, true), 'the compile error does not name the file: ' .. reason)
end

-- A leading UTF-8 byte-order mark, as some Windows editors write, is dropped.
do
  local driver = fresh()
  local values, notes = driver.config.read(file('\239\187\191return { port = 1 }'))
  assert(values, 'a file with a byte-order mark did not read: ' .. tostring(notes))
  assert(values.port == 1, 'the file after the byte-order mark was not read')
end

-- A file that is there and will not open is not absent: nil and why. Root
-- opens it anyway, so the case is skipped there.
do
  local id = io.popen('id -u')
  local root = id:read('*l') == '0'
  id:close()
  if not root then
    local driver = fresh()
    local path = file('return { port = 1 }')
    assert(os.execute("chmod 000 '" .. path .. "'") == 0)
    local values, reason = driver.config.read(path)
    os.execute("chmod 644 '" .. path .. "'")
    assert(values == nil, 'a file that will not open read as the defaults')
    assert(reason:find('does not open', 1, true) and reason:find(path, 1, true),
      'the reason reads ' .. tostring(reason))
  end
end

-- The checks the broker cannot make: each refusal names the key, and a pair
-- names both of its keys.
for _, case in ipairs({
  { "return { route = 'A' }", 'route is A, and it takes stock or modified' },
  { "return { route = 'Stock' }", 'route is Stock' },
  { "return { log_level = 'verbose' }", 'log_level is verbose' },
  { "return { hook_driver_disabled_files = { 1 } }", 'hook_driver_disabled_files is not a list' },
  { "return { sim_driver_disabled_files = { x = 'a' } }", 'sim_driver_disabled_files is not a list' },
  { 'return { eval_stable_polls = 1.5 }', 'eval_stable_polls is 1.5' },
  { 'return { max_spots = -1 }', 'max_spots is -1' },
  { 'return { max_spots = 1/0 }', 'max_spots is inf' },
  { 'return { max_spots = 0/0 }', 'nan, and it takes a whole number from 0' },
  { 'return { recent_admin_keys = 100 }', 'recent_admin_keys is 100, below ring_in_hook_driver_records at 256' },
  { 'return { ring_in_sim_driver_records = 2048 }', 'recent_idempotency_keys is 1024, below ring_in_sim_driver_records at 2048' },
  { 'return { max_subscriptions = 65 }', 'subscription_max_evals is 64, below max_subscriptions at 65' },
  { 'return { hook_driver_dispatch_max_commands = 0 }',
    'hook_driver_dispatch_max_commands is 0, and it takes a whole number from 1' },
}) do
  local driver = fresh()
  local values, reason = driver.config.read(file(case[1]))
  assert(values == nil, case[1] .. ' read')
  assert(type(reason) == 'string' and reason:find(case[2], 1, true),
    case[1] .. ': the reason reads ' .. tostring(reason))
end

-- What passes: the other route, a lower level, a named file list, both
-- sides of a pair raised together, and the defaults themselves.
do
  local driver = fresh()
  local values, notes = driver.config.read(file([[
    return {
      route = 'modified',
      log_level = 'debug',
      hook_driver_disabled_files = { 'a.lua', 'b.lua' },
      max_subscriptions = 128,
      subscription_max_evals = 128,
      hook_driver_dispatch_max_commands = 1,
    }
  ]]))
  assert(values, 'a valid file was refused: ' .. tostring(notes))
  assert(values.route == 'modified' and values.max_subscriptions == 128)
  assert(values.hook_driver_dispatch_max_commands == 1, 'a cap of 1 was not read')
end
do
  local driver, dir = fresh()
  local values = assert(driver.config.read(dir .. 'none'))
  assert(driver.config.validate(values) == nil, 'the defaults do not pass')
end

-- With a ring size in force that is not the file's, a pair is checked
-- against the one in force: a memory lowered with its ring reads alone,
-- and is refused while the ring stays at 256. The values read are the
-- file's either way. ADR 0058.
do
  local driver = fresh()
  local path = file('return { ring_in_hook_driver_records = 128, recent_admin_keys = 128 }')
  local values = assert(driver.config.read(path))
  assert(values.ring_in_hook_driver_records == 128, 'the file value was not read')

  local refused, reason = driver.config.read(path, { ring_in_hook_driver_records = 256 })
  assert(refused == nil, 'a memory below the ring in force read')
  assert(reason:find('recent_admin_keys is 128, below ring_in_hook_driver_records at 256', 1, true),
    'the reason reads ' .. tostring(reason))

  values = assert(driver.config.read(file('return { ring_in_hook_driver_records = 64 }'),
    { ring_in_hook_driver_records = 256 }))
  assert(values.ring_in_hook_driver_records == 64, 'the value in force replaced the file value')
end

-- Split by owner: the broker's table holds its 30 keys and no table but
-- tokens, and each other key sits under its own owner.
do
  local driver, dir = fresh()
  local split = driver.config.split(assert(driver.config.read(dir .. 'none')))
  local n = 0
  for name, value in pairs(split.broker) do
    n = n + 1
    assert(type(value) ~= 'table' or name == 'tokens', 'the broker is handed a table under ' .. name)
  end
  assert(n == 30, n .. ' broker keys, expected 30')
  assert(type(split.broker.tokens) == 'table', 'the broker has no tokens list')
  assert(type(split.sim.options) == 'table' and split.sim.max_spots == 16, 'a sim key is not under sim')
  assert(split.hook.route == 'stock' and split.hook.log_level == 'info', 'a hook key is not under hook')
  assert(split.broker.route == nil and split.broker.options == nil, 'the broker is handed a key it does not own')
end

-- The hook driver's restart-tier keys against the ones running: a changed
-- route is reported with both values and held at the running one, a
-- changed log_level is live and moves, and the same file list in a new
-- table is no change. ADR 0057.
do
  local driver, dir = fresh()
  local running = driver.config.split(assert(driver.config.read(dir .. 'none'))).hook
  running.hook_driver_disabled_files = { 'a.lua', 'b.lua' }
  local file = driver.config.split(assert(driver.config.read(dir .. 'none'))).hook
  file.route = 'modified'
  file.log_level = 'debug'
  file.hook_driver_disabled_files = { 'a.lua', 'b.lua' }

  local pending, held = driver.config.pending(running, file)
  assert(#pending == 1, #pending .. ' keys pending, expected route alone')
  assert(pending[1].key == 'route' and pending[1].effective == 'stock'
    and pending[1].file == 'modified', 'route is not reported with both values')
  assert(held.route == 'stock', 'route moved to ' .. tostring(held.route))
  assert(held.log_level == 'debug', 'log_level did not move')
  assert(file.route == 'modified', 'the file values were changed')

  file.hook_driver_disabled_files = { 'a.lua' }
  pending, held = driver.config.pending(running, file)
  assert(#pending == 2 and pending[2].key == 'hook_driver_disabled_files',
    'a shorter file list is not reported')
  assert(#held.hook_driver_disabled_files == 2, 'the file list moved')

  -- One name holding a comma joins as the two names do, and is a change.
  file.hook_driver_disabled_files = { 'a.lua, b.lua' }
  pending = driver.config.pending(running, file)
  assert(#pending == 2 and pending[2].key == 'hook_driver_disabled_files',
    'one name with a comma compared as two names')
end

for _, dir in ipairs(scratch) do
  dcs.remove(dir)
end

print('the configuration names every key once, reads the file over the defaults and checks it')
