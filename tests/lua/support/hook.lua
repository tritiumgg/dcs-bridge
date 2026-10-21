-- The hook driver installed into a scratch write directory, and a broker
-- module stub that records each configure, for the tests that run the hook
-- driver's start and the calls after it. Lives outside tests/lua so
-- tools/luatest.sh does not run it as a test.

local dcs = require('dcs')

local hook = {
  DRIVER = 'Mods/services/DCSBridge/lua/HookDriver.lua',
  BUILTIN = 'Mods/services/DCSBridge/lua/HookDriver.builtin.lua',
  DLL = 'Mods/services/DCSBridge/bin/lua-dcsbridge.dll',
  GEN = 'Mods/services/DCSBridge/lua/HookDriver.gen.lua',
  SCHEMA = 'Mods/services/DCSBridge/schema.pb',
  SIM = 'Mods/services/DCSBridge/lua/SimDriver.lua',
  -- The hash of abc, which the golden generated files carry.
  ABC = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
}

local ROOT = dcs.root()

local scratch = {}

function hook.read(path)
  local f = io.open(path, 'rb')
  if not f then
    return nil
  end
  local text = f:read('*a')
  f:close()
  return text
end

function hook.write(path, text)
  assert(os.execute("mkdir -p '" .. path:match('^(.*)/[^/]+$') .. "'") == 0)
  local f = assert(io.open(path, 'wb'))
  f:write(text)
  f:close()
end

-- A write directory with Logs/, the hook driver and its built-ins file, the
-- sim driver, the generator's golden HookDriver.gen.lua and a schema.pb
-- holding abc, whose hash the golden carries, in it, and the config file
-- when given. A path in omit is left out. Returns the driver and the
-- directory.
function hook.install(config, omit)
  omit = omit or {}
  local dir = dcs.scratch()
  scratch[#scratch + 1] = dir
  assert(os.execute("mkdir -p '" .. dir .. "Logs'") == 0)
  hook.write(dir .. hook.DRIVER, assert(hook.read(ROOT .. 'lua/' .. hook.DRIVER)))
  if not omit[hook.BUILTIN] then
    hook.write(dir .. hook.BUILTIN, assert(hook.read(ROOT .. 'lua/' .. hook.BUILTIN)))
  end
  local sim = hook.read(ROOT .. 'lua/' .. hook.SIM)
  if sim and not omit[hook.SIM] then
    hook.write(dir .. hook.SIM, sim)
  end
  if not omit[hook.GEN] then
    hook.write(dir .. hook.GEN,
      assert(hook.read(ROOT .. 'crates/generator/tests/golden/HookDriver.gen.lua')))
  end
  if not omit[hook.SCHEMA] then
    hook.write(dir .. hook.SCHEMA, 'abc')
  end
  if config then
    hook.write(dir .. 'Config/DCSBridge.lua', config)
  end
  lfs = dcs.lfs(dir)
  log = dcs.log()
  DCS = dcs.DCS()
  net = dcs.net()
  return dofile(dir .. hook.DRIVER), dir
end

-- Replace the sim driver in a write directory with text.
function hook.sim(dir, text)
  hook.write(dir .. hook.SIM, text)
end

-- An opener whose module records each configure and answers as the broker
-- does, without one. Its interface is the driver's unless given, its schema
-- answers the hash of abc unless given another function, and configure
-- answers nothing applied or pending unless given another answer.
--
-- The third value holds each mission time tick was called with, in order,
-- and answer, what tick returns: true until a test sets it, and nothing at
-- all when a test sets it to nil, as a module older than the answer does.
function hook.recording(driver, interface, schema, answer)
  interface = interface or driver.INTERFACE
  answer = answer or { interface = interface, applied = 0, unknown = 0, pending_restart = 0, pending = {} }
  local calls = {}
  local ticks = { answer = true }
  local function open()
    return {
      interface = interface,
      configure = function(table)
        calls[#calls + 1] = table
        return answer
      end,
      tick = function(time)
        ticks[#ticks + 1] = time
        if ticks.answer == nil then
          return
        end
        return ticks.answer
      end,
      schema = schema or function(bytes)
        assert(bytes == 'abc', 'the schema was handed as ' .. tostring(bytes))
        return hook.ABC
      end,
    }
  end
  return open, calls, ticks
end

-- DCSBridge.log as lines, without their timestamps.
function hook.lines(dir)
  local found = {}
  for line in assert(hook.read(dir .. 'Logs/DCSBridge/DCSBridge.log')):gmatch('[^\n]+') do
    found[#found + 1] = line:gsub('^%S+ %S+ ', '')
  end
  return found
end

-- Every write directory install made.
function hook.remove()
  for _, dir in ipairs(scratch) do
    dcs.remove(dir)
  end
end

return hook
