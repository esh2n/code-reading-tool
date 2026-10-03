--- The scenario view: pick a scenario of the function under the cursor, see
--- its steps in a side buffer, and the lines it goes through highlighted in
--- the source. <CR> on a step jumps to its line.

local state = require("code-reading.state")

local M = {}

local ns = vim.api.nvim_create_namespace("code-reading-scenario")

local LABEL = { normal = "normal", boundary = "boundary", concurrent = "concurrent" }

--- The lines of the scenario buffer, and which source line each step row
--- points at (keyed by 1-based row). Exposed for tests.
function M.lines(fn, sc)
  local rows = {
    ("%s %s — %s (%s)"):format(fn.kind, fn.name, sc.title, LABEL[sc.kind] or sc.kind),
    "guess: reasoned from the code, not executed",
    "",
    "input:   " .. sc.input,
    "",
  }
  local targets = {}
  for _, step in ipairs(sc.steps) do
    table.insert(rows, ("  L%-4d %s"):format(step.line, step.what))
    targets[#rows] = step.line
  end
  table.insert(rows, "")
  table.insert(rows, "outcome: " .. sc.outcome)
  if #sc.assumptions > 0 then
    table.insert(rows, "assumes:")
    for _, a in ipairs(sc.assumptions) do
      table.insert(rows, "  - " .. a)
    end
  end
  return rows, targets
end

local function show(src_buf, src_win, fn, sc)
  local rows, targets = M.lines(fn, sc)
  vim.api.nvim_buf_clear_namespace(src_buf, ns, 0, -1)
  for _, step in ipairs(sc.steps) do
    vim.api.nvim_buf_set_extmark(src_buf, ns, step.line - 1, 0, {
      line_hl_group = "CodeReadingStep",
      priority = 80,
    })
  end

  vim.cmd("botright vsplit")
  local buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_win_set_buf(0, buf)
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, rows)
  vim.bo[buf].modifiable = false
  vim.bo[buf].bufhidden = "wipe"
  vim.bo[buf].filetype = "code-reading-scenario"
  pcall(vim.api.nvim_buf_set_name, buf, ("crt://scenario/%s/%s"):format(fn.name, sc.title))

  vim.keymap.set("n", "<CR>", function()
    local target = targets[vim.api.nvim_win_get_cursor(0)[1]]
    if target and vim.api.nvim_win_is_valid(src_win) then
      vim.api.nvim_set_current_win(src_win)
      vim.api.nvim_win_set_cursor(src_win, { target, 0 })
    end
  end, { buffer = buf, desc = "Jump to this step's line" })
  vim.keymap.set("n", "q", "<cmd>close<cr>", { buffer = buf, desc = "Close the scenario" })
  vim.api.nvim_create_autocmd("BufWipeout", {
    buffer = buf,
    once = true,
    callback = function()
      if vim.api.nvim_buf_is_valid(src_buf) then
        vim.api.nvim_buf_clear_namespace(src_buf, ns, 0, -1)
      end
    end,
  })
  return buf
end

--- Opens the scenario view for the function under the cursor.
function M.open()
  local src_buf = vim.api.nvim_get_current_buf()
  local src_win = vim.api.nvim_get_current_win()
  local line = vim.api.nvim_win_get_cursor(src_win)[1]
  local fn, reading = state.function_at(src_buf, line)
  if not fn then
    vim.notify("crt: no function under the cursor", vim.log.levels.WARN)
    return
  end
  if not reading or #reading.scenarios == 0 then
    vim.notify("crt: no scenarios for " .. fn.name .. " yet (:CrRead to read it)", vim.log.levels.INFO)
    return
  end
  vim.ui.select(reading.scenarios, {
    prompt = "Scenario for " .. fn.name,
    format_item = function(sc)
      return ("[%s] %s"):format(LABEL[sc.kind] or sc.kind, sc.title)
    end,
  }, function(sc)
    if sc then
      show(src_buf, src_win, fn, sc)
    end
  end)
end

return M
