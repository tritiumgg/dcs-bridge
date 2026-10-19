-- The generated decoders, loaded by a stock Lua 5.1 and fed bodies built
-- by hand.
--
-- The adopter fixture's goldens hold two commands: GetPosition in the sim
-- slice and SetPath in the hook file. This loads them the way the drivers
-- will and checks what each decoder reads, skips and refuses.
--
-- Run it through tools/luatest.sh. It needs no module: a decoder never
-- calls the shim.

local root = assert(arg[0]:match('^(.*)/tests/lua/[^/]*$'), 'run it by its path: ' .. arg[0])
local golden = root .. '/crates/generator/tests/golden/'

DCSBridge = { code = {} }
local code = dofile(golden .. 'SimDriver.gen.lua')
dofile(golden .. 'fixture.adopter.sim.gen.lua')
DCSBridge = nil
local hook = assert(loadfile(golden .. 'fixture.adopter.hook.gen.lua'))(nil)

local get_position = assert(code.decode['fixture.adopter.GetPosition'], 'no GetPosition decoder')
local set_path = assert(hook.decode['fixture.adopter.SetPath'], 'no SetPath decoder')

-- A varint of a whole number below 2^53, minimal.
local function varint(n)
  local out = {}
  repeat
    local low = n % 128
    n = (n - low) / 128
    out[#out + 1] = string.char(n > 0 and low + 128 or low)
  until n == 0
  return table.concat(out)
end

local function key(number, wire)
  return varint(number * 8 + wire)
end

local function delimited(number, body)
  return key(number, 2) .. varint(#body) .. body
end

local function raises(what, decoder, bytes, expected)
  local ok, err = pcall(decoder, bytes)
  assert(not ok, what .. ' decoded')
  assert(tostring(err):find(expected, 1, true),
    what .. ' raised "' .. tostring(err) .. '", not "' .. expected .. '"')
end

-- One unknown field number in each skippable wire type.
local unknown = key(99, 0) .. varint(300)
  .. key(98, 1) .. string.rep('\255', 8)
  .. delimited(97, 'skipped')
  .. key(96, 5) .. string.rep('\255', 4)

local function equal(what, actual, expected)
  assert(actual == expected,
    what .. ' decoded as ' .. tostring(actual) .. ', not ' .. tostring(expected))
end

-- Every field absent reads as its zero.
local empty = get_position('')
equal('an absent request_id', empty.request_id, 0)
equal('an absent unit_name', empty.unit_name, '')

-- The known fields read with an unknown field of each wire type around them.
local position = get_position(unknown .. key(1, 0) .. varint(2 ^ 40 + 5)
  .. unknown .. delimited(2, 'Enfield 1-1') .. unknown)
equal('request_id', position.request_id, 2 ^ 40 + 5)
equal('unit_name', position.unit_name, 'Enfield 1-1')

-- uint32 keeps the low 32 bits of a ten-byte varint, which is how a sender
-- writes -1 into it.
local minus_one = string.rep('\255', 9) .. '\1'
local path = set_path(key(2, 0) .. minus_one .. delimited(1, 'a') .. delimited(1, 'b'))
equal('flags', path.flags, 2 ^ 32 - 1)
equal('a name sent twice', path.name, 'b')
equal('an absent serial', path.serial, 0)

-- A known field in another wire type is skipped like an unknown one.
equal('name sent as a varint', set_path(key(1, 0) .. varint(7)).name, '')

-- A length padded to three bytes, as the broker writes one. ADR 0012.
assert(type(set_path(key(97, 2) .. '\131\128\000abc')) == 'table', 'a padded length was refused')

for _, wire in ipairs({ 3, 4, 6, 7 }) do
  raises('wire type ' .. wire, set_path, unknown .. key(95, wire),
    'wire type ' .. wire .. ', which a decoder refuses')
end
raises('field number zero', set_path, key(0, 0) .. '\0', 'out of range')
raises('a varint past the end', set_path, key(99, 0) .. '\128', 'runs past the end')
raises('an eleven-byte varint', set_path, key(99, 0) .. string.rep('\128', 10) .. '\0',
  'longer than ten bytes')
raises('a length past the end', set_path, key(97, 2) .. varint(5) .. 'abc', 'runs past the end')
raises('a fixed64 past the end', set_path, key(98, 1) .. '1234567', 'runs past the end')
raises('a fixed32 past the end', set_path, key(96, 5) .. '123', 'runs past the end')

print('ok  a decoder skips an unknown field in each skippable wire type')
print('ok  unsigned integers and strings read, and an absent one reads as its zero')
print('ok  a field in the wrong wire type is skipped, and one sent twice keeps the last')
print('ok  a padded length is read')
print('ok  wire types 3, 4, 6 and 7 raise, and so does a body cut short')
