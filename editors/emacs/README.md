# Moth for Emacs

Status: design only. No major mode is installable yet.

Implement Tree-sitter-enabled major modes in Emacs Lisp using the [shared Moth grammar project](../tree-sitter-moth/README.md).

## Planned integration

- A package with major modes and filename associations for .moth, .mtf and .mon.
- Native parser installation and availability checks.
- `treesit` font-lock and indentation rules using shared syntax node shapes and Emacs faces.
- Explicit guest parser ranges and active language selection for mixed template content.
- Imenu, folding, structural navigation and snippets through appropriate native facilities.
- Explicit compiler commands and the eventual shared LSP setup.
- ERT tests and interactive checks for fontification, indentation and editing mixed-language buffers.

Choose the minimum supported Tree-sitter-enabled Emacs build during M1. Do not promise legacy regex fontification support for builds without Tree-sitter. Provide an actionable installation message if a required parser is missing.

Guest ranges must update after edits and leave parent-language interpolation under Moth's control. Respect user configuration and native themes. Adapt shared capture categories to Emacs faces rather than maintain a separate Moth grammar.

Publish through an appropriate Emacs package channel after qualification. See [design.md](../design.md), [shared tests](../tests/README.md) and the Emacs manuals for [Tree-sitter](https://www.gnu.org/software/emacs/manual/html_node/elisp/Parsing-Program-Source.html) and [multiple languages](https://www.gnu.org/software/emacs/manual/html_node/elisp/Multiple-Languages.html).
