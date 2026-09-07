-- shim.epoch, from a stock Lua 5.1.
--
-- The hook driver opens an epoch with its id at mission load end and closes
-- it with nil at simulation stop. What the broker does between the two, the
-- stamp on every record, is not observable from here: the Rust tests read
-- it off a socket. What this checks is the Lua side of the call: it is on
-- the table; an id is taken and nothing comes back; nil and no argument
-- close; and zero, a fraction, a negative, an id past 2^32 - 1, a string
-- and a table raise rather than opening an epoch by that name.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/epoch.lua <module path>')

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
assert(type(shim.epoch) == 'function', 'shim.epoch is missing')

local function raises(what, f, ...)
  local ok, err = pcall(f, ...)
  assert(not ok, what .. ' did not raise')
  return err
end

local function returns_nothing(what, ...)
  local results = { shim.epoch(...) }
  assert(#results == 0, what .. ' returned ' .. #results .. ' values')
end

-- The call stores and nothing more, so it needs no configure first: the
-- hook driver's first boundary can come from any order of its start.
returns_nothing('epoch 1', 1)
returns_nothing('epoch 2^32 - 1', 4294967295)
returns_nothing('epoch nil', nil)
returns_nothing('epoch with nothing')

-- Zero is what the field reads as between epochs, a fraction and a
-- negative are no id, and past 2^32 - 1 the field cannot carry it.
for _, bad in ipairs({ 0, 1.5, -1, 4294967296 }) do
  local err = raises('epoch ' .. bad, shim.epoch, bad)
  assert(err:find('not an epoch id', 1, true), 'the wrong complaint: ' .. err)
end

-- A string would convert under a looser check, and a table is no number.
local str = raises('epoch with a string', shim.epoch, '7')
assert(str:find('number expected', 1, true), 'the wrong complaint: ' .. str)
raises('epoch with a table', shim.epoch, {})
raises('epoch with a boolean', shim.epoch, true)

print('ok  epoch is on the table and takes an id or nil')
print('ok  zero, a fraction, a negative and an id past 2^32 - 1 raise')
print('ok  a string, a table and a boolean raise')
