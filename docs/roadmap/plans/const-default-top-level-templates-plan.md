# Const-default direct top-level templates

## Status

- Status: Ready for implementation on the feature branch
- Current slice: Plan accepted, implementation not started
- Blockers: None for the accepted syntax change
- Next action: Update the header classifier and capture paths, then validate the semantic handoff

## Purpose

Make a direct top-level template in an executable `@entry.moth` root compile-time required by default. Require `~` before a direct top-level template that must execute through the entry runtime.

The ordinary entry page becomes mostly unmarked static composition. Runtime fragment production stands out at the call site instead of static output carrying a marker. This is a language surface change, not a new kind of template or a new rendering strategy.

The migration is deliberately breaking. Replace the old accepted syntax and its documentation and fixtures in the same work. No compatibility parser, transitional warning or dedicated migration diagnostic is permitted.

## Accepted language contract

A direct top-level template in `@entry.moth` has two forms:

```moth
@page content as body

[$in title:Moth]
[$md:
    # Welcome
]
[body]

name = "Moth"
~[: Hello, [name].]
```

- `[...]` at the direct entry top level is a **const-required fragment**. The compiler must validate and fold it using the existing constant-template semantics. It contributes compile-time fragment metadata and no runtime fragment slot or template HIR.
- `~[...]` at that position is a **runtime fragment**. It is evaluated through the existing dormant entry body and published only on successful entry invocation. It retains a runtime fragment slot even if the template could be folded. Normal subexpression folding remains legal.
- Prefix `~` is consumed by entry header parsing and is not included in the body token range handed to AST. AST sees the ordinary template token stream it already handles.
- Prefix `~` is valid only when attached to a direct top-level template in the active entry root, at a statement boundary. The canonical spelling is `~[... ]` with no space, shown in examples as `~[...]`. Horizontal whitespace between the marker and opening bracket on the same statement is accepted. A newline cannot separate them. Use existing tokens and source spans, not a new lexer mode.
- An unmarked direct template is never silently promoted to runtime because one of its inputs is not constant. It receives the existing constant-evaluation diagnostic at the appropriate authored location.
- `#[...]` is no longer a valid direct fragment. Do not support it as an alias or add a migration-specific diagnostic. Keep ordinary explicit compile-time bindings such as `name #= value` unchanged.
- Direct unmarked `$in` templates preserve named builder-slot behavior, including resource and URL facts. Explicit runtime `~[$in ...]` is rejected under the existing unsupported runtime named-slot contract. Nested, assigned and aliased inserts remain normal template composition.
- Only direct top-level templates in an executable entry root publish page fragments. Ordinary helper files, `@mod.moth`, `@package.moth`, `config.moth` and imported or dormant entry roots keep their existing role restrictions.
- In ordinary expression positions, functions, declarations, nested templates and `.mtf` source content, `[...]` retains the current compile-time-or-runtime behavior. This change creates no general `~` template expression operator, reactive template marker or inferred entry-phase selection.
- Documentation-only templates (`$doc`, `$note` and `$todo`) remain semantic non-output categories. Preserve their existing placement and validation contract rather than counting them as page fragments.

The change does not alter template body syntax, `if`, `loop`, slots, wrappers, formatting, numeric rules, structural String values or constant loop limits.

## Retained runtime and build contracts

- The entry root stays presence-selected, including empty and metadata-only entries. Imported entry bodies remain dormant.
- Header preparation remains the only owner of direct-fragment classification, fragment counts and const-fragment placement relative to runtime fragments.
- Constant fragments continue to fold once through AST/TIR and enter `ModuleCompilerMetadata` with their structural resources, destinations, authored spans and runtime insertion indices.
- Runtime fragments continue through existing start-body AST, `PushStartRuntimeFragment` and HIR lowering. No alternate AST/TIR node, fragment registry or builder-side source parser is needed.
- Static fragments retain authored source order around runtime slots, remain present on entry startup failure and have no required runtime output machinery in a static-only page.
- Runtime fragments remain staged until the entry finishes successfully. The change does not add a retry, rollback, error HTML channel, per-request renderer or additional builder lifecycle.
- Resource tracking, site-root rendering, builder slot deduplication, JS/Wasm assembly, executable reachability and fingerprint domains continue to consume the same compiler facts.

## Authority to read before implementation

Follow `AGENTS.md` and the task-reading guides of the current repository worktree.

- `docs/docs/templates/template-basics.mtf`, `template-directives.mtf` and `template-control-flow.mtf`
- `docs/docs/constants/const-templates.mtf` and the constant binding/folding references
- `docs/docs/project-structure/entry-runtime-and-fragments.mtf` and `entry-config.mtf`
- `docs/compiler-design-overview.md`: module inputs and root roles, Stage 2 header preparation, Stage 3 ordering, Stage 4 constants and TIR, metadata and HIR handoff
- `docs/build-system-design.md`: entry/root eligibility, direct builder slots, entry assembly, HTML fragment assembly and resource linking
- `docs/developer-docs/style-guide/style-guide.mtf`, `testing.mtf` and `validation.mtf`
- `docs/docs/cheatsheet/moth-language-cheatsheet.mtf`, the relevant progress matrix and `docs/roadmap/roadmap.md`

The canonical unsuffixed language references own final observable semantics. Before implementation completes, update those references to this accepted contract. This plan does not become a separate permanent syntax authority.

## Current code and ownership map

Investigated on main before this plan was created. Re-read these owners when the feature branch begins implementation, as concurrent compiler work can change them.

- `src/compiler_frontend/headers/top_level_classifier.rs`: `HeaderFileItem` and `classify_tagged_item` currently dispatch a bare `TEMPLATE_HEAD` to `RuntimeTemplate` and distinguish top-level `HASH`.
- `src/compiler_frontend/headers/file_parser.rs`: `parse_headers_in_file_inner` handles classified items, source modifiers and nesting. `handle_runtime_template_item` captures an ordinary runtime template and increments the runtime fragment counter. Export parsing and non-entry role rejection also pass through here.
- `src/compiler_frontend/headers/hash_items.rs`: dedicated handler for the old `#[...]` direct const form. Most of this special-case path should become unnecessary. Delete this module if nothing meaningful remains after consolidating the new dispatch.
- `src/compiler_frontend/headers/const_fragments.rs`: `create_top_level_const_template` already owns synthetic constant headers, dependencies, token range, spans and ordering facts. Reuse this implementation for the new default direct form.
- `src/compiler_frontend/headers/start_capture.rs`: `capture_runtime_template_range_from_cursor` captures from `TEMPLATE_HEAD`. Reuse it after consuming the explicit `~` marker.
- `src/compiler_frontend/headers/file_state.rs`, `types.rs` and `parse_file_headers.rs`: preserve `TopLevelConstFragment`, `runtime_fragment_count`, `const_template_count` and insertion-index aggregation.
- `src/compiler_frontend/ast/templates/top_level_templates.rs` and AST emission/finalisation: already fold const fragments and return them to metadata. Preserve the single TIR authority.
- `src/compiler_frontend/ast/statements/body_dispatch.rs`: ordinary entry-body template statements already produce `PushStartRuntimeFragment`. The new marker must not leak here.
- `src/compiler_frontend/tokenizer/lexer.rs`: `~` is already tokenised as `TokenTag::MUTABLE`. Do not introduce an overlapping punctuation token.
- `src/compiler_frontend/compiler_messages/diagnostics/families/names_declarations.rs` and template diagnostics: retain precise wrong-position and const-required errors. Add a typed reason only if an existing family cannot express a genuinely distinct error.

Audit test owners before adding coverage: `src/compiler_frontend/headers/tests/`, `src/compiler_frontend/tests/`, `src/compiler_tests/` and `tests/cases/`. The docs site and CLI scaffold are real consumers of this syntax.

## Implementation phases

### Phase 0 - Inventory and baseline

1. Check the branch against the current main tree, including adjacent changes to header parsing, entry assembly, template directives and TIR.
2. Search for `#[`, bare direct root `[`, builder slot inserts and top-level runtime template fixtures across `src/`, `tests/`, `docs/`, `benchmarks/`, `crates/` and examples. Classify each match by semantic context. A code example inside `$code` is not a direct output template, and an intentionally invalid test should not be rewritten without reviewing its assertion.
3. Inspect the present focused header/AST tests and the canonical integration manifest. Choose existing primary test owners and identify any required distinct new scenarios.
4. Record baseline targeted test and documentation-build results when feasible. Do not pin an unstarted baseline commit into this plan.

### Phase 1 - Reverse header classification

1. Make a direct top-level `TEMPLATE_HEAD` classify as a constant fragment where the file role permits entry fragments.
2. Classify `MUTABLE` followed by `TEMPLATE_HEAD` on the same statement boundary as an explicitly runtime fragment, taking horizontal whitespace and newline handling into account. Keep the check at the existing header classifier/parser owner, without a second scan or lexer mode.
3. Dispatch an ordinary const fragment through the existing `create_top_level_const_template` owner, recording its true opening token, header path, span and placement. Do not fabricate a `#` token or duplicate the synthetic-header construction logic.
4. Dispatch the explicit runtime form through the existing balanced template capture routine. Drop only the `~` prefix from the retained runtime body sequence. Keep all authored token locations intact.
5. Reject inappropriate placements using the existing role, export, nested statement and source-modifier boundaries. Do not turn `~` on a mutable write or a normal expression into a template prefix.
6. Delete the old `#[...]` dispatch implementation and any unneeded `hash_items` indirection. Keep `HASH` handling only where source language syntax actually needs it. Do not leave a parallel compat path.

Pay particular attention to the current parser loop's `cursor.advance()` before dispatch, its `NestingDepth` state, reached-facade balancing and source-modifier attachment. No marker may be counted as a runtime statement separate from its template.

### Phase 2 - Preserve semantic handoffs and build output

1. Verify the default direct fragment still creates its synthetic constant declaration, including header ordering against local and provider constants, structural references and full const-required validation.
2. Confirm its collected value and destination retain the correct fragment insertion index for every sequence of static and runtime templates. Fragment numbering must reflect only actual runtime fragments, not other top-level statements.
3. Confirm the explicitly marked form retains exactly one runtime fragment slot and the existing startup/error/publication behavior, including when its content is constant.
4. Confirm direct unmarked `$in` slots still fold and publish metadata with the same resource identities. Runtime named slots stay rejected. No builder output or URL resolution logic should need to be modified merely to detect the marker.
5. Verify config, helper, facade, synthetic single-file and direct `.mtf` service boundaries still behave as specified. In particular, avoid accidentally treating a standalone `.mtf` body as an entry-level template.
6. Update internal names and comments that claim bare direct templates are runtime or `#` is required for const fragments. Rename only where doing so makes meaning clearer. Keep existing `TopLevelConstFragment` and runtime handoff structures if their names remain accurate.
7. Check that static-only entries do not acquire an unnecessary runtime helper, JS companion obligation or altered output fingerprint just because their syntax changed.

### Phase 3 - Focused regression coverage

Extend existing tests where possible. New tests must cover distinct failure mechanisms, not repeat one syntactic check under many names.

- **Header classification**: adjacent and same-line spaced `~[`, newline-separated `~` and `[`, end of file, CRLF, leading indentation, adjacent statements, nested brackets and the old `#[` spelling.
- **Roles and contexts**: direct templates permitted in `@entry.moth` and synthetic entry compilation, rejected in helper and facade roots, not confused with bindings, `~=` writes, mutable-access statements or expression-level templates.
- **Const enforcement**: constant literals, imported folded values, direct resource pieces, constant `if` and `loop`, and a genuinely runtime-dependent interpolation in an unmarked direct fragment that must diagnose rather than emit runtime HIR.
- **Runtime enforcement**: `~[...]` with runtime interpolation and with wholly static content. Assert correct runtime-fragment count and publication contract in both cases. Check that neither causes a redundant const-fragment record.
- **Metadata and interleaving**: several const fragments separated by explicit runtime fragments and other root work, including named `$in` slots and repeated resources. Assert body order, static metadata, deduplication and the exact fragment count or authoritative output.
- **Failure**: a running entry that fails after static fragments and before runtime fragment publication must preserve static content and suppress staged runtime fragments. Keep the existing JS and applicable Wasm output contracts.
- **Comments and non-output**: direct documentation/comment template categories remain non-output, and nested `$in` continues to mean normal template composition.

Use focused Rust unit tests for token/dispatch invariants, typed diagnostics and fragment metadata. Use a small number of realistic integration cases under `tests/cases/` for semantics, generated HTML, runtime behavior and target parity. Prefer runtime output assertions over snapshots of emitted JS. Do not change test runner infrastructure or duplicate cases just to inflate coverage.

### Phase 4 - Migrate every consumer of the old syntax

1. Change real direct entry fragments from `#[...]` to `[...]` and direct runtime fragments from `[...]` to `~[...]`. Do this based on their context, not as a blind global replacement.
2. Migrate the Moth documentation site's entry files, source-backed page examples, test fixtures, CLI-generated starter projects, package examples and any benchmark input sources that are meant to remain executable.
3. Fix expected output, diagnostics and test manifests that legitimately changed. Preserve intentional negative tests and update their asserted reason where necessary.
4. Update canonical reference text and snippets in `const-templates.mtf`, `template-basics.mtf`, `template-directives.mtf`, `entry-runtime-and-fragments.mtf` and `entry-config.mtf`. Update paired Basic references, cheatsheet, other source examples and relevant compiler/build architecture paragraphs that still describe the old syntax. Keep final-design references free of migration status or implementation progress.
5. Review syntax-highlighting, scaffolding text, tests that inspect source characters and developer tooling for a baked-in `#[` assumption. Change them only where there is a real dependency.
6. Update the compiler progress matrix if implemented syntax or backend coverage changed. Update `index.md` only if an owning module/file moves or fundamentally changes. Apply the repository's audit-log staleness policy if this work changes the scope of a recorded audit.
7. Do not hand-edit generated `docs/_release` outputs. Regenerate using the normal builder.

### Phase 5 - Validation and final Slice review

During implementation use targeted parser/AST tests and the smallest affected integration selections after each meaningful slice. At the integrated checkpoint run `just validate` plus any relevant tests that routine validation omits. Before merge run `just validate-full` against the final integrated tree, as required by `validation.mtf`.

Also prove the documentation site with `moth check docs --terse` and `moth build docs --release`, or the documented Cargo equivalents when a suitable Moth executable is unavailable. Validate the CLI scaffold by creating and building a generated project. Check JS output and applicable HTML-Wasm target behavior rather than claiming parity for unsupported capabilities.

Run a focused stale-syntax audit after migration. Search for old `#[` direct fragments and unmarked direct fragments that still require runtime evaluation. Every remaining match must be intentionally non-entry syntax, quoted source, an invalid test or an explained unsupported case. Recheck assertions and sample projects rather than accepting search counts as validation.

Finish with the required Slice review:

- **Correctness**: each direct template selects one and only one existing const or runtime handoff, with correct spans, ordering and failure behavior.
- **Complexity**: no new token kind, parsing pipeline, TIR representation, runtime fragment model, builder-specific detection or syntax compatibility shim.
- **Deletion**: remove obsolete hash-template handlers, dead branches, stale comments and duplicated tests where no longer justified.
- **Ownership**: Stage 2 classifies, Stage 3 orders, AST/TIR validates and folds, HIR carries runtime work, and builders consume metadata and executable facts.
- **Diagnostics**: authored invalid syntax follows typed diagnostic lanes and never panics. No migration-only warning or special error.
- **Tests**: each retained new test catches a distinct credible regression, and all changed existing tests retain their original useful assertions.
- **Docs**: canonical references, cheatsheet, starter, examples, roadmap/status and generated site agree with delivered behavior.
- **Performance**: avoid a second scan or added allocation on common const entries. If hot-path code changes materially, compare focused frontend/documentation build evidence before claiming an improvement.

The implementation is complete only when the new syntax is the sole accepted direct top-level surface and every supported project sample builds. In the same final implementation commit, delete this plan and remove its roadmap entry instead of leaving a completed plan behind.
