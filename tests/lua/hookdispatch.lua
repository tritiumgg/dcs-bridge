-- The hook driver's dispatch loop: each call pops the hook driver ring and
-- hands each record, decoded, to its topic's handler, at most
-- hook_driver_dispatch_max_commands of them. One record past the cap is
-- held for the next call, dispatched first, and counted once in
-- hook_driver_dispatch_deferred_total, so a ring that held exactly the cap
-- counts nothing. A topic with no handler or decoder, a record that does
-- not decode and a handler that raises drop the record with one ERROR line
-- per topic until the next mission load, and the loop goes on. ReloadConfig
-- reaches driver.reload, and is the one topic a disabled bridge runs.
-- ADR 0062, ADR 0063, ADR 0071.
--
-- Each case installs the hook driver into a scratch write directory and
-- starts it against the recording stub, whose poll pops a list the case
-- fills, under a stock Lua 5.1 with lfs, DCS's log and the DCS namespace
-- stubbed. The counter is not published yet, so it is read through
-- driver.counters().
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookdispatch.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local hook = require('hook')

local install, recording, lines = hook.install, hook.recording, hook.lines

local RELOAD = 'dcsbridge.hook.ReloadConfig'

-- A ReloadConfig record whose idempotency key is key, from connection 7.
local function record(key)
  return { 7, RELOAD, '\10' .. string.char(#key) .. key }
end

-- A hook driver started on the file given, against the recording stub,
-- with a handler on RELOAD that records each key it is handed. Returns the
-- driver, its directory, the ring and the keys seen.
local function started(config)
  local driver, dir = install(config)
  local open, _, _, _, ring = recording(driver)
  driver.open = open
  driver.start({ banner = 'the banner' })
  local seen = {}
  driver.handlers[RELOAD] = function(decoded, conn, topic)
    assert(conn == 7 and topic == RELOAD, 'the handler was handed ' .. tostring(conn) .. ', ' .. tostring(topic))
    seen[#seen + 1] = decoded.idempotency_key
  end
  return driver, dir, ring, seen
end

local function fill(ring, from, to)
  for i = from, to do
    ring[#ring + 1] = record(tostring(i))
  end
end

local function deferred(driver)
  return driver.counters().hook_driver_dispatch_deferred_total
end

local function keys(seen, from, to)
  assert(#seen == to - from + 1, #seen .. ' records dispatched, expected ' .. (to - from + 1))
  for i = from, to do
    assert(seen[i - from + 1] == tostring(i),
      'record ' .. (i - from + 1) .. ' was ' .. tostring(seen[i - from + 1]) .. ', expected ' .. i)
  end
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

-- The counter starts at zero, and an empty ring dispatches and counts
-- nothing after one poll.
do
  local driver, _, ring, seen = started()
  assert(deferred(driver) == 0, 'the counter started at ' .. tostring(deferred(driver)))
  driver.dispatch()
  assert(#seen == 0 and deferred(driver) == 0 and ring.polls == 1,
    'an empty ring dispatched ' .. #seen .. ' over ' .. ring.polls .. ' polls')
  print('ok  an empty ring dispatches and counts nothing')
end

-- Exactly the cap: every record dispatches in order and nothing is counted,
-- since the extra pop found nothing waiting.
do
  local driver, _, ring, seen = started()
  fill(ring, 1, 8)
  driver.dispatch()
  keys(seen, 1, 8)
  assert(deferred(driver) == 0, 'a ring that held the cap counted ' .. deferred(driver))
  assert(ring.polls == 9, ring.polls .. ' polls, expected the cap and one more')
  print('ok  a ring that holds exactly the cap dispatches it and counts nothing')
end

-- One past the cap: eight dispatch, the ninth is held and counted once, and
-- the next call dispatches it first and counts nothing more.
do
  local driver, _, ring, seen = started()
  fill(ring, 1, 9)
  driver.dispatch()
  keys(seen, 1, 8)
  assert(#ring == 0, #ring .. ' records left in the ring, expected the ninth held')
  assert(deferred(driver) == 1, 'one past the cap counted ' .. deferred(driver))
  ring[#ring + 1] = record('10')
  driver.dispatch()
  keys(seen, 1, 10)
  assert(deferred(driver) == 1, 'a call under the cap counted ' .. (deferred(driver) - 1))
  print('ok  one record past the cap is held for the next call and counted once')
end

-- A long burst: each call dispatches the cap, the held record first, and
-- counts one deferral, until the ring is empty.
do
  local driver, _, ring, seen = started()
  fill(ring, 1, 17)
  driver.dispatch()
  driver.dispatch()
  keys(seen, 1, 16)
  assert(deferred(driver) == 2, 'two calls over the cap counted ' .. deferred(driver))
  driver.dispatch()
  keys(seen, 1, 17)
  assert(deferred(driver) == 2, 'the last call counted ' .. (deferred(driver) - 2))
  print('ok  a burst dispatches the cap per call, in order')
end

-- The cap is the file's.
do
  local driver, _, ring, seen = started('return { hook_driver_dispatch_max_commands = 2 }')
  fill(ring, 1, 3)
  driver.dispatch()
  keys(seen, 1, 2)
  driver.dispatch()
  keys(seen, 1, 3)
  assert(deferred(driver) == 1, 'a cap of 2 over 3 records counted ' .. deferred(driver))
  print('ok  the cap is the file\'s')
end

-- A topic with no handler, and one with a handler and no decoder: each
-- record is dropped, each topic writes one ERROR line, and the records
-- behind them still dispatch. A dropped record takes its place under the
-- cap, so nine records are lifted to a cap of nine.
do
  local driver, dir, ring, seen = started('return { hook_driver_dispatch_max_commands = 9 }')
  driver.handlers['dcsbridge.hook.Undecoded'] = function()
    error('a record with no decoder reached its handler', 0)
  end
  for i = 1, 3 do
    ring[#ring + 1] = { 7, 'dcsbridge.hook.Absent', '' }
    ring[#ring + 1] = { 7, 'dcsbridge.hook.Undecoded', '' }
    ring[#ring + 1] = record(tostring(i))
  end
  driver.dispatch()
  keys(seen, 1, 3)
  local found = errors(dir)
  assert(#found == 2, #found .. ' ERROR lines for two dropped topics, expected 2')
  assert(found[1] == 'ERROR dcsbridge.hook.Absent has no handler: dropped;'
    .. ' suppressed until the next mission load', found[1])
  assert(found[2] == 'ERROR dcsbridge.hook.Undecoded has no decoder in HookDriver.gen.lua:'
    .. ' dropped; suppressed until the next mission load', found[2])
  print('ok  a topic with no handler or decoder is dropped and logged once')
end

-- A handler that raises, and a record that does not decode: one ERROR line
-- for the topic, the loop goes on, and the next mission load lets the
-- topic log once more.
do
  local driver, dir, ring = started()
  local calls = 0
  driver.handlers[RELOAD] = function()
    calls = calls + 1
    error('deliberate', 0)
  end
  fill(ring, 1, 3)
  ring[#ring + 1] = { 7, RELOAD, '\10\5a' }
  driver.dispatch()
  assert(calls == 3, calls .. ' handler calls, expected 3')
  local found = errors(dir)
  assert(#found == 1, #found .. ' ERROR lines for a raising topic, expected 1')
  assert(found[1] == 'ERROR ' .. RELOAD .. ' raised: deliberate; suppressed until the next mission load',
    found[1])

  DCS.registered[1].onMissionLoadEnd()
  local before = #errors(dir)
  fill(ring, 4, 5)
  driver.dispatch()
  found = errors(dir)
  assert(#found == before + 1 and found[#found]:find(' raised: deliberate;', 1, true),
    'a mission load did not let the topic log once more: ' .. tostring(found[#found]))
  print('ok  a raising handler or a bad record is dropped and logged once per mission load')
end

-- A poll that raises raises out of the loop, for the callback to contain.
do
  local driver, _, ring = started()
  ring.raise = 'poll refused: the bridge is gone'
  local ran, err = pcall(driver.dispatch)
  assert(not ran and err == ring.raise, 'a raising poll answered ' .. tostring(err))
  print('ok  a poll that raises raises out of the loop')
end

-- A topic beside ReloadConfig, whose decoder a case adds to the installed
-- HookDriver.gen.lua so its records reach their handler.
local OTHER = 'dcsbridge.hook.Other'

-- A hook driver started on the file given with its own handlers, against
-- the recording stub, and with a handler on OTHER that counts its calls.
-- Returns the driver, its directory, the stub's configure calls, the ring
-- and the count.
local function shipped(config)
  local driver, dir = install(config)
  local gen = dir .. hook.GEN
  hook.write(gen, assert(hook.read(gen)):gsub('\nreturn gen\n$',
    "\ngen.decode['" .. OTHER .. "'] = function() return {} end\nreturn gen\n"))
  local open, calls, _, _, ring = recording(driver)
  driver.open = open
  driver.start({ banner = 'the banner' })
  local other = { calls = 0 }
  driver.handlers[OTHER] = function()
    other.calls = other.calls + 1
  end
  return driver, dir, calls, ring, other
end

-- The lines DCSBridge.log holds after the first n.
local function since(dir, n)
  local own, found = lines(dir), {}
  for i = n + 1, #own do
    found[#found + 1] = own[i]
  end
  return found
end

-- ReloadConfig, dispatched, reads the file again: a live key reaches the
-- broker and the answer is logged, and a file that raises changes nothing
-- and logs one ERROR. hookreload.lua holds the rest of what a reload does.
-- ADR 0071.
do
  local driver, dir, calls, ring = shipped('return { port = 0 }')
  hook.write(dir .. 'Config/DCSBridge.lua', 'return { port = 0, handshake_timeout_ms = 250 }')
  local before = #lines(dir)
  ring[1] = record('a')
  driver.dispatch()
  assert(#calls == 2, #calls .. ' configure calls, expected the reload\'s')
  assert(calls[2].handshake_timeout_ms == 250,
    'handshake_timeout_ms was handed as ' .. tostring(calls[2].handshake_timeout_ms))
  local own = since(dir, before)
  assert(own[#own] and own[#own]:find('^INFO configured: '), 'the reload logged ' .. tostring(own[#own]))

  hook.write(dir .. 'Config/DCSBridge.lua', 'local broken = nil + 1')
  before = #lines(dir)
  ring[1] = record('b')
  driver.dispatch()
  assert(#calls == 2, 'a file that raises was configured')
  own = since(dir, before)
  assert(#own == 1 and own[1]:find('^ERROR the configuration raised: .*arithmetic'), 'a file that raises logged ' .. tostring(own[1]))
  print('ok  ReloadConfig reloads the file, and a file that raises changes nothing')
end

-- A disabled bridge discards every topic but ReloadConfig, with one WARN
-- per topic, and runs a ReloadConfig that turns it back on. The next record
-- of the discarded topic then runs, and a later disable lets the topic log
-- once more. ADR 0063, ADR 0071.
do
  local driver, dir, _, ring, other = shipped('return { enabled = false }')
  local before = #lines(dir)
  ring[1] = { 7, OTHER, '' }
  ring[2] = { 7, OTHER, '' }
  driver.dispatch()
  assert(other.calls == 0, 'a disabled bridge ran ' .. other.calls .. ' records')
  local own = since(dir, before)
  assert(#own == 1 and own[1] == 'WARN ' .. OTHER .. ' discarded: bridge disabled',
    'two discarded records logged ' .. #own .. ' lines: ' .. tostring(own[1]))

  hook.write(dir .. 'Config/DCSBridge.lua', 'return { enabled = true }')
  ring[1] = record('a')
  ring[2] = { 7, OTHER, '' }
  driver.dispatch()
  assert(driver.enabled() == true, 'ReloadConfig did not reach a disabled bridge')
  assert(other.calls == 1, 'the record after the re-enable ran ' .. other.calls .. ' times')

  hook.write(dir .. 'Config/DCSBridge.lua', 'return { enabled = false }')
  before = #lines(dir)
  ring[1] = record('b')
  ring[2] = { 7, OTHER, '' }
  driver.dispatch()
  own = since(dir, before)
  assert(own[#own] == 'WARN ' .. OTHER .. ' discarded: bridge disabled',
    'a topic discarded again after a re-enable logged ' .. tostring(own[#own]))
  assert(other.calls == 1, 'a disabled bridge ran a record')
  print('ok  a disabled bridge runs ReloadConfig alone and discards the rest, logged once a topic')
end

hook.remove()
