# Moth for VS Code

Status: unpublished TextMate fallback candidate. It is not the completed TypeScript/Tree-sitter extension.

## Imported fallback

Migrated from [moth-vscode-highlighting](https://github.com/nyejames/moth-vscode-highlighting/tree/392641f3476a05bd73951832e3ce45536775ae4c) at commit `392641f3476a05bd73951832e3ce45536775ae4c`. Retain its Apache-2.0 license, language IDs and existing publisher/name (`Nye.moth-highligher`). The original repository is untouched. The placeholder SemVer manifest is not an ObVer release and must not be published.

Reuse the existing nested body, Markdown, dependency and listing rules. Update new brace/arm and square bracket syntax presentation, remove obsolete keyword/raw-string roles, retain `$fresh`, add `$supplied`, documentation Markdown presentation, trait scopes, additional literal guest listing routes and an initial MON fallback. The original bundled icons, old VSIX builds and retired fixtures are not imported. The old test harness informs the focused tests here.

```sh
npm ci
npm run test:static
npm test
```

The real tokenizer and editor integration are not verified in this environment. This is a candidate, not complete syntax coverage. The guest registry must supply matching external grammars. Generic and explicit plain text currently share the simple fallback. Full nesting across guest strings, comments, Moth-specific Markdown and CSS inline declarations remain host prototype work. Review the port against the final syntax before replacing the old extension.

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
