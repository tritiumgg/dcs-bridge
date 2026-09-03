-- A put call allocates nothing in Lua.
--
-- The broker writes into a preallocated buffer, and a put call passes a value
-- that already exists: a number, a string, a boolean. So the Lua heap should
-- not move across a run of them. If it does, the shim is building a value
-- rather than passing one, and the emitter would pay a collector step per
-- field on top of the crossing. This measures it the one way reference Lua
-- 5.1 allows: collectgarbage('count') reports memory in use rather than bytes
-- allocated, so the delta is taken with the collector stopped, and a delta of
-- zero then means zero allocation.
--
-- commit is the exception. It returns the body as a Lua string, which
-- allocates once per call, until the commit ring takes the body instead.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local path = ...
assert(path, 'usage: lua tests/lua/alloc.lua <module path>')

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
local begin, integer, double, string_, boolean =
  shim.begin, shim.integer, shim.double, shim.string, shim.boolean
local message, end_message, commit = shim.message, shim.end_message, shim.commit

local N = 10000
local topic = 'dcs.builtin.UnitDestroyed'
local text = 'a string that already exists'

-- Kilobytes the heap grew across N runs of f, collector stopped. Each f is run
-- once first, so the stack and the string table are settled before the count.
local function growth(f)
  f()
  collectgarbage('collect')
  collectgarbage('stop')
  local before = collectgarbage('count')
  for _ = 1, N do
    f()
  end
  local after = collectgarbage('count')
  collectgarbage('restart')
  return after - before
end

local function zero(what, f)
  local kb = growth(f)
  assert(kb == 0, what .. ' grew the heap by ' .. kb * 1024 / N .. ' bytes per call')
end

begin(topic)
zero('integer', function()
  integer(1, 3)
end)
zero('double', function()
  double(2, 1.5)
end)
zero('string', function()
  string_(3, text)
end)
zero('boolean', function()
  boolean(4, true)
end)
zero('message and end_message', function()
  message(5)
  end_message()
end)

-- begin discards the record built above and every one after it.
zero('begin', function()
  begin(topic)
end)

-- A whole record, less the commit: what one emitter call costs the heap.
zero('a ten-field record', function()
  begin(topic)
  integer(1, 3)
  integer(2, 4)
  double(3, 1.5)
  double(4, 2.5)
  string_(5, text)
  string_(6, topic)
  boolean(7, true)
  boolean(8, false)
  integer(9, -1)
  double(10, 0)
end)

-- commit allocates the returned string and nothing else. Lua interns short
-- strings, so the body is made long enough to be allocated on every call, and
-- the check is that the growth is the body, the wrapper naming the topic,
-- and nothing more.
local body = text .. text
local wrapper = #('type.googleapis.com/' .. topic) + 16
local kb = growth(function()
  begin(topic)
  string_(1, body)
  commit()
end)
local per_call = kb * 1024 / N
assert(
  per_call > 0 and per_call < #body + wrapper + 64,
  'commit grew the heap by ' .. per_call .. ' bytes per call for a ' .. #body .. ' byte body'
)

print('ok  the six puts and begin allocate nothing, over ' .. N .. ' calls each')
print('ok  a ten-field record allocates nothing')
print('ok  commit allocates its returned body and nothing more')
