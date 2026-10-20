-- The parts of DCS's hook state the hook driver reaches, for a stock Lua 5.1
-- with no DCS present. Lives outside tests/lua so tools/luatest.sh does not
-- run it as a test.
--
-- lfs is DCS's own binding and stock Lua has none, so this one is built over
-- io, os and the shell. It reaches real files in a scratch directory, which
-- is what the rotation tests need. Linux and macOS only, as luatest.sh is.

local dcs = {}

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

  function lfs.dir(path)
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

  -- Only the size is asked for by name, which is one of the three fields
  -- DCS's own code depends on.
  function lfs.attributes(path, field)
    local f = io.open(path, 'rb')
    if not f then
      return nil
    end
    local n = f:seek('end')
    f:close()
    if field == 'size' then
      return n
    end
    return { mode = 'file', size = n }
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

-- The repository root with a trailing slash, from the path the test script
-- was run by.
function dcs.root()
  local root = assert(arg[0]:match('^(.*)tests/lua/[^/]+$'),
    'run this from the repository, or through tools/luatest.sh')
  return root == '' and './' or root
end

return dcs
