-- with lspconfig
--
if require('lspconfig.configs').maills ~= nil then
  require('lspconfig.configs').maills = nil
end

require('lspconfig.configs').maills_dev = {
  default_config = {
    cmd = { 'target/debug/maills', '--stdio' },
    filetypes = { 'mail' },
    root_dir = function(_)
      return '/'
    end,
  },
}
require('lspconfig').maills_dev.setup { }

-- or without lspconfig
--
-- vim.lsp.start({
--   name = 'maills',
--   cmd = { 'target/debug/maills' },
--   root_dir = '.',
-- })

vim.lsp.set_log_level("DEBUG")
vim.keymap.set('n', 'K', vim.lsp.buf.hover, { noremap = true })
vim.keymap.set('n', 'gd', vim.lsp.buf.definition, { noremap = true })
vim.keymap.set('n', 'ga', vim.lsp.buf.code_action, { noremap = true })
