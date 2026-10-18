-- The generated files, loaded by a stock Lua 5.1 and registered.
--
-- The generator's golden copies are what it writes over the shipped schema
-- and over an adopter's. This loads them the way the drivers will: the sim
-- driver's files into DCSBridge.code, the shipped one first, and the hook
-- driver's into tables of their own with no global touched. It then
-- registers their tables through the four calls and checks that nothing is
-- refused, that each call adds the rows the schema has, and that loading
-- and registering the sim files again adds nothing.
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
local hook = assert(loadfile(golden .. 'HookDriver.gen.lua'))(shim)
assert(DCSBridge == nil, 'the hook file wrote a global')
shaped('HookDriver.gen.lua', hook)

-- The calls only store, so none is refused before configure.
register('SimDriver.gen.lua', sim, { 1, 0, 1, 0 })
register('HookDriver.gen.lua', hook, { 0, 0, 0, 0 })

-- An adopter's pair loads after the shipped pair. Its sim file adds its
-- topics to DCSBridge.code beside the shipped ones and returns a slice of
-- its own, which registers the adopter's rows and nothing else.
DCSBridge = { code = sim }
local adopter = 'fixture.adopter.'
local slice = dofile(golden .. adopter .. 'sim.gen.lua')
shaped(adopter .. 'sim.gen.lua', slice)
assert(slice ~= sim, 'the adopter file returned DCSBridge.code, not its slice')
assert(sim.topics.UnitDestroyed == unit, 'the adopter file dropped a shipped topic')
for _, name in ipairs({ 'RocketImpact', 'GetPosition', 'Position' }) do
  assert(sim.topics[name] == adopter .. name, 'DCSBridge.code.topics has no ' .. name)
end
assert(slice.topics.UnitDestroyed == nil, 'the slice holds a shipped topic')
register(adopter .. 'sim.gen.lua', slice, { 3, 1, 3, 1 })

DCSBridge = nil
local adopter_hook = assert(loadfile(golden .. adopter .. 'hook.gen.lua'))(shim)
assert(DCSBridge == nil, 'the adopter hook file wrote a global')
shaped(adopter .. 'hook.gen.lua', adopter_hook)
register(adopter .. 'hook.gen.lua', adopter_hook, { 1, 0, 1, 0 })

-- A sim driver reload loads the shipped file and then the adopter's again,
-- into the same DCSBridge.code, and registers the same rows: a no-op that
-- succeeds, with both sets of topics in place.
DCSBridge = { code = sim }
local reloaded = dofile(golden .. 'SimDriver.gen.lua')
assert(reloaded == sim, 'a reload did not write into the same DCSBridge.code')
register('SimDriver.gen.lua reloaded', reloaded, { 0, 0, 0, 0 })
register(adopter .. 'sim.gen.lua reloaded', dofile(golden .. adopter .. 'sim.gen.lua'), { 0, 0, 0, 0 })
assert(sim.topics.UnitDestroyed == unit and sim.topics.RocketImpact == adopter .. 'RocketImpact',
  'a reload lost a topic')

-- The emitters write through the real shim once it is configured: the sim
-- files' through DCSBridge.shim, the hook files' through the shim each was
-- loaded with. Each commit returning true says the shim took every put.
local function queued(what, result)
  assert(result == true, what .. ': commit returned ' .. tostring(result))
end
shim.configure({ port = 0 })
DCSBridge = { code = sim, shim = shim }
queued('emit.UnitDestroyed', sim.emit.UnitDestroyed('x', 1))
local point = { x = 1.5, z = -2 }
queued('emit.RocketImpact', sim.emit.RocketImpact(
  'S-8', { 1, 2, 3 }, { point, { x = 0 } }, 4.5, true, 2, '\0\1', point))
queued('emit.RocketImpact with every argument nil', sim.emit.RocketImpact())
queued('emit.Position', sim.emit.Position(1, 7, point))
assert(sim.emit.GetPosition == nil, 'a command has an emitter')
queued('the adopter hook emit.Chat', adopter_hook.emit.Chat('hello'))
assert(next(hook.emit) == nil, 'the shipped hook file has an emitter for no record')

-- A wrapper hands its emitter and arguments to the buffer the runtime puts
-- on the table after loading. Until the runtime exists, this buffer calls
-- the emitter at once and records what it was handed.
local handed
local function buffer(emit, ...)
  handed = select('#', ...)
  return emit(...)
end
sim.buffer = buffer
adopter_hook.buffer = buffer
queued('send.UnitDestroyed', sim.send.UnitDestroyed('x', 1))
queued('send.RocketImpact', sim.send.RocketImpact('S-8', { 4 }, { point }, nil, false, 1, '', point))
assert(handed == 8, 'send.RocketImpact handed the buffer ' .. handed .. ' arguments, not 8')
queued('send_to.Position', sim.send_to.Position(1, 7, point))
assert(handed == 3, 'send_to.Position handed the buffer ' .. handed .. ' arguments, not 3')
queued('the adopter hook send.Chat', adopter_hook.send.Chat('hello'))
assert(sim.send.Position == nil, 'a reply has a send wrapper')
assert(sim.send_to.RocketImpact == nil, 'a fan-out record has a send_to wrapper')
assert(sim.send.GetPosition == nil and sim.send_to.GetPosition == nil, 'a command has a wrapper')

print('ok  every generated file loads, each carrying the schema hash')
print('ok  the sim files write into DCSBridge.code and the hook files touch no global')
print('ok  an adopter slice adds its topics beside the shipped ones')
print('ok  their tables register through the four calls with nothing refused')
print('ok  a reload registers the same rows and adds none')
print('ok  each emitter commits through the shim, a reply through begin_to')
print('ok  send and send_to hand their emitter and every argument to the buffer')
