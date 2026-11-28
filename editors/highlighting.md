# Highlighting vocabulary

This is the V1 coverage contract, not a claim that the initial grammar implements every row. Use ordinary Tree-sitter capture conventions and adapt to each host's native faces or token types. The compiler's lexical inventory and canonical language reference own spelling, syntax and reservation rules. Read them again at activation. Syntactic role is not resolved symbol identity.

## Code and data

- `@comment`: ordinary `--` line comments in code and MON, not in template body text. `@comment.documentation` for documentation content where appropriate, and `@comment.todo` for TODO annotations without pretending they are line comments.
- `@keyword`, `@keyword.conditional`, `@keyword.repeat`, `@keyword.return`, `@keyword.exception`, `@keyword.import`, `@keyword.type`: branching, match/readiness arms, loops, returns, catches, casts, aliases, conformance and dependency clauses. Context matters. `case` and `into` follow the delivered syntax. Retired bare words do not get keyword colour.
- `@keyword.directive`: source `$export`, `$config`, `$doc`, `$todo`, `$fast_math`, `$safe_math`, `$infallible`, plus accepted future directives only when their contract exists. Template directives `$slot`, `$in`, `$supplied`, `$before`, `$after`, `$note`, `$raw`, `$escape_html`, `$md`, `$html`, `$css`, `$code`. Unknown directives remain identifiable without a fabricated validity verdict.
- `@operator`: assignment and compound writes, mutable and compile-time markers, arithmetic, comparisons, word operators, range, error/option propagation, return arrows, channel send/receive, qualification and closure arrow in its accepted position. Distinguish postfix type markers and Never return contract structurally.
- `@punctuation.bracket`, `@punctuation.delimiter`, `@punctuation.special`: round, square, brace and pipe delimiters, commas, head/body colon, arm colon, projection dot and qualification delimiters. No semicolon scope marker or fat-arrow match arm in the new syntax.
- `@string`, `@string.escape`, `@character`, `@number`, `@number.float`, `@boolean`, `@constant.builtin`: quoted text and characters, exact-width-independent numeric spelling, booleans, `none`. Distinguish source and MON escape rules. Do not infer literal numeric widths.
- `@type`, `@type.definition`, `@type.builtin`, `@type.parameter`: nominal structs, choices, aliases, type references, generic parameters/arguments, fixed-width numeric types, `Dec` and canonical `Dec0` through `Dec256`, `Bool`, `String`, `Char`, `Byte`, `None`, `True`, `False`, `Error`, `This` where appropriate. Distinguish collection/map/optional type syntax from value construction. Removed Int/Uint/Float/Number and Wire/Route are not builtin aliases.
- `@type.trait`: declared trait names, trait requirements' owning trait, explicit conformance and generic bound references. Map to interface/type where the host has no trait capture. Do not colour every ALL_CAPS identifier as a trait: constants can have that spelling too.
- `@constructor`: struct and choice constructor occurrences where syntax distinguishes them, including qualified/contextual variants. `@constant.enum` for choice variant definitions and explicit variant patterns/references. MON qualifiers follow literal-only syntax.
- `@function`, `@function.call`, `@function.method`, `@function.method.call`, `@function.builtin`: declarations, method/requirement signatures, call and receiver-call shapes, compiler intrinsics such as assert. Calls whose role requires resolution stay a neutral call role until semantic tooling exists.
- `@variable`, `@variable.parameter`, `@variable.builtin`, `@variable.member`, `@property`: bindings, parameters, captures, `this`, field definitions, projections, named arguments and record labels. MON fields are data, never local declarations.
- `@constant`: explicit compile-time binding definitions and syntactically known constants. Do not assume every capitalised name is a constant or every reference has resolved mutability.
- `@module`, `@module.builtin`, `@string.special.path`, `@label`: dependency providers, aliases, qualified reads, file and resource values, reserved root paths, exported route suffixes, builder destinations and named regions. Distinguish paths from Markdown links, annotations and section IDs.
- `@error` or host equivalent: local malformed syntax and recovery ranges only. No type, lifetime, target or schema diagnostics from a CST.

## Templates and documentation

Plain body text uses `@string` or the host's neutral text scope. Moth owns construction delimiters, nested code, directive names, wrapper/head expressions, slots, insert/guard targets and positional keys. Treat discarded `$note` body as a balanced comment. Raw and escaped text have plain presentation while preserving nested Moth contributions. A `$doc` template operand defaults to the same presentation as `$md`, with explicit formatter overrides. This is presentation only: the compiler does not implicitly apply Markdown formatting to `$doc`.

Moth Markdown uses markup headings, strong/emphasis, lists, inline code, link target/label and explicit section-ID captures (e.g. `@markup.heading`, `@markup.strong`, `@markup.italic`, `@markup.list`, `@markup.raw`, `@markup.link.url`, `@markup.link.label`). Use actual Moth Markdown, not CommonMark fences, pipe tables or ordinary Markdown links. Default .mtf content and nested unformatted templates follow the implicit Markdown rule. Moth expressions and explicit guest bodies override it.

`$html` and `$css` (including `"inline"`) require their guest grammar captures. Include HTML tags, attributes, entities, comments, text, CSS selectors, declarations, properties, values, units, strings and comments through native guest parsers. Avoid claiming compile-time CSS validity. Moth interpolation must remain visible.

## Code-listing inventory

The compiler's `LANGUAGE_ALIASES` in `src/compiler_frontend/ast/templates/styles/language_profiles.rs` is the current authority. At the branch inspection the supported aliases were:

- Moth: moth
- JavaScript: js, javascript
- TypeScript: ts, typescript
- Python: py, python
- Rust: rs, rust
- Shell: bash, sh, shell
- HTML: html
- Markdown: md, markdown (code listing guest, distinct from Moth Markdown)
- TOML: toml
- JSON: json
- YAML: yaml, yml
- CSS: css
- C: c
- SQL: sql
- Plain text: txt, text
- No operand: generic code highlighting

A computed operand gets a conservative fallback without evaluation. Unknown or unavailable parsers must not cause incorrect Moth code ranges. Listing brackets are literal balanced source, not live outer template children. Use one guest parser per selected language and appropriate host capture mappings. Avoid treating aliases as new languages. Verify the literal alias inventory and guest availability at activation.

## Acceptance

Every role must have a real source-range witness in its owning corpus or host test. Separate syntax-only coverage from semantic refinements. A function call versus constructor distinction that needs resolution remains neutral until LSP. Distinguish grammar coverage, shared query coverage and actual editor rendering. No requirement for identical colours across themes.
