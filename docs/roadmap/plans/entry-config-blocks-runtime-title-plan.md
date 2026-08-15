# Entry-local config blocks and runtime title implementation plan

## Purpose

Implement root-local builder metadata through one `config:` block that uses ordinary module visibility and AST folding, replace reserved HTML metadata constants with resolved entry settings and add supported runtime title mutation.

## Current-state capsule

```text
ACTIVE_PLAN: docs/roadmap/plans/entry-config-blocks-runtime-title-plan.md
STATUS: queued
CURRENT_SLICE: Phase 0 - refresh module-root, metadata, schema and HTML document owners
LAST_GOOD_COMMIT: none until the first implementation slice is accepted
POST_TIR_REVIEW_COMMIT: 1298da468
BRANCH: main
IMPLEMENTATION_SCOPE: frontend header parsing, AST folding, module metadata, HTML builder, Core IO
```

## Hard prerequisites

- final TIR one-store/exact-view architecture and post-TIR roadmap review accepted at `1298da468`
- canonical modules
- project config and `@project`
- section-aware builder schemas
- anonymous const records
- source `#Import`
- immutable module artefact metadata lanes

## Required authority documents

- `docs/compiler-design-overview.md` for header syntax preparation, AST folding, module artefact metadata and target validation
- `docs/build-system-design.md` for entry-local `config:` block placement rules, section schemas and entry assembly
- `docs/src/docs/codebase/language/overview.mtf` and its project-structure references for source syntax
- `docs/src/docs/codebase/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`
- `docs/src/docs/progress/@page.moth` for current support
- `docs/roadmap/plans/canonical-module-compilation-and-scoped-packages-plan.md` for the module graph
- `docs/roadmap/plans/import_values_anonymous_records_plan.md` for config schemas and `@project`

## Accepted entry config design

See `docs/build-system-design.md` "Entry-local config: blocks" for the full contract.

The block is not an isolated config compilation unit. It uses ordinary module visibility and AST folding. This replaces the rejected isolation model.

Placement and cardinality:
- valid only at the top level of a normal module root
- at most one block per normal root
- invalid in normal non-root files, support roots, the project package facade, inside `export:`, inside executable bodies, and in `config.moth`

Block contents:
- section records only
- no dependency clauses, aliases, helper constants, support types or `#Import` declarations inside the block
- these live outside the block in the normal root file

Visibility and folding:
- the block uses the root file's ordinary compile-time visibility
- it may reference dependency-bound constants, `@project`, same-file constants declared before the block, resolved source `#Import` constants, foldable local const-record types and selected-builder compile-time values through normal module dependency clauses
- same-file forward references remain invalid
- header syntax records local dependencies
- interface binding resolves dependency clauses normally
- AST folds the block through the ordinary module semantic path

What the block does not create:
- no ordinary module symbol
- no HIR
- no project-global value
- no `project` section or project-level builder behaviour change

Schema and activity:
- may contain active artefact-builder and tooling-overlay sections
- active entry sections are schema-validated
- inactive sections are parsed and folded but not schema-validated
- the block is optional, and its active artefact-builder section is also optional so tooling-only metadata remains possible
- every selected normal module captures, folds and validates its own block once as part of canonical module compilation
- resolved entry metadata is stored in that module's canonical `ModuleCompilerMetadata`
- consumers never apply another module's entry metadata
- entry assembly activates entry metadata only for the selected module
- only active artefact-builder settings contribute entry activity
- entry metadata contributes to the root-activity fingerprint
- entry activation does not compile it later

Use `current entry config surface`, not `V1`.

## HTML migration

Retain:
- HTML entry fields for title, description, language, favicon, body style and head content
- shared initial document metadata for JavaScript and mixed output
- config-only HTML entries (a root containing only a `config:` block with no start body still produces an HTML document)
- deletion of `page_title`, `page_head` and related reserved-constant scanning
- targeted migration diagnostics only when they describe current replacement syntax
- `io.set_title` for supported JavaScript hosts with canonical string-content conversion
- target validation before lowering for unsupported assignments
- no silent no-op
- no assumption that every JavaScript target has `document`

Active `html` entry-section fields (String shape):
- `title` for `<title>` content
- `description` for `<meta name="description">`
- `lang` for `<html lang="...">`
- `favicon` for `<link rel="icon">`
- `body_style` for `<body style="...">`
- `head` for raw or folded extra HTML inserted into `<head>`

`head` is one folded string or template value. Authors compose multiple fragments in an ordinary compile-time template. No `+=`, per-key merge or typed head nodes.

Runtime title:

- `io.set_title(StringContent) -> Void` sets the live document title on targets that advertise browser document-title capability.
- The call records a stable external function identity and a browser document-title capability requirement in per-function link facts.
- HTML-JS advertises the capability and emits the helper only when the function is reachable.
- Calling it after initial page load overrides the initial title from entry config.
- HTML-Wasm does not advertise the capability and rejects reachable calls before lowering.
- Standalone or embedded JavaScript targets without browser document-title capability reject reachable calls before lowering.
- Unsupported hosts never reach a generated helper and never use a runtime fallback or silent no-op.
- If a host advertises the capability but violates that target contract at runtime, the failure is an infrastructure or host-contract failure rather than normal unsupported-target behaviour.

## Non-goals

- no isolated config compilation unit for entry blocks
- no separate parser for entry config
- no dependency clauses, helper declarations or `#Import` inside the block
- no `project` section inside the block
- no shared project or entry field scope
- no multiple blocks per root
- no block merging or last-write-wins semantics
- no `io.get_title`, reactive title binding or runtime mutation of other metadata
- no HTML-Wasm lowering for `io.set_title`
- no silent no-op on unsupported hosts

## Risks and blockers

- the block uses ordinary module visibility, so dependency clauses and contract declarations must live outside the block in the normal root file
- current HTML pages may derive metadata from surrounding dependency clauses that need to move to allowed clauses outside the block
- `io.set_title` is host-capability-sensitive and must not assume every JavaScript target has a browser `document`
- modules without `config:` must stay on a near-zero-overhead path

## Implementation phases

Each phase must leave one coherent path. Reference `docs/build-system-design.md` "Entry-local config: blocks" for the full contract.

### Phase 1: Refresh module-root, metadata, schema and HTML document owners

Context: this plan depends on canonical modules, project config, `@project`, section-aware schemas,
anonymous const records and source `#Import`. Its template boundary was reviewed against
`1298da468`; refresh ordinary implementation anchors before code starts without adding a TIR-facing
entry-config API.

- Confirm all hard prerequisites are accepted.
- Confirm entry metadata consumes folded owned values or neutral owned runtime template payloads; no TIR identity, view, overlay or preparation state enters module metadata, HIR or the HTML builder.
- Record `git rev-parse HEAD`, branch and `git status --short` in the context capsule.
- Inventory current module-root header parser, config schema registry, HTML page metadata resolution, Core IO registration and JS helper emission.
- Search and count legacy `page_title`, `page_head`, `page_description`, `page_lang`, `page_favicon`, `page_body_style` usage across source, fixtures and docs.
- Run baseline `just validate` and record results.

### Phase 2: Add root-block shell parsing and placement diagnostics

Context: the root parser isolates the `config:` block and produces an entry-config payload for later AST folding. This phase adds syntax and placement rules but does not yet hand values to the builder.

See `docs/build-system-design.md` "Entry-local config: blocks" placement rules.

- Add `config:` recognition only at a top-level item boundary in a normal module root.
- Parse through the matching top-level closing `;`.
- Capture the block body with original source file identity, token or source locations and block start location.
- Ensure block tokens are not emitted as normal module headers, constants, exports, `start` body tokens or page fragments.
- Enforce one block per normal module root. A second block is a structured duplicate-block diagnostic.
- Reject `config:` in normal non-root files, support roots, the project facade, inside `export:`, inside executable bodies and in `config.moth`.
- Reject nested `config:` blocks.
- Reject malformed or missing colon and unterminated forms.
- Use one canonical normal-root parse path: every selected normal module captures its own optional block whether it later becomes an entry or is consumed by another module.
- Do not add active-root or imported-root parser modes. Consumers never receive, suppress or activate another module's block payload.
- Add block payload remapping for worker-local string tables.
- Thread the optional block payload through the owning module's header aggregation without making it a declaration graph node.

### Phase 3: Add local dependency hints and AST folding through bound visibility

Context: the block uses ordinary root-file visibility and is folded through the normal module AST path, not through an isolated config compiler.

See `docs/build-system-design.md` "Entry-local config: blocks" visibility rules.

- Header syntax records local dependencies for the block (references to same-file earlier constants, `@project`, dependency-bound constants, resolved source `#Import` constants).
- Interface binding resolves dependency clauses normally for the root file. The block benefits from the same bound visibility.
- AST folds the block once through the ordinary canonical module semantic path.
- Same-file forward references remain invalid.
- The block creates no ordinary module symbol, no HIR and no project-global value.
- Do not inject surrounding module runtime values or start bindings into the block.
- Do not create config-only generated symbols visible to normal module code.

### Phase 4: Add separate entry schemas and module metadata storage

Context: entry config needs its own schema validation and must be stored as builder-facing metadata, not HIR.

- Add separate entry schemas (distinct from project schemas, no shared fields, no `ProjectAndEntry`).
- Active entry sections are schema-validated. Unknown fields in active sections are diagnostics.
- Inactive sections are parsed and folded but not schema-validated.
- Store resolved entry config in the owning module's canonical `ModuleCompilerMetadata`, not in HIR.
- Ensure HIR generation consumes only the normal module AST.
- Ensure HIR validation and borrow validation need no config-specific nodes.
- Add empty or default representation for roots without a block.
- Implement `StringId` and source-location remapping for the module payload.
- Update root activity representation so non-empty entry config is available to builder artefact policy.
- Confirm consumers never apply another module's entry metadata.
- Confirm every selected normal module processes its optional block through capture, folding and validation exactly once.

### Phase 5: Replace HTML reserved-constant scanning

Context: this is the behaviour cutover. The HTML builder reads resolved entry config instead of scanning HIR constants by name.

- Register `title`, `description`, `lang`, `favicon`, `body_style` and `head` as fields of the active `html` entry section (all String shape).
- Add an HTML entry-config resolver that reads already-validated resolved values and maps them to `HtmlPageMetadata`.
- Keep one resolver shared by HTML-JS and HTML-Wasm paths.
- Preserve document shell defaults and escaping behaviour.
- Preserve raw `head` insertion behaviour.
- Remove HIR module constant scanning from page metadata extraction.
- Remove reserved metadata-name tables and entry-scope prefix matching.
- Remove duplicate and wrong-string page metadata diagnostics now covered by shared config validation.
- Add targeted migration diagnostics for legacy `page_*` constants in normal modules selected as HTML entries.
- Update HTML artefact filtering: non-empty entry config counts as HTML artefact activity, config-only root emits a document, API-only root with no config, body or fragments remains skipped.

### Phase 6: Share document metadata across JavaScript and mixed output

Context: the initial document shell must be identical for HTML-JS and HTML-Wasm.

- Ensure HTML-JS and HTML-Wasm use the same resolved active `html` entry section when rendering the initial document shell.
- Add JS and Wasm shell parity tests.
- Add config-only page test.
- Add no-config fallback tests.

### Phase 7: Add `io.set_title` metadata, capability validation and JavaScript lowering

Context: runtime title mutation is a browser document capability. Unsupported targets reject it through the normal target-contract lane before lowering.

- Add stable external function identity for `io.set_title`.
- Register `io.set_title` in Core IO with one shared `StringContent` parameter, `Void` success and no source-visible return value.
- Add an explicit browser document-title capability to target metadata.
- Record the capability requirement in per-function link facts when `io.set_title` is referenced.
- Make HTML-JS advertise the capability.
- Keep HTML-Wasm and non-browser JavaScript targets unsupported unless they explicitly advertise an equivalent host contract.
- Run external package and target-capability validation against the completed reachable root set before lowering.
- Reject every reachable unsupported call with one stable target-contract diagnostic.
- Do not generate the helper for a rejected target.
- Emit the JavaScript helper only when the call is reachable on a capable target.
- Convert input through canonical Moth string-content conversion.
- Set the live document title.
- Treat a capable host that lacks its advertised document-title facility as an infrastructure or host-contract failure.
- Add helper reachability tests.
- Add a generated artefact assertion containing title mutation.
- Add HTML-Wasm and standalone non-browser JavaScript rejection fixtures with the same stable diagnostic family.
- Verify runtime title can override the initial `config:` block title.

### Phase 8: Delete isolated config parsing, compatibility scanning and old constants

Context: the refactor is not complete while old isolated config parsing, compatibility scanning or reserved constants remain.

- Delete any isolated config compilation path for entry blocks.
- Delete any separate parser for entry config.
- Delete reserved HIR page metadata scanner and fallback.
- Delete duplicate value extraction helpers.
- Delete `ProjectAndEntry` shared-scope escape hatch if it exists.
- Delete unused page-metadata diagnostic variants.
- Delete temporary adapters introduced during phased wiring.

### Phase 9: Migrate examples, scaffolding, user docs and progress rows

Context: documentation and scaffolding must teach the accepted entry config model.

- Migrate every legacy HTML metadata declaration into the active `html` entry section (`page_title` to its `title` field, `page_head` to its `head` field, etc).
- Move required dependency clauses outside the block in the normal root file.
- Update generated HTML project scaffolding.
- Update integration manifest entries.
- Keep focused migration-diagnostic fixtures for legacy names.
- Update `docs/src/docs/project-structure/entry-config.mtf` with `config:` block source semantics.
- Update HTML page and document metadata source pages.
- Update Core IO source pages with `io.set_title`.
- Update progress matrix rows for entry config, scoped builder keys, HTML metadata, legacy removal and `io.set_title`.
- Rebuild generated documentation through the compiler.

## Old owners and paths to remove

- isolated config compilation for entry blocks
- separate parser for entry config
- reserved HIR page metadata scanner (`page_title`, `page_head` and related constants)
- legacy page metadata fallback
- duplicate value extraction helpers
- `ProjectAndEntry` shared-scope escape hatch
- unused page-metadata diagnostic variants

## Required tests

Cover:

- one block at most in a normal root
- block rejected in non-root files, support roots, `export:` and executable bodies
- section records only, no dependency clauses or helpers inside
- ordinary root-file visibility for the block
- same-file earlier constants usable inside
- dependency-bound constants and `@project` usable through normal visibility
- no ordinary module symbol or HIR from the block
- strict active entry schema validation
- inactive section folding without schema validation
- every selected normal module validates its block
- dependency modules do not apply entry metadata to consumers
- entry metadata stored in module compiler metadata
- HTML entry fields: title, description, language, favicon, body style, head
- shared initial document metadata for JavaScript and mixed output
- config-only HTML entries produce a document
- `page_title` and `page_head` migration diagnostics
- reachable `io.set_title` on capable HTML-JS emits the helper
- unreachable `io.set_title` emits no helper
- reachable `io.set_title` is rejected before lowering on HTML-Wasm
- reachable `io.set_title` is rejected before lowering on standalone non-browser JavaScript
- unsupported targets produce no generated helper and no runtime fallback
- a target that advertises browser document-title capability reaches lowering through the same external function identity and capability metadata
- no silent no-op exists

## Documentation and progress-matrix impact

- update `docs/src/docs/project-structure/entry-config.mtf` with `config:` block source semantics
- update `docs/build-system-design.md` only if a durable entry-config contract is confirmed missing
- update HTML page and document metadata source pages
- update Core IO source pages with `io.set_title`
- update README examples
- progress matrix rows: entry-local `config:` blocks, scoped builder keys, HTML compile-time metadata, legacy `page_*` removal, `io.set_title`

## Validation requirements

Each code-bearing phase runs:

```bash
cargo fmt
just validate
```

Run the documentation release build when source docs change.

## Final architecture audit

Before marking this plan complete, verify:

- the block uses ordinary module visibility and AST folding, not an isolated config compiler
- no separate parser for entry config remains
- no reserved HIR page metadata scanner remains
- entry metadata is stored in module compiler metadata, not HIR
- dependency modules do not apply entry metadata to consumers
- `io.set_title` lowers only for targets that advertise browser document-title capability and every unsupported reachable call is rejected before lowering
