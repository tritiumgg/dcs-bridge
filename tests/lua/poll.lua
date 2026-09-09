-- poll, from a stock Lua 5.1.
--
-- A record a consumer sends reaches the ring its route names, and poll
-- takes it off that ring as the sender's connection id, the topic and the
-- bytes. Nothing sends from here: the Rust tests put a record on a socket
-- and poll it through the bridge. What this checks is the Lua side of the
-- call: it is on the table, it is refused before configure, a target is
-- read by name or by the schema's number and anything else is refused
-- naming the members, and an empty ring answers nil for either target.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/poll.lua <module path>')

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
assert(type(shim.poll) == 'function', 'shim.poll is missing')

local function raises(what, f, ...)
  local ok, err = pcall(f, ...)
  assert(not ok, what .. ' did not raise')
  return err
end

local function refused(what, argument, ...)
  local err = raises(what, shim.poll, argument)
  assert(err:find('poll refused', 1, true), what .. ': the error is not a refusal: ' .. err)
  for _, text in ipairs({ ... }) do
    assert(err:find(text, 1, true), what .. ': the refusal does not say `' .. text .. '`: ' .. err)
  end
  return err
end

-- configure comes first: the rings it allocates are what poll reads.
refused('poll before configure', 'sim_driver', 'configure comes first')
refused('poll by number before configure', 2, 'configure comes first')

shim.configure({ port = 0 })

-- An empty ring answers one nil, for either target, spelled either way.
for _, target in ipairs({ 'sim_driver', 'hook_driver', 1, 2 }) do
  local n = select('#', shim.poll(target))
  local id = shim.poll(target)
  assert(n == 1 and id == nil, 'an empty ring answered ' .. n .. ' values for ' .. tostring(target))
end

-- A target that names no member is refused naming the two that do, and the
-- refusal is the same whatever kind of value was given.
for _, bad in ipairs({ 'sim', 'SIM_DRIVER', 0, 3, 1.5, -1, 2 ^ 32, true, {} }) do
  refused('poll with target ' .. tostring(bad), bad, 'names no member', 'sim_driver, hook_driver')
end
refused('poll with no target', nil, 'names no member')

print('ok  poll is on the table and is refused before configure')
print('ok  an empty ring answers nil for either target, by name or by number')
print('ok  a target that names no member is refused naming the members')
