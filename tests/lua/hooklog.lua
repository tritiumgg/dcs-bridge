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

-- 100 lines of 100 bytes against a 4096-byte cap. The live file rotates
-- before the 41st and the 81st, so two files rotate out, in the same second
-- and so with a suffix on the second. Only one fits the cap, and it has to be
-- the newer, which a sort on the raw name gets wrong: "-2.log" sorts before
-- ".log".
do
  local driver, dir = fresh()
  driver.log.configure({ log_max_bytes = 4096 })
  driver.log.open(dir)

  for i = 1, 100 do
    driver.log.info(message(i))
    local live = lfs.attributes(dir .. '/DCSBridge.log', 'size')
    assert(live <= 4096, 'the live file reached ' .. live .. ' bytes after line ' .. i)
  end

  local names = rotated(dir)
  assert(#names == 1, #names .. ' rotated files survive, expected 1')
  local kept = read(dir .. '/' .. names[1])
  assert(#kept <= 4096, 'the rotated file holds ' .. #kept .. ' bytes')
  assert(kept:find('line 041 ', 1, true), 'the newer rotated file was deleted')
  assert(not kept:find('line 001 ', 1, true), 'the older rotated file survived')

  local live = read(dir .. '/DCSBridge.log')
  assert(live:find('line 081 ', 1, true) and live:find('line 100 ', 1, true),
    'the live file does not hold the last twenty lines')
  assert(live:find('^%d%d%d%d%-%d%d%-%d%d %d%d:%d%d:%d%d INFO line 081 '),
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

for _, dir in ipairs(scratch) do
  dcs.remove(dir)
end

print('the bridge log holds to its cap')
