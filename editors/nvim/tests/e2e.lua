-- Headless end-to-end test. Run by crates/crt-cli/tests/nvim.rs with:
--   CRT_BIN, CRT_CONFIG, CRT_CACHE, CRT_FILE, CRT_PLUGIN set.
-- Exits 0 on success, 1 with a message on failure.

local function fail(msg)
  io.stderr:write("FAIL: " .. msg .. "\n")
  vim.cmd("cquit 1")
end

local function check(cond, msg)
  if not cond then
    fail(msg)
  end
end

vim.opt.runtimepath:prepend(os.getenv("CRT_PLUGIN"))
vim.cmd("runtime plugin/code-reading.lua")

local cr = require("code-reading")
cr.setup({
  cmd = { os.getenv("CRT_BIN"), "lsp", "--config", os.getenv("CRT_CONFIG"), "--cache-dir", os.getenv("CRT_CACHE") },
  debounce_ms = 10,
})

vim.cmd("edit " .. os.getenv("CRT_FILE"))
local buf = vim.api.nvim_get_current_buf()
local state = require("code-reading.state")
local render = require("code-reading.render")

-- Structural facts arrive first, then the auto-read fills both readings.
-- The notes of Inc arrive one at a time; the first is drawn on its own.
local saw_partial = false
check(vim.wait(10000, function()
  local s = state.get(buf)
  if not s then
    return false
  end
  for _, notes in pairs(s.partial) do
    if #notes == 1 then
      saw_partial = true
    end
  end
  for i = 1, #s.params.analysis.functions do
    if not state.present(s.params.readings[i]) then
      return false
    end
  end
  return true
end, 50), "readings did not arrive")
check(saw_partial, "notes were not shown while they arrived")
check(not state.present(state.get(buf).params.readings[1].scenarios), "scenarios were written before being asked for")

-- End-of-line annotations: a fact note on line 6.
local marks = vim.api.nvim_buf_get_extmarks(buf, render.ns, 0, -1, { details = true })
local by_line = {}
for _, m in ipairs(marks) do
  local text = {}
  for _, chunk in ipairs(m[4].virt_text) do
    table.insert(text, chunk[1])
  end
  by_line[m[2] + 1] = { text = table.concat(text), hl = m[4].virt_text[1][2] }
end
check(by_line[6] and by_line[6].text:find("● calls add", 1, true), "no fact note on line 6: " .. vim.inspect(by_line))
check(by_line[6].hl == "CodeReadingFact", "fact note is not highlighted as a fact")
check(by_line[9] and by_line[9].text:find("◌ returns the sum", 1, true), "no guess note on line 9")

-- Hover goes through Neovim's own LSP client.
local client = vim.lsp.get_clients({ bufnr = buf, name = "crt" })[1]
check(client ~= nil, "crt client not attached")
local hover = client:request_sync("textDocument/hover", {
  textDocument = { uri = vim.uri_from_bufnr(buf) },
  position = { line = 5, character = 2 },
}, 5000, buf)
check(hover and hover.result and hover.result.contents.value:find("calls add", 1, true), "hover lacks the note")

-- Scenario view: the first :CrScenario asks the server to write them,
-- then lets the user choose one and shows its steps.
-- Stand in for the user picking the first scenario.
---@diagnostic disable-next-line: duplicate-set-field
vim.ui.select = function(items, _, on_choice)
  on_choice(items[1])
end
vim.api.nvim_win_set_cursor(0, { 5, 0 })
cr.scenario()
check(vim.wait(10000, function()
  return vim.api.nvim_get_current_buf() ~= buf
end, 50), "scenario buffer did not open")
local sbuf = vim.api.nvim_get_current_buf()
local rows = vim.api.nvim_buf_get_lines(sbuf, 0, -1, false)
check(rows[1]:find("two Inc at once", 1, true), "scenario title missing: " .. vim.inspect(rows))
check(table.concat(rows, "\n"):find("L6    both read c.n", 1, true), "scenario step missing: " .. vim.inspect(rows))
check(table.concat(rows, "\n"):find("outcome: one increment is lost", 1, true), "outcome missing")
local step_marks = vim.api.nvim_buf_get_extmarks(buf, vim.api.nvim_create_namespace("code-reading-scenario"), 0, -1, {})
check(#step_marks == 2, "source lines of the scenario are not highlighted")

-- The concurrent scenario arrives as a diagnostic with related info.
check(vim.wait(5000, function()
  return #vim.diagnostic.get(buf) > 0
end, 50), "no diagnostics")
local d = vim.diagnostic.get(buf)[1]
check(d.lnum == 5 and d.message:find("two Inc at once", 1, true), "unexpected diagnostic: " .. vim.inspect(d))

-- <CR> on a step jumps back to the source line.
for i, r in ipairs(rows) do
  if r:find("L6", 1, true) then
    vim.api.nvim_win_set_cursor(0, { i, 0 })
    break
  end
end
vim.api.nvim_feedkeys(vim.api.nvim_replace_termcodes("<CR>", true, false, true), "x", false)
check(vim.api.nvim_get_current_buf() == buf and vim.api.nvim_win_get_cursor(0)[1] == 6, "<CR> did not jump to line 6")

-- Toggle hides and shows, and the statusline part follows.
check(cr.status() == "crt on", "status: " .. cr.status())
cr.toggle()
check(#vim.api.nvim_buf_get_extmarks(buf, render.ns, 0, -1, {}) == 0, "toggle did not hide")
check(cr.status() == "crt off", "status: " .. cr.status())
cr.toggle()
check(#vim.api.nvim_buf_get_extmarks(buf, render.ns, 0, -1, {}) > 0, "toggle did not show")

-- :CrConfig opens the configuration file the server reads.
vim.api.nvim_set_current_buf(buf)
cr.open_config()
check(vim.wait(5000, function()
  return vim.api.nvim_buf_get_name(0) == vim.fn.resolve(os.getenv("CRT_CONFIG"))
    or vim.api.nvim_buf_get_name(0) == os.getenv("CRT_CONFIG")
end, 50), "config not opened: " .. vim.api.nvim_buf_get_name(0))

io.stdout:write("OK\n")
vim.cmd("qall!")
