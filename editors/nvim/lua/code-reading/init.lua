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
  --- Command that starts the server.
  cmd = { "crt", "lsp" },
  filetypes = { "rust", "go", "python", "javascript", "javascriptreact" },
  --- Read uncached functions in view without being asked.
  auto_read = true,
  --- How many functions may be read at once.
  max_parallel = 2,
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
  if not client then
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

function M.toggle()
  state.enabled = not state.enabled
  for _, bufnr in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_loaded(bufnr) then
      render.draw(bufnr)
    end
  end
end

M.scenario = scenario.open

function M.setup(opts)
  M.config = vim.tbl_deep_extend("force", vim.deepcopy(defaults), opts or {})
  render.define_highlights()

  vim.lsp.config("crt", {
    cmd = M.config.cmd,
    filetypes = M.config.filetypes,
    root_markers = { ".git" },
    workspace_required = false,
    init_options = { autoRead = M.config.auto_read, maxParallel = M.config.max_parallel },
    handlers = { ["codeReading/fileReadings"] = on_file_readings },
  })
  vim.lsp.enable("crt")

  local group = vim.api.nvim_create_augroup("code-reading", { clear = true })
  vim.api.nvim_create_autocmd("LspAttach", {
    group = group,
    callback = function(args)
      local client = vim.lsp.get_client_by_id(args.data.client_id)
      if client and client.name == "crt" then
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
