# Explicit Markdown section markers - implementation plan

> **Agent contract:** Implement this accepted design on the existing `feature/explicit-markdown-section-markers` PR branch. The initial PR contains only this plan. Keep it draft until implementation and validation are complete. Refresh from `main` at activation without importing unrelated `$export` work into this change.

## Goal and authority

Replace automatic `$md` heading IDs with explicit bare `@#id` markers. Assign the ID to the Markdown-owned element containing the marker or emit a standalone empty `span` when no such element exists. Existing labelled links stay unchanged.

This is unsequenced work running independently of the `$export` plan. The scope is the compiler's Moth `$md` formatter, which is also used by implicit `.mtf` content. The separate plain `.md` renderer is unchanged.

At activation, read `AGENTS.md`, `index.md` and the current versions of:
- `docs/compiler-design-overview.md`: authority, invariants, Stage 2 source-kind adapters and Stage 4 Templates and TIR
- `docs/build-system-design.md`: authority and invariants, then applicable HTML builder sections
- `docs/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`
- `moth-code-review-guide.md` where available in the project, applying its deletion/duplication audit
- `docs/docs/templates/markdown-formatting.mtf` and its Basic page
- `docs/docs/moth-templates/implicit-markdown.mtf` and its Basic page
- `docs/docs/markdown/rendering-contract.mtf`, the cheatsheet and both progress matrices
- Existing formatter, template diagnostics, unit tests and integration-case owners

This plan explicitly supersedes only the automatic heading-ID contract in `docs/compiler-design-overview.md`. Permanent language references describe final semantics, while the roadmap and progress matrices describe implementation status.

Principles: one explicit spelling, minimal grammar, one semantic owner, no compatibility shims, no alternative HTML/DOM interpretation, no new TIR/backend/build concepts. Prefer **removing existing state and branches** to layering the feature onto them. Preserve opaque child boundaries, source diagnostics and the existing structural string handoff. A plan phase is complete only after a mandatory independent correctness audit, style-guide review and targeted validation, with findings resolved.

## Accepted grammar and rendering

### Meaning and scope

Bare `@#id` is a section marker and produces no visible text. It attaches to the **nearest current Markdown-owned block**, using the existing block parser's context, never by inspecting rendered HTML:

- Heading `h1`-`h6`: apply to the heading at any inline position.
- Paragraph `p`: apply to the whole paragraph, including soft-wrapped lines.
- List item `li`: apply to the owning item, even when it contains several paragraphs or nested lists. An ID inside a nested item belongs to that nested `li`.
- No enclosing block: emit `<span id="id"></span>` at that position.

A block consisting solely of one marker is standalone when separated from surrounding paragraph text by a blank line or document/block boundary. It emits a span and no empty paragraph. A marker on a single soft-wrapped line immediately following paragraph text belongs to that paragraph. Inside a list continuation, it belongs to the list item rather than escaping into a standalone span.

Examples:

~~~moth
# Introduction @#intro

Welcome to Moth. @#welcome

@#examples

## Examples

- First @#first
  - Nested @#nested

See @#intro (Introduction).
~~~

Expected HTML structure:

~~~html
<h1 id="intro">Introduction</h1>
<p id="welcome">Welcome to Moth.</p>
<span id="examples"></span>
<h2>Examples</h2>
<ul><li id="first">First<ul><li id="nested">Nested</li></ul></li></ul>
<p>See <a href="#intro">Introduction</a>.</p>
~~~

Unmarked headings produce no ID. An explicit marker works even if the block contains dynamic expressions or opaque child templates. The formatter decorates only its own block, never a child-generated HTML tag or inline `em`, `strong`, `code` or `a` element.

### Reference precedence and boundaries

1. **Labelled links take precedence.** `@#intro (Introduction)` renders an ordinary `<a>`, not a marker. Keep the current spaces/tabs/at-most-one-soft-newline label separator, existing URL rules and existing nonempty-label rule. Link labels are opaque to marker recognition.
2. A bare `@#id` is eligible at the beginning of formatter-visible inline text or after Markdown whitespace. It requires a valid terminator: whitespace, end of its inline unit or a recognised closing inline-emphasis delimiter. A marker embedded in a word (`prefix@#id`) is ordinary text.
3. Only `@#...` gets the new interpretation. Bare `@https://...`, `@/path` and other unlabelled references retain their present literal/fallback behaviour. The feature must not turn them into element-wrapping links.
4. Inline code protects its contents. Markers inside code spans or inside a labelled link's label are literal content. An opaque child template is neither parsed for markers nor a place to attach an ID. Adjacent parent-authored markers remain eligible.
5. A marker is allowed at the start, middle or end of its enclosing Markdown block, not just as a heading suffix. Consuming it must not leave empty emphasis markup or add/remove unrelated visible content.

**Malformed labelled-link precedence:** if `@#id` is followed by a link-label introducer (horizontal whitespace or one soft newline then `(`) but that label is empty or unclosed, retain the *existing malformed-link fallback* for the whole candidate. Do not silently reinterpret its valid prefix as a marker. This is a compatibility rule for existing link authoring, not a new link syntax.

**Whitespace on removal:** treat marker syntax as zero-width content. Remove one redundant separator when its deletion would leave two normal word separators (for example `A @#x B` becomes `A B`), and remove the adjacent separator at a block edge (`# Title @#x` becomes `Title`). Otherwise preserve authored whitespace, soft-line behaviour and opaque anchors. Do this in the inline text emission path, not by trimming rendered HTML or globally normalising paragraph whitespace. A marker with only surrounding whitespace is still a standalone marker.

### ID spelling and conflicts

An ID begins with a Unicode alphabetic character and continues with Unicode alphanumeric characters, ASCII `_` or ASCII `-`. It is nonempty. Preserve authored case and Unicode spelling exactly. No whitespace, `#`, slash, quotes, angle brackets, ampersands, punctuation, URL escaping, lowercasing, slugification or suffix generation. This deliberately small alphabet keeps output safe inside a quoted HTML attribute. Reuse the existing escaped-output operation if convenient.

A marker-like reference with an empty or malformed ID (for example `@#`, `@#bad/name` or `@#bad!`) is a typed source error. Two markers targeting one Markdown block are a typed source error identifying the second and, when available, the first. Equal IDs on **different elements** are allowed: the compiler has no document-wide uniqueness registry and does not inspect raw HTML or separately composed templates. Authors must keep IDs unique within the final page.

No `{#id}` syntax, `id` template helper, Markdown attribute facility, automatic ID generation, whole-element URL linking, fragment-target validation or plain `.md` extension is authorised.

## Existing code and cleanup opportunities

These paths were inspected on `main` when the plan was revised. Verify their current shapes at activation rather than treating filenames as stable APIs.

1. `src/compiler_frontend/ast/templates/styles/markdown/mod.rs` owns formatter entry, line/opaque splitting and block dispatch.
2. `parsing.rs` contains heading/list parsing and `try_parse_link_at_atoms`, including existing target prefixes and label newline rules.
3. `inline.rs` currently owns `InlineRenderer`, inline emphasis/code/links, **heading label accumulation** and **lazy paragraph tag opening**.
4. `blocks.rs` already owns heading, paragraph/list grouping and nested-list structure. `render_inline_blocks` chooses when a paragraph wrapper exists.
5. `types.rs` contains small parsed records. `output.rs` owns escaped/raw/opaque piece buffering.
6. `heading_id.rs` owns the obsolete streaming heading normaliser. **Delete this file**, including `mod heading_id;` and every use, comment and test of its normalisation.
7. `src/compiler_frontend/ast/templates/formatter_contract.rs` and `tir/formatter_view.rs` adapt TIR text/opaque anchors into formatter input and output. `styles/whitespace.rs` performs `$md` pre-format dedent/trim. `styles/validation.rs` has source-span mapping used by HTML/CSS validators.
8. `src/compiler_frontend/compiler_messages/diagnostics/` owns typed diagnostic schemas, reason codes and rendering. `styles/markdown/` must not create a separate error presentation pipeline.

### Structural simplification to make as part of this feature

The existing `InlineRenderer` carries `wrapper_tag`, `wrapper_open` and `open_wrapper_immediately`, although only `blocks.rs` calls it. The block builder already knows whether each paragraph is wrapped, and already classifies standalone line-leading child templates. **Move paragraph opening/closing tags into `blocks.rs`** and remove that inline wrapper state, instead of adding an ID parameter to it. Preserve the child-anchor standalone and inline-continuation cases exactly. Keep a minimal distinction between ordinary paragraphs and standalone opaque child lines only where that distinction is semantically necessary.

After removing heading auto-ID state, **make ordinary heading content use the same inline renderer as other blocks**. Delete `RenderedHeadingContent` and `render_heading_content` if they have no independent responsibility. Inline rendering should return rendered pieces plus at most the parsed marker fact(s). The block renderer emits the owning HTML tag with its ID. A tiny local opening-tag helper is acceptable if it removes real duplicated `h`/`p`/`li` attribute emission. Do not add a generic HTML element/attribute framework.

For reference parsing, prefer **one shared target scan and one link/marker decision** over writing a second competing scanner that reparses `@#...`. The existing label parser must continue to own label boundaries. Keep per-block marker accumulation explicit: collecting from an item's own paragraph fragments must not collect IDs already rendered into its nested items.

### Diagnostic provenance: mandatory engineering gate

The current `tir/formatter_view.rs` runs the `$md` whitespace pass and calls `output_to_input(whitespace_output, representative_span, ...)`. This replaces individual authored text spans with one run-level span. `MarkdownInlineAtom::Char` retains no position. Therefore applying `SourceSpan::narrow_inline` to a transformed buffer as if its byte offsets matched the source would **misreport marker locations**.

Correct this using the **smallest lossless provenance path justified by the existing adapters**:
- Retain original text-piece source spans through preprocessing wherever they remain byte-exact.
- Where dedent/trim deletes bytes, map a marker's retained UTF-8 byte interval back to its original source interval. A sparse map of preserved contiguous text ranges is preferable to per-character `SourceSpan` records or scanning generated HTML.
- Keep provenance private to the formatter path unless another existing consumer genuinely needs the same mapping. Share an existing helper from `styles/validation.rs` only when its source-to-output mapping contract actually applies. Do not duplicate a subtly different offset algorithm.
- Preserve safe fallbacks for genuinely synthetic/missing-span content: a typed error may use the smallest **verified containing authored span**. Never fabricate an exact marker span from a representative span. For ordinary authored `.mtf` and `[$md: ...]` literals, precise marker locations after dedent are required.
- The map must preserve piece boundaries and opaque-anchor order, and must not change `SiteRoot` or TIR structural semantics.

Prove source locations in an AST/integration test with multiple text pieces, indentation removal, a non-ASCII character before the marker and an opaque insertion. Formatter-only tests with `span: None` cannot satisfy this gate.

**Diagnostics:** these are hard user errors, not CSS/HTML `MalformedTemplateReason` warnings. Add minimal typed Markdown-specific reasons through the existing family/schema, stable reason-key and rendering owners. Use one primary error for invalid/duplicate markers, with a related first-marker location for duplicates when available. Maintain existing user-error versus infrastructure/compiler-bug lanes.

## Implementation phases

Every phase ends in a **mandatory audit + Moth style-guide review + validation**. Do not advance with unresolved findings. Record the revision, changed ownership, deletions, test commands and results in the PR. Use `docs/developer-docs/style-guide/validation.mtf` to choose the smallest meaningful gate; intermediate slices need targeted coverage and important integration checkpoints use `just validate`. The final integrated tree requires `just validate-full`.

### Phase 0 - activation and baseline

- Refresh the implementation branch from `main`, identify concurrent `$export` touch points and record the starting SHA.
- Read the authorities and inspect the full renderer/whitespace/span/diagnostic chain, not just `markdown/`. Inventory all auto-ID production references and tests, existing `@#... (label)` link tests, and docs links currently relying on generated IDs.
- Choose the minimal provenance representation before implementation. Verify `SourceSpan` limitations and the exact point where whitespace preprocessing loses mapping. Record which existing formatter/test path proves the proposed fix.
- Identify the canonical `tests/cases/` owner and capture baseline tests/build failures.
- **Gate:** independent architecture/correctness audit of boundaries and identified risks, style/test guide review and baseline focused validation. Close findings before Phase 1.

### Phase 1 - consolidate block ownership, preserving existing behaviour

- Move ordinary `p` opening/closing tag emission into `blocks.rs`. Delete `InlineRenderer`'s lazy wrapper state and unused wrapper arguments. Preserve the existing paragraph, list-item, child-anchor and dynamic-anchor distinctions.
- Keep the current auto-heading-ID behaviour temporarily so this phase is a behaviour-preserving refactor, with no new marker semantics yet.
- Improve heading/list parser precision only where needed for this change. In particular `parse_heading_line` currently accepts more than six `#` markers even though the documented flavour supports only `h1`-`h6`. Correct this locally to prevent generating `h7` (treat longer runs as ordinary text) and add one focused regression test. Do not expand Markdown grammar.
- Run existing Markdown and template integration tests before touching marker parsing.
- **Gate:** audit for unnecessary helper layers, wrapper remnants and unrelated output changes. Review code style and tests. Run focused Rust unit/integration checks and `just fmt-check`, plus scoped Clippy.

### Phase 2 - reference recognition and reliable diagnostics

- Share the existing reference target/label scanning path. Give valid `@#id (label)` links precedence and retain literal fallback for incomplete labelled links.
- Add the bare `@#id` spelling, boundary and Unicode validation, with typed invalid/duplicate reasons and safe HTML attribute output. Bare non-fragment URLs are unchanged.
- Carry marker facts and real authored locations through formatter input and dedent as described above. Do not add a global parser, source rescanning pass, TIR variant or per-character heavyweight span record.
- Make the inline renderer consume markers as zero-width content without prematurely opening pending emphasis or leaving empty emphasis tags.
- Add focused parser and source-span tests. Include one label separated by a soft LF/CRLF/CR newline and malformed labels, one Unicode + dedent diagnostic and one marker adjacent to an opaque piece.
- **Gate:** audit parsing precedence, exact source offsets, time/space overhead and diagnostic family ownership. Review the style/testing guide, then run focused tests, formatter checks and scoped Clippy.

### Phase 3 - block IDs, standalone spans and complete legacy removal

- Thread one optional marker ID to the current block owner before it writes `h`, `p` or `li`. For list items, collect markers from item-owned content without merging facts from nested `li`s. Reject two markers on one block.
- Recognise isolated marker-only blocks and emit a bare span without an empty paragraph. Preserve ordinary soft-newline paragraph joining, list continuation and line-leading child-template handling.
- Delete `heading_id.rs` and all `HeadingIdBuilder`/heading-label accumulation, `mod` entry and now-redundant `RenderedHeadingContent` or wrapper code. Unmarked headings emit no ID. Explicit IDs work with opaque children.
- Update existing output expectations that depended on generated IDs. **Delete** slug-alphabet, Unicode-normalisation, case-folding and duplicate-heading-label tests rather than rewriting each as a new marker test. Preserve tests covering unrelated inline escaping, headings, code, links and child boundaries.
- **Gate:** search for dead auto-ID code and duplicate parsing, audit nested-list context/HTML correctness and style. Run targeted unit, TIR and integration tests plus `just validate` at this semantic integration boundary.

### Phase 4 - documentation and existing docs-site links

- Update the `$md` contract in `docs/compiler-design-overview.md` > Stage 4 > Templates and TIR. Preserve TIR/formatter ownership and delete the former automatic-heading-ID rule.
- Update `docs/docs/templates/markdown-formatting.mtf` and Basic, `docs/docs/moth-templates/implicit-markdown.mtf` and Basic, and the Moth Markdown section of `docs/docs/cheatsheet/moth-language-cheatsheet.mtf`. Use short heading/paragraph/list/span examples, unchanged labelled links and clear .md exclusion. Update crosslinks such as `#automatic-heading-ids`.
- Update `docs/docs/markdown/rendering-contract.mtf` where it contrasts .md with old `$md` heading IDs. Search `docs/developer-docs/` and the wider repo for other stale wording.
- Inventory **all site-local and cross-page fragment links in Moth Markdown docs** before editing targets. Supply explicit `@#id` on the intended `.mtf`/`$md` headings, preserving established fragment spellings when they matter. Do not parse or rewrite fenced/literal examples as active links, and do not inject Moth markers into plain `.md` content.
- Check emitted HTML for representative internal fragment routes and direct/relative path cases, rather than claiming source compilation proves link integrity. If the existing docs harness has a link check, use it; do not invent a permanent HTML crawler for this feature.
- **Gate:** docs/design/style audit, stale-reference search, documented `moth check docs --terse` and `moth build docs --release` (or Cargo equivalents), and evidence of matching emitted fragment IDs.

### Phase 5 - regression consolidation and merge readiness

- Consolidate tests under the current testing guide. Unit tests in `src/compiler_frontend/ast/templates/tests/markdown_tests.rs` own grammar, block scoping, escaping and opaque-boundary invariants. `src/compiler_frontend/ast/templates/tests/create_template_node/markdown_tests.rs` owns actual TIR/template folding, source spans and const/runtime composition. One or more existing `tests/cases/` scenarios own emitted HTML and the implicit `.mtf` path. Avoid reproducing the same large input at all three layers.
- Audit all old auto-ID tests: tests such as `static_headings_at_every_level_carry_kebab_case_ids`, `heading_ids_fold_case_and_collapse_separator_runs`, `equal_heading_labels_produce_equal_unsuffixed_ids` and `opaque_anchors_anywhere_in_a_heading_omit_the_id` become obsolete. Keep independent tests `inline_code_in_headings`, `heading_line_breaks_out_of_list_without_blank_line` and link/escaping cases with corrected output assertions.
- Prove the acceptance cases below and verify deterministic typed diagnostics after dedent/Unicode with real authored locations. Verify no malformed HTML or altered `SiteRoot`/resource pieces.
- Audit allocations and representation size. The final code should be simpler than the original where responsibility has moved; reject generic attribute models, global ID collections, duplicated target scanners and per-character span objects. Use targeted non-recording performance checks only if there is a plausible/measured hot-path regression.
- Update `docs/docs/progress/@entry.moth` **only to actual support/coverage evidence**, especially `Templates and style directives` and `.mtf` content assets. Remove its automatic-ID claims. Check the separate packages/builders matrix for contradictory wording, but do not add a new package feature.
- **Gate:** separate correctness + deletion/style/test-pruning audits, actual docs build and HTML checks, `just validate-full` on the final integrated tree, and PR finding/CI review. If validation is unavailable or fails, leave the PR draft and state the exact blocker. Rerun invalidated checks after fixes.

## Coverage checklist

**Block and identifier semantics**
- Explicit ID at start, middle and end of each supported heading level, with a distinct unmarked-heading check.
- Mid-paragraph and one-soft-newline paragraph markers, including `A @#x B` spacing. Marker-only blocks at document start/end, between blank lines, before/after headings and alongside standalone child-template lines.
- Simple, ordered, nested and continuation list items, including multiple paragraphs within one item. A parent ID must never migrate to a child item or inner `p`.
- Case-sensitive and Unicode IDs preserved, including non-ASCII before the marker for diagnostic byte mapping. Duplicate IDs on separate elements permitted, duplicates *on one* element diagnosed with locations.
- Plain/static headings, bold/emphasis, inline code, escaped HTML, dynamic expressions and opaque child templates preserve output. A marker inside only the visible portion of a compound block decorates the outer Markdown block. Marker text is absent from rendered content.
- Explicit `$md` and implicit `.mtf` agree. Separate `.md` remains unchanged.

**References and failures**
- Labelled `@#id (label)` stays an anchor, including one wrapped separator newline and mixed spaces/tabs; empty/unclosed labels retain existing fallback.
- Bare ordinary URL/path remains literal. `prefix@#id` and inline-code/label-contained `@#id` remain literal.
- Invalid `@#`, bad punctuation/space/terminator and two markers for one block produce typed diagnostics rather than silent output. Markers adjacent to opaque content neither read child output nor borrow child spans.
- No site-root/reference/resource regression, including URL origins and structural string pieces.
- Non-marked Markdown remains byte-equivalent except the deliberately removed automatic heading attributes and the documented h7 parser correction.

**Integration and documentation**
- At least one real emitted HTML page exercises an ID on `h`/`p`/`li` and a standalone span, with a fragment link resolving to each.
- Documentation pages with links into generated-heading fragments now explicitly author their targets. Source and emitted-link checks agree on representative routes.
- Test/fixture inventory has no redundant automatic-heading normalisation suite or obsolete source assertions.

For each new test, answer: **what credible regression does it catch that a neighbouring test would not?** Prefer extending an owner to adding another file. Do not create a new test framework, large golden-output collection or site-wide URL registry.

## Roadmap, deferred scope and completion

**Plan-only PR:** only this plan is committed. Do not change the roadmap or progress matrix before implementation.

**On actual implementation activation:** add one bullet under `## Ongoing in parallel` in `docs/roadmap/roadmap.md` (never under sequenced work):

~~~markdown
- [Explicit Markdown section markers](./plans/explicit-markdown-section-markers-plan.md) - **IN PROGRESS, PARALLEL**. Replace automatic $md heading IDs with explicit @#id markers for Markdown-owned blocks and standalone spans. Independent of $export.
~~~

Do not reorder or block the `$export` plan. On completion, remove this temporary parallel-work bullet rather than leaving it as a sequenced roadmap item.

**Deferred/excluded:** document cross-element uniqueness, generic HTML attributes, link wrapping, missing-target validation and plain `.md` syntax as intentional scope exclusions in the language reference, **not speculative roadmap tickets**. Everything in the accepted grammar, diagnostics, block types and docs migration is required for completion. For an independently approved follow-up genuinely deferred after implementation, record its owner, trigger, scope and why it is separate in the roadmap's appropriate unsequenced area. Mark `Deferred`/coverage limitations in the relevant progress-matrix row when they affect claimed support. Never mark a missing required case as completed or quietly reclassify it as a future feature.

**Retirement:** preserve every unique accepted semantic contract in permanent docs before following the repository's plan retirement policy. Keep this plan on the implementation branch until the agent finishes it.

The PR may request merge approval only when `heading_id.rs` and old automatic-ID behaviours are gone, the new syntax and diagnostics pass the coverage checklist, docs links and status are accurate, every phase audit is closed and the final validation gate has passed. Record the exact tested revision and command results.
