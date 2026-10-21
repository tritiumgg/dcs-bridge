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

hook.remove()
