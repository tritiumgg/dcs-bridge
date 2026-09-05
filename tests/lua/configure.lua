-- shim.configure, from a stock Lua 5.1.
--
-- The call hands the broker its keys from Config\DCSBridge.lua, applied as
-- one swap or refused whole. What a key does once in force is not
-- observable from here: the Rust tests read the limits off a socket. What
-- this checks is the Lua side of the call: it is on the table and answers
-- the interface version; a well-formed table is accepted and the answer
-- counts what it did; a bad value is refused naming the key and leaves the
-- whole table unapplied; a later call reports a changed restart-tier key
-- as pending and applies the live ones; and the tokens list is read as it
-- was, with capabilities by name or by number, a bad entry refused by
-- number, and a hole refused by the position that is missing.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/configure.lua <module path>')

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
assert(type(shim.configure) == 'function', 'shim.configure is missing')
assert(type(shim.interface) == 'string', 'shim.interface is missing')
assert(shim.tokens == nil, 'shim.tokens is still on the table')

local function raises(what, f, ...)
  local ok, err = pcall(f, ...)
  assert(not ok, what .. ' did not raise')
  return err
end

local function complains(what, table, about)
  local err = raises(what, shim.configure, table)
  assert(err:find('configure refused: ', 1, true), what .. ': not a refusal: ' .. err)
  assert(err:find(about, 1, true), what .. ': the wrong complaint: ' .. err)
end

-- Before the first call nothing listens and no record can be opened: a
-- begin raises saying so, and a put or a commit says no record is open.
local early = raises('begin before configure', shim.begin, 't')
assert(early:find('configure comes first', 1, true), 'the wrong complaint: ' .. early)
raises('begin_to before configure', shim.begin_to, 1, 'dcs.bridge.CommandAck')
assert(raises('integer before configure', shim.integer, 1, 1):find('no record is open', 1, true))
assert(raises('commit before configure', shim.commit):find('no record is open', 1, true))

-- The first call: every tier applies, a key the broker does not own is
-- counted, the listener binds on the port the table names, and the answer
-- carries the interface version the hook driver compares and the address
-- bound. Port 0 lets the system pick, so the run collides with nothing.
local first = shim.configure({
  port = 0,
  max_connections = 8,
  max_unauthenticated_connections = 2,
  handshake_timeout_ms = 2500,
  enabled = true,
  route = 'A',
  tokens = {
    { id = 'map', secret = 'read-only-secret', caps = { 'read' } },
    { id = 'bot', secret = 'bot-secret', caps = { 'read', 'command' } },
    { id = 'admin', secret = 'admin-secret', caps = { 1, 2, 3 } },
  },
})
assert(first.interface == shim.interface, 'the answer and the table disagree on interface')
assert(first.applied == 4, 'the first call applied ' .. tostring(first.applied))
assert(first.unknown == 1, 'the first call counted ' .. tostring(first.unknown) .. ' unknown')
assert(first.pending_restart == 0, 'the first call left something pending')
assert(#first.pending == 0)
assert(
  type(first.listening) == 'string' and first.listening:find('^127%.0%.0%.1:%d+$'),
  'listening reads ' .. tostring(first.listening)
)
assert(first.listening ~= '127.0.0.1:0', 'the answer carries the port asked for, not the one bound')

-- A record opens once configured, and queues.
shim.begin('dcs.builtin.UnitDestroyed')
assert(shim.commit() == true, 'a record after configure did not queue')

-- A later call: the live keys apply and a changed restart-tier key is
-- reported with both values, not applied. An unchanged one is not pending.
local later = shim.configure({
  port = 7743,
  max_connections = 8,
  max_unauthenticated_connections = 3,
  tokens = {},
})
assert(later.applied == 2, 'the later call applied ' .. tostring(later.applied))
assert(later.pending_restart == 1, 'pending_restart is ' .. tostring(later.pending_restart))
assert(later.pending[1].key == 'port', 'pending names ' .. tostring(later.pending[1].key))
assert(later.pending[1].effective == 0 and later.pending[1].file == 7743)
assert(later.listening == first.listening, 'a later call rebound the listener')
assert(later.unknown == 0)

-- The frame cap is live, and the record buffer follows it at the next
-- begin: under a cap of 64 bytes a 200-byte string outgrows the buffer
-- and the put raises, and once the cap is back at its default the same
-- record queues.
shim.configure({ max_frame_bytes = 64 })
shim.begin('dcs.builtin.UnitDestroyed')
local full = raises('a put over the lowered cap', shim.string, 1, string.rep('x', 200))
assert(full:find('outgrew its buffer', 1, true), 'the wrong complaint: ' .. full)
shim.configure({})
shim.begin('dcs.builtin.UnitDestroyed')
shim.string(1, string.rep('x', 200))
assert(shim.commit() == true, 'the record buffer did not follow the cap back up')

-- An empty table is a valid configuration: every live key at its default,
-- and no token, which is what a bridge nobody can authenticate to looks
-- like. A token granting nothing is a valid entry: the broker refuses it
-- at Auth, where the operator's mistake bites.
shim.configure({})
shim.configure({ tokens = { { id = 'useless', secret = 'x', caps = {} } } })

-- The argument is a table and every key a string.
raises('configure with no argument', shim.configure)
raises('configure with a string', shim.configure, 'port')
complains('a number key', { [1] = 7742 }, 'a key is not a string')

-- A bad value refuses the whole call, naming the key and what it takes.
-- That a good key before the bad one did not apply is not observable from
-- Lua; the Rust test over the bridge shows it.
complains('a fraction', { port = 1.5 }, '`port` must be a whole number from 0 to 65535')
complains('a string number', { port = '7742' }, '`port` must be a number')
complains('a wrong boolean', { enabled = 1 }, '`enabled` must be true or false')
complains('a bad address', { bind_address = 'localhost' }, '`bind_address` must be an IP address')
complains('a nested table', { options = { a = 1 } }, '`options` is a table')
complains('a function', { port = print }, '`port` is not a number, a string or a boolean')
complains(
  'an invariant',
  { max_unauthenticated_connections = 8 },
  '`max_unauthenticated_connections` must be below `max_connections`'
)
complains('a good key before a bad one', { enabled = false, port = -1 }, '`port` must be')

-- The tokens list, as shim.tokens read it. Each entry names its id, its
-- secret and its caps, and the complaint says which entry, so the third
-- bad entry in a list of three is entry 3.
local function tokens(list)
  return { tokens = list }
end
complains('tokens not a list', { tokens = 'map' }, '`tokens` must be a list of token entries')
complains('a bare string entry', tokens({ 'map' }), '`tokens` entry 1 is not a table')

local entry = { id = 'a', secret = 's', caps = { 'read' } }
complains('a hole in the list', tokens({ [1] = entry, [3] = entry }), 'entry 2 is missing')
complains('a named key in the list', tokens({ entry, map = entry }), 'entry 2 is missing')
complains(
  'a hole in a caps list',
  tokens({ { id = 'a', secret = 's', caps = { [1] = 'read', [3] = 'reload' } } }),
  'entry 1 has no caps list'
)
complains('an entry with no id', tokens({ { secret = 's', caps = {} } }), 'entry 1 has no id string')
complains('an entry with no secret', tokens({ { id = 'a', caps = {} } }), 'entry 1 has no secret string')
complains('an entry with an empty secret', tokens({ { id = 'a', secret = '', caps = {} } }), 'entry 1 has an empty secret')
complains('an entry with no caps', tokens({ { id = 'a', secret = 's' } }), 'entry 1 has no caps list')
complains('an unknown capability', tokens({ { id = 'a', secret = 's', caps = { 'admin' } } }), 'entry 1 has no caps list')
complains('a capability number outside the three', tokens({ { id = 'a', secret = 's', caps = { 4 } } }), 'entry 1 has no caps list')
complains(
  'the third entry bad',
  tokens({
    { id = 'a', secret = 's', caps = { 'read' } },
    { id = 'b', secret = 's', caps = { 'read' } },
    { id = 'c', secret = 's' },
  }),
  'entry 3 has no caps list'
)

print('ok  configure is on the table and answers the interface version')
print('ok  nothing opens a record before the first call, which binds and allocates')
print('ok  the first call applies every tier and a later one the live keys')
print('ok  a changed restart-tier key is reported pending with both values')
print('ok  a bad value is refused naming the key')
print('ok  the tokens list is read as before, a bad entry refused by number')
