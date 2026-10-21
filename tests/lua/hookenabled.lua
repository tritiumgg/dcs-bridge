-- The kill switch in the hook driver: the per-frame body reads the clock,
-- ticks the broker on every frame whether the bridge is enabled or not, and
-- takes the switch from the tick's answer, so a disable and a re-enable
-- arrive with no reload and no command path. Each move logs one INFO line.
-- A module that answers nothing leaves the switch where it was, and a clock
-- that does not read is one contained ERROR and no tick. A policy refusal's
-- latch holds the bridge off under any switch. ADR 0063, ADR 0067.
--
-- Each case installs the hook driver into a scratch write directory, under a
-- stock Lua 5.1 with lfs, DCS's log and the DCS namespace stubbed, starts it
-- and calls onSimulationFrame the way DCS would. One case opens the real
-- module, and it is the only one: the broker is one per process.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookenabled.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local hook = require('hook')

local install, recording, lines, write = hook.install, hook.recording, hook.lines, hook.write

-- A hook driver started on the file given, against the recording stub.
-- Returns the driver, its directory, the stub's ticks and the
-- onSimulationFrame DCS was handed.
local function started(config)
  local driver, dir = install(config)
  local open, _, ticks = recording(driver)
  driver.open = open
  driver.start({ banner = 'the banner' })
  return driver, dir, ticks, DCS.registered[1].onSimulationFrame
end

-- The lines DCSBridge.log holds that match pattern.
local function matching(dir, pattern)
  local found = {}
  for _, line in ipairs(lines(dir)) do
    if line:find(pattern) then
      found[#found + 1] = line
    end
  end
  return found
end

-- Every frame ticks once with the model time DCS reads, and an enabled
-- bridge that stays enabled logs nothing about it.
do
  local driver, dir, ticks, frame = started()
  for i = 1, 100 do
    DCS.time = i / 60
    frame()
  end
  assert(#ticks == 100, #ticks .. ' ticks over 100 frames')
  for i = 1, 100 do
    assert(ticks[i] == i / 60, 'tick ' .. i .. ' carried ' .. tostring(ticks[i]))
  end
  assert(driver.enabled() == true, 'an enabled bridge read off')
  assert(#matching(dir, 'bridge %a+abled') == 0, 'a frame that changed nothing logged')
  print('ok  every frame ticks with the model time')
end

-- A tick that answers off disables the bridge with one line, and the frames
-- after it still tick. One that answers on again re-enables it with one line,
-- with no reload and nothing dispatched in between.
do
  local driver, dir, ticks, frame = started()
  ticks.answer = false
  for _ = 1, 100 do
    frame()
  end
  assert(driver.enabled() == false, 'a tick answering off left the switch on')
  assert(#ticks == 100, #ticks .. ' ticks over 100 disabled frames')
  local found = matching(dir, 'bridge %a+abled')
  assert(#found == 1 and found[1] == 'INFO bridge disabled',
    #found .. ' switch lines over 100 disabled frames: ' .. tostring(found[1]))

  ticks.answer = true
  for _ = 1, 100 do
    frame()
  end
  assert(driver.enabled() == true, 'a tick answering on left the switch off')
  assert(#ticks == 200, #ticks .. ' ticks over 200 frames')
  found = matching(dir, 'bridge %a+abled')
  assert(#found == 2 and found[2] == 'INFO bridge enabled',
    #found .. ' switch lines after a re-enable: ' .. tostring(found[2]))
  print('ok  a disable and a re-enable arrive by the tick, each logged once')
end

-- A tick that answers nothing, as a module older than the answer does,
-- leaves the switch at the file's value either way and logs nothing.
for _, case in ipairs({ { 'return {}', true }, { 'return { enabled = false }', false } }) do
  local driver, dir, ticks, frame = started(case[1])
  local before = #matching(dir, 'bridge %a+abled')
  ticks.answer = nil
  for _ = 1, 10 do
    frame()
  end
  assert(#ticks == 10, #ticks .. ' ticks over 10 frames')
  assert(driver.enabled() == case[2], case[1] .. ': a tick answering nothing moved the switch')
  assert(#matching(dir, 'bridge %a+abled') == before, case[1] .. ': a tick answering nothing logged')
end
print('ok  a tick that answers nothing leaves the switch where it was')

-- A clock that does not read is one contained ERROR per mission load, no
-- tick, and nothing raised into DCS.
do
  local driver, dir, ticks, frame = started()
  DCS.getModelTime = nil
  for _ = 1, 100 do
    local ok, n = pcall(function()
      return select('#', frame())
    end)
    assert(ok and n == 0, 'a frame with no clock raised or returned: ' .. tostring(n))
  end
  assert(#ticks == 0, #ticks .. ' ticks with no clock')
  assert(driver.enabled() == true, 'a frame with no clock moved the switch')
  local found = matching(dir, '^ERROR ')
  assert(#found == 1 and found[1] == 'ERROR onSimulationFrame raised: '
    .. 'DCS.getModelTime is not a function; suppressed until the next mission load',
    #found .. ' ERROR lines with no clock: ' .. tostring(found[1]))
  print('ok  a clock that does not read is one ERROR and no tick')
end

-- A latched hook driver stays off whatever moves the switch: a tick that
-- answers on and a reload of a file that says enabled. The switch moves
-- under it, and its line says the latch holds. The latch is logged once,
-- and a second latch keeps the first reason. ADR 0067.
do
  local driver, dir, ticks, frame = started()
  driver.latch('the first reason')
  driver.latch('the second reason')
  assert(driver.enabled() == false, 'a latched hook driver read on')
  local found = matching(dir, 'until DCS restarts')
  assert(#found == 1 and found[1] == 'INFO bridge disabled until DCS restarts: the first reason',
    #found .. ' latch lines: ' .. tostring(found[1]))

  for _ = 1, 100 do
    frame()
  end
  assert(driver.enabled() == false, 'a tick answering on cleared the latch')
  ticks.answer = false
  frame()
  ticks.answer = true
  frame()
  assert(driver.enabled() == false, 'a tick answering off then on cleared the latch')

  write(dir .. 'Config/DCSBridge.lua', 'return { enabled = true }')
  assert(driver.reload())
  assert(driver.enabled() == false, 'a reload to enabled = true cleared the latch')

  found = matching(dir, 'until DCS restarts')
  assert(#found == 3
    and found[2] == 'INFO bridge disabled, but refused until DCS restarts'
    and found[3] == 'INFO bridge enabled, but refused until DCS restarts',
    #found .. ' lines naming the latch, expected the latch, off and on')
  assert(#matching(dir, '^ERROR ') == 0, 'a latched hook driver logged an ERROR')
  print('ok  a latch holds the hook driver off whatever moves the switch')
end

-- The real module. The file starts the bridge off and the first frame's tick
-- agrees. A configure the hook driver did not make, standing in for a
-- SetEnabled, which only a socket sends, turns it on at the next frame with
-- no reload; a reload puts the file's off back. load.lua says why both
-- spellings are tried.
do
  local driver, dir = install('return { port = 0, enabled = false }')
  local shim
  driver.open = function()
    for _, name in ipairs({ 'luaopen_dcsbridge', '_luaopen_dcsbridge' }) do
      local opener = package.loadlib(MODULE, name)
      if opener then
        shim = opener()
        return shim
      end
    end
    error('no opener in ' .. MODULE, 0)
  end
  driver.start({ banner = 'the banner' })
  local frame = DCS.registered[1].onSimulationFrame
  DCS.time = 1
  frame()
  assert(driver.enabled() == false, 'a file with enabled = false ticked on')

  shim.configure({ port = 0, enabled = true })
  DCS.time = 2
  frame()
  assert(driver.enabled() == true, 'the broker\'s on did not reach the next frame')

  write(dir .. 'Config/DCSBridge.lua', 'return { port = 0, enabled = false }')
  assert(driver.reload())
  DCS.time = 3
  frame()
  assert(driver.enabled() == false, 'a reload to enabled = false ticked on')
  local found = matching(dir, 'bridge %a+abled')
  assert(#found == 3 and found[1] == 'INFO bridge disabled' and found[2] == 'INFO bridge enabled'
    and found[3] == 'INFO bridge disabled',
    #found .. ' switch lines from the real module, expected off, on, off')
  assert(#matching(dir, '^ERROR ') == 0, 'a frame against the real module raised')
  print('ok  the real module\'s tick answers the switch the broker holds')
end

hook.remove()
