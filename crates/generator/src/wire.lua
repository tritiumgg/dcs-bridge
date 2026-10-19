-- The wire reader the decoders below share. A file that decodes carries its
-- own copy, so it loads with nothing around it but the shim. Lua 5.1 has no
-- bit library, so every read is arithmetic. ADR 0044.

local byte, floor = string.byte, math.floor

-- A varint at pos, read no further than limit: its low and high 32 bits
-- apart, so a 64-bit value loses nothing before its kind reads it, and the
-- position after it.
local function varint(bytes, pos, limit)
  local lo, hi = 0, 0
  for i = 0, 9 do
    if pos + i > limit then
      error('a varint runs past the end of its message', 0)
    end
    local b = byte(bytes, pos + i)
    local v = b % 128
    if i < 4 then
      lo = lo + v * 2 ^ (7 * i)
    elseif i == 4 then
      lo = lo + v % 16 * 2 ^ 28
      hi = floor(v / 16)
    else
      hi = hi + v * 2 ^ (7 * i - 32)
    end
    if b < 128 then
      return lo, hi % 2 ^ 32, pos + i + 1
    end
  end
  error('a varint is longer than ten bytes', 0)
end

-- A length-delimited value's first and last positions. A length may carry
-- more bytes than it needs. ADR 0012.
local function length(bytes, pos, limit)
  local n, high, start = varint(bytes, pos, limit)
  local stop = start + n - 1
  if high ~= 0 or stop > limit then
    error('a length runs past the end of its message', 0)
  end
  return start, stop
end

-- The position after a value of a wire type, read no further than limit.
-- Groups, 3 and 4, and the invalid 6 and 7 cannot be skipped.
local function skip(bytes, pos, limit, wire)
  local after
  if wire == 0 then
    local _
    _, _, after = varint(bytes, pos, limit)
  elseif wire == 1 then
    after = pos + 8
  elseif wire == 2 then
    local _, stop = length(bytes, pos, limit)
    after = stop + 1
  elseif wire == 5 then
    after = pos + 4
  else
    error('a field has wire type ' .. wire .. ', which a decoder refuses', 0)
  end
  if after > limit + 1 then
    error('a value runs past the end of its message', 0)
  end
  return after
end

-- The fields of one message between pos and limit, by shape: a table from
-- field number to field.
local function message(shape, bytes, pos, limit)
  local out = {}
  while pos <= limit do
    local key, high
    key, high, pos = varint(bytes, pos, limit)
    local number, wire = floor(key / 8), key % 8
    if high ~= 0 or number == 0 then
      error('a field number is out of range', 0)
    end
    pos = skip(bytes, pos, limit, wire)
  end
  return out
end
