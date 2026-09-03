-- The put calls, from a stock Lua 5.1.
--
-- A record opens with begin, takes typed puts and closes with commit, which
-- returns the body until the commit ring takes it. The bytes are compared
-- against hand-assembled protobuf, because a harness that only checks the
-- calls return is a harness that exercised nothing. The Rust tests decode the
-- same encoder through a stock library; this checks the Lua side of the
-- crossing delivers the same bytes.
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

local function hex(s)
  return (s:gsub('.', function(c)
    return string.format('%02x', c:byte())
  end))
end

local function expect(got, want, what)
  assert(hex(got) == want, what .. ': got ' .. hex(got) .. ', want ' .. want)
end

-- A length padded to three bytes, the width of the largest record the buffer
-- holds, as hex. ADR 0012.
local function padded(n)
  return string.format('%02x%02x%02x', n % 128 + 128, math.floor(n / 128) % 128 + 128, math.floor(n / 16384))
end

-- What commit returns: the envelope's payload field, an Any whose type URL is
-- the topic behind protobuf's prefix and whose value is the record's fields.
-- Both lengths are padded like a nested message's.
local function wrap(topic, body)
  local url = 'type.googleapis.com/' .. topic
  local value = '12' .. padded(#body / 2) .. body
  local type_url = string.format('0a%02x', #url) .. hex(url)
  return '22' .. padded(#type_url / 2 + #value / 2) .. type_url .. value
end

local shim = open(path)
for _, name in ipairs({
  'begin', 'integer', 'double', 'string', 'boolean', 'message', 'end_message', 'commit',
}) do
  assert(type(shim[name]) == 'function', 'shim.' .. name .. ' is missing')
end

-- Two scalars, minimal tags and values.
shim.begin('dcs.builtin.UnitDestroyed')
shim.string(1, 'x')
shim.integer(2, 3)
expect(
  shim.commit(),
  wrap('dcs.builtin.UnitDestroyed', '0a0178' .. '1003'),
  'a string and an integer, wrapped in the payload'
)

-- A negative integer is ten bytes, a double eight behind its tag, a boolean
-- one, and a two-byte tag starts at field 16.
shim.begin('t')
shim.integer(1, -1)
shim.double(2, 1.5)
shim.boolean(3, true)
shim.integer(16, 0)
expect(
  shim.commit(),
  wrap('t', '08ffffffffffffffffff01' .. '11000000000000f83f' .. '1801' .. '800100'),
  'the scalar wire forms'
)

-- A nested message carries its length padded to three bytes, the width of
-- the largest record the buffer holds. ADR 0012.
shim.begin('t')
shim.message(3)
shim.string(1, 'a')
shim.end_message()
expect(shim.commit(), wrap('t', '1a838000' .. '0a0161'), 'a nested message')

-- An empty record is an empty Any value behind the topic.
shim.begin('t')
expect(shim.commit(), wrap('t', ''), 'an empty record')

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

-- A commit with a message open is refused: nil back, and the next record
-- starts clean.
shim.begin('t')
shim.message(3)
assert(shim.commit() == nil, 'a commit with a message open returned a body')
shim.begin('t')
shim.integer(1, 1)
expect(shim.commit(), wrap('t', '0801'), 'the record after a refused one')

-- A begin over an open record discards it.
shim.begin('t')
shim.integer(1, 1)
shim.begin('t')
expect(shim.commit(), wrap('t', ''), 'a record begun over another')

-- Each state has its own record in progress.
local second = open(path)
shim.begin('t')
shim.integer(1, 1)
second.begin('t')
second.integer(1, 2)
expect(second.commit(), wrap('t', '0802'), "the second table's record")
expect(shim.commit(), wrap('t', '0801'), "the first table's record, after the second committed")

print('ok  the eight put calls are on the table')
print('ok  scalars, a padded nested length and an empty record match by hand')
print('ok  a defect raises, a refused commit returns nil, a begin discards')
print('ok  two tables hold two records in progress')
