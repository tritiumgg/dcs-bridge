-- The wire reader the decoders below share. A file that decodes carries its
-- own copy, so it loads with nothing around it but the shim. Lua 5.1 has no
-- bit library, so every read is arithmetic. ADR 0044.

local byte, sub, floor, ldexp = string.byte, string.sub, math.floor, math.ldexp

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

-- The width bytes at pos, which must end no later than limit.
local function fixed(bytes, pos, limit, width)
  if pos + width - 1 > limit then
    error('a value runs past the end of its message', 0)
  end
  return byte(bytes, pos, pos + width - 1)
end

-- An unsigned 32-bit value read as two's complement.
local function signed32(n)
  if n >= 2 ^ 31 then
    return n - 2 ^ 32
  end
  return n
end

-- Each scalar kind: the wire type it arrives in, the value it reads as when
-- the sender left it out, and its read, which returns the value and the
-- position after it. ADR 0043.
local scalar = {}

scalar.uint32 = {
  wire = 0,
  zero = 0,
  read = function(bytes, pos, limit)
    local lo, _, after = varint(bytes, pos, limit)
    return lo, after
  end,
}

scalar.uint64 = {
  wire = 0,
  zero = 0,
  read = function(bytes, pos, limit)
    local lo, hi, after = varint(bytes, pos, limit)
    return hi * 2 ^ 32 + lo, after
  end,
}

scalar.int32 = {
  wire = 0,
  zero = 0,
  read = function(bytes, pos, limit)
    local lo, _, after = varint(bytes, pos, limit)
    return signed32(lo), after
  end,
}

scalar.enum = scalar.int32

scalar.int64 = {
  wire = 0,
  zero = 0,
  read = function(bytes, pos, limit)
    local lo, hi, after = varint(bytes, pos, limit)
    return signed32(hi) * 2 ^ 32 + lo, after
  end,
}

scalar.bool = {
  wire = 0,
  zero = false,
  read = function(bytes, pos, limit)
    local lo, hi, after = varint(bytes, pos, limit)
    return lo ~= 0 or hi ~= 0, after
  end,
}

-- An IEEE 754 double from its eight little-endian bytes.
scalar.double = {
  wire = 1,
  zero = 0,
  read = function(bytes, pos, limit)
    local b1, b2, b3, b4, b5, b6, b7, b8 = fixed(bytes, pos, limit, 8)
    local exponent = b8 % 128 * 16 + floor(b7 / 16)
    local mantissa = b7 % 16 * 2 ^ 48 + b6 * 2 ^ 40 + b5 * 2 ^ 32
      + b4 * 2 ^ 24 + b3 * 2 ^ 16 + b2 * 2 ^ 8 + b1
    local value
    if exponent == 2047 then
      value = mantissa == 0 and math.huge or 0 / 0
    elseif exponent == 0 then
      value = ldexp(mantissa, -1074)
    else
      value = ldexp(mantissa + 2 ^ 52, exponent - 1075)
    end
    if b8 >= 128 then
      value = -value
    end
    return value, pos + 8
  end,
}

scalar.sint32 = {
  wire = 0,
  zero = 0,
  read = function(bytes, pos, limit)
    local lo, _, after = varint(bytes, pos, limit)
    if lo % 2 == 1 then
      return -(lo + 1) / 2, after
    end
    return lo / 2, after
  end,
}

-- Halved through the high bits, so the value is exact below 2^53.
scalar.sint64 = {
  wire = 0,
  zero = 0,
  read = function(bytes, pos, limit)
    local lo, hi, after = varint(bytes, pos, limit)
    local half = hi * 2 ^ 31 + floor(lo / 2)
    if lo % 2 == 1 then
      return -half - 1, after
    end
    return half, after
  end,
}

local function le32(b1, b2, b3, b4)
  return b1 + b2 * 2 ^ 8 + b3 * 2 ^ 16 + b4 * 2 ^ 24
end

scalar.fixed32 = {
  wire = 5,
  zero = 0,
  read = function(bytes, pos, limit)
    return le32(fixed(bytes, pos, limit, 4)), pos + 4
  end,
}

scalar.sfixed32 = {
  wire = 5,
  zero = 0,
  read = function(bytes, pos, limit)
    return signed32(le32(fixed(bytes, pos, limit, 4))), pos + 4
  end,
}

scalar.fixed64 = {
  wire = 1,
  zero = 0,
  read = function(bytes, pos, limit)
    local b1, b2, b3, b4, b5, b6, b7, b8 = fixed(bytes, pos, limit, 8)
    return le32(b5, b6, b7, b8) * 2 ^ 32 + le32(b1, b2, b3, b4), pos + 8
  end,
}

scalar.sfixed64 = {
  wire = 1,
  zero = 0,
  read = function(bytes, pos, limit)
    local b1, b2, b3, b4, b5, b6, b7, b8 = fixed(bytes, pos, limit, 8)
    return signed32(le32(b5, b6, b7, b8)) * 2 ^ 32 + le32(b1, b2, b3, b4), pos + 8
  end,
}

-- An IEEE 754 single from its four little-endian bytes.
scalar.float = {
  wire = 5,
  zero = 0,
  read = function(bytes, pos, limit)
    local b1, b2, b3, b4 = fixed(bytes, pos, limit, 4)
    local exponent = b4 % 128 * 2 + floor(b3 / 128)
    local mantissa = b3 % 128 * 2 ^ 16 + b2 * 2 ^ 8 + b1
    local value
    if exponent == 255 then
      value = mantissa == 0 and math.huge or 0 / 0
    elseif exponent == 0 then
      value = ldexp(mantissa, -149)
    else
      value = ldexp(mantissa + 2 ^ 23, exponent - 150)
    end
    if b4 >= 128 then
      value = -value
    end
    return value, pos + 4
  end,
}

scalar.string = {
  wire = 2,
  zero = '',
  read = function(bytes, pos, limit)
    local start, stop = length(bytes, pos, limit)
    return sub(bytes, start, stop), stop + 1
  end,
}

-- Each message's shape, by full name: a table from field number to field.
local shape = {}

-- A value into its field: appended to a repeated one, which holds at most
-- MAX_ELEMENTS, and replacing a single one.
local function store(out, field, value)
  if field.repeated then
    local list = out[field.name]
    if #list >= MAX_ELEMENTS then
      error('a repeated field holds more than ' .. MAX_ELEMENTS .. ' elements', 0)
    end
    list[#list + 1] = value
  else
    out[field.name] = value
  end
end

-- The fields of one message between pos and limit, depth messages below the
-- command, into out when the message was seen before and merges. A field in
-- another wire type than its kind's is skipped like an unknown one, a scalar
-- seen twice keeps the last value, a repeated scalar reads packed or not,
-- and a map reads as a table from each entry's key to its value.
local function message(fields, bytes, pos, limit, depth, out)
  if depth > MAX_DEPTH then
    error('a body nests more than ' .. MAX_DEPTH .. ' messages deep', 0)
  end
  if out == nil then
    out = {}
    for _, field in pairs(fields) do
      if field.repeated or field.kind == 'map' then
        out[field.name] = {}
      elseif not field.optional and field.kind ~= 'message' then
        out[field.name] = scalar[field.kind].zero
      end
    end
  end
  -- Map entries read, by field number. A map's table counts keys only.
  local entries = {}
  while pos <= limit do
    local key, high
    key, high, pos = varint(bytes, pos, limit)
    local number, wire = floor(key / 8), key % 8
    if high ~= 0 or number == 0 then
      error('a field number is out of range', 0)
    end
    local field = fields[number]
    local kind = field and scalar[field.kind]
    local value
    if kind and wire == kind.wire then
      value, pos = kind.read(bytes, pos, limit)
      store(out, field, value)
    elseif kind and field.repeated and wire == 2 then
      local start, stop = length(bytes, pos, limit)
      pos = start
      while pos <= stop do
        value, pos = kind.read(bytes, pos, stop)
        store(out, field, value)
      end
    elseif field and field.kind == 'message' and wire == 2 then
      local start, stop = length(bytes, pos, limit)
      local into = not field.repeated and out[field.name] or nil
      value = message(shape[field.type], bytes, start, stop, depth + 1, into)
      store(out, field, value)
      pos = stop + 1
    elseif field and field.kind == 'map' and wire == 2 then
      entries[number] = (entries[number] or 0) + 1
      if entries[number] > MAX_ELEMENTS then
        error('a map holds more than ' .. MAX_ELEMENTS .. ' entries', 0)
      end
      local start, stop = length(bytes, pos, limit)
      local entry = message(shape[field.type], bytes, start, stop, depth + 1)
      out[field.name][entry.key] = entry.value
      pos = stop + 1
    else
      pos = skip(bytes, pos, limit, wire)
    end
  end
  return out
end
