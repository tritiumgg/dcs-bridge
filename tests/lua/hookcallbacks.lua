-- The hook driver's reach into DCS: every call through one accessor, which
-- never raises, and one callback table registered at start, before the
-- configure that binds, whose callbacks run no body until start finishes.
-- A body that raises returns nothing to DCS and is logged once per name
-- until the next mission load, to DCSBridge.log alone. ADR 0060, ADR 0061,
-- ADR 0062.
--
-- Each case installs the hook driver into a scratch write directory, under a
-- stock Lua 5.1 with lfs, DCS's log and the DCS namespace stubbed, and runs
-- the callbacks DCS was handed the way DCS would.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookcallbacks.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local dcs = require('dcs')
local hook = require('hook')

local install, recording, lines = hook.install, hook.recording, hook.lines

-- The accessor returns true and every value the function returns, a
-- trailing nil counted.
do
  local driver = install()
  DCS.answer = function(a, b)
    return a + b, nil
  end
  local n = select('#', driver.dcs.call('answer', 2, 3))
  local ok, sum = driver.dcs.call('answer', 2, 3)
  assert(ok == true and sum == 5, 'the call answered ' .. tostring(ok) .. ', ' .. tostring(sum))
  assert(n == 3, n .. ' values returned, expected true and two results')
  print('ok  a call returns true and its results')
end

-- A raise comes back as false and a reason naming the function and the
-- error, and a value that will not convert to text still gives a reason.
do
  local driver = install()
  DCS.raises = function()
    error('deliberate', 0)
  end
  local ok, reason = driver.dcs.call('raises')
  assert(ok == false, 'a raise answered ' .. tostring(ok))
  assert(reason == 'DCS.raises raised: deliberate', 'the reason was ' .. tostring(reason))

  DCS.raises = function()
    error(setmetatable({}, { __tostring = function() error('again') end }))
  end
  ok, reason = driver.dcs.call('raises')
  assert(ok == false and type(reason) == 'string', 'a raising __tostring answered ' .. tostring(ok))
  print('ok  a raise returns false and a reason')
end

-- The namespace is looked up at each call, so one removed after the driver
-- loaded, or a function it does not hold, is a reason and not a raise.
do
  local driver = install()
  local ok, reason = driver.dcs.call('absent')
  assert(ok == false and reason == 'DCS.absent is not a function',
    'an absent function answered ' .. tostring(ok) .. ', ' .. tostring(reason))
  DCS = nil
  ok, reason = driver.dcs.call('setUserCallbacks', {})
  assert(ok == false and reason == 'no DCS namespace',
    'an absent namespace answered ' .. tostring(ok) .. ', ' .. tostring(reason))
  DCS = dcs.DCS()
  print('ok  an absent namespace or function returns false and a reason')
end

-- A namespace whose __index raises is a reason and not a raise.
do
  local driver = install()
  setmetatable(DCS, { __index = function() error('no such key') end })
  local ok, reason = driver.dcs.call('absent')
  assert(ok == false and reason == 'DCS.absent is not a function',
    'a raising __index answered ' .. tostring(ok) .. ', ' .. tostring(reason))
  DCS = dcs.DCS()
  print('ok  a namespace whose __index raises returns false and a reason')
end

-- No line of the driver names the namespace except the one that holds its
-- name, so no call goes around the accessor.
do
  local path = dcs.root() .. 'lua/' .. hook.DRIVER
  local n = 0
  for line in assert(io.open(path, 'rb')):lines() do
    n = n + 1
    if not line:find('^%s*%-%-') then
      assert(not line:find('DCS%.%a') and not line:find('DCS%['),
        path .. ':' .. n .. ' reaches DCS around the accessor: ' .. line)
    end
  end
  print('ok  the driver reaches DCS through the accessor alone')
end

-- Every callback the bridge uses, in no order.
local NAMES = {
  'onSimulationFrame', 'onSimulationStop', 'onSimulationPause', 'onSimulationResume',
  'onMissionLoadBegin', 'onMissionLoadEnd', 'onPlayerConnect', 'onPlayerDisconnect',
  'onPlayerStart', 'onPlayerStop', 'onPlayerChangeSlot', 'onPlayerChangeCoalition',
  'onPlayerTryConnect', 'onPlayerTryChangeSlot', 'onPlayerTrySendChat', 'onChatMessage',
}

-- A hook driver started against the recording stub. Returns the driver,
-- its directory, the stub's calls and the table DCS was handed.
local function started(config)
  local driver, dir = install(config)
  local open, calls = recording(driver)
  driver.open = open
  driver.start({ banner = 'the banner' })
  assert(#DCS.registered == 1, #DCS.registered .. ' tables registered, expected 1')
  return driver, dir, calls, DCS.registered[1]
end

-- A clean start registers one table holding every callback and no other
-- key, before the configure that binds.
do
  local driver = install()
  local open, calls = recording(driver)
  driver.open = open
  local configured
  local record = DCS.setUserCallbacks
  DCS.setUserCallbacks = function(callbacks)
    configured = #calls
    record(callbacks)
  end
  driver.start({ banner = 'the banner' })

  assert(#DCS.registered == 1, #DCS.registered .. ' tables registered, expected 1')
  assert(configured == 0, 'the callbacks were registered after ' .. tostring(configured) .. ' configure calls')
  assert(#calls == 1, #calls .. ' configure calls, expected 1')
  local callbacks, n = DCS.registered[1], 0
  for name, fn in pairs(callbacks) do
    n = n + 1
    assert(type(fn) == 'function', name .. ' is a ' .. type(fn))
  end
  for _, name in ipairs(NAMES) do
    assert(callbacks[name], name .. ' was not registered')
  end
  assert(n == #NAMES, n .. ' callbacks registered, expected ' .. #NAMES)
  print('ok  a start registers every callback once, before configure')
end

-- A file that does not read stops the start before anything is registered.
do
  local driver = install('return 1')
  driver.open = recording(driver)
  assert(not pcall(driver.start, { banner = 'the banner' }), 'a bad file started')
  assert(#DCS.registered == 0, #DCS.registered .. ' tables registered behind a bad file')
  print('ok  a file that does not read registers nothing')
end

-- A registration DCS refuses stops the start with an ERROR line naming it,
-- and nothing is configured, so nothing binds.
do
  local driver, dir = install()
  local open, calls = recording(driver)
  driver.open = open
  DCS.setUserCallbacks = function()
    error('refused', 0)
  end
  local ok, err = pcall(driver.start, { banner = 'the banner' })
  local expected = 'the callbacks were not registered: DCS.setUserCallbacks raised: refused'
  assert(not ok and err == expected, 'the start answered ' .. tostring(err))
  assert(#calls == 0, #calls .. ' configure calls behind a refused registration')
  local own = lines(dir)
  assert(own[#own] == 'ERROR ' .. expected, 'the last line is ' .. tostring(own[#own]))
  print('ok  a refused registration stops the start before configure')
end

-- A start that stops at configure leaves the table registered, and its
-- callbacks call no body and return nothing.
do
  local driver = install()
  driver.open = function()
    return {
      interface = driver.INTERFACE,
      configure = function()
        error('the broker refused', 0)
      end,
    }
  end
  local ran = false
  driver.callbacks.onSimulationFrame = function()
    ran = true
    return 1
  end
  assert(not pcall(driver.start, { banner = 'the banner' }), 'a refused configure started')
  assert(#DCS.registered == 1, #DCS.registered .. ' tables registered, expected 1')
  local n = select('#', DCS.registered[1].onSimulationFrame(0))
  assert(not ran, 'a body ran behind a start that stopped')
  assert(n == 0, n .. ' values returned behind a start that stopped')
  print('ok  a start that stops after registering calls no body')
end

-- A start that stops at the schema, after configure, leaves the callbacks
-- calling no body too, so the flag is set after the last stop.
do
  local driver = install()
  driver.open = recording(driver, nil, function()
    error('the schema is too large', 0)
  end)
  local ran = false
  driver.callbacks.onSimulationFrame = function()
    ran = true
    return 1
  end
  assert(not pcall(driver.start, { banner = 'the banner' }), 'a refused schema started')
  assert(#DCS.registered == 1, #DCS.registered .. ' tables registered, expected 1')
  local n = select('#', DCS.registered[1].onSimulationFrame(0))
  assert(not ran, 'a body ran behind a start that stopped at the schema')
  assert(n == 0, n .. ' values returned behind a start that stopped at the schema')
  print('ok  a start that stops at the schema calls no body')
end

-- After a clean start a body's values reach DCS exactly, and a name with no
-- body returns nothing, so an onPlayerTry* callback passes the event on.
do
  local driver, _, _, callbacks = started()
  driver.callbacks.onPlayerTryConnect = function(addr, name)
    return false, 'no ' .. name
  end
  local n = select('#', callbacks.onPlayerTryConnect('127.0.0.1', 'pilot', 'ucid', 2))
  local allowed, reason = callbacks.onPlayerTryConnect('127.0.0.1', 'pilot', 'ucid', 2)
  assert(n == 2 and allowed == false and reason == 'no pilot',
    'the callback returned ' .. n .. ' values: ' .. tostring(allowed) .. ', ' .. tostring(reason))
  n = select('#', callbacks.onPlayerTrySendChat(2, 'hello', -1))
  assert(n == 0, n .. ' values returned by a callback with no body')
  print('ok  a body returns its values and a missing body returns nothing')
end

-- A body set after the file loads and before start is the one that runs:
-- the table is created once and start does not replace it.
do
  local driver = install()
  local seen
  driver.callbacks.onSimulationFrame = function(t)
    seen = t
  end
  local open = recording(driver)
  driver.open = open
  driver.start({ banner = 'the banner' })
  DCS.registered[1].onSimulationFrame(42)
  assert(seen == 42, 'the body set before start saw ' .. tostring(seen))
  print('ok  a body set before start is the one called')
end

-- The ERROR lines DCSBridge.log holds.
local function errors(dir)
  local found = {}
  for _, line in ipairs(lines(dir)) do
    if line:find('^ERROR ') then
      found[#found + 1] = line
    end
  end
  return found
end

-- A per-frame raise reaches DCS on no call, writes one line to
-- DCSBridge.log, and writes nothing to dcs.log.
do
  local driver, dir, _, callbacks = started()
  driver.callbacks.onSimulationFrame = function()
    error('deliberate', 0)
  end
  local before = #log.lines
  for _ = 1, 1000 do
    local n = select('#', callbacks.onSimulationFrame())
    assert(n == 0, n .. ' values returned by a raising body')
  end
  local found = errors(dir)
  assert(#found == 1, #found .. ' ERROR lines over 1000 raises, expected 1')
  assert(found[1] == 'ERROR onSimulationFrame raised: deliberate; suppressed until the next mission load',
    'the line was ' .. found[1])
  assert(#log.lines == before, (#log.lines - before) .. ' lines written to dcs.log')
  print('ok  a per-frame raise is contained and logged once, outside dcs.log')
end

-- A raising onPlayerTry* body returns nothing, so the event passes on, and
-- each name is logged once on its own.
do
  local driver, dir, _, callbacks = started()
  driver.callbacks.onPlayerTrySendChat = function()
    error('chat', 0)
  end
  driver.callbacks.onPlayerConnect = function()
    error('connect', 0)
  end
  for _ = 1, 10 do
    local n = select('#', callbacks.onPlayerTrySendChat(2, 'hello', -1))
    assert(n == 0, n .. ' values returned by a raising onPlayerTrySendChat')
    callbacks.onPlayerConnect(2, 'pilot')
  end
  local found = errors(dir)
  assert(#found == 2, #found .. ' ERROR lines for two raising names, expected 2')
  assert(found[1]:find('^ERROR onPlayerTrySendChat raised: chat'), found[1])
  assert(found[2]:find('^ERROR onPlayerConnect raised: connect'), found[2])
  print('ok  a raising onPlayerTry* body returns nothing, and each name logs once')
end

-- A raised value whose __tostring raises still gives one line and nothing
-- reaching DCS.
do
  local driver, dir, _, callbacks = started()
  driver.callbacks.onSimulationFrame = function()
    error(setmetatable({}, { __tostring = function() error('again') end }))
  end
  for _ = 1, 3 do
    local n = select('#', callbacks.onSimulationFrame())
    assert(n == 0, n .. ' values returned by a body raising an unprintable value')
  end
  local found = errors(dir)
  assert(#found == 1, #found .. ' ERROR lines for an unprintable raise, expected 1')
  assert(found[1]:find('^ERROR onSimulationFrame raised: '), found[1])
  print('ok  a raise that does not convert to text is logged once')
end

-- A logger that raises while reporting a raise still leaves nothing
-- reaching DCS.
do
  local driver, _, _, callbacks = started()
  local tried = 0
  driver.log.error = function()
    tried = tried + 1
    error('the log is gone', 0)
  end
  driver.callbacks.onSimulationFrame = function()
    error('deliberate', 0)
  end
  local ok, n = pcall(function()
    return select('#', callbacks.onSimulationFrame())
  end)
  assert(ok, 'a raising logger raised into DCS: ' .. tostring(n))
  assert(n == 0, n .. ' values returned behind a raising logger')
  assert(tried == 1, 'the logger was called ' .. tried .. ' times, expected 1')
  print('ok  a logger that raises while reporting a raise raises nothing')
end

-- The next mission load lets a raise log once more, and only once.
do
  local driver, dir, _, callbacks = started()
  driver.callbacks.onSimulationFrame = function()
    error('deliberate', 0)
  end
  for _ = 1, 1000 do
    callbacks.onSimulationFrame()
  end
  callbacks.onMissionLoadEnd()
  for _ = 1, 1000 do
    callbacks.onSimulationFrame()
  end
  local found = errors(dir)
  assert(#found == 2, #found .. ' ERROR lines across a mission load, expected 2')
  print('ok  a mission load logs a raise once more')
end

hook.remove()
