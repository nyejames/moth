# Moth editor support

Status: initial grammar and fallback preparation. No editor package is release-ready.

This directory owns the official editor integrations for Moth, Moth template files and MON. The first release supports VS Code, Zed, Neovim and Emacs through a shared Tree-sitter grammar project. Highlighting works without an installed compiler or language server.

- [Design](design.md): goals, syntax ownership, embedded languages, editor integration and compiler tooling boundaries.
- [Roadmap](roadmap.md): ordered work, prerequisites and release gates.
- [Highlighting vocabulary](highlighting.md): V1 capture roles and embedded language requirements.
- [Versioning](versioning.md): public ObVer and internal SemVer boundaries.
- [Tree-sitter grammars](tree-sitter-moth/README.md): shared grammar, queries and parser tests.
- [VS Code](vscode/README.md): TypeScript extension and compatibility fallback.
- [Zed](zed/README.md): declarative language extension.
- [Neovim](neovim/README.md): Lua integration.
- [Emacs](emacs/README.md): Tree-sitter major modes.
- [Integration tests](tests/README.md): shared editor behaviour and conformance fixtures.

## Branch boundary

Preparation on this branch changes only `editors/`. The branch targets main independently of the syntax migration. Read the new syntax authorities on the migration branch while preparing, then rebase onto the delivered syntax and read the canonical references again before activating syntax-specific implementation.

The compiler, root manifests, CI, index and project roadmap remain untouched during preparation. No compiler command or release capability is implied by these documents. After activation, coordinate any compiler-side work separately and add the normal roadmap and index entries at the appropriate checkpoint.

Keep the source of truth for each behaviour in one place. The editor grammar describes authored syntax and error recovery, while compiler and MON validation remain authoritative. Each editor adapts shared syntax facts to its native interfaces.

Use the repository's [agent rules](../AGENTS.md) and routed style, testing and validation references. Keep implementation and its tests within the owning directory. Add files when they acquire a real consumer rather than creating empty implementation modules in anticipation of future work.
