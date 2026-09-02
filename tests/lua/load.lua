-- The module opens under a stock Lua 5.1.
--
-- SPEC 17's *Any (native module)* rows run against a host-native build of the
-- broker loaded by a stock Lua 5.1, which is what lets them run in CI with no
-- DCS present. This is that load, spelled the way SPEC 5.1.1 spells it inside
-- DCS: package.loadlib against an explicit path, never require.
--
-- SPEC 17 also says a clean harness run that exercised nothing is a failure
-- rather than a pass, so every check below reads a value.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path, expected = ...

local usage = 'usage: lua tests/lua/load.lua <module path> <expected version>'
assert(path, usage)
assert(expected, usage)

-- package.loadlib is the Lua 5.1 spelling. The bare loadlib global is Lua 5.0
-- and is absent from both this interpreter and the DCS hook state.
assert(package.loadlib, 'package.loadlib is missing, so this is not Lua 5.1')
assert(loadlib == nil, 'the Lua 5.0 loadlib global is present')

-- Which name finds the opener is a property of how the interpreter was built,
-- not of the module.
--
-- Lua 5.1 predates dlopen on macOS and ships a second implementation over the
-- old dyld API, which package.loadlib reaches unless the build defines
-- LUA_USE_MACOSX. That one passes the name through untouched, so it wants the
-- Mach-O spelling with its leading underscore. Every dlopen build takes the
-- bare name, and dlopen adds the underscore itself.
--
-- DCS is a dlopen-equivalent load on Windows and takes the bare name, which is
-- what SPEC 5.1.1 writes. So try that first and let the underscore be the
-- fallback: wherever the bare name works, the harness has exercised the
-- spelling DCS uses.
local function open(module)
  for _, name in ipairs({ 'luaopen_dcsbridge', '_luaopen_dcsbridge' }) do
    local loader = package.loadlib(module, name)
    if loader then
      return loader, name
    end
  end

  local _, err = package.loadlib(module, 'luaopen_dcsbridge')
  error('no opener in ' .. module .. ': ' .. tostring(err), 0)
end

local loader, symbol = open(path)
local shim = loader()

assert(type(shim) == 'table', 'luaopen_dcsbridge returned a ' .. type(shim))
assert(
  shim.version == expected,
  'version reads ' .. tostring(shim.version) .. ', expected ' .. expected
)

assert(shim.opens == 1, 'the first open reads ' .. tostring(shim.opens))

-- If both DCS states load the broker, luaopen_* runs twice and each state gets
-- its own Lua table over one bridge. Two opens here stand in for that, and this
-- interpreter is one process the way DCS is one process.
--
-- Two distinct tables is half of it. The other half is that they read one
-- counter: a second table reading 1 would mean the second open got its own
-- bridge, which is the failure the process-global rule exists to prevent.
local second = (open(path))()
assert(second ~= shim, 'a second open returned the same table')
assert(second.version == expected, 'the second open carries no version')
assert(
  second.opens == 2,
  'the second open reads ' .. tostring(second.opens) .. ', so it did not share'
)

print('ok  opened ' .. path)
print('ok  ' .. symbol .. ' returns a table, version ' .. shim.version)
print('ok  a second open returns a distinct table')
print('ok  both tables read one bridge, opens 1 then 2')
