-- schema_sha256 at configure and at shim.classes, from a stock Lua 5.1.
--
-- The hook driver's generated hash rides the configure table and is no
-- configuration key, so it counts as nothing unknown. The sim driver's is
-- the second argument of shim.classes: equal to the held schema's hash, the
-- call answers the count alone; different, it answers the count and a
-- warning naming both, and the rows are registered all the same. A second
-- argument that is not a hash raises before the broker is asked. ADR 0052,
-- ADR 0053.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/hash.lua <module path>')

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

-- The hash of abc, which the generator's golden files carry.
local ABC = 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad'
local OTHER = string.rep('0', 64)

-- Port 0 lets the system pick, so the run collides with nothing.
local answer = shim.configure({ port = 0, schema_sha256 = ABC })
assert(answer.unknown == 0, 'schema_sha256 counted as ' .. answer.unknown .. ' unknown')

local ok, err = pcall(shim.configure, { schema_sha256 = 'abc' })
assert(not ok and err:find('`schema_sha256`', 1, true), 'a short hash: ' .. tostring(err))

assert(shim.schema('abc') == ABC, 'the held hash is not the hash of abc')

local added, warning = shim.classes({ ['adopter.Same'] = 'lossy' }, ABC)
assert(added == 1 and warning == nil, 'a matching hash answered ' .. tostring(warning))

added, warning = shim.classes({ ['adopter.Other'] = 'durable' }, OTHER)
assert(added == 1, 'a mismatched table was not registered')
assert(warning and warning:find(ABC, 1, true) and warning:find(OTHER, 1, true),
  'the warning does not name both hashes: ' .. tostring(warning))

added, warning = shim.classes({ ['adopter.Unhashed'] = 'lossy' })
assert(added == 1 and warning == nil, 'a table with no hash answered ' .. tostring(warning))

for _, bad in ipairs({ 'abcdef0123', string.rep('g', 64), 7 }) do
  ok, err = pcall(shim.classes, { ['adopter.Refused'] = 'lossy' }, bad)
  assert(not ok and err:find('classes refused: the schema hash', 1, true),
    tostring(bad) .. ' as a hash: ' .. tostring(err))
end
added = shim.classes({ ['adopter.Refused'] = 'lossy' })
assert(added == 1, 'a refused call registered its rows')

print('schema_sha256 rides configure and classes, and a mismatch is a warning')
