# Editor support design

Status: approved direction with open prototype decisions. This document describes the intended first release, not implemented support. Syntax authority remains with the delivered language references. Open technical decisions are marked explicitly.

## Goals

Moth should feel like a native language in VS Code, Zed, Neovim and Emacs. The first release covers all three authored formats:

- `.moth`: declarations, statements, expressions and bracket constructions.
- `.mtf`: implicit compile-time Markdown template bodies, including nested Moth expressions and formatter bodies.
- `.mon`: literal data with a root record and shared Moth notation.

Highlighting, including embedded language highlighting, is a release requirement in all four editors. Support folding, useful indentation, bracket matching and closing, context-aware comments, snippets, document outlines and structural selection/navigation through the facilities each editor exposes. Provide convenient explicit compiler commands and a small saved-source diagnostics integration. Highlighting remains local, responsive and usable without the compiler.

Defer full semantic LSP work, completion from resolved types and symbols, cross-file navigation, rename, inlay hints, automatic formatting, debugging and incremental compiler analysis. Helix and additional editors are future candidates, not V1 release gates.

## Authorities and ownership

Read the root [agent rules](../AGENTS.md), the [language index](../docs/developer-docs/language/overview.mtf) and the canonical unsuffixed references at activation. The [compiler architecture](../docs/compiler-design-overview.md), [data layout](../docs/compiler-data-layout-design.md) and [build system](../docs/build-system-design.md) govern compiler handoffs. The [MON reference](../docs/docs/mon/mon-format.mtf) owns data syntax and validation. The formatter reference routed by the language index (split into `docs/docs/template-directives/formatters.mtf` on the syntax branch) and [implicit Markdown reference](../docs/docs/moth-templates/implicit-markdown.mtf) own template modes. Follow the delivered language index at activation rather than assuming the current file layout.

Preparation targets the new syntax documented on `moth-new-syntax`. Read its unified bracket and surface syntax designs and migration plan as temporary navigation, then consume the final canonical language references once the changes land. Do not preserve a second dialect. Recheck the status of closures, channels, declared regions and other accepted but deferred syntax at activation. Syntactic recognition is not a claim of compiler or target support.

Three layers have distinct purposes:

- Tree-sitter: a tolerant, incremental concrete syntax tree and source-based presentation. It has no type system, dependency resolver, evaluator or compiler diagnostics authority.
- Compiler and MON codec: semantic and data validation, project discovery, exact diagnostics and eventual language-server analysis. Neither consumes editor trees as compiler input.
- Editor adapters: file association, native theme and editing facilities, commands, process lifecycle and future LSP registration. They do not implement another Moth grammar.

The selected approach is a single Tree-sitter grammar project with multiple root languages. TextMate alone would leave the native Tree-sitter editors with separate parsers and poor structural features. Driving all highlighting through the compiler or an LSP would require error recovery and source services that do not exist yet and make local highlighting depend on a process. Sharing a Tree-sitter CST with compiler semantic stages would introduce an unrelated frontend refactor. Accept the small duplication of *syntactic* rules in exchange for editor-specific recovery and enforce parity through tests. The compiler's existing documentation code-block formatter remains independent.

## Grammar project

The proposed source layout is:

```text
editors/
  README.md
  design.md
  roadmap.md
  tree-sitter-moth/
  vscode/
  zed/
  neovim/
  emacs/
  tests/
```

The grammar project owns shared JavaScript grammar definitions, generated parser sources and node metadata, portable queries and grammar corpus tests. See [highlighting vocabulary](highlighting.md) for the required syntax roles and formatter inventory. The project uses Node.js LTS, npm and a pinned Tree-sitter CLI. Start with ABI 14 for native host compatibility and revisit it against measured host requirements. Track generated C and node metadata after successful generation, not native or Wasm build products. Generate separate parser entry points for `moth`, `moth_template` and `mon`. A small internal Moth Markdown grammar may be needed for injection. Its packaging and entry point count follow the prototype rather than being fixed now.

Share identifier, literal, named-entry, record, sequence and map productions when their syntax is the same. Parameterise the allowed value forms where necessary. MON rejects arbitrary expressions, calls, templates and references even if a Moth expression could fold. Respect its implicit or explicit root record, commas, spacing, literal qualifier rules and exact string/character escapes. Retain any intentional lexical differences between Moth and MON. Do not approximate schema validation, duplicate key equality or type inference in editor grammar rules.

Moth brackets have a single authored construction shape. A head-only construction such as `[42]` does not by itself tell the editor whether the result is String or a sequence. Distinguish maps, type positions, nested constructions and raw body boundaries syntactically. Leave all resolved types to the compiler. Only a colon at the correct head boundary enters body mode. A map equals sign or arm colon inside a nested operand does not change its parent mode.

Braces delimit blocks and choices in the new grammar. Pipes delimit signatures and captures where the delivered syntax permits them. Preserve logical line boundaries, the contextual `case` rule and ordinary `else` versus fallback `else:`. Follow the final migration rules for inline forms and missing delimiters. Do not classify a word as reserved solely because it appears in examples or because of its naming convention.

Use the compiler-independent `moth-lexical` word inventory (`SourceWord::ALL`), identifier and numeric spelling policies as the lexical reference. At activation, select the smallest generation or conformance test that prevents drift. Keep context-sensitive words, directive names and numeric type families with their actual authorities. A shared inventory does not justify extracting the compiler tokenizer or introducing a general grammar generator.

Prefer the grammar DSL. Use a small external C scanner only where tests demonstrate that contextual lexical boundaries or recovery require it. If used, keep its state bounded and serialisable for incremental reparsing, and prove identical behaviour on native and Wasm builds. It must not evaluate expressions or interpret types. Test actual syntax tree shapes, not just successful parser generation.

## Template modes and embedded highlighting

Moth syntax owns all outer delimiters, even when a formatter body contains another language. In ordinary code, `--` begins a line comment. In a template body it is content. Braces, strings, backticks and colons obey the active mode. A quoted Moth string can contribute literal bracket characters. Raw text must not be retokenized as expressions because of a receiving type.

Recognise a formatter in its authored head, including a head with a wrapper, insert or control suffix. The formatter applies to its own body, not the head or arbitrary descendants. In an `.mtf` file, the implicit outer body and nested constructions with no explicit formatter default to Moth Markdown. Explicit nested formatters override that default. Outside that file kind, do not infer inherited formatting beyond the canonical contract.

V1 classification:

- `$html`: HTML markup and its authored attributes.
- `$css`: CSS stylesheet rules. `$css "inline"` uses CSS declarations.
- `$md`: Moth's Markdown subset, including headings, lists, emphasis, inline code, Moth-aware links and explicit section IDs.
- `$code "language"`: the named supported source language, including Moth itself. The compiler-owned language inventory defines aliases and available names.
- `$code` without an operand: generic code highlighting. `$code "text"`, `$raw` and `$escape_html`: plain text, subject to the applicable child rules.
- `$note`: comment across its entire balanced body, with no nested live Moth highlights.

A plain template operand of `$doc` receives the same Moth Markdown presentation as `$md`. This is an editor-only default, not implicit compiler formatting. Explicit formatters override the default. The documentation directive and its operand keep distinct scopes. `$todo` remains a source annotation with its own role.

The `$code` body keeps balanced brackets as listing source and does not evaluate nested Moth contributions. Highlight that source using the selected language, not ordinary template interpolation. Unknown, unavailable or nonliteral language operands use the documented generic or plain fallback. A valid literal supported language must highlight in a normal supported installation. A missing parser is an explicit degraded state, not a reason to silently claim complete support.

Moth Markdown is not CommonMark. Standard fenced blocks, pipe tables and ordinary Markdown links are not valid Moth Markdown features. Avoid applying those captures to text in Moth Markdown. Prefer a small shared grammar or query strategy that matches the documented subset. Plain `.md` files remain the editor's ordinary Markdown.

A mixed-language body needs source ranges that exclude the Moth head, delimiters, interpolation and child template ranges. For example, in an HTML tag split around a Moth interpolation, the parent parser should receive the ordered pieces of that *one* body, and Moth owns the interpolation's highlight ranges. Nested child bodies get independent language contexts. Never combine unrelated sibling bodies into one embedded document. Use included ranges or the host's equivalent and avoid reparsing the whole buffer once per child. If a guest language needs a synthetic gap to parse coherently, prototype it, preserve exact source-range mappings and keep synthetic text out of the editor buffer.

Some hosts have different query predicates, child exclusion rules, parser availability and CSS entry points. Validate real highlighting rather than assume that one injection query has identical semantics across hosts. Ordinary HTML/CSS/Markdown embeddings and the literal `$code` language inventory are mandatory. Where a host lacks a guest parser, arrange a supported installation or bundled parser and verify licensing. A computed formatter or language operand is outside syntax-only resolution. It must not cause the plugin to execute code.

### Early prototype gates

Before completing the whole grammar, exercise the same small mixed-language fixtures in all four hosts. Include HTML attributes containing interpolation, a CSS declaration body, nested Markdown with links and IDs, a code listing containing Moth brackets and notes, sibling bodies using the same guest language and an unclosed template head. Measure recovery when editing the delimiters. Prove that code and guest captures do not overlap incorrectly.

VS Code is the main integration risk because the public extension APIs do not offer the same native injection mechanism as Zed and Neovim. Test token appearance with semantic highlighting enabled and disabled, language configuration behaviour inside guest bodies and parser availability. Do not lower the release requirement or turn the fallback TextMate grammar into a full competing parser to get around a failed prototype. Bring a concrete limitation and options to the project lead before selecting a different approach.

## Editor adapters

Keep the canonical syntax capture categories and shared query patterns with the grammar. Editors map them to native capture names, face names, token types and editing interfaces. Shared structure is preferable to exact colour parity. Respect the user's theme and existing settings.

- **Zed:** register grammars by pinned repository/revision in the extension manifest and define language configurations and queries for highlights, brackets, indents, outlines, text objects and injections. Prefer a declarative extension for the syntax slice. A Rust/Wasm extension may be required for language-server startup or another demonstrated need.
- **Neovim:** register filetypes and parser languages in Lua, use the native Tree-sitter query and highlighting APIs and provide fold/indent/navigation configuration. Integrate with current native or plugin-manager parser installation. Do not require a second parsing engine.
- **Emacs:** use Tree-sitter-enabled major modes with `treesit` font-lock rules, indentation rules and Imenu/navigation. Explicitly configure guest parser ranges and active language selection as needed. Determine the minimum supported build with the relevant Tree-sitter API during the prototype.
- **VS Code:** use a TypeScript extension with a bundled Wasm parser and the public document semantic token, folding, symbol and selection providers. Prototype guest parser queries and compose their captures into non-overlapping document tokens. A migrated copy of the existing TextMate grammars is a practical starting point for early/theme-compatible fallback and embedded language scopes where needed. Keep the useful mode and query-like rules, remove stale syntax and test the candidate against the same fixtures before deciding its production role. Do not ship two equally ambitious independent syntactic engines unless the prototype demonstrates a necessary supported requirement. Use the shared grammar and query facts for rich highlighting.

Use editor-native comments, auto-closing and indentation where they respect syntax modes. Comment commands must not insert Moth's line-comment marker into raw template text as if it were a comment. Prefer conservative behaviour in an ambiguous context. Snippets insert only canonical syntax. Avoid changing global user settings or automatically running a build.

## Compiler tooling and MON

The existing compiler can remain unchanged throughout the syntax-only implementation. The branch currently proposes no compiler changes. On activation inspect the structured report owner in `src/projects/check.rs`, CLI parsing and presentation in `crates/moth-cli/src/cli.rs` and the standalone `moth-mon` crate. The user-facing machine-readable command is:

```sh
moth check . --mon
```

`--mon` selects one MON document on standard output, rather than adding a general diagnostic-format option. Its format and producer need a separate small compiler slice after the branch restriction is lifted. Use the existing typed report and codec rather than scraping `--terse` or reimplementing semantic diagnostics. Plain and terse output remain human-facing. Reject mutually exclusive output switches. Preserve exit status, deterministic order, diagnostic discovery stops and the distinct diagnosed, infrastructure and compiler-bug lanes. Operational or process failure may prevent a complete document, which consumers must handle as a tool failure. Keep progress, timing and incidental text off the machine-readable stdout stream. Decide the complete failure envelope and verify it end-to-end before promising a stable protocol.

The proposed owned diagnostic document contains a schema version, request outcome, diagnostic codes, severity, message, resolved source locations, exact half-open UTF-8 byte offsets, related locations and coverage/stop metadata. Source-less diagnostics remain source-less. Expose source identity as resolved paths or URI-compatible facts with an explicit policy for native path representation. Do not serialise compact compiler-local IDs or key by lossy path display strings. Choose exact scalar receiving types and valid MON field labels. Include snapshot identity or sufficient evidence that the referenced saved file has not changed since analysis. The actual schema and names remain open until the consumer/codec spike verifies them.

Moth owns this interchange contract, so use MON and the existing Rust codec. Do not use JSON for an owned format without a specific technical reason. Tree-sitter metadata, package manifests and standard LSP JSON-RPC are externally defined formats, not violations of that rule. The syntax tree for a MON document is not a production codec.

Consider a tiny diagnostics-only LSP as one possible delivery mechanism before creating four native subprocess consumers. Existing clients in the four editors can consume standard diagnostics. A small shared server could read structured report views in-process or consume `check --mon` if process isolation is useful, then adapt to mandatory LSP JSON-RPC. Do not add a MON-to-JSON subprocess merely to parse data which the same Rust process already owns. If native CLI adapters are simpler, use a shared codec binding or a justified single MON adapter approach, not four new parsers. Choose this boundary from a prototype and record the decision before implementation. The MON CLI remains useful for other tools regardless of the chosen integration.

Checks run only on saved source in this release. Use one in-flight check per selected project and configuration, coalesce subsequent saves, discard stale results and clear obsolete diagnostics without replacing them with an older result. Use explicit workspace/project selection, including nested module roots, single-file checks and active builder/input context. Reuse the compiler's discovery policy. Never treat a blocked or skipped module as verified. Unsaved text uses syntax tooling and does not receive diagnostics for unrelated disk text.

Treat compiler execution as a user-controlled operation. Honour workspace trust or explicit opt-in, avoid shell interpolation, use argument arrays, respect configured binary paths, handle unavailable or incompatible binaries and keep editor operation independent of the compiler. No automatic execution of workspace-local binaries or third-party code for highlighting. Development/build commands are explicit tasks. Use retained source snapshots and line indexes to convert UTF-8 byte ranges to host-specific positions. LSP position encoding follows negotiation (UTF-16 default). Test non-ASCII, astral characters, CRLF, empty ranges and multi-line labels.

## Packaging and version compatibility

Keep source in this monorepo. Generated parser C, node types, native/Wasm parser binaries, guest parsers and host query files must come from reproducible pinned revisions. Pin a compatible Tree-sitter CLI/runtime ABI and minimum supported editor versions based on the spike. Use host-required metadata and lockfiles. Do not commit native build products to the source tree unless the actual release process requires them.

Zed may need an exported standalone grammar repository, and Neovim parser registries may require one. Confirm whether each supports subdirectory or multiple grammar packaging before choosing the release layout. If needed, publish a reproducible projection of the monorepo directory, never a second hand-maintained source of truth. Distribute VS Code through its existing extension identity where access permits, Zed through its registry, Neovim through plugin managers and Emacs through an appropriate package channel. Establish ownership of publisher credentials and the existing VS Code extension before publication. Removal or replacement of the old extension occurs only after the new one passes release gates.

Public editor and grammar releases use Obvious Versioning (ObVer) with the Moth ecosystem. A compatible feature increments MINOR, a compatible fix increments MICRO and a breaking public contract requires a new UTC date-based major. First official release starts at MAJOR.0. Emit canonical versions without .0 MICRO. Labels identify immutable alternative builds and have no ordering precedence. Exact build selection includes the label. ObVer has no version ranges. Internal Rust crates and compiler crate dependencies use SemVer. npm, Tree-sitter metadata and editor registries use SemVer where required. A registry version is a separate monotonically valid packaging identity, mapped explicitly to the full public ObVer identifier and source revision. Do not reinterpret date-qualified ObVer as SemVer or claim the mapping is reversible without metadata. No official release version or conversion algorithm is chosen during preparation. See [versioning.md](versioning.md).

The compiler's exact version and the editor grammar's syntax revision are distinct compatibility facts. No legacy grammar version switches in V1. Install/update instructions must make the supported syntax and compiler versions clear. Highlighting works offline after supported parser assets are installed. Verify licensing and attribution for third-party parsers and packaging.

## Technical references

Consult the current documentation and pin versions during the prototype rather than assuming API details stay stable.

- [Tree-sitter grammar guide](https://tree-sitter.github.io/tree-sitter/creating-parsers/)
- [Tree-sitter queries](https://tree-sitter.github.io/tree-sitter/using-parsers/queries/)
- [VS Code syntax highlighting](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide) and [semantic highlighting](https://code.visualstudio.com/api/language-extensions/semantic-highlight-guide)
- [Zed language extensions](https://zed.dev/docs/extensions/languages)
- [Neovim Tree-sitter integration](https://neovim.io/doc/user/treesitter/)
- [Emacs multi-language parsing](https://www.gnu.org/software/emacs/manual/html_node/elisp/Multiple-Languages.html)
- [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)
