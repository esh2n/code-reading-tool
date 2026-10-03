if vim.g.loaded_code_reading then
  return
end
vim.g.loaded_code_reading = true

local function cr()
  return require("code-reading")
end

vim.api.nvim_create_user_command("CrRead", function(o)
  cr().read(o.bang)
end, { bang = true, desc = "Explain the lines of the function under the cursor (! asks the model again)" })

vim.api.nvim_create_user_command("CrScenario", function(o)
  cr().scenario(o.bang)
end, { bang = true, desc = "Show a behaviour scenario of the function under the cursor (! writes them again)" })

vim.api.nvim_create_user_command("CrToggle", function()
  cr().toggle()
end, { desc = "Turn explanations on or off (off: nothing shown, no model calls unless asked)" })

vim.api.nvim_create_user_command("CrConfig", function()
  cr().open_config()
end, { desc = "Open crt's configuration (endpoint, model, API key, language)" })

vim.api.nvim_create_user_command("CrInstall", function(o)
  local install = require("code-reading.install")
  vim.notify("crt: downloading…")
  local path, err = install.install({ version = o.args ~= "" and o.args or nil })
  if path then
    vim.notify("crt: installed " .. path .. " (restart Neovim to use it)")
  else
    vim.notify("crt: install failed: " .. err, vim.log.levels.ERROR)
  end
end, { nargs = "?", desc = "Download the crt binary for this platform (optional version)" })
