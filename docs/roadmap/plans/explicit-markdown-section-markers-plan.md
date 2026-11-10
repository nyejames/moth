# Explicit Markdown section markers - implementation plan

> **For the implementation agent:** This is an accepted design and an executable implementation plan. Implement on `feature/explicit-markdown-section-markers`, the branch containing this plan. Do not implement while preparing the plan-only PR. Refresh the branch from `main` when implementation begins, especially if concurrent `$export` work has landed. Keep the PR open until implementation, documentation and validation are complete. Do not merge the plan-only change as a substitute for implementation.

## 1. Goal and status

Replace automatic `$md` heading IDs with explicit `@#id` section markers. A bare marker assigns an HTML `id` to the Markdown-owned element currently containing it. A marker with no enclosing Markdown element emits an empty `<span id="..."></span>`. Existing labelled Markdown links retain their current behaviour.

This is an **independent parallel feature branch**. It is not in the ordered roadmap queue and must not block, reorder or borrow implementation scope from the separate `$export` design/implementation effort.

Current-state facts verified against `main` while writing this plan:
- `src/compiler_frontend/ast/templates/styles/markdown/heading_id.rs` implements automatic normalisation of static heading labels.
- `inline.rs` accumulates heading label characters, invalidates that accumulation on opaque content and renders links, code and emphasis.
- `blocks.rs` emits `id` attributes for headings, groups paragraphs and creates ordered/unordered/nested list items.
- `mod.rs` splits formatter text/opaque anchors into Markdown lines and selects block renderers.
- `parsing.rs` recognises `@target (label)`, including a single soft line break before the label.
- `formatter_contract.rs` carries formatter text with an optional source span and preserved opaque anchors. `MarkdownInlineAtom` currently carries characters without per-character provenance.
- `docs/compiler-design-overview.md` currently states that `$md` generates automatic heading IDs.
- `.mtf` files use the same `$md` formatter implicitly. Plain `.md` files use a separate renderer.

These paths are navigation, not permission to ignore newer refactors. Verify them against the implementation worktree in Phase 0. The actual file is named `heading_id.rs`, not `head_id.rs`. Delete it fully.

## 2. Authority, philosophy and boundaries

Read from the active worktree before code changes:
- `AGENTS.md`, `index.md` and the attached/project `moth-code-review-guide.md` when available
- `docs/compiler-design-overview.md`: opening authority, architectural invariants, Stage 2 source-kind adapters and Stage 4 Templates and TIR
- `docs/build-system-design.md`: opening authority and architectural invariants, plus only relevant HTML builder/documentation passages
- `docs/developer-docs/style-guide/style-guide.mtf`
- `docs/developer-docs/style-guide/testing.mtf`
- `docs/developer-docs/style-guide/validation.mtf`
- `docs/docs/templates/markdown-formatting.mtf` and `markdown-formatting-basic.mtf`
- `docs/docs/moth-templates/implicit-markdown.mtf` and `implicit-markdown-basic.mtf`
- `docs/docs/cheatsheet/moth-language-cheatsheet.mtf`
- `docs/docs/markdown/rendering-contract.mtf` and `docs/docs/progress/@entry.moth`
- Current formatter, tests, diagnostics constructors and relevant integration fixtures

This plan explicitly supersedes only the automatic `$md` heading-ID contract in the compiler overview and language documentation. The rest of the architecture remains authoritative.

Design rules:
1. Keep Moth Markdown small, explicit, composable and predictable. One spelling for one purpose. No inferred slug, silent fallback or syntax registry.
2. Reuse existing Markdown block parsing, inline parsing, formatter outputs and diagnostic transport. Add the smallest complete implementation rather than a general attribute system or DOM parser.
3. The Markdown formatter owns the new syntax and final HTML. No new TIR node, builder metadata, package helper, project setting or backend behaviour.
4. Maintain the opaque child-template boundary. The parent formatter may place an ID on its own Markdown element but must never inspect or attach an ID to HTML emitted by a child.
5. Keep source diagnostics typed, deterministic and located at the authored marker. Do not panic or silently discard malformed marker intent.
6. Delete the old implementation, tests, comments and obsolete documentation rather than preserving compatibility layers. Backward compatibility with automatically generated IDs is intentionally broken.
7. Follow the repository's test-admission rules: extend owners and consolidate superseded tests before creating more fixtures. A passing test must observe meaningful output or diagnostic behaviour.
8. Avoid performance regressions from repeated scanning or allocation-heavy intermediate HTML. The formatter should classify a block's ID before writing its opening tag without parsing generated HTML.

## 3. Accepted user-visible grammar and semantics

### 3.1 Forms

`@#identifier` is an explicit section marker when it appears as a bare, unlabelled reference in `$md`-formatted text. It emits no visible text. Its identifier is preserved exactly, including case.

`@#identifier (Label)` stays an ordinary labelled link producing `<a href="#identifier">Label</a>`. Keep the existing requirement for a nonempty label and the existing supported spaces/tabs/one-line-break separator contract. **Link recognition has precedence over marker recognition**, including when a label starts on the next soft Markdown line.

All other labelled links (`@./... (label)`, `@/... (label)`, `@https://... (label)`, etc.) remain unchanged. A bare non-fragment URL (`@https://...` or `@/path`) does not wrap a block and acquires no new meaning. No implicit link-body or whole-element wrapping feature is authorised.

### 3.2 Element scope

A marker attaches to the closest Markdown-owned *block* context currently being rendered, not to arbitrary generated HTML and not to inline emphasis/code/link elements:
- Markdown heading: `<h1>` through `<h6>`, regardless of where the marker appears among its inline text.
- Markdown paragraph: its `<p>` element, including multi-line soft-wrapped paragraphs.
- Markdown list item: the containing `<li>`, including continuation text, multiple rendered paragraphs and nested lists. A marker in a nested list item belongs to that nested `<li>`, not its parent. This rule deliberately avoids changing target identity when a list item's internal paragraphs become wrapped or unwrapped.
- With no Markdown block context: render exactly `<span id="identifier"></span>` at that source position.

A marker is standalone when it is the only meaningful content of an otherwise standalone Markdown paragraph block. Blank lines on both sides, or a document/structural block boundary, isolate such a marker. A single soft newline between ordinary paragraph text and a marker remains the same paragraph and therefore attaches to `<p>`. Do not invent a `<p>` around a standalone span.

The same grammar and semantics apply in an explicit `[$md: ...]` template and the implicit `$md` body of a `.mtf` file. The separate raw `.md` content renderer does not change.

### 3.3 Token boundaries and formatting contexts

A bare marker is recognised at a normal Moth Markdown reference boundary: at the start of inline/block text or after Markdown whitespace. Require whitespace or the end of its Markdown text unit after its identifier. This avoids treating fragments embedded in e-mail-like text, ordinary URLs or arbitrary punctuation as declarations. The canonical spelling for punctuation adjacent to a marker is to separate it with whitespace.

Markers may occur at the start, in the middle or at the end of a heading/paragraph/list item. They are not suffix-only metadata. Markers do not render as visible words and must not introduce spurious leading, trailing or doubled separator spaces where a consumed token was adjacent to words. Remove only spacing made redundant by consuming the marker, preserving unrelated authored whitespace and normal soft-line rules.

Do not interpret markers inside inline code spans, labelled-link labels or opaque child output. Maintain the existing rule that Markdown delimiters do not pair across child-template boundaries. A marker outside such an opaque child can still decorate the surrounding Markdown block.

### 3.4 Identifier contract

An explicit ID must begin with a Unicode alphabetic character. Subsequent characters may be Unicode letters/digits, ASCII `_` or ASCII `-`. It is nonempty, has no whitespace and contains no fragment `#`, slash, quote, angle bracket, ampersand or arbitrary punctuation. Preserve the exact source spelling and Unicode form. No lowercase transform, slugification, transliteration, URL decoding, percent encoding, document-order suffix or normalisation.

This narrow alphabet is safe to render inside a quoted HTML `id` attribute. The implementation should still use the existing HTML attribute-escaping writer or the same shared escaping operation where convenient rather than build another general attribute writer.

A marker-like spelling with missing or invalid identifier receives a typed source diagnostic with a precise marker location and actionable guidance. Do not silently render it as text. An incomplete or malformed *labelled-link attempt* such as `@#intro ()` or `@#intro (Unclosed` must continue following the existing malformed-link fallback rather than being reclassified as an ID declaration. Test this precedence deliberately.

### 3.5 Duplicates, responsibility and non-goals

Only one marker may target a particular Markdown element. Multiple markers for one heading, paragraph or list item are a compile-time diagnostic. Label the conflicting marker and refer to the first marker when source locations permit. Two separate Markdown elements may deliberately use the same literal ID: there is **no cross-element, cross-template or whole-page uniqueness registry** in this feature. Authors remain responsible for unique page-level IDs, including IDs introduced through raw HTML. Document that responsibility.

Other deliberate exclusions:
- No automatic heading IDs, even for entirely static headings.
- No `id` helper added to the HTML source package as a substitute for this syntax.
- No `{#id}`, `#id` or parenthesised attribute syntax.
- No bare-URL link wrapping, no link grammar changes and no empty-label links.
- No assignment of IDs to arbitrary HTML, inline `<em>`/`<strong>`/`<code>`/`<a>` or a child template's output.
- No link-target existence checking or generated-document HTML scanning.
- No changes to the plain `.md` renderer, resources, file graph, HIR, JavaScript/Wasm backend or HTML builder.
- No speculative grammar extension or extra warning system. Accepted deferred features, if separately approved later, must be tracked explicitly as described in Section 9.

### 3.6 Observable acceptance examples

~~~moth
# Introduction @#intro

Welcome to Moth. @#welcome

@#examples

## Examples

- First example @#first
  - Nested example @#nested

See @#intro (Introduction).
~~~

Expected equivalent HTML (normal whitespace formatting and site-root handling still apply):

~~~html
<h1 id="intro">Introduction</h1>
<p id="welcome">Welcome to Moth.</p>
<span id="examples"></span>
<h2>Examples</h2>
<ul><li id="first">First example<ul><li id="nested">Nested example</li></ul></li></ul>
<p>See <a href="#intro">Introduction</a>.</p>
~~~

The following should also work:
- `# @#name Dynamic [child] heading` assigns `name` to `h1` while preserving opaque child output.
- `A @#section B` renders one paragraph with ID `section` and visible text equivalent to `A B`.
- `A\n@#section` (one soft newline) assigns `section` to the paragraph rather than emitting a span.
- `@#solo` alone between blank lines emits only an empty span.
- `# Unmarked heading` renders `<h1>Unmarked heading</h1>` with no ID.
- `@#CamelCase` remains `CamelCase`, not `camelcase`.
- `@#café` remains a Unicode literal ID (no implicit transliteration).

## 4. Architecture and implementation map

### 4.1 Owning modules

Inspect and modify these existing owners instead of adding a parallel pipeline:
- `src/compiler_frontend/ast/templates/styles/markdown/mod.rs`: formatter entry, line splitting, block dispatch, module map and diagnostics handoff.
- `src/compiler_frontend/ast/templates/styles/markdown/parsing.rs`: reference recognition and lexically bounded ID parsing, integrated with existing link precedence.
- `src/compiler_frontend/ast/templates/styles/markdown/inline.rs`: inline semantics, marker consumption, spacing and returned marker information alongside opaque rendered pieces.
- `src/compiler_frontend/ast/templates/styles/markdown/blocks.rs`: heading/paragraph/list-item scope, conflict detection, emission of the ID on the correct opening tag and standalone spans.
- `src/compiler_frontend/ast/templates/styles/markdown/types.rs`: only the minimal stage-local record(s) actually needed.
- `src/compiler_frontend/ast/templates/styles/markdown/output.rs`: reuse existing buffered escaped HTML output.
- `src/compiler_frontend/ast/templates/formatter_contract.rs` and formatter/whitespace owners: inspect source-span preservation before extending any shared type.
- `src/compiler_frontend/ast/templates/error.rs` and owning diagnostic family: use existing typed `TemplateError::Diagnostic` or the appropriate diagnostic constructor.
- `src/compiler_frontend/ast/templates/tests/markdown_tests.rs` and `src/compiler_frontend/ast/templates/tests/create_template_node/markdown_tests.rs`: existing unit/AST-integrated owners.
- Existing `tests/cases/` Markdown and HTML compiler integration scenarios, chosen by actual ownership.

This is a frontend formatter feature. It does not require a new public parser API, new global state or any extra downstream handoff. If one small `section_marker.rs` helper is justified, it may contain only the local marker grammar and facts. Prefer keeping it in `parsing.rs` if that remains easier to read. Never keep `heading_id.rs` as a repurposed compatibility owner.

### 4.2 Data flow and source positions

The formatter must know a block's ID before emitting that block's opening tag. Parse the authored inline atoms into rendered pieces plus the optional marker fact, then let the existing block renderer add the attribute to its own opening tag. For list items, collect the marker across that item's authored continuation blocks but do not accidentally collect nested list children's markers. This is one Markdown structural interpretation; do not parse flattened rendered HTML, reopen an emitted tag or add a separate full-document label scan.

The existing `MarkdownInlineAtom` records `Char` or `Opaque` without per-character source offsets. `FormatterTextPiece` can carry an optional `SourceSpan` but the shared pre-format whitespace pass may transform text. Inspect that path carefully in Phase 0. Preserve sufficient marker-origin information through the actual transformation path to report a reliable exact span. Prefer a narrowly scoped offset/provenance record over broad source-text cloning or storing heavyweight spans on every character unless evidence justifies it. Use UTF-8 byte offsets correctly for Unicode marker text. Do not invent absolute source spans from transformed strings or broaden unrelated formatter contracts without evidence.

Preserve the existing opaque anchor order and the `SiteRoot` special link piece exactly. A marker changes HTML text emitted by `$md`, never resource/site-root identity or TIR node kinds.

### 4.3 Error policy

Use the existing diagnostic family if it has a suitable typed Markdown syntax reason. Otherwise add the smallest new reason and stable code through the normal registry, with source-label precision:
- Empty marker (`@#`).
- Invalid identifier/terminator (such as `@#bad/name` or `@#bad!`).
- Two markers targeting the same Markdown element.

The first marker should be a related label for a duplicate when available. Prefer one primary error per actual invalid occurrence, not a cascade. Preserve existing malformed-labelled-link fallback behaviour. Keep both `.mtf` and `[$md: ...]` source locations meaningful in diagnostics. An unexpected missing internal formatter invariant is a compiler bug, not a source diagnostic, but arbitrary malformed source never merits a panic.

## 5. Phased implementation

Each phase ends at a **mandatory audit, style-guide review and validation gate**. The agent must not begin the next phase while a substantive correctness or style finding from the current phase remains open. Review code against the current `moth-code-review-guide.md` and authoritative style/testing guides, document which checks ran and the exact revision, and fix findings before proceeding. If a phase changes only Markdown documentation, use the documentation-specific gate rather than claiming unrelated compiler tests passed.

### Phase 0 - activation, evidence and specification checkpoint

Tasks:
1. Refresh the independent implementation branch from current `main`. Record the baseline commit and concurrent `$export` touch points; do not take unrelated changes into scope.
2. Read the authorities from Section 2 and inspect all owners from Section 4, especially current whitespace preprocessing, formatter spans, diagnostic family constructors and list/paragraph child-template grouping.
3. Inventory automatic heading-ID tests and code, Markdown `@#...` link tests, docs-site fragment links and examples that currently rely on generated IDs. Separate genuine `$md` consumers from plain `.md` content.
4. Identify the smallest feasible span-provenance path and block-scoping approach. Write down any real mismatch with this accepted specification before touching implementation. Prefer adjusting internal representations to satisfy the contract, not weakening user-visible semantics.
5. Record baseline relevant test commands and existing failures. Locate the canonical integration-case owner before proposing new cases.
6. On implementation activation only, add the unsequenced parallel tracking entry described in Section 9. Do not insert this work into the sequenced roadmap list.

**Mandatory Phase 0 gate:** independent source/design review of the chosen owner boundaries, style-guide review of planned representations and a baseline targeted test/documentation check. Capture findings and dispositions in the branch/PR notes. Do not proceed without resolving real design contradictions.

### Phase 1 - bounded marker grammar and diagnostic plumbing

Tasks:
1. Keep the labelled-link parser as the first recogniser. Add the bare `@#id` recognition path only for candidates not consumed as valid labelled links and not protected by the existing malformed-link fallback rule.
2. Recognise marker boundaries, permitted Unicode identifier characters and invalid marker-like attempts without touching bare non-fragment URLs. Ensure recognition is not active inside code spans or an existing link label.
3. Retain the marker's source origin and precise token range through the formatter input path. Create the minimal typed diagnostics for invalid IDs and conflicts; decide their ownership at the existing formatter/template error boundary.
4. Return marker facts with rendered inline pieces through a small stage-local result. Keep opaque children and site-root links undisturbed. No public registry or TIR type.
5. Add focused grammar/provenance tests: bare marker, labelled fragment link, wrapped label over one newline, empty/malformed label, malformed marker, Unicode spelling and non-fragment bare URL.

**Mandatory Phase 1 gate:** audit parser precedence, source-span accuracy, code duplication and allocations using the code-review guide. Run focused Markdown parser/formatter and diagnostic tests, `just fmt-check` and relevant Clippy checks. Do not proceed on failing or unrun claimed coverage.

### Phase 2 - block scoping and HTML output

Tasks:
1. Carry a parsed marker to the existing heading renderer and emit `id` directly on `h1`-`h6`. Remove marker text without disturbing inline formatting.
2. Apply markers to paragraph opening tags, including soft-wrapped paragraphs and paragraphs containing opaque child/dynamic anchors.
3. Apply markers to the correct `li` in simple, continued, nested and multi-paragraph list items. Reuse the existing list builder rather than postprocessing HTML.
4. Emit a bare marker-only block as a standalone empty `span`. Preserve block boundaries and avoid an empty `p`. A normal paragraph with other meaningful text or opaque content is never misclassified as marker-only.
5. Diagnose multiple marker facts targeting one element. Keep distinct sibling elements independent and never add a global page-ID registry.
6. Preserve whitespace, emphasis, code, labelled links, escaped text, child-template positions and HTML structure when no marker is present.
7. Extend focused output tests and at least one real `.mtf` and one explicit `$md` compiler integration path, reusing nearby fixtures where possible.

**Mandatory Phase 2 gate:** audit parent/child scoping, nested lists, lazy paragraph wrappers, diagnostic location, opaque-anchor preservation and HTML correctness. Review against style/testing guides. Run affected unit tests, integration cases, formatter checks and appropriate routine validation at an important boundary (`just validate`). Resolve findings before proceeding.

### Phase 3 - fully remove automatic heading-ID machinery

Tasks:
1. Delete `src/compiler_frontend/ast/templates/styles/markdown/heading_id.rs`.
2. Delete `mod heading_id;`, every `HeadingIdBuilder` field/import/use and all visible-label accumulation (`push_label_char` and label-derived ID construction) from `inline.rs`.
3. Simplify `RenderedHeadingContent` and any associated wrapper or API shape; retain only data needed by the new marker path. Do not keep a renamed normaliser, shim or dead helper.
4. Update `blocks.rs` so unmarked headings never gain IDs, even with fully static text, links, emphasis or code. Explicit markers work even when headings contain opaque children.
5. Replace the obsolete automatic-ID unit tests with a compact set protecting the new explicit contract. Update remaining heading assertions throughout the test suite to either omit IDs or author markers intentionally. Retain independent coverage for escaping, link rendering and opaque content.
6. Search the full tree for obsolete `heading_id`, `HeadingIdBuilder`, automatic slug claims, ID-normalisation comments and tests relying on old output. Remove stale sources rather than keeping temporary compatibility.

**Mandatory Phase 3 gate:** audit that old logic is truly deleted and no duplicate parsing/normalisation path survives. Apply the code-review/style guide to every touched formatter module. Run targeted unit/integration tests, relevant static source searches and `just validate` if this is an integration checkpoint.

### Phase 4 - docs-site migration and permanent design authorities

Tasks:
1. Replace the automatic-ID paragraph in `docs/compiler-design-overview.md` > Stage 4 > Templates and TIR with the explicit `@#id` formatter-owned contract. State its bounded block scopes, opaque-child exclusion and no extra TIR/backend facts. This is a final-design authority update, not implementation-status prose.
2. Rewrite `docs/docs/templates/markdown-formatting.mtf`, `markdown-formatting-basic.mtf`, `docs/docs/moth-templates/implicit-markdown.mtf` and `implicit-markdown-basic.mtf`. Show short examples for heading, paragraph, list item, standalone span and unchanged labelled links. Replace the old `#automatic-heading-ids` docs heading and inbound links with a stable explicit-marker documentation target.
3. Update the Moth Markdown section in `docs/docs/cheatsheet/moth-language-cheatsheet.mtf` with an explicit example, removing the static-heading kebab-case claim. Keep the cheatsheet compact and final-design-only.
4. Adjust `docs/docs/markdown/rendering-contract.mtf` and its Basic page only where they describe old `$md` behaviour. Reiterate that raw `.md` has a different renderer, without promising marker support there.
5. Audit the real docs site's `.mtf` and explicit `$md` pages for incoming fragment links that previously depended on generated heading IDs. Add explicit markers to the intended target headings (or deliberately selected standalone targets), preserving stable existing fragments where possible. Handle duplicate heading labels intentionally. Do not blindly rewrite links, code-example text or raw `.md` source.
6. Search other current documentation, code comments, package guides and tests for stale promises of implicit IDs. Fix accurate references, including `index.md` if its formatter navigation names `heading_id.rs`.
7. Inspect the generated docs HTML for representative navigation and links. A passing source compilation alone does not prove that `#fragment` links have matching targets in emitted pages.

**Mandatory Phase 4 gate:** documentation architecture/style review, check that no stale automatic-heading ID promise remains and validate docs compilation and a release docs build according to `validation.mtf`. Inspect actual emitted IDs and fragment targets. If a broken link is discovered, fix it rather than calling documentation migration deferred.

### Phase 5 - regression hardening, matrix and full release gate

Tasks:
1. Run a final audit of success and failure paths across the accepted matrix in Section 6. Ensure test additions are necessary under the testing guide. Consolidate redundant legacy cases and avoid fixture proliferation.
2. Verify diagnostic exactness across implicit `.mtf` and explicit `$md` source, including multiline input and Unicode byte positions. Check determinism across repeated compilation where the harness supports it.
3. Inspect output structure in at least one end-to-end HTML project build. Confirm IDs attach to actual heading/paragraph/list-item tags, standalone markers emit spans and links still use `<a>`.
4. Review formatter allocations and hot-path work against the baseline. This is a small parser feature; reject generic DOM traversal, global ID maps, extra full-document passes and avoidable per-character heap records. Run non-recording performance checks only if profiling or comparative evidence suggests a material regression.
5. Update the compiler progress matrix row `Templates and style directives` and the `.mtf` content row in `docs/docs/progress/@entry.moth` to reflect the **actual** support and test coverage. Remove claims of automatic IDs. Verify the separate packages-and-builders matrix only if a claim there is affected. Do not mark this supported before the tests pass.
6. Follow the roadmap and deferral policy in Section 9. Ensure permanent authority and documentation have all unique accepted contracts before any plan retirement.
7. Run the final code-bearing gate `just validate-full` on the final integrated tree and record toolchain/environment, revision and results. Re-run on a new final tree after any gate-invalidating changes. Review CI status and outstanding PR findings. Keep the PR open if this gate is unavailable or failing; report the exact evidence and blocker.

**Mandatory Phase 5 gate:** a separate final correctness and Moth code-style audit of the whole change, testing/fixture audit, docs link audit, complete merge-readiness validation and explicit closure of all findings. No unverified success claims.

## 6. Mandatory acceptance and regression matrix

Prefer extending existing tests in `src/compiler_frontend/ast/templates/tests/markdown_tests.rs`, `src/compiler_frontend/ast/templates/tests/create_template_node/markdown_tests.rs` and canonical `tests/cases/` owners. This is a risk checklist, not a mandate for one test per row.

### Success behaviour
- Every heading level can receive an explicit ID. An unmarked heading receives no ID.
- Heading marker at start, middle and end, alongside emphasis, code and labelled links.
- Paragraph marker at start, middle, end and after one soft newline. It stays on the same `p`.
- Marker-only block at start/end of document and between blank-line-separated blocks produces a standalone span, never an empty paragraph.
- Unordered/ordered list items, nested items, continuation lines and multi-paragraph items receive IDs on the correct `li`.
- Unicode identifier, mixed case and hyphen/underscore are retained exactly.
- Labelled fragment link is still a link and resolves in the same document. A wrapped link label over one newline keeps working.
- Opaque child and dynamic anchors remain in place. IDs apply only to surrounding Markdown-owned blocks, never child-generated HTML.
- Explicit `$md` and implicit `.mtf` paths produce equivalent content; no change to plain `.md`.
- Ordinary unmarked Markdown, escaping, inline code, emphasis, link targets and site-root pieces match pre-change output apart from removed automatic heading IDs.
- No duplicate marker-generated text, spurious paragraph wrappers, stray spaces or invalid nested HTML.

### Failure and non-regression behaviour
- Bare `@#` and malformed identifier forms produce the intended typed diagnostics with useful positions.
- Two markers on one element diagnose deterministically, with first and second authored locations where available.
- Separate elements with equal IDs remain permitted without a whole-document registry.
- `@#id (Label)` remains a link even when a soft newline separates the target and label.
- Existing malformed labelled-link candidates, especially empty or unclosed labels, do not accidentally become IDs.
- `@https://example.com` and `@/docs` do not become block-wrapping links.
- An embedded `email@#id`, inline code ``@#id`` and labelled-link text do not declare a section.
- A marker beside opaque content never triggers inspection of that content.
- Whitespace, blank-line and child-template line boundaries remain correct.
- Old normalisation paths no longer exist in production or tests, including the `heading_id.rs` file.

### Artifact and docs verification
- Generated HTML contains `id` on the intended element rather than on a synthetic span when there is a block context.
- Standalone span contains no visible content or surprise paragraph.
- Representative existing docs-site `#fragment` references still target an explicit ID after migration.
- Compiler docs, basic docs, cheatsheet, progress matrix and architectural authority agree about the final behaviour.

## 7. Implementation guidance and anti-bloat review

A minimal solution should keep the formatter's data flow legible:

~~~text
formatter text/opaque input
-> current Markdown line and block parsing
-> existing inline parsing with labelled-link precedence
-> optional explicit marker fact + rendered pieces
-> current Markdown block owns opening tag/id
-> output buffer preserves text and opaque anchors
~~~

A good implementation need not use the exact shape of this sketch, but must preserve those ownership boundaries.

On every phase audit, ask:
- Can existing parsing/output helpers do the work without parallel scanners or a new registry?
- Did we delete the automatic-label collection rather than just disable it?
- Is there one authoritative ID fact per Markdown block, with source provenance only as long as needed?
- Did list-item handling accidentally collect IDs from nested child items?
- Are malformed inputs diagnosed, not panics, silent drop, or misleading literal fallback?
- Is a new struct/module providing an actual invariant or merely moving a few lines?
- Are all comments accurate and do they explain WHY instead of narrating syntax?
- Can any new clones, temporary strings, state or tests be removed while preserving distinct coverage?

Record findings as `remove`, `merge`, `move`, `split` or `leave local` and explain the latter when superficially duplicated code is intentionally separate. Review the code with fresh eyes after fixes.

## 8. Validation and agent execution contract

- Keep the active branch isolated. All implementation commits go to `feature/explicit-markdown-section-markers` and update this PR. Do not branch again for the implementation. Never push code changes to `main`.
- Use sequential, reviewable phase commits, with a short phase completion report: files changed, accepted behaviour, deleted code, tests/commands actually run, audit findings and remaining issues.
- The mandatory end-of-phase gate is **correctness audit + style-guide review + meaningful targeted validation**. A wider routine gate is required at important cross-module boundaries, not on every tiny iteration. Follow `docs/developer-docs/style-guide/validation.mtf` for the current commands and environment.
- Run `just fmt-check`, the relevant Clippy/test filters and selected integration cases during coding. Use `just validate` at integration milestones and `just validate-full` for final merge readiness. Use `moth check docs --terse` and `moth build docs --release`, or their documented Cargo equivalents, when validating docs changes.
- Validate on the repository's pinned Rust toolchain and required Node runtime. Do not claim the default `cargo test` covers the compiler suite when the CLI default member selection would miss it.
- Preserve deterministic diagnostics and test results. Do not disable unrelated tests or edit baselines to conceal a failure.
- No asynchronous promise, partial feature merge or passing-status claim without actual execution evidence.
- If an audit discovers a genuine accepted-design contradiction, document the evidence and propose the smallest correction in the PR. Do not silently choose a new language syntax or relax the accepted rules.

## 9. Roadmap, progress matrix, deferrals and closeout

### Tracking this independent implementation

**Plan-only PR:** add just this Markdown plan on the new branch. Do not edit `docs/roadmap/roadmap.md` or the progress matrices yet. A document being written is not an implemented language feature.

**When implementation actually starts:** add a single bullet under `## Ongoing in parallel` of `docs/roadmap/roadmap.md`, not under `## Sequenced work`:

~~~markdown
- [Explicit Markdown section markers](./plans/explicit-markdown-section-markers-plan.md) - **IN PROGRESS, PARALLEL**. Replace automatic $md heading IDs with explicit @#id markers for Markdown-owned elements and standalone spans, with docs-site migration. Independent of the sequenced $export work.
~~~

Do not reorder the current queue, mark `$export` blocked or imply this feature has dependencies on `$export`.

**At actual implementation completion:** update the two relevant rows of `docs/docs/progress/@entry.moth` to `Supported` only after passing implementation/coverage gates. Describe actual explicit-marker coverage, retained labelled links, plain `.md` exclusion and removed automatic IDs. Correct the documentation and compiler authority in Phase 4 regardless of status claims. Check the package/builder matrix for conflicting prose but do not add a new package feature row for frontend-owned syntax.

**Deferred work:** all capabilities in Sections 3 and 6 are required. Do not defer source-span diagnostics, standalone markers, list items, docs link migration or removal of the old heading-ID path merely to report completion. The following are explicit *non-goals*, not implied promises of future work: cross-element ID uniqueness, general HTML attribute syntax, URL wrapping and `.md` support. Record them in the language docs as boundaries, **not** as speculative roadmap items. If a truly separate, accepted follow-up emerges and cannot land, record its exact scope, reason, concrete trigger, affected owner and implementation status in the roadmap's appropriate unsequenced/ongoing tracking area or an already accepted companion plan, and reflect `Deferred`/`Targeted` in the relevant progress-matrix coverage notes. Do not add a future feature without design approval.

**On merge/retirement:** remove the temporary `Ongoing in parallel` bullet rather than leaving a completed task in the ordered roadmap. Preserve the accepted syntax and ownership contracts in permanent documentation before retiring or deleting this plan according to the current roadmap plan-retirement policy. Do not remove this plan while agents still depend on it to finish the implementation branch.

## 10. Definition of done

This work is complete only when all of the following are true:

- `@#id` works in explicit `$md` and implicit `.mtf` with heading, paragraph, list-item and standalone-span contexts as specified.
- Labelled links, other bare references and plain `.md` semantics are unchanged.
- Invalid or duplicate per-element markers have deterministic typed diagnostics with useful authored locations.
- `heading_id.rs` is deleted and the old automatic ID feature has no surviving code, tests or documentation claims.
- The docs site demonstrates the new form and its migrated fragment links work in actual generated HTML.
- The compiler overview, language docs, cheatsheet and progress matrix encode the final design and actual support correctly.
- Each phase has a recorded correctness audit, code-style review and validation result.
- The final integrated tree passes the appropriate full validation gate, or the PR explicitly remains unfinished with reported blockers.
- The PR contains implementation commits on this original branch and is not merged prematurely.

Only then request merge approval.
