-- The cost of one put call crossing from Lua into the broker.
--
-- The broker encodes, so what remains on the logic thread is the crossing
-- itself: one call from Lua into C per field, per record. This times N calls
-- of each put in a tight loop and reports microseconds per call, with the
-- bytes the Lua heap grew per call beside it. A stock string.len call is timed
-- the same way, as the generic crossing this machine pays with no broker in
-- it, so a reading can be placed against the proxy the budget rests on.
--
-- It asserts nothing. A figure from a shared machine is a reading rather than
-- a verdict, and the reading is the maintainer's. tests/lua/alloc.lua is where
-- the allocation column is asserted.
--
-- Each row is its own loop with the calls written inline and the functions
-- hoisted into locals, because a generated emitter does the same and a
-- closure call or a table lookup per put would be the harness's cost rather
-- than the crossing's. The empty row is what the loop alone costs.
--
-- Run it through tools/putcost.sh, which builds the module in release and
-- finds it. Inside DCS, set PUTCOST_MODULE to the DLL's path and dofile this;
-- docs/decisions/0013-one-put-call-per-field.md gives the steps.

local path, count = ...
path = path or PUTCOST_MODULE
assert(path, 'usage: lua tools/putcost.lua <module path> [calls]')
local N = tonumber(count) or 1000000

-- os.clock where the state has os. The mission scripting state does not, and
-- timer.getTime is what it has: model time, which advances with the wall
-- clock while the mission runs unpaused.
local clock = (os and os.clock) or (timer and timer.getTime)
assert(clock, 'neither os.clock nor timer.getTime is available')

-- Where the report goes. Inside DCS, log.write lands in dcs.log.
local function say(line)
  if log and log.write then
    log.write('putcost', log.INFO, line)
  else
    print(line)
  end
end

-- tests/lua/load.lua says why both spellings are tried.
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
local len = string.len

local topic = 'dcs.builtin.UnitDestroyed'
local text = 'a string that already exists'

-- A million puts into one record would outgrow its buffer, so each row runs
-- in chunks and a fresh record is begun between them. That is one begin in
-- every CHUNK calls, so it is in the figure at a four-thousandth of its cost.
local CHUNK = 4096
local chunks = math.floor(N / CHUNK)
N = chunks * CHUNK

-- Seconds for N runs of the loop in `run`, timed after one chunk so the first
-- call's stack growth is not in the figure.
local function seconds(run)
  begin(topic)
  run(CHUNK)
  local start = clock()
  for _ = 1, chunks do
    begin(topic)
    run(CHUNK)
  end
  return clock() - start
end

-- Kilobytes the heap grew across N runs, collector stopped. The count reports
-- memory in use rather than bytes allocated, so this is the one way reference
-- Lua 5.1 gives to read allocation.
local function growth(run)
  begin(topic)
  run(CHUNK)
  collectgarbage('collect')
  collectgarbage('stop')
  local before = collectgarbage('count')
  for _ = 1, chunks do
    begin(topic)
    run(CHUNK)
  end
  local after = collectgarbage('count')
  collectgarbage('restart')
  return after - before
end

local rows = {}

-- One row: what was timed, how many crossings one pass of the loop makes,
-- and the loop. us/run is one pass as the emitter would pay it; us/cross is
-- one crossing's share of it.
local function row(what, crossings, run)
  local us = seconds(run) * 1e6 / N
  local bytes = growth(run) * 1024 / N
  rows[#rows + 1] = string.format(
    '%-28s %10.3f %10.3f %10.1f',
    what,
    us,
    us / crossings,
    bytes
  )
end

-- The loop with nothing in it, so the reader can see what the harness costs.
row('empty loop', 1, function(n)
  for _ = 1, n do
  end
end)

-- A stock C function on an existing string: the generic crossing.
row('string.len (proxy)', 1, function(n)
  for _ = 1, n do
    len(text)
  end
end)

row('integer', 1, function(n)
  for _ = 1, n do
    integer(1, 3)
  end
end)
row('double', 1, function(n)
  for _ = 1, n do
    double(2, 1.5)
  end
end)
row('string, 28 bytes', 1, function(n)
  for _ = 1, n do
    string_(3, text)
  end
end)
row('boolean', 1, function(n)
  for _ = 1, n do
    boolean(4, true)
  end
end)
row('message + end_message', 2, function(n)
  for _ = 1, n do
    message(5)
    end_message()
  end
end)

row('begin + commit, empty', 2, function(n)
  for _ = 1, n do
    begin(topic)
    commit()
  end
end)

-- A record the size the sim driver emits: ten scalar fields between a begin
-- and a commit, twelve crossings.
row('ten-field record', 12, function(n)
  for _ = 1, n do
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
    commit()
  end
end)

say(string.format('%d calls per row', N))
say(string.format('%-28s %10s %10s %10s', 'row', 'us/run', 'us/cross', 'bytes'))
for _, line in ipairs(rows) do
  say(line)
end
