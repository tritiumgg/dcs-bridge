-- The autoexec.cfg lines a policy refusal logs. The hook driver runs the
-- operator's file with its own net table and a stub for every other name,
-- takes the two lists it sets, and writes each key as the file's names in
-- order and then the bridge's names it lacks. A file that does not read
-- gives the bridge's names alone, and the reason. ADR 0046, ADR 0068.
--
-- Each case installs the hook driver into a scratch write directory, under a
-- stock Lua 5.1 with lfs, DCS's log, the DCS namespace and net stubbed.
--
-- Run it through tools/luatest.sh, which builds the module and finds it.

local MODULE = ...
assert(MODULE, 'usage: lua tests/lua/hookpolicy.lua <module path>')

package.path = arg[0]:match('^(.*)/[^/]+$') .. '/support/?.lua;' .. package.path
local hook = require('hook')

-- The README's two lines, for a file that has neither key.
local UNSAFE = 'net.allow_unsafe_api = {"userhooks"}'
local DOSTRING = 'net.allow_dostring_in = {"server", "mission", "scripting", "gui", "export", "config"}'

-- The driver's policy table, the path its file is read from, and the lines
-- it gives for a file holding text, or for no file when text is nil.
local function policy(text)
  local driver, dir = hook.install()
  local path = dir .. 'Config/autoexec.cfg'
  if text then
    hook.write(path, text)
  end
  local lists, reason, absent = driver.policy.read(path)
  return driver.policy.lines(lists), lists, reason, absent, path
end

local function expect(what, lines, unsafe, dostring)
  assert(#lines == 2, what .. ': ' .. #lines .. ' lines, expected 2')
  assert(lines[1] == unsafe, what .. ': the first line was ' .. tostring(lines[1]))
  assert(lines[2] == dostring, what .. ': the second line was ' .. tostring(lines[2]))
end

-- An absent file gives the README's two lines, and says it is absent.
do
  local lines, lists, reason, absent, path = policy(nil)
  assert(lists == nil and absent == true and reason == 'no ' .. path,
    'an absent file read as ' .. tostring(reason))
  expect('an absent file', lines, UNSAFE, DOSTRING)
  print('ok  an absent file gives the bridge\'s two lines')
end

-- A file with one key keeps its names first and gains the rest. The other
-- key, which it lacks, is the bridge's line.
do
  local lines, lists = policy('net.allow_dostring_in = {"server"}\n')
  assert(lists, 'a file with one key did not read')
  expect('one key', lines, UNSAFE, DOSTRING)
  lines = policy('net.allow_unsafe_api = {"other"}\nnet.allow_dostring_in = {"export", "srs"}\n')
  expect('another tool\'s names', lines, 'net.allow_unsafe_api = {"other", "userhooks"}',
    'net.allow_dostring_in = {"export", "srs", "server", "mission", "scripting", "gui", "config"}')
  print('ok  another tool\'s names stay, in place, ahead of the bridge\'s')
end

-- A file that already holds every name gives its own lines back, in its own
-- order, and a name it holds twice or an entry that is not a string is
-- written once or not at all.
do
  local lines = policy('net.allow_unsafe_api = {"userhooks"}\n'
    .. 'net.allow_dostring_in = {"config", "export", "gui", "scripting", "mission", "server"}\n')
  expect('a complete file', lines, UNSAFE,
    'net.allow_dostring_in = {"config", "export", "gui", "scripting", "mission", "server"}')
  lines = policy('net.allow_dostring_in = {"gui", 4, "gui", true, "a \\"quoted\\" name"}\n')
  expect('odd entries', lines, UNSAFE, 'net.allow_dostring_in = {"gui", "a \\"quoted\\" name", '
    .. '"server", "mission", "scripting", "export", "config"}')
  print('ok  a complete file is unchanged, and a name is written once')
end

-- A file that calls DCS's functions, reads its globals, calls one nobody
-- defines and builds a list with table.insert still yields its names. One
-- that replaces net, or starts with a byte-order mark, does too.
do
  local lines, lists, reason = policy('log.set_output("dcs", "", log.ALL, log.FULL)\n'
    .. 'if options.graphics then options.graphics.x = 1 end\n'
    .. 'nobody_defines_this("x")\n'
    .. 'net.allow_dostring_in = {"srs"}\n'
    .. 'table.insert(net.allow_dostring_in, "server")\n')
  assert(lists, 'a file calling an unknown global did not read: ' .. tostring(reason))
  expect('stubbed globals', lines, UNSAFE,
    'net.allow_dostring_in = {"srs", "server", "mission", "scripting", "gui", "export", "config"}')
  lines = policy('\239\187\191if not net then net = {} end\nnet = { allow_unsafe_api = { "own" } }\n')
  expect('a replaced net', lines, 'net.allow_unsafe_api = {"own", "userhooks"}', DOSTRING)
  print('ok  a file that reaches DCS\'s globals still yields its names')
end

-- A file that does not compile or raises anyway gives the bridge's lines
-- and the reason, naming the file.
do
  local cases = {
    { 'net.allow_dostring_in = {', 'does not compile: ' },
    { 'error("deliberate", 0)', 'raised: deliberate' },
    { 'local x = log + 1', 'raised: ' },
  }
  for _, case in ipairs(cases) do
    local lines, lists, reason, absent, path = policy(case[1])
    assert(lists == nil and not absent, case[1] .. ': read as lists')
    assert(reason:find(path .. ' ' .. case[2], 1, true) == 1, case[1] .. ': the reason was ' .. reason)
    expect(case[1], lines, UNSAFE, DOSTRING)
  end
  print('ok  a file that does not compile or raises gives the bridge\'s lines and why')
end

-- A path that opens and reads nothing, as a directory does on this
-- platform, gives the bridge's lines and the reason, and raises nothing.
-- Where a directory does not open, the open's reason covers it already.
do
  local driver, dir = hook.install()
  local path = dir .. 'Config/autoexec.cfg'
  assert(os.execute("mkdir -p '" .. path .. "'") == 0)
  local probe = io.open(path, 'rb')
  if probe then
    probe:close()
    local lists, reason, absent = driver.policy.read(path)
    assert(lists == nil and not absent, 'a directory read as lists')
    assert(reason:find(path .. ' does not read: ', 1, true) == 1, 'the reason was ' .. tostring(reason))
    expect('a directory', driver.policy.lines(lists), UNSAFE, DOSTRING)
    print('ok  a path that reads nothing gives the bridge\'s lines and why')
  else
    print('ok  a directory does not open here, so the open\'s reason covers it')
  end
end

-- Nothing the file does reaches the hook driver's state: not its globals,
-- not the libraries it shares.
do
  local _, lists = policy('DCS = nil\nlfs = nil\nstring.format = nil\ntable.concat = nil\n'
    .. 'net.allow_unsafe_api = {"userhooks"}\n')
  assert(lists, 'a file that assigns globals did not read')
  assert(type(DCS) == 'table' and type(lfs) == 'table', 'the file reached the hook state\'s globals')
  assert(type(string.format) == 'function' and type(table.concat) == 'function',
    'the file reached the hook state\'s libraries')
  print('ok  the file reaches nothing of the hook state')
end

hook.remove()
