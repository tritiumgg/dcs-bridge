-- The hook driver's reach into DCS: every call through one accessor, which
-- never raises. ADR 0060.
--
-- Each case installs the hook driver into a scratch write directory, under a
-- stock Lua 5.1 with lfs, DCS's log and the DCS namespace stubbed.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookcallbacks.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local dcs = require('dcs')
local hook = require('hook')

local install = hook.install

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

hook.remove()
