-- classes, routes, caps and replies, from a stock Lua 5.1.
--
-- The four registration calls merge into one process-global registry, which
-- this script checks as the two registrars would use it: a second call over
-- new topics merges, an identical call is a no-op that answers zero, a
-- conflicting row is refused whole naming the topic and both values, an
-- outbound-only topic registers with a class and a capability and no route,
-- and a table that is not topic-to-member is refused before the broker sees
-- it. Whether a begin on a registered topic is allowed is put.lua's to
-- check; here the maps only fill.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/register.lua <module path>')

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
for _, name in ipairs({ 'classes', 'routes', 'caps', 'replies' }) do
  assert(type(shim[name]) == 'function', 'shim.' .. name .. ' is missing')
end

local function raises(what, f, ...)
  local ok, err = pcall(f, ...)
  assert(not ok, what .. ' did not raise')
  return err
end

local function refused(what, f, argument, ...)
  local err = raises(what, f, argument)
  for _, text in ipairs({ ... }) do
    assert(err:find(text, 1, true), what .. ': the refusal does not say `' .. text .. '`: ' .. err)
  end
  return err
end

local event = 'dcsbridge.builtin.sim.UnitDestroyed'
local command = 'dcsbridge.builtin.sim.SetFlag'
local reply = 'dcsbridge.builtin.sim.FlagValue'

-- The calls only store, so none is refused before configure. The hook
-- driver registers at DCS start, right after its first configure, and the
-- order between the two is not the broker's to insist on.
assert(shim.classes({}) == 0, 'an empty table added something')

-- The hook driver's tables: an outbound event with a class and a capability
-- and no route.
assert(shim.classes({ [event] = 'durable' }) == 1, 'the event class was not added')
assert(shim.caps({ [event] = 'read' }) == 1, 'the event capability was not added')

-- The sim driver's tables, over new topics: they merge beside the first
-- registrar's rows, and a value spelled by number reads the same as by name.
assert(shim.classes({ [command] = 3, [reply] = 'durable' }) == 2, 'the sim tables did not merge')
assert(shim.routes({ [command] = 'sim_driver' }) == 1, 'the route was not added')
assert(shim.caps({ [command] = 2, [reply] = 'read' }) == 2, 'the sim capabilities did not merge')
assert(shim.replies({ reply }) == 1, 'the reply was not added')

-- A sim driver reload re-registers the same tables: a no-op that succeeds.
assert(shim.classes({ [command] = 'command', [reply] = 1 }) == 0, 'a re-registration added a row')
assert(shim.routes({ [command] = 1 }) == 0, 'a re-registered route added a row')
assert(shim.caps({ [command] = 'command', [reply] = 'read' }) == 0, 'a re-registered capability added a row')
assert(shim.replies({ reply }) == 0, 'a re-registered reply added a row')

-- A conflicting row refuses the whole call: the new topic beside it is not
-- registered either, which a later registration of it alone shows by adding
-- one.
local other = 'dcsbridge.builtin.sim.Other'
refused('a conflicting class', shim.classes, { [other] = 'lossy', [event] = 'lossy' },
  'classes refused', event, 'registered as durable', 'not lossy')
assert(shim.classes({ [other] = 'lossy' }) == 1, 'a refused call registered the row beside the conflict')
refused('a conflicting route', shim.routes, { [command] = 'hook_driver' },
  'routes refused', command, 'registered as sim_driver', 'not hook_driver')
refused('a conflicting capability', shim.caps, { [command] = 'reload' },
  'caps refused', command, 'registered as command', 'not reload')

-- A table that is not topic-to-member is refused before the broker sees it,
-- naming the row.
refused('a value that names no class', shim.classes, { [other] = 'sturdy' },
  'classes refused', other, 'names no member', 'durable, lossy, command, lifecycle')
refused('an uppercase name', shim.classes, { [other] = 'LOSSY' }, 'names no member')
refused('a number past the enum', shim.routes, { [other] = 3 }, 'routes refused', 'sim_driver, hook_driver')
refused('a fraction', shim.caps, { [other] = 1.5 }, 'caps refused', 'read, command, reload')
refused('a boolean value', shim.caps, { [other] = true }, 'caps refused', other)
refused('a number key', shim.classes, { 'durable' }, 'classes refused', 'a key is not a topic')
refused('a reply that is not a string', shim.replies, { 1 }, 'replies refused', 'entry 1 is not a topic')
refused('a reply table with a string key', shim.replies, { [reply] = true }, 'replies refused', 'not a position')
for _, call in ipairs({ 'classes', 'routes', 'caps', 'replies' }) do
  raises(call .. ' with no table', shim[call])
  raises(call .. ' with a string', shim[call], 'durable')
end

print('ok  the four calls are on the table and a second registrar merges')
print('ok  an identical registration answers zero, and a conflicting row is refused whole')
print('ok  an outbound-only topic registers with a class and a capability and no route')
print('ok  a table that is not topic-to-member is refused naming the row')
