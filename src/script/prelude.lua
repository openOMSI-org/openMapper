-- openMapper's test run of a map's scripts: the parts of openOMSI's `omsi` table written in
-- Lua here (events, timers, the helpers). The game's own are in its API registry; these do
-- the same for a run in the editor, with no game around them.
local omsi, traceback, report, note, now = ...

local type, error, select, ipairs, pairs, tostring = type, error, select, ipairs, pairs, tostring
local floor, abs = math.floor, math.abs
local fmt_s = string.format

local function check(fname, i, pname, v, want, optional)
  if v == nil and optional then return end
  local t = type(v)
  if want == "integer" then
    if math.type(v) == "integer" or (t == "number" and v == floor(v)) then return end
  elseif t == want then
    return
  end
  error(fmt_s("omsi.%s: argument %d (%s) must be a %s, not %s", fname, i, pname, want, t), 3)
end

-- events -------------------------------------------------------------------------------------

local handlers = {}

function omsi.on(event, fn)
  check("on", 1, "event", event, "string")
  check("on", 2, "fn", fn, "function")
  local list = handlers[event]
  if not list then list = {}; handlers[event] = list end
  list[#list + 1] = fn
  return fn
end

function omsi.off(event, fn)
  check("off", 1, "event", event, "string")
  local list = handlers[event]
  if not list then return end
  for i = #list, 1, -1 do
    if list[i] == fn then table.remove(list, i) end
  end
end

-- (an error of a Rust function is an object: its text first, then where it happened)
local function handler(e) return traceback(tostring(e), 2) end

local function call(where, fn, ...)
  local ok, err = xpcall(fn, handler, ...)
  if not ok then report(where, err) end
end

local function copy(list)
  return table.move(list, 1, #list, 1, {})
end

-- (an event of the game: every handler on its own, an error of one is logged and the others
-- still hear it; then a global `on_<event>`)
local function fire(event, ...)
  local list = handlers[event]
  if list then
    for _, fn in ipairs(copy(list)) do call("event " .. event, fn, ...) end
  end
  local g = rawget(_G, "on_" .. event)
  if type(g) == "function" then call("on_" .. event, g, ...) end
end

function omsi.emit(event, ...)
  check("emit", 1, "event", event, "string")
  local list = handlers[event]
  if not list then return end
  for _, fn in ipairs(copy(list)) do fn(...) end
end

-- timers and watches ---------------------------------------------------------------------------

local timers, next_id = {}, 0

local function timer(name, seconds, fn, every)
  check(name, 1, "seconds", seconds, "number")
  check(name, 2, "fn", fn, "function")
  next_id = next_id + 1
  timers[next_id] = { at = now() + seconds, every = every and math.max(seconds, 0.001) or nil, fn = fn }
  return next_id
end

function omsi.every(seconds, fn) return timer("every", seconds, fn, true) end
function omsi.after(seconds, fn) return timer("after", seconds, fn, false) end

function omsi.cancel(id)
  check("cancel", 1, "id", id, "integer")
  local had = timers[id] ~= nil
  timers[id] = nil
  return had
end

function omsi.watch(kind, name, fn)
  if fn == nil then kind, name, fn = "var", kind, name end
  check("watch", 2, "name", name, "string")
  check("watch", 3, "fn", fn, "function")
  note("omsi.watch", "the test run has no bus or game values: " .. tostring(kind) .. " " .. name .. " never changes here")
  next_id = next_id + 1
  return next_id
end

local function tick()
  local t = now()
  local due = {}
  for id, tm in pairs(timers) do
    if tm.at <= t then due[#due + 1] = id end
  end
  table.sort(due)
  for _, id in ipairs(due) do
    local tm = timers[id]
    if tm then
      if tm.every then
        tm.at = tm.at + tm.every
        if tm.at <= t then tm.at = t + tm.every end
      else
        timers[id] = nil
      end
      call("timer", tm.fn)
    end
  end
end

-- what is listening, for the editor's Events panel
local function counts()
  local out = {}
  for event, list in pairs(handlers) do out[event] = #list end
  for k, v in pairs(_G) do
    if type(k) == "string" and k:sub(1, 3) == "on_" and type(v) == "function" then
      out[k:sub(4)] = (out[k:sub(4)] or 0) + 1
    end
  end
  local n = 0
  for _ in pairs(timers) do n = n + 1 end
  return out, n
end

-- saved data -------------------------------------------------------------------------------------

omsi.data = {}
function omsi.save() end

-- helpers (as the game's `fmt`, `vec` and `util`) -------------------------------------------------

local fmt = omsi.fmt

function fmt.clock(seconds, with_seconds)
  check("fmt.clock", 1, "seconds", seconds, "number")
  local s = floor(seconds) % 86400
  if with_seconds then return fmt_s("%02d:%02d:%02d", s // 3600, s // 60 % 60, s % 60) end
  return fmt_s("%02d:%02d", s // 3600, s // 60 % 60)
end

function fmt.delay(seconds)
  check("fmt.delay", 1, "seconds", seconds, "number")
  local s = floor(abs(seconds) + 0.5)
  local sign = seconds >= 0.5 and "+" or (seconds <= -0.5 and "-" or "")
  return fmt_s("%s%d:%02d", sign, s // 60, s % 60)
end

function fmt.distance(m)
  check("fmt.distance", 1, "metres", m, "number")
  if abs(m) < 1000 then return fmt_s("%.0f m", m) end
  return fmt_s("%.1f km", m / 1000)
end

function fmt.duration(seconds)
  check("fmt.duration", 1, "seconds", seconds, "number")
  local s = floor(abs(seconds) + 0.5)
  local r
  if s < 60 then r = s .. " s"
  elseif s < 3600 then r = fmt_s("%d min %02d s", s // 60, s % 60)
  else r = fmt_s("%d h %d min", s // 3600, s // 60 % 60) end
  return (seconds < 0 and "-" or "") .. r
end

function fmt.money(amount, symbol)
  check("fmt.money", 1, "amount", amount, "number")
  if symbol and symbol ~= "" then return fmt_s("%.2f %s", amount, symbol) end
  return fmt_s("%.2f", amount)
end

function fmt.number(x, decimals, separator)
  check("fmt.number", 1, "x", x, "number")
  local s = fmt_s("%." .. floor(decimals or 0) .. "f", x)
  if not separator or separator == "" then return s end
  local sign, int, frac = s:match("^(-?)(%d+)(.*)$")
  int = int:reverse():gsub("(%d%d%d)", "%1" .. separator:reverse()):reverse()
  if int:sub(1, #separator) == separator then int = int:sub(#separator + 1) end
  return sign .. int .. frac
end

function fmt.pad(text, width, right)
  text = tostring(text)
  if #text >= width then return text:sub(1, width) end
  if right then return string.rep(" ", width - #text) .. text end
  return text .. string.rep(" ", width - #text)
end

function fmt.speed(kmh, unit)
  check("fmt.speed", 1, "kmh", kmh, "number")
  if unit == "mph" then return fmt_s("%.0f mph", kmh / 1.609344) end
  if unit == "m/s" then return fmt_s("%.1f m/s", kmh / 3.6) end
  return fmt_s("%.0f km/h", kmh)
end

function fmt.split(text, separator)
  check("fmt.split", 1, "text", text, "string")
  separator = separator or ","
  local out, from = {}, 1
  while true do
    local i, j = string.find(text, separator, from, true)
    if not i then out[#out + 1] = text:sub(from); return out end
    out[#out + 1] = text:sub(from, i - 1)
    from = j + 1
  end
end

function fmt.trim(text)
  check("fmt.trim", 1, "text", text, "string")
  return (text:gsub("^%s+", ""):gsub("%s+$", ""))
end

local vec = omsi.vec

function vec.heading(dx, dy) return (math.deg(math.atan(dx, dy)) + 360) % 360 end
function vec.bearing(x1, y1, x2, y2) return vec.heading(x2 - x1, y2 - y1) end
function vec.angle_diff(a, b) return (b - a + 180) % 360 - 180 end
function vec.clamp(x, lo, hi) return math.max(lo, math.min(hi, x)) end
function vec.distance(x1, y1, x2, y2) return math.sqrt((x2 - x1) ^ 2 + (y2 - y1) ^ 2) end
function vec.distance3(x1, y1, z1, x2, y2, z2) return math.sqrt((x2 - x1) ^ 2 + (y2 - y1) ^ 2 + (z2 - z1) ^ 2) end
function vec.dot(x1, y1, x2, y2) return x1 * x2 + y1 * y2 end
function vec.length(x, y, z) return math.sqrt(x * x + y * y + (z or 0) ^ 2) end
function vec.lerp(a, b, t) return a + (b - a) * t end
function vec.normalize(x, y, z)
  local l = vec.length(x, y, z)
  if l == 0 then return 0, 0, z and 0 or nil end
  return x / l, y / l, z and z / l or nil
end
function vec.rotate(x, y, degrees)
  local r = math.rad(degrees)
  local c, s = math.cos(r), math.sin(r)
  return x * c + y * s, -x * s + y * c
end
function vec.to_local(x, y, ox, oy, heading)
  local dx, dy = x - ox, y - oy
  local r = math.rad(heading)
  local fx, fy = math.sin(r), math.cos(r)
  return dx * fy - dy * fx, dx * fx + dy * fy
end

return fire, tick, counts
