-- shim.schema, from a stock Lua 5.1.
--
-- The call hands the broker the bytes of schema.pb once, after the first
-- configure, and answers their SHA-256 in hex. What the broker serves from
-- them is not observable from here: the Rust tests read it off a socket.
-- What this checks is the Lua side of the call: it is on the table; before
-- configure it is refused saying so; empty bytes are refused; a string of
-- bytes is accepted and the answer is the hash a person can compare
-- against the file; a second call is refused as held; and an argument
-- that is not a string raises rather than being taken as one.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/schema.lua <module path>')

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
assert(type(shim.schema) == 'function', 'shim.schema is missing')

local function raises(what, f, ...)
  local ok, err = pcall(f, ...)
  assert(not ok, what .. ' did not raise')
  return err
end

local function refused(what, bytes, about)
  local err = raises(what, shim.schema, bytes)
  assert(err:find('schema refused: ', 1, true), what .. ': not a refusal: ' .. err)
  assert(err:find(about, 1, true), what .. ': the wrong complaint: ' .. err)
end

-- The hook driver's start is configure then schema, and the other order
-- is refused naming the call that comes first.
refused('schema before configure', 'abc', 'configure comes first')

-- Port 0 lets the system pick, so the run collides with nothing.
shim.configure({ port = 0 })

-- An empty read is a file that was not found, and is refused as such
-- rather than served as an empty set.
refused('an empty schema', '', 'empty')

-- The hash is the one every SHA-256 gives for these three bytes, in
-- lowercase hex, which is what a person compares against the file.
local hash = shim.schema('abc')
assert(
  hash == 'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
  'the hash of abc reads ' .. tostring(hash)
)

-- A second hand-off changes nothing: replacing the served set is a DCS
-- restart, and the refusal says so.
refused('a second schema', 'abd', 'held already')

-- A number would convert to its decimal string under a looser check, and
-- a table is not bytes at all; both are calls that meant something else.
local number = raises('schema with a number', shim.schema, 42)
assert(number:find('string expected', 1, true), 'the wrong complaint: ' .. number)
raises('schema with a table', shim.schema, { 'abc' })
raises('schema with nothing', shim.schema)

print('ok  schema is on the table and is refused before configure')
print('ok  an empty schema is refused')
print('ok  the bytes are accepted once and the answer is their SHA-256 in hex')
print('ok  a second schema is refused as held')
print('ok  an argument that is not a string raises')
