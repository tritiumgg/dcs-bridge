-- shim.tick, from a stock Lua 5.1.
--
-- The hook driver calls it every frame with the sim's clock. What the
-- broker does with the value, the throttle and the Pong it feeds, is not
-- observable from here: the Rust tests drive the clock and read the stamp.
-- What this checks is the Lua side of the call: it is on the table; before
-- configure it is refused saying so; a string, nil, NaN and an infinity
-- raise rather than being taken as a clock reading; a burst of calls after
-- configure answers the kill switch and raises nothing; and a configure that
-- turns the switch off is answered by the next tick. ADR 0063.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/tick.lua <module path>')

-- load.lua says why both spellings are tried.
local function open(module)
  for _, name in ipairs({ 'luaopen_dcsbridge', '_luaopen_dcsbridge' }) do
    local loader = package.loadlib(module, name)
    if loader then
      return loader()
    end
  end
  error('no opener in ' .. module, 0)
end

local shim = open(path)
assert(type(shim.tick) == 'function', 'shim.tick is missing')

local function raises(what, f, ...)
  local ok, err = pcall(f, ...)
  assert(not ok, what .. ' did not raise')
  return err
end

-- The hook driver's start is configure then the per-frame callback, and
-- a tick before it is refused naming the call that comes first.
local early = raises('tick before configure', shim.tick, 0)
assert(early:find('configure comes first', 1, true), 'the wrong complaint: ' .. early)

-- Port 0 lets the system pick, so the run collides with nothing.
shim.configure({ port = 0 })

-- A string would convert under a looser check, and nil is a clock that
-- was never read. Neither is a mission time.
local str = raises('tick with a string', shim.tick, '12.5')
assert(str:find('number expected', 1, true), 'the wrong complaint: ' .. str)
raises('tick with nothing', shim.tick)
raises('tick with a table', shim.tick, {})

-- NaN and the infinities are numbers to Lua and no time to a consumer.
local nan = raises('tick with NaN', shim.tick, 0 / 0)
assert(nan:find('not a finite number', 1, true), 'the wrong complaint: ' .. nan)
raises('tick with infinity', shim.tick, math.huge)
raises('tick with negative infinity', shim.tick, -math.huge)

-- A frame's worth of calls, and more: each answers the switch, on by
-- default, as one boolean.
for frame = 0, 999 do
  local n = select('#', shim.tick(frame / 60))
  assert(n == 1, 'tick returned ' .. n .. ' values')
  local enabled = shim.tick(frame / 60)
  assert(enabled == true, 'tick answered ' .. tostring(enabled))
end

-- A configure with the switch off is answered by the next tick, and one
-- with it on again by the tick after.
shim.configure({ port = 0, enabled = false })
assert(shim.tick(17) == false, 'a tick after a disable answered on')
shim.configure({ port = 0 })
assert(shim.tick(18) == true, 'a tick after a re-enable answered off')

print('ok  tick is on the table and is refused before configure')
print('ok  a string, nil, a table, NaN and an infinity raise')
print('ok  a burst of ticks after configure answers the switch, on')
print('ok  a configure that turns the switch off and on is the next tick\'s answer')
