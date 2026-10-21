-- The sim driver. The hook driver injects a chunk into the 'server' state
-- on every mission load, and that chunk runs this file with dofile. The
-- chunk assigns the load's parameters to DCSBridge_load first. This file
-- takes them and clears the name, so DCSBridge is the one global it leaves.
-- ADR 0065.

-- The grammar of what the hook driver hands this file. The hook driver
-- reads this line from disk by its shape before it injects, and refuses a
-- file whose value is not its own, since this file's source never crosses
-- into the hook state. ADR 0065.
local GRAMMAR_VERSION = '1'

local params = DCSBridge_load
DCSBridge_load = nil
if type(params) ~= 'table' then
  error('DCSBridge_load is not a table: the hook driver loads this file', 0)
end

-- The hook driver's chunk reads load.epoch back from here, which is how it
-- knows this file ran and not only that the chunk did. ADR 0065.
DCSBridge = {
  grammar = GRAMMAR_VERSION,
  load = params,
}
