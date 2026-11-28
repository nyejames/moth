# Moth for Neovim

Status: design only. No plugin is installable yet.

Use Neovim's native Tree-sitter integration and the [shared Moth grammar project](../tree-sitter-moth/README.md). Lua owns editor configuration, not language parsing.

## Planned integration

- Filetype detection for .moth, .mtf and .mon.
- Buffer-local setup through `ftplugin/` or the equivalent native filetype API.
- Parser registration and a documented installation route for the native parser libraries and guest parsers.
- `queries/` for highlights, injections and structural features, reusing the common query sources where possible.
- Small Lua modules for folds, indentation, text objects/navigation, snippets and optional compiler tasks where native facilities need configuration.
- Headless tests and real-editor checks for buffer lifecycle, captures and commands.

Decide the minimum Neovim version and parser installation method in M1. Avoid requiring a separate parsing engine or an untested dependency on a particular plugin manager. A registry entry is useful if it can consume a reproducible grammar release.

Use buffer-local settings and respect user overrides. Missing guest parsers should degrade gracefully without disabling Moth highlighting, with a clear installation remedy. Test nested injections and range grouping on the supported version. Do not globally combine unrelated template bodies.

Compiler diagnostics follow the shared tooling design and eventual native LSP integration, not a Lua implementation of MON. See [design.md](../design.md), [shared tests](../tests/README.md) and the [Neovim Tree-sitter documentation](https://neovim.io/doc/user/treesitter/).
