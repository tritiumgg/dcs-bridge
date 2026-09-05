-- The put calls, from a stock Lua 5.1.
--
-- A record opens with begin, takes typed puts and closes with commit, which
-- queues it for every connection and says whether it did. The bytes are not
-- observable from here: the Rust tests decode the same encoder through a
-- stock library and read the frames off a socket. What this checks is the
-- Lua side of the crossing: the calls are on the table, each argument is
-- checked, a defect raises, a refused record says so, and each state has its
-- own record in progress.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/put.lua <module path>')

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
-- configure comes first: nothing opens a record before it. configure.lua
-- checks the refusal; here the call is what lets the puts run.
shim.configure({ port = 0 })
for _, name in ipairs({
  'begin', 'begin_to', 'integer', 'double', 'string', 'boolean', 'message', 'end_message', 'commit',
}) do
  assert(type(shim[name]) == 'function', 'shim.' .. name .. ' is missing')
end

local function queued(what, result)
  assert(result == true, what .. ': commit returned ' .. tostring(result))
end

-- One of every put, then a commit that queues.
shim.begin('dcs.builtin.UnitDestroyed')
shim.string(1, 'x')
shim.integer(2, 3)
shim.double(3, 1.5)
shim.boolean(4, true)
shim.message(5)
shim.string(1, 'a')
shim.end_message()
shim.integer(16, 0)
queued('a record of every put', shim.commit())

-- An empty record queues too.
shim.begin('t')
queued('an empty record', shim.commit())

-- A put before begin, a stray end_message and a non-boolean all raise.
local function raises(what, f, ...)
  local ok, err = pcall(f, ...)
  assert(not ok, what .. ' did not raise')
  return err
end
assert(
  raises('a put before begin', shim.integer, 1, 1):find('no record is open', 1, true),
  'the wrong error for a put before begin'
)
shim.begin('t')
raises('a stray end_message', shim.end_message)
shim.begin('t')
raises('a number for a boolean', shim.boolean, 1, 1)
shim.begin('t')
raises('field zero', shim.integer, 0, 1)

-- A commit with a message open is refused: false back, and the next record
-- starts clean.
shim.begin('t')
shim.message(3)
assert(shim.commit() == false, 'a commit with a message open was not refused')
shim.begin('t')
shim.integer(1, 1)
queued('the record after a refused one', shim.commit())

-- A begin over an open record discards it, and the new one queues.
shim.begin('t')
shim.integer(1, 1)
shim.begin('t')
queued('a record begun over another', shim.commit())

-- Each state has its own record in progress.
local second = open(path)
shim.begin('t')
shim.integer(1, 1)
second.begin('t')
second.integer(1, 2)
queued("the second table's record", second.commit())
queued("the first table's record, after the second committed", shim.commit())

print('ok  the nine put calls are on the table')
print('ok  a record of every put queues, and so does an empty one')
print('ok  a defect raises, a refused commit returns false, a begin discards')
print('ok  two tables hold two records in progress')
