-- The generated files, loaded by a stock Lua 5.1 and registered.
--
-- The generator's golden copies are what it writes over the shipped schema.
-- This loads both the way the drivers will: the sim driver's file into
-- DCSBridge.code, the hook driver's into a table of its own with no global
-- touched. It then registers their tables through the four calls and checks
-- that nothing is refused, that each call adds the rows the schema has, and
-- that loading and registering the sim file again adds nothing.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/generated.lua <module path>')

local root = assert(arg[0]:match('^(.*)/tests/lua/[^/]*$'), 'run it by its path: ' .. arg[0])
local golden = root .. '/crates/generator/tests/golden/'

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

-- The hash of abc, which the goldens carry because the generator test
-- points the schema parameter at a three-byte fixture.
local abc = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
local fields = { 'topics', 'classes', 'routes', 'caps', 'replies' }

local function shaped(what, gen)
  assert(type(gen) == 'table', what .. ' returned no table')
  for _, name in ipairs(fields) do
    assert(type(gen[name]) == 'table', what .. ' has no ' .. name .. ' table')
  end
  assert(gen.schema_sha256 == abc, what .. ' carries the hash ' .. tostring(gen.schema_sha256))
end

local function register(what, gen, expected)
  for index, call in ipairs({ 'classes', 'routes', 'caps', 'replies' }) do
    local ok, added = pcall(shim[call], gen[call])
    assert(ok, what .. ': shim.' .. call .. ' refused its table: ' .. tostring(added))
    assert(added == expected[index],
      what .. ': shim.' .. call .. ' added ' .. tostring(added) .. ', not ' .. expected[index])
  end
end

DCSBridge = { code = {} }
local sim = dofile(golden .. 'SimDriver.gen.lua')
assert(sim == DCSBridge.code, 'the sim file did not return DCSBridge.code')
shaped('SimDriver.gen.lua', sim)
local unit = 'dcsbridge.builtin.sim.UnitDestroyed'
assert(sim.topics.UnitDestroyed == unit, 'the topics table does not name UnitDestroyed')

-- The hook file loads with no DCSBridge global at all.
DCSBridge = nil
local hook = dofile(golden .. 'HookDriver.gen.lua')
assert(DCSBridge == nil, 'the hook file wrote a global')
shaped('HookDriver.gen.lua', hook)

-- The calls only store, so none is refused before configure.
register('SimDriver.gen.lua', sim, { 1, 0, 1, 0 })
register('HookDriver.gen.lua', hook, { 0, 0, 0, 0 })

-- A sim driver reload loads its file again into the same DCSBridge.code and
-- registers the same rows: a no-op that succeeds.
DCSBridge = { code = sim }
local reloaded = dofile(golden .. 'SimDriver.gen.lua')
assert(reloaded == sim, 'a reload did not write into the same DCSBridge.code')
register('SimDriver.gen.lua reloaded', reloaded, { 0, 0, 0, 0 })

print('ok  both generated files load, each carrying the schema hash')
print('ok  the sim file writes into DCSBridge.code and the hook file touches no global')
print('ok  their tables register through the four calls with nothing refused')
print('ok  a reload registers the same rows and adds none')
