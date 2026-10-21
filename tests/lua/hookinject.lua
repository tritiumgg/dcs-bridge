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

hook.remove()
