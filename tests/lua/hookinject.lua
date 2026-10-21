-- Sim driver injection. At a mission load's end the hook driver takes the
-- switch from a tick, reads the sim driver's grammar from disk, hands it the
-- load's parameters as one literal inside a chunk no larger than
-- bridge_return_max_bytes, and reads the epoch back from the global the sim
-- driver set. A refusal is one logged line and nothing crosses. Each answer
-- the server state gives falls in one case and logs a line of its own.
-- ADR 0063, ADR 0065, ADR 0066.
--
-- Each case installs the hook driver into a scratch write directory, under a
-- stock Lua 5.1 with lfs, DCS's log, the DCS namespace and net stubbed.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookinject.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local dcs = require('dcs')
local hook = require('hook')

local install = hook.install

-- A chunk run in a stub state sets its global there and not in the real
-- global table, and the call is recorded with its answer.
do
  install()
  local answer = net.dostring_in('server', 'injected = 42 return "set"')
  assert(answer == 'set', 'the chunk answered ' .. tostring(answer))
  assert(net.states.server.injected == 42, 'the state holds ' .. tostring(net.states.server.injected))
  assert(rawget(_G, 'injected') == nil, 'the chunk set its global in the real table')
  assert(#net.calls == 1 and net.calls[1].state == 'server' and net.calls[1].answer == 'set',
    'the call was not recorded')
  print('ok  a chunk sets its globals in its own state')
end

-- A file a chunk runs with dofile sets its globals in the same state, and
-- two states do not share one.
do
  local _, dir = install()
  hook.write(dir .. 'fixture.lua', 'fixture = (fixture or 0) + 1')
  local chunk = 'dofile(' .. string.format('%q', dir .. 'fixture.lua') .. ') return fixture'
  assert(net.dostring_in('server', chunk) == 1, 'the server ran the file once')
  assert(net.dostring_in('server', chunk) == 2, 'the server state did not persist')
  assert(net.dostring_in('mission', chunk) == 1, 'the mission state shares the server one')
  assert(rawget(_G, 'fixture') == nil, 'the file set its global in the real table')
  print('ok  a dofile in a state sets its globals in that state alone')
end

-- A state with no name answers nil, a chunk that raises answers its message,
-- and a refusal is the answer to every call.
do
  install()
  assert(net.dostring_in('nowhere', 'return 1') == nil, 'an unknown state answered')
  local answer = net.dostring_in('server', 'error("deliberate", 0)')
  assert(answer == 'deliberate', 'a raise answered ' .. tostring(answer))
  net.refusal = 'refused'
  assert(net.dostring_in('server', 'return 1') == 'refused', 'a refusal did not answer')
  print('ok  an unknown state, a raise and a refusal each answer as DCS does')
end

-- Each of the two drivers declares one grammar line of the shape injection
-- reads, and they declare the same value.
do
  local root = dcs.root() .. 'lua/'
  local found = {}
  for _, path in ipairs({ hook.DRIVER, hook.SIM }) do
    local values = {}
    for line in assert(io.open(root .. path, 'rb')):lines() do
      local value = line:match("^local GRAMMAR_VERSION = '([^']*)'$")
      if value then
        values[#values + 1] = value
      end
    end
    assert(#values == 1, path .. ' declares ' .. #values .. ' grammar lines, expected 1')
    found[#found + 1] = values[1]
  end
  assert(found[1] == found[2], 'the hook driver is grammar ' .. found[1]
    .. ' and the sim driver is grammar ' .. found[2])
  print('ok  both drivers declare one grammar, the same one')
end

-- The sim driver takes the parameters the chunk set, clears their name and
-- publishes them under DCSBridge. Run without them, it raises and sets
-- nothing.
do
  local driver, dir = install()
  local server = net.states.server
  local params = { epoch = 7 }
  server.DCSBridge_load = params
  server.dofile(dir .. hook.SIM)
  assert(server.DCSBridge.load == params, 'the sim driver published ' .. tostring(server.DCSBridge.load))
  assert(server.DCSBridge.grammar == driver.GRAMMAR_VERSION,
    'the sim driver published grammar ' .. tostring(server.DCSBridge.grammar))
  assert(rawget(server, 'DCSBridge_load') == nil, 'the sim driver left DCSBridge_load set')

  local mission = net.states.mission
  local ok, err = pcall(mission.dofile, dir .. hook.SIM)
  assert(not ok and err:find('DCSBridge_load is not a table'), 'a bare dofile answered ' .. tostring(err))
  assert(rawget(mission, 'DCSBridge') == nil, 'a bare dofile published DCSBridge')
  print('ok  the sim driver publishes what the chunk handed it, and needs it')
end

-- Whether two values are the same, tables by their contents.
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

-- The value literal's source compiles back to, with no global in reach.
local function compiled(source)
  local chunk = assert(loadstring('return ' .. source))
  setfenv(chunk, {})
  return chunk()
end

-- A nested table crosses whole: strings whatever bytes they hold, numbers
-- to the last bit, booleans, list and string keys, and a table two keys
-- share.
do
  local driver = install()
  local shared = { 1, 2 }
  local value = {
    quoted = 'a "quote", a \\ and a \'',
    lines = 'one\ntwo\r\nthree',
    zero = 'a\0b',
    high = '\255\128',
    third = 1 / 3,
    big = 2 ^ 53,
    negative = -0.5,
    yes = true,
    no = false,
    list = { 'a', 'b', { deep = { deeper = 'c' } } },
    [1] = 'first',
    [-4] = 'negative key',
    ['not a name'] = 1,
    left = shared,
    right = shared,
  }
  local source = assert(driver.injection.literal(value, 'value'))
  assert(same(compiled(source), value), 'the value did not cross whole: ' .. source)
  print('ok  a nested table crosses as source and compiles back the same')
end

-- One table gives one text, whatever order its keys went in.
do
  local driver = install()
  local a, b = {}, {}
  for i = 1, 50 do
    a['k' .. i] = i
    a[i] = i
  end
  for i = 50, 1, -1 do
    b[i] = i
    b['k' .. i] = i
  end
  assert(driver.injection.literal(a, 'a') == driver.injection.literal(b, 'b'),
    'two insertion orders gave two texts')
  print('ok  one table gives one text')
end

-- A hundred tables nested inside one another cross, and compile inside
-- the chunk's own wrapping. A hundred and one are refused, naming the
-- table past the bound, before the chunk fails to compile.
do
  local driver = install()
  local function nest(n)
    local value = {}
    for _ = 2, n do
      value = { n = value }
    end
    return value
  end
  local source = assert(driver.injection.literal(nest(100), 'options'))
  assert(loadstring('DCSBridge_load = {settings={options=' .. source .. '}}'),
    'a hundred nested tables did not compile inside the chunk')
  local refused, reason = driver.injection.literal(nest(101), 'options')
  local deepest = 'options' .. string.rep('.n', 100)
  assert(refused == nil, 'a hundred and one nested tables crossed')
  assert(reason == deepest .. ' nests more than 100 tables deep, which does not cross as source',
    'the refusal was ' .. tostring(reason))
  print('ok  a table nested past the bound is refused, naming its path')
end

-- A value that does not compile back is refused, naming its path.
do
  local driver = install()
  local cycle = {}
  cycle.inner = { back = cycle }
  local cases = {
    { { builtin = { resync = function() end } }, 'options.builtin.resync is a function' },
    { { [1.5] = 'x' }, 'options has a number key 1.5' },
    { { [true] = 'x' }, 'options has a boolean key true' },
    { { n = 0 / 0 }, 'options.n is ' },
    { { n = math.huge }, 'options.n is inf' },
    { { ['a b'] = { c = -math.huge } }, 'options["a b"].c is -inf' },
    { cycle, 'options.inner.back holds a table that holds it' },
    { { co = coroutine.create(function() end) }, 'options.co is a thread' },
  }
  for _, case in ipairs(cases) do
    local source, reason = driver.injection.literal(case[1], 'options')
    assert(source == nil, 'a value with no literal gave ' .. tostring(source))
    assert(reason:find(case[2], 1, true) == 1,
      'the refusal was ' .. reason .. ', expected ' .. case[2])
  end
  print('ok  a value that does not cross is refused, naming its path')
end

-- The chunk runs the real sim driver in the server state, which publishes
-- the parameters whole, and its answer carries the epoch read back from
-- the sim driver's global. The handoff name is gone after it.
do
  local driver, dir = install()
  local params = {
    epoch = 7,
    mission = 'a "quoted"\nmission',
    settings = { bridge_return_max_bytes = 32768, options = { nested = { 'x' } } },
    files = { dir .. 'DCSBridge/simdriver.d/a.lua' },
  }
  local chunk = assert(driver.injection.chunk(dir .. hook.SIM, params))
  assert(loadstring(chunk), 'the chunk does not compile')
  local answer = net.dostring_in('server', chunk)
  assert(answer == 'OK|loaded 7', 'the chunk answered ' .. tostring(answer))
  local server = net.states.server
  assert(same(server.DCSBridge.load, params), 'the sim driver published other parameters')
  assert(rawget(server, 'DCSBridge_load') == nil, 'the chunk left DCSBridge_load set')
  print('ok  the chunk hands the sim driver its parameters and reads the epoch back')
end

-- A sim driver that raises answers its error after the failed: prefix, and
-- one that sets no global answers no epoch. Neither leaves the handoff name.
do
  local driver, dir = install()
  hook.sim(dir, 'error("deliberate", 0)')
  local chunk = assert(driver.injection.chunk(dir .. hook.SIM, { epoch = 1 }))
  local answer = net.dostring_in('server', chunk)
  assert(answer == 'OK|failed: deliberate', 'a raising sim driver answered ' .. tostring(answer))
  assert(rawget(net.states.server, 'DCSBridge_load') == nil, 'a raise left DCSBridge_load set')

  hook.sim(dir, 'local quiet = DCSBridge_load')
  answer = net.dostring_in('mission', chunk)
  assert(answer == 'OK|loaded nil', 'a sim driver that set nothing answered ' .. tostring(answer))
  assert(rawget(net.states.mission, 'DCSBridge_load') == nil, 'the chunk left DCSBridge_load set')
  print('ok  a raise answers failed, and a sim driver that sets nothing answers no epoch')
end

-- Each answer the server state can give falls in one case, with the detail
-- its log line carries. A value that is not a string has no prefix.
do
  local driver = install()
  local shown = {}
  local cases = {
    { 'OK|loaded 7', 'loaded', '7' },
    { 'OK|loaded nil', 'epoch', 'nil' },
    { 'OK|failed: boom', 'failed', 'boom' },
    { 'OK|x', 'unknown', 'x' },
    { nil, 'nil', 'nil' },
    { 'Invalid state name', 'invalid', 'Invalid state name' },
    { 'anything else', 'noprefix', 'anything else' },
    { shown, 'noprefix', tostring(shown) },
  }
  for i = 1, #cases do
    local answer, case, detail = cases[i][1], cases[i][2], cases[i][3]
    local got, said = driver.injection.answer(answer, 7)
    assert(got == case and said == detail, tostring(answer) .. ' fell in ' .. tostring(got)
      .. ' with ' .. tostring(said) .. ', expected ' .. case .. ' with ' .. detail)
  end
  print('ok  each answer falls in its own case')
end

-- A value in the parameters that does not cross refuses the chunk, naming
-- where it is.
do
  local driver, dir = install()
  local chunk, reason = driver.injection.chunk(dir .. hook.SIM,
    { epoch = 1, settings = { options = { builtin = { resync = print } } } })
  assert(chunk == nil, 'a function crossed')
  assert(reason:find('settings.options.builtin.resync is a function', 1, true) == 1,
    'the refusal was ' .. tostring(reason))
  print('ok  a value that does not cross refuses the chunk')
end

-- The extension list: *.lua files only, *.gen.lua first, each part in name
-- order, full paths, a disabled name dropped.
do
  local driver, dir = install()
  local d = dir .. 'DCSBridge/simdriver.d/'
  for _, name in ipairs({ 'b.lua', 'a.lua', 'z.gen.lua', 'notes.txt', 'off.lua', 'c.gen.lua' }) do
    hook.write(d .. name, '')
  end
  local files = driver.injection.files(d, { 'off.lua', 'SimDriver.builtin.lua' })
  local expected = { 'c.gen.lua', 'z.gen.lua', 'a.lua', 'b.lua' }
  assert(#files == #expected, #files .. ' files listed, expected ' .. #expected)
  for i, name in ipairs(expected) do
    assert(files[i] == d .. name, 'file ' .. i .. ' is ' .. tostring(files[i]) .. ', expected ' .. name)
  end
  print('ok  the extension list is ordered and drops a disabled name')
end

-- An absent directory, or one that does not list, is no files.
do
  local driver, dir = install()
  local files = driver.injection.files(dir .. 'DCSBridge/simdriver.d/', {})
  assert(#files == 0, #files .. ' files listed from an absent directory')
  lfs.dir = function()
    error('denied', 0)
  end
  files = driver.injection.files(dir, {})
  assert(#files == 0, #files .. ' files listed from a directory that does not list')
  print('ok  an absent or unlistable directory is no files')
end

-- A hook driver started on the file given, against the recording stub.
-- Returns the driver, its directory and the stub's ticks.
local function started(config, omit)
  local driver, dir = install(config, omit)
  local open, _, ticks = hook.recording(driver)
  driver.open = open
  driver.start({ banner = 'the banner' })
  return driver, dir, ticks
end

-- The lines DCSBridge.log holds that match pattern.
local function matching(dir, pattern)
  local found = {}
  for _, line in ipairs(hook.lines(dir)) do
    if line:find(pattern) then
      found[#found + 1] = line
    end
  end
  return found
end

-- A sim driver whose grammar is not the hook driver's, that declares none,
-- or that is not there is refused with one ERROR line, and no chunk.
do
  local cases = {
    { "local GRAMMAR_VERSION = '2'\n",
      'the sim driver declares grammar 2, and this hook driver is grammar 1: not injected' },
    { "local version = '1'\n", ' declares no GRAMMAR_VERSION: not injected' },
    { false, 'the sim driver does not open: ' },
  }
  for _, case in ipairs(cases) do
    local driver, dir = started(nil, { [hook.SIM] = true })
    if case[1] then
      hook.sim(dir, case[1])
    end
    local chunk, reason = driver.injection.prepare(1, 'mission')
    assert(chunk == nil, 'a sim driver with the wrong grammar was prepared')
    assert(reason:find(case[2], 1, true), 'the refusal was ' .. tostring(reason))
    local found = matching(dir, '^ERROR ')
    assert(#found == 1 and found[1] == 'ERROR ' .. reason,
      #found .. ' ERROR lines, the first ' .. tostring(found[1]))
  end
  print('ok  a sim driver of another grammar, of none, or absent is refused')
end

-- An options table past bridge_return_max_bytes refuses the chunk with one
-- ERROR line naming its size and the bound. Under a larger bound the same
-- table is prepared whole.
do
  local pad = string.rep('x', 40000)
  local driver, dir = started('return { options = { pad = "' .. pad .. '" } }')
  local chunk, reason = driver.injection.prepare(1, 'mission')
  assert(chunk == nil, 'an oversize chunk was prepared')
  local size = reason:match('^the injection chunk is (%d+) bytes, over bridge_return_max_bytes 32768: not injected$')
  assert(size and tonumber(size) > 40000, 'the refusal was ' .. tostring(reason))
  local found = matching(dir, '^ERROR ')
  assert(#found == 1 and found[1] == 'ERROR ' .. reason, #found .. ' ERROR lines')

  driver = started('return { bridge_return_max_bytes = 65536, options = { pad = "' .. pad .. '" } }')
  chunk = assert(driver.injection.prepare(1, 'mission'))
  assert(#chunk == tonumber(size), 'the chunk under a larger bound is ' .. #chunk .. ' bytes, not ' .. size)
  assert(chunk:find(pad, 1, true), 'the chunk under a larger bound does not carry the options whole')
  print('ok  an oversize chunk is refused and logged, and a larger bound carries it whole')
end

-- The bound is inclusive: a chunk exactly bridge_return_max_bytes long is
-- prepared, and one byte under it refuses. The options are padded so the
-- bound has five digits whichever value it takes, since the chunk carries
-- the bound and its length moves with the digits.
do
  local pad = 'options = { pad = "' .. string.rep('x', 20000) .. '" }'
  local driver = started('return { bridge_return_max_bytes = 65536, ' .. pad .. ' }')
  local size = #assert(driver.injection.prepare(1, 'mission'))
  assert(size >= 10001 and size < 65536, 'the chunk is ' .. size .. ' bytes')

  driver = started('return { bridge_return_max_bytes = ' .. size .. ', ' .. pad .. ' }')
  local chunk = assert(driver.injection.prepare(1, 'mission'))
  assert(#chunk == size, 'the chunk at the bound is ' .. #chunk .. ' bytes, not ' .. size)

  local dir
  driver, dir = started('return { bridge_return_max_bytes = ' .. (size - 1) .. ', ' .. pad .. ' }')
  local refused, reason = driver.injection.prepare(1, 'mission')
  assert(refused == nil, 'a chunk one byte over the bound was prepared')
  assert(reason == 'the injection chunk is ' .. size .. ' bytes, over bridge_return_max_bytes '
    .. (size - 1) .. ': not injected', 'the refusal was ' .. tostring(reason))
  assert(#matching(dir, '^ERROR ') == 1, 'the refusal was not logged once')
  print('ok  a chunk at the bound is prepared, and one byte over is refused')
end

-- A name in sim_driver_disabled_files is not in the list the prepared
-- chunk hands the sim driver.
do
  local driver, dir = started('return { sim_driver_disabled_files = { "off.lua" } }')
  local d = dir .. 'DCSBridge/simdriver.d/'
  hook.write(d .. 'off.lua', '')
  hook.write(d .. 'a.lua', '')
  local chunk = assert(driver.injection.prepare(1, 'mission'))
  assert(net.dostring_in('server', chunk) == 'OK|loaded 1', 'the prepared chunk did not load')
  local files = net.states.server.DCSBridge.load.files
  assert(#files == 1 and files[1] == d .. 'a.lua', 'the files were ' .. table.concat(files, ', '))
  print('ok  a disabled extension file is not handed to the sim driver')
end

-- A function nested in options, which the file check does not reach, is
-- refused naming where it is.
do
  local driver, dir = started('return { options = { builtin = { resync = function() end } } }')
  local chunk, reason = driver.injection.prepare(1, 'mission')
  assert(chunk == nil, 'a function was prepared')
  assert(reason:find('^settings%.options%.builtin%.resync is a function'), 'the refusal was ' .. reason)
  assert(#matching(dir, '^ERROR settings%.options%.builtin%.resync') == 1, 'the refusal was not logged')
  print('ok  a nested function in options is refused naming its path')
end

-- A mission load's end ticks the broker and injects the sim driver into
-- the server state with epoch 1, the mission's name, every sim-driver key
-- and the extension list, and logs the epoch read back. The next load
-- carries epoch 2.
do
  local driver, dir, ticks = started()
  hook.write(dir .. 'DCSBridge/simdriver.d/a.lua', '')
  DCS.mission = 'Caucasus test'
  DCS.registered[1].onMissionLoadEnd()
  assert(#ticks == 1, #ticks .. ' ticks at a mission load, expected 1')
  assert(#net.calls == 1 and net.calls[1].state == 'server',
    #net.calls .. ' injections, the first into ' .. tostring(net.calls[1] and net.calls[1].state))
  local load = net.states.server.DCSBridge.load
  assert(load.epoch == 1 and load.mission == 'Caucasus test', 'the load carried epoch '
    .. tostring(load.epoch) .. ' and mission ' .. tostring(load.mission))
  local n = 0
  for _, row in ipairs(driver.config.KEYS) do
    if row[3] == 'sim' then
      n = n + 1
      assert(same(load.settings[row[1]], row[2]), row[1] .. ' crossed as ' .. tostring(load.settings[row[1]]))
    end
  end
  for _ in pairs(load.settings) do
    n = n - 1
  end
  assert(n == 0, 'the settings carried a key that is not the sim driver\'s')
  assert(#load.files == 1 and load.files[1] == dir .. 'DCSBridge/simdriver.d/a.lua',
    'the files were ' .. table.concat(load.files, ', '))
  local found = matching(dir, '^INFO injected ')
  assert(#found == 1 and found[1] == 'INFO injected ' .. dir .. hook.SIM .. ': OK|loaded 1',
    'the injection line was ' .. tostring(found[1]))
  assert(#matching(dir, '^ERROR ') == 0, 'an injection that loaded logged an ERROR')

  DCS.registered[1].onMissionLoadEnd()
  assert(net.states.server.DCSBridge.load.epoch == 2, 'the second load was not epoch 2')
  assert(#matching(dir, ': OK|loaded 2$') == 1, 'the second load did not read epoch 2 back')
  print('ok  a mission load injects the sim driver and reads its epoch back')
end

-- inject answers true only when the epoch reads back. A sim driver that
-- sets no global is one ERROR line, and the load is not counted.
do
  local driver, dir = started()
  local loaded, name = driver.inject(5)
  assert(loaded == true and name == 'loaded', 'an injection that read back 5 answered '
    .. tostring(loaded) .. ' and ' .. tostring(name))
  hook.sim(dir, "local GRAMMAR_VERSION = '1'\nDCSBridge_load = nil\n")
  loaded, name = driver.inject(6)
  assert(loaded == false and name == 'epoch', 'an injection that read nothing back answered '
    .. tostring(loaded) .. ' and ' .. tostring(name))
  local found = matching(dir, '^ERROR ')
  assert(#found == 1 and found[1] == 'ERROR the sim driver set no epoch: sent 6, read back 5: not loaded',
    'the read-back line was ' .. tostring(found[1]))
  print('ok  a sim driver that sets no epoch is not counted as loaded')
end

-- A tick at the load's end that answers off is the switch for that load:
-- nothing crosses, with no frame in between, and the skip is one line.
do
  local driver, dir, ticks = started()
  ticks.answer = false
  DCS.registered[1].onMissionLoadEnd()
  assert(driver.enabled() == false, 'the load did not take the switch from its tick')
  assert(#net.calls == 0, #net.calls .. ' injections while disabled')
  assert(#matching(dir, '^INFO injection skipped: bridge disabled$') == 1, 'the skip was not logged')
  print('ok  a load disabled by its own tick injects nothing')
end

-- A clock that does not read at the load's end is one ERROR line, and the
-- load injects on the switch it held.
do
  local _, dir, ticks = started()
  DCS.getModelTime = nil
  DCS.registered[1].onMissionLoadEnd()
  assert(#ticks == 0, 'a tick was taken with no clock')
  local found = matching(dir, '^ERROR ')
  assert(#found == 1 and found[1] == 'ERROR DCS.getModelTime is not a function; the switch stays on',
    'the clock line was ' .. tostring(found[1]))
  assert(#net.calls == 1, #net.calls .. ' injections on the held switch, expected 1')
  print('ok  a load with no clock injects on the switch it held')
end

-- A clock that answers nil or NaN at the load's end is a clock that does
-- not read: one ERROR line, no tick, and the load injects.
do
  for _, answer in ipairs({ { nil }, { 0 / 0 } }) do
    local _, dir, ticks = started()
    DCS.getModelTime = function()
      return answer[1]
    end
    DCS.registered[1].onMissionLoadEnd()
    assert(#ticks == 0, 'a tick was taken with a time of ' .. tostring(answer[1]))
    local found = matching(dir, '^ERROR ')
    assert(#found == 1 and found[1] == 'ERROR DCS.getModelTime answered ' .. tostring(answer[1])
      .. ', not a finite number; the switch stays on', 'the clock line was ' .. tostring(found[1]))
    assert(#net.calls == 1, #net.calls .. ' injections behind a time of ' .. tostring(answer[1]))
  end
  print('ok  a load whose clock answers no finite number injects on the switch it held')
end

-- The modified route loads the sim driver itself, so nothing crosses.
do
  local _, dir = started('return { route = "modified" }')
  DCS.registered[1].onMissionLoadEnd()
  assert(#net.calls == 0, #net.calls .. ' injections on the modified route')
  local found = matching(dir, '^INFO injection skipped')
  assert(#found == 1 and found[1] == 'INFO injection skipped: route is modified, where the sim driver loads itself',
    'the skip line was ' .. tostring(found[1]))
  print('ok  the modified route injects nothing')
end

-- A mission name that does not read refuses the injection with one ERROR.
do
  local _, dir = started()
  DCS.getMissionName = function()
    error('no mission', 0)
  end
  DCS.registered[1].onMissionLoadEnd()
  assert(#net.calls == 0, #net.calls .. ' injections with no mission name')
  local found = matching(dir, '^ERROR ')
  assert(#found == 1 and found[1]
    == 'ERROR the mission name does not read: DCS.getMissionName raised: no mission: not injected',
    'the line was ' .. tostring(found[1]))
  print('ok  a mission name that does not read refuses the injection')
end

-- Each answer that is not a load writes one ERROR line of its own, and
-- inject answers false and the case. The sim driver that fails to load is
-- told apart from an answer with no prefix, and nothing raises into DCS.
do
  local cases = {
    { 'nil', function()
      net.states.server = nil
    end, 'ERROR injection answered nil: no Lua state named server: not injected' },
    { 'invalid', function()
      net.refusal = 'Invalid state name'
    end, 'ERROR injection refused: the server state is not available: not injected' },
    { 'noprefix', function()
      net.refusal = 'not allowed'
    end, 'ERROR injection answered with no OK| prefix, refused or unreachable: not allowed: not injected' },
    { 'unknown', function()
      net.refusal = 'OK|something'
    end, 'ERROR the injection chunk answered an unknown body: something: not loaded' },
    { 'failed', function(dir)
      hook.sim(dir, "local GRAMMAR_VERSION = '1'\nerror('deliberate', 0)\n")
    end, 'ERROR the sim driver failed to load: deliberate: not loaded' },
  }
  for _, case in ipairs(cases) do
    local driver, dir = started()
    case[2](dir)
    local loaded, name = driver.inject(1)
    assert(loaded == false and name == case[1], 'the ' .. case[1] .. ' answer gave '
      .. tostring(loaded) .. ' and ' .. tostring(name))
    assert(#net.calls == 1, #net.calls .. ' injections for the ' .. case[1] .. ' answer, expected 1')
    local found = matching(dir, '^ERROR ')
    assert(#found == 1 and found[1] == case[3], 'the ' .. case[1] .. ' line was ' .. tostring(found[1]))
    assert(#matching(dir, 'injection returned') == 0, 'the ' .. case[1] .. ' answer was logged as it came')
    assert(#matching(dir, 'raised') == 0, 'the ' .. case[1] .. ' answer raised')
  end
  print('ok  each answer that is not a load logs its own line and answers its case')
end

-- A call that gets no answer names why: a skipped load, a refusal before
-- the call, a net or dostring_in that is not there, or a call that raised.
-- Each call that did not run is one ERROR line, and nothing raises into
-- DCS.
do
  local driver = started('return { route = "modified" }')
  local loaded, name = driver.inject(1)
  assert(loaded == false and name == 'skipped', 'the modified route gave ' .. tostring(name))

  driver = started('return { enabled = false }')
  loaded, name = driver.inject(1)
  assert(loaded == false and name == 'skipped', 'a disabled bridge gave ' .. tostring(name))

  driver = started()
  DCS.getMissionName = nil
  loaded, name = driver.inject(1)
  assert(loaded == false and name == 'unprepared', 'no mission name gave ' .. tostring(name))

  local dir
  driver, dir = started()
  hook.sim(dir, "local GRAMMAR_VERSION = '2'\n")
  loaded, name = driver.inject(1)
  assert(loaded == false and name == 'unprepared', 'a grammar mismatch gave ' .. tostring(name))

  local cases = {
    { 'absent', function()
      net = nil
    end, 'ERROR injection did not run: no net namespace' },
    { 'absent', function()
      net.dostring_in = nil
    end, 'ERROR injection did not run: net.dostring_in is not a function' },
    { 'unreached', function()
      net.dostring_in = function()
        error('deliberate', 0)
      end
    end, 'ERROR injection did not run: net.dostring_in raised: deliberate' },
  }
  for _, case in ipairs(cases) do
    driver, dir = started()
    case[2]()
    loaded, name = driver.inject(1)
    assert(loaded == false and name == case[1], case[3] .. ' gave ' .. tostring(name))
    local found = matching(dir, '^ERROR ')
    assert(#found == 1 and found[1] == case[3], 'the ' .. case[1] .. ' line was ' .. tostring(found[1]))
  end
  print('ok  a call with no answer names why, and one that did not run is one ERROR')
end

hook.remove()
