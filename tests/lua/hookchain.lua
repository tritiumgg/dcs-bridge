-- The hook driver's extension chain: after the schema is handed over, the
-- shipped HookDriver.builtin.lua, then every *.lua in DCSBridge/hookdriver.d/,
-- the *.gen.lua files first and each part in name order, each run as a bare
-- chunk, less the names hook_driver_disabled_files lists. A file that is
-- missing, does not compile or raises is one ERROR line and one count in
-- hook_driver_files_failed_total, the rest load and the start finishes. An
-- absent directory writes nothing. ADR 0070.
--
-- Each case installs the hook driver into a scratch write directory and
-- runs its start against the recording stub, under a stock Lua 5.1 with
-- lfs and DCS's log stubbed. Each chain file appends its name to the global
-- CHAIN, which every case clears before its start.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookchain.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local hook = require('hook')

local install, recording, lines, write = hook.install, hook.recording, hook.lines, hook.write

local D = 'DCSBridge/hookdriver.d/'

-- A chain file that appends name to CHAIN.
local function marks(name)
  return "CHAIN[#CHAIN + 1] = '" .. name .. "'\n"
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

-- Start driver against the recording stub with CHAIN cleared, and return
-- whether a callback body set before the start runs after it, which it
-- does only once the start has finished.
local function start(driver)
  CHAIN = {}
  local open = recording(driver)
  driver.open = open
  local ran = false
  driver.callbacks.onSimulationPause = function()
    ran = true
  end
  driver.start({ banner = 'the banner' })
  DCS.registered[1].onSimulationPause(0)
  driver.callbacks.onSimulationPause = nil
  return ran
end

-- A raising file is skipped with one ERROR line naming it and one count,
-- the files on either side of it load, a *.gen.lua file loads first, and
-- the start finishes.
do
  local driver, dir = install()
  write(dir .. D .. 'a.lua', marks('a'))
  write(dir .. D .. 'b.lua', "error('b raised')\n")
  write(dir .. D .. 'c.lua', marks('c'))
  write(dir .. D .. 'z.gen.lua', marks('z'))
  write(dir .. D .. 'notes.txt', marks('notes'))
  assert(start(driver), 'the start did not finish behind a raising file')

  assert(table.concat(CHAIN, ' ') == 'z a c', 'the chain ran ' .. table.concat(CHAIN, ' '))
  local counted = driver.counters().hook_driver_files_failed_total
  assert(counted == 1, counted .. ' files counted, expected 1')
  local found = errors(dir)
  assert(#found == 1, #found .. ' ERROR lines, expected 1')
  assert(found[1]:find('ERROR extension file skipped: ' .. dir .. D .. 'b.lua: ', 1, true) == 1,
    'the ERROR line was ' .. found[1])
  assert(found[1]:find('b raised', 1, true), 'the ERROR line does not give the reason: ' .. found[1])
  -- The chain runs after the schema is handed over, so its line follows
  -- the schema's.
  local own, schema, skipped = lines(dir), nil, nil
  for i, line in ipairs(own) do
    if line == 'INFO schema ' .. hook.ABC then
      schema = i
    elseif line == found[1] then
      skipped = i
    end
  end
  assert(schema and skipped and schema < skipped,
    'the ERROR line is not after the schema line: ' .. table.concat(own, ' | '))
  print('ok  a raising file is skipped and counted, and the rest load in order')
end

-- An error whose message spans lines is still one ERROR line, holding
-- every part.
do
  local driver, dir = install()
  write(dir .. D .. 'a.lua', "error('a\\nb', 0)\n")
  assert(start(driver), 'the start did not finish')

  local found = errors(dir)
  assert(#found == 1, #found .. ' ERROR lines, expected 1')
  assert(found[1]:find(': a | b', 1, true), 'the ERROR line was ' .. found[1])
  for _, line in ipairs(lines(dir)) do
    assert(line ~= 'b', 'a part of the message has a line of its own')
  end
  print('ok  a multi-line error is one ERROR line')
end

-- The chain loads before the start is marked finished, so a callback a
-- file calls during the load runs no body.
do
  local driver, dir = install()
  write(dir .. D .. 'a.lua', 'DCS.registered[1].onSimulationPause(0)\n'
    .. "CHAIN[#CHAIN + 1] = PAUSED and 'paused' or 'quiet'\n")
  CHAIN, PAUSED = {}, false
  driver.open = recording(driver)
  driver.callbacks.onSimulationPause = function()
    PAUSED = true
  end
  driver.start({ banner = 'the banner' })

  assert(CHAIN[1] == 'quiet', 'a callback during the load ran its body')
  DCS.registered[1].onSimulationPause(0)
  assert(PAUSED, 'the body did not run after the start')
  print('ok  the chain loads before the start is marked finished')
end

-- A file that does not compile is skipped and counted the same way.
do
  local driver, dir = install()
  write(dir .. D .. 'a.lua', 'this is not lua\n')
  write(dir .. D .. 'b.lua', marks('b'))
  assert(start(driver), 'the start did not finish behind a file that does not compile')

  assert(table.concat(CHAIN, ' ') == 'b', 'the chain ran ' .. table.concat(CHAIN, ' '))
  local counted = driver.counters().hook_driver_files_failed_total
  assert(counted == 1, counted .. ' files counted, expected 1')
  local found = errors(dir)
  assert(#found == 1 and found[1]:find('ERROR extension file skipped: ' .. dir .. D .. 'a.lua: ', 1, true) == 1,
    'the ERROR lines were ' .. table.concat(found, ' | '))
  print('ok  a file that does not compile is skipped and counted')
end

-- An absent directory writes what an empty one writes: the start's own
-- lines, the schema line last, nothing at ERROR or WARN, and no count.
do
  local logs = {}
  for _, present in ipairs({ false, true }) do
    local driver, dir = install()
    if present then
      assert(os.execute("mkdir -p '" .. dir .. D .. "'") == 0)
    end
    assert(start(driver), 'the start did not finish')
    local counted = driver.counters().hook_driver_files_failed_total
    assert(counted == 0, counted .. ' files counted, expected 0')
    local own = lines(dir)
    assert(own[#own] == 'INFO schema ' .. hook.ABC, 'the last line is ' .. tostring(own[#own]))
    for i, line in ipairs(own) do
      assert(not line:find('^ERROR ') and not line:find('^WARN '), 'a start wrote ' .. line)
      local at = line:find(dir, 1, true)
      if at then
        line = line:sub(1, at - 1) .. '<dir>' .. line:sub(at + #dir)
      end
      own[i] = line
    end
    logs[#logs + 1] = table.concat(own, '\n')
  end
  assert(logs[1] == logs[2], 'an absent directory wrote\n' .. logs[1] .. '\nan empty one wrote\n' .. logs[2])
  print('ok  an absent directory is silent')
end

-- A name hook_driver_disabled_files lists is dropped from the directory
-- with no line and no count, and the rest load.
do
  local driver, dir = install("return { hook_driver_disabled_files = { 'b.lua' } }")
  write(dir .. D .. 'a.lua', marks('a'))
  write(dir .. D .. 'b.lua', "error('b raised')\n")
  assert(start(driver), 'the start did not finish')

  assert(table.concat(CHAIN, ' ') == 'a', 'the chain ran ' .. table.concat(CHAIN, ' '))
  local counted = driver.counters().hook_driver_files_failed_total
  assert(counted == 0, counted .. ' files counted, expected 0')
  local found = errors(dir)
  assert(#found == 0, 'a disabled file wrote ' .. table.concat(found, ' | '))
  print('ok  a disabled name in the directory is skipped and not counted')
end

-- The shipped built-ins file runs before the directory, ahead of its
-- *.gen.lua files too.
do
  local driver, dir = install()
  write(dir .. hook.BUILTIN, marks('builtin'))
  write(dir .. D .. 'a.gen.lua', marks('a'))
  assert(start(driver), 'the start did not finish')

  assert(table.concat(CHAIN, ' ') == 'builtin a', 'the chain ran ' .. table.concat(CHAIN, ' '))
  assert(driver.counters().hook_driver_files_failed_total == 0, 'a clean chain counted a file')
  print('ok  the built-ins file runs before the directory')
end

-- hook_driver_disabled_files naming the built-ins file skips it with no
-- line and no count.
do
  local driver, dir = install("return { hook_driver_disabled_files = { 'HookDriver.builtin.lua' } }")
  write(dir .. hook.BUILTIN, marks('builtin'))
  write(dir .. D .. 'a.lua', marks('a'))
  assert(start(driver), 'the start did not finish')

  assert(table.concat(CHAIN, ' ') == 'a', 'the chain ran ' .. table.concat(CHAIN, ' '))
  local counted = driver.counters().hook_driver_files_failed_total
  assert(counted == 0, counted .. ' files counted, expected 0')
  local found = errors(dir)
  assert(#found == 0, 'a disabled built-ins file wrote ' .. table.concat(found, ' | '))
  print('ok  a disabled built-ins file is skipped and not counted')
end

-- A missing built-ins file is one ERROR line naming it and one count, the
-- directory still loads, and the start finishes.
do
  local driver, dir = install(nil, { [hook.BUILTIN] = true })
  write(dir .. D .. 'a.lua', marks('a'))
  assert(start(driver), 'the start did not finish without the built-ins file')

  assert(table.concat(CHAIN, ' ') == 'a', 'the chain ran ' .. table.concat(CHAIN, ' '))
  local counted = driver.counters().hook_driver_files_failed_total
  assert(counted == 1, counted .. ' files counted, expected 1')
  local found = errors(dir)
  assert(#found == 1
    and found[1]:find('ERROR extension file skipped: ' .. dir .. hook.BUILTIN .. ': ', 1, true) == 1,
    'the ERROR lines were ' .. table.concat(found, ' | '))
  print('ok  a missing built-ins file is counted and the start finishes')
end

-- counters answers a copy, so a caller cannot move the count.
do
  local driver = install()
  driver.counters().hook_driver_files_failed_total = 9
  assert(driver.counters().hook_driver_files_failed_total == 0, 'the count moved through a copy')
  print('ok  counters answers a copy')
end

hook.remove()
