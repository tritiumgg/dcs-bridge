-- The Modified route's bootstrap. A dofile of the sim driver with nothing
-- handed over reads the sim driver's keys from Config/DCSBridge.lua itself,
-- publishes them under DCSBridge with the modified route and logs one INFO
-- line to dcs.log. It holds package.loadlib and returns an opener over it,
-- which nothing global reaches. A file that does not read is one ERROR line
-- and no DCSBridge. Nothing raises out of the dofile, and nothing reaches
-- net.
-- ADR 0069.
--
-- Each case installs the drivers into a scratch write directory, under a
-- stock Lua 5.1 with lfs and DCS's log stubbed, and runs the sim driver in
-- the stub mission state. Run it through tools/luatest.sh.

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local hook = require('hook')

-- A table that fails the test on any read, standing in for net, so a sim
-- driver that reaches net.dostring_in on this route raises out of the run.
local unreachable = setmetatable({}, {
  __index = function(_, key)
    error('the sim driver reached net.' .. tostring(key), 2)
  end,
})

-- The sim driver run by a bare dofile in the mission state of a fresh
-- install with config as Config/DCSBridge.lua, outside pcall, so a raise
-- fails the test. before, when given, is called with the write directory
-- first. Returns the state, the write directory, the hook driver, and what
-- the dofile returned, packed.
local function run(config, before)
  local driver, dir = hook.install(config)
  if before then
    before(dir)
  end
  local mission = net.states.mission
  local stub = net
  net = unreachable
  local returned = { n = 0 }
  local function pack(...)
    returned = { n = select('#', ...), ... }
  end
  pack(mission.dofile(dir .. hook.SIM))
  net = stub
  return mission, dir, driver, returned
end

-- The lines the sim driver wrote to dcs.log at level, as messages.
local function said(level)
  local found = {}
  for _, line in ipairs(log.lines) do
    assert(line.subsystem == 'DCSBridge', 'a line was written under ' .. tostring(line.subsystem))
    if line.level == level then
      found[#found + 1] = line.message
    end
  end
  return found
end

local function same(a, b)
  if type(a) ~= 'table' or type(b) ~= 'table' then
    return a == b
  end
  for key, value in pairs(a) do
    if not same(value, b[key]) then
      return false
    end
  end
  for key in pairs(b) do
    if a[key] == nil then
      return false
    end
  end
  return true
end

-- A file naming sim, hook, broker and unknown keys gives the sim values,
-- the defaults for every sim key it leaves out, and nothing else. The INFO
-- line names the file and each moved key.
do
  local mission, dir, driver = run([[
    return {
      drain_max_records = 128,
      max_spots = 4,
      sim_driver_disabled_files = { 'off.lua' },
      mission_sim_driver_dirs = true,
      port = 9000,
      log_level = 'debug',
      route = 'modified',
      not_a_key = 1,
    }
  ]])
  local bridge = rawget(mission, 'DCSBridge')
  assert(type(bridge) == 'table', 'the bootstrap published no DCSBridge')
  assert(bridge.grammar == driver.GRAMMAR_VERSION, 'the bootstrap published grammar ' .. tostring(bridge.grammar))
  local load = bridge.load
  assert(load.route == 'modified', 'the load carries route ' .. tostring(load.route))
  assert(load.epoch == nil and load.mission == nil, 'the load carries an epoch or a mission')
  local settings = load.settings
  local n = 0
  for _, row in ipairs(driver.config.KEYS) do
    if row[3] == 'sim' then
      n = n + 1
      local want = row[2]
      if row[1] == 'drain_max_records' then
        want = 128
      elseif row[1] == 'max_spots' then
        want = 4
      elseif row[1] == 'sim_driver_disabled_files' then
        want = { 'off.lua' }
      elseif row[1] == 'mission_sim_driver_dirs' then
        want = true
      end
      assert(same(settings[row[1]], want), row[1] .. ' read as ' .. tostring(settings[row[1]]))
    end
  end
  for _ in pairs(settings) do
    n = n - 1
  end
  assert(n == 0, 'the settings carry a key that is not the sim driver\'s')
  assert(rawget(mission, 'DCSBridge_load') == nil, 'the bootstrap left DCSBridge_load set')
  local info = said('INFO')
  assert(#info == 1 and info[1] == 'the sim driver loaded on the modified route: read '
    .. dir .. 'Config/DCSBridge.lua, drain_max_records 128, max_spots 4, sim_driver_disabled_files,'
    .. ' mission_sim_driver_dirs true', 'the INFO line was ' .. tostring(info[1]))
  assert(#said('ERROR') == 0, 'a bootstrap that loaded logged an ERROR')
  print('ok  a bare dofile reads the sim keys itself and publishes a modified-route load')
end

-- An absent file is the defaults, and the INFO line names no key.
do
  local mission, dir = run(nil)
  local settings = rawget(mission, 'DCSBridge').load.settings
  assert(settings.drain_max_records == 256 and #settings.options == 0, 'an absent file did not give the defaults')
  local info = said('INFO')
  assert(#info == 1 and info[1] == 'the sim driver loaded on the modified route: no '
    .. dir .. 'Config/DCSBridge.lua, defaults in force', 'the INFO line was ' .. tostring(info[1]))
  print('ok  an absent file is the defaults')
end

-- The bootstrap lists simdriver.d itself, in the hook driver's order: the
-- *.lua files alone, *.gen.lua first, each part in name order, full paths,
-- each name in sim_driver_disabled_files dropped. An absent directory is
-- no files.
do
  local d
  local mission = run('return { sim_driver_disabled_files = { "off.lua" } }', function(dir)
    d = dir .. 'DCSBridge/simdriver.d/'
    for _, name in ipairs({ 'b.lua', 'a.lua', 'z.gen.lua', 'notes.txt', 'off.lua', 'c.gen.lua' }) do
      hook.write(d .. name, '')
    end
  end)
  local files = rawget(mission, 'DCSBridge').load.files
  local expected = { 'c.gen.lua', 'z.gen.lua', 'a.lua', 'b.lua' }
  assert(#files == #expected, #files .. ' files listed, expected ' .. #expected)
  for i, name in ipairs(expected) do
    assert(files[i] == d .. name, 'file ' .. i .. ' is ' .. tostring(files[i]) .. ', expected ' .. name)
  end

  mission = run(nil)
  files = rawget(mission, 'DCSBridge').load.files
  assert(type(files) == 'table' and #files == 0, 'an absent directory listed files')
  assert(#said('ERROR') == 0, 'an absent directory logged an ERROR')
  print('ok  the bootstrap lists simdriver.d in order, less the disabled names')
end

-- A byte-order mark is dropped, as the hook driver drops it.
do
  local mission = run('\239\187\191return { drain_max_records = 64 }')
  assert(rawget(mission, 'DCSBridge').load.settings.drain_max_records == 64, 'a file with a BOM did not read')
  print('ok  a byte-order mark is dropped')
end

-- A file that is there and will not open is one ERROR line naming why, and
-- no DCSBridge. Root opens it anyway, so the case is skipped there.
do
  local _, dir = hook.install('return {}')
  local config = dir .. 'Config/DCSBridge.lua'
  assert(os.execute("chmod 000 '" .. config .. "'") == 0, 'chmod failed')
  local mission = net.states.mission
  local returned = { n = select('#', mission.dofile(dir .. hook.SIM)) }
  local f = io.open(config, 'rb')
  if f then
    f:close()
    print('skip  an unreadable file: this user opens a mode 000 file')
  else
    local errors = said('ERROR')
    assert(#errors == 1 and errors[1]:find('does not open', 1, true), 'an unreadable file logged '
      .. tostring(errors[1]))
    assert(rawget(mission, 'DCSBridge') == nil and returned.n == 0, 'an unreadable file loaded')
    print('ok  a file that does not open is one ERROR line and no DCSBridge')
  end
  os.execute("chmod 644 '" .. config .. "'")
end

-- Each file the sim driver refuses, and the reason its ERROR line carries.
local REFUSED = {
  { 'return {', 'does not compile' },
  { 'local t = {} return t.a.b', 'the configuration raised: ' },
  { 'return 7', 'returned a number, not a table' },
  { 'return { drain_max_records = "128" }', 'drain_max_records is a string, and it takes a number' },
  { 'return { mission_sim_driver_dirs = "yes" }', 'mission_sim_driver_dirs is a string, and it takes a boolean' },
  { 'return { options = 1 }', 'options is a number, and it takes a table' },
  { 'return { drain_max_records = 1.5 }', 'drain_max_records is 1.5, and it takes a whole number from 0' },
  { 'return { max_spots = -1 }', 'max_spots is -1, and it takes a whole number from 0' },
  { 'return { max_spots = 1/0 }', ', and it takes a whole number from 0' },
  { 'return { sim_driver_disabled_files = { 1 } }', 'sim_driver_disabled_files is not a list of file names' },
  { 'return { subscription_max_evals = 8 }',
    'subscription_max_evals is 8, below max_subscriptions at 64, and it has to be at least that' },
}

-- A file that does not read logs one ERROR line naming why, publishes no
-- DCSBridge, and returns nothing.
do
  for _, case in ipairs(REFUSED) do
    local mission, _, _, returned = run(case[1])
    local errors = said('ERROR')
    assert(#errors == 1 and errors[1]:find('^the sim driver did not load on the modified route: ')
      and errors[1]:find(case[2], 1, true), case[1] .. ' logged ' .. tostring(errors[1]))
    assert(#said('INFO') == 0, case[1] .. ' logged an INFO line')
    assert(rawget(mission, 'DCSBridge') == nil, case[1] .. ' published DCSBridge')
    assert(returned.n == 0, case[1] .. ' returned a value')
  end
  print('ok  a file that does not read is one ERROR line and no DCSBridge')
end

-- The hook driver's reader and the bootstrap agree on one file: on its sim
-- values when it reads, and on the reason when a sim key refuses it.
do
  local good = 'return { drain_max_records = 128, options = { a = { 1 } }, mission_sim_driver_dirs = true }'
  local mission, dir, driver = run(good)
  local values = assert(driver.config.read(dir .. 'Config/DCSBridge.lua'))
  local sim = driver.config.split(values).sim
  assert(same(rawget(mission, 'DCSBridge').load.settings, sim), 'the two readers read other sim values')
  for _, case in ipairs(REFUSED) do
    mission, dir, driver = run(case[1])
    local read, reason = driver.config.read(dir .. 'Config/DCSBridge.lua')
    assert(read == nil, case[1] .. ' reads in the hook driver')
    local errors = said('ERROR')
    assert(errors[1] == 'the sim driver did not load on the modified route: ' .. reason,
      case[1] .. ' gave ' .. tostring(errors[1]) .. ' and the hook driver ' .. reason)
  end
  print('ok  the hook driver and the bootstrap agree on the sim keys')
end

-- A file the hook driver refuses for a pair across owners, or for a key
-- the sim driver does not own, still loads here. ADR 0069 says why.
do
  for _, config in ipairs({ 'return { recent_idempotency_keys = 8 }', 'return { port = "x" }' }) do
    local mission, dir, driver = run(config)
    assert(driver.config.read(dir .. 'Config/DCSBridge.lua') == nil, config .. ' reads in the hook driver')
    assert(rawget(mission, 'DCSBridge'), config .. ' did not load the sim driver')
  end
  print('ok  a file refused for another owner\'s key still loads the sim driver')
end

-- Whether any value reachable from value is a function.
local function reaches(value, seen)
  if type(value) == 'function' then
    return true
  end
  if type(value) ~= 'table' or seen[value] then
    return false
  end
  seen[value] = true
  for key, inner in pairs(value) do
    if reaches(key, seen) or reaches(inner, seen) then
      return true
    end
  end
  return reaches(getmetatable(value), seen)
end

-- The bootstrap holds package.loadlib when it runs, and the opener it
-- returns reaches that one after the name is replaced, as the sanitization
-- block removes it. The opener is not published: nothing reachable from
-- DCSBridge is a function.
do
  local original = package.loadlib
  local calls = {}
  local module = { opened = true }
  package.loadlib = function(path, name)
    calls[#calls + 1] = { path = path, name = name }
    return function()
      return module
    end
  end
  local mission, dir, _, returned = run(nil)
  package.loadlib = function()
    error('the opener looked package.loadlib up after the bootstrap', 0)
  end
  assert(returned.n == 1 and type(returned[1]) == 'function', 'the bootstrap returned no opener')
  assert(#calls == 0, 'the bootstrap opened the broker module')
  assert(returned[1]() == module, 'the opener did not answer the module table')
  assert(#calls == 1 and calls[1].path == dir .. 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll'
    and calls[1].name == 'luaopen_dcsbridge',
    'the opener asked for ' .. tostring(calls[1] and calls[1].path) .. ' ' .. tostring(calls[1] and calls[1].name))
  assert(not reaches(rawget(mission, 'DCSBridge'), {}), 'a function is reachable from DCSBridge')
  -- DCSBridge is the one name the bootstrap set in the state, beside the
  -- stub's own dofile, so neither loadlib nor the opener is published
  -- under another name.
  for key, value in pairs(mission) do
    if key ~= 'dofile' then
      assert(key == 'DCSBridge', 'the bootstrap set ' .. tostring(key) .. ' in the mission state')
      assert(not reaches(value, {}), 'a function is reachable from ' .. tostring(key))
    end
  end
  assert(rawget(mission, 'package') == nil and rawget(mission, 'loadlib') == nil,
    'the bootstrap republished package.loadlib')

  -- A loadlib that finds nothing raises from the opener, naming the path.
  package.loadlib = function()
    return nil, 'not found'
  end
  mission, dir, _, returned = run(nil)
  package.loadlib = original
  local ok, err = pcall(returned[1])
  assert(not ok and err == 'the broker module did not open: ' .. dir
    .. 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll: not found', 'the opener raised ' .. tostring(err))

  -- With no package.loadlib, nothing loads.
  package.loadlib = nil
  mission, _, _, returned = run(nil)
  package.loadlib = original
  local errors = said('ERROR')
  assert(#errors == 1 and errors[1] == 'the sim driver did not load on the modified route:'
    .. ' package.loadlib is not a function, so the broker module cannot open', 'no loadlib logged ' .. tostring(errors[1]))
  assert(rawget(mission, 'DCSBridge') == nil and returned.n == 0, 'a bootstrap with no loadlib loaded')
  print('ok  the bootstrap holds package.loadlib and returns an opener it does not publish')
end

-- With no lfs the bootstrap logs one ERROR line, and with no log it loads
-- and writes nothing. Neither raises.
do
  local _, dir = hook.install()
  local mission = net.states.mission
  local held = lfs
  lfs = nil
  mission.dofile(dir .. hook.SIM)
  lfs = held
  assert(rawget(mission, 'DCSBridge') == nil, 'a bootstrap with no lfs published DCSBridge')
  assert(#said('ERROR') == 1, 'a bootstrap with no lfs logged no ERROR')

  hook.install()
  mission = net.states.mission
  local dcslog = log
  log = nil
  mission.dofile(dir .. hook.SIM)
  log = dcslog
  assert(rawget(mission, 'DCSBridge').load.route == 'modified', 'a bootstrap with no log did not load')
  print('ok  a missing lfs or log raises nothing')
end

hook.remove()
