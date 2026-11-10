# Export directive migration

## Status

- Status: ACTIVE on `feature/export-directive`.
- Current slice: 3 - remove the legacy `export:` block parser, reserved keyword and block-only diagnostics.
- Blockers: None. Source modifiers, facade roles, public-interface construction, `$export` parser attachment and first-party source/fixture migration are implemented and audited.
- Next action: Delete the legacy block and keyword machinery, then finish source-directive diagnostics and validation. Retire this plan in the completing implementation commit.

## Goal

Replace the `export:` block and reserved source keyword with operand-free `$export` on individual public declarations and direct-selection dependency clauses. This is a breaking alpha change. Remove the legacy language form and its exclusive machinery without compatibility parsing, migration warnings or special diagnostics. `export` becomes an ordinary identifier.

Preserve the existing public surface, binding and re-export semantics. This change controls how a public flag is authored, not how an interface is resolved, validated, fingerprinted or linked. Keep `@mod.moth` and `@package.moth` as the only authoritative public facades, with everything private by default.

## Contract

```moth
-- @mod.moth
private_helper || -> String:
    return "internal"
;

$export
greeting #= "Hello"

$export
render || -> String:
    return private_helper()
;

$export
@components/card CardData as Card, render_card

$export
@core/math PI, sin
```

- `$export` takes no arguments and must occupy its own line. It marks exactly the next eligible top-level item in that same facade. Comments may intervene but a blank physical line breaks attachment. LF and CRLF behave alike. A second modifier, unrelated statement, EOF or scope boundary cannot be skipped.
- The supported direct declaration kinds remain functions, structs, choices, transparent type aliases, traits and compile-time `#` constants, including inferred and explicitly typed constants. Reuse `HeaderKind::is_authored_public_export_declaration()` for the structural gate. All existing public type, trait, error, receiver, project-context and semantic-closure restrictions remain.
- A dependency clause is eligible only with a nonempty, explicit direct-selection list. One directive exports every selection on that clause. Keep current comma/newline continuation, leaf `as` aliases, original target identities, public-name collisions, private-target rejection and provider rules. Bare namespaces, namespace aliases, wildcard exports and export lists remain unsupported.
- An unmarked declaration or dependency stays private. Public direct-selection dependencies still create ordinary local selected bindings as before. A `$export` on an already declared private helper name is **not** a new export-list form: a facade re-exports that declaration using its existing direct-selection dependency syntax.
- Exclude runtime/mutable bindings, receiver methods as independent exports, conformances, incompatibilities, const/runtime templates, executable statements, nested declarations and `$config` contracts. Existing receiver methods may still be reachable through their exported nominal owner according to the current API rules.
- Only `@mod.moth` and `@package.moth` may use `$export`, including reached facades. Reject it in `@entry.moth`, ordinary source, config and other contexts. Co-located facade roles remain isolated. No implicit exports, `$export:` block, `$private` or exports from arbitrary helper files.
- Keep the current single-source-modifier policy. `$config` retains its exclusively typed `#` binding target and may not modify a dependency. Do not introduce generic modifier stacking or a new declaration annotation representation.

## Authorities

Read `AGENTS.md`, the compiler code review guide, `docs/compiler-design-overview.md`, `docs/build-system-design.md`, `docs/developer-docs/style-guide/{style-guide,testing,validation}.mtf` and the language navigation reference. Consult:

- `docs/docs/directives/directives.mtf` for attachment and contexts
- `docs/docs/project-structure/{public-api,module-roots}.mtf` for facade role and visibility
- `docs/docs/packages/public-reexports.mtf` for selections and aliases
- Both progress matrices and `index.md` for affected implementation owners

These are current-source references, not substitutes for their complete canonical sections. Record the actual branch baseline in working notes when beginning code work, not in this plan.

## Implementation

### 0. Inventory and baseline

Confirm the current branch tip and parent before editing. Search source, tests, source packages, starter templates and docs for Moth `export:` usage, the retired parser symbols and diagnostic text. Distinguish this from legitimate JavaScript/Wasm/Rust uses of `export`. Inspect these concrete owners before changing their interfaces:

- `src/compiler_frontend/style_directives/{specs,builtins}.rs`
- `src/compiler_frontend/headers/{file_parser,top_level_classifier,file_state,file_dependency_clauses,header_dispatch,types,public_exports,hash_items}.rs`
- `src/compiler_frontend/declaration_syntax/declaration_shell.rs`
- `crates/moth-lexical/src/words.rs`, `src/compiler_frontend/keywords.rs` and `src/compiler_frontend/tokenizer/schema.rs`
- `src/compiler_frontend/compiler_messages/{diagnostics,render}` and the public-interface/AST projection consumers

### 1. Attach the directive and reuse the existing public flag

Register compiler-owned `export` through `StyleDirectiveSpec::source_modifier` with one new `SourceModifierTarget` category for exportable public API items. Do not branch on the directive name string or create a second directive dispatcher. `PendingSourceModifier` already retains a typed target and authored span.

Extend the file walk so a pending modifier may reach either a declaration or `HeaderFileItem::Dependency`. Validate the target against the spec and `FileRole::is_export_capable()` (also respect config source classification). Consume the modifier once. Derive `HeaderExportMode::Public` solely from an attached `$export`, otherwise `Private`. Feed that mode through existing `HeaderDeclarationMetadata` and `RetainedDependencyClause`. Reuse `HeaderKind::is_authored_public_export_declaration()` rather than creating another kind whitelist. Preserve the special existing rejection of exported receiver methods.

**Critical metadata separation:** `file_parser.rs` currently passes every pending modifier span as `HeaderDeclarationMetadata.config_input_span`. `declaration_shell.rs` uses that span to admit a missing initializer and build a `$config` contract. Pass `config_input_span` **only** for the `ExplicitTypedCompileTimeBinding` modifier. A `$export`-annotated typed `#` constant must remain an ordinary required-initializer constant and must never gain `config_contract`. Cover both valid and missing-initializer cases in tests.

In `file_dependency_clauses.rs`, merge `parse_and_record_private_dependency` and `parse_and_record_public_dependency` into one parser entry receiving the export mode. Derive the existing strict-selection requirement from `mode.is_public()`, removing the redundant `require_selection_clause` flag and any now-pointless request wrapper. Preserve the private config-file dependency prohibition, checked path validation and one `DependencyShellId` per authored clause. Reuse the retained selection table and existing public export passes unchanged.

The two-pass public export builder and the separate source-package/module-root passes have distinct ownership and remain. Avoid new binding tables, AST rescans, facade lookups or fingerprints.

### 2. Delete the old block and keyword, not translate them

Delete `handle_export_block`, `parse_export_block_item`, `reject_non_block_export`, `classify_export_block_item_ref`, `HeaderFileItem::{Export,ExportBlock}` and block-specific request helpers. Drop `HeaderFileParseState::{seen_export_block,export_mode,export_block_item_count}`. The normal file loop becomes the sole top-level item walker. Remove obsolete checks in `hash_items.rs` that only applied while parsing an export block. Retain `HeaderExportMode` itself as meaningful per-item metadata.

Remove `SourceWord::Export` and `TokenTag::EXPORT` classification from `crates/moth-lexical/src/words.rs`, `keywords.rs` and `tokenizer/schema.rs`. Keep existing token discriminants stable and leave the removed numeric slot unused rather than renumbering tags. Update lexical, reserved-name and Moth highlighter tests. Ensure other languages' `export` keywords remain intact.

Remove the block-only `DUPLICATE_EXPORT_BLOCK` and `EXPORT_OUTSIDE_MODULE_ROOT` diagnostic families, their typed reason, renderer, message tests and obsolete fixture expectations. Use the existing source-directive placement diagnostic for `$export` in an ineligible file. Keep the general `INVALID_EXPORT_TARGET` diagnostic for invalid strict re-export selections, but make its title independent of the removed `export:` spelling. Do not reuse retired numeric/stable diagnostic IDs or retain compatibility aliases.

### 3. Make attachment and error messages source-directive aware

Keep current `InvalidTemplateDirectiveReason` codes and authored spans:

- `UnexpectedArguments` for `$export(...)`, `$export:` or other same-line operands
- `DuplicateModifier` for repeated/stacked source modifiers, with the earlier span
- `DanglingModifier` for blank-line separation or EOF
- `WrongTarget` for a following ineligible item, including `$config` before a dependency, `$export` before executable syntax or an invalid declaration
- `DirectiveNotAllowedHere` for `$export` in a file without a public facade surface
- `WrongContext` for a known source directive inside a template head and vice versa

The pending-modifier preflight currently maps every non-declaration item to `DanglingModifier`. Distinguish an actual missing target from a present, invalid one while allowing dependency clauses through to per-directive validation. Do not accidentally ignore a pending modifier on a `Hash`/template/runtime branch or leak it onto later items.

Update `src/compiler_frontend/compiler_messages/render/mod.rs`: its source-modifier messages are currently hardcoded to `$config` or say "template directive" and "declaration". `$export` must receive correct own-line, allowed-facade, declaration-or-dependency and wrong-template-context guidance. Update the unknown-directive suggestions to include `$export`. Make the smallest name-aware adjustments to the existing renderer, not a second diagnostic family or registry.

Also fix stale `export:` recommendations in other diagnostic renderers, including the missing-module-public-surface guidance in `render/payload.rs`. Reject old `export:` text only by ordinary post-removal syntax rules. No "use $export instead" legacy diagnostic.

### 4. Migrate, test and validate

The final-surface language references, basic guides, cheatsheet and compiler/build design authorities have been updated ahead of implementation on this branch. Recheck them after the compiler lands, but avoid a second wholesale rewrite. Keep the progress matrix truthful to implementation status until validation passes.

Convert all remaining first-party Moth `export:` sites in packages, examples, scenarios and scaffolds to per-item `$export`, preserving public names and private declarations. Group related selected names into one re-export clause where already supported. Do not reorganise module topology just to reduce lines.

After implementation, update the progress matrices to reflect actual support, update Moth syntax highlighting and diagnostic help, then audit source and documentation examples for drift. The final-design references are already prepared. Leave JavaScript export syntax unchanged.

Revise existing parser, lexical, module-role, renderer and integration tests instead of keeping export-block-specific fixtures and adding parallel tests. Required distinct regression boundaries:

1. Every eligible authored declaration kind, inferred and typed `#` constants, required initializer, private-by-default declarations and empty facades.
2. Public versus private dependency shell retention, grouped selections, aliases, multiline comma continuation, source-file re-exports and external providers, invalid namespace/bare/empty/trailing-comma clauses, collisions and inaccessible targets.
3. Directive adjacency (first line, intervening comments, whitespace, LF/CRLF, blank lines and EOF), second modifier, operands, non-declaration targets, and `$config` on a dependency.
4. Active normal/support, reached facade, entry, helper, config and selected single-file roots. Preserve root-local isolation including `@entry` and `@package` cross-role restrictions.
5. Consumer-visible public names, private type/trait/error/receiver closure, origin identity, project-context rejection, valid public fingerprints and cache invalidation when exports change. Confirm private-only edits do not unnecessarily alter published interfaces under proven reuse.
6. `export` as an identifier in ordinary Moth source and shared lexical classification. Verify unrelated JavaScript export scanning/highlighting survives.

Use targeted checks first, then the applicable validation gate from `validation.mtf`, real package/project scenarios and release docs build. Audit leftover legacy code, diagnostics, help text and fixtures. Check changed-code bloat and keep the two-pass public export machinery untouched unless a proven correctness defect requires otherwise.

When the implementation passes, remove this plan **and its active roadmap bullet in the same completion commit**. Preserve stable design rules in canonical docs. This draft PR becomes implementation-ready only after code, tests and applicable CI gates pass.
