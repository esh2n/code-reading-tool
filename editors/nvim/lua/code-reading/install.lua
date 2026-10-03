--- Downloads the crt binary for this platform from a release, checks its
--- SHA-256 against the published checksum, and puts it where `setup()` looks
--- first. Uses `curl` and `tar`, which ship with macOS, Linux and Windows 10+.

local M = {}

M.version = "0.1.0"
M.base_url = "https://github.com/esh2n/code-reading-tool/releases/download"

--- The Rust target triple of this machine, or nil with a reason.
function M.target()
  local u = vim.uv.os_uname()
  local arch = ({ arm64 = "aarch64", aarch64 = "aarch64", x86_64 = "x86_64", AMD64 = "x86_64" })[u.machine]
  if not arch then
    return nil, "unsupported CPU architecture " .. tostring(u.machine)
  end
  if u.sysname == "Darwin" then
    return arch .. "-apple-darwin"
  elseif u.sysname == "Linux" then
    return arch .. "-unknown-linux-gnu"
  elseif u.sysname:match("^Windows") then
    return arch .. "-pc-windows-msvc"
  end
  return nil, "unsupported operating system " .. u.sysname
end

--- Where the installed binary lives.
function M.bin_path()
  local exe = vim.uv.os_uname().sysname:match("^Windows") and "crt.exe" or "crt"
  return vim.fs.joinpath(vim.fn.stdpath("data"), "code-reading", "bin", exe)
end

function M.asset_name(target)
  return ("crt-%s.tar.gz"):format(target)
end

local function read_bytes(path)
  local f = io.open(path, "rb")
  if not f then
    return nil
  end
  local data = f:read("*a")
  f:close()
  return data
end

--- curl restricted to https (and file:// for local testing), including
--- across redirects.
local CURL = { "curl", "-fsSL", "--proto", "=https,file", "--proto-redir", "=https" }

local function run(cmd)
  local r = vim.system(cmd, { text = true }):wait()
  if r.code ~= 0 then
    return nil, (table.concat(cmd, " ") .. ": " .. (r.stderr or ""))
  end
  return true
end

--- Installs the binary. Returns the path, or nil and an error message.
--- opts: { version = "0.1.0", base_url = "...", target = "..." }
function M.install(opts)
  opts = opts or {}
  local target, why = opts.target, nil
  if not target then
    target, why = M.target()
  end
  if not target then
    return nil, why
  end
  local version = opts.version or M.version
  local base = (opts.base_url or M.base_url) .. "/v" .. version .. "/"
  local asset = M.asset_name(target)

  local tmp = vim.fn.tempname()
  vim.fn.mkdir(tmp, "p")
  local archive = vim.fs.joinpath(tmp, asset)
  local sums = archive .. ".sha256"
  local ok, err = run(vim.list_extend(vim.deepcopy(CURL), { "-o", archive, base .. asset }))
  if not ok then
    return nil, err
  end
  ok, err = run(vim.list_extend(vim.deepcopy(CURL), { "-o", sums, base .. asset .. ".sha256" }))
  if not ok then
    return nil, err
  end

  local expected = (read_bytes(sums) or ""):match("^%s*(%x+)")
  local actual = vim.fn.sha256(read_bytes(archive) or "")
  if not expected or expected:lower() ~= actual then
    return nil, ("checksum mismatch for %s: expected %s, got %s"):format(asset, tostring(expected), actual)
  end

  ok, err = run({ "tar", "-xzf", archive, "-C", tmp })
  if not ok then
    return nil, err
  end
  local exe = vim.fs.basename(M.bin_path())
  local extracted = vim.fs.joinpath(tmp, exe)
  if not vim.uv.fs_stat(extracted) then
    return nil, asset .. " does not contain " .. exe
  end
  local dest = M.bin_path()
  vim.fn.mkdir(vim.fs.dirname(dest), "p")
  assert(vim.uv.fs_copyfile(extracted, dest))
  vim.uv.fs_chmod(dest, tonumber("755", 8))
  vim.fn.delete(tmp, "rf")
  return dest
end

return M
