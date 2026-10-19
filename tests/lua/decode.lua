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

-- A negative n as a sender writes it into a varint: ten bytes of its 64-bit
-- two's complement. n is at least -2^32.
local function negative(n)
  local lo, hi = 2 ^ 32 + n, 2 ^ 32 - 1
  local out = {}
  for i = 1, 10 do
    local low = lo % 128
    lo = (lo - low) / 128 + hi % 128 * 2 ^ 25
    hi = (hi - hi % 128) / 128
    out[i] = string.char(i < 10 and low + 128 or low)
  end
  return table.concat(out)
end

local signed = set_path(key(4, 0) .. negative(-2 ^ 31) .. key(5, 0) .. negative(-5)
  .. key(6, 0) .. varint(2) .. key(7, 0) .. varint(1))
equal('count at -2^31', signed.count, -2 ^ 31)
equal('offset at -5', signed.offset, -5)
equal('fuze', signed.fuze, 2)
equal('loop', signed.loop, true)
equal('count at -1', set_path(key(4, 0) .. negative(-1)).count, -1)
equal('offset above 2^32', set_path(key(5, 0) .. varint(2 ^ 40 + 1)).offset, 2 ^ 40 + 1)
equal('loop sent as zero', set_path(key(7, 0) .. varint(0)).loop, false)

local absent = set_path('')
equal('an absent count', absent.count, 0)
equal('an absent loop', absent.loop, false)
equal('an absent speed', absent.speed, 0)
equal('an absent optional limit', absent.limit, nil)
equal('an optional limit sent as zero', set_path(key(9, 0) .. varint(0)).limit, 0)

-- Each double as its eight little-endian bytes.
local function speed(bytes)
  return set_path(key(8, 1) .. bytes).speed
end
equal('1.5', speed('\0\0\0\0\0\0\248\63'), 1.5)
equal('-0.25', speed('\0\0\0\0\0\0\208\191'), -0.25)
equal('0', speed(string.rep('\0', 8)), 0)
equal('-0', 1 / speed(string.rep('\0', 7) .. '\128'), -math.huge)
equal('the least subnormal', speed('\1' .. string.rep('\0', 7)), math.ldexp(1, -1074))
equal('infinity', speed('\0\0\0\0\0\0\240\127'), math.huge)
equal('-infinity', speed('\0\0\0\0\0\0\240\255'), -math.huge)
local nan = speed('\0\0\0\0\0\0\248\127')
assert(nan ~= nan, 'NaN decoded as ' .. tostring(nan))

-- Nested messages: origin and waypoints are Points, x and z doubles.
local one_and_a_half, minus_a_quarter = '\0\0\0\0\0\0\248\63', '\0\0\0\0\0\0\208\191'
local point = key(1, 1) .. one_and_a_half .. key(2, 1) .. minus_a_quarter
local nested = set_path(delimited(12, point) .. delimited(11, point)
  .. delimited(11, key(1, 1) .. one_and_a_half))
equal('origin.x', nested.origin.x, 1.5)
equal('origin.z', nested.origin.z, -0.25)
equal('the waypoints', #nested.waypoints, 2)
equal('an absent z in a waypoint', nested.waypoints[2].z, 0)
equal('an absent origin', absent.origin, nil)
equal('absent waypoints', #absent.waypoints, 0)
equal('absent ids', #absent.ids, 0)

-- A message sent twice merges.
local merged = set_path(delimited(12, key(1, 1) .. one_and_a_half)
  .. delimited(12, key(2, 1) .. minus_a_quarter)).origin
equal('a merged x', merged.x, 1.5)
equal('a merged z', merged.z, -0.25)

-- A repeated scalar reads packed and one per tag, mixed in one body.
local ids = set_path(delimited(10, varint(1) .. varint(2)) .. key(10, 0) .. varint(3)
  .. delimited(10, varint(4) .. negative(-1))).ids
equal('the ids', table.concat(ids, ','), '1,2,3,4,-1')

-- A Node levels deep below tree, each holding its level in v.
local function tree(levels)
  local body = key(2, 0) .. varint(levels)
  for level = levels - 1, 1, -1 do
    body = key(2, 0) .. varint(level) .. delimited(1, body)
  end
  return delimited(13, body)
end
local node = set_path(tree(3)).tree
equal('the tree root', node.v, 1)
equal('the third level', node.children[1].children[1].v, 3)

-- SetPath's schema nests two levels deep, tree and then Node within Node,
-- and the margin is 8. ADR 0045.
local max_depth = 10
equal('a tree at the bound', select(2, pcall(set_path, tree(max_depth))).tree.v, 1)
raises('a tree past the bound', set_path, tree(max_depth + 1), 'more than 10 messages deep')

-- At most 4096 elements in one repeated field, however they arrive.
local function ones(n)
  return string.rep(varint(1), n)
end
equal('ids at the cap', #set_path(delimited(10, ones(4096))).ids, 4096)
raises('ids past the cap, packed', set_path, delimited(10, ones(4097)), 'more than 4096')
raises('ids past the cap, in two chunks', set_path,
  delimited(10, ones(2048)) .. delimited(10, ones(2049)), 'more than 4096')
raises('ids past the cap, one per tag', set_path,
  string.rep(key(10, 0) .. varint(1), 4097), 'more than 4096')
raises('waypoints past the cap', set_path, string.rep(delimited(11, ''), 4097), 'more than 4096')

-- Zigzag varints: n reads from 2n, and -n from 2n - 1.
local zigzag = set_path(key(14, 0) .. varint(9) .. key(15, 0) .. varint(2 ^ 41 - 1))
equal('dx at -5', zigzag.dx, -5)
equal('dy at -2^40', zigzag.dy, -2 ^ 40)
equal('dx at 7', set_path(key(14, 0) .. varint(14)).dx, 7)
equal('dy at 2^40', set_path(key(15, 0) .. varint(2 ^ 41)).dy, 2 ^ 40)

-- Fixed widths, little-endian.
local widths = set_path(key(17, 5) .. '\255\255\255\255'
  .. key(18, 1) .. '\255\255\255\255\255\255\31\0'
  .. key(19, 5) .. '\254\255\255\255'
  .. key(20, 1) .. '\254' .. string.rep('\255', 7))
equal('mask', widths.mask, 2 ^ 32 - 1)
equal('seed at 2^53 - 1', widths.seed, 2 ^ 53 - 1)
equal('bias at -2', widths.bias, -2)
equal('drift at -2', widths.drift, -2)

-- Each float as its four little-endian bytes.
local function scale(bytes)
  return set_path(key(16, 5) .. bytes).scale
end
equal('1.5 as a float', scale('\0\0\192\63'), 1.5)
equal('-0.25 as a float', scale('\0\0\128\190'), -0.25)
equal('the least subnormal float', scale('\1\0\0\0'), math.ldexp(1, -149))
equal('infinity as a float', scale('\0\0\128\127'), math.huge)
local float_nan = scale('\0\0\192\127')
assert(float_nan ~= float_nan, 'a float NaN decoded as ' .. tostring(float_nan))

-- A map, each entry a key and a value; a later entry for a key replaces it.
local function entry(name, value)
  return delimited(21, delimited(1, name) .. key(2, 0) .. varint(value))
end
local tags = set_path(entry('a', 1) .. entry('b', 2) .. entry('a', 3)).tags
equal('tags.a', tags.a, 3)
equal('tags.b', tags.b, 2)
equal('absent tags', next(absent.tags), nil)
raises('a map past the cap', set_path, string.rep(entry('a', 1), 4097), 'more than 4096')

-- A tree sent twice merges its labels map, and the cap counts both halves.
local function labels(from, to)
  local out = {}
  for i = from, to do
    out[#out + 1] = delimited(3, delimited(1, 'k' .. i) .. key(2, 0) .. varint(i))
  end
  return delimited(13, table.concat(out))
end
local labelled = set_path(labels(1, 2) .. labels(3, 4)).tree.labels
equal('a merged label', labelled.k1 + labelled.k4, 5)
raises('a merged map past the cap', set_path, labels(1, 2048) .. labels(2049, 4097),
  'more than 4096')

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
print('ok  int32, int64 and enum read negatives from ten bytes, and bool reads')
print('ok  a double reads, zeros, the infinities, NaN and a subnormal included')
print('ok  an absent optional field reads as nil, and one sent as zero as zero')
print('ok  nested messages read and merge, and a repeated scalar reads packed or not')
print('ok  a body past the depth bound or the element cap raises')
print('ok  zigzag, fixed-width, float and map fields read')
print('ok  a padded length is read')
print('ok  wire types 3, 4, 6 and 7 raise, and so does a body cut short')
