# Export directive migration

## Status

- Status: ACTIVE. Dedicated implementation branch: \`feature/export-directive\`.
- Current slice: 0 - inspect the branch baseline, finalise the source-modifier attachment points and inventory legacy export ownership.
- Blockers: None. General source directives, facade-root roles, public export metadata and strict direct-selection re-exports are already delivered.
- Next action: Implement the ordered slices below on this branch. Keep this plan and its roadmap entry until the completing implementation commit retires the plan.

## Goal

Replace the Moth source keyword and block \`export:\` with the compiler-owned, operand-free \`$export\` source directive. Make \`export\` a normal, unreserved source identifier again.

\`$export\` attaches to exactly one following exportable top-level declaration or one strict direct-selection dependency clause. It is legal only in \`@mod.moth\` and \`@package.moth\`. Everything else stays private by default.

This is an intentional breaking alpha change. Remove the old block grammar, special parser state, keyword recognition and obsolete implementation machinery immediately. Do not accept the old spelling, provide compatibility, or create a migration-specific diagnostic or hint. Ordinary syntax/target diagnostics may reject old text without recognising it as a retired form.

Preserve the existing public interface model, stable declaration origins, export bindings, aliases, re-export restrictions, access and effect summaries, root isolation and consumer-visible behaviour. This is a source-syntax and preparation-metadata change, not a new public interface or facade model.

## Authorities and prerequisites

Follow \`AGENTS.md\`, the compiler code style, testing and validation guides, and the compiler code review guide. Read the authority text and applicable sections of:

- \`docs/compiler-design-overview.md\`: module root roles, Stage 1-3 ownership, public semantic interfaces, fingerprints and diagnostics.
- \`docs/build-system-design.md\`: root discovery, \`@mod\`/\`@package\` facades, dependency clauses, support packages and incremental reuse.
- \`docs/docs/directives/directives.mtf\`: compiler-owned source directive form, attachment, trivia, operands and wrong-context diagnostics.
- \`docs/docs/project-structure/public-api.mtf\`, \`docs/docs/project-structure/module-roots.mtf\`, \`docs/docs/packages/public-reexports.mtf\` and the visibility/alias references.
- \`docs/developer-docs/style-guide/style-guide.mtf\`, \`testing.mtf\` and \`validation.mtf\`.
- Both progress matrices, \`index.md\` for source navigation and the official language reference routes when updating syntax examples.

Consume the delivered directive registry, retained source-modifier attachment, \`HeaderExportMode\`, \`HeaderKind::is_authored_public_export_declaration\`, strict dependency-selection records and existing public export builders. Do not create another export resolver, second interface representation, AST search or build-system parser.

## 1. Accepted source contract

### Direct declarations

\`\`\`moth
-- @mod.moth
private_helper || -> String:
    return "private"
;

$export
greeting #String = "Hello"

$export
render || -> String:
    return private_helper()
;
\`\`\`

The directive marks the next eligible item public in that facade's surface. All other items remain private. A facade with no marked items exports nothing.

Allowed direct declarations are exactly the currently supported public declaration categories:

- Free functions, including valid generic functions.
- Structs and choices.
- Type aliases and traits.
- Compile-time constant bindings, including inferred and explicitly typed \`#\` constants, subject to the existing public-constant validation.

Keep all existing public-surface restrictions, including private-type/evidence leakage, closed exported error contracts and project-context provenance. Receiver methods are never independently exported. Their public visibility continues to follow the exported receiver type and existing receiver checks.

Not eligible: ordinary runtime or mutable bindings, executable statements, page fragments, runtime templates, const-template fragments, trait conformances and incompatibilities, receiver methods as free public symbols, \`$config\` input contracts, nested/body-local declarations and non-declaration source syntax. The shared header eligibility predicate is authoritative for direct declarations. Do not casually widen it.

### Direct-selection re-exports

\`\`\`moth
-- @package.moth
$export
@widgets Button, Card as PublicCard, Icon

$export
@rendering render_button
\`\`\`

A single \`$export\` on a dependency clause marks every selected symbol on that clause as public. Reuse the existing direct-selection list, comma/newline and per-leaf \`as\` alias grammar. The public names, original origins and alias semantics are unchanged.

Accept only an explicit, non-empty direct-selection list. Reject bare namespace clauses, namespace aliases, implicit/wildcard exports and unavailable or private targets through the same semantic restrictions as today. A private dependency clause remains a private local binding. No automatic publication follows an ordinary import.

### Attachment, locations and diagnostics

- \`$export\` takes no operand and must appear alone on its source line. \`$export(...)\`, \`$export:\` and authored operands are invalid.
- Reuse the existing source-modifier adjacency rule: comment-only lines may intervene, blank physical lines may not. LF and CRLF must be equivalent. An unrelated item, another modifier, a block boundary or EOF ends attachment with the existing typed modifier error.
- Exactly one source modifier may target an item under the current directive model. Repeated or conflicting modifiers receive the ordinary duplicate/wrong-target diagnostic and appropriate related spans. This does not create generic modifier stacking.
- The marked item must be a top-level item in \`@mod.moth\` or \`@package.moth\`. \`@entry.moth\`, ordinary helper \`.moth\` files, \`.mtf\`, config, functions and nested scopes have no export surface. Known \`$export\` in the wrong context receives a typed directive placement/target error with the authored span.
- Recognise a pending source modifier before either a declaration or a dependency clause. \`$config\` must still accept only its existing explicitly typed top-level \`#\` binding, not dependency clauses or exportable items in general.
- Preserve diagnostic identity discipline: do not repurpose unrelated codes, manufacture a legacy export diagnostic or give migration advice. Remove unused export-block-specific diagnostic variants/rendering when no longer needed.

### Intentional non-goals

Do not make all facade declarations implicitly public. Do not allow \`$export\` in arbitrary module files, eliminate the authoritative \`@mod\`/\`@package\` facade files, add a \`$private\` directive, create export lists, support \`$export:\` grouping or keep a dual syntax. Revisit these only through a separate language-design decision informed by real projects.

## 2. Implementation slices

### Slice 0 - baseline and owner inventory

At activation, record the actual implementation base SHA and current tree state in working notes, not the plan. Confirm the separate markdown-section-marker branch is not accidentally included. Inventory all \`export:\` source sites, examples, fixtures, diagnostics and Rust symbols, distinguishing Moth source exports from unrelated JavaScript, Wasm and Rust uses of the English word "export".

Inspect especially:

- \`src/compiler_frontend/style_directives/{specs,builtins,registry}.rs\`
- \`src/compiler_frontend/headers/{file_parser,top_level_classifier,file_state,file_dependency_clauses,hash_items,header_dispatch,types,public_exports}.rs\`
- \`crates/moth-lexical/src/words.rs\`, \`src/compiler_frontend/keywords.rs\` and \`src/compiler_frontend/tokenizer/schema.rs\`
- \`src/compiler_frontend/public_interface/export_projection.rs\` and the AST public-surface owners
- Header/export tests, \`tests/cases/\`, first-party source packages, starter/scaffold templates, editor/highlighter source-word consumers and canonical docs.

Use \`index.md\` and symbol references to locate further consumers. Prefer deleting obsolete code over retaining a translation layer.

### Slice 1 - register and attach one source directive

Add \`export\` to the compiler-owned source directive registry as an operand-free \`SourceModifier\`. Specify exportable top-level items as its target contract. Keep \`$config\`'s narrower target unchanged.

Generalise only the existing pending-source-modifier dispatch needed to reach both declaration and dependency-clause targets. Preserve all existing blank-line, comment, EOF, duplicate, invalid-argument and context behaviour. Avoid splitting the parser into a second directive dispatcher or introducing a general AST annotation framework.

Check facade role at the source-owned header boundary. Check target eligibility before committing public metadata. A modifier before a dependency must use the existing strict public dependency parser/recording contract, not a new path scanner.

### Slice 2 - apply existing visibility metadata

For direct declarations, mark the accepted header with \`HeaderExportMode::Public\`. For dependencies, mark the retained \`RetainedDependencyClause\` public and require its strict direct selections. In particular, audit the \`#\` constant path through \`hash_items.rs\`, since the old block temporarily set file parser export mode. Do not miss constants or let pending directive state leak onto later items.

Keep export mode private as the default. A modifier is per item, never a file-wide mode. Reuse \`HeaderKind::is_authored_public_export_declaration\` and existing public-export passes. Preserve the two-pass public declaration and re-export resolution, root-qualified public origins, aliases, collision validation, generated semantics, effects, fingerprints and codegen-facing facts. The resulting exported interface for equivalent new/old valid programs must have the same semantic content.

### Slice 3 - delete the legacy language form completely

Delete \`handle_export_block\`, its item parser, nested export classification, duplicate/empty block policy, file-wide \`seen_export_block\`, \`export_block_item_count\`, mutable export-block parsing mode, the old inline-export rejection branch and obsolete helpers and imports used solely by that path.

Remove \`Export\` from the shared Moth source-word inventory and keyword mapping, plus \`TokenTag::EXPORT\` classification and associated tests and name reservation. Follow the stable packed-token taxonomy's invariants if its old numeric tag value must remain unused. Never retain an accepted legacy token or reuse a persisted ID in a way that misclassifies data. Check lexical shared identifier policy and Moth \`$code\` highlighting. Keep JavaScript's \`export\` keyword, Wasm export concepts and external JS export parsing intact.

Verify \`export\` is now a legal user-authored identifier wherever ordinary source identifier rules permit it. Old \`export:\` text has no special recognition, warning, migration hint or compatibility handling. A resulting generic parse error is acceptable.

Audit/remove export-block-only diagnostics, reason enums, message/render cases, tests, comments, categories and source fixtures. Keep ordinary valid export semantics and their diagnostics.

### Slice 4 - migrate all first-party Moth source and tests

Convert all \`export:\` blocks in first-party \`.moth\` sources, packages, examples, fixtures and generated starter assets to the new directive. Keep visibility and API shape unchanged. Use a grouped \`$export\` direct-selection clause where declarations are already defined in helpers and it improves the facade, without gratuitously moving code or changing dependency graphs simply to reduce lines.

For each legacy block item, preserve category, name, alias and relative source intent. Remove old block delimiters. Leave private items private. Explicitly inspect mixed files with both private and public declarations, root-local identifiers and modules containing both \`@mod\` and \`@package\`.

Migrate tests instead of leaving old spelling as accepted fixtures. Retire legacy-only export block tests and repurpose meaningful regressions to exercise the new directive. A small generic-invalid-syntax assertion may cover leftover old text, but must not encode an export-specific migration diagnostic.

### Slice 5 - documentation, matrix and tooling updates

Update the canonical directives, public API, module roots, dependency and re-export references first. Update Basic docs, cheatsheet, examples, reserved-name inventory, package examples, language overview and progress matrices to describe only the delivered form. Remove every "current export block / future $export" transitional note.

Update the editor syntax support, docs highlighting and any package/scaffold examples governed by the shared lexical catalogue. Preserve \`export\` recognition for other languages such as JavaScript. No duplicate new highlighter keyword inventory.

Preserve permanent semantic contracts from this plan in the owning canonical docs before retiring the plan. Do not change independent module topology, private/public rules or build architecture.

### Slice 6 - validate, audit and retire the plan

Run targeted parser/directive, lexical and header/public-interface tests first. Run the relevant complete project integration cases and incremental reuse tests next. Follow the selected routine/full validation gates in \`validation.mtf\`, including release docs build after examples are migrated, and applicable macOS/Linux CI gates.

Perform a final deletion and code-bloat audit against the code review guide. Search production Rust, tests, source packages and docs for old block-specific parser symbols, obsolete diagnostics and old Moth \`export:\` examples. Exclude intentional unrelated JavaScript/Wasm/Rust export terminology from removal.

Verify a stable public-interface fingerprint for a semantically equivalent public surface and the correct invalidation when \`$export\` is added, removed, renamed or changed. Avoid introducing a directive-specific fingerprint dimension.

After all implementation and validation gates pass, delete this plan in the same completing implementation commit and replace the active roadmap entry with the roadmap's normal completed-work disposition. No intermediate "completed but still present" plan commit.

## 3. Required regression coverage

Follow the existing test ownership and pruning rules. Prefer improving existing tests over duplicating them.

1. Direct public declaration acceptance: function, generic function, struct, choice, type alias, trait, inferred and typed \`#\` constant. Unmarked counterparts are private. An empty facade exports nothing.
2. Strict re-exports: several selections on one clause, aliases, comma-continuation, external and source providers, same-module declarations, public/private targets, missing target, duplicate public name, bare namespace, namespace alias, empty list and malformed trailing comma. Verify consumers observe the selected names only.
3. Directive attachment: own line and no operand, leading file position, intervening comments, blank lines, LF/CRLF, separated items, EOF, duplicate modifiers, source-to-template wrong context, template-to-source wrong context, unsupported targets and \`$config\` before a dependency.
4. Root roles: normal and support facades accepted, entry, helper, config and nested scopes rejected. Co-located \`@entry\` does not turn an entry exportable. Root-local isolation and support-consumer topology remain enforced.
5. Public semantic closure: exported type surfaces/methods, generic functions, trait evidence, result/effect/borrow summaries, closed error contracts and private/project-context leakage. Re-exports preserve origins and aliases. Tests must validate public consumer results, not only \`export_mode\` bits.
6. Lexical release: \`export\` is a legal identifier and no longer a Moth source keyword in the shared word classifier, tokenizer, name policy and Moth code highlighter. JavaScript export scanning/highlighting still works.
7. Build and incremental behaviour: validate representative first-party packages and a multi-file app through real compilation and runtime where supported. Prove true cached-module reuse before checking a private-only edit versus added/removed/renamed public export. Match diagnostic codes and relevant authored spans for wrong-target and misplaced directives.

## Completion gates

- One final \`$export\` prefix syntax for exportable declarations and direct-selection dependencies in the two facade roots only.
- No supported \`export:\` or legacy inline \`export\` syntax, export-specific migration diagnostic, compatibility mode, retained export-block parser or reserved Moth source keyword.
- No new public-interface pass, duplicate export metadata owner, independent source scanner or material semantic/backend changes.
- No leakage through facade privacy, aliases, package exports, type/effect/lifetime contracts or project globals.
- All first-party sources, examples and fixtures compile or diagnose as intended using only the new syntax.
- Focused regressions, relevant integration suites, full applicable validation and docs release build pass.
- Obsolete code/tests/docs are removed and the plan is retired in the completing implementation commit.
