-- begin_to, from a stock Lua 5.1.
--
-- A record opened with begin_to goes to one connection. Where it goes is not
-- observable from here: the Rust tests read the frames off two sockets. What
-- this checks is the Lua side of the call: it is on the table, the id is
-- checked as a number the broker could have handed out, a topic the schema
-- did not mark as a reply or an acknowledgement is refused with an error
-- naming it, and the acknowledgement commits. Nothing is registered, so the
-- acknowledgement is the one addressable topic.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/address.lua <module path>')

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
assert(type(shim.begin_to) == 'function', 'shim.begin_to is missing')

local ack = 'dcs.bridge.CommandAck'
local fanout = 'dcs.builtin.UnitDestroyed'

local function raises(what, f, ...)
  local ok, err = pcall(f, ...)
  assert(not ok, what .. ' did not raise')
  return err
end

local function queued(what, result)
  assert(result == true, what .. ': commit returned ' .. tostring(result))
end

-- The acknowledgement is addressable. Connection 1 need not exist: the
-- writer thread is where a record with nowhere to go is dropped and counted,
-- and commit says only that the record was queued.
shim.begin_to(1, ack)
shim.integer(3, 1)
shim.string(4, 'done')
queued('an acknowledgement', shim.commit())

-- A fan-out topic is refused, the error names it, and no record is open.
local err = raises('begin_to on a fan-out topic', shim.begin_to, 1, fanout)
assert(err:find(fanout, 1, true), 'the refusal does not name the topic: ' .. err)
assert(err:find('refused', 1, true), 'the refusal does not say so: ' .. err)
local none = raises('a put after a refused begin_to', shim.integer, 1, 1)
assert(none:find('no record is open', 1, true), 'a refused begin_to left a record open: ' .. none)

-- The id is a whole number from one that a double carries exactly.
for _, bad in ipairs({ 0, -1, 1.5, 2 ^ 53 + 2, 1 / 0 }) do
  local msg = raises('begin_to with id ' .. tostring(bad), shim.begin_to, bad, ack)
  assert(msg:find('connection id', 1, true), 'the wrong complaint for ' .. tostring(bad) .. ': ' .. msg)
end
-- A string is refused where Lua would have converted it, and the complaint
-- is Lua's own: a number was expected.
local text = raises('begin_to with a string id', shim.begin_to, '1', ack)
assert(text:find('number expected', 1, true), 'a string id was not a type error: ' .. text)
raises('begin_to with no id', shim.begin_to)
raises('begin_to with no topic', shim.begin_to, 1)

-- An abandoned begin_to does not address the next record: begin fans out,
-- and commits.
shim.begin_to(1, ack)
shim.begin(fanout)
shim.integer(1, 1)
queued('a fan-out record after an abandoned begin_to', shim.commit())

-- A second begin_to after the first committed addresses only its own record.
shim.begin_to(2, ack)
queued('a second acknowledgement', shim.commit())
shim.begin(fanout)
queued('a fan-out record after an addressed one', shim.commit())

print('ok  begin_to is on the table and the acknowledgement commits')
print('ok  a fan-out topic is refused by name, with no record left open')
print('ok  an id that the broker could not have handed out is an argument error')
print('ok  an address does not outlive its record')
