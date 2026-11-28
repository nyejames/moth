# Moth for Zed

Status: design only. No extension manifest or parser installation is available yet.

Zed is the reference native integration for the first full syntax release. Register the [shared Tree-sitter grammars](../tree-sitter-moth/README.md) through the extension manifest.

## Planned integration

- `extension.toml` with pinned grammar sources and language declarations.
- `languages/` configurations for .moth, .mtf and .mon.
- Highlight, injection, bracket, indentation, outline and text-object queries using Zed's native facilities.
- Snippets and documented explicit compiler commands using the supported host facilities.
- A small Rust/Wasm extension only when a demonstrated feature requires executable extension code, such as future language-server startup.

Start declaratively. In the initial prototype, establish the grammar repository/revision packaging for multiple roots, including guest parser availability. If Zed requires an exported repository, derive it from the monorepo release instead of maintaining a second grammar.

Verify nested formatter queries in the real editor, including Moth Markdown and CSS inline mode. Reuse [shared fixtures](../tests/README.md) and keep Zed-specific tests for host behaviour.

Publish through the Zed registry after the common V1 gates. See the [language extension documentation](https://zed.dev/docs/extensions/languages) and [editor design](../design.md).
