-- The parts of DCS's hook state the hook driver reaches, for a stock Lua 5.1
-- with no DCS present. Lives outside tests/lua so tools/luatest.sh does not
-- run it as a test.
--
-- lfs is DCS's own binding and stock Lua has none, so this one is built over
-- io, os and the shell. It reaches real files in a scratch directory, which
-- is what the rotation tests need. Linux and macOS only, as luatest.sh is.

local dcs = {}

-- 2023-11-14 22:13:20 UTC, and a different wall-clock time in any zone but
-- UTC, so a banner in local time would not match.
dcs.MODIFIED = 1700000000

local function quote(path)
  return "'" .. path:gsub("'", "'\\''") .. "'"
end

-- A scratch directory with a trailing slash, as lfs.writedir returns one.
function dcs.scratch()
  local path = os.tmpname()
  os.remove(path)
  assert(os.execute('mkdir -p ' .. quote(path)) == 0, 'no scratch directory')
  return path .. '/'
end

function dcs.remove(path)
  os.execute('rm -rf ' .. quote(path))
end

function dcs.lfs(writedir)
  local lfs = {}

  function lfs.writedir()
    return writedir
  end

  -- A path that is not a directory raises, as DCS's lfs.dir does.
  function lfs.dir(path)
    if os.execute('test -d ' .. quote(path)) ~= 0 then
      error('cannot open ' .. path .. ': No such file or directory', 2)
    end
    local names = {}
    local p = assert(io.popen('ls -a ' .. quote(path)))
    for name in p:lines() do
      names[#names + 1] = name
    end
    p:close()
    local i = 0
    return function()
      i = i + 1
      return names[i]
    end
  end

  -- The three fields DCS's own code depends on. Every file reports the same
  -- modification time, dcs.MODIFIED, so a test can compare against it.
  function lfs.attributes(path, field)
    local f = io.open(path, 'rb')
    if not f then
      return nil
    end
    local n = f:seek('end')
    f:close()
    local attributes = { mode = 'file', size = n, modification = dcs.MODIFIED }
    if field then
      return attributes[field]
    end
    return attributes
  end

  function lfs.mkdir(path)
    if os.execute('mkdir ' .. quote(path) .. ' 2>/dev/null') == 0 then
      return true
    end
    return nil, 'File exists'
  end

  return lfs
end

-- DCS's log global, recording each line instead of writing dcs.log.
function dcs.log()
  local lines = {}
  return {
    ERROR = 'ERROR',
    WARNING = 'WARNING',
    INFO = 'INFO',
    lines = lines,
    write = function(subsystem, level, message)
      lines[#lines + 1] = { subsystem = subsystem, level = level, message = message }
    end,
  }
end

-- DCS's hook namespace, holding the functions the hook driver calls.
-- setUserCallbacks keeps each table it is handed in registered, so a test
-- can call what DCS would. getModelTime answers time, which a test moves.
function dcs.DCS()
  local registered = {}
  local namespace = {
    registered = registered,
    time = 0,
    setUserCallbacks = function(callbacks)
      registered[#registered + 1] = callbacks
    end,
  }
  namespace.getModelTime = function()
    return namespace.time
  end
  return namespace
end

-- DCS's net namespace, holding dostring_in over two stub states, 'server'
-- and 'mission'. Each state is its own global table, reading through to the
-- real one for the base library, so a global a chunk sets stays in its
-- state. calls holds each { state, chunk, answer } in order. refusal, when
-- a test sets it, is the answer to every call, as a policy refusal is. A
-- name with no state answers nil, as DCS answers an unknown name.
--
-- Each state carries its own dofile. Stock dofile compiles the file against
-- the real global table whatever the caller's, so a file it ran would set
-- its globals outside the state.
function dcs.net()
  local states = {}
  local net = { calls = {}, states = states }
  for _, name in ipairs({ 'server', 'mission' }) do
    local state = setmetatable({}, { __index = _G })
    state.dofile = function(path)
      local chunk = assert(loadfile(path))
      setfenv(chunk, state)
      return chunk()
    end
    states[name] = state
  end
  net.dostring_in = function(target, chunk)
    local answer
    if net.refusal ~= nil then
      answer = net.refusal
    elseif states[target] then
      -- DCS answers a chunk that does not compile or raises with the
      -- message, as a string.
      local fn, err = loadstring(chunk)
      if fn then
        setfenv(fn, states[target])
        local ok, result = pcall(fn)
        answer = ok and result or tostring(result)
      else
        answer = err
      end
    end
    net.calls[#net.calls + 1] = { state = target, chunk = chunk, answer = answer }
    return answer
  end
  return net
end

-- The repository root with a trailing slash, from the path the test script
-- was run by.
function dcs.root()
  local root = assert(arg[0]:match('^(.*)tests/lua/[^/]+$'),
    'run this from the repository, or through tools/luatest.sh')
  return root == '' and './' or root
end

return dcs
