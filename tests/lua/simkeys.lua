-- The sim driver's keys are the hook driver's sim rows: the same names,
-- defaults and order, read from both files by the one-row-per-line shape
-- each declares them in. The two drivers never share a Lua state, so the
-- rows are copied, and this is what keeps the copy honest. ADR 0069.
--
-- Reads source text only. Run it through tools/luatest.sh.

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local dcs = require('dcs')

local ROOT = dcs.root() .. 'lua/Mods/services/DCSBridge/lua/'

-- Each row of owner in the file at path, as { name, default text }, in
-- file order.
local function rows(path, owner)
  local found = {}
  for line in assert(io.open(path, 'rb')):lines() do
    local name, default, of = line:match("^%s*{ '([%w_]+)', (.-), '(%w+)' },$")
    if name and of == owner then
      found[#found + 1] = { name, default }
    end
  end
  return found
end

local hook = rows(ROOT .. 'HookDriver.lua', 'sim')
local sim = rows(ROOT .. 'SimDriver.lua', 'sim')

-- The shape reads something in each file, so a reformat that hides every
-- row fails here rather than comparing two empty lists.
do
  assert(#hook == 21, 'the hook driver declares ' .. #hook .. ' sim rows, expected 21')
  assert(#rows(ROOT .. 'SimDriver.lua', 'hook') == 0 and #rows(ROOT .. 'SimDriver.lua', 'broker') == 0,
    'the sim driver declares a key it does not own')
  print('ok  the hook driver declares 21 sim rows, and the sim driver only sim rows')
end

do
  assert(#sim == #hook, 'the sim driver declares ' .. #sim .. ' rows, the hook driver ' .. #hook)
  for i, row in ipairs(hook) do
    assert(sim[i][1] == row[1], 'row ' .. i .. ' is ' .. sim[i][1] .. ' in the sim driver and '
      .. row[1] .. ' in the hook driver')
    assert(sim[i][2] == row[2], row[1] .. ' defaults to ' .. sim[i][2] .. ' in the sim driver and '
      .. row[2] .. ' in the hook driver')
  end
  print('ok  the sim driver\'s keys match the hook driver\'s sim rows')
end
