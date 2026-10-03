--- What the server last said about each buffer.

local M = { enabled = true }

local by_buf = {}

local function present(v)
  return v ~= nil and v ~= vim.NIL
end

M.present = present

function M.set(bufnr, params, client_id)
  local pending = {}
  for _, h in ipairs(params.pending or {}) do
    pending[h] = true
  end
  -- Notes still arriving, by function hash.
  local partial = {}
  for _, p in ipairs(params.partial or {}) do
    partial[p.functionHash] = p.notes
  end
  by_buf[bufnr] = { params = params, pending = pending, partial = partial, client_id = client_id }
end

function M.get(bufnr)
  return by_buf[bufnr]
end

function M.clear(bufnr)
  by_buf[bufnr] = nil
end

--- The innermost function containing 1-based `line`, with its reading.
--- Returns function, reading (or nil).
function M.function_at(bufnr, line)
  local s = by_buf[bufnr]
  if not s then
    return nil
  end
  local best, best_reading, best_len
  for i, f in ipairs(s.params.analysis.functions) do
    if f.start_line <= line and line <= f.end_line then
      local len = f.end_line - f.start_line
      if not best_len or len < best_len then
        best, best_len = f, len
        local r = s.params.readings[i]
        best_reading = present(r) and r or nil
      end
    end
  end
  return best, best_reading
end

return M
