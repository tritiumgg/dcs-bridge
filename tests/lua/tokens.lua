-- shim.tokens, from a stock Lua 5.1.
--
-- The call hands the broker its consumer credentials, replaced whole. What
-- a token lets a consumer do is not observable from here: the Rust tests
-- authenticate against the table off a socket. What this checks is the Lua
-- side of the call: it is on the table, a well-formed list is accepted with
-- capabilities by name or by number, an empty list is accepted, and a bad
-- entry is refused with an error naming the entry and leaving the whole
-- list unapplied.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/tokens.lua <module path>')

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
assert(type(shim.tokens) == 'function', 'shim.tokens is missing')

local function raises(what, f, ...)
  local ok, err = pcall(f, ...)
  assert(not ok, what .. ' did not raise')
  return err
end

local function complains(what, list, about)
  local err = raises(what, shim.tokens, list)
  assert(err:find(about, 1, true), what .. ': the wrong complaint: ' .. err)
end

-- A well-formed list, with capabilities written both ways.
shim.tokens({
  { id = 'map', secret = 'read-only-secret', caps = { 'read' } },
  { id = 'bot', secret = 'bot-secret', caps = { 'read', 'command' } },
  { id = 'admin', secret = 'admin-secret', caps = { 1, 2, 3 } },
})

-- An empty list clears the table, and a token granting nothing is a valid
-- entry here: the broker refuses it at Auth, with its own error, so the
-- operator's mistake is reported where it bites.
shim.tokens({})
shim.tokens({ { id = 'useless', secret = 'x', caps = {} } })

-- The argument is a table, and each entry is one.
raises('tokens with no argument', shim.tokens)
raises('tokens with a string', shim.tokens, 'map')
complains('a bare string entry', { 'map' }, 'entry 1 is not a table')

-- Each entry names its id, its secret and its caps, and the complaint says
-- which entry, so the third bad entry in a list of three is entry 3.
complains('an entry with no id', { { secret = 's', caps = {} } }, 'entry 1 has no id string')
complains('an entry with no secret', { { id = 'a', caps = {} } }, 'entry 1 has no secret string')
complains('an entry with an empty secret', { { id = 'a', secret = '', caps = {} } }, 'entry 1 has an empty secret')
complains('an entry with no caps', { { id = 'a', secret = 's' } }, 'entry 1 has no caps list')
complains('an unknown capability', { { id = 'a', secret = 's', caps = { 'admin' } } }, 'entry 1 has no caps list')
complains('a capability number outside the three', { { id = 'a', secret = 's', caps = { 4 } } }, 'entry 1 has no caps list')
complains(
  'the third entry bad',
  {
    { id = 'a', secret = 's', caps = { 'read' } },
    { id = 'b', secret = 's', caps = { 'read' } },
    { id = 'c', secret = 's' },
  },
  'entry 3 has no caps list'
)

print('ok  tokens is on the table and a well-formed list is accepted')
print('ok  capabilities are read by name and by number')
print('ok  a bad entry is refused with an error naming the entry')
