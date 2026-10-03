if vim.g.loaded_code_reading then
  return
end
vim.g.loaded_code_reading = true

local function cr()
  return require("code-reading")
end

vim.api.nvim_create_user_command("CrRead", function(o)
  cr().read(o.bang)
end, { bang = true, desc = "Explain the function under the cursor (! asks the model again)" })

vim.api.nvim_create_user_command("CrScenario", function()
  cr().scenario()
end, { desc = "Show a behaviour scenario of the function under the cursor" })

vim.api.nvim_create_user_command("CrToggle", function()
  cr().toggle()
end, { desc = "Show or hide code-reading annotations" })
