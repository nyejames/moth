# Builder slots and template directive redesign - implementation plan

Status: Active on `builder-slots`
Current slice: Phase 5 cleanup, documentation and performance
Blockers: None identified
Next action: Complete documentation, performance checks and the final validation gate.
Scope: Core template syntax and composition, builder-defined top-level const slots, HTML metadata and stylesheet assembly

## Purpose

Make builder-defined top-level slots an extension of ordinary Moth template composition. Replace string-named slot helpers with symbol-named declarations and terse inserts. Add compile-time defaults per slot occurrence. Replace `$children(...)` with `$before = ...` and `$after = ...` while preserving reusable constant-template wrappers, named-slot composition and existing const/runtime child semantics. Migrate the HTML builder from reserved `page_*` constants to named slots and add an origin-deduplicated `stylesheets` slot.

Deliver the smallest complete implementation. Preserve current source ordering, TIR authority, structural resource identity, source provenance, compiler/build ownership and JS/Wasm output parity. Delete superseded paths instead of retaining compatibility layers.

This plan supersedes the `$stylesheet` resource-requirement proposal for this slice. It does not implement automatic dependency propagation through arbitrary component Strings. A component's stylesheet is explicitly contributed to a builder slot at the entry root. The separate `$css` inline-only redesign is not part of this slice.

## 0. Required reading and baseline

Read `AGENTS.md`, the repository style guide and cheatsheet, the relevant unsuffixed template and directive references, the compiler and build-system design overviews and the testing and validation guides. Prioritise approved decisions in this plan where older documentation conflicts. Do not treat stale `$page` documentation as a reason to implement `$page`: the updated direction is a general `$entry` directive with builder-specific metadata supplied separately through slots.

Inspect at least these current code owners before editing:

- `src/compiler_frontend/style_directives/builtins.rs`, `specs.rs` and `compatibility.rs`
- `src/compiler_frontend/tokenizer/lexer.rs`, template-head token recognition and matching token tests
- `src/compiler_frontend/ast/templates/template_head_parser/{head_parser,core_directives,directive_args,children_directive}.rs`
- `src/compiler_frontend/ast/templates/{template,template_build_state,template_body_parser,create_template_node}.rs`
- `src/compiler_frontend/ast/templates/tir/{slot_layout,wrapper_sets,overlays,preparation,public_const_rebuild,fold/wrappers}.rs`
- `src/compiler_frontend/ast/templates/tir/slot_composition/{contributions,head_chain,child_wrappers}.rs`
- `src/compiler_frontend/ast/templates/template_slots/runtime_plan/{sources,sites}.rs` and `tir/handoff_materialization.rs`
- `src/compiler_frontend/headers/{hash_items,types,const_fragments}.rs`
- `src/compiler_frontend/ast/templates/top_level_templates.rs`, `ast/module_ast/emission/emitter.rs`
- `src/compiler_frontend/module_compilation/{artefact,service}.rs`
- `src/compiler_frontend/folded_value.rs` and `src/compiler_frontend/paths/{module_resources,resource_identity}.rs`
- `src/projects/html_project/{html_project_builder,compile_input,page_metadata,document_shell,js_path,resource_output_plan,structural_url_renderer}.rs`
- `src/projects/html_project/wasm/artifacts.rs`

The filenames are investigated locators as of 5 November 2026. Reconcile them with the current branch rather than restoring any earlier API shape.

### Mandatory first action: install baseline regression tests before refactoring

**Do not start the syntax or wrapper refactor until an end-to-end pre-refactor test establishes the existing named-slot composition behaviour.** Prefer extending an existing `tests/cases/` owner if it proves the contract distinctly. Add a separate focused case only if necessary.

The baseline must exercise the actual old `$children(...)` form with a wrapper containing both `[$slot("row_style")]` and `[$slot]`. Supply a named `row_style` contribution through an enclosing template application, render multiple children and assert the full output rather than only successful compilation. Make the selected value visible in the opening `<tr>` attribute, not merely elsewhere in the output. Test import or stored-const-template use if the established primary owner does not already cover that boundary.

Investigate and record the current result, with an exact expected output and an explicit explanation of which composition scope owns `row_style`. Existing code in `packages/html/@mod.moth` contains a related pattern, but its presence is not proof that the name is correctly routed. **If the baseline fails, preserve the failing test, investigate the existing behaviour and repair the intended named-slot composition contract before proceeding.** Do not weaken the assertion, silently change the intended output or assume that a named slot within a wrapper is necessarily supplied by the outer application.

Also establish a compact baseline for `$fresh`, nested children, conditional no-output and runtime loop wrappers. Existing candidates include:

- `tests/cases/template_runtime_repeated_slot_different_wrappers/`
- `tests/cases/template_runtime_slot_site_fresh/`
- `tests/cases/template_runtime_loop_children_wrapper_regression/`
- `tests/cases/template_runtime_wrapper_slot_inside_branch/`
- `tests/cases/template_control_flow_children_wrapper_skipped_when_no_output/`

Capture current targeted test results and a clean performance baseline before code changes. Preserve the meaningful baseline assertions after migrating their source syntax. This is a **regression gate**, not a one-time exploratory test to delete after the rewrite.

Baseline at `919ec4e50`: the enclosing application of the imported `rows` template owns
`row_style`. The original-syntax `template_children_named_slot_wrapper_application` case
failed with `MOTH-RULE-0052` at `@page.moth:7:5`; it must render exactly
`<tr style="background: red;">first</tr>\n<tr style="background: red;">second</tr>`.
The five existing wrapper cases listed above each passed before compiler changes.
The named-slot test passes after correction, as do fixed-child and nested runtime cases.

Non-recording baseline on unchanged compiler sources: `just bench-frontend-check`
(76 cases, 10 iterations) and `just bench-check` (74 cases, 10 iterations) passed.
The worktree was dirty with the admitted regression and documentation changes.
Mean milliseconds: frontend template stress 23.300, frontend render-plan churn 21.243,
CLI template stress 9.966, CLI render-plan churn 9.186, external-JS imports build 11.500,
module-root stress build 12.004 and external-import churn build 15.809.

## 1. Accepted language surface

The following examples describe the intended final syntax. Remove old spellings throughout compiler code, tests, first-party packages and docs. There are no aliases or transition modes.

```moth
-- Slot declaration and per-occurrence default
[$slot]
[$slot title]
[$slot title = "Untitled"]
[$slot 1]
[$slot 1 = "Fallback"]

-- Named insertion
[$in title: My Page]
```

Named slot identifiers are statically recognised symbols, not String expressions or ordinary lexical value bindings. Their exact namespace is the immediate receiving template's declared slot schema. Positional slots retain the current positive-index rules and loose-head routing. The unnamed slot remains the default receiver of otherwise unassigned content.

Examples:

```moth
card #= [:
    <h1>[$slot title = "Untitled"]</h1>
    <section>[$slot]</section>
]

[card:
    [$in title: Welcome]
    Card body
]
```

### Slot composition and defaults

- Named inserts still target only their named slot and preserve source-order concatenation.
- Loose contributions still fill positional slots in ascending index order, then the default slot.
- A slot with no routed contribution uses its own constant default when present. Otherwise it renders empty.
- Each slot **occurrence** owns its own default, even if another slot occurrence has the same key. Repeated named occurrences consume the same routed contributions but may independently fall back to different values.
- A routed contribution, even an explicitly empty one, suppresses that occurrence's default. Determine presence from routing, not from nonempty output. Do not add a runtime `is_empty()` fallback or require a dynamic branch to produce characters.
- Multiple contributions to any named slot concatenate. In particular, builder metadata fields do not gain scalar cardinality errors.
- Defaults must use Moth's existing compile-time `String` value rules, including structural pieces where supported by ordinary constant String defaults. Do not create a second arbitrary expression evaluator. Diagnose nonconstant defaults and type errors at the authored declaration.
- Unknown insert targets remain diagnosed at the applicable receiver. Unresolved insert helpers cannot become ordinary backend String values.
- Stored insert helpers, cross-module const-template projection, wrapper contexts and runtime slot applications retain their established semantics.

Do not conflate key identity with occurrence identity. The current `TirSlotSchema` indexes keys while `TirSlotPlaceholderRef` carries occurrences. Defaults belong to the latter and must survive any public folded-template projection and rebuild.

### Future constraint syntax, not implemented

The agreed direction is a same-head modifier on an individual slot occurrence:

```moth
[$slot title, $required]
[$slot styles, $unique]
```

Neither `$required` nor `$unique` is implemented here. Reserve no speculative runtime machinery for them. Roadmap notes should mark `$required` as a much later design candidate whose value and exact behaviour must be reconsidered. `$unique` is also deferred. The idea of `$unique` on loops is excluded.

## 2. Replace `$children(...)` with `$before` and `$after`

### Syntax and value contract

```moth
[
    $before = [$html:
        <tr style="[$slot row_style]">
    ],
    $after = [$html:
        </tr>
    ]:
    [$slot]
]
```

Each directive is optional, and either may be used on its own. Each may occur once per template head. Both accept compile-time `String` values **or constant template wrapper values containing unresolved named slots**. They reject runtime-dependent expressions and values that are neither valid constant Strings nor valid constant templates.

A template wrapper can be compile-time known without yet being a concrete String. Accept the compiler's existing `TemplateConstValueKind::WrapperTemplate` classification in addition to `RenderableString`. Preserve its TIR reference, named slots, structural resource pieces and public-interface identity until composition.

**Only named slots may appear anywhere in a `$before` or `$after` value.** An explicit `[$slot]` or `[$slot 1]` within the value, including a nested template that semantically belongs to the same wrapper, must yield a useful diagnostic at the offending slot. This restriction leaves exactly one implicit default child insertion point and avoids accidental child duplication. If a named slot is present, normal slot routing must resolve it through the intended enclosing composition, as proven by the baseline regression.

Do not add a new special rule to evaluate before/after template expressions once per runtime child. The wrapper value is a compile-time composition pattern. Existing runtime child emission should remain responsible for when the resulting output is produced.

### Semantic model

Normalise the two optional values to a single equivalent wrapper pattern:

```text
before template or text
implicit default slot: current direct child
 after template or text
```

Use `TemplateIrNodeKind::Sequence` with references to the original prefix and suffix templates and a compiler-created default-slot placeholder. Build and validate it **once per owning template**, not once per direct child or runtime iteration. Reuse the established `TemplateWrapperReference` and wrapper-context overlay path. Avoid copying each prefix and suffix TIR subtree merely to compose the wrapper. Preserve phase and exact-view context in references.

Investigate whether the existing slot-layout and head-chain collectors already traverse referenced prefix and suffix templates correctly. `collect_tir_slot_layout()` currently follows child-template references and its schema must contain the implicit default slot and every intended named slot. The new wrapper must not accidentally treat prefix/suffix named placeholders as a separate receiving interface that the caller cannot address. Add a focused TIR test that proves both layout and successful resolution, then rely on the baseline integration test for user-visible correctness.

Keep these current `$children` contracts:

- Only direct child templates receive the wrapper. Literal body text is not individually wrapped.
- Nested descendants are owned by their own immediate parents.
- `$fresh` skips the immediate parent wrapper and does not disable nested wrappers.
- A false condition, absent option or zero-iteration loop producing structural no-output does not emit prefix or suffix.
- An explicitly selected empty String still counts as output and is wrapped.
- A direct child that is a loop is wrapped around its aggregate output once. Per-iteration wrappers must be written inside the loop body.
- Wrapper order and repeated slot replay remain deterministic.
- Const folding, runtime lowering, imports, re-exports and retained structural resource pieces have matching observable behaviour.

Check `TirWrapperApplicationMode::Always` versus `IfChildEmits`, aggregate wrappers, runtime slot-site construction and folded wrapper insertion explicitly. Do not replace proven no-output logic with a simpler String-only concatenation that loses these distinctions.

### Specific parser and ownership work

- Extend template-head directive parsing to accept `$before = <constant expression>` and `$after = <constant expression>`. Reuse a small assignment-style directive-expression parser where the grammar genuinely overlaps with other upcoming directive work, but do not undertake the wider directives redesign in this branch.
- Handle comma-separated heads and nested bracket-balanced template arguments without breaking the existing token cursor's separator ownership. In particular, the current `$children` path expects a parenthesised call argument and advances differently from an assigned value. Preserve correct body-colon detection and precise EOF diagnostics.
- In the core directive registry, replace `Children` with `Before` and `After`. Continue using existing head-compatibility checks for duplicate directives and formatter conflicts. Keep `$fresh`.
- Validate constant template kinds using existing TIR preparation. Do not flatten `WrapperTemplate` to a concrete String before its slots are filled.
- Extend the existing child-wrapper normalisation to build the new composite wrapper. Reuse current wrapper sets, TIR overlays, folding and runtime handoff. Change only what the new syntax requires.
- Generalise the old `$children` string-only normalisation where necessary to preserve structural `ConstStringValue::Pieces`. A `Resource` or `SiteRoot` piece is never a literal URL during frontend composition.
- After tests pass, delete the old `children_directive.rs` public parsing contract, obsolete diagnostics and unused wrapper-specific adapters. Keep general wrapper logic that is still used, even if its previous comments mention `$children`.

### Proving the migration

Run the old baseline regression **before** modifications, then translate its input to `$before`/`$after` while retaining the same observable output assertions. Verify that named `row_style` routing still fills the opening tag. Add a separate negative test for an explicit default slot inside either assigned template and one for positional slots. Reconfirm `$fresh`, no-output and runtime loop behaviour on both JS and the supported Wasm path.

## 3. Extend active-root top-level const fragments to named builder destinations

### Surface and scope

```moth
#[$in title: Moth Documentation]
#[$in description: Documentation for Moth]
#[$in stylesheets: [@styles/docs.css]]
#[$in head: <meta name="theme-color" content="#ffffff">]
#[: <main>Hello Moth</main>]
```

A builder registers statically known named slots. A **direct** top-level const `$in` helper on the selected active root contributes to one registered destination. An ordinary top-level const fragment still goes to the default body destination. Named inserts nested inside an ordinary fragment or passed through aliases remain ordinary template-composition helpers, not implicit builder messages. This is a deliberate shallow boundary.

Runtime top-level templates keep their current body output and start interleaving. Runtime named builder slots are out of scope and should be recorded as unsettled final design in the roadmap, not implemented or promised as a supported syntax.

Preserve current root policy: active application entry roots accept top-level const fragments. Ordinary helper `.moth` files do not. Imported module roots do not execute or contribute entry fragments. API-only roots and package facades do not gain a page-side-effect path. No global registry of contributions or hidden recursive dependency propagation through ordinary folded Strings.

### Compiler/build interface

Prefer a **narrow root-fragment routing extension**, not a synthetic parent TIR for the whole builder entry. Reuse current `TemplateType::SlotInsert`, `SlotKey`, TIR slot routing principles and constant-validation machinery. The current top-level AST fold rejects `TemplatePreparationOutcome::SlotInsertHelper` as `HelperInConstTemplate`. Recognise and extract a **direct root insert** before that ordinary String fold boundary rather than relaxing the rejection of escaped helpers globally.

The fragment handoff currently carries `runtime_insertion_index`, `span` and `OwnedFoldedString`. Extend this existing ordered record with only the destination identity needed by the builder, or an equivalent compact typed classification proven cheaper. Preserve the existing default-body fast path. Keep the source order of fragments with the same runtime insertion index instead of sorting them by slot name. Names must be stable across module compilation rather than donor-local `StringId`s without an owner.

The frontend accepts a builder-supplied read-only slot schema through an existing compiler-owned input or builder-surface contract. The HTML builder supplies names and later interprets their content, while `compiler_frontend` stays independent of `projects/html_project`, `build_system` config types and arbitrary builder callbacks. A minimal fixed specification is preferable to a user-extensible slot framework or a generic builder template runtime.

Separate two responsibilities:

- The compiler validates that a direct root insert targets a registered name and folds its contribution to a structural String or retained resource-bearing value.
- The builder validates slot-specific meaning and assembles its final artefact. For example, `stylesheets` requires one tracked CSS resource, but `title` is ordinary String content.

Preserve authored spans, stable package-domain provenance and correct error lanes. An unknown builder slot should produce an authored diagnostic with the relevant list of available names where practical. Illegal nested/root placement and runtime-only contribution attempts should diagnose at their source, not panic at build assembly.

### Root activity and resource liveness

A root with only named const metadata slots still counts as an output-producing entry where existing entry rules allow it. Preserve pure API module behaviour and the current root-activation authority. Keep resource-origin pieces in their structural form through module publication and output planning. Ensure named contributions, like current body fragments, can keep resource outputs live even with no runtime HIR use. Do not introduce a second pass that scans emitted HTML for paths.

## 4. HTML builder slot contract and reserved metadata migration

### Registered slots

The HTML builder registers these destinations:

- Default destination: ordered body fragments.
- `title`: escaped page title text, retaining the existing route/project fallback and title prefix/postfix configuration when absent.
- `description`: escaped description meta attribute text, absent when no contribution and no configured override.
- `lang`: escaped `<html lang>` value, with existing builder defaults when absent.
- `favicon`: escaped icon URL, supporting existing structural resources and ordinary permitted Strings.
- `body_style`: escaped body style attribute, with existing builder defaults when absent.
- `stylesheets`: local CSS resource origins only, emitted as stylesheet link elements.
- `head`: raw additional head HTML, inserted **after** the builder's built-in metadata and other dedicated head content.

There are no cardinality errors for ordinary metadata slots. Multiple contributions concatenate exactly as normal template inserts do. A supplied empty String counts as present even if it looks odd for `title`. A missing slot uses the builder's existing fallback, distinct from the language-level per-occurrence default rule. Metadata values remain builder-interpreted Strings, not raw manually written `<title>` or `<meta>` elements.

Preserve current built-in document-head responsibilities, including charset, viewport, color scheme, core CSS, import-map placement and runtime script wiring. The builder owns `<html>`, `<head>` and `<body>`. Generate built-in head content and dedicated head elements in deterministic order, then append `head` contributions last. Do not let `head` replace or bypass escaping of scalar metadata.

### `stylesheets` validation, identity and output

Each top-level `$in stylesheets` contributes **exactly one tracked structural resource reference**. Accept canonical resource references, including paths supplied through valid compile-time aliases, only when the folded structural value still identifies a single resource. Ordinary rendered path Strings, combined URLs, `SiteRoot`, multiple resource references and non-CSS file types are invalid. Ignore only normal formatting-only whitespace permitted by the template value contract. Diagnose invalid contributions at the authored span. External URLs can be supplied manually through `head`.

Use existing `StableResourceOriginId` identity for deduplication. This identity is hashable and combines the resource owner with its normalised owner-relative logical path. Two differently authored paths to the same stable origin deduplicate. Two distinct origins do not become equal merely because their bytes match or they share a physical alias.

Retain the **first** occurrence of each origin in contribution order. Use the existing HTML resource plan and page-relative structural URL renderer to obtain the final `href`, then HTML-escape the attribute and generate `<link rel="stylesheet" href="...">`. Never compare source spellings, rendered HTML or content hashes to decide stylesheet identity. Do not read resource bytes just to determine inclusion or deduplication.

### `head` deduplication

`head` accepts arbitrary folded structural HTML Strings. Resolve each *complete contribution* in its page artefact URL context, then compare the final rendered String with earlier complete contributions. Emit only the first identical complete contribution, preserving order. Do not parse HTML and do not deduplicate duplicate subelements inside a single contribution. There is no generic `$unique` operation in this release.

Keep `head` and `stylesheets` deduplication as narrow HTML-builder output policies, not divergent language-level `$in` routing semantics. All other slots concatenate their contributions without deduplication.

### Eliminate the old metadata path

Retire the six reserved constant names entirely:

- `page_title`
- `page_description`
- `page_lang`
- `page_favicon`
- `page_body_style`
- `page_head`

Remove the HIR constant-name scan and its reserved-name extraction from `src/projects/html_project/page_metadata.rs`. Replace or delete that module according to its remaining responsibilities, and thread the new slot-driven entry metadata through the existing JS and Wasm HTML shell paths. Do not keep a compatibility alias from old constants to new slots. Old names become ordinary Moth declarations without implicit HTML metadata effects.

Retain the useful existing builder-side contracts: optional metadata values, source-aware structural resource uses, title fallback and prefix/postfix, HTML escaping, route-dependent URLs and identical JS/Wasm shell output. Reuse the existing `HtmlPageMetadataPlan` or its successor only if that remains the smallest complete shared output-planning shape. One owning path must gather builder slots and feed both resource planning and document rendering.

Update the documented `$page` metadata plan and any operational docs to reflect the accepted supersession: general `$entry` is a future separate entry-purpose directive and builder-specific values belong to top-level const slots. Do not implement `$entry`, `io.set_title` or legacy `$page` in this slice. Record that prior `$page(head = ...)` design is superseded rather than reproducing it.

## 5. Implementation phases and hard gates

### Phase 0 - Baseline, evidence and test admission

- Audit existing tests and select one primary integration case for the named-slot-inside-`$children` behaviour.
- Install the regression test in its **old** syntax and run it before altering compiler code. Assert rendered HTML and the exact named attribute value.
- If it fails, investigate the current composition scope and repair the accepted behaviour before migration. Retain the test throughout.
- Capture focused existing const/runtime wrapper results, JS/Wasm parity evidence and a clean benchmark baseline.

Gate: named-slot wrapper behaviour is either verified passing or documented as a concrete existing defect with a failing test and required correction. No rewrite begins with that behaviour unmeasured.

### Phase 1 - Slot/insertion syntax and per-occurrence defaults

- Replace `$slot(...)` parsing with `$slot`, `$slot identifier`, `$slot positive-integer` and optional `= <compile-time String>`.
- Replace `$insert(...)` with `$in identifier`. Preserve current stored-helper and runtime handoff semantics.
- Extend TIR slot occurrence metadata, constant folding, runtime plans, public constant projection and rebuilding to retain and select per-occurrence defaults.
- Ensure slot-name lookup supports static diagnostics and future tooling without creating a full symbol-table subsystem for every template.
- Migrate the smallest focused parser/slot integration examples first. Remove old syntax and dead argument adapters.

Gate: repeated named slots with different defaults, explicit empty fills, positional/default routing, stored inserts, imports and runtime slot applications all pass. Invalid old syntax has no supported compatibility path.

### Phase 2 - Before/after wrapper migration

- Add assignment-form parsing, const-value validation and named-only slot validation for `$before` and `$after`.
- Construct a shared TIR composite wrapper with the implicit default child slot, prefix value and suffix value. Use references and existing overlay machinery.
- Prove that the existing named-slot baseline still resolves from the intended composition scope. Keep the exact output assertion.
- Migrate `$children` regressions, first-party HTML helpers and benchmark sources to the new syntax. Rework helpers that require adaptation without changing their public output contracts.
- Remove old `$children` parser, diagnostic text and unused machinery after semantic parity is established.

Gate: old named-slot composition, `$fresh`, const/runtime no-output, loops, nested children, resource pieces and cross-module wrappers pass under the new syntax.

### Phase 3 - General builder slot routing

- Add a narrow builder slot schema input to frontend compilation.
- Recognise direct active-root top-level `$in` contributions and preserve the current default body lane and runtime insertion indices.
- Add destination, source span and owned structural resource value to the existing fragment handoff with minimal allocation and copying.
- Validate unknown/unsupported/root-only cases at their semantic owner.
- Provide builder-neutral tests with at least one non-HTML registered schema to prove frontend independence from HTML slot names.

Gate: direct root routing, default-body ordering, runtime interleaving, imported/API-only isolation, resource liveness and diagnostic provenance pass.

### Phase 4 - HTML metadata and stylesheet assembly

- Register HTML metadata, `stylesheets`, `head` and the implicit body destination.
- Replace reserved HIR metadata extraction and thread builder slot planning into resource preflight and both JS/Wasm output paths.
- Implement structural-resource validation and first-origin-wins stylesheet deduplication, then generate links with the existing final URL renderer.
- Implement identical-final-contribution head deduplication and place additional head contributions last.
- Migrate actual first-party pages, docs site, package examples and fixtures. Remove all implicit `page_*` effects and old extraction logic.

Gate: both HTML output modes preserve title, meta, route fallback, favicon and document shell contracts while using slots. Stylesheet and head deduplication pass without HTML scanning.

### Phase 5 - Cleanup, documentation and performance

- Audit old parser helpers, redundant string adapters, unused TIR wrapper code, outdated comments, docs and test duplication.
- Update canonical template, directive, entry-config, resource and HTML-builder docs, cheatsheet, progress matrix and roadmap.
- Record possible runtime top-level named slots as unsettled and `$required` and `$unique` as deferred candidates. Exclude loop uniqueness and const-function implementation.
- Review allocation growth and hot-path counters from the baseline. Keep the existing fast path for templates without decorators, defaults or named builder inserts.
- Run the required final validation for the complete branch and report exact commands, results, limitations and remaining risks.

Gate: no surviving old public syntax, no duplicate semantic pipeline, no unreviewed compiler/build boundary changes and no material unexplained performance regression.

## 6. Test matrix and realistic failure targets

Use focused Rust unit tests for parser and representation invariants. Use real Moth integration cases for observable composition and HTML artefacts. Extend existing cases before adding new fixtures. Follow `docs/developer-docs/style-guide/testing.mtf`. Tests belong outside production files.

### Slots and defaults

- Named identifier parsing, invalid names and unknown target diagnostic with source span.
- Positive positional indexes, invalid zero/negative/fractional indexes and existing loose-fill order.
- Named default and positional default, unfilled default slot, explicit empty insert and two occurrences of the same key with **different** defaults.
- Multiple inserts concatenate in source order. Explicit empty contribution prevents default even when its rendered output is empty.
- Stored insert helpers through immediate receivers and imported wrapper templates.
- Dynamic slot filling and const folding produce the same semantic output where both are supported.
- A missing default remains empty. No accidental mandatory-slot rule appears.
- Resource-bearing constant defaults retain resource identity through folding and public-interface transfer if ordinary String defaults permit that value form.

### Before/after

- **Primary regression:** named `row_style` inside a `$children` wrapper baseline, migrated to `$before` and/or `$after` with unchanged rendered output.
- Both directives, prefix only, suffix only and no decorators.
- Named slots in both values and named inserts from the intended receiving context.
- Explicit default or positional slot inside either value is rejected with the offending slot span.
- Runtime expression as a directive value, non-String value, missing `=`, malformed nested brackets and duplicate `$before` or `$after` yield precise diagnostics.
- `$fresh` applies to just one child and does not leak into descendants.
- False/absent conditional and zero-iteration loop emit no wrapper. A selected empty String still gets wrapped.
- Loop aggregate wrapped once versus per-iteration wrapping in the loop body.
- Nested child templates and repeated slot occurrences preserve existing application order.
- Import/re-export of a wrapper containing a named slot, structural resource pieces and direct-child decorators.
- JS and supported Wasm runtime handoff behaviour parity.

### Builder slots and HTML

- Direct active-root named insert, ordinary body fragment and runtime fragment interleaving in one entry.
- Named insert nested inside a normal root const template does **not** escape to builder routing.
- Unknown registered target, invalid placement, nonfoldable direct root helper and imported or API-only root isolation.
- Page containing only title or head content still gets a page if current entry-purpose rules permit it.
- Multiple `title` contributions concatenate rather than error. Absent title uses route/project fallback. Explicit empty title remains supplied.
- Escape text and attribute metadata correctly, preserve title prefix/postfix and existing HTML defaults.
- Two stylesheets written with different valid source spellings resolving to one `StableResourceOriginId` produce one link. Different origins produce separate links even with identical bytes.
- Non-CSS resource, plain String path, external URL String, `SiteRoot` and multiple resources in one stylesheet contribution reject with source-oriented diagnostics.
- One complete duplicated `head` contribution emits once, even if authored twice. Duplicates inside one contribution are not rewritten. Different full contributions remain ordered.
- Resolve structural URLs in nested page routes before head comparison. Ensure resource publication/preflight still tracks paths in discarded duplicate contributions according to ordinary graph and liveness rules.
- JS and Wasm HTML documents have equivalent metadata, link order and body/runtime output.
- Old `page_*` names are ordinary constants with no remaining reserved effect, and all shipped examples use slots.

## 7. Performance, code-size and safety review

Measure before and after on the existing template and builder benchmark suites, including `benchmarks/template-stress.moth`, `benchmarks/adversarial/template-render-plan-churn.moth` and the HTML builder workload. Use `just bench-frontend-check` and `just bench-check` where the repository's benchmark safety rules permit, and report their limitations. Benchmarks are not correctness tests.

Targets:

- No extra full TIR walk per child just to construct a before/after wrapper.
- No new recursive materialisation of prefix/suffix templates for each contribution.
- One builder root fragment traversal and no emitted-HTML parsing for resources or metadata.
- No duplicated resource provenance caches when existing module tables and output plans suffice.
- Zero or negligible extra retained storage for ordinary templates with no slot defaults or decorators.
- Preserve owner-local compact IDs, stable cross-module resource origins and use of existing source spans. Use shared references instead of cloning large structural Strings wherever ownership allows.
- Keep user-authored invalid cases in diagnostic lanes, true internal authority failures in infrastructure lanes and actual impossible states as compiler bugs. Do not convert malformed source into panics.

Follow up with an independent code-bloat review using the plan-orchestrator skill's `code-review-guide.md`. In particular look for orphaned `$children` helpers, old directive-argument parsers, duplicate metadata models, unnecessary slot registry abstractions and benchmark-only shims. Remove redundant code rather than preserving a compatibility wrapper.

## 8. Validation and deliverables

At each phase, run the targeted compiler/unit/integration owners. At broader checkpoints use `just validate`. For merge preparation run the full validation gate specified by the current `docs/developer-docs/style-guide/validation.mtf`. Use `just fmt-check`, compiler tests and selected `tests/cases` according to the repository's exact instructions. The Moth CLI is in the `moth-cli` crate, while library tests require the `moth` package. Run platform and backend variants supported by the current branch, without claiming unsupported Wasm or Windows coverage.

The implementation agent must deliver:

1. The original named-slot `$children` baseline test and recorded pre-refactor result, followed by the migrated test with the **same** expected output.
2. Final language/parser/TIR implementation of `$slot`, `$in`, per-occurrence defaults and `$before`/`$after`, with no legacy spellings supported.
3. The narrow builder-root slot handoff and HTML metadata/stylesheets/head implementation.
4. Migrated first-party packages, example source, documentation, progress matrix and roadmap notes.
5. A validation log with exact commands and outcomes, performance comparison, known limitations and any unresolved design/implementation questions.
6. A final correctness and code-bloat audit focused on stage boundaries, lifetime of source/resource identity, test footprint and deletion of stale code.

## 9. Explicitly out of scope

- General runtime named builder slots and runtime resource registration.
- Compile-time function syntax and struct-based component authoring.
- `$unique`, `$required`, any loop-level uniqueness and general slot-constraint execution.
- General `$entry` or superseded `$page` implementation and runtime title mutation.
- `$css` inline-only grammar changes, CSS parser expansion or external stylesheet URL inference.
- Automatic propagation of stylesheet dependencies from arbitrary nested component templates or folded Strings.
- Synthetic full-entry TIR parents, new builder callback engines, generic user-defined builder slot declarations and HTML parsing to discover dependencies.

These exclusions must remain visible in the roadmap where they affect unsettled final design. The current feature must be independently useful and complete without them.

## Acceptance checklist

- [x] Pre-refactor named-slot-inside-`$children` regression is installed, executed and retained through the migration.
- [x] `$slot`, `$in` and per-occurrence defaults match the approved syntax and routing behaviour.
- [x] `$before`/`$after` accept constant wrappers with named slots, reject their default/positional slots and preserve `$fresh` and runtime no-output semantics.
- [x] Root named inserts route only when direct and const, with builder schema independence and accurate source provenance.
- [x] HTML metadata uses slots only, with existing fallbacks and escaping intact.
- [x] `stylesheets` validates one tracked CSS origin per insert and deduplicates by stable origin.
- [x] `head` deduplicates only identical fully rendered complete contributions and is emitted after dedicated content.
- [x] Old `page_*` reserved behaviour, `$insert(...)`, parenthesised `$slot(...)` and `$children(...)` are gone.
- [x] JS/Wasm output parity, resource preflight, mixed const/runtime ordering and import isolation have regression coverage.
- [ ] Docs and roadmap reflect approved design and explicit deferrals, without introducing stale `$page` or `$stylesheet` claims.
- [ ] Validation and benchmarks are recorded, and no unnecessary parallel representation or legacy compatibility code remains.
