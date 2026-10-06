--- code-reading.nvim: per-line explanations from the `crt` language server.
---
--- The server does the work; this plugin starts it through Neovim's LSP
--- client, renders what it pushes (`codeReading/fileReadings`), tells it what
--- is visible (`codeReading/visibleRange`), and offers a scenario view.

local render = require("code-reading.render")
local scenario = require("code-reading.scenario")
local state = require("code-reading.state")

local M = {}

local defaults = {
  --- Command that starts the server. Default: the binary installed by
  --- :CrInstall if present, else `crt` on PATH.
  cmd = nil,
  filetypes = {
    "rust", "go", "python", "javascript", "javascriptreact", "typescript", "typescriptreact",
    "java", "c", "cpp", "cs", "ruby", "php", "sh", "bash",
  },
  --- Show explanations (line ends, diagnostics) and write them for the code
  --- in view. When off, they are hidden and the model is called only when
  --- asked (:CrRead, :CrScenario); hover still works. :CrToggle switches it.
  enabled = true,
  --- Read uncached functions in view without being asked.
  auto_read = true,
  --- How many functions may be read at once.
  max_parallel = 2,
  --- When an edited function is explained again without being asked:
  --- "save" (when the file is saved) or "idle" (2 s after typing stops).
  --- Until then it shows its earlier explanation, marked (old).
  read_on = "save",
  --- Milliseconds to wait after scrolling before reporting the view.
  debounce_ms = 150,
}

M.config = vim.deepcopy(defaults)

local function client_for(bufnr)
  return vim.lsp.get_clients({ bufnr = bufnr, name = "crt" })[1]
end

local timers = {}

--- Tells the server which lines of `bufnr` are visible in any window.
function M.report_view(bufnr)
  local client = client_for(bufnr)
  -- Off means no model calls unless asked: the server reads only what it
  -- is told is in view.
  if not client or not state.enabled then
    return
  end
  local first, last
  for _, win in ipairs(vim.fn.win_findbuf(bufnr)) do
    local top = vim.fn.line("w0", win)
    local bot = vim.fn.line("w$", win)
    first = first and math.min(first, top) or top
    last = last and math.max(last, bot) or bot
  end
  if not first then
    return
  end
  ---@diagnostic disable-next-line: param-type-mismatch
  client:notify("codeReading/visibleRange", {
    uri = vim.uri_from_bufnr(bufnr),
    startLine = first - 1,
    endLine = last - 1,
  })
end

local function schedule_view(bufnr)
  local t = timers[bufnr]
  if t then
    t:stop()
  else
    t = assert(vim.uv.new_timer())
    timers[bufnr] = t
  end
  t:start(M.config.debounce_ms, 0, vim.schedule_wrap(function()
    if vim.api.nvim_buf_is_valid(bufnr) then
      M.report_view(bufnr)
    end
  end))
end

local function on_file_readings(_, params, ctx)
  local bufnr = vim.uri_to_bufnr(params.uri)
  if not vim.api.nvim_buf_is_loaded(bufnr) then
    return
  end
  -- Ignore results computed from an older version of the text.
  local current = vim.lsp.util.buf_versions[bufnr]
  if current and params.version < current then
    return
  end
  state.set(bufnr, params, ctx.client_id)
  render.draw(bufnr)
end

--- Reads the function under the cursor now (`refresh` asks the model again).
function M.read(refresh)
  local bufnr = vim.api.nvim_get_current_buf()
  local client = client_for(bufnr)
  if not client then
    vim.notify("crt: no crt server attached to this buffer", vim.log.levels.WARN)
    return
  end
  local line = vim.api.nvim_win_get_cursor(0)[1] - 1
  vim.notify("crt: reading…")
  -- A custom method; Neovim's annotations only list the standard ones.
  ---@diagnostic disable-next-line: param-type-mismatch
  client:request("codeReading/read", {
    uri = vim.uri_from_bufnr(bufnr),
    line = line,
    refresh = refresh or false,
  }, function(err, result)
    if err then
      vim.notify("crt: " .. err.message, vim.log.levels.ERROR)
      return
    end
    local src = result.from_cache and "cache" or result.reading.model
    vim.notify(("crt: read %s (%s)"):format(result["function"].name, src))
  end, bufnr)
end

--- Opens crt's configuration file (the endpoint, model, key and output
--- language shared by every editor), creating a commented example first
--- when there is none. Changes apply on the next read; no restart needed.
function M.open_config()
  local bufnr = vim.api.nvim_get_current_buf()
  local client = client_for(bufnr)
  if not client then
    vim.notify("crt: no crt server attached to this buffer", vim.log.levels.WARN)
    return
  end
  -- A custom method; Neovim's annotations only list the standard ones.
  ---@diagnostic disable-next-line: param-type-mismatch
  client:request("codeReading/configPath", vim.NIL, function(err, result)
    if err then
      vim.notify("crt: " .. err.message, vim.log.levels.ERROR)
      return
    end
    vim.cmd.edit(vim.fn.fnameescape(result.path))
    if result.created then
      vim.notify("crt: wrote an example configuration; edit it to choose a model")
    end
  end, bufnr)
end

--- Shows or hides the crt server's diagnostics (concurrency scenarios) to
--- match `state.enabled`. Hover is left alone: it is asked for.
local function apply_diagnostics()
  for _, client in ipairs(vim.lsp.get_clients({ name = "crt" })) do
    for _, pull in ipairs({ false, true }) do
      vim.diagnostic.enable(state.enabled, { ns_id = vim.lsp.diagnostic.get_namespace(client.id, pull) })
    end
  end
end

function M.toggle()
  state.enabled = not state.enabled
  apply_diagnostics()
  for _, bufnr in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_loaded(bufnr) then
      render.draw(bufnr)
      if state.enabled then
        M.report_view(bufnr)
      end
    end
  end
  vim.cmd.redrawstatus()
end

--- A short text for a statusline: whether explanations are on, and whether
--- some are being written for the current buffer. Empty when the buffer has
--- no crt server. For lualine: `{ require("code-reading").status, on_click =
--- function() require("code-reading").toggle() end }`.
function M.status()
  local bufnr = vim.api.nvim_get_current_buf()
  if not client_for(bufnr) then
    return ""
  end
  if not state.enabled then
    return "crt off"
  end
  local s = state.get(bufnr)
  if s and next(s.pending) then
    return "crt …"
  end
  return "crt on"
end

M.scenario = scenario.open

local function default_cmd()
  local installed = require("code-reading.install").bin_path()
  if vim.uv.fs_stat(installed) then
    return { installed, "lsp" }
  end
  return { "crt", "lsp" }
end

function M.setup(opts)
  M.config = vim.tbl_deep_extend("force", vim.deepcopy(defaults), opts or {})
  M.config.cmd = M.config.cmd or default_cmd()
  state.enabled = M.config.enabled
  render.define_highlights()

  vim.lsp.config("crt", {
    cmd = M.config.cmd,
    filetypes = M.config.filetypes,
    root_markers = { ".git" },
    workspace_required = false,
    init_options = {
      autoRead = M.config.auto_read,
      maxParallel = M.config.max_parallel,
      readOn = M.config.read_on,
    },
    handlers = { ["codeReading/fileReadings"] = on_file_readings },
  })
  vim.lsp.enable("crt")
  -- enable() attaches on FileType; buffers opened before setup() ran (a
  -- lazy-loaded plugin, a file given on the command line) need a nudge.
  local wanted = {}
  for _, ft in ipairs(M.config.filetypes) do
    wanted[ft] = true
  end
  for _, bufnr in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_loaded(bufnr) and wanted[vim.bo[bufnr].filetype] then
      pcall(vim.api.nvim_exec_autocmds, "FileType", { group = "nvim.lsp.enable", buffer = bufnr })
    end
  end

  local group = vim.api.nvim_create_augroup("code-reading", { clear = true })
  vim.api.nvim_create_autocmd("LspAttach", {
    group = group,
    callback = function(args)
      local client = vim.lsp.get_client_by_id(args.data.client_id)
      if client and client.name == "crt" then
        apply_diagnostics()
        schedule_view(args.buf)
      end
    end,
  })
  vim.api.nvim_create_autocmd({ "WinScrolled", "BufWinEnter" }, {
    group = group,
    callback = function(args)
      if client_for(args.buf) then
        schedule_view(args.buf)
      end
    end,
  })
  vim.api.nvim_create_autocmd("BufWipeout", {
    group = group,
    callback = function(args)
      state.clear(args.buf)
      if timers[args.buf] then
        timers[args.buf]:close()
        timers[args.buf] = nil
      end
    end,
  })
end

return M
