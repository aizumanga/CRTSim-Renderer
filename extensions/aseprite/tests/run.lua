-- Tests the extension against a stand-in for Aseprite's scripting API. Given the crtsim command
-- line, it also renders through it, as Aseprite would run it, from a folder whose name needs
-- quoting, and checks the extension's looks are the command line's:
--
--   lua5.4 extensions/aseprite/tests/run.lua [path/to/crtsim]
--
-- POSIX systems only: the stand-in makes folders with the shell.

local here = arg[0]:match("^(.*)/tests/[^/]*$") or "."
local crtsim = arg[1]
local failures = 0

local function check(name, condition, detail)
  if condition then
    print("ok      " .. name)
  else
    failures = failures + 1
    print("FAILED  " .. name .. (detail and (": " .. tostring(detail)) or ""))
  end
end

local function exists(path)
  local file = io.open(path, "rb")
  if file then
    file:close()
  end
  return file ~= nil
end

local function contents(path)
  local file = assert(io.open(path, "rb"))
  local bytes = file:read("a")
  file:close()
  return bytes
end

local function shell_quote(text)
  return "'" .. (text:gsub("'", "'\\''")) .. "'"
end

-- The stand-in for Aseprite.
local work = os.tmpname()
os.remove(work)
work = work .. " Aseprite's temp"
assert(os.execute("mkdir -p " .. shell_quote(work)))
local fixture = work .. "/fixture.png"
local opened, alerts, exported, commands = {}, {}, {}, {}

app = {
  fs = {
    pathSeparator = "/",
    tempPath = work,
    joinPath = function(...)
      return table.concat({ ... }, "/")
    end,
    makeAllDirectories = function(path)
      return os.execute("mkdir -p " .. shell_quote(path))
    end,
    removeDirectory = function(path)
      return os.remove(path)
    end,
    filePath = function(path)
      return path:match("^(.*)/[^/]*$") or ""
    end,
    fileTitle = function(path)
      return (path:match("([^/]*)$"):gsub("%.[^.]*$", ""))
    end,
  },
  command = {
    -- Writes the fixture as a sheet of the frames asked for, each a different half of it.
    ExportSpriteSheet = function(params)
      exported[#exported + 1] = params
      local sprite = app.activeSprite
      local first, last = 1, #sprite.frames
      for _, tag in ipairs(sprite.tags) do
        if tag.name == params.tag then
          first, last = tag.fromFrame.frameNumber, tag.toFrame.frameNumber
        end
      end
      local copy = assert(io.open(params.textureFilename, "wb"))
      copy:write(contents(fixture))
      copy:close()
      local frames = {}
      for n = first, last do
        frames[#frames + 1] = string.format(
          '{ "filename": "sprite %d.aseprite", "frame": { "x": %d, "y": 0, "w": 64, "h": 48 }, '
            .. '"rotated": false, "trimmed": false, '
            .. '"spriteSourceSize": { "x": 0, "y": 0, "w": 64, "h": 48 }, '
            .. '"sourceSize": { "w": 64, "h": 48 }, "duration": %d }',
          n - 1, n % 2 * 64, math.floor(sprite.frames[n].duration * 1000 + 0.5))
      end
      local json = assert(io.open(params.dataFilename, "w"))
      json:write('{ "frames": [' .. table.concat(frames, ",") .. '], "meta": { '
        .. '"app": "https://www.aseprite.org/", "image": "sheet.png", "format": "RGBA8888", '
        .. '"size": { "w": 128, "h": 48 }, "scale": "1" } }')
      json:close()
    end,
  },
  open = function(path)
    opened[#opened + 1] = { path = path, bytes = exists(path) and contents(path) or nil }
    return {}
  end,
  alert = function(message)
    alerts[#alerts + 1] = message
  end,
}
ColorMode = { RGB = 0 }
SpriteSheetType = { ROWS = 3 }
SpriteSheetDataFormat = { JSON_ARRAY = 2 }
function Image(width, height, mode)
  return {
    drawSprite = function(self, sprite, frame)
      self.frame = frame
    end,
    saveAs = function(self, path)
      local copy = assert(io.open(path, "wb"))
      copy:write(contents(fixture))
      copy:close()
    end,
  }
end

-- A dialog that answers with its widgets' starting values, changed by `answers`.
local answers = {}
function Dialog(options)
  local widgets, data = {}, {}
  local dialog = {}
  local function widget(kind)
    return function(self, spec)
      widgets[spec.id] = { kind = kind, spec = spec }
      local start = spec.option or spec.filename or spec.selected
      if start == nil and kind == "check" then
        start = false
      end
      if kind == "button" then
        start = false
      end
      data[spec.id] = start
    end
  end
  dialog.combobox = widget("combobox")
  dialog.file = widget("file")
  dialog.check = widget("check")
  dialog.button = widget("button")
  dialog.modify = function(self, spec)
    widgets[spec.id].visible = spec.visible
  end
  dialog.show = function(self)
    for id, value in pairs(answers) do
      data[id] = value
    end
    dialog.widgets = widgets
  end
  dialog.data = data
  return dialog
end

local crt = dofile(here .. "/crtsim.lua")

-- How each piece is built.
check("a POSIX argument is quoted whole", crt.quote("it's", false) == [['it'\''s']])
check("a Windows argument is quoted", crt.quote([[C:\My art]], true) == [["C:\My art"]])
check("a Windows command line is quoted again for cmd",
  crt.command_line([[C:\crt\crtsim.exe]], { "render" }, [[C:\t\log]], true)
    == [[""C:\crt\crtsim.exe" "render" > "C:\t\log" 2>&1"]])
check("frames a second follow the shortest frame", crt.fps({ 0.1, 0.1, 0.2 }, 60) == 10)
check("frames a second stop at the format's most", crt.fps({ 0.016 }, 30) == 30)
check("a slow animation is drawn at one a second at least", crt.fps({ 3 }, 60) == 1)
local still = crt.arguments({ program = "crtsim", input = "in.png", output = "out.png",
  look = "NTSC 240p", look_file = "", size = "1080p", moving = false })
check("a still renders at its size with its look",
  table.concat(still, " ") == "render -i in.png -o out.png --preset NTSC 240p --size 1080p")
local moving = crt.arguments({ program = "/opt/CRTSim-Renderer.AppImage", input = "a.json",
  output = "a.webp", look = "NTSC 240p", look_file = "look.json", side = "640", fps = 12,
  moving = true })
check("an animation renders whole, losslessly as WebP, through the AppImage's command line",
  table.concat(moving, " ")
    == "--cli render -i a.json -o a.webp --config look.json --max-side 640 --fps 12 --seconds 0 --lossless")

-- The menu command.
local registered
init({ preferences = {}, newCommand = function(self, command)
  registered = command
end })
check("the command is in File → Export, by Export Sprite Sheet", registered and registered.group == "file_export_1")

if not crtsim then
  print("Rendering: skipped, no crtsim given")
  os.exit(failures == 0)
end

-- The looks, as the command line lists them.
local listed = {}
local presets = assert(io.popen(shell_quote(crtsim) .. " presets"))
for line in presets:lines() do
  local name = line:match("^  ([^:]+): ")
  if name then
    listed[#listed + 1] = name
  end
end
presets:close()
check("the looks are the command line's", table.concat(listed, "|") == table.concat(crt.LOOKS, "|"),
  table.concat(listed, "|"))
local default_listed = false
for _, name in ipairs(listed) do
  default_listed = default_listed or name == crt.DEFAULT_LOOK
end
check("the starting look is one of them", default_listed)

assert(os.execute(shell_quote(crtsim) .. " render -o " .. shell_quote(fixture)
  .. " --size 128x48 > /dev/null 2>&1"), "crtsim draws the fixture")

local sprite = {
  width = 64,
  height = 48,
  filename = work .. "/walk.aseprite",
  frames = { { duration = 0.1 }, { duration = 0.1 }, { duration = 0.2 } },
  tags = { { name = "run", fromFrame = { frameNumber = 2 }, toFrame = { frameNumber = 3 } } },
}
app.activeSprite = sprite
app.activeFrame = { frameNumber = 2 }

local function job(what, extra)
  local data = { what = what, look = "Pixel art 240p", look_file = "", size = "320x240",
    side = "160", open = true, save = "", program = crtsim, frame = 1 }
  for key, value in pairs(extra or {}) do
    data[key] = value
  end
  return data
end

local function leftovers()
  local listing = io.popen("ls -A " .. shell_quote(work))
  local names = {}
  for name in listing:lines() do
    if name ~= "fixture.png" then
      names[#names + 1] = name
    end
  end
  listing:close()
  return names
end

local result, failure = crt.render(sprite, job(crt.THIS_FRAME))
local png = opened[#opened]
check("a frame renders to a PNG that opens", result and png and png.bytes
  and png.bytes:sub(2, 4) == "PNG", failure)
check("its temporary files are gone", #leftovers() == 0, table.concat(leftovers(), ", "))

result, failure = crt.render(sprite, job(crt.ALL_FRAMES))
local webp = opened[#opened]
check("the animation renders to an animated WebP that opens", result and webp ~= png
  and webp.bytes and webp.bytes:sub(9, 12) == "WEBP" and webp.bytes:find("ANIM", 1, true), failure)
check("the sheet is exported as crtsim reads it", exported[1].type == SpriteSheetType.ROWS
  and exported[1].dataFormat == SpriteSheetDataFormat.JSON_ARRAY and exported[1].ui == false
  and exported[1].tag == "" and exported[1].trim == false and exported[1].mergeDuplicates == false)

local saved = work .. "/run.gif"
result, failure = crt.render(sprite, job(crt.TAG .. "run", { save = saved }))
check("a tag renders to the GIF asked for, which stays", result == saved and exists(saved)
  and contents(saved):sub(1, 3) == "GIF", failure)
check("only the tag's frames are exported", exported[2].tag == "run")
os.remove(saved)

result, failure = crt.render(sprite, job(crt.THIS_FRAME, { program = work .. "/no crtsim" }))
check("a missing crtsim fails, saying why", result == nil and failure and failure ~= "", failure)
check("and leaves nothing behind", #leftovers() == 0, table.concat(leftovers(), ", "))

-- The dialog, answered as a person would: the whole animation, kept in the preferences.
local plugin = { preferences = {} }
answers = { what = crt.ALL_FRAMES, look = "NTSC 240p", side = "160", program = crtsim, ok = true }
local before = #opened
crt.dialog(plugin)
check("the dialog renders what it was asked to", #opened == before + 1 and #alerts == 0,
  alerts[1] and table.concat(alerts[1].text or { alerts[1] }, " / "))
check("and remembers the look and the program", plugin.preferences.look == "NTSC 240p"
  and plugin.preferences.program == crtsim)

os.remove(fixture)
os.remove(work)
os.exit(failures == 0)
