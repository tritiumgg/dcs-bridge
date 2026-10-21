-- The bridge's own log stops growing at its cap, and the files it rotates out
-- stay under the same cap together, oldest deleted first. ADR 0047.
--
-- Real files in a scratch directory, under a stock Lua 5.1 with lfs and DCS's
-- log stubbed. Run it through tools/luatest.sh.

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local dcs = require('dcs')

local DRIVER = dcs.root() .. 'lua/Mods/services/DCSBridge/lua/HookDriver.lua'

local scratch = {}

-- A fresh hook driver over a fresh directory, so no case sees another's
-- logger or files.
local function fresh()
  local dir = dcs.scratch()
  scratch[#scratch + 1] = dir
  lfs = dcs.lfs(dir)
  log = dcs.log()
  local driver = dofile(DRIVER)
  return driver, dir .. 'DCSBridge'
end

local function read(path)
  local f = assert(io.open(path, 'rb'))
  local text = f:read('*a')
  f:close()
  return text
end

local function rotated(dir)
  local names = {}
  for name in lfs.dir(dir) do
    if name:match('^DCSBridge%..+%.log$') then
      names[#names + 1] = name
    end
  end
  table.sort(names)
  return names
end

-- A message that makes a 100-byte line once the stamp and level are on it:
-- "YYYY-MM-DD HH:MM:SS INFO " is 25 bytes and the newline is one.
local function message(i)
  local text = string.format('line %03d ', i)
  return text .. string.rep('x', 74 - #text)
end

-- 520 lines of 100 bytes against a 4096-byte cap, with the clock held on one
-- second. The live file rotates before every fortieth line, so twelve files
-- rotate out with the same stamp: the unsuffixed name, then -2 to -12. Only
-- one fits the cap, and it has to be the newest each time. A sort on the raw
-- name gets that wrong twice: "-2.log" sorts before ".log", and "-10" before
-- "-9".
do
  local driver, dir = fresh()
  driver.log.configure({ log_max_bytes = 4096 })
  driver.log.open(dir)

  local date = os.date
  local now = os.time()
  os.date = function(format, t)
    return date(format, t or now)
  end
  for i = 1, 520 do
    driver.log.info(message(i))
    local live = lfs.attributes(dir .. '/DCSBridge.log', 'size')
    assert(live <= 4096, 'the live file reached ' .. live .. ' bytes after line ' .. i)
  end
  os.date = date

  local names = rotated(dir)
  assert(#names == 1, #names .. ' rotated files survive, expected 1')
  assert(names[1]:find('%-12%.log$'), 'the survivor is ' .. names[1] .. ', expected the -12')
  local kept = read(dir .. '/' .. names[1])
  assert(#kept <= 4096, 'the rotated file holds ' .. #kept .. ' bytes')
  assert(kept:find('line 441 ', 1, true), 'the newest rotated file was deleted')

  local live = read(dir .. '/DCSBridge.log')
  assert(live:find('line 481 ', 1, true) and live:find('line 520 ', 1, true),
    'the live file does not hold the last forty lines')
  assert(live:find('^%d%d%d%d%-%d%d%-%d%d %d%d:%d%d:%d%d INFO line 481 '),
    'a line does not start with its UTC stamp and level')
end

-- A line below log_level is dropped, and one at it is written.
do
  local driver, dir = fresh()
  driver.log.open(dir)
  driver.log.debug('below the level')
  driver.log.warn('above the level')
  local live = read(dir .. '/DCSBridge.log')
  assert(not live:find('below the level', 1, true), 'a debug line was written at info')
  assert(live:find('WARN above the level', 1, true), 'a warn line was dropped at info')

  driver.log.configure({ log_level = 'debug' })
  driver.log.debug('now at the level')
  assert(read(dir .. '/DCSBridge.log'):find('DEBUG now at the level', 1, true),
    'a debug line was dropped at debug')
end

-- A message with line breaks is one line, each break shown as ' | ', so no
-- line in the file lacks its stamp and level.
do
  local driver, dir = fresh()
  driver.log.open(dir)
  driver.log.error('a\nb\r\nc\rd')
  driver.log.info('after')
  local live = read(dir .. '/DCSBridge.log')
  local n = 0
  for line in live:gmatch('[^\n]+') do
    n = n + 1
    assert(line:find('^%d%d%d%d%-%d%d%-%d%d %d%d:%d%d:%d%d %u+ '), 'a line has no stamp: ' .. line)
  end
  assert(n == 2, n .. ' lines, expected 2')
  assert(live:find('ERROR a | b | c | d\n', 1, true), 'the breaks were not replaced: ' .. live)
end

-- A key configure is not given keeps its value, and an unknown level raises.
do
  local driver, dir = fresh()
  driver.log.configure({ log_level = 'error' })
  driver.log.configure({ log_max_bytes = 4096 })
  driver.log.open(dir)
  driver.log.warn('dropped at error')
  assert(read(dir .. '/DCSBridge.log') == '', 'log_level did not keep its value')

  local ok, err = pcall(driver.log.configure, { log_level = 'verbose' })
  assert(not ok and err:find('log_level', 1, true), 'an unknown level was accepted')
end

-- A line longer than the cap lands alone in an empty file rather than being
-- dropped, and the next line rotates it out.
do
  local driver, dir = fresh()
  driver.log.configure({ log_max_bytes = 64 })
  driver.log.open(dir)
  driver.log.info(string.rep('y', 100))
  local live = lfs.attributes(dir .. '/DCSBridge.log', 'size')
  assert(live == 126, 'the long line left ' .. live .. ' bytes')
  driver.log.info('short')
  live = lfs.attributes(dir .. '/DCSBridge.log', 'size')
  assert(live < 64, 'the line after the long one did not rotate it out')
end

-- A rename that fails truncates the live file so the cap holds, and says so
-- once in dcs.log.
do
  local driver, dir = fresh()
  driver.log.configure({ log_max_bytes = 4096 })
  driver.log.open(dir)
  local rename = os.rename
  os.rename = function()
    return nil, 'held open'
  end
  for i = 1, 41 do
    driver.log.info(message(i))
  end
  os.rename = rename

  local live = read(dir .. '/DCSBridge.log')
  assert(#live == 100 and live:find('line 041 ', 1, true),
    'a failed rename did not leave only the newest line')
  assert(#rotated(dir) == 0, 'a failed rename left a rotated file')
  assert(#log.lines == 1, #log.lines .. ' dcs.log lines, expected 1')
  assert(log.lines[1].level == 'ERROR'
    and log.lines[1].message:find('held open', 1, true),
    'dcs.log does not name the failed rename')
end

-- A reopen that fails after a rotation drops lines rather than raising into
-- the caller, says so once in dcs.log, and the logger writes again once the
-- file can be opened.
do
  local driver, dir = fresh()
  driver.log.configure({ log_max_bytes = 4096 })
  driver.log.open(dir)
  for i = 1, 40 do
    driver.log.info(message(i))
  end

  local open = io.open
  io.open = function(path, mode)
    if path:find('DCSBridge.log$') and mode ~= 'r' and mode ~= 'rb' then
      return nil, 'denied'
    end
    return open(path, mode)
  end
  local ok, err = pcall(driver.log.info, message(41))
  local again = pcall(driver.log.info, message(42))
  io.open = open

  assert(ok, 'a failed reopen raised: ' .. tostring(err))
  assert(again, 'a line after a failed reopen raised')
  assert(#log.lines == 1 and log.lines[1].message:find('denied', 1, true),
    'dcs.log does not name the failed reopen once')

  driver.log.info(message(43))
  local live = read(dir .. '/DCSBridge.log')
  assert(live:find('line 043 ', 1, true), 'the logger did not write again')
  assert(not live:find('line 041 ', 1, true), 'a dropped line was written')
end

-- Every open for writing is binary. On Windows text mode writes a newline as
-- two bytes, which the cap would not count, and this host cannot see that.
do
  local driver, dir = fresh()
  driver.log.configure({ log_max_bytes = 4096 })
  local modes = {}
  local open = io.open
  io.open = function(path, mode)
    if path:find('DCSBridge.log$') then
      modes[#modes + 1] = mode
    end
    return open(path, mode)
  end
  driver.log.open(dir)
  for i = 1, 41 do
    driver.log.info(message(i))
  end
  io.open = open

  local writes = 0
  for _, mode in ipairs(modes) do
    if mode ~= 'r' and mode ~= 'rb' then
      writes = writes + 1
      assert(mode:find('b', 1, true), 'the log was opened in text mode ' .. mode)
    end
  end
  assert(writes == 2, writes .. ' opens for writing, expected the open and the rotation')
end

-- Opening the logger deletes a rotated file older than log_retention_days,
-- keeps a recent one, and leaves a name that does not parse alone.
do
  local driver, dir = fresh()
  lfs.mkdir(dir)
  local recent = 'DCSBridge.' .. os.date('!%Y%m%d-%H%M%S', os.time() - 86400) .. '.log'
  for _, name in ipairs({ 'DCSBridge.20200101-000000.log', recent, 'DCSBridge.notastamp.log' }) do
    local f = assert(io.open(dir .. '/' .. name, 'w'))
    f:write('old\n')
    f:close()
  end

  driver.log.configure({ log_retention_days = 14 })
  driver.log.open(dir)
  local names = rotated(dir)
  assert(#names == 2, #names .. ' files left, expected 2')
  assert(names[1] == recent, 'a file a day old was deleted')
  assert(names[2] == 'DCSBridge.notastamp.log', 'a name that does not parse was deleted')

  driver.log.configure({ log_retention_days = 0 })
  driver.log.open(dir)
  names = rotated(dir)
  assert(#names == 1 and names[1] == 'DCSBridge.notastamp.log',
    'a retention of zero days kept a rotated file')
end

for _, dir in ipairs(scratch) do
  dcs.remove(dir)
end

print('the bridge log holds to its cap')
