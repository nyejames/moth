# Automatic Markdown heading section links implementation plan

## Purpose

Add deterministic automatic HTML IDs to statically known `$md` headings so existing Moth-aware
links can address document sections without manual HTML wrappers or new heading syntax.

Moth template `.mtf` files inherit the behaviour through their existing implicit `$md` body. The
Markdown formatter owns the feature. It must not add a builder HTML scan, document-global compiler
state or a second heading parser.

## Current-state capsule

```text
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh `$md` heading, inline rendering and documentation owners
BLOCKERS: build configuration values and project globals must be delivered first
NEXT_ACTION: activate after the prerequisite and start Phase 0 inventory
```

Keep this block concise. Establish the active revision, branch, worktree state and validation
baseline in untracked working notes when implementation starts. Do not pin a queued plan to a
commit.

## Roadmap position

This plan runs immediately after build configuration values and project globals and before the
diagnostics and tokens optimised memory layout plan.

At closeout, delete this plan and remove its roadmap entry in the same commit.

## Hard prerequisites

- build configuration values and project globals are delivered
- the final TIR exact-view formatter pipeline is delivered and formatters see only text and opaque
  anchors
- the current `$md` heading, inline-code and Moth-aware link paths are stable
- Moth-aware links already preserve same-page fragments and route fragments
- `.mtf` preparation already creates one implicit compile-time `$md` content value
- the Basic and Advanced documentation source layout and release build are working

Name delivered capabilities rather than citing temporary plans as semantic authorities.

## Required authorities

Read these from the active worktree before implementation:

- `AGENTS.md`
- `docs/compiler-design-overview.md`, especially `Frontend stages > Stage 4: AST semantics >
  Templates and TIR`
- `docs/build-system-design.md`, especially site-root URL rendering and HTML output ownership
- `docs/src/docs/templates/markdown-formatting-basic.mtf`
- `docs/src/docs/templates/markdown-formatting.mtf`
- `docs/src/docs/moth-templates/implicit-markdown-basic.mtf`
- `docs/src/docs/moth-templates/implicit-markdown.mtf`
- `docs/src/docs/markdown/rendering-contract-basic.mtf`
- `docs/src/docs/markdown/rendering-contract.mtf`
- `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf`
- `docs/src/developer-docs/style-guide/style-guide.mtf`
- `docs/src/developer-docs/style-guide/testing.mtf`
- `docs/src/developer-docs/style-guide/validation.mtf`
- `docs/src/docs/progress/@page.moth`
- `docs/roadmap/roadmap.md`

# Accepted design

## Source and output contract

A static `$md` heading receives an HTML `id` derived from its visible label:

```moth
content #= [$md:
    # Hello
]
```

```html
<h1 id="hello">Hello</h1>
```

Existing Moth-aware links address the generated fragment:

```moth
@#hello (Hello)
@/docs/message#hello (Hello)
@./message#hello (Hello)
```

The first target is a same-page fragment. The other targets keep the fragment inside the existing
site-root or relative URL. This plan adds no heading syntax, link syntax, explicit section
declaration or automatic anchor element around the heading. It adds the `id` attribute only.

Automatic IDs apply to explicit `$md` templates and to implicit `$md` content in `.mtf` files. They
apply to all currently supported heading levels. Plain `.md` files remain on the separate
`pulldown-cmark` renderer and do not gain this behaviour in this plan.

## Static visible heading labels

A heading receives an ID only when its complete visible label is known from formatter-visible static
content.

- ordinary text contributes its visible characters
- emphasis markers contribute no label text
- inline code contributes its content without backtick delimiters
- a Moth-aware link contributes its label and not its target
- HTML-sensitive authored text contributes its visible characters before output escaping
- any opaque anchor anywhere in the heading disables the automatic ID for the whole heading

Opaque anchors include dynamic expressions and child templates. The formatter must not inspect their
contents or derive a partial ID from surrounding text.

```moth
[$md:
    # Hello [name]
]
```

This renders the normal heading body without an `id`. It is valid source and produces no warning.

The visible-label path must share the current inline interpretation or its focused parser helpers.
It must not parse rendered HTML, duplicate link grammar or inspect TIR content behind an opaque
anchor.

## Heading ID normalisation

Normalise the complete static visible label with one Markdown-local helper:

1. Lowercase Unicode letters through the standard character mapping.
2. Keep Unicode letters and digits.
3. Treat whitespace, `_` and `-` runs as one pending `-` separator.
4. Discard other punctuation.
5. Omit leading and trailing separators.
6. Omit the `id` when no letters or digits remain.

Examples:

| Heading label | Generated ID |
|---|---|
| `Hello` | `hello` |
| `Hello World` | `hello-world` |
| `What's new?` | `whats-new` |
| `Compiler - architecture` | `compiler-architecture` |
| inline code containing `String values` | `string-values` |
| a link whose label is `Templates` | `templates` |
| `!!!` | no automatic ID |

The generated alphabet is Unicode alphanumeric characters plus ASCII `-`. Never concatenate raw
heading text directly into an HTML attribute.

## Duplicate headings

The mapping is pure and independent of heading order. Equal normalised labels produce equal IDs.

Do not add `-1`, `-2` or another order-dependent suffix. Do not add a formatter-global or
document-global slug registry. Adding an earlier heading must not rename a later fragment.

The compiler produces no duplicate-ID diagnostic. Authors are responsible for unique linkable
heading labels in one final document. The documentation phase must audit generated pages for
collisions with automatic and existing manual IDs, then improve documentation headings where needed.

## Link and ownership boundaries

The existing `$md` link parser already accepts same-page fragments and keeps fragments in relative,
site-root, protocol-relative and absolute targets. A route such as
`@/docs/message#hello (Hello)` must retain `#hello` after the builder renders the site-root prefix.
Section links create no path resolution, file discovery or resource identity.

The Markdown formatter owns the complete output path:

```text
parsed heading atoms
-> shared inline recognition and optional static visible label
-> Markdown-local ID normalisation
-> <hN id="..."> output when an ID exists
```

Keep heading recognition in the current Markdown parser, heading element emission in the block
renderer and link or inline-code recognition in their existing owners. Use ordinary formatter text
for the attribute. Add no TIR node, formatter anchor kind, AST side table, HIR field,
public-interface fact, builder metadata or HTML post-processing pass.

A focused Markdown-local module is acceptable if label collection and normalisation would otherwise
make the block renderer own several concerns. Do not create a generic project-wide slug utility or a
wrapper module for a trivial helper.

## Silent omission cases

These remain valid and produce no new diagnostics:

- a dynamic or child-template heading with no automatic ID
- a static heading whose normalised label is empty
- two headings that normalise to the same ID

Malformed heading, inline-code and link syntax keep their current literal fallback or diagnostics.

# Non-goals

- no automatic heading IDs for plain `.md` files
- no explicit custom heading-ID syntax
- no user-selected slug algorithm or transliteration table
- no GitHub or CommonMark slug-compatibility claim
- no duplicate suffixing, collision registry or compiler duplicate-ID diagnostic
- no table-of-contents generation
- no permalink icon or automatic self-link around headings
- no route, path, resource or site-root syntax change
- no rendered-HTML parsing or builder post-processing
- no TIR, HIR, public-interface or backend representation for heading IDs
- no compatibility path that retains old and new heading output together

# Implementation rules

- Read the active worktree rather than relying on paths named in this queued plan.
- Preserve one Markdown heading parser and one inline grammar.
- Keep the normaliser local, deterministic and independent of document order.
- Stop label collection when an opaque heading atom appears but continue normal heading rendering.
- Avoid a visible-label allocation for non-heading blocks.
- Preserve current HTML escaping and opaque-anchor order.
- Keep tests outside production files.
- Prefer focused formatter unit tests plus rendered-output `.moth` and `.mtf` integration cases.
- Do not edit `docs/release/**` by hand.

Before each code-bearing phase is accepted, re-read the affected authorities, run a read-only diff
audit, resolve actionable findings, run focused tests, run the integration audit, run
`just validate`, run `git diff --check` and commit one coherent checkpoint.

# Phase 0 - Refresh owners and baseline

## Goal

Re-anchor the plan after build configuration values and project globals land. Confirm current
formatter, source-kind, test and documentation owners before editing behaviour.

## Work

- [ ] Read every required authority from the active worktree.
- [ ] Record HEAD, branch, status and active worktrees in untracked working notes.
- [ ] Confirm all hard prerequisites are delivered.
- [ ] Inventory heading parsing and rendering, inline rendering, inline-code and link parsing,
  formatter output and opaque-anchor adaptation.
- [ ] Inventory Markdown unit tests and relevant `.moth` and `.mtf` integration cases.
- [ ] Inventory Basic and Advanced docs for `$md`, `.mtf` and plain `.md` rendering.
- [ ] Inspect representative generated docs for current manual IDs and fragment links.
- [ ] Record exact baseline results and unrelated failures without weakening gates.

Suggested searches:

```bash
rg -n 'parse_heading_line|render_heading_line|render_inline_atoms' \
  src/compiler_frontend/ast/templates
rg -n 'try_parse_link_at_atoms|inline_code|FormatterOpaqueKind' \
  src/compiler_frontend/ast/templates
rg -n 'markdown-formatting|implicit-markdown|rendering-contract' docs/src/docs
rg -n 'href=.*#|id=' docs/src docs/release
```

## Validation

```bash
cargo fmt --all -- --check
cargo test --workspace --quiet markdown -- --format terse
cargo run --quiet -- tests --audit
just validate
```

- [ ] Confirm `.mtf` still uses implicit `$md` and plain `.md` still uses its separate renderer.
- [ ] Commit only if this queued plan needs factual corrections.

# Phase 1 - Derive static labels and normalised IDs

## Goal

Add one static visible-label path and one deterministic Markdown-local normaliser without changing
heading HTML yet.

## Work

- [ ] Add an optional heading-label collection path to the existing inline interpretation.
- [ ] Keep paragraph and list rendering on the current no-label fast path.
- [ ] Reuse the current Moth-aware link parser so a link contributes only its label.
- [ ] Reuse the current inline-code parser so code contributes content without delimiters.
- [ ] Preserve emphasis, escaping and literal fallback behaviour.
- [ ] Return no static label as soon as an opaque heading atom occurs.
- [ ] Add the private normaliser with pending-separator state and no repeated temporary strings.
- [ ] Return no ID for an empty normalised result.
- [ ] Keep the helper independent of heading level, source location and document order.

## Coverage and audit

- [ ] Cover case folding, spaces, tabs, `_`, `-`, repeated separators and edge trimming.
- [ ] Cover punctuation removal, empty results and Unicode letters or digits without
  transliteration.
- [ ] Cover emphasis, inline code, link labels and escaped authored text.
- [ ] Cover a link target containing words absent from its visible label.
- [ ] Cover dynamic and child-template anchors at the start, middle and end.
- [ ] Confirm static text around an opaque anchor produces no partial ID.
- [ ] Confirm no link or inline-code grammar was duplicated.
- [ ] Confirm non-heading rendering allocates no label buffer.

## Validation

```bash
cargo fmt --all
cargo test --workspace --quiet markdown -- --format terse
cargo run --quiet -- tests --audit
just validate
```

# Phase 2 - Emit IDs and close source coverage

## Goal

Attach IDs to static `$md` headings while preserving heading content, escaping and opaque-anchor
behaviour through the full template pipeline.

## Work

- [ ] Emit `<hN id="slug">` when Phase 1 returns an ID.
- [ ] Preserve `<hN>` without `id` for dynamic or empty labels.
- [ ] Preserve current heading levels, inline output, closing tags and anchor order.
- [ ] Emit through the formatter output builder without unsanitised heading text.
- [ ] Add no builder, AST, TIR or HIR post-pass.
- [ ] Cover every supported heading level and exact static HTML output.
- [ ] Cover formatted labels, dynamic headings, child-template headings and empty labels.
- [ ] Cover equal labels producing the same unsuffixed ID.
- [ ] Confirm paragraphs, lists and non-heading inline output remain unchanged.
- [ ] Add or extend a `.moth` rendered-output case with a heading and same-page link.
- [ ] Add or extend an `.mtf` case proving implicit `$md` receives the same ID.
- [ ] Cover a site-root route fragment under a configured non-empty origin.
- [ ] Cover a dynamic heading through the complete template pipeline.
- [ ] Keep plain `.md` expected output unchanged.

## Audit and validation

- [ ] Confirm every ID originates in the `$md` formatter.
- [ ] Confirm no new formatter anchor, semantic side table or hidden counter exists.
- [ ] Confirm route fragments remain literal suffixes after site-root rendering.
- [ ] Confirm unrelated template output did not change.

```bash
cargo fmt --all
cargo test --workspace --quiet markdown -- --format terse
cargo run --quiet -- tests --audit
just validate
```

# Phase 3 - Document and adopt section links

## Goal

Make the behaviour canonical, use it in the Moth docs and verify generated pages have stable,
unambiguous section targets.

## Documentation updates

- [ ] In `docs/compiler-design-overview.md` under `Frontend stages > Stage 4: AST semantics >
  Templates and TIR`, add one ownership paragraph. State that `$md` derives IDs during formatting
  only from complete static visible labels, opaque labels omit IDs and builders never scan HTML to
  recreate them.
- [ ] In `docs/src/docs/templates/markdown-formatting-basic.mtf`, add the basic rule and one
  `@#section (Section)` example after the opening example.
- [ ] In `docs/src/docs/templates/markdown-formatting.mtf`, add `Automatic heading IDs` after
  `Supported block forms`. Define visible-label scope, normalisation, dynamic and empty omission,
  duplicate behaviour and fragment links.
- [ ] In `docs/src/docs/moth-templates/implicit-markdown-basic.mtf`, say under `What works` that
  static headings inherit automatic IDs from `$md`.
- [ ] In `docs/src/docs/moth-templates/implicit-markdown.mtf`, add a short inheritance note near
  `Supported Markdown` or `Links use Moth-aware syntax` and point to `Automatic heading IDs`.
- [ ] In `docs/src/docs/markdown/rendering-contract-basic.mtf` and
  `docs/src/docs/markdown/rendering-contract.mtf`, state concisely that plain `.md` uses a separate
  renderer and does not receive `$md` automatic IDs.
- [ ] Extend the existing `$md` paragraph in
  `docs/src/docs/cheatsheet/moth-language-cheatsheet.mtf` with generated kebab-case IDs and full
  `@#section (Section)` or `@/route#section (Section)` examples.
- [ ] Update the relevant `$md` and `.mtf` coverage text in
  `docs/src/docs/progress/@page.moth` after implementation is supported.
- [ ] Keep `docs/build-system-design.md` unchanged unless implementation exposes a real missing
  site-root or output-ownership contract.
- [ ] Keep README unchanged. The canonical template references and cheatsheet are the correct user
  surfaces for this focused feature.

## Documentation adoption and validation

- [ ] Change at least one existing cross-page docs link to target the generated
  `#automatic-heading-ids` heading rather than a manual HTML ID.
- [ ] Keep the link in ordinary `$md` source so the docs exercise the taught route and fragment
  syntax.
- [ ] Rebuild `docs/release/**` through the compiler.
- [ ] Follow the generated same-page and cross-page links in representative pages.
- [ ] Audit IDs in each generated HTML page for automatic duplicates and collisions with manual IDs.
- [ ] Resolve docs collisions by improving heading labels. Do not add a docs-only suffixer or
  suppression path.
- [ ] Confirm Basic and Advanced docs agree, `.mtf` describes inheritance and plain `.md` preserves
  its separate contract.

```bash
cargo fmt --all
cargo test --workspace --quiet markdown -- --format terse
cargo run --quiet -- tests --audit
just validate
moth build docs --release
```

- [ ] Run `git diff --check` and inspect the complete generated diff.

# Completion criteria

- every static `$md` heading emits the accepted deterministic ID
- `.mtf` headings inherit the same behaviour through implicit `$md`
- any opaque heading content omits the ID without a warning
- link labels and inline-code content contribute correctly
- empty labels omit the ID and duplicate labels remain unsuffixed
- same-page, relative and site-root route fragments reach generated IDs
- plain `.md` output remains unchanged
- no builder scan, document-global registry, new formatter anchor, TIR field or HIR field exists
- focused unit and integration coverage passes
- Basic, Advanced, cheatsheet, architecture and progress docs are updated
- generated docs are rebuilt and collision-audited
- all mandatory validation passes

# Closeout

In the final implementation commit:

- delete this plan
- remove its roadmap entry
- retain the delivered contract in permanent language and compiler documentation
- keep generated documentation in sync
- record validation and audit evidence in the commit or handoff rather than completion notes in the
  repository
