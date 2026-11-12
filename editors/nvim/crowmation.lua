-- Crowmation .cwm support for Neovim 0.11+.
-- Copy to ~/.config/nvim/plugin/crowmation.lua and set the repo path below.
-- Needs `crowmation` on PATH (cargo install --path cli).
local repo = vim.fn.expand("~/projects/Crowmation")

vim.opt.runtimepath:append(repo .. "/editors/nvim") -- syntax and ftplugin
vim.filetype.add({ extension = { cwm = "cwm" } })

vim.lsp.config.crowmation = {
  cmd = { "crowmation", "lsp" },
  filetypes = { "cwm" },
  root_markers = { ".git" },
  on_attach = function(client, buf)
    -- Completion pops up as you type; <C-y> accepts.
    vim.lsp.completion.enable(true, client.id, buf, { autotrigger = true })
  end,
}
vim.lsp.enable("crowmation")
