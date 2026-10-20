-- The thin loader. DCS runs each Scripts/Hooks/*.lua once at start, so a
-- change here takes a restart. It holds as little as it can and runs the
-- hook driver, which is the file that changes. ADR 0048.

-- false keeps the bridge installed and inactive. A change takes a DCS
-- restart, and a release overwrites this file.
local ENABLED = true

local LOADER = lfs.writedir() .. 'Scripts/Hooks/DCSBridge.lua'
local DRIVER = lfs.writedir() .. 'Mods/services/DCSBridge/lua/HookDriver.lua'

if not ENABLED then
  log.write('DCSBridge', log.INFO,
    'disabled by ENABLED = false in ' .. LOADER .. ', nothing loaded')
  return
end

-- No filename carries a version, since two copies would both load. A build
-- is named by its files' sizes and modification times instead, read the two
-- ways DCS's own code reads them.
local function identify(path)
  local size = lfs.attributes(path, 'size')
  local attributes = lfs.attributes(path)
  if not size or not attributes then
    return path .. ' missing'
  end
  return path .. ' ' .. size .. ' bytes, modified '
    .. os.date('!%Y-%m-%d %H:%M:%S', attributes.modification)
end

-- Written before anything else runs. No banner means this file never parsed.
local banner = 'loading, ' .. identify(LOADER) .. ', ' .. identify(DRIVER)
log.write('DCSBridge', log.INFO, banner)

local loaded, driver = pcall(dofile, DRIVER)
if not loaded then
  log.write('DCSBridge', log.ERROR, 'the hook driver did not load: ' .. tostring(driver))
  return
end

local started, err = pcall(driver.start, { banner = banner })
if not started then
  log.write('DCSBridge', log.ERROR, 'the hook driver did not start: ' .. tostring(err))
end
