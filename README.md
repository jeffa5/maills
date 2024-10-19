# Maills: Mail language server

A tool to help with writing emails.

## Actions

- [x] `hover` shows a summary of the contact
- [x] `gotoDefinition` of an email address to view the vcard
- [x] completion for email addresses, and contact names
- [ ] diagnostics for addresses not in your contacts
- [ ] code action to add addresses to contacts

## Installation

### Cargo

Currently, the main way to install maills is by cloning the repo and running

```sh
cargo install --force maills
```

This adds the binary `maills` to the rust bin location.

### Nix

You can also get it on nix, using the flake in this repo:

```sh
nix shell github:jeffa5/maills
```

## Configuration

Capabilities are all enabled by default, but can be disabled in the `initializationOptions` (e.g. to prevent conflicting handling of `hover` or `gotoDefinition`):

```json
{
  "vcard_dir": "~/path/to/contacts",
  "enable_completion": false,
  "enable_hover": false,
  "enable_code_actions": false,
  "enable_goto_definition": false
}
```

### Neovim

For debugging and quickly adding it to neovim you can use the provided `vim.lua` file, provided you have `nvim-lspconfig`.
Just make sure to run `cargo build` and enter `nvim` from the root of this repo.

```sh
nvim test.eml
# then :LspStop
# then :luafile vim.lua
# then :LspStart
# Write some words and hit K to hover one
```

It by default is set up for the `text` and `markdown` filetypes.
