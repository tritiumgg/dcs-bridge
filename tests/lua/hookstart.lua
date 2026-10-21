-- The hook driver's start: the banner first, then Config/DCSBridge.lua read
-- and its log keys handed to the logger, so every line after the banner is
-- written at the file's level, then the broker module opened, its interface
-- compared with the driver's, the generated file and schema.pb read, the
-- module handed every broker key and the generated hash in one call, the
-- schema after it, and the generated tables after that. A file that does
-- not read stops the start before the module opens, a module from another
-- build or a missing generated file or schema stops it before configure,
-- and a refused registration stops it after. ADR 0048, ADR 0050, ADR 0051,
-- ADR 0054, ADR 0055, ADR 0056, ADR 0072.
--
-- Each case installs the hook driver into a scratch write directory and
-- runs its start directly, under a stock Lua 5.1 with lfs and DCS's log
-- stubbed. Every case replaces the driver's open, so no module needs to sit
-- at the shipped path. One case opens the real module, and it is the only
-- one: the broker is one per process, so only the first configure in this
-- process binds.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookstart.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local hook = require('hook')

local DLL, GEN, SCHEMA, ABC = hook.DLL, hook.GEN, hook.SCHEMA, hook.ABC
local read, write, install, recording, lines =
  hook.read, hook.write, hook.install, hook.recording, hook.lines

-- Absent: the banner, the defaults, and one configure carrying exactly the
-- broker's keys, each at its default, and the generated file's hash.
do
  local driver, dir = install()
  local open, calls = recording(driver)
  driver.open = open
  driver.start({ banner = 'the banner' })

  assert(#calls == 1, #calls .. ' configure calls, expected 1')
  local handed, n = calls[1], 0
  for _ in pairs(handed) do
    n = n + 1
  end
  assert(n == 31, n .. ' keys handed to the broker, expected 30 and schema_sha256')
  assert(handed.schema_sha256 == ABC, 'schema_sha256 was handed as ' .. tostring(handed.schema_sha256))
  for _, row in ipairs(driver.config.KEYS) do
    local name, default, owner = row[1], row[2], row[3]
    if owner ~= 'broker' then
      assert(handed[name] == nil, 'the broker was handed ' .. name)
    elseif name == 'tokens' then
      assert(type(handed.tokens) == 'table' and next(handed.tokens) == nil,
        'tokens is not an empty list')
    else
      assert(handed[name] == default, name .. ' was handed as ' .. tostring(handed[name]))
    end
  end

  local own = lines(dir)
  assert(own[1] == 'INFO the banner', 'the first line is not the banner: ' .. tostring(own[1]))
  assert(own[2] == 'INFO no ' .. dir .. 'Config/DCSBridge.lua, defaults in force',
    'the second line is not the defaults: ' .. tostring(own[2]))
  assert(own[3] == 'INFO configured: interface ' .. driver.INTERFACE
    .. ', applied 0, unknown 0, pending_restart 0',
    'the third line is not the answer: ' .. tostring(own[3]))
  assert(own[4] == 'INFO schema ' .. ABC, 'the fourth line is not the hash: ' .. tostring(own[4]))
end

-- The file's broker values reach the broker, tokens with them, and its hook
-- values stay out of the broker's table.
do
  local driver = install([[
    return {
      port = 7743,
      tokens = { { id = 'map', secret = 'map-secret', caps = { 'read' } } },
      route = 'modified',
    }
  ]])
  local open, calls = recording(driver)
  driver.open = open
  driver.start({ banner = 'the banner' })

  local handed = calls[1]
  assert(handed.port == 7743, 'port was handed as ' .. tostring(handed.port))
  local token = handed.tokens[1]
  assert(#handed.tokens == 1 and token.id == 'map' and token.secret == 'map-secret'
    and token.caps[1] == 'read', 'the token did not arrive as written')
  assert(handed.route == nil, 'the broker was handed route')
end

-- A key the broker left pending: the answer's count, then one WARN naming
-- the key, the value in force and the file's, before the schema's line.
do
  local driver, dir = install()
  driver.open = recording(driver, nil, nil, {
    interface = driver.INTERFACE,
    applied = 19,
    unknown = 0,
    pending_restart = 1,
    pending = { { key = 'ring_out_lossy_records', effective = 3584, file = 64 } },
  })
  driver.start({ banner = 'the banner' })
  local own = lines(dir)
  assert(own[3] == 'INFO configured: interface ' .. driver.INTERFACE
    .. ', applied 19, unknown 0, pending_restart 1',
    'the third line is not the answer: ' .. tostring(own[3]))
  assert(own[4] == 'WARN ring_out_lossy_records: effective 3584, file 64, waits for a DCS restart',
    'the pending key is not a warning: ' .. tostring(own[4]))
  assert(own[5] == 'INFO schema ' .. ABC, 'the fifth line is not the hash: ' .. tostring(own[5]))
end

-- A file at error: the banner is there, and no INFO line follows it.
do
  local driver, dir = install("return { log_level = 'error' }")
  driver.open = recording(driver)
  driver.start({ banner = 'the banner' })
  local own = lines(dir)
  assert(own[1] == 'INFO the banner', 'the banner is missing at error')
  assert(#own == 1, 'a line followed the banner at error: ' .. tostring(own[2]))
end

-- A key that is not one: a WARN line naming it.
do
  local driver, dir = install('return { prot = 1 }')
  driver.open = recording(driver)
  driver.start({ banner = 'the banner' })
  local own = lines(dir)
  assert(own[2] == 'INFO read ' .. dir .. 'Config/DCSBridge.lua', 'the read was not logged')
  assert(own[3] and own[3]:find('^WARN .*names prot, which is not a key'),
    'no WARN line names prot: ' .. tostring(own[3]))
end

-- The file's cap and retention reach the logger. The open pruned at the
-- defaults, before the file was read, so a rotated file five days old
-- survives it. Then a rotation comes at the file's 4096-byte cap, not the
-- default 32 MiB, and its prune deletes that file at a one-day retention.
do
  local old = 'DCSBridge.' .. os.date('!%Y%m%d-%H%M%S', os.time() - 5 * 86400) .. '.log'
  local driver, dir = install('return { log_max_bytes = 4096, log_retention_days = 1 }')
  local logs = dir .. 'Logs/DCSBridge/'
  write(logs .. old, 'old\n')
  driver.open = recording(driver)
  driver.start({ banner = 'the banner' })
  assert(read(logs .. old), 'the old file did not survive the open')

  local long = string.rep('x', 3000)
  driver.log.info(long)
  driver.log.info(long)

  assert(not read(logs .. old), 'the old file outlived a one-day retention')
  local rotated = 0
  for name in lfs.dir(logs) do
    if name:find('^DCSBridge%.%d+%-%d+.*%.log$') then
      rotated = rotated + 1
    end
  end
  assert(rotated == 1, rotated .. ' rotated files, expected the one rotated at 4096 bytes')
end

-- A file that does not read: start raises the reason after logging it, and
-- the module is never opened.
do
  local driver, dir = install('return 1')
  local open, calls = recording(driver)
  local opened = false
  driver.open = function(...)
    opened = true
    return open(...)
  end
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran on a file that does not read')
  assert(err:find('returned a number, not a table', 1, true), 'start raised ' .. tostring(err))
  assert(not opened and #calls == 0, 'the module opened on a file that does not read')
  local own = lines(dir)
  assert(own[1] == 'INFO the banner', 'the banner is missing before the error')
  assert(own[2] == 'ERROR ' .. err, 'the reason is not logged: ' .. tostring(own[2]))
end

-- A refusal from the broker: logged and raised.
do
  local driver, dir = install()
  driver.open = function()
    return {
      interface = driver.INTERFACE,
      configure = function() error('configure refused: `port` must be 0 to 65535', 0) end,
    }
  end
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran past a refusal')
  assert(err == 'configure refused: `port` must be 0 to 65535', 'start raised ' .. tostring(err))
  local own = lines(dir)
  assert(own[#own] == 'ERROR ' .. err, 'the refusal is not logged: ' .. tostring(own[#own]))
end

-- A module from another build: start raises naming both versions, and
-- configure is never called, so nothing binds.
do
  local driver, dir = install()
  local open, calls = recording(driver, 'another')
  driver.open = open
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran against another interface')
  assert(err:find('interface another', 1, true)
    and err:find('interface ' .. driver.INTERFACE, 1, true),
    'the reason does not name both versions: ' .. tostring(err))
  assert(#calls == 0, 'configure was called against another interface')
  local own = lines(dir)
  assert(own[#own] == 'ERROR ' .. err, 'the mismatch is not logged: ' .. tostring(own[#own]))
end

-- No generated file, or one with no hash: start raises naming the path,
-- and configure is never called.
do
  local driver, dir = install(nil, { [GEN] = true })
  local open, calls = recording(driver)
  driver.open = open
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran with no generated file')
  assert(err:find(dir .. GEN, 1, true), 'the reason does not name the file: ' .. tostring(err))
  assert(#calls == 0, 'configure was called with no generated file')
  local own = lines(dir)
  assert(own[#own] == 'ERROR ' .. err, 'the failure is not logged: ' .. tostring(own[#own]))

  write(dir .. GEN, 'return { schema_sha256 = "abc" }')
  started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started and err:find('carries no schema_sha256', 1, true),
    'a short hash was taken: ' .. tostring(err))
  assert(#calls == 0, 'configure was called with a short hash')
end

-- No schema.pb: start raises naming the path, and configure is never
-- called.
do
  local driver, dir = install(nil, { [SCHEMA] = true })
  local open, calls = recording(driver)
  driver.open = open
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran with no schema')
  assert(err:find(dir .. SCHEMA, 1, true), 'the reason does not name the file: ' .. tostring(err))
  assert(#calls == 0, 'configure was called with no schema')
  local own = lines(dir)
  assert(own[#own] == 'ERROR ' .. err, 'the failure is not logged: ' .. tostring(own[#own]))
end

-- An empty schema.pb, as a truncated unzip leaves one: start raises before
-- configure, so nothing binds.
do
  local driver, dir = install(nil, { [SCHEMA] = true })
  write(dir .. SCHEMA, '')
  local open, calls = recording(driver)
  driver.open = open
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started and err:find('the schema is empty', 1, true),
    'an empty schema was taken: ' .. tostring(err))
  assert(#calls == 0, 'configure was called with an empty schema')
end

-- A schema whose hash is not the generated file's: a WARN line naming both,
-- and the start completes.
do
  local OTHER = string.rep('0', 64)
  local driver, dir = install()
  driver.open = recording(driver, nil, function() return OTHER end)
  driver.start({ banner = 'the banner' })
  local own = lines(dir)
  assert(own[#own] == 'WARN schema ' .. OTHER .. ' but HookDriver.gen.lua was generated from ' .. ABC,
    'the mismatch is not a warning: ' .. tostring(own[#own]))
end

-- A schema the broker refuses: the one stop after configure, logged and
-- raised with the listener up.
do
  local driver, dir = install()
  local open, calls = recording(driver, nil, function()
    error('schema refused: the schema is 3 bytes and its answer would not fit', 0)
  end)
  driver.open = open
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran past a refused schema')
  assert(err:find('^schema refused: .*no schema to serve$'), 'start raised ' .. tostring(err))
  assert(#calls == 1, #calls .. ' configure calls before the refusal, expected 1')
  local own = lines(dir)
  assert(own[#own] == 'ERROR ' .. err, 'the refusal is not logged: ' .. tostring(own[#own]))
end

-- The generated tables register after the configure and the schema, in the
-- broker's order, and before the first extension file runs. They are the
-- tables of the one load of HookDriver.gen.lua, and classes is handed no
-- hash. ADR 0072.
do
  local driver, dir = install()
  write(dir .. 'DCSBridge/hookdriver.d/a.lua', 'CHAINED()')
  local open, calls, _, order = recording(driver)
  driver.open = open
  local chained
  CHAINED = function()
    chained = #order
  end
  local loads, loadfile = 0, loadfile
  _G.loadfile = function(path, ...)
    if path == dir .. GEN then
      loads = loads + 1
    end
    return loadfile(path, ...)
  end
  driver.start({ banner = 'the banner' })
  _G.loadfile = loadfile

  local want = { 'configure', 'schema', 'classes', 'routes', 'caps', 'replies' }
  assert(#order == #want, #order .. ' calls, expected ' .. #want)
  for i, name in ipairs(want) do
    assert(order[i] == name, 'call ' .. i .. ' was ' .. tostring(order[i]) .. ', expected ' .. name)
  end
  assert(chained == #want, 'the chain ran after ' .. tostring(chained) .. ' calls, expected all six')
  assert(#calls == 1 and loads == 1, loads .. ' loads of the generated file, expected 1')
  local topic = 'dcsbridge.hook.ReloadConfig'
  assert(order.classes[topic] == 'command', 'the class was handed as ' .. tostring(order.classes[topic]))
  assert(order.routes[topic] == 'hook_driver', 'the route was handed as ' .. tostring(order.routes[topic]))
  assert(order.caps[topic] == 'reload', 'the capability was handed as ' .. tostring(order.caps[topic]))
  assert(type(order.replies) == 'table' and next(order.replies) == nil, 'the replies were not the empty list')
  CHAINED = nil
end

-- A registration the broker refuses stops the start after the configure,
-- with an ERROR line naming the call, and no extension file runs.
do
  local driver, dir = install()
  write(dir .. 'DCSBridge/hookdriver.d/a.lua', 'CHAINED = true')
  local open, calls = recording(driver)
  driver.open = function()
    local shim = open()
    shim.routes = function()
      error('routes refused: conflicting', 0)
    end
    return shim
  end
  local started, err = pcall(driver.start, { banner = 'the banner' })
  assert(not started, 'start ran past a refused registration')
  assert(err == 'the generated routes were not registered: routes refused: conflicting',
    'start raised ' .. tostring(err))
  assert(#calls == 1, #calls .. ' configure calls before the refusal, expected 1')
  assert(CHAINED == nil, 'an extension file ran behind a refused registration')
  local own = lines(dir)
  assert(own[#own] == 'ERROR ' .. err, 'the refusal is not logged: ' .. tostring(own[#own]))
end

-- The kill switch is the file's from the start: off with one INFO line
-- after the broker's answer, and on by default with no line. ADR 0063.
do
  local driver, dir = install('return { enabled = false }')
  driver.open = recording(driver)
  driver.start({ banner = 'the banner' })
  assert(driver.enabled() == false, 'a file with enabled = false started on')
  local own = lines(dir)
  assert(own[#own - 1] == 'INFO bridge disabled',
    'the line before the hash is not the switch: ' .. tostring(own[#own - 1]))

  driver, dir = install()
  driver.open = recording(driver)
  driver.start({ banner = 'the banner' })
  assert(driver.enabled() == true, 'the defaults started off')
  for _, line in ipairs(lines(dir)) do
    assert(not line:find('bridge %a+abled'), 'a start that stays on logged ' .. line)
  end
end

-- The real module, asked for at the shipped path and opened at the
-- harness's. Port 0 lets the system pick, so the run collides with nothing.
-- Its interface matches the driver's, it counts schema_sha256 as nothing
-- unknown, and the hash it answers for schema.pb is the golden's. A stock
-- Lua opens no socket, so the handshake carrying that hash is the Rust
-- loopback tests' to read. load.lua says why both spellings are tried.
do
  local driver, dir = install('return { port = 0 }')
  local shim
  driver.open = function(path)
    assert(path == dir .. DLL, 'start asked for ' .. tostring(path))
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

  local own = lines(dir)
  for _, line in ipairs(own) do
    assert(not line:find('defaults in force', 1, true), 'the file was not read')
  end
  local want = 'INFO configured: interface ' .. shim.interface .. ', applied '
  local answer = own[#own - 1]
  assert(answer:sub(1, #want) == want and answer:find(', unknown 0, pending_restart 0$'),
    'the line before the hash is not the broker\'s answer: ' .. tostring(answer))
  assert(own[#own] == 'INFO schema ' .. ABC,
    'the last line is not the hash of schema.pb: ' .. tostring(own[#own]))
end

hook.remove()

print('start reads the configuration after the banner and configures the broker')
