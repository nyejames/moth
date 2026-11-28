# Moth for VS Code

Status: design only. This is the intended replacement for the existing TextMate-based extension, not an installable package.

## Planned integration

- `package.json` and language configuration for .moth, .mtf and .mon.
- A small TypeScript extension using the shared [Tree-sitter grammar](../tree-sitter-moth/README.md) compiled to Wasm.
- Document-local parser lifecycle, highlighting and guest-language capture mapping.
- Native folding, document symbol and selection providers, language configuration, snippets and explicit compiler tasks.
- Minimal TextMate fallback where needed for initial rendering or host integration.
- Extension-host tests and packaging configuration.

The first milestone is the mixed-language prototype described in [design.md](../design.md). Prove how HTML, CSS, Moth Markdown and nested code listings work with both semantic highlighting enabled and disabled. Handle guest token precedence, UTF-16 positions, cancellation and disposal without a second full Moth grammar. If the public extension APIs cannot deliver a required capability, record the witness and revisit the approach before expanding it.

The extension must work without the compiler. Saved-source diagnostics and eventual LSP use the shared tooling decision in the design. Do not add a VS Code-specific MON parser or scrape compiler terminal output.

Before publishing, inventory the existing extension source and publisher identity. Prefer an upgrade under the existing identity if access permits. Establish the supported VS Code versions, platforms and extension hosts through testing. Use the Marketplace or other approved distribution channels only when release gates pass.

References: [syntax highlighting](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide), [semantic highlighting](https://code.visualstudio.com/api/language-extensions/semantic-highlight-guide) and [extension manifest](https://code.visualstudio.com/api/references/extension-manifest).
