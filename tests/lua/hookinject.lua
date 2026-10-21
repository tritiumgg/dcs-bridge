-- Sim driver injection: the stub states a chunk runs in, and what crosses
-- into them.
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

hook.remove()
