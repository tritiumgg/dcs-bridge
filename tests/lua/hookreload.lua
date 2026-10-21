-- The hook driver's reload: Config/DCSBridge.lua read again and every broker
-- key handed over in one more configure. A live key and the log level move,
-- a restart-tier key of the hook driver's is held at the value in force and
-- logged pending, and a file that does not read or is gone leaves the
-- running configuration in force and logs why. ADR 0057, ADR 0058,
-- ADR 0059.
--
-- Each case installs the hook driver into a scratch write directory, starts
-- it against a module stub that records each configure, then rewrites the
-- file and reloads, under a stock Lua 5.1 with lfs and DCS's log stubbed.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookreload.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local hook = require('hook')

local ABC = hook.ABC
local write, install, recording, lines = hook.write, hook.install, hook.recording, hook.lines

-- A hook driver started on the file given, against the recording stub.
-- Returns the driver, its directory and the stub's calls.
local function started(config)
  local driver, dir = install(config)
  local open, calls = recording(driver)
  driver.open = open
  driver.start({ banner = 'the banner' })
  return driver, dir, calls
end

local function rewrite(dir, config)
  write(dir .. 'Config/DCSBridge.lua', config)
end

local function count(t)
  local n = 0
  for _ in pairs(t) do
    n = n + 1
  end
  return n
end

-- A live key and the log level: a second configure carrying every broker
-- key, the new value and the generated hash, and a debug line written after
-- the reload reaches the log.
do
  local driver, dir, calls = started('return { port = 0 }')
  rewrite(dir, "return { port = 0, handshake_timeout_ms = 250, log_level = 'debug' }")
  local ok, answer, pending = driver.reload()
  assert(ok, 'the reload was refused: ' .. tostring(answer))
  assert(type(answer) == 'table' and #pending == 0, 'the reload answered no broker answer')

  assert(#calls == 2, #calls .. ' configure calls, expected 2')
  local handed = calls[2]
  assert(count(handed) == 31, count(handed) .. ' keys handed, expected 30 and schema_sha256')
  assert(handed.handshake_timeout_ms == 250,
    'handshake_timeout_ms was handed as ' .. tostring(handed.handshake_timeout_ms))
  assert(handed.schema_sha256 == ABC, 'schema_sha256 was handed as ' .. tostring(handed.schema_sha256))

  driver.log.debug('after the reload')
  local own = lines(dir)
  assert(own[#own] == 'DEBUG after the reload', 'the level did not move: ' .. tostring(own[#own]))
end

-- A restart-tier key of the broker's is handed over all the same, for the
-- broker to keep in force and report.
do
  local driver, dir, calls = started()
  rewrite(dir, 'return { max_connections = 16 }')
  assert(driver.reload())
  assert(calls[2].max_connections == 16,
    'max_connections was handed as ' .. tostring(calls[2].max_connections))
end

-- The route is the hook driver's and waits for a restart: logged pending
-- with both values, and held at the first, so a reload after it is still
-- pending against stock.
do
  local driver, dir = started()
  rewrite(dir, "return { route = 'modified' }")
  local ok, _, pending = driver.reload()
  assert(ok, 'the reload was refused')
  assert(#pending == 1 and pending[1].key == 'route' and pending[1].effective == 'stock'
    and pending[1].file == 'modified', 'route is not pending against stock')
  local own = lines(dir)
  assert(own[#own] == 'WARN route: effective stock, file modified, waits for a DCS restart',
    'the pending route is not a warning: ' .. tostring(own[#own]))

  ok, _, pending = driver.reload()
  assert(ok and #pending == 1 and pending[1].effective == 'stock',
    'the second reload saw the route as ' .. tostring(pending[1] and pending[1].effective))
end

-- A reload that lowers the hook driver ring is taken, and the broker keeps
-- the ring at 256. A reload after it that lowers the memory to match is
-- checked against that 256, not the 128 the first reload's file named.
-- ADR 0058.
do
  local driver, dir, calls = started()
  rewrite(dir, 'return { ring_in_hook_driver_records = 128 }')
  assert(driver.reload(), 'a lowered ring was refused')
  assert(calls[2].ring_in_hook_driver_records == 128,
    'the ring size was handed as ' .. tostring(calls[2].ring_in_hook_driver_records))

  rewrite(dir, 'return { ring_in_hook_driver_records = 128, recent_admin_keys = 128 }')
  local ok, reason = driver.reload()
  assert(not ok, 'a memory below the ring in force reloaded')
  assert(reason:find('recent_admin_keys is 128, below ring_in_hook_driver_records at 256', 1, true),
    'the reason reads ' .. tostring(reason))
  assert(#calls == 2, #calls .. ' configure calls, expected 2')
end

-- A file that does not read: no second configure, one ERROR line naming the
-- reason, and the level the start set still in force. The last case lowers
-- the hook driver's memory with its ring, which the ring in force refuses.
for _, case in ipairs({
  { "error('x')", 'the configuration raised:' },
  { 'return {', 'does not compile' },
  { 'return 1', 'returned a number, not a table' },
  { "return { route = 'x', log_level = 'debug' }", 'route is x' },
  { "return { ring_in_hook_driver_records = 128, recent_admin_keys = 128, log_level = 'debug' }",
    'recent_admin_keys is 128, below ring_in_hook_driver_records at 256' },
}) do
  local driver, dir, calls = started()
  local before = #lines(dir)
  rewrite(dir, case[1])
  local ok, reason = driver.reload()
  assert(not ok, case[1] .. ' reloaded')
  assert(type(reason) == 'string' and reason:find(case[2], 1, true),
    case[1] .. ': the reason reads ' .. tostring(reason))
  assert(#calls == 1, case[1] .. ': ' .. #calls .. ' configure calls, expected 1')

  driver.log.debug('after the refusal')
  local own = lines(dir)
  assert(#own == before + 1 and own[#own] == 'ERROR ' .. reason,
    case[1] .. ': the log after the refusal reads ' .. tostring(own[#own]))
end

-- A file gone since the start: not the defaults, which would drop every
-- token in force, but no second configure and one ERROR line. ADR 0059.
do
  local driver, dir, calls = started("return { tokens = { { id = 'map', secret = 's', caps = { 'read' } } } }")
  local before = #lines(dir)
  assert(os.remove(dir .. 'Config/DCSBridge.lua'))
  local ok, reason = driver.reload()
  assert(not ok and reason == 'no ' .. dir .. 'Config/DCSBridge.lua: the running configuration stays in force',
    'the reason reads ' .. tostring(reason))
  assert(#calls == 1, #calls .. ' configure calls, expected 1')
  local own = lines(dir)
  assert(#own == before + 1 and own[#own] == 'ERROR ' .. reason,
    'the log after the refusal reads ' .. tostring(own[#own]))
end

-- A table the broker refuses: one ERROR line naming the refusal, and the
-- log level the start set still in force.
do
  local driver, dir = install()
  local open, calls = recording(driver)
  local shim
  driver.open = function()
    shim = open()
    return shim
  end
  driver.start({ banner = 'the banner' })
  local configure = shim.configure
  shim.configure = function(table)
    configure(table)
    error('configure refused: `handshake_timeout_ms` must be a whole number', 0)
  end

  local before = #lines(dir)
  rewrite(dir, "return { handshake_timeout_ms = 0.5, log_level = 'debug', route = 'modified' }")
  local ok, reason = driver.reload()
  assert(not ok and reason == 'configure refused: `handshake_timeout_ms` must be a whole number',
    'the refusal reads ' .. tostring(reason))
  assert(#calls == 2, #calls .. ' configure calls, expected the refused one')
  driver.log.debug('after the refusal')
  local own = lines(dir)
  assert(#own == before + 1 and own[#own] == 'ERROR ' .. reason,
    'the log after the refusal reads ' .. tostring(own[#own]))
end

-- The kill switch follows the file at a reload, with one INFO line each
-- way, and a reload that is refused or finds the file gone leaves it.
-- ADR 0063.
do
  local driver, dir = started('return { enabled = false }')
  rewrite(dir, 'return { enabled = true }')
  assert(driver.reload())
  assert(driver.enabled() == true, 'a reload to enabled = true left the switch off')
  local own = lines(dir)
  assert(own[#own] == 'INFO bridge enabled', 'the last line is ' .. tostring(own[#own]))

  rewrite(dir, 'return { enabled = false }')
  assert(driver.reload())
  assert(driver.enabled() == false, 'a reload to enabled = false left the switch on')
  own = lines(dir)
  assert(own[#own] == 'INFO bridge disabled', 'the last line is ' .. tostring(own[#own]))

  rewrite(dir, 'return { enabled = true, route = 1 }')
  assert(not driver.reload(), 'a file that does not read reloaded')
  assert(driver.enabled() == false, 'a refused reload moved the switch')

  assert(os.remove(dir .. 'Config/DCSBridge.lua'))
  assert(not driver.reload(), 'an absent file reloaded')
  assert(driver.enabled() == false, 'a reload with the file gone moved the switch')
end

-- A reload before the start has nothing to keep in force, and raises.
do
  local driver = install()
  local ran, err = pcall(driver.reload)
  assert(not ran and err:find('before the hook driver started', 1, true),
    'a reload before the start ran: ' .. tostring(err))
end

-- The real module, the only case here that opens it: the broker is one per
-- process. A reload that lowers a ring size and the frame cap answers the
-- same applied count and listener as the start, since applied counts the
-- live keys the table names, reports the ring size pending in the answer
-- and the log, and puts the frame cap in force: a 200-byte string outgrows
-- a 64-byte buffer. load.lua says why both spellings are tried.
do
  local driver, dir = install('return { port = 0 }')
  local shim, first
  driver.open = function()
    for _, name in ipairs({ 'luaopen_dcsbridge', '_luaopen_dcsbridge' }) do
      local opener = package.loadlib(MODULE, name)
      if opener then
        shim = opener()
        local configure = shim.configure
        shim.configure = function(table)
          local answer = configure(table)
          first = first or answer
          return answer
        end
        return shim
      end
    end
    error('no opener in ' .. MODULE, 0)
  end
  driver.start({ banner = 'the banner' })

  rewrite(dir, 'return { port = 0, ring_out_lossy_records = 64, max_frame_bytes = 64 }')
  local ok, answer = driver.reload()
  assert(ok, 'the reload was refused: ' .. tostring(answer))
  assert(answer.applied == first.applied,
    'the reload applied ' .. tostring(answer.applied) .. ', the start ' .. tostring(first.applied))
  assert(answer.listening == first.listening, 'the reload rebound the listener')
  assert(answer.pending_restart == 1 and answer.pending[1].key == 'ring_out_lossy_records'
    and answer.pending[1].effective == 3584 and answer.pending[1].file == 64,
    'the ring size is not pending')
  local own = lines(dir)
  assert(own[#own] == 'WARN ring_out_lossy_records: effective 3584, file 64, waits for a DCS restart',
    'the pending ring size is not a warning: ' .. tostring(own[#own]))
  assert(own[#own - 1]:find(', pending_restart 1$'), 'the answer is not logged: ' .. tostring(own[#own - 1]))

  shim.classes({ ['dcsbridge.builtin.sim.UnitDestroyed'] = 'durable' })
  shim.caps({ ['dcsbridge.builtin.sim.UnitDestroyed'] = 'read' })
  shim.begin('dcsbridge.builtin.sim.UnitDestroyed')
  local put, err = pcall(shim.string, 1, string.rep('x', 200))
  assert(not put and tostring(err):find('outgrew its buffer', 1, true),
    'the lowered frame cap is not in force: ' .. tostring(err))
end

hook.remove()

print('reload hands the broker the file again and keeps the running one on a refusal')
