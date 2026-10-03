--- End-of-line annotations. One persistent extmark per annotated line, so
--- annotations follow the text while an edit waits for new results.

local state = require("code-reading.state")

local M = {}

M.ns = vim.api.nvim_create_namespace("code-reading")

function M.define_highlights()
  local set = function(name, link)
    vim.api.nvim_set_hl(0, name, { link = link, default = true })
  end
  set("CodeReadingFact", "DiagnosticOk")
  set("CodeReadingGuess", "Comment")
  set("CodeReadingCall", "NonText")
  set("CodeReadingPending", "DiagnosticHint")
  set("CodeReadingStep", "Visual")
end

local MARK = { fact = "● ", inference = "◌ " }
local HL = { fact = "CodeReadingFact", inference = "CodeReadingGuess" }

--- The virtual text chunks for every annotated line, keyed by 1-based line.
--- Exposed for tests.
function M.chunks(params, pending)
  local out = {}
  for i, f in ipairs(params.analysis.functions) do
    local reading = params.readings[i]
    local notes_by_line = {}
    if state.present(reading) then
      for _, n in ipairs(reading.notes) do
        notes_by_line[n.line] = notes_by_line[n.line] or {}
        table.insert(notes_by_line[n.line], n)
      end
    end
    local calls_by_line = {}
    for _, l in ipairs(f.lines) do
      local names = {}
      for _, c in ipairs(l.calls) do
        table.insert(names, c.name)
      end
      calls_by_line[l.line] = names
    end
    for line = f.start_line, f.end_line do
      local chunks = {}
      local notes = notes_by_line[line]
      if notes then
        local n = notes[1]
        local kind = n.basis.kind
        table.insert(chunks, { "  " .. MARK[kind] .. n.text, HL[kind] })
        if #notes > 1 then
          table.insert(chunks, { (" +%d"):format(#notes - 1), "CodeReadingGuess" })
        end
      elseif calls_by_line[line] then
        table.insert(chunks, { "  → " .. table.concat(calls_by_line[line], ", "), "CodeReadingCall" })
      end
      if line == f.start_line and pending[f.hash] then
        table.insert(chunks, { "  ⋯ reading", "CodeReadingPending" })
      end
      if #chunks > 0 then
        out[line] = chunks
      end
    end
  end
  return out
end

function M.draw(bufnr)
  vim.api.nvim_buf_clear_namespace(bufnr, M.ns, 0, -1)
  local s = state.get(bufnr)
  if not (s and state.enabled) then
    return
  end
  local count = vim.api.nvim_buf_line_count(bufnr)
  for line, chunks in pairs(M.chunks(s.params, s.pending)) do
    if line <= count then
      vim.api.nvim_buf_set_extmark(bufnr, M.ns, line - 1, 0, {
        virt_text = chunks,
        virt_text_pos = "eol",
        hl_mode = "combine",
        priority = 90,
      })
    end
  end
end

return M
