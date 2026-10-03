-- Headless test of :CrInstall's logic against a local "release" directory.
-- Env: CRT_PLUGIN, CRT_RELEASE (a file:// base URL), CRT_TARGET.
local function fail(msg)
  io.stderr:write("FAIL: " .. msg .. "\n")
  vim.cmd("cquit 1")
end

vim.opt.runtimepath:prepend(os.getenv("CRT_PLUGIN"))
local install = require("code-reading.install")

local target = install.target()
if not target then
  fail("no target for this machine")
end

-- A good archive installs and runs.
local path, err = install.install({ base_url = os.getenv("CRT_RELEASE"), version = "9.9.9", target = os.getenv("CRT_TARGET") })
if not path then
  fail("install failed: " .. tostring(err))
end
local r = vim.system({ path, "--version" }, { text = true }):wait()
if r.code ~= 0 or not r.stdout:find("crt", 1, true) then
  fail("installed binary does not run: " .. vim.inspect(r))
end

-- A tampered archive is refused.
local _, bad = install.install({ base_url = os.getenv("CRT_RELEASE"), version = "6.6.6", target = os.getenv("CRT_TARGET") })
if not (bad and bad:find("checksum mismatch", 1, true)) then
  fail("tampered archive was not refused: " .. tostring(bad))
end

io.stdout:write("OK\n")
vim.cmd("qall!")
