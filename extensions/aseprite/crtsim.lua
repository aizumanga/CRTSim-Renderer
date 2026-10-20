-- CRTSim Renderer for Aseprite: draws the sprite's frame, or its animation, through the CRT with
-- the crtsim command line, and opens the picture or animation it makes. See docs/ASEPRITE.md.
--
-- A frame goes to crtsim as a PNG; an animation as a sprite sheet with its JSON, every pixel and
-- each frame's duration kept, which crtsim reads as Aseprite writes it.

local M = {}

-- The included looks, in the gallery's order, as `crtsim presets` lists them. tests/run.lua
-- keeps them in step with the command line.
M.LOOKS = {
  "Super Win the Game",
  "Super Win the Game NES palette",
  "General image",
  "Soft television",
  "Clean RGB",
  "Pixel art 240p",
  "NTSC 240p",
  "NTSC 480i",
  "Widescreen television",
  "PAL 288p",
  "PAL 576i",
  "Warm analog",
  "Linear light",
}
M.DEFAULT_LOOK = "Pixel art 240p"

M.THIS_FRAME = "This frame"
M.ALL_FRAMES = "All frames"
M.TAG = "Tag: "

-- A still's size, and an animation's longest side, as crtsim takes them.
M.SIZES = { "1080p", "720p", "1440p", "4k", "reference" }
M.SIDES = { "640", "480", "960", "1280" }

-- The most frames a second each kind of animation crtsim writes takes.
M.MOST_FPS = { gif = 30, webp = 60, png = 60, mp4 = 60, mkv = 60, webm = 60 }
-- What Aseprite opens of what crtsim writes.
M.OPENS = { png = true, gif = true, webp = true }

function M.extension(path)
  return (path:match("%.([^%.\\/]+)$") or ""):lower()
end

function M.windows()
  return app.fs.pathSeparator == "\\"
end

-- `text` as one argument of a command line: in single quotes for a POSIX shell, in double
-- quotes for Windows' cmd, where file names cannot hold one.
function M.quote(text, windows)
  if windows then
    return '"' .. text .. '"'
  end
  return "'" .. (text:gsub("'", "'\\''")) .. "'"
end

-- The command line that runs `program` with `args`, writing what it says to `log`.
function M.command_line(program, args, log, windows)
  local parts = { M.quote(program, windows) }
  for _, arg in ipairs(args) do
    parts[#parts + 1] = M.quote(arg, windows)
  end
  local line = table.concat(parts, " ") .. " > " .. M.quote(log, windows) .. " 2>&1"
  if windows then
    -- cmd drops the first and the last quote of a line that starts with one.
    line = '"' .. line .. '"'
  end
  return line
end

-- How many frames a second an animation whose frames last `durations` seconds is drawn at:
-- enough for its shortest frame, and at most `most`.
function M.fps(durations, most)
  local shortest = math.huge
  for _, duration in ipairs(durations) do
    if duration > 0 and duration < shortest then
      shortest = duration
    end
  end
  if shortest == math.huge then
    return math.min(10, most)
  end
  return math.max(1, math.min(most, math.floor(1 / shortest + 0.5)))
end

-- The arguments crtsim renders with. `job` holds the input and output, the look or look
-- file, and a still's size or an animation's longest side and frame rate.
function M.arguments(job)
  local args = {}
  local function add(...)
    for _, arg in ipairs({ ... }) do
      args[#args + 1] = arg
    end
  end
  -- The AppImage runs the desktop app unless asked for its command line.
  if job.program:lower():match("%.appimage$") then
    add("--cli")
  end
  add("render", "-i", job.input, "-o", job.output)
  if job.look_file and job.look_file ~= "" then
    add("--config", job.look_file)
  else
    add("--preset", job.look)
  end
  if job.moving then
    add("--max-side", job.side, "--fps", tostring(job.fps), "--seconds", "0")
    if M.extension(job.output) == "webp" then
      add("--lossless")
    end
  else
    add("--size", job.size)
  end
  return args
end

-- The frames `what` names, by number, counting from 1.
local function frames_of(sprite, what)
  if what:sub(1, #M.TAG) == M.TAG then
    local name = what:sub(#M.TAG + 1)
    for _, tag in ipairs(sprite.tags) do
      if tag.name == name then
        return tag.fromFrame.frameNumber, tag.toFrame.frameNumber, name
      end
    end
  end
  return 1, #sprite.frames, ""
end

local function read(path)
  local file = io.open(path, "r")
  if not file then
    return ""
  end
  local text = file:read("a")
  file:close()
  return text
end

-- Writes the animation `what` names as a sprite sheet with its JSON into `folder`, and
-- returns the JSON's path and its frames' durations.
local function export_sheet(sprite, what, folder)
  local first, last, tag = frames_of(sprite, what)
  local json = app.fs.joinPath(folder, "sheet.json")
  app.command.ExportSpriteSheet({
    ui = false,
    askOverwrite = false,
    recent = false,
    type = SpriteSheetType.ROWS,
    textureFilename = app.fs.joinPath(folder, "sheet.png"),
    dataFilename = json,
    dataFormat = SpriteSheetDataFormat.JSON_ARRAY,
    tag = tag,
    layer = "",
    borderPadding = 0,
    shapePadding = 0,
    innerPadding = 0,
    trimSprite = false,
    trim = false,
    trimByGrid = false,
    extrude = false,
    ignoreEmpty = false,
    mergeDuplicates = false,
    splitLayers = false,
    splitTags = false,
    splitGrid = false,
    listLayers = false,
    listTags = false,
    listSlices = false,
    fromTilesets = false,
    openGenerated = false,
  })
  local durations = {}
  for n = first, last do
    durations[#durations + 1] = sprite.frames[n].duration
  end
  return json, durations
end

-- Renders the sprite as the dialog's `data` says, and opens or saves what crtsim makes.
-- Returns the output's path, or nil and why it failed.
function M.render(sprite, data)
  local windows = M.windows()
  local folder = app.fs.joinPath(app.fs.tempPath, string.format("crtsim-aseprite-%d-%d", os.time(),
    math.random(1, 999999)))
  app.fs.makeAllDirectories(folder)
  local moving = data.what ~= M.THIS_FRAME
  local job = {
    program = data.program ~= "" and data.program or "crtsim",
    look = data.look,
    look_file = data.look_file,
    size = data.size,
    side = data.side,
    moving = moving,
  }
  local made = {}
  if moving then
    local durations
    job.input, durations = export_sheet(sprite, data.what, folder)
    made = { job.input, app.fs.joinPath(folder, "sheet.png") }
    job.output = data.save ~= "" and data.save or app.fs.joinPath(folder, "crt.webp")
    job.fps = M.fps(durations, M.MOST_FPS[M.extension(job.output)] or 60)
  else
    local image = Image(sprite.width, sprite.height, ColorMode.RGB)
    image:drawSprite(sprite, data.frame)
    job.input = app.fs.joinPath(folder, "frame.png")
    image:saveAs(job.input)
    made = { job.input }
    job.output = data.save ~= "" and data.save or app.fs.joinPath(folder, "crt.png")
  end
  local log = app.fs.joinPath(folder, "crtsim.log")
  made[#made + 1] = log
  local ok = os.execute(M.command_line(job.program, M.arguments(job), log, windows))
  local said = read(log)
  local result, failure = nil, nil
  if ok then
    result = job.output
    if data.open and M.OPENS[M.extension(job.output)] then
      local opened = app.open(job.output)
      -- Saving the CRT picture puts it beside the sprite, not in the temporary folder.
      if opened and data.save == "" and app.fs.filePath(sprite.filename) ~= "" then
        opened.filename = app.fs.joinPath(app.fs.filePath(sprite.filename),
          app.fs.fileTitle(sprite.filename) .. "-crt." .. M.extension(job.output))
      end
    end
  else
    failure = said ~= "" and said or ("Could not run " .. job.program)
  end
  if data.save == "" then
    made[#made + 1] = job.output
  end
  for _, path in ipairs(made) do
    os.remove(path)
  end
  if app.fs.removeDirectory then
    app.fs.removeDirectory(folder)
  end
  return result, failure
end

-- The lines of crtsim's message worth showing in an alert: its last few.
local function last_lines(text, count)
  local lines = {}
  for line in text:gmatch("[^\r\n]+") do
    if not line:match("^%s*%d+%%") then
      lines[#lines + 1] = line
    end
  end
  return { table.unpack(lines, math.max(1, #lines - count + 1)) }
end

function M.dialog(plugin)
  local sprite = app.activeSprite
  if not sprite then
    return app.alert("Open a sprite to render it through the CRT.")
  end
  local prefs = plugin.preferences
  local whats = { M.THIS_FRAME }
  if #sprite.frames > 1 then
    whats[#whats + 1] = M.ALL_FRAMES
    for _, tag in ipairs(sprite.tags) do
      whats[#whats + 1] = M.TAG .. tag.name
    end
  end
  local dialog = Dialog({ title = "CRTSim Renderer" })
  local function show_sizes()
    local moving = dialog.data.what ~= M.THIS_FRAME
    dialog:modify({ id = "size", visible = not moving })
    dialog:modify({ id = "side", visible = moving })
  end
  dialog:combobox({ id = "what", label = "Render", option = whats[1], options = whats,
    onchange = show_sizes })
  dialog:combobox({ id = "look", label = "Look", option = prefs.look or M.DEFAULT_LOOK,
    options = M.LOOKS })
  -- File fields that take typing can be emptied again.
  dialog:file({ id = "look_file", label = "Look file", entry = true, filetypes = { "json" },
    filename = prefs.look_file or "" })
  dialog:combobox({ id = "size", label = "Size", option = prefs.size or M.SIZES[1],
    options = M.SIZES })
  dialog:combobox({ id = "side", label = "Longest side", option = prefs.side or M.SIDES[1],
    options = M.SIDES })
  dialog:check({ id = "open", text = "Open it in Aseprite", selected = prefs.open ~= false })
  dialog:file({ id = "save", label = "Save as", save = true, entry = true,
    filetypes = { "png", "gif", "webp", "mp4", "mkv", "webm" }, filename = "" })
  dialog:file({ id = "program", label = "crtsim program", entry = true,
    filename = prefs.program or "" })
  dialog:button({ id = "ok", text = "Render", focus = true })
  dialog:button({ id = "cancel", text = "Cancel" })
  show_sizes()
  dialog:show()
  local data = dialog.data
  if not data.ok then
    return
  end
  prefs.look, prefs.look_file, prefs.size, prefs.side = data.look, data.look_file, data.size,
    data.side
  prefs.open, prefs.program = data.open, data.program
  if not data.open and data.save == "" then
    return app.alert("Choose where to save it, or open it in Aseprite.")
  end
  data.frame = app.activeFrame.frameNumber
  local result, failure = M.render(sprite, data)
  if not result then
    local text = { "crtsim could not render the sprite:" }
    for _, line in ipairs(last_lines(failure, 8)) do
      text[#text + 1] = line
    end
    text[#text + 1] = "Set crtsim program to the crtsim of CRTSim Renderer's package."
    app.alert({ title = "CRTSim Renderer", text = text })
  end
end

function init(plugin)
  math.randomseed(os.time())
  plugin:newCommand({
    id = "CRTSimRenderer",
    title = "CRTSim Renderer...",
    group = "file_export_1",
    onclick = function()
      M.dialog(plugin)
    end,
    onenabled = function()
      return app.activeSprite ~= nil
    end,
  })
end

function exit(plugin) end

return M
